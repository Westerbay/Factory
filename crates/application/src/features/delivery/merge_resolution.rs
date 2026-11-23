//! Orchestrate an explicit local merge resolution and its verified adoption.
use super::{FeatureDeliveryService, draft_read_error, workspace_error};
use crate::features::coordinator::{ReservedTask, TaskRequest, task_prompt};
use factory_core::{
    environments::EnvironmentActor,
    features::{
        AttemptStatus, AttemptUpdate, FeatureError, FeatureState, FeatureTask, GateContent,
        GitSnapshot, MergeResolutionCandidate, TaskAttempt, TaskRole, TaskWorkspace,
        WorkspaceAccess, WorkspaceError,
    },
    identity::RepositoryActor,
};
use std::sync::Arc;
use uuid::Uuid;

impl FeatureDeliveryService {
    /// Authorize a local writer for the currently blocked reviewed snapshot.
    pub async fn resolve_merge_conflict(
        self: &Arc<Self>,
        actor: EnvironmentActor,
        feature_id: Uuid,
        operation_id: Uuid,
        snapshot_id: Uuid,
    ) -> Result<FeatureState, FeatureError> {
        if operation_id.is_nil() || snapshot_id.is_nil() || feature_id.is_nil() {
            return Err(FeatureError::Invalid);
        }
        let lock = self.lock_for(feature_id)?;
        let _serial = lock.lock().await;
        let (env, project, state) = self.features.context(actor, feature_id).await?;
        if let Some(attempt) = state
            .attempts
            .iter()
            .find(|attempt| attempt.id() == operation_id)
        {
            return if attempt.merge_resolution.is_some()
                && attempt.claim.snapshot_id == Some(snapshot_id)
            {
                Ok(state)
            } else {
                Err(FeatureError::Conflict)
            };
        }
        let review = state
            .review()
            .filter(|review| review.snapshot.id == snapshot_id)
            .ok_or(FeatureError::StaleSnapshot)?;
        if !state.can_resolve_merge_conflict(snapshot_id) {
            return Err(FeatureError::Conflict);
        }
        let architecture = state.current_architecture().ok_or(FeatureError::Conflict)?;
        let GateContent::Architecture { tasks, .. } = &architecture.content else {
            return Err(FeatureError::Conflict);
        };
        let mut paths: Vec<_> = tasks
            .iter()
            .flat_map(|task| task.paths.iter().cloned())
            .collect();
        paths.sort_unstable();
        paths.dedup();
        if paths.is_empty() {
            return Err(FeatureError::MergeResolutionUnsupported);
        }
        let reviewed = &review.snapshot.git;
        let workspace = TaskWorkspace {
            feature_id,
            attempt_id: operation_id,
            base_oid: reviewed.head_oid.clone(),
            access: WorkspaceAccess::Write,
        };
        let access = self
            .repository
            .draft_access(
                RepositoryActor {
                    actor,
                    environment_id: env.id,
                    generation: env.generation,
                },
                &project,
            )
            .await
            .map_err(draft_read_error)?;
        let candidate = self
            .git
            .stage_merge_resolution(&env, &project, &workspace, reviewed, &paths)
            .await;
        // A conflict observed against a substituted origin must not be reported.
        access.recheck().await.map_err(draft_read_error)?;
        let candidate = candidate.map_err(resolution_error)?;
        let task = FeatureTask {
            id: operation_id,
            title: "Resolve merge conflicts".into(),
            instructions: "The checkout contains an automatic merge with conflict markers. Inspect its two parent revisions to preserve the feature and target changes. Edit only the listed conflicting files. Do not remove files, change file modes, commit, push, or publish. Leave unsupported conflicts unresolved and report them. Factory will test and review the result.".into(),
            role: TaskRole::Correction,
            paths: candidate.paths.clone(),
            depends_on: Vec::new(),
            checks: Vec::new(),
        };
        let input = task_prompt(&state, &task)?;
        let request = TaskRequest {
            operation_id,
            attempt_id: operation_id,
            task,
            revision: Some(architecture.key.clone()),
            snapshot_id: Some(snapshot_id),
            base_oid: workspace.base_oid,
            retry_of: None,
            input,
        };
        let reserved = self
            .features
            .reserve_task_with_resolution(actor, &state, request, Some((&env, &project, candidate)))
            .await?;
        let delivery = self.clone();
        let reviewed = reviewed.clone();
        self.features.conversations.spawn(Box::pin(async move {
            delivery.run_resolution(reserved, reviewed).await;
        }));
        self.features.get(actor, feature_id).await
    }

    async fn run_resolution(&self, reserved: ReservedTask, reviewed: GitSnapshot) {
        let actor = reserved.actor;
        let env = reserved.env.clone();
        let project = reserved.project.clone();
        let attempt = if reserved.fresh {
            let Ok(result) = self
                .features
                .run_reserved_with_delivery(reserved, Some(self))
                .await
            else {
                return;
            };
            result.attempt
        } else {
            reserved.attempt
        };
        if attempt.status == AttemptStatus::Validated {
            self.features
                .schedule(actor, attempt.claim.workspace.feature_id);
        }
        if scratch_can_be_discarded(&attempt) {
            let _ = self
                .git
                .discard_merge_resolution(&env, &project, &attempt.claim.workspace, &reviewed)
                .await;
        }
    }

    /// Reclaim an interrupted worker's scratch only after exact recovery settled it.
    pub fn cleanup_recovered_resolution(
        self: &Arc<Self>,
        actor: EnvironmentActor,
        state: &FeatureState,
        attempt_id: Uuid,
    ) {
        let Some(attempt) = state.attempts.iter().find(|attempt| {
            attempt.id() == attempt_id
                && attempt.status == AttemptStatus::Interrupted
                && scratch_can_be_discarded(attempt)
        }) else {
            return;
        };
        let Some(review) = state
            .reviews
            .iter()
            .find(|review| Some(review.snapshot.id) == attempt.claim.snapshot_id)
        else {
            return;
        };
        let workspace = attempt.claim.workspace.clone();
        let reviewed = review.snapshot.git.clone();
        let delivery = self.clone();
        self.features.conversations.spawn(Box::pin(async move {
            if let Ok((env, project, _)) =
                delivery.features.context(actor, workspace.feature_id).await
            {
                let _ = delivery
                    .git
                    .discard_merge_resolution(&env, &project, &workspace, &reviewed)
                    .await;
            }
        }));
    }

    pub(in crate::features) async fn finish_merge_resolution(
        &self,
        task: &ReservedTask,
        candidate: &MergeResolutionCandidate,
    ) -> AttemptUpdate {
        match self.adopt_resolution(task, candidate).await {
            Ok(checkpoint) => AttemptUpdate::Validated {
                checkpoint: Some(checkpoint),
            },
            Err(FeatureError::ExecutionUncertain) => AttemptUpdate::Uncertain,
            Err(error) => AttemptUpdate::Failed {
                reason: error.to_string(),
            },
        }
    }

    async fn adopt_resolution(
        &self,
        task: &ReservedTask,
        candidate: &MergeResolutionCandidate,
    ) -> Result<String, FeatureError> {
        let workspace = &task.attempt.claim.workspace;
        let feature_id = workspace.feature_id;
        let lock = self.lock_for(feature_id)?;
        let _serial = lock.lock().await;
        let (env, project, state) = self.features.context(task.actor, feature_id).await?;
        if env.id != task.env.id
            || env.generation != task.env.generation
            || project.id != task.project.id
        {
            return Err(FeatureError::StaleSnapshot);
        }
        let review = state
            .review()
            .filter(|review| Some(review.snapshot.id) == task.attempt.claim.snapshot_id)
            .ok_or(FeatureError::StaleSnapshot)?;
        let access = self
            .repository
            .draft_access(
                RepositoryActor {
                    actor: task.actor,
                    environment_id: env.id,
                    generation: env.generation,
                },
                &project,
            )
            .await
            .map_err(draft_read_error)?;
        access.recheck().await.map_err(draft_read_error)?;
        let reviewed = &review.snapshot.git;
        let mut adopted = self
            .git
            .adopt_merge_resolution(&env, &project, workspace, reviewed, candidate)
            .await;
        if adopted == Err(WorkspaceError::Uncertain) && access.recheck().await.is_ok() {
            // The controller can acknowledge the exact already-adopted tree.
            let retry = self
                .git
                .adopt_merge_resolution(&env, &project, workspace, reviewed, candidate)
                .await;
            if retry.is_ok() {
                adopted = retry;
            }
        }
        adopted
            .map(|diff| diff.git.head_oid)
            .map_err(resolution_error)
    }
}

fn scratch_can_be_discarded(attempt: &TaskAttempt) -> bool {
    attempt.merge_resolution.is_some()
        && attempt.launch_settled
        && !attempt.status.holds_reservation()
}

fn resolution_error(error: WorkspaceError) -> FeatureError {
    match error {
        WorkspaceError::MergeConflict
        | WorkspaceError::ScopeViolation
        | WorkspaceError::DiffUnrenderable => FeatureError::MergeResolutionUnsupported,
        WorkspaceError::Conflict => FeatureError::StaleSnapshot,
        WorkspaceError::Invalid => FeatureError::Unavailable,
        other => workspace_error(other),
    }
}
