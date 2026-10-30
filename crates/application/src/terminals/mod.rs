//! One-use attachment authorization and serialized terminal ownership.
use crate::environments::EnvironmentService;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use factory_core::{
    environments::{Environment, EnvironmentActor},
    identity::CredentialHash,
    terminals::*,
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use tokio::sync::{Mutex as AsyncMutex, watch};
use uuid::Uuid;

pub const TICKET_TTL_SECONDS: i64 = 30;
type Slot = Arc<AsyncMutex<Option<Active>>>;

struct Active {
    id: Uuid,
    environment: Environment,
    handle: String,
    replaced: watch::Sender<bool>,
}

struct Ticket {
    actor: EnvironmentActor,
    environment: Environment,
    expires_at: DateTime<Utc>,
    take_over: bool,
    previous: Option<Uuid>,
    slot: Slot,
}

pub struct TerminalAttachment {
    pub output: Box<dyn TerminalOutput>,
    pub replaced: watch::Receiver<bool>,
    input: Box<dyn TerminalInput>,
    id: Uuid,
    actor: EnvironmentActor,
    environment: Environment,
    slot: Slot,
}

pub struct TerminalService {
    environments: Arc<EnvironmentService>,
    runtime: Arc<dyn TerminalRuntime>,
    slots: Mutex<HashMap<Uuid, Slot>>,
    // This map is exclusively for terminal tickets: no cross-purpose consumption.
    tickets: Mutex<HashMap<CredentialHash, Ticket>>,
}

impl TerminalService {
    pub fn new(environments: Arc<EnvironmentService>, runtime: Arc<dyn TerminalRuntime>) -> Self {
        Self {
            environments,
            runtime,
            slots: Mutex::new(HashMap::new()),
            tickets: Mutex::new(HashMap::new()),
        }
    }

    fn slot(&self, id: Uuid) -> Result<Slot, TerminalError> {
        let mut slots = self.slots.lock().map_err(|_| TerminalError::Unavailable)?;
        slots.retain(|_, slot| {
            Arc::strong_count(slot) > 1 || slot.try_lock().map_or(true, |active| active.is_some())
        });
        if let Some(slot) = slots.get(&id) {
            return Ok(slot.clone());
        }
        let slot = Arc::new(AsyncMutex::new(None));
        slots.insert(id, slot.clone());
        Ok(slot)
    }

    pub async fn issue(
        &self,
        actor: EnvironmentActor,
        hash: CredentialHash,
        take_over: bool,
        session_expires_at: DateTime<Utc>,
    ) -> Result<DateTime<Utc>, TerminalError> {
        let environment = self.environments.require_running(actor).await?;
        let slot = self.slot(environment.id)?;
        let active = slot.lock().await;
        if active.is_some() && !take_over {
            return Err(TerminalError::Conflict);
        }
        let now = Utc::now();
        let expires_at =
            (now + ChronoDuration::seconds(TICKET_TTL_SECONDS)).min(session_expires_at);
        if expires_at <= now {
            return Err(TerminalError::Unauthorized);
        }
        let mut tickets = self
            .tickets
            .lock()
            .map_err(|_| TerminalError::Unavailable)?;
        // At most one pending ticket per user. Issuing again invalidates the old
        // ticket; expired entries retain neither an environment lock nor secrets.
        tickets
            .retain(|_, ticket| ticket.expires_at > now && ticket.actor.user_id != actor.user_id);
        tickets.insert(
            hash,
            Ticket {
                actor,
                environment,
                expires_at,
                take_over,
                previous: active.as_ref().map(|active| active.id),
                slot: slot.clone(),
            },
        );
        Ok(expires_at)
    }

    pub async fn attach(
        &self,
        actor: EnvironmentActor,
        hash: CredentialHash,
    ) -> Result<TerminalAttachment, TerminalError> {
        let ticket = self
            .tickets
            .lock()
            .map_err(|_| TerminalError::Unavailable)?
            .remove(&hash)
            .ok_or(TerminalError::InvalidTicket)?;
        if ticket.expires_at <= Utc::now()
            || ticket.actor.user_id != actor.user_id
            || ticket.actor.session_hash != actor.session_hash
        {
            return Err(TerminalError::InvalidTicket);
        }
        let mut active = ticket.slot.lock().await;
        let environment = self.environments.require_running(actor).await?;
        if ticket.expires_at <= Utc::now() {
            return Err(TerminalError::InvalidTicket);
        }
        require_same_environment(&ticket.environment, &environment)?;
        if active.as_ref().map(|active| active.id) != ticket.previous {
            return Err(TerminalError::Conflict);
        }
        if let Some(previous) = active.as_ref() {
            if !ticket.take_over {
                return Err(TerminalError::Conflict);
            }
            previous.replaced.send_replace(true);
            self.runtime
                .detach(&previous.environment, &previous.handle)
                .await?;
        }
        *active = None;
        let process = self.runtime.attach(&environment).await?;
        let id = Uuid::new_v4();
        let (replaced, receiver) = watch::channel(false);
        *active = Some(Active {
            id,
            environment: environment.clone(),
            handle: process.handle,
            replaced,
        });
        Ok(TerminalAttachment {
            output: process.output,
            input: process.input,
            replaced: receiver,
            id,
            actor,
            environment,
            slot: ticket.slot.clone(),
        })
    }

    pub async fn check(&self, attachment: &mut TerminalAttachment) -> Result<(), TerminalError> {
        let active = attachment.slot.lock().await;
        self.authorize(
            attachment.actor,
            &attachment.environment,
            attachment.id,
            active.as_ref(),
        )
        .await
    }

    async fn authorize(
        &self,
        actor: EnvironmentActor,
        expected: &Environment,
        id: Uuid,
        active: Option<&Active>,
    ) -> Result<(), TerminalError> {
        if active.is_none_or(|active| active.id != id || *active.replaced.borrow()) {
            return Err(TerminalError::Conflict);
        }
        let environment = self.environments.require_running(actor).await?;
        require_same_environment(expected, &environment)
    }

    pub async fn write(
        &self,
        attachment: &mut TerminalAttachment,
        input: &[u8],
    ) -> Result<(), TerminalError> {
        if input.is_empty() || input.len() > MAX_INPUT_BYTES {
            return Err(TerminalError::InvalidInput);
        }
        let active = attachment.slot.lock().await;
        self.authorize(
            attachment.actor,
            &attachment.environment,
            attachment.id,
            active.as_ref(),
        )
        .await?;
        attachment.input.write(input).await
    }

    pub async fn resize(
        &self,
        attachment: &mut TerminalAttachment,
        size: TerminalSize,
    ) -> Result<(), TerminalError> {
        let size = TerminalSize::new(size.cols, size.rows)?;
        let active = attachment.slot.lock().await;
        self.authorize(
            attachment.actor,
            &attachment.environment,
            attachment.id,
            active.as_ref(),
        )
        .await?;
        self.runtime
            .resize(
                &active.as_ref().ok_or(TerminalError::Conflict)?.handle,
                size,
            )
            .await
    }

    pub async fn finish(&self, attachment: TerminalAttachment) -> Result<(), TerminalError> {
        let TerminalAttachment {
            input,
            output,
            id,
            slot,
            ..
        } = attachment;
        // Both Docker stream halves must close before verifying EOF. The slot
        // lock prevents an old cleanup from detaching a newer connection.
        drop((input, output));
        let mut active = slot.lock().await;
        if let Some(current) = active.as_ref().filter(|current| current.id == id) {
            current.replaced.send_replace(true);
            self.runtime
                .detach(&current.environment, &current.handle)
                .await?;
            *active = None;
        }
        Ok(())
    }
}

fn require_same_environment(
    expected: &Environment,
    current: &Environment,
) -> Result<(), TerminalError> {
    if expected.id != current.id
        || expected.user_id != current.user_id
        || expected.generation != current.generation
        || expected.image_id != current.image_id
        || expected.policy_version != current.policy_version
    {
        Err(TerminalError::Conflict)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests;
