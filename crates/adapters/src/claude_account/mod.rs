//! Claude Code account state remains inside the owning environment.
use crate::environments::{
    DockerEnvironmentRuntime,
    process::{DockerProcess, FixedProcess},
};
use async_trait::async_trait;
use bollard::container::LogOutput;
use chrono::{DateTime, Utc};
use factory_core::{agent_connections::*, environments::Environment};
use futures_util::StreamExt;
use serde_json::Value;
use std::{sync::Arc, time::Duration};
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

const LIMIT: usize = 64 * 1024;

pub struct ClaudeAccountAdapter {
    runtime: Arc<DockerEnvironmentRuntime>,
}
impl ClaudeAccountAdapter {
    pub fn new(runtime: Arc<DockerEnvironmentRuntime>) -> Self {
        Self { runtime }
    }

    async fn command(
        &self,
        env: &Environment,
        command: FixedProcess,
    ) -> Result<(i64, Vec<u8>), AgentConnectionError> {
        let DockerProcess {
            id,
            mut input,
            mut output,
        } = self.runtime.start_process(env, command).await?;
        // These commands consume no input. They may close stdin before Docker
        // accepts EOF; output and a confirmed exit establish completion instead.
        let _ = tokio::time::timeout(Duration::from_secs(3), input.shutdown()).await;
        drop(input);
        let result = tokio::time::timeout(Duration::from_secs(35), async {
            let mut stdout = Vec::new();
            let mut stderr = 0usize;
            while let Some(chunk) = output.next().await {
                match chunk.map_err(|_| AgentConnectionError::Unavailable)? {
                    LogOutput::StdOut { message } | LogOutput::Console { message } => {
                        if stdout.len().saturating_add(message.len()) > LIMIT {
                            return Err(AgentConnectionError::Protocol);
                        }
                        stdout.extend_from_slice(&message);
                    }
                    LogOutput::StdErr { message } => {
                        stderr = stderr.saturating_add(message.len());
                        if stderr > LIMIT {
                            return Err(AgentConnectionError::Protocol);
                        }
                    }
                    _ => return Err(AgentConnectionError::Protocol),
                }
            }
            Ok(stdout)
        })
        .await
        .map_err(|_| AgentConnectionError::Timeout)
        .and_then(|r| r);
        drop(output);
        let exit = self
            .runtime
            .wait_process_exit(&id, Duration::from_secs(15))
            .await
            .map_err(|_| AgentConnectionError::CleanupUncertain)?;
        Ok((exit, result?))
    }
}

#[async_trait]
impl AgentConnectionPort for ClaudeAccountAdapter {
    async fn read(&self, env: &Environment) -> Result<ConnectionSnapshot, AgentConnectionError> {
        let (exit, bytes) = self.command(env, FixedProcess::ClaudeStatus).await?;
        account(exit, &bytes)
    }
    // Claude sign-in runs in the member's terminal, directly through the native CLI.
    async fn begin_login(
        &self,
        _env: &Environment,
        _id: Uuid,
        _expires: DateTime<Utc>,
    ) -> Result<ConnectionSnapshot, AgentConnectionError> {
        Err(AgentConnectionError::UnsupportedMethod)
    }
    async fn cancel_login(
        &self,
        _env: &Environment,
        _id: Uuid,
    ) -> Result<ConnectionSnapshot, AgentConnectionError> {
        Err(AgentConnectionError::UnsupportedMethod)
    }
    async fn logout(&self, env: &Environment) -> Result<ConnectionSnapshot, AgentConnectionError> {
        let (exit, _) = self.command(env, FixedProcess::ClaudeLogout).await?;
        if exit != 0 {
            return Err(AgentConnectionError::Unavailable);
        }
        let snapshot = self.read(env).await?;
        if snapshot.status != ConnectionStatus::Disconnected {
            return Err(AgentConnectionError::Protocol);
        }
        Ok(snapshot)
    }
}

fn account(exit: i64, bytes: &[u8]) -> Result<ConnectionSnapshot, AgentConnectionError> {
    let value: Value = serde_json::from_slice(bytes).map_err(|_| AgentConnectionError::Protocol)?;
    let logged_in = value["loggedIn"]
        .as_bool()
        .ok_or(AgentConnectionError::Protocol)?;
    if !logged_in && exit == 1 {
        return Ok(ConnectionSnapshot::account(None, Utc::now()));
    }
    if !logged_in || exit != 0 {
        return Err(AgentConnectionError::Protocol);
    }
    if value["authMethod"] != "claude.ai" {
        return Err(AgentConnectionError::UnsupportedMethod);
    }
    fn safe(value: &Value, limit: usize) -> Option<String> {
        value
            .as_str()
            .filter(|s| {
                !s.trim().is_empty() && s.len() <= limit && !s.chars().any(char::is_control)
            })
            .map(str::to_owned)
    }
    Ok(ConnectionSnapshot::account(
        Some(AccountSummary {
            email: safe(&value["email"], 320),
            plan: safe(&value["subscriptionType"], 64)
                .unwrap_or_else(|| "Claude subscription".into()),
        }),
        Utc::now(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_confirmed_personal_cli_login_is_projected() {
        assert_eq!(
            account(1, br#"{"loggedIn":false,"authMethod":"none"}"#)
                .unwrap()
                .status,
            ConnectionStatus::Disconnected
        );
        assert_eq!(
            account(0, br#"{"loggedIn":true,"authMethod":"claude.ai"}"#)
                .unwrap()
                .status,
            ConnectionStatus::Connected
        );
        assert!(matches!(
            account(0, br#"{"loggedIn":true,"authMethod":"api_key"}"#),
            Err(AgentConnectionError::UnsupportedMethod)
        ));
        assert!(matches!(
            account(0, br#"{"loggedIn":false,"authMethod":"none"}"#),
            Err(AgentConnectionError::Protocol)
        ));
        assert!(matches!(
            account(0, b"not json"),
            Err(AgentConnectionError::Protocol)
        ));
    }
}
