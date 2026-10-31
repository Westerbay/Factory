//! Share only an in-flight model discovery, never a completed server-side cache.
use factory_core::{
    conversations::{ConversationError, execution::ExecutionModel},
    environments::Environment,
    identity::CredentialHash,
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, Weak},
};
use tokio::sync::OnceCell;
use uuid::Uuid;

pub(super) struct ModelProbe {
    environment: Uuid,
    generation: i64,
    image: String,
    policy: String,
    pub result: OnceCell<ModelProbeResult>,
}

pub(super) struct ModelProbeResult {
    pub initiator: CredentialHash,
    pub models: Result<Vec<ExecutionModel>, ConversationError>,
}

impl ModelProbeResult {
    /// Call only after independently revalidating the receiving session/environment.
    pub fn for_session(
        &self,
        caller: CredentialHash,
    ) -> Result<Vec<ExecutionModel>, ConversationError> {
        match &self.models {
            Err(ConversationError::Unauthorized) if self.initiator != caller => {
                Err(ConversationError::Conflict)
            }
            result => result.clone(),
        }
    }
}

#[derive(Default)]
pub(super) struct ModelProbes(Mutex<HashMap<Uuid, Weak<ModelProbe>>>);

impl ModelProbes {
    pub fn for_environment(&self, env: &Environment) -> Result<Arc<ModelProbe>, ConversationError> {
        let mut probes = self.0.lock().map_err(|_| ConversationError::Unavailable)?;
        probes.retain(|_, probe| probe.strong_count() > 0);
        if let Some(probe) = probes.get(&env.user_id).and_then(Weak::upgrade) {
            if probe.environment != env.id
                || probe.generation != env.generation
                || probe.image != env.image_id
                || probe.policy != env.policy_version
            {
                return Err(ConversationError::Conflict);
            }
            return Ok(probe);
        }
        if probes.len() >= 256 {
            return Err(ConversationError::Unavailable);
        }
        let probe = Arc::new(ModelProbe {
            environment: env.id,
            generation: env.generation,
            image: env.image_id.clone(),
            policy: env.policy_version.clone(),
            result: OnceCell::new(),
        });
        probes.insert(env.user_id, Arc::downgrade(&probe));
        Ok(probe)
    }
}

#[cfg(test)]
mod tests;
