//! Codex 0.160 account protocol over a short-lived, private container process.
mod protocol;
use crate::environments::{
    DockerEnvironmentRuntime,
    process::{DockerProcess, FixedProcess},
};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use factory_core::{agent_connections::*, environments::Environment};
use protocol::Client;
use serde_json::json;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::AsyncWriteExt,
    sync::{Mutex as AsyncMutex, oneshot, watch},
};
use uuid::Uuid;

struct Pending {
    operation_id: Uuid,
    state: watch::Receiver<ConnectionSnapshot>,
    cancel: Option<oneshot::Sender<()>>,
    finished: oneshot::Receiver<Result<(), AgentConnectionError>>,
}
type Slot = Arc<AsyncMutex<Option<Pending>>>;
pub struct CodexAccountAdapter {
    runtime: Arc<DockerEnvironmentRuntime>,
    slots: Mutex<HashMap<Uuid, Slot>>,
}
impl CodexAccountAdapter {
    pub fn new(runtime: Arc<DockerEnvironmentRuntime>) -> Self {
        Self {
            runtime,
            slots: Mutex::new(HashMap::new()),
        }
    }
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
    async fn require_egress(&self) -> Result<(), AgentConnectionError> {
        if self.runtime.egress().available().await? {
            Ok(())
        } else {
            Err(AgentConnectionError::Unavailable)
        }
    }
    async fn open(
        &self,
        environment: &Environment,
    ) -> Result<(String, Client), AgentConnectionError> {
        let DockerProcess { id, input, output } = self
            .runtime
            .start_process(environment, FixedProcess::CodexAccount)
            .await?;
        Ok((id, Client::new(input, output)))
    }
    async fn short(
        &self,
        environment: &Environment,
        logout: bool,
    ) -> Result<ConnectionSnapshot, AgentConnectionError> {
        let (id, mut client) = self.open(environment).await?;
        let result = async {
            client.initialize().await?;
            if logout {
                client.logout().await
            } else {
                client.read_account().await
            }
        }
        .await;
        close(&self.runtime, &id, client).await?;
        result
    }
}
#[async_trait]
impl AgentConnectionPort for CodexAccountAdapter {
    async fn read(
        &self,
        environment: &Environment,
    ) -> Result<ConnectionSnapshot, AgentConnectionError> {
        self.require_egress().await?;
        let slot = self.slot(environment.id)?;
        let mut pending = slot.lock().await;
        if let Some(snapshot) = take_observation(&mut pending) {
            return Ok(snapshot);
        }
        self.short(environment, false).await
    }
    async fn begin_login(
        &self,
        environment: &Environment,
        operation_id: Uuid,
        expires_at: DateTime<Utc>,
    ) -> Result<ConnectionSnapshot, AgentConnectionError> {
        self.require_egress().await?;
        let slot = self.slot(environment.id)?;
        let mut pending = slot.lock().await;
        if pending
            .as_ref()
            .is_some_and(|p| p.state.borrow().status == ConnectionStatus::Connecting)
        {
            return Err(AgentConnectionError::Conflict);
        }
        *pending = None;
        let (id, mut client) = self.open(environment).await?;
        let start = async {
            client.initialize().await?;
            let response = client
                .request("account/login/start", json!({"type":"chatgptDeviceCode"}))
                .await?;
            protocol::challenge(response, operation_id, expires_at)
        }
        .await;
        let (challenge, login_id) = match start {
            Ok(v) => v,
            Err(error) => {
                close(&self.runtime, &id, client).await?;
                return Err(error);
            }
        };
        if expires_at <= Utc::now() {
            let _ = client.cancel(&login_id).await;
            close(&self.runtime, &id, client).await?;
            return Err(AgentConnectionError::Expired);
        }
        let snapshot = ConnectionSnapshot {
            status: ConnectionStatus::Connecting,
            account: None,
            challenge: Some(challenge),
            error: None,
            observed_at: Utc::now(),
        };
        let (state, receiver) = watch::channel(snapshot.clone());
        let (cancel, cancellation) = oneshot::channel();
        let (finished, completion) = oneshot::channel();
        let runtime = self.runtime.clone();
        tokio::spawn(async move {
            let duration = (expires_at - Utc::now()).to_std().unwrap_or(Duration::ZERO);
            let outcome = tokio::select! {
                result = client.completion(&login_id) => match result {
                    Ok(()) => client.read_account().await.and_then(|snapshot| {
                        if snapshot.status == ConnectionStatus::Connected {
                            Ok(snapshot)
                        } else {
                            Err(AgentConnectionError::LoginFailed)
                        }
                    }),
                    Err(error) => Err(error),
                },
                _ = cancellation => {
                    let result = client.cancel(&login_id).await;
                    result.and(Err(AgentConnectionError::Cancelled))
                },
                _ = tokio::time::sleep(duration) => {
                    let _ = client.cancel(&login_id).await;
                    Err(AgentConnectionError::Expired)
                },
            };
            let cleanup = close(&runtime, &id, client).await;
            let final_state = match cleanup {
                Err(error) => ConnectionSnapshot::failure(error, Utc::now()),
                Ok(()) => match outcome {
                    Ok(s) => s,
                    Err(e) => ConnectionSnapshot::failure(e, Utc::now()),
                },
            };
            state.send_replace(final_state);
            let _ = finished.send(cleanup);
        });
        *pending = Some(Pending {
            operation_id,
            state: receiver,
            cancel: Some(cancel),
            finished: completion,
        });
        Ok(snapshot)
    }
    async fn cancel_login(
        &self,
        environment: &Environment,
        operation_id: Uuid,
    ) -> Result<ConnectionSnapshot, AgentConnectionError> {
        let slot = self.slot(environment.id)?;
        let mut pending = slot.lock().await;
        let Some(active) = pending.as_mut().filter(|p| p.operation_id == operation_id) else {
            return Err(AgentConnectionError::Conflict);
        };
        if let Some(cancel) = active.cancel.take() {
            let _ = cancel.send(());
        }
        tokio::time::timeout(Duration::from_secs(35), &mut active.finished)
            .await
            .map_err(|_| AgentConnectionError::CleanupUncertain)?
            .map_err(|_| AgentConnectionError::CleanupUncertain)??;
        let snapshot = active.state.borrow().clone();
        *pending = None;
        Ok(snapshot)
    }
    async fn logout(
        &self,
        environment: &Environment,
    ) -> Result<ConnectionSnapshot, AgentConnectionError> {
        let slot = self.slot(environment.id)?;
        let mut pending = slot.lock().await;
        if pending
            .as_ref()
            .is_some_and(|p| p.state.borrow().status == ConnectionStatus::Connecting)
        {
            return Err(AgentConnectionError::Conflict);
        }
        *pending = None;
        self.short(environment, true).await
    }
}

/// Final observations are delivered once. A later read must inspect the actual
/// account, including when a watchdog has since resolved uncertain cleanup.
fn take_observation(pending: &mut Option<Pending>) -> Option<ConnectionSnapshot> {
    let snapshot = pending.as_ref()?.state.borrow().clone();
    if snapshot.status != ConnectionStatus::Connecting {
        *pending = None;
    }
    Some(snapshot)
}

async fn close(
    runtime: &DockerEnvironmentRuntime,
    id: &str,
    mut client: Client,
) -> Result<(), AgentConnectionError> {
    let shutdown = tokio::time::timeout(Duration::from_secs(3), client.input.shutdown()).await;
    drop(client);
    let exited = runtime
        .wait_process_exit(id, Duration::from_secs(15))
        .await
        .map_err(|_| AgentConnectionError::CleanupUncertain)?;
    if exited == 75 {
        return Err(AgentConnectionError::Conflict);
    }
    if shutdown.is_err() {
        return Err(AgentConnectionError::CleanupUncertain);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
