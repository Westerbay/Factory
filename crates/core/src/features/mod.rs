//! Provider-neutral feature workflow records and transitions.

mod ports;
mod workflow;

pub use ports::*;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum FeatureError {
    #[error("An active session is required")]
    Unauthorized,
    #[error("Feature not found")]
    NotFound,
    #[error("Invalid feature input")]
    Invalid,
    #[error("Feature state changed")]
    Conflict,
    #[error("Execution may still be active")]
    ExecutionUncertain,
    #[error("Review cannot proceed")]
    ReviewBlocked,
    #[error("The reviewed diff changed")]
    StaleSnapshot,
    #[error("Feature service is unavailable")]
    Unavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FeatureStage {
    Scope,
    Architecture,
    Execution,
    Review,
    Result,
    Draft,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Feature {
    pub id: Uuid,
    pub user_id: Uuid,
    pub project_id: Uuid,
    pub conversation_id: Uuid,
    pub environment_id: Uuid,
    pub provider: String,
    pub workflow_revision: String,
    pub title: String,
    pub stage: FeatureStage,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewFeature {
    pub operation_id: Uuid,
    pub project_id: Uuid,
    pub provider: String,
    pub workflow_revision: String,
    pub title: String,
    pub opener: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeaturePage {
    pub items: Vec<Feature>,
    pub next_cursor: Option<Uuid>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeatureState {
    pub feature: Feature,
    pub revisions: Vec<GateRevision>,
    pub scope: Option<RevisionKey>,
    pub architecture: Option<RevisionKey>,
    pub attempts: Vec<TaskAttempt>,
    pub reviews: Vec<ReviewRound>,
    pub current_review: Option<Uuid>,
    pub additional_review_rounds: u32,
    pub deliveries: Vec<DeliveryReceipt>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevisionKey {
    pub id: Uuid,
    /// Digest of the canonical, validated proposal, computed by the application.
    pub digest: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateRevision {
    pub key: RevisionKey,
    pub content: GateContent,
    pub approval: Option<GateApproval>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GateContent {
    Scope {
        brief: String,
    },
    Architecture {
        scope: RevisionKey,
        summary: String,
        diagrams: Vec<FeatureDiagram>,
        tasks: Vec<FeatureTask>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateApproval {
    pub user_id: Uuid,
    pub at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeatureDiagram {
    pub title: String,
    pub source: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeatureTask {
    pub id: Uuid,
    pub title: String,
    pub instructions: String,
    pub role: TaskRole,
    pub paths: Vec<String>,
    pub depends_on: Vec<Uuid>,
    /// Verification commands approved with the task plan.
    pub checks: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum TaskRole {
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

impl TaskRole {
    pub fn is_reviewer(self) -> bool {
        matches!(
            self,
            Self::GeneralReview | Self::ArchitectureReview | Self::WebReview | Self::SecurityReview
        )
    }

    pub fn access(self) -> WorkspaceAccess {
        match self {
            Self::Implementation | Self::ComplexImplementation | Self::Correction => {
                WorkspaceAccess::Write
            }
            _ => WorkspaceAccess::ReadOnly,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkspaceAccess {
    ReadOnly,
    Write,
}

/// The workspace adapter derives its actual path from these identities.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskWorkspace {
    pub feature_id: Uuid,
    pub attempt_id: Uuid,
    pub base_oid: String,
    pub access: WorkspaceAccess,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AttemptStatus {
    Starting,
    Running,
    Validated,
    Failed,
    Interrupted,
    Uncertain,
}

impl AttemptStatus {
    pub fn holds_reservation(self) -> bool {
        matches!(self, Self::Starting | Self::Running | Self::Uncertain)
    }
}

/// Inputs durably reserved before launching a worker.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttemptClaim {
    pub task: FeatureTask,
    pub revision: Option<RevisionKey>,
    pub workspace: TaskWorkspace,
    pub snapshot_id: Option<Uuid>,
    pub model: String,
    pub effort: Option<String>,
    pub skill_revision: String,
    pub retry_of: Option<Uuid>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskAttempt {
    pub claim: AttemptClaim,
    pub generation: i64,
    pub status: AttemptStatus,
    pub launch_settled: bool,
    /// Private correlation only. Never expose in HTTP projections.
    pub process_receipt: Option<String>,
    /// Private correlation only. Never expose in HTTP projections.
    pub provider_thread: Option<String>,
    pub observed_model: Option<String>,
    pub activity: Option<String>,
    pub result: Option<String>,
    pub checkpoint: Option<String>,
}

impl TaskAttempt {
    pub fn id(&self) -> Uuid {
        self.claim.workspace.attempt_id
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AttemptUpdate {
    LaunchSettled {
        process_receipt: Option<String>,
    },
    ProviderThread {
        id: String,
    },
    ObservedModel {
        model: String,
    },
    Activity {
        summary: String,
    },
    Result {
        text: String,
    },
    Validated {
        checkpoint: Option<String>,
    },
    Failed {
        reason: String,
    },
    /// Internal only: a trusted Git comparison established the last integrated head.
    IntegrationConflict {
        retry_base: String,
    },
    Interrupted,
    Uncertain,
}

/// Git identity returned by the trusted project controller.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitSnapshot {
    pub base_oid: String,
    pub head_oid: String,
    pub patch_digest: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewSnapshot {
    pub id: Uuid,
    pub git: GitSnapshot,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewRound {
    pub snapshot: ReviewSnapshot,
    pub architecture: RevisionKey,
    pub required_roles: Vec<TaskRole>,
    pub verdict: Option<ReviewVerdict>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewFinding {
    pub role: TaskRole,
    pub summary: String,
    pub blocking: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewCheck {
    pub label: String,
    pub passed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewVerdict {
    pub findings: Vec<ReviewFinding>,
    pub checks: Vec<ReviewCheck>,
}

impl ReviewVerdict {
    pub fn ready(&self) -> bool {
        !self.checks.is_empty()
            && self.checks.iter().all(|check| check.passed)
            && self.findings.iter().all(|finding| !finding.blocking)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveryReceipt {
    pub request_id: Uuid,
    pub snapshot_id: Uuid,
    pub status: DeliveryStatus,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeliveryStatus {
    Claimed,
    Uncertain,
    Failed { reason: String },
    Published { draft_url: String },
    PublishedStale { draft_url: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FeatureCommand {
    ProposeRevision(GateRevision),
    ApproveRevision(RevisionKey),
    ClaimAttempt(AttemptClaim),
    RecordAttempt {
        attempt_id: Uuid,
        update: AttemptUpdate,
    },
    RecoverAttempt {
        attempt_id: Uuid,
        launch_generation: i64,
    },
    BeginReview {
        snapshot: ReviewSnapshot,
        required_roles: Vec<TaskRole>,
    },
    RecordReview {
        snapshot_id: Uuid,
        patch_digest: String,
        verdict: ReviewVerdict,
    },
    BeginCorrections {
        snapshot_id: Uuid,
    },
    AuthorizeAdditionalReview {
        snapshot_id: Uuid,
    },
    ClaimDelivery {
        snapshot_id: Uuid,
        observed: GitSnapshot,
    },
    RecordDelivery {
        request_id: Uuid,
        status: DeliveryStatus,
    },
    InvalidateResult {
        snapshot_id: Uuid,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeatureReceipt {
    pub state: FeatureState,
    /// Only a fresh claim authorizes a new effect.
    pub fresh: bool,
}

#[cfg(test)]
mod tests;
