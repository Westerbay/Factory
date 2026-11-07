pub mod bundle;
mod coordinator;
mod delivery;
pub(crate) mod discussion;
mod profiles;
mod recovery;
mod review;

pub use delivery::FeatureDeliveryService;

use crate::conversations::coordinator::ConversationRuntime;
use factory_core::{
    conversations::ConversationError,
    environments::{Environment, EnvironmentActor},
    features::{
        AttemptStatus, FeatureCodec, FeatureCommand, FeatureError, FeatureExecutionRuntime,
        FeaturePage, FeatureStage, FeatureState, FeatureStore, FeatureWorkspaceRuntime, NewFeature,
        RevisionKey, TaskRole, WorkspaceError,
    },
    projects::Project,
};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex, Weak},
};
use uuid::Uuid;

pub struct FeatureCoordinator {
    pub(super) store: Arc<dyn FeatureStore>,
    conversations: Arc<ConversationRuntime>,
    pub(super) workspaces: Arc<dyn FeatureWorkspaceRuntime>,
    pub(super) runtime: Arc<dyn FeatureExecutionRuntime>,
    pub(super) codec: Arc<dyn FeatureCodec>,
    drivers: Mutex<HashMap<Uuid, Weak<tokio::sync::Mutex<()>>>>,
    local_attempts: LocalAttempts,
}

impl FeatureCoordinator {
    pub fn new(
        store: Arc<dyn FeatureStore>,
        conversations: Arc<ConversationRuntime>,
        workspaces: Arc<dyn FeatureWorkspaceRuntime>,
        runtime: Arc<dyn FeatureExecutionRuntime>,
        codec: Arc<dyn FeatureCodec>,
    ) -> Arc<Self> {
        Arc::new(Self {
            store,
            conversations,
            workspaces,
            runtime,
            codec,
            drivers: Mutex::new(HashMap::new()),
            local_attempts: LocalAttempts::default(),
        })
    }

    pub async fn create(
        &self,
        actor: EnvironmentActor,
        operation_id: Uuid,
        project_id: Uuid,
        provider: String,
        title: String,
        locale: &str,
    ) -> Result<FeatureState, FeatureError> {
        if !self.conversations.owns_feature_store(&self.store) {
            return Err(FeatureError::Unavailable);
        }
        if profiles::preferred(&provider, TaskRole::Coordinator).is_none() {
            return Err(FeatureError::Invalid);
        }
        let env = self
            .conversations
            .service
            .current_environment(actor)
            .await
            .map_err(conversation_error)?;
        let _account = self
            .conversations
            .lock_feature_account(actor)
            .await
            .map_err(conversation_error)?;
        self.conversations
            .require_connected(actor, &provider)
            .await
            .map_err(conversation_error)?;
        let opener = if locale == "fr" {
            "Qu'est-ce que tu veux construire ? Décris le résultat attendu et ce qui reste à décider."
        } else {
            "What would you like to build? Describe the outcome and any decisions we still need to make."
        };
        self.store
            .create(
                actor,
                &env,
                NewFeature {
                    operation_id,
                    project_id,
                    provider,
                    workflow_revision: bundle::WORKFLOW_REVISION.into(),
                    title,
                    opener: opener.into(),
                },
            )
            .await
    }

    pub async fn list(
        &self,
        actor: EnvironmentActor,
        project_id: Option<Uuid>,
        before: Option<Uuid>,
    ) -> Result<FeaturePage, FeatureError> {
        self.store.list(actor, project_id, before).await
    }

    pub async fn get(
        &self,
        actor: EnvironmentActor,
        id: Uuid,
    ) -> Result<FeatureState, FeatureError> {
        self.store.get(actor, id).await
    }

    pub async fn approve(
        self: &Arc<Self>,
        actor: EnvironmentActor,
        id: Uuid,
        operation_id: Uuid,
        revision: RevisionKey,
    ) -> Result<FeatureState, FeatureError> {
        let (env, _, state) = self.context(actor, id).await?;
        if state.feature.stage == FeatureStage::Architecture
            && state.architecture.as_ref() == Some(&revision)
            && !state.attempts.iter().any(|attempt| {
                attempt.claim.task.role == TaskRole::ArchitectureReview
                    && attempt.claim.revision.as_ref() == Some(&revision)
                    && attempt.claim.snapshot_id.is_none()
                    && attempt.status == AttemptStatus::Validated
            })
        {
            return Err(FeatureError::ReviewBlocked);
        }
        let receipt = self
            .store
            .apply(
                actor,
                &env,
                id,
                operation_id,
                FeatureCommand::ApproveRevision(revision),
            )
            .await?;
        self.schedule(actor, id);
        Ok(receipt.state)
    }

    /// A locally owned attempt may still be verifying or integrating after its
    /// provider process stops. Only another process can reconcile its durable state.
    pub fn is_attempt_locally_active(&self, feature_id: Uuid, attempt_id: Uuid) -> bool {
        self.local_attempts.contains((feature_id, attempt_id))
    }

    fn driver(&self, feature: Uuid) -> Result<Arc<tokio::sync::Mutex<()>>, FeatureError> {
        let mut drivers = self.drivers.lock().map_err(|_| FeatureError::Unavailable)?;
        drivers.retain(|_, driver| driver.strong_count() > 0);
        if let Some(driver) = drivers.get(&feature).and_then(Weak::upgrade) {
            return Ok(driver);
        }
        let driver = Arc::new(tokio::sync::Mutex::new(()));
        drivers.insert(feature, Arc::downgrade(&driver));
        Ok(driver)
    }

    pub(super) async fn context(
        &self,
        actor: EnvironmentActor,
        id: Uuid,
    ) -> Result<(Environment, Project, FeatureState), FeatureError> {
        let state = self.store.get(actor, id).await?;
        let (env, conversation, project) = self
            .conversations
            .service
            .context(actor, state.feature.conversation_id)
            .await
            .map_err(conversation_error)?;
        let project = project
            .filter(|project| project.id == state.feature.project_id)
            .ok_or(FeatureError::Conflict)?;
        if env.id != state.feature.environment_id || conversation.user_id != state.feature.user_id {
            return Err(FeatureError::Conflict);
        }
        Ok((env, project, state))
    }
}

type AttemptKey = (Uuid, Uuid);

#[derive(Default)]
struct LocalAttempts(Arc<Mutex<HashSet<AttemptKey>>>);

impl LocalAttempts {
    fn claim(&self, key: AttemptKey) -> Result<LocalAttempt, FeatureError> {
        let mut active = self.0.lock().map_err(|_| FeatureError::Unavailable)?;
        if !active.insert(key) {
            return Err(FeatureError::ExecutionUncertain);
        }
        Ok(LocalAttempt {
            registry: self.0.clone(),
            key,
        })
    }

    fn contains(&self, key: AttemptKey) -> bool {
        // A poisoned registry cannot establish that a local worker has finished.
        self.0.lock().map_or(true, |active| active.contains(&key))
    }
}

struct LocalAttempt {
    registry: Arc<Mutex<HashSet<AttemptKey>>>,
    key: AttemptKey,
}

impl Drop for LocalAttempt {
    fn drop(&mut self) {
        self.registry
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&self.key);
    }
}

fn conversation_error(error: ConversationError) -> FeatureError {
    match error {
        ConversationError::Unauthorized => FeatureError::Unauthorized,
        ConversationError::NotFound => FeatureError::NotFound,
        ConversationError::Conflict => FeatureError::Conflict,
        ConversationError::Invalid | ConversationError::ContextLimit => FeatureError::Invalid,
        ConversationError::Unavailable => FeatureError::Unavailable,
    }
}

fn workspace_error(error: WorkspaceError) -> FeatureError {
    match error {
        WorkspaceError::Invalid | WorkspaceError::ScopeViolation => FeatureError::Invalid,
        WorkspaceError::Conflict => FeatureError::Conflict,
        WorkspaceError::Unavailable => FeatureError::Unavailable,
        WorkspaceError::Uncertain => FeatureError::ExecutionUncertain,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_local_owner_holds_one_feature_attempt_until_it_is_dropped() {
        let attempts = LocalAttempts::default();
        let attempt = Uuid::new_v4();
        let key = (Uuid::new_v4(), attempt);
        let other_key = (Uuid::new_v4(), attempt);

        let guard = attempts.claim(key).unwrap();
        assert!(attempts.contains(key));
        assert!(matches!(
            attempts.claim(key),
            Err(FeatureError::ExecutionUncertain)
        ));
        assert!(!attempts.contains(other_key));

        let other = attempts.claim(other_key).unwrap();
        drop(guard);
        assert!(!attempts.contains(key));
        assert!(attempts.contains(other_key));
        assert!(attempts.claim(key).is_ok());
        drop(other);
    }
}
