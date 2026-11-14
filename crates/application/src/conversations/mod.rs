//! Conversation entry points validate the supervisor's current environment before storage.
pub mod coordinator;
mod model_probes;
mod provider_sessions;
use crate::{environments::EnvironmentService, projects::ProjectService};
use factory_core::{
    conversations::*,
    environments::{DesiredState, Environment, EnvironmentActor, EnvironmentError, ObservedState},
    projects::{Project, ProjectState},
};
use std::sync::Arc;
use uuid::Uuid;

pub struct ConversationService {
    environments: Arc<EnvironmentService>,
    store: Arc<dyn ConversationStore>,
    projects: Option<Arc<ProjectService>>,
}
impl ConversationService {
    pub fn new(environments: Arc<EnvironmentService>, store: Arc<dyn ConversationStore>) -> Self {
        Self {
            environments,
            store,
            projects: None,
        }
    }
    /// The project-aware constructor is required for project conversations.
    pub fn new_with_projects(
        environments: Arc<EnvironmentService>,
        store: Arc<dyn ConversationStore>,
        projects: Arc<ProjectService>,
    ) -> Self {
        Self {
            environments,
            store,
            projects: Some(projects),
        }
    }

    /// Resolves an owned conversation and project under the current environment.
    pub async fn context(
        &self,
        actor: EnvironmentActor,
        id: Uuid,
    ) -> Result<(Environment, Conversation, Option<Project>), ConversationError> {
        let env = self.current_environment(actor).await?;
        let conversation = self.store.get(actor, env.id, env.generation, id).await?;
        let project = if let Some(project_id) = conversation.project_id {
            let projects = self
                .projects
                .as_ref()
                .ok_or(ConversationError::Unavailable)?;
            let project = projects
                .get(actor, project_id)
                .await
                .map_err(|_| ConversationError::Unavailable)?;
            if project.user_id != actor.user_id
                || project.environment_id != env.id
                || project.state != ProjectState::Ready
            {
                return Err(ConversationError::Conflict);
            }
            Some(project)
        } else {
            None
        };
        self.same_environment(actor, &env).await?;
        Ok((env, conversation, project))
    }

    pub async fn current_environment(
        &self,
        actor: EnvironmentActor,
    ) -> Result<Environment, ConversationError> {
        self.environments
            .require_running(actor)
            .await
            .map_err(|error| match error {
                EnvironmentError::Unauthorized => ConversationError::Unauthorized,
                EnvironmentError::Conflict | EnvironmentError::PolicyMismatch => {
                    ConversationError::Conflict
                }
                _ => ConversationError::Unavailable,
            })
    }
    pub async fn has_open_runs(&self, actor: EnvironmentActor) -> Result<bool, ConversationError> {
        self.store.has_open_runs(actor).await
    }
    pub async fn create(&self, actor: EnvironmentActor) -> Result<Conversation, ConversationError> {
        let env = self.current_environment(actor).await?;
        self.store.create(actor, env.id, env.generation).await
    }
    pub async fn create_in_project(
        &self,
        actor: EnvironmentActor,
        project_id: Option<Uuid>,
    ) -> Result<Conversation, ConversationError> {
        if project_id.is_some() && self.projects.is_none() {
            return Err(ConversationError::Unavailable);
        }
        let env = self.current_environment(actor).await?;
        self.store
            .create_in_project(actor, env.id, env.generation, project_id)
            .await
    }
    pub async fn list(
        &self,
        actor: EnvironmentActor,
        before: Option<Uuid>,
    ) -> Result<ConversationPage, ConversationError> {
        let env = self.current_environment(actor).await?;
        self.store.list(actor, env.id, env.generation, before).await
    }
    pub async fn history(
        &self,
        actor: EnvironmentActor,
        id: Uuid,
    ) -> Result<ConversationHistory, ConversationError> {
        let env = self.current_environment(actor).await?;
        self.store.history(actor, env.id, env.generation, id).await
    }
    /// Resolve an owned conversation under the current environment.
    pub async fn get(
        &self,
        actor: EnvironmentActor,
        id: Uuid,
    ) -> Result<Conversation, ConversationError> {
        let env = self.current_environment(actor).await?;
        self.store.get(actor, env.id, env.generation, id).await
    }
    pub async fn find_turn(
        &self,
        actor: EnvironmentActor,
        conversation_id: Uuid,
        operation_id: Uuid,
        model: String,
        input: String,
        attachment_ids: Vec<Uuid>,
    ) -> Result<Option<Run>, ConversationError> {
        let env = self.current_environment(actor).await?;
        self.store
            .find_turn(
                actor,
                StartTurn {
                    conversation_id,
                    operation_id,
                    environment_id: env.id,
                    generation: env.generation,
                    model,
                    input,
                    attachment_ids,
                },
            )
            .await
    }
    /// Returns true only for a freshly reserved turn. The caller must not start the provider for an existing receipt.
    pub async fn begin_turn(
        &self,
        actor: EnvironmentActor,
        conversation_id: Uuid,
        operation_id: Uuid,
        model: String,
        input: String,
        attachment_ids: Vec<Uuid>,
    ) -> Result<(Environment, Run, bool), ConversationError> {
        let env = self.current_environment(actor).await?;
        let (run, created) = self
            .store
            .begin_turn(
                actor,
                StartTurn {
                    conversation_id,
                    operation_id,
                    environment_id: env.id,
                    generation: env.generation,
                    model,
                    input,
                    attachment_ids,
                },
            )
            .await?;
        Ok((env, run, created))
    }
    async fn same_environment(
        &self,
        actor: EnvironmentActor,
        expected: &Environment,
    ) -> Result<(), ConversationError> {
        let current = self.current_environment(actor).await?;
        if current.id != expected.id
            || current.generation != expected.generation
            || current.image_id != expected.image_id
            || current.policy_version != expected.policy_version
        {
            return Err(ConversationError::Conflict);
        }
        Ok(())
    }
    pub async fn set_thread(
        &self,
        actor: EnvironmentActor,
        env: &Environment,
        run_id: Uuid,
        thread: &str,
        previous: Option<&str>,
    ) -> Result<(), ConversationError> {
        self.same_environment(actor, env).await?;
        self.store
            .set_thread(actor, env.id, env.generation, run_id, thread, previous)
            .await
    }
    pub async fn append_assistant(
        &self,
        actor: EnvironmentActor,
        env: &Environment,
        run_id: Uuid,
        provider_item: &str,
        text: &str,
    ) -> Result<(Message, bool), ConversationError> {
        self.same_environment(actor, env).await?;
        self.store
            .append_assistant(actor, env.id, env.generation, run_id, provider_item, text)
            .await
    }
    pub async fn add_request(
        &self,
        actor: EnvironmentActor,
        env: &Environment,
        request: PendingRequest,
        provider_reference: &str,
    ) -> Result<(), ConversationError> {
        self.same_environment(actor, env).await?;
        self.store
            .add_request(actor, env.id, env.generation, request, provider_reference)
            .await
    }
    pub async fn claim_response(
        &self,
        actor: EnvironmentActor,
        env: &Environment,
        request_id: Uuid,
        operation_id: Uuid,
        answer: &str,
    ) -> Result<ResponseClaim, ConversationError> {
        self.same_environment(actor, env).await?;
        self.store
            .claim_response(
                actor,
                env.id,
                env.generation,
                request_id,
                operation_id,
                answer,
            )
            .await
    }
    pub async fn record_response(
        &self,
        actor: EnvironmentActor,
        env: &Environment,
        request_id: Uuid,
        operation_id: Uuid,
        state: ResponseState,
    ) -> Result<(), ConversationError> {
        self.same_environment(actor, env).await?;
        self.store
            .record_response(
                actor,
                env.id,
                env.generation,
                request_id,
                operation_id,
                state,
            )
            .await
    }
    /// Reserve the user’s durable slot before any model process can start.
    pub async fn reserve_model_probe(
        &self,
        actor: EnvironmentActor,
        env: &Environment,
        lease: Uuid,
    ) -> Result<(), ConversationError> {
        self.same_environment(actor, env).await?;
        self.store
            .reserve_model_probe(actor, env.id, env.generation, lease)
            .await
    }
    /// Trusted runtime callback: a Docker launch has conclusively settled.
    pub async fn settle_model_probe_launch(
        &self,
        lease: Uuid,
        env: &Environment,
    ) -> Result<(), ConversationError> {
        self.store
            .settle_model_probe_launch(lease, env.id, env.generation)
            .await
    }
    /// Trusted runtime callback; no browser endpoint can assert this proof.
    pub async fn settle_run_launch(
        &self,
        run: Uuid,
        env: &Environment,
    ) -> Result<(), ConversationError> {
        self.store
            .settle_run_launch(run, env.id, env.generation)
            .await
    }
    /// The caller must have proved the runtime process exited, or never started.
    pub async fn release_model_probe(
        &self,
        actor: EnvironmentActor,
        lease: Uuid,
    ) -> Result<(), ConversationError> {
        self.store.release_model_probe(actor, lease).await
    }
    /// Only a fresh stopped observation for a newer generation may release an uncertain probe.
    pub async fn recover_model_probe_stopped(
        &self,
        actor: EnvironmentActor,
    ) -> Result<bool, ConversationError> {
        let env = self
            .environments
            .get(actor)
            .await
            .map_err(|_| ConversationError::Unavailable)?
            .ok_or(ConversationError::Conflict)?;
        if !self.environments.can_report(Some(&env))
            || env.desired_state != DesiredState::Stopped
            || env.observed_state != ObservedState::Stopped
            || env.error_code.is_some()
        {
            return Err(ConversationError::Conflict);
        }
        let at = env.observed_at.ok_or(ConversationError::Conflict)?;
        let recovered = self
            .store
            .recover_model_probe_stopped(actor, env.id, env.generation, at)
            .await?;
        if !self.environments.is_available() {
            return Err(ConversationError::Unavailable);
        }
        Ok(recovered)
    }
    /// Read minimal recovery identifiers only with a fresh, verified stopped observation.
    pub async fn recoverable_stopped(
        &self,
        actor: EnvironmentActor,
    ) -> Result<Vec<RecoverableRun>, ConversationError> {
        let env = self
            .environments
            .get(actor)
            .await
            .map_err(|error| match error {
                EnvironmentError::Unauthorized => ConversationError::Unauthorized,
                _ => ConversationError::Unavailable,
            })?
            .ok_or(ConversationError::Conflict)?;
        if !self.environments.can_report(Some(&env))
            || env.desired_state != DesiredState::Stopped
            || env.observed_state != ObservedState::Stopped
            || env.error_code.is_some()
        {
            return Err(ConversationError::Conflict);
        }
        let observed_at = env.observed_at.ok_or(ConversationError::Conflict)?;
        let runs = self
            .store
            .recoverable_stopped(actor, env.id, env.generation, observed_at)
            .await?;
        if !self.environments.is_available() {
            return Err(ConversationError::Unavailable);
        }
        Ok(runs)
    }
    /// Explicit recovery needs a stopped observation made by the current supervisor.
    /// This releases local execution ownership; it cannot undo external side effects.
    pub async fn recover_stopped(
        &self,
        actor: EnvironmentActor,
        run_id: Uuid,
    ) -> Result<(), ConversationError> {
        let env = self
            .environments
            .get(actor)
            .await
            .map_err(|_| ConversationError::Unavailable)?
            .ok_or(ConversationError::Conflict)?;
        if !self.environments.can_report(Some(&env))
            || env.desired_state != DesiredState::Stopped
            || env.observed_state != ObservedState::Stopped
            || env.error_code.is_some()
        {
            return Err(ConversationError::Conflict);
        }
        let observed_at = env.observed_at.ok_or(ConversationError::Conflict)?;
        self.store
            .recover_stopped(actor, env.id, env.generation, observed_at, run_id)
            .await?;
        if !self.environments.is_available() {
            return Err(ConversationError::Unavailable);
        }
        Ok(())
    }
    pub async fn finish_turn(
        &self,
        actor: EnvironmentActor,
        env: &Environment,
        run_id: Uuid,
        status: RunStatus,
    ) -> Result<(), ConversationError> {
        self.same_environment(actor, env).await?;
        self.store
            .finish_turn(actor, env.id, env.generation, run_id, status)
            .await
    }
}
