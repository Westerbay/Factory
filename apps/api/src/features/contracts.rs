use factory_core::features::{
    AttemptStatus, DeliveryReceipt, DeliveryStatus, Feature, FeaturePage, FeatureStage,
    FeatureState, GateContent, IntegratedCheck, IntegratedCheckStatus, ReviewVerdict, RevisionKey,
    TaskAttempt, TaskRole, WorkspaceDiff,
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum FeatureProvider {
    Codex,
    Claude,
}

impl FeatureProvider {
    pub(super) fn id(&self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum FeatureLocale {
    En,
    Fr,
}

impl FeatureLocale {
    pub(super) fn language(&self) -> &'static str {
        match self {
            Self::En => "en",
            Self::Fr => "fr",
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateFeatureBody {
    pub operation_id: Uuid,
    pub project_id: Uuid,
    pub provider: FeatureProvider,
    pub title: String,
    pub locale: FeatureLocale,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FeatureTurnBody {
    pub operation_id: Uuid,
    pub input: String,
    /// Staged uploads of the feature's conversation to send with the message, at most 10.
    #[serde(default)]
    pub attachment_ids: Vec<Uuid>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FeatureRevisionBody {
    pub operation_id: Uuid,
    pub revision_id: Uuid,
    pub digest: String,
}

impl FeatureRevisionBody {
    pub(super) fn into_revision(self) -> RevisionKey {
        RevisionKey {
            id: self.revision_id,
            digest: self.digest,
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FeatureResumeBody {
    pub operation_id: Uuid,
    /// Omit to continue the current workflow stage without retrying an attempt.
    pub attempt_id: Option<Uuid>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FeatureSnapshotBody {
    pub operation_id: Uuid,
    pub snapshot_id: Uuid,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct FeatureListQuery {
    pub project_id: Option<Uuid>,
    pub before: Option<Uuid>,
}

#[derive(Clone, Copy, Debug, Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum FeatureStageView {
    Scope,
    Architecture,
    Execution,
    Review,
    Result,
    Draft,
}

impl From<FeatureStage> for FeatureStageView {
    fn from(value: FeatureStage) -> Self {
        match value {
            FeatureStage::Scope => Self::Scope,
            FeatureStage::Architecture => Self::Architecture,
            FeatureStage::Execution => Self::Execution,
            FeatureStage::Review => Self::Review,
            FeatureStage::Result => Self::Result,
            FeatureStage::Draft => Self::Draft,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FeatureSummaryView {
    pub id: Uuid,
    pub project_id: Uuid,
    pub conversation_id: Uuid,
    pub provider: String,
    pub title: String,
    pub stage: FeatureStageView,
    pub created_at: String,
}

impl From<Feature> for FeatureSummaryView {
    fn from(feature: Feature) -> Self {
        Self {
            id: feature.id,
            project_id: feature.project_id,
            conversation_id: feature.conversation_id,
            provider: feature.provider,
            title: feature.title,
            stage: feature.stage.into(),
            created_at: feature.created_at.to_rfc3339(),
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FeaturePageView {
    pub items: Vec<FeatureSummaryView>,
    pub next_cursor: Option<Uuid>,
}

impl From<FeaturePage> for FeaturePageView {
    fn from(page: FeaturePage) -> Self {
        Self {
            items: page.items.into_iter().map(Into::into).collect(),
            next_cursor: page.next_cursor,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FeatureScopeView {
    pub id: Uuid,
    pub digest: String,
    pub brief: String,
    pub approved_at: Option<String>,
    pub can_approve: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct FeatureDiagramView {
    pub title: String,
    pub source: String,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FeatureArchitectureView {
    pub id: Uuid,
    pub digest: String,
    pub summary: String,
    pub diagrams: Vec<FeatureDiagramView>,
    pub approved_at: Option<String>,
    pub can_approve: bool,
}

#[derive(Clone, Copy, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum FeatureRoleView {
    Coordinator,
    Architecture,
    Implementation,
    ComplexImplementation,
    Visual,
    Triage,
    GeneralReview,
    ArchitectureReview,
    WebReview,
    SecurityReview,
    Correction,
}

impl From<TaskRole> for FeatureRoleView {
    fn from(role: TaskRole) -> Self {
        match role {
            TaskRole::Coordinator => Self::Coordinator,
            TaskRole::Architecture => Self::Architecture,
            TaskRole::Implementation => Self::Implementation,
            TaskRole::ComplexImplementation => Self::ComplexImplementation,
            TaskRole::Visual => Self::Visual,
            TaskRole::Triage => Self::Triage,
            TaskRole::GeneralReview => Self::GeneralReview,
            TaskRole::ArchitectureReview => Self::ArchitectureReview,
            TaskRole::WebReview => Self::WebReview,
            TaskRole::SecurityReview => Self::SecurityReview,
            TaskRole::Correction => Self::Correction,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum FeatureAttemptStatusView {
    Starting,
    Running,
    Validated,
    Failed,
    Interrupted,
    Uncertain,
}

impl From<AttemptStatus> for FeatureAttemptStatusView {
    fn from(status: AttemptStatus) -> Self {
        match status {
            AttemptStatus::Starting => Self::Starting,
            AttemptStatus::Running => Self::Running,
            AttemptStatus::Validated => Self::Validated,
            AttemptStatus::Failed => Self::Failed,
            AttemptStatus::Interrupted => Self::Interrupted,
            AttemptStatus::Uncertain => Self::Uncertain,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FeatureAttemptView {
    pub id: Uuid,
    pub title: String,
    pub role: FeatureRoleView,
    pub assigned_model: String,
    pub observed_model: Option<String>,
    pub effort: Option<String>,
    pub status: FeatureAttemptStatusView,
    pub activity: Option<String>,
    pub retry_of: Option<Uuid>,
    pub checkpoint: Option<String>,
    pub can_resume: bool,
}

impl FeatureAttemptView {
    fn new(attempt: TaskAttempt, can_resume: bool) -> Self {
        Self {
            id: attempt.id(),
            title: attempt.claim.task.title,
            role: attempt.claim.task.role.into(),
            assigned_model: attempt.claim.model,
            observed_model: attempt.observed_model,
            effort: attempt.claim.effort,
            status: attempt.status.into(),
            activity: attempt.activity,
            retry_of: attempt.claim.retry_of,
            checkpoint: attempt.checkpoint,
            can_resume,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct FeatureFindingView {
    pub role: FeatureRoleView,
    pub summary: String,
    pub blocking: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct FeatureCheckView {
    pub label: String,
    pub passed: bool,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum FeatureIntegratedCheckStatusView {
    Preparing,
    Running,
    RecoveryNeeded,
    Finished,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct FeatureIntegratedCheckView {
    pub command: String,
    pub status: FeatureIntegratedCheckStatusView,
    pub passed: Option<bool>,
}

impl FeatureIntegratedCheckView {
    fn new(check: &IntegratedCheck, driver_active: bool) -> Self {
        use FeatureIntegratedCheckStatusView as View;
        let (status, passed) = match check.status {
            IntegratedCheckStatus::Preparing => (View::Preparing, None),
            IntegratedCheckStatus::Launching if driver_active => (View::Running, None),
            IntegratedCheckStatus::Launching => (View::RecoveryNeeded, None),
            IntegratedCheckStatus::Finished { passed } => (View::Finished, Some(passed)),
        };
        Self {
            command: check.command.clone(),
            status,
            passed,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FeatureReviewView {
    pub snapshot_id: Uuid,
    pub base_oid: String,
    pub head_oid: String,
    pub patch_digest: String,
    pub ready: bool,
    pub findings: Vec<FeatureFindingView>,
    pub checks: Vec<FeatureCheckView>,
    pub integrated_checks: Vec<FeatureIntegratedCheckView>,
    pub can_extend: bool,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FeatureDiffView {
    pub base_oid: String,
    pub head_oid: String,
    pub patch_digest: String,
    pub patch: String,
    pub changed_paths: Vec<String>,
}

impl From<WorkspaceDiff> for FeatureDiffView {
    fn from(diff: WorkspaceDiff) -> Self {
        Self {
            base_oid: diff.git.base_oid,
            head_oid: diff.git.head_oid,
            patch_digest: diff.git.patch_digest,
            patch: diff.patch,
            changed_paths: diff.changed_paths,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum FeatureDeliveryStatusView {
    Claimed,
    Uncertain,
    Failed,
    Published,
    PublishedStale,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FeatureDeliveryView {
    pub request_id: Uuid,
    pub snapshot_id: Uuid,
    pub status: FeatureDeliveryStatusView,
    pub reason: Option<String>,
    pub draft_url: Option<String>,
}

impl From<DeliveryReceipt> for FeatureDeliveryView {
    fn from(receipt: DeliveryReceipt) -> Self {
        use FeatureDeliveryStatusView as View;
        let (status, reason, draft_url) = match receipt.status {
            DeliveryStatus::Claimed => (View::Claimed, None, None),
            DeliveryStatus::Uncertain => (View::Uncertain, None, None),
            DeliveryStatus::Failed { reason } => (View::Failed, Some(reason), None),
            DeliveryStatus::Published { draft_url } => (View::Published, None, Some(draft_url)),
            DeliveryStatus::PublishedStale { draft_url } => {
                (View::PublishedStale, None, Some(draft_url))
            }
        };
        Self {
            request_id: receipt.request_id,
            snapshot_id: receipt.snapshot_id,
            status,
            reason,
            draft_url,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FeatureView {
    pub feature: FeatureSummaryView,
    pub scope: Option<FeatureScopeView>,
    pub architecture: Option<FeatureArchitectureView>,
    pub attempts: Vec<FeatureAttemptView>,
    pub review: Option<FeatureReviewView>,
    pub delivery: Option<FeatureDeliveryView>,
    pub can_continue: bool,
}

impl From<FeatureState> for FeatureView {
    fn from(state: FeatureState) -> Self {
        Self::from_state(state, false, false, |_| true)
    }
}

impl FeatureView {
    pub(super) fn from_state(
        mut state: FeatureState,
        can_continue: bool,
        driver_active: bool,
        is_locally_active: impl Fn(Uuid) -> bool,
    ) -> Self {
        let stage = state.feature.stage;
        let idle = !state
            .attempts
            .iter()
            .any(|attempt| attempt.status.holds_reservation())
            && !state.deliveries.iter().any(|receipt| {
                matches!(
                    receipt.status,
                    DeliveryStatus::Claimed | DeliveryStatus::Uncertain
                )
            });
        let scope = state.current_scope().and_then(|revision| {
            let GateContent::Scope { brief } = &revision.content else {
                return None;
            };
            Some(FeatureScopeView {
                id: revision.key.id,
                digest: revision.key.digest.clone(),
                brief: brief.clone(),
                approved_at: revision.approval.map(|approval| approval.at.to_rfc3339()),
                can_approve: idle && stage == FeatureStage::Scope && revision.approval.is_none(),
            })
        });
        let architecture = state.current_architecture().and_then(|revision| {
            let GateContent::Architecture {
                summary, diagrams, ..
            } = &revision.content
            else {
                return None;
            };
            let critique_complete = state.attempts.iter().any(|attempt| {
                attempt.claim.task.role == TaskRole::ArchitectureReview
                    && attempt.claim.revision.as_ref() == Some(&revision.key)
                    && attempt.claim.snapshot_id.is_none()
                    && attempt.status == AttemptStatus::Validated
            });
            Some(FeatureArchitectureView {
                id: revision.key.id,
                digest: revision.key.digest.clone(),
                summary: summary.clone(),
                diagrams: diagrams
                    .iter()
                    .map(|diagram| FeatureDiagramView {
                        title: diagram.title.clone(),
                        source: diagram.source.clone(),
                    })
                    .collect(),
                approved_at: revision.approval.map(|approval| approval.at.to_rfc3339()),
                can_approve: idle
                    && stage == FeatureStage::Architecture
                    && revision.approval.is_none()
                    && critique_complete,
            })
        });
        let review = state.review().map(|review| {
            let ready = review.verdict.as_ref().is_some_and(ReviewVerdict::ready);
            let review_count = state
                .reviews
                .iter()
                .filter(|round| round.architecture == review.architecture)
                .count();
            FeatureReviewView {
                snapshot_id: review.snapshot.id,
                base_oid: review.snapshot.git.base_oid.clone(),
                head_oid: review.snapshot.git.head_oid.clone(),
                patch_digest: review.snapshot.git.patch_digest.clone(),
                ready,
                findings: review
                    .verdict
                    .iter()
                    .flat_map(|verdict| &verdict.findings)
                    .map(|finding| FeatureFindingView {
                        role: finding.role.into(),
                        summary: finding.summary.clone(),
                        blocking: finding.blocking,
                    })
                    .collect(),
                checks: review
                    .verdict
                    .iter()
                    .flat_map(|verdict| &verdict.checks)
                    .map(|check| FeatureCheckView {
                        label: check.label.clone(),
                        passed: check.passed,
                    })
                    .collect(),
                integrated_checks: if review.verdict.is_none() {
                    review
                        .integrated_checks
                        .iter()
                        .map(|check| FeatureIntegratedCheckView::new(check, driver_active))
                        .collect()
                } else {
                    Vec::new()
                },
                can_extend: idle
                    && matches!(stage, FeatureStage::Review | FeatureStage::Execution)
                    && review.verdict.is_some()
                    && !ready
                    && review_count >= 2 + state.additional_review_rounds as usize,
            }
        });
        let mut seen = HashSet::new();
        let mut attempts: Vec<_> = state
            .attempts
            .into_iter()
            .rev()
            .map(|attempt| {
                let key = (
                    attempt.claim.task.id,
                    attempt.claim.revision.as_ref().map(|revision| revision.id),
                    attempt.claim.snapshot_id,
                );
                let revision = attempt.claim.revision.as_ref();
                let role = attempt.claim.task.role;
                let matches_stage = match stage {
                    FeatureStage::Scope => {
                        role == TaskRole::Coordinator && revision == state.scope.as_ref()
                    }
                    FeatureStage::Architecture => match role {
                        TaskRole::Coordinator | TaskRole::Architecture => {
                            revision == state.scope.as_ref()
                        }
                        TaskRole::ArchitectureReview => revision == state.architecture.as_ref(),
                        _ => false,
                    },
                    FeatureStage::Execution => {
                        !role.is_reviewer() && revision == state.architecture.as_ref()
                    }
                    FeatureStage::Review => {
                        role.is_reviewer()
                            && attempt.claim.snapshot_id == state.current_review
                            && revision == state.architecture.as_ref()
                    }
                    FeatureStage::Result | FeatureStage::Draft => false,
                };
                let can_resume = seen.insert(key)
                    && matches_stage
                    && match attempt.status {
                        AttemptStatus::Failed | AttemptStatus::Interrupted => {
                            attempt.launch_settled
                        }
                        status if status.holds_reservation() => !is_locally_active(attempt.id()),
                        _ => false,
                    };
                FeatureAttemptView::new(attempt, can_resume)
            })
            .collect();
        attempts.reverse();
        let can_continue = can_continue
            && !attempts.iter().any(|attempt| attempt.can_resume)
            && !state
                .deliveries
                .iter()
                .any(|receipt| matches!(receipt.status, DeliveryStatus::PublishedStale { .. }));
        Self {
            feature: state.feature.into(),
            scope,
            architecture,
            attempts,
            review,
            delivery: state.deliveries.pop().map(Into::into),
            can_continue,
        }
    }
}
