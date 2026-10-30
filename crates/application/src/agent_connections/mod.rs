//! Admission and ownership of account ceremonies. The HTTP host owns revocation timers.
use crate::environments::EnvironmentService;
use chrono::{DateTime, Duration, Utc};
use factory_core::{
    agent_connections::*,
    environments::{Environment, EnvironmentActor},
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use tokio::sync::Mutex as AsyncMutex;
use uuid::Uuid;

struct Pending {
    actor: EnvironmentActor,
    environment: Environment,
    operation_id: Uuid,
    expires_at: DateTime<Utc>,
    completed: Option<ConnectionSnapshot>,
}
type Slot = Arc<AsyncMutex<Option<Pending>>>;
pub struct AgentConnectionService {
    environments: Arc<EnvironmentService>,
    port: Arc<dyn AgentConnectionPort>,
    slots: Mutex<HashMap<Uuid, Slot>>,
}
impl AgentConnectionService {
    pub fn new(environments: Arc<EnvironmentService>, port: Arc<dyn AgentConnectionPort>) -> Self {
        Self {
            environments,
            port,
            slots: Mutex::new(HashMap::new()),
        }
    }
    fn slot(&self, user_id: Uuid) -> Result<Slot, AgentConnectionError> {
        let mut slots = self
            .slots
            .lock()
            .map_err(|_| AgentConnectionError::Unavailable)?;
        slots.retain(|_, slot| {
            Arc::strong_count(slot) > 1
                || slot.try_lock().map_or(true, |p| {
                    p.as_ref().is_some_and(|pending| {
                        pending.completed.is_none() || pending.expires_at > Utc::now()
                    })
                })
        });
        Ok(slots
            .entry(user_id)
            .or_insert_with(|| Arc::new(AsyncMutex::new(None)))
            .clone())
    }
    pub async fn read(
        &self,
        actor: EnvironmentActor,
    ) -> Result<ConnectionSnapshot, AgentConnectionError> {
        let slot = self.slot(actor.user_id)?;
        let mut pending = slot.lock().await;
        let environment = self.environments.require_running(actor).await?;
        // A monitor may have consumed the provider's final error. Deliver it once
        // to its initiating session; a changed environment/session gets a fresh read.
        if pending.as_ref().is_some_and(|p| p.completed.is_some()) {
            let completed = pending.take().expect("completed observation exists");
            if completed.expires_at > Utc::now()
                && validate(&completed, actor, &environment).is_ok()
            {
                return Ok(completed.completed.expect("completed observation exists"));
            }
        }
        if let Some(current) = pending.as_ref() {
            validate(current, actor, &environment)?;
        }
        let snapshot = self.port.read(&environment).await?;
        if snapshot.status == ConnectionStatus::Connecting
            && !pending.as_ref().is_some_and(|p| {
                snapshot
                    .challenge
                    .as_ref()
                    .is_some_and(|c| c.operation_id == p.operation_id)
            })
        {
            return Err(AgentConnectionError::Conflict);
        }
        if snapshot.status != ConnectionStatus::Connecting {
            *pending = None;
        }
        Ok(snapshot)
    }
    pub async fn begin_login(
        &self,
        actor: EnvironmentActor,
        session_expires_at: DateTime<Utc>,
    ) -> Result<ConnectionSnapshot, AgentConnectionError> {
        let slot = self.slot(actor.user_id)?;
        let mut pending = slot.lock().await;
        let environment = self.environments.require_running(actor).await?;
        discard_completed(&mut pending);
        if pending.is_some() {
            return Err(AgentConnectionError::Conflict);
        }
        // Reserve time for protocol completion and cleanup before the image watchdog.
        let expires_at = session_expires_at.min(Utc::now() + Duration::seconds(570));
        if expires_at <= Utc::now() {
            return Err(AgentConnectionError::Unauthorized);
        }
        let operation_id = Uuid::new_v4();
        let result = self
            .port
            .begin_login(&environment, operation_id, expires_at)
            .await;
        if result.is_ok() {
            *pending = Some(Pending {
                actor,
                environment,
                operation_id,
                expires_at,
                completed: None,
            });
        }
        result
    }
    pub async fn cancel_login(
        &self,
        actor: EnvironmentActor,
        operation_id: Uuid,
    ) -> Result<ConnectionSnapshot, AgentConnectionError> {
        let slot = self.slot(actor.user_id)?;
        let mut pending = slot.lock().await;
        let environment = self.environments.require_running(actor).await?;
        let Some(current) = pending.as_ref().filter(|p| p.operation_id == operation_id) else {
            return Err(AgentConnectionError::Conflict);
        };
        validate(current, actor, &environment)?;
        if current.completed.is_some() {
            return Err(AgentConnectionError::Conflict);
        }
        let result = self
            .port
            .cancel_login(&current.environment, current.operation_id)
            .await;
        *pending = None;
        result
    }
    pub async fn logout(
        &self,
        actor: EnvironmentActor,
    ) -> Result<ConnectionSnapshot, AgentConnectionError> {
        let slot = self.slot(actor.user_id)?;
        let mut pending = slot.lock().await;
        let environment = self.environments.require_running(actor).await?;
        discard_completed(&mut pending);
        if let Some(current) = pending.as_ref() {
            validate(current, actor, &environment)?;
            self.port
                .cancel_login(&current.environment, current.operation_id)
                .await?;
            // Cancellation can wait on a provider process; authorize logout again afterwards.
            let revalidated = self
                .environments
                .require_running(actor)
                .await
                .map_err(AgentConnectionError::from)
                .and_then(|fresh| validate(current, actor, &fresh));
            *pending = None;
            revalidated?;
        }
        self.port.logout(&environment).await
    }
    /// The host monitors an already-authorized operation, including after its HTTP request ends.
    pub async fn revalidate_pending(
        &self,
        actor: EnvironmentActor,
        operation_id: Uuid,
    ) -> Result<bool, AgentConnectionError> {
        let slot = self.slot(actor.user_id)?;
        let mut pending = slot.lock().await;
        let environment = self.environments.require_running(actor).await?;
        let Some(current) = pending.as_ref().filter(|p| p.operation_id == operation_id) else {
            return Ok(false);
        };
        if current.completed.is_some() {
            return Ok(false);
        }
        validate(current, actor, &environment)?;
        if current.expires_at <= Utc::now() {
            return Err(AgentConnectionError::Expired);
        }
        let snapshot = self.port.read(&environment).await?;
        if snapshot.status != ConnectionStatus::Connecting {
            if snapshot.status == ConnectionStatus::Error {
                pending
                    .as_mut()
                    .expect("authorized pending operation")
                    .completed = Some(snapshot);
            } else {
                *pending = None;
            }
            return Ok(false);
        }
        Ok(true)
    }
    /// Cancellation after revocation deliberately does not require a still-active session.
    /// Both actor and operation must match the reservation made during authorized begin_login.
    pub async fn interrupt(
        &self,
        actor: EnvironmentActor,
        operation_id: Uuid,
    ) -> Result<(), AgentConnectionError> {
        let slot = self.slot(actor.user_id)?;
        let mut pending = slot.lock().await;
        let Some(current) = pending.as_ref().filter(|p| {
            p.operation_id == operation_id
                && p.actor.session_hash == actor.session_hash
                && p.actor.user_id == actor.user_id
        }) else {
            return Ok(());
        };
        if current.completed.is_some() {
            *pending = None;
            return Ok(());
        }
        let result = self
            .port
            .cancel_login(&current.environment, current.operation_id)
            .await;
        *pending = None;
        result.map(|_| ())
    }
}

fn discard_completed(pending: &mut Option<Pending>) {
    if pending.as_ref().is_some_and(|p| p.completed.is_some()) {
        *pending = None;
    }
}

fn validate(
    pending: &Pending,
    actor: EnvironmentActor,
    current: &Environment,
) -> Result<(), AgentConnectionError> {
    let expected = &pending.environment;
    if pending.actor.session_hash != actor.session_hash
        || pending.actor.user_id != actor.user_id
        || expected.id != current.id
        || expected.generation != current.generation
        || expected.image_id != current.image_id
        || expected.policy_version != current.policy_version
    {
        Err(AgentConnectionError::Conflict)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests;
