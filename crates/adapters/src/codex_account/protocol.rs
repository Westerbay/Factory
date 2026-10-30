use crate::environments::process::{ProcessInput, ProcessOutput};
use bollard::container::LogOutput;
use chrono::{DateTime, Utc};
use factory_core::agent_connections::*;
use futures_util::StreamExt;
use serde_json::{Value, json};
use std::{collections::VecDeque, time::Duration};
use tokio::io::AsyncWriteExt;
use url::Url;
use uuid::Uuid;

const MAX_FRAME: usize = 64 * 1024;
const MAX_NOTIFICATIONS: usize = 16;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
pub(super) struct Client {
    pub input: ProcessInput,
    pub output: ProcessOutput,
    buffer: Vec<u8>,
    next_id: u64,
    notifications: VecDeque<Value>,
}
impl Client {
    pub fn new(input: ProcessInput, output: ProcessOutput) -> Self {
        Self {
            input,
            output,
            buffer: Vec::new(),
            next_id: 1,
            notifications: VecDeque::new(),
        }
    }
    pub async fn initialize(&mut self) -> Result<(), AgentConnectionError> {
        let response = self
            .request(
                "initialize",
                json!({"clientInfo":{"name":"factory","version":env!("CARGO_PKG_VERSION")}}),
            )
            .await?;
        if !response.get("userAgent").is_some_and(Value::is_string) {
            return Err(AgentConnectionError::Protocol);
        }
        self.send(json!({"method":"initialized"})).await
    }
    async fn send(&mut self, value: Value) -> Result<(), AgentConnectionError> {
        let mut bytes = serde_json::to_vec(&value).map_err(|_| AgentConnectionError::Protocol)?;
        if bytes.len() > MAX_FRAME {
            return Err(AgentConnectionError::Protocol);
        }
        bytes.push(b'\n');
        tokio::time::timeout(REQUEST_TIMEOUT, async {
            self.input.write_all(&bytes).await?;
            self.input.flush().await
        })
        .await
        .map_err(|_| AgentConnectionError::Timeout)?
        .map_err(|_| AgentConnectionError::Unavailable)
    }
    pub async fn request(
        &mut self,
        method: &str,
        params: Value,
    ) -> Result<Value, AgentConnectionError> {
        let id = self.next_id;
        self.next_id += 1;
        self.send(json!({"id":id,"method":method,"params":params}))
            .await?;
        tokio::time::timeout(REQUEST_TIMEOUT, async {
            loop {
                let value = self.frame().await?;
                if value.get("id").is_some() {
                    if value.get("id").and_then(Value::as_u64) != Some(id)
                        || value.get("method").is_some()
                    {
                        return Err(AgentConnectionError::Protocol);
                    }
                    if value.get("error").is_some() {
                        return Err(AgentConnectionError::LoginFailed);
                    }
                    return value
                        .get("result")
                        .cloned()
                        .ok_or(AgentConnectionError::Protocol);
                }
                if self.notifications.len() >= MAX_NOTIFICATIONS {
                    return Err(AgentConnectionError::Protocol);
                }
                self.notifications.push_back(value);
            }
        })
        .await
        .map_err(|_| AgentConnectionError::Timeout)?
    }
    async fn frame(&mut self) -> Result<Value, AgentConnectionError> {
        let mut discarded = 0;
        for _ in 0..128 {
            if let Some(newline) = self.buffer.iter().position(|b| *b == b'\n') {
                let frame: Vec<_> = self.buffer.drain(..=newline).collect();
                if frame.iter().all(u8::is_ascii_whitespace) {
                    continue;
                }
                return parse_frame(&frame);
            }
            let chunk = self
                .output
                .next()
                .await
                .ok_or(AgentConnectionError::Unavailable)?
                .map_err(|_| AgentConnectionError::Unavailable)?;
            match chunk {
                LogOutput::StdOut { message } | LogOutput::Console { message } => {
                    if self.buffer.len() + message.len() > MAX_FRAME {
                        return Err(AgentConnectionError::Protocol);
                    }
                    self.buffer.extend_from_slice(&message);
                }
                // Never project or log stderr: it may contain credentials from user configuration.
                LogOutput::StdErr { message } => {
                    discarded += message.len();
                    if discarded > MAX_FRAME {
                        return Err(AgentConnectionError::Protocol);
                    }
                }
                _ => return Err(AgentConnectionError::Protocol),
            }
        }
        Err(AgentConnectionError::Protocol)
    }
    pub async fn completion(&mut self, login_id: &str) -> Result<(), AgentConnectionError> {
        for _ in 0..128 {
            let notification = match self.notifications.pop_front() {
                Some(v) => v,
                None => self.frame().await?,
            };
            if notification.get("id").is_some() {
                return Err(AgentConnectionError::Protocol);
            }
            let method = notification
                .get("method")
                .and_then(Value::as_str)
                .ok_or(AgentConnectionError::Protocol)?;
            if method != "account/login/completed" {
                continue;
            }
            let params = &notification["params"];
            if params["loginId"].as_str() != Some(login_id) {
                return Err(AgentConnectionError::Protocol);
            }
            return match params["success"].as_bool() {
                Some(true) => Ok(()),
                Some(false) => Err(AgentConnectionError::LoginFailed),
                None => Err(AgentConnectionError::Protocol),
            };
        }
        Err(AgentConnectionError::Protocol)
    }
    pub async fn read_account(&mut self) -> Result<ConnectionSnapshot, AgentConnectionError> {
        account(
            self.request("account/read", json!({"refreshToken":false}))
                .await?,
        )
    }
    pub async fn cancel(&mut self, login_id: &str) -> Result<(), AgentConnectionError> {
        let value = self
            .request("account/login/cancel", json!({"loginId":login_id}))
            .await?;
        match value["status"].as_str() {
            Some("canceled" | "notFound") => Ok(()),
            _ => Err(AgentConnectionError::Protocol),
        }
    }
    pub async fn logout(&mut self) -> Result<ConnectionSnapshot, AgentConnectionError> {
        let result = self.request("account/logout", json!({})).await?;
        if !result.is_object() {
            return Err(AgentConnectionError::Protocol);
        }
        let snapshot = self.read_account().await?;
        if snapshot.status != ConnectionStatus::Disconnected {
            return Err(AgentConnectionError::Protocol);
        }
        Ok(snapshot)
    }
}
fn parse_frame(bytes: &[u8]) -> Result<Value, AgentConnectionError> {
    if bytes.len() > MAX_FRAME {
        return Err(AgentConnectionError::Protocol);
    }
    let value: Value = serde_json::from_slice(bytes).map_err(|_| AgentConnectionError::Protocol)?;
    fn bounded(value: &Value, depth: usize) -> bool {
        if depth > 32 {
            return false;
        }
        match value {
            Value::Array(v) => v.iter().all(|v| bounded(v, depth + 1)),
            Value::Object(v) => v.values().all(|v| bounded(v, depth + 1)),
            _ => true,
        }
    }
    if !value.is_object() || !bounded(&value, 0) {
        return Err(AgentConnectionError::Protocol);
    }
    Ok(value)
}
fn text(value: &Value, max: usize) -> Result<String, AgentConnectionError> {
    let value = value.as_str().ok_or(AgentConnectionError::Protocol)?;
    if value.is_empty() || value.len() > max || value.chars().any(char::is_control) {
        return Err(AgentConnectionError::Protocol);
    }
    Ok(value.to_owned())
}
pub(super) fn challenge(
    value: Value,
    operation_id: Uuid,
    expires_at: DateTime<Utc>,
) -> Result<(DeviceChallenge, String), AgentConnectionError> {
    if value["type"] != "chatgptDeviceCode" {
        return Err(AgentConnectionError::Protocol);
    }
    let verification_url = text(&value["verificationUrl"], 2048)?;
    let url = Url::parse(&verification_url).map_err(|_| AgentConnectionError::Protocol)?;
    if url.scheme() != "https"
        || url.host_str() != Some("auth.openai.com")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.fragment().is_some()
        || url.query().is_some()
        || url.path() != "/codex/device"
    {
        return Err(AgentConnectionError::Protocol);
    }
    let user_code = text(&value["userCode"], 64)?;
    if !user_code
        .bytes()
        .all(|c| c.is_ascii_alphanumeric() || c == b'-')
    {
        return Err(AgentConnectionError::Protocol);
    }
    let login_id = text(&value["loginId"], 256)?;
    Ok((
        DeviceChallenge {
            operation_id,
            verification_url,
            user_code,
            expires_at,
        },
        login_id,
    ))
}
fn account(value: Value) -> Result<ConnectionSnapshot, AgentConnectionError> {
    if !value["requiresOpenaiAuth"].is_boolean() {
        return Err(AgentConnectionError::Protocol);
    }
    let raw = &value["account"];
    if raw.is_null() {
        return Ok(ConnectionSnapshot::account(None, Utc::now()));
    }
    match raw["type"].as_str() {
        Some("apiKey" | "amazonBedrock") => Err(AgentConnectionError::UnsupportedMethod),
        Some("chatgpt") => Ok(ConnectionSnapshot::account(
            Some(AccountSummary {
                email: if raw["email"].is_null() {
                    None
                } else {
                    Some(text(&raw["email"], 320)?)
                },
                plan: text(&raw["planType"], 64)?,
            }),
            Utc::now(),
        )),
        _ => Err(AgentConnectionError::Protocol),
    }
}

#[cfg(test)]
mod tests;
