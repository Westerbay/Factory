use super::{
    FeatureCoordinator, conversation_error,
    coordinator::{TaskRequest, task_prompt},
    discussion::architecture_request,
    review::{correction_request, review_input},
    workspace_error,
};
use factory_core::{
    conversations::RunStatus,
    environments::EnvironmentActor,
    features::{
        AttemptStatus, FeatureCommand, FeatureError, FeatureState, TaskObservation, TaskRole,
    },
};
use std::sync::Arc;
use uuid::Uuid;

impl FeatureCoordinator {
    pub async fn resume(
        self: &Arc<Self>,
        actor: EnvironmentActor,
        id: Uuid,
        operation_id: Uuid,
        attempt_id: Uuid,
    ) -> Result<FeatureState, FeatureError> {
        if operation_id.is_nil() || attempt_id.is_nil() || operation_id == attempt_id {
            return Err(FeatureError::Invalid);
        }
        let (env, project, mut state) = self.context(actor, id).await?;
        if let Some(replay) = state
            .attempts
            .iter()
            .find(|attempt| attempt.id() == operation_id)
        {
            return if replay.claim.retry_of == Some(attempt_id) {
                Ok(state)
            } else {
                Err(FeatureError::Conflict)
            };
        }
        let mut prior = state
            .attempts
            .iter()
            .find(|attempt| attempt.id() == attempt_id)
            .ok_or(FeatureError::NotFound)?
            .clone();
        if prior.status.holds_reservation() {
            if self.is_attempt_locally_active(id, attempt_id)
                || self.runtime.reconcile_task(&env, &project, &prior).await
                    != Ok(TaskObservation::Stopped)
            {
                return Err(FeatureError::ExecutionUncertain);
            }
            // Close the visible conversation only when it was bound to this
            // attempt in the current environment. The existing Stop recovery
            // handles older conversation generations.
            let history = self
                .conversations
                .service
                .history(actor, state.feature.conversation_id)
                .await
                .map_err(conversation_error)?;
            if let Some(run) = history.runs.iter().find(|run| {
                run.operation_id == prior.id()
                    && run.environment_generation == env.generation
                    && matches!(run.status, RunStatus::Active | RunStatus::Uncertain)
            }) {
                self.conversations
                    .service
                    .finish_turn(actor, &env, run.id, RunStatus::Interrupted)
                    .await
                    .map_err(conversation_error)?;
            }
            state = self
                .store
                .apply(
                    actor,
                    &env,
                    id,
                    Uuid::new_v4(),
                    FeatureCommand::RecoverAttempt {
                        attempt_id,
                        launch_generation: prior.generation,
                    },
                )
                .await?
                .state;
            prior = state
                .attempts
                .iter()
                .find(|attempt| attempt.id() == attempt_id)
                .ok_or(FeatureError::Conflict)?
                .clone();
        }
        if !matches!(
            prior.status,
            AttemptStatus::Failed | AttemptStatus::Interrupted
        ) || !prior.launch_settled
        {
            return Err(FeatureError::Conflict);
        }

        let claim = &prior.claim;
        if claim.task.title == "Feature discussion" {
            return self
                .resume_discussion(actor, &state, operation_id, prior)
                .await;
        }
        let input = if let Some(snapshot_id) = claim.snapshot_id {
            let round = state
                .review()
                .filter(|round| round.snapshot.id == snapshot_id)
                .ok_or(FeatureError::StaleSnapshot)?;
            if claim.task.role == TaskRole::Correction {
                let verdict = round.verdict.as_ref().ok_or(FeatureError::ReviewBlocked)?;
                correction_request(self.codec.as_ref(), &state, round, verdict)?.input
            } else {
                let diff = self
                    .workspaces
                    .snapshot(&env, &project, id)
                    .await
                    .map_err(workspace_error)?;
                if diff.git != round.snapshot.git {
                    return Err(FeatureError::StaleSnapshot);
                }
                review_input(&diff)
            }
        } else if matches!(
            claim.task.role,
            TaskRole::Architecture | TaskRole::ArchitectureReview
        ) {
            let (_, revision, input) =
                architecture_request(self.codec.as_ref(), &state, claim.task.role)?;
            if claim.revision.as_ref() != Some(&revision) {
                return Err(FeatureError::Conflict);
            }
            input
        } else {
            task_prompt(&state, &claim.task)?
        };
        let request = TaskRequest {
            operation_id,
            attempt_id: operation_id,
            task: claim.task.clone(),
            revision: claim.revision.clone(),
            snapshot_id: claim.snapshot_id,
            base_oid: prior
                .checkpoint
                .clone()
                .unwrap_or_else(|| claim.workspace.base_oid.clone()),
            retry_of: Some(attempt_id),
            input,
        };
        let reserved = self.reserve_task(actor, &state, request).await?;
        self.spawn_reserved(reserved);
        self.store.get(actor, id).await
    }
}
