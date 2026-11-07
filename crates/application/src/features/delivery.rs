use super::{FeatureCoordinator, workspace_error};
use crate::identity::RepositoryAccessService;
use factory_core::{
    environments::{Environment, EnvironmentActor},
    features::{
        DeliveryStatus, FeatureCommand, FeatureDeliveryRuntime, FeatureError, FeatureRemoteRefs,
        FeatureStage, FeatureState, GitSnapshot, WorkspaceDiff, WorkspaceError,
    },
    identity::{
        RepositoryAccessError, RepositoryActor, RepositoryDraft, RepositoryDraftDiffTarget,
        RepositoryDraftTarget,
    },
    projects::Project,
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, Weak},
};
use tokio::sync::Mutex as AsyncMutex;
use uuid::Uuid;

pub struct FeatureDeliveryService {
    features: Arc<FeatureCoordinator>,
    git: Arc<dyn FeatureDeliveryRuntime>,
    repository: Arc<RepositoryAccessService>,
    locks: Mutex<HashMap<Uuid, Weak<AsyncMutex<()>>>>,
}

struct ClaimedPublication<'a> {
    actor: EnvironmentActor,
    env: &'a Environment,
    project: &'a Project,
    feature_id: Uuid,
    operation_id: Uuid,
    title: &'a str,
    approved: &'a GitSnapshot,
    current: &'a GitSnapshot,
    recovering: bool,
}

enum Publication {
    Published(RepositoryDraft),
    PublishedStale(RepositoryDraft),
    Stale { refresh_base: bool },
    Failed(&'static str),
    Uncertain,
}

enum RemoteBranch {
    NeedsPush,
    Ready,
}

fn verified_remote(
    remote: &FeatureRemoteRefs,
    approved: &GitSnapshot,
    unproven_push: bool,
) -> Result<RemoteBranch, Publication> {
    if remote.head_oid.is_some() && remote.head_oid != remote.managed_head_oid {
        return Err(if unproven_push {
            Publication::Uncertain
        } else {
            Publication::Failed("The existing remote feature branch is not owned by Factory.")
        });
    }
    if remote.base_oid.as_deref() != Some(&approved.base_oid) {
        return Err(Publication::Stale { refresh_base: true });
    }
    Ok(if remote.head_oid.as_deref() == Some(&approved.head_oid) {
        RemoteBranch::Ready
    } else {
        RemoteBranch::NeedsPush
    })
}

impl FeatureDeliveryService {
    pub fn new(
        features: Arc<FeatureCoordinator>,
        git: Arc<dyn FeatureDeliveryRuntime>,
        repository: Arc<RepositoryAccessService>,
    ) -> Arc<Self> {
        Arc::new(Self {
            features,
            git,
            repository,
            locks: Mutex::new(HashMap::new()),
        })
    }

    /// A stale draft has its own remote diff. Never substitute a local review patch
    /// when the provider cannot confirm the current PR.
    pub async fn published_stale_diff(
        &self,
        actor: EnvironmentActor,
        feature_id: Uuid,
    ) -> Result<Option<WorkspaceDiff>, FeatureError> {
        let state = self.features.get(actor, feature_id).await?;
        let Some((receipt, draft_url)) =
            state
                .deliveries
                .iter()
                .rev()
                .find_map(|receipt| match &receipt.status {
                    DeliveryStatus::PublishedStale { draft_url } => {
                        Some((receipt, draft_url.as_str()))
                    }
                    _ => None,
                })
        else {
            return Ok(None);
        };
        let (env, project, _) = self.features.context(actor, feature_id).await?;
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
        let diff = access
            .draft_diff(&RepositoryDraftDiffTarget {
                repository_id: project.repository_id,
                full_name: &project.cloned_full_name,
                feature_id,
                operation_id: receipt.request_id,
                draft_url,
            })
            .await
            .map_err(draft_read_error)?;
        Ok(Some(WorkspaceDiff {
            git: GitSnapshot {
                base_oid: diff.base_oid,
                head_oid: diff.head_oid,
                patch_digest: diff.patch_digest,
            },
            patch: diff.patch,
            changed_paths: diff.changed_paths,
        }))
    }

    /// Retry a local comparison refresh before the normal review command. This
    /// applies only to the unchanged ready snapshot left by a failed publication.
    pub async fn refresh_stale_comparison(
        &self,
        actor: EnvironmentActor,
        feature_id: Uuid,
        snapshot_id: Uuid,
    ) -> Result<(), FeatureError> {
        let lock = self.lock_for(feature_id)?;
        let _serial = lock.lock().await;
        let (env, project, state) = self.features.context(actor, feature_id).await?;
        let Some(review) = state.review().filter(|review| {
            state.feature.stage == FeatureStage::Review
                && review.snapshot.id == snapshot_id
                && review
                    .verdict
                    .as_ref()
                    .is_some_and(|verdict| verdict.ready())
                && state.deliveries.last().is_some_and(|receipt| {
                    receipt.snapshot_id == snapshot_id
                        && matches!(receipt.status, DeliveryStatus::Failed { .. })
                })
        }) else {
            return Ok(());
        };
        let approved = &review.snapshot.git;
        let current = self
            .features
            .workspaces
            .snapshot(&env, &project, feature_id)
            .await
            .map_err(workspace_error)?;
        if current.git != *approved
            || self
                .refresh_stale_base(actor, &env, &project, feature_id, approved)
                .await
        {
            Ok(())
        } else {
            Err(FeatureError::StaleSnapshot)
        }
    }

    /// Publish only after a separate authenticated request names the reviewed snapshot.
    /// Reusing the operation ID reconciles the same branch and draft.
    pub async fn publish(
        &self,
        actor: EnvironmentActor,
        feature_id: Uuid,
        operation_id: Uuid,
        snapshot_id: Uuid,
    ) -> Result<FeatureState, FeatureError> {
        if feature_id.is_nil() || operation_id.is_nil() || snapshot_id.is_nil() {
            return Err(FeatureError::Invalid);
        }
        // Serialize retries in this service; a restart is fenced by the durable claim
        // and the exact branch/PR identity checked before either external write.
        let lock = self.lock_for(feature_id)?;
        let _serial = lock.lock().await;
        let (env, project, state) = self.features.context(actor, feature_id).await?;
        let prior = state
            .deliveries
            .iter()
            .find(|receipt| receipt.request_id == operation_id);
        if let Some(prior) = prior {
            if prior.snapshot_id != snapshot_id {
                return Err(FeatureError::Conflict);
            }
            if !matches!(
                prior.status,
                DeliveryStatus::Claimed | DeliveryStatus::Uncertain
            ) {
                return Ok(state);
            }
        }
        let approved = &state
            .review()
            .filter(|review| review.snapshot.id == snapshot_id)
            .ok_or(FeatureError::StaleSnapshot)?
            .snapshot
            .git;
        let current = self
            .features
            .workspaces
            .snapshot(&env, &project, feature_id)
            .await
            .map_err(workspace_error)?;
        let recovering = if prior.is_some() {
            true
        } else {
            if &current.git != approved {
                self.invalidate(actor, &env, feature_id, snapshot_id)
                    .await?;
                self.features.schedule(actor, feature_id);
                return Err(FeatureError::StaleSnapshot);
            }
            let receipt = self
                .features
                .store
                .apply(
                    actor,
                    &env,
                    feature_id,
                    operation_id,
                    FeatureCommand::ClaimDelivery {
                        snapshot_id,
                        observed: current.git.clone(),
                    },
                )
                .await?;
            !receipt.fresh
        };
        let publication = ClaimedPublication {
            actor,
            env: &env,
            project: &project,
            feature_id,
            operation_id,
            title: &state.feature.title,
            approved,
            current: &current.git,
            recovering,
        };
        let outcome = self.perform(&publication).await;
        let (status, stale) = match outcome {
            Publication::Published(draft) => (
                DeliveryStatus::Published {
                    draft_url: draft.url,
                },
                None,
            ),
            Publication::PublishedStale(draft) => (
                DeliveryStatus::PublishedStale {
                    draft_url: draft.url,
                },
                None,
            ),
            Publication::Stale { refresh_base } => {
                let refreshed = if refresh_base {
                    self.refresh_stale_base(actor, &env, &project, feature_id, approved)
                        .await
                } else {
                    &current.git != approved
                };
                let reason = if refreshed {
                    "The reviewed diff changed. Review the current result before publishing."
                } else {
                    "The reviewed diff is stale. Factory could not prepare the current comparison."
                };
                (
                    DeliveryStatus::Failed {
                        reason: reason.into(),
                    },
                    Some(refreshed),
                )
            }
            Publication::Failed(reason) => (
                DeliveryStatus::Failed {
                    reason: reason.into(),
                },
                None,
            ),
            Publication::Uncertain => (DeliveryStatus::Uncertain, None),
        };
        let receipt = self
            .features
            .store
            .apply(
                actor,
                &env,
                feature_id,
                Uuid::new_v4(),
                FeatureCommand::RecordDelivery {
                    request_id: operation_id,
                    status,
                },
            )
            .await?;
        if let Some(refreshed) = stale {
            self.invalidate(actor, &env, feature_id, snapshot_id)
                .await?;
            if refreshed {
                self.features.schedule(actor, feature_id);
            }
            return Err(FeatureError::StaleSnapshot);
        }
        Ok(receipt.state)
    }

    async fn refresh_stale_base(
        &self,
        actor: EnvironmentActor,
        env: &Environment,
        project: &Project,
        feature_id: Uuid,
        approved: &GitSnapshot,
    ) -> bool {
        let repository_actor = RepositoryActor {
            actor,
            environment_id: env.id,
            generation: env.generation,
        };
        if self
            .repository
            .draft_access(repository_actor, project)
            .await
            .is_err()
        {
            return false;
        }
        // Hold the delivery claim while refreshing the private comparison.
        // The Git controller also revalidates any existing remote head's receipt.
        match self
            .git
            .refresh_base(env, project, feature_id, approved)
            .await
        {
            Ok(diff) => diff.git != *approved,
            Err(WorkspaceError::Uncertain) => self
                .features
                .workspaces
                .snapshot(env, project, feature_id)
                .await
                .is_ok_and(|diff| diff.git != *approved),
            Err(_) => false,
        }
    }

    async fn invalidate(
        &self,
        actor: EnvironmentActor,
        env: &Environment,
        feature_id: Uuid,
        snapshot_id: Uuid,
    ) -> Result<(), FeatureError> {
        let state = self.features.store.get(actor, feature_id).await?;
        if state.feature.stage == FeatureStage::Result {
            self.features
                .store
                .apply(
                    actor,
                    env,
                    feature_id,
                    Uuid::new_v4(),
                    FeatureCommand::InvalidateResult { snapshot_id },
                )
                .await?;
        }
        Ok(())
    }

    async fn perform(&self, request: &ClaimedPublication<'_>) -> Publication {
        let actor = RepositoryActor {
            actor: request.actor,
            environment_id: request.env.id,
            generation: request.env.generation,
        };
        let access = match self.repository.draft_access(actor, request.project).await {
            Ok(access) => access,
            Err(error) => return access_failure(error, request.recovering),
        };
        let branch = match self
            .features
            .initialize_verified(
                request.actor,
                request.env,
                request.project,
                request.feature_id,
            )
            .await
        {
            Ok(branch) => branch,
            Err(FeatureError::Unauthorized) => {
                return access_failure(RepositoryAccessError::Unauthorized, request.recovering);
            }
            Err(FeatureError::ExecutionUncertain) => return Publication::Uncertain,
            Err(_) if request.recovering => return Publication::Uncertain,
            Err(_) => return Publication::Failed("The feature branch could not be verified."),
        };
        let (Some(base), Some(head)) = (
            branch.base_ref.strip_prefix("refs/heads/"),
            branch.head_ref.strip_prefix("refs/heads/"),
        ) else {
            return Publication::Failed("The feature branch could not be verified.");
        };
        let target = RepositoryDraftTarget {
            repository_id: request.project.repository_id,
            full_name: &request.project.cloned_full_name,
            base,
            head,
            head_oid: &request.approved.head_oid,
            operation_id: request.operation_id,
        };
        match access.find_draft(&target).await {
            Ok(Some(draft)) => return self.classify_draft(request, draft).await,
            Ok(None) => {}
            Err(error) => return access_failure(error, request.recovering),
        }
        let remote = match self
            .git
            .remote(request.env, request.project, request.feature_id)
            .await
        {
            Ok(remote) => remote,
            Err(error) => return git_failure(error, request.recovering),
        };
        if request.current != request.approved {
            return Publication::Stale {
                refresh_base: false,
            };
        }
        match verified_remote(&remote, request.approved, request.recovering) {
            Err(outcome) => return outcome,
            Ok(RemoteBranch::Ready) => {}
            Ok(RemoteBranch::NeedsPush) => {
                if let Err(error) = access.recheck().await {
                    return access_failure(error, request.recovering);
                }
                let push = self
                    .git
                    .push_reviewed(
                        request.env,
                        request.project,
                        request.feature_id,
                        request.approved,
                    )
                    .await;
                let after = match self
                    .git
                    .remote(request.env, request.project, request.feature_id)
                    .await
                {
                    Ok(remote) => remote,
                    Err(_) => return Publication::Uncertain,
                };
                let unproven_push = request.recovering || push == Err(WorkspaceError::Uncertain);
                match verified_remote(&after, request.approved, unproven_push) {
                    Err(outcome) => return outcome,
                    Ok(RemoteBranch::Ready) => {}
                    Ok(RemoteBranch::NeedsPush) => {
                        return match push {
                            Err(error) => git_failure(error, request.recovering),
                            Ok(()) => Publication::Uncertain,
                        };
                    }
                }
            }
        }
        // GitHub's draft request names branches, not commit IDs. Check both refs
        // immediately before it and verify the returned draft before recording success.
        if let Err(error) = access.recheck().await {
            return access_failure(error, request.recovering);
        }
        match self
            .features
            .workspaces
            .snapshot(request.env, request.project, request.feature_id)
            .await
        {
            Ok(diff) if diff.git == *request.approved => {}
            Ok(_) => {
                return Publication::Stale {
                    refresh_base: false,
                };
            }
            Err(_) => return Publication::Uncertain,
        }
        let before = match self
            .git
            .remote(request.env, request.project, request.feature_id)
            .await
        {
            Ok(remote) => remote,
            Err(_) => return Publication::Uncertain,
        };
        match verified_remote(&before, request.approved, request.recovering) {
            Err(outcome) => return outcome,
            Ok(RemoteBranch::NeedsPush) => {
                return Publication::Failed(
                    "The remote feature branch changed before draft creation.",
                );
            }
            Ok(RemoteBranch::Ready) => {}
        }
        let title: String = request
            .title
            .chars()
            .map(|character| {
                if character.is_control() {
                    ' '
                } else {
                    character
                }
            })
            .collect();
        match access.create_draft(&target, &title).await {
            Ok(draft) => self.classify_draft(request, draft).await,
            Err(error) => match access.find_draft(&target).await {
                Ok(Some(draft)) => self.classify_draft(request, draft).await,
                Ok(None) => access_failure(
                    error,
                    request.recovering || error == RepositoryAccessError::Unavailable,
                ),
                Err(_) => Publication::Uncertain,
            },
        }
    }

    async fn classify_draft(
        &self,
        request: &ClaimedPublication<'_>,
        draft: RepositoryDraft,
    ) -> Publication {
        let current = self
            .features
            .workspaces
            .snapshot(request.env, request.project, request.feature_id)
            .await;
        let remote = self
            .git
            .remote(request.env, request.project, request.feature_id)
            .await;
        if draft.base_oid != request.approved.base_oid
            || draft.head_oid != request.approved.head_oid
            || current
                .as_ref()
                .is_ok_and(|diff| diff.git != *request.approved)
            || remote.as_ref().is_ok_and(|refs| {
                !matches!(
                    verified_remote(refs, request.approved, request.recovering),
                    Ok(RemoteBranch::Ready)
                )
            })
        {
            return Publication::PublishedStale(draft);
        }
        match (current, remote) {
            (Ok(_), Ok(_)) => Publication::Published(draft),
            _ => Publication::Uncertain,
        }
    }

    fn lock_for(&self, feature_id: Uuid) -> Result<Arc<AsyncMutex<()>>, FeatureError> {
        let mut locks = self.locks.lock().map_err(|_| FeatureError::Unavailable)?;
        locks.retain(|_, lock| lock.strong_count() > 0);
        if let Some(lock) = locks.get(&feature_id).and_then(Weak::upgrade) {
            return Ok(lock);
        }
        let lock = Arc::new(AsyncMutex::new(()));
        locks.insert(feature_id, Arc::downgrade(&lock));
        Ok(lock)
    }
}

fn draft_read_error(error: RepositoryAccessError) -> FeatureError {
    match error {
        RepositoryAccessError::Unauthorized => FeatureError::Unauthorized,
        RepositoryAccessError::Conflict => FeatureError::StaleSnapshot,
        _ => FeatureError::Unavailable,
    }
}

fn access_failure(error: RepositoryAccessError, recovering: bool) -> Publication {
    use RepositoryAccessError::*;
    match error {
        Conflict => Publication::Failed("The repository or draft identity could not be verified."),
        _ if recovering => Publication::Uncertain,
        Unauthorized | ProviderUnauthorized | InvalidLogin => {
            Publication::Failed("Reconnect GitHub before publishing the reviewed draft.")
        }
        Forbidden | ProviderForbidden => {
            Publication::Failed("GitHub denied permission to create the draft.")
        }
        RateLimited => {
            Publication::Failed("GitHub rate limited the draft request. Try again later.")
        }
        Unavailable => Publication::Failed("GitHub could not be reached before publishing."),
        _ => Publication::Failed("GitHub could not accept this draft request."),
    }
}

fn git_failure(error: WorkspaceError, recovering: bool) -> Publication {
    match error {
        WorkspaceError::Uncertain => Publication::Uncertain,
        WorkspaceError::Unavailable if recovering => Publication::Uncertain,
        WorkspaceError::Conflict => Publication::Failed("The Git refs could not be verified."),
        _ => Publication::Failed("GitHub did not accept the reviewed branch."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_base_even_when_the_reviewed_head_is_already_pushed() {
        let approved = GitSnapshot {
            base_oid: "a".repeat(40),
            head_oid: "b".repeat(40),
            patch_digest: "c".repeat(64),
        };
        let remote = FeatureRemoteRefs {
            base_oid: Some("d".repeat(40)),
            head_oid: Some(approved.head_oid.clone()),
            managed_head_oid: Some(approved.head_oid.clone()),
        };
        assert!(matches!(
            verified_remote(&remote, &approved, false),
            Err(Publication::Stale { refresh_base: true })
        ));
        let unowned = FeatureRemoteRefs {
            managed_head_oid: None,
            ..remote
        };
        assert!(matches!(
            verified_remote(&unowned, &approved, false),
            Err(Publication::Failed(_))
        ));
        assert!(matches!(
            verified_remote(&unowned, &approved, true),
            Err(Publication::Uncertain)
        ));
    }
}
