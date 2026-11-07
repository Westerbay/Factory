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
        WorkspaceAccess, WorkspaceError,
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
        let reconciled = prior.status.holds_reservation();
        if reconciled {
            if self.is_attempt_locally_active(id, attempt_id) {
                return Err(FeatureError::ExecutionUncertain);
            }
            // A pending check has its own launch. The provider's settlement
            // does not prove that an unseen check launch failed to reach Docker.
            let mut worker = prior.clone();
            if let Some(check_id) = prior.pending_verification {
                worker.claim.workspace.attempt_id = check_id;
                worker.claim.workspace.access = WorkspaceAccess::Write;
                worker.launch_settled = false;
                worker.process_receipt = None;
            }
            let observation = self.runtime.reconcile_task(&env, &project, &worker).await;
            let command = match observation {
                Ok(TaskObservation::StoppedWithWorkerProof { worker_id }) => {
                    FeatureCommand::RecoverObservedAttempt {
                        attempt_id,
                        launch_generation: prior.generation,
                        worker_id,
                    }
                }
                Ok(TaskObservation::Stopped) if prior.pending_verification.is_none() => {
                    FeatureCommand::RecoverAttempt {
                        attempt_id,
                        launch_generation: prior.generation,
                    }
                }
                _ => return Err(FeatureError::ExecutionUncertain),
            };
            let observed_worker = matches!(&command, FeatureCommand::RecoverObservedAttempt { .. });
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
                if observed_worker {
                    self.conversations
                        .service
                        .settle_run_launch(run.id, &env)
                        .await
                        .map_err(conversation_error)?;
                }
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
                    self.codec.derived_id(attempt_id, "recover-stopped"),
                    command,
                )
                .await?
                .state;
            if observed_worker {
                // Retain the proof until the durable recovery above succeeds.
                worker.launch_settled = true;
                worker.pending_verification = None;
                let _ = self.runtime.reconcile_task(&env, &project, &worker).await;
            }
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

        if prior.claim.workspace.access == WorkspaceAccess::Write && prior.checkpoint.is_none() {
            if !reconciled
                && (self.is_attempt_locally_active(id, attempt_id)
                    || !matches!(
                        self.runtime.reconcile_task(&env, &project, &prior).await,
                        Ok(TaskObservation::Stopped
                            | TaskObservation::StoppedWithWorkerProof { .. })
                    ))
            {
                return Err(FeatureError::ExecutionUncertain);
            }
            match self.workspaces.checkpoint(&env, &project, &prior).await {
                Ok(base_oid) if base_oid != prior.claim.workspace.base_oid => {
                    state = self
                        .store
                        .apply(
                            actor,
                            &env,
                            id,
                            self.codec.derived_id(attempt_id, "retained-retry-base"),
                            FeatureCommand::SaveRetryBase {
                                attempt_id,
                                base_oid,
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
                Err(WorkspaceError::Uncertain) => return Err(FeatureError::ExecutionUncertain),
                // Missing or unsafe retained work stays in its original checkout.
                // Retry the approved base while leaving the failure intact.
                _ => {}
            }
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
                review_input(&state, &diff)?
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
