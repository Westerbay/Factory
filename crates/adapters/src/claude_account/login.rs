//! A bounded, private connection to Claude Code's own interactive sign-in.
use super::{ClaudeAccountAdapter, account};
use crate::environments::{
    DockerEnvironmentRuntime,
    process::{DockerProcess, FixedProcess, ProcessInput, ProcessOutput, ProcessStartError},
};
use bollard::container::LogOutput;
use chrono::{DateTime, Utc};
use factory_core::{agent_connections::*, environments::Environment};
use futures_util::StreamExt;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::AsyncWriteExt,
    sync::{Mutex as AsyncMutex, mpsc, oneshot, watch},
};
use url::Url;
use uuid::Uuid;

const OUTPUT_LIMIT: usize = 64 * 1024;
type Slot = Arc<AsyncMutex<Option<Pending>>>;
struct Pending {
    id: Uuid,
    state: watch::Receiver<ConnectionSnapshot>,
    commands: mpsc::Sender<Command>,
}
enum Command {
    Submit(String, oneshot::Sender<Result<(), AgentConnectionError>>),
    Cancel(oneshot::Sender<Result<(), AgentConnectionError>>),
}
#[derive(Default)]
pub(super) struct LoginManager {
    slots: Mutex<HashMap<Uuid, Slot>>,
}
impl LoginManager {
    fn slot(&self, id: Uuid) -> Result<Slot, AgentConnectionError> {
        let mut slots = self
            .slots
            .lock()
            .map_err(|_| AgentConnectionError::Unavailable)?;
        slots.retain(|_, slot| {
            Arc::strong_count(slot) > 1 || slot.try_lock().map_or(true, |p| p.is_some())
        });
        Ok(slots
            .entry(id)
            .or_insert_with(|| Arc::new(AsyncMutex::new(None)))
            .clone())
    }
    pub(super) async fn read(
        &self,
        env: Uuid,
    ) -> Result<Option<ConnectionSnapshot>, AgentConnectionError> {
        let slot = self.slot(env)?;
        let mut pending = slot.lock().await;
        let Some(current) = pending.as_ref() else {
            return Ok(None);
        };
        let state = current.state.borrow().clone();
        if state.status != ConnectionStatus::Connecting
            && state.error != Some(AgentConnectionError::CleanupUncertain)
        {
            *pending = None;
        }
        Ok(Some(state))
    }
    pub(super) async fn has_uncertain_cleanup(
        &self,
        env: Uuid,
    ) -> Result<bool, AgentConnectionError> {
        let slot = self.slot(env)?;
        let pending = slot.lock().await;
        Ok(pending.as_ref().is_some_and(|current| {
            current.state.borrow().error == Some(AgentConnectionError::CleanupUncertain)
        }))
    }
    pub(super) async fn begin(
        &self,
        runtime: Arc<DockerEnvironmentRuntime>,
        env: &Environment,
        id: Uuid,
        expires: DateTime<Utc>,
    ) -> Result<ConnectionSnapshot, AgentConnectionError> {
        if !runtime.egress().available().await? {
            return Err(AgentConnectionError::Unavailable);
        }
        let slot = self.slot(env.id)?;
        let mut pending = slot.lock().await;
        if pending.is_some() {
            return Err(AgentConnectionError::Conflict);
        }
        let mut process = runtime
            .start_process_checked(env, FixedProcess::ClaudeLogin)
            .await
            .map_err(|error| match error {
                ProcessStartError::PreSubmission(_) => AgentConnectionError::Unavailable,
                ProcessStartError::Submitted(_) => AgentConnectionError::CleanupUncertain,
            })?;
        let url = match tokio::time::timeout(Duration::from_secs(30), read_url(&mut process.output))
            .await
        {
            Ok(Ok(url)) => url,
            result => {
                let reason = match result {
                    Ok(Err(error)) => error,
                    _ => AgentConnectionError::Timeout,
                };
                stop(&runtime, process).await?;
                return Err(reason);
            }
        };
        if expires <= Utc::now() {
            stop(&runtime, process).await?;
            return Err(AgentConnectionError::Expired);
        }
        let snapshot = ConnectionSnapshot {
            status: ConnectionStatus::Connecting,
            account: None,
            challenge: Some(LoginChallenge {
                operation_id: id,
                verification_url: url,
                user_code: None,
                expires_at: expires,
            }),
            error: None,
            observed_at: Utc::now(),
        };
        let (state, receiver) = watch::channel(snapshot.clone());
        let (commands, rx) = mpsc::channel(2);
        let environment = env.clone();
        tokio::spawn(async move {
            run(runtime, environment, process, expires, rx, state).await;
        });
        *pending = Some(Pending {
            id,
            state: receiver,
            commands,
        });
        Ok(snapshot)
    }
    pub(super) async fn submit(
        &self,
        env: Uuid,
        id: Uuid,
        code: &str,
    ) -> Result<ConnectionSnapshot, AgentConnectionError> {
        let slot = self.slot(env)?;
        let pending = slot.lock().await;
        let current = pending
            .as_ref()
            .filter(|p| p.id == id && p.state.borrow().status == ConnectionStatus::Connecting)
            .ok_or(AgentConnectionError::Conflict)?;
        let (tx, rx) = oneshot::channel();
        current
            .commands
            .send(Command::Submit(code.to_owned(), tx))
            .await
            .map_err(|_| AgentConnectionError::Unavailable)?;
        tokio::time::timeout(Duration::from_secs(10), rx)
            .await
            .map_err(|_| AgentConnectionError::Timeout)?
            .map_err(|_| AgentConnectionError::Unavailable)??;
        let snapshot = current.state.borrow().clone();
        Ok(snapshot)
    }
    pub(super) async fn cancel(
        &self,
        env: Uuid,
        id: Uuid,
    ) -> Result<ConnectionSnapshot, AgentConnectionError> {
        let slot = self.slot(env)?;
        let mut pending = slot.lock().await;
        let current = pending
            .as_ref()
            .filter(|p| p.id == id)
            .ok_or(AgentConnectionError::Conflict)?;
        let (tx, rx) = oneshot::channel();
        current
            .commands
            .send(Command::Cancel(tx))
            .await
            .map_err(|_| AgentConnectionError::CleanupUncertain)?;
        tokio::time::timeout(Duration::from_secs(25), rx)
            .await
            .map_err(|_| AgentConnectionError::CleanupUncertain)?
            .map_err(|_| AgentConnectionError::CleanupUncertain)??;
        let snapshot = current.state.borrow().clone();
        *pending = None;
        Ok(snapshot)
    }
}

async fn read_url(output: &mut ProcessOutput) -> Result<String, AgentConnectionError> {
    let mut bytes = Vec::new();
    while let Some(chunk) = output.next().await {
        match chunk.map_err(|_| AgentConnectionError::Unavailable)? {
            LogOutput::Console { message }
            | LogOutput::StdOut { message }
            | LogOutput::StdErr { message } => {
                if bytes.len().saturating_add(message.len()) > OUTPUT_LIMIT {
                    return Err(AgentConnectionError::Protocol);
                }
                bytes.extend_from_slice(&message);
            }
            _ => return Err(AgentConnectionError::Protocol),
        }
        if bytes
            .windows(b"Paste code here".len())
            .any(|window| window == b"Paste code here")
        {
            let output = std::str::from_utf8(&bytes).map_err(|_| AgentConnectionError::Protocol)?;
            // Claude wraps the link in an OSC-8 terminal hyperlink when stdout
            // is a PTY, so the URL can follow an escape sequence without whitespace.
            let start = output
                .find("https://claude.com/cai/oauth/authorize?")
                .ok_or(AgentConnectionError::Protocol)?;
            let rest = &output[start..];
            let end = rest
                .find(|c: char| c.is_whitespace() || c.is_control())
                .unwrap_or(rest.len());
            let candidate = &rest[..end];
            return validate_url(candidate);
        }
    }
    Err(AgentConnectionError::LoginFailed)
}
fn validate_url(value: &str) -> Result<String, AgentConnectionError> {
    let url = Url::parse(value).map_err(|_| AgentConnectionError::Protocol)?;
    if value.len() > 8192
        || url.scheme() != "https"
        || url.host_str() != Some("claude.com")
        || url.path() != "/cai/oauth/authorize"
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || url.query().is_none()
    {
        return Err(AgentConnectionError::Protocol);
    }
    let required = [
        (
            "redirect_uri",
            "https://platform.claude.com/oauth/code/callback",
        ),
        ("response_type", "code"),
        ("code_challenge_method", "S256"),
    ];
    for (name, expected) in required {
        let values: Vec<_> = url.query_pairs().filter(|(key, _)| key == name).collect();
        if values.len() != 1 || values[0].1 != expected {
            return Err(AgentConnectionError::Protocol);
        }
    }
    for name in ["state", "code_challenge"] {
        let values: Vec<_> = url.query_pairs().filter(|(key, _)| key == name).collect();
        if values.len() != 1 || values[0].1.is_empty() {
            return Err(AgentConnectionError::Protocol);
        }
    }
    Ok(value.to_owned())
}
async fn interrupt(input: &mut ProcessInput) -> Result<(), AgentConnectionError> {
    tokio::time::timeout(Duration::from_secs(3), async {
        input.write_all(b"\x03").await?;
        input.flush().await
    })
    .await
    .map_err(|_| AgentConnectionError::CleanupUncertain)?
    .map_err(|_| AgentConnectionError::CleanupUncertain)
}
async fn stop(
    runtime: &DockerEnvironmentRuntime,
    mut process: DockerProcess,
) -> Result<(), AgentConnectionError> {
    let signal = interrupt(&mut process.input).await;
    let id = process.id.clone();
    drop(process);
    let exit = runtime
        .wait_process_exit(&id, Duration::from_secs(15))
        .await
        .map_err(|_| AgentConnectionError::CleanupUncertain)?;
    if exit == 75 {
        return Err(AgentConnectionError::Conflict);
    }
    signal
}

async fn run(
    runtime: Arc<DockerEnvironmentRuntime>,
    env: Environment,
    mut process: DockerProcess,
    expires: DateTime<Utc>,
    mut commands: mpsc::Receiver<Command>,
    state: watch::Sender<ConnectionSnapshot>,
) {
    let mut sent = false;
    let mut total = 0usize;
    let mut cancelled = None;
    let mut problem = None;
    let deadline =
        tokio::time::Instant::now() + (expires - Utc::now()).to_std().unwrap_or_default();
    loop {
        tokio::select! {
            _ = tokio::time::sleep_until(deadline) => { problem = Some(AgentConnectionError::Expired); break; }
            command = commands.recv() => match command {
                Some(Command::Submit(code, reply)) => {
                    if sent { let _ = reply.send(Err(AgentConnectionError::Conflict)); continue; }
                    sent = true;
                    let result = tokio::time::timeout(Duration::from_secs(5), async {
                        process.input.write_all(code.as_bytes()).await?;
                        process.input.write_all(b"\n").await?;
                        process.input.flush().await
                    }).await.map_err(|_| AgentConnectionError::Timeout).and_then(|r| r.map_err(|_| AgentConnectionError::Unavailable));
                    let _ = reply.send(result);
                    if let Err(error) = result { problem = Some(error); break; }
                }
                Some(Command::Cancel(reply)) => { cancelled = Some(reply); break; }
                None => { problem = Some(AgentConnectionError::Cancelled); break; }
            },
            chunk = process.output.next() => match chunk {
                None => break,
                Some(Ok(chunk)) => { total = total.saturating_add(chunk.as_ref().len()); if total > OUTPUT_LIMIT { problem = Some(AgentConnectionError::Protocol); break; } },
                Some(Err(_)) => { problem = Some(AgentConnectionError::Unavailable); break; },
            }
        }
    }
    let interrupted = cancelled.is_some() || problem.is_some();
    let signal = if interrupted {
        interrupt(&mut process.input).await
    } else {
        Ok(())
    };
    let id = process.id.clone();
    drop(process);
    let exit = runtime
        .wait_process_exit(&id, Duration::from_secs(15))
        .await
        .map_err(|_| AgentConnectionError::CleanupUncertain);
    let cleanup = exit
        .as_ref()
        .map(|_| ())
        .map_err(|error| *error)
        .and(signal);
    let result = match cleanup {
        Err(error) => Err(error),
        Ok(()) if cancelled.is_some() => Err(AgentConnectionError::Cancelled),
        Ok(()) if problem.is_some() => Err(problem.expect("problem checked")),
        Ok(()) if exit == Ok(0) && sent => {
            let adapter = ClaudeAccountAdapter::new(runtime);
            adapter
                .command(&env, FixedProcess::ClaudeStatus)
                .await
                .and_then(|(exit, bytes)| account(exit, &bytes))
                .and_then(|snapshot| {
                    if snapshot.status == ConnectionStatus::Connected {
                        Ok(snapshot)
                    } else {
                        Err(AgentConnectionError::LoginFailed)
                    }
                })
        }
        Ok(()) if exit == Ok(75) => Err(AgentConnectionError::Conflict),
        Ok(()) => Err(AgentConnectionError::LoginFailed),
    };
    state.send_replace(
        result.unwrap_or_else(|error| ConnectionSnapshot::failure(error, Utc::now())),
    );
    if let Some(reply) = cancelled {
        let _ = reply.send(cleanup);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::stream;
    const QUERY: &str = "redirect_uri=https%3A%2F%2Fplatform.claude.com%2Foauth%2Fcode%2Fcallback&response_type=code&code_challenge_method=S256&state=fixture&code_challenge=fixture";
    #[test]
    fn only_an_official_authorization_url_is_accepted() {
        assert!(validate_url(&format!("https://claude.com/cai/oauth/authorize?{QUERY}")).is_ok());
        assert!(
            validate_url("https://claude.com.evil.test/cai/oauth/authorize?state=fixture").is_err()
        );
        assert!(validate_url("https://user@claude.com/cai/oauth/authorize?state=fixture").is_err());
        assert!(
            validate_url("https://claude.com/cai/oauth/authorize?state=fixture#fragment").is_err()
        );
        assert!(
            validate_url(&format!(
                "https://claude.com/cai/oauth/authorize?{QUERY}&redirect_uri=https://evil.test"
            ))
            .is_err()
        );
    }

    #[tokio::test]
    async fn extracts_a_fragmented_cli_challenge_without_accepting_foreign_links() {
        fn output(parts: &[&'static str]) -> ProcessOutput {
            Box::pin(stream::iter(
                parts
                    .iter()
                    .map(|part| {
                        Ok(LogOutput::Console {
                            message: (*part).into(),
                        })
                    })
                    .collect::<Vec<_>>(),
            ))
        }
        let mut valid = output(&[
            "Opening browser\r\n\x1b]8;;https://claude.com/cai/oauth/",
            "authorize?redirect_uri=https%3A%2F%2Fplatform.claude.com%2Foauth%2Fcode%2Fcallback&response_type=code&code_challenge_method=S256&state=fixture&code_challenge=fixture\x07\r\nPaste code ",
            "here if prompted > ",
        ]);
        assert_eq!(
            read_url(&mut valid).await.unwrap(),
            format!("https://claude.com/cai/oauth/authorize?{QUERY}")
        );
        let mut foreign = output(&[
            "https://example.com/cai/oauth/authorize?state=fixture\r\nPaste code here if prompted > ",
        ]);
        assert!(matches!(
            read_url(&mut foreign).await,
            Err(AgentConnectionError::Protocol)
        ));
    }
}
