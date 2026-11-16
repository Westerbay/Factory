use super::*;
use crate::{
    conversations::execution::{ExecutionError, ExecutionSession},
    environments::{Environment, EnvironmentActor},
    projects::Project,
};
use async_trait::async_trait;

pub enum FeatureProposal {
    Scope {
        brief: String,
    },
    Architecture {
        summary: String,
        diagrams: Vec<FeatureDiagram>,
        tasks: Vec<FeatureTask>,
    },
}

pub struct FeatureReviewReply {
    pub patch_digest: String,
    pub complete: bool,
    pub findings: Vec<FeatureReplyFinding>,
}

pub struct FeatureReplyFinding {
    pub summary: String,
    pub blocking: bool,
}

/// Provider response encoding and stable content identity are adapter concerns.
pub trait FeatureCodec: Send + Sync {
    fn parse_proposal(&self, raw: &str) -> Result<FeatureProposal, FeatureError>;
    fn revision_digest(&self, content: &GateContent) -> Result<String, FeatureError>;
    fn render_revision(&self, content: &GateContent) -> Result<String, FeatureError>;
    fn parse_review(&self, raw: &str) -> Result<FeatureReviewReply, FeatureError>;
    fn derived_id(&self, namespace: Uuid, label: &str) -> Uuid;
}

/// Establish the stable repository identity before using clone-time names.
#[async_trait]
pub trait FeatureRepositoryVerifier: Send + Sync {
    async fn verify(
        &self,
        actor: EnvironmentActor,
        environment: &Environment,
        project: &Project,
    ) -> Result<(), FeatureError>;
}

/// Recheck ownership in every transaction. Mutation paths also check the current
/// environment generation. Match operation inputs before returning a prior receipt.
#[async_trait]
pub trait FeatureStore: Send + Sync {
    /// Persist the feature, project conversation and assistant opener together.
    async fn create(
        &self,
        actor: EnvironmentActor,
        environment: &Environment,
        request: NewFeature,
    ) -> Result<FeatureState, FeatureError>;

    async fn list(
        &self,
        actor: EnvironmentActor,
        project_id: Option<Uuid>,
        before: Option<Uuid>,
    ) -> Result<FeaturePage, FeatureError>;

    async fn get(&self, actor: EnvironmentActor, id: Uuid) -> Result<FeatureState, FeatureError>;

    /// Resolve an owned feature conversation without exposing another member's data.
    async fn get_by_conversation(
        &self,
        actor: EnvironmentActor,
        conversation_id: Uuid,
    ) -> Result<Option<FeatureState>, FeatureError>;

    /// Compare any existing operation first, then apply the pure transition under
    /// lock and persist its effects and receipt in the same transaction.
    async fn apply(
        &self,
        actor: EnvironmentActor,
        environment: &Environment,
        id: Uuid,
        operation_id: Uuid,
        command: FeatureCommand,
    ) -> Result<FeatureReceipt, FeatureError>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FeatureRun {
    pub workspace: TaskWorkspace,
    pub provider: String,
    pub model: String,
    pub effort: Option<String>,
    pub input: String,
    /// Conversation whose attachments the worker may read and whose outputs it fills.
    pub conversation_id: Option<Uuid>,
}

pub struct FeatureExecution {
    pub session: ExecutionSession,
    /// Private runtime correlation, in addition to the durable attempt identity.
    pub process_receipt: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TaskObservation {
    Running,
    /// The launch settled and all processes associated with the attempt have ended.
    Stopped,
    /// An owned immutable worker was directly observed exited. Retain it
    /// until the application can persist proof of the stopped launch.
    StoppedWithWorkerProof {
        worker_id: String,
    },
    Unknown,
}

/// Evidence from the exact isolated check worker. Absence cannot settle a launch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckObservation {
    Running,
    Unknown,
    Exited { exit_code: i64 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FeatureCheckOutput {
    pub exit_code: i64,
    pub output: String,
}

#[async_trait]
pub trait FeatureExecutionRuntime: Send + Sync {
    /// Establish role-specific isolation before issuing a process launch.
    async fn open_task(
        &self,
        environment: &Environment,
        project: &Project,
        request: &FeatureRun,
    ) -> Result<FeatureExecution, ExecutionError>;

    /// Run an approved check in a disposable workspace with no credentials or network.
    async fn run_check(
        &self,
        environment: &Environment,
        project: &Project,
        workspace: &TaskWorkspace,
        command: &str,
    ) -> Result<FeatureCheckOutput, ExecutionError>;

    /// Release the stopped check worker after its outcome is recorded.
    async fn acknowledge_check(
        &self,
        _environment: &Environment,
        _project: &Project,
        _workspace: &TaskWorkspace,
        _generation: i64,
    ) -> Result<(), ExecutionError> {
        Ok(())
    }

    async fn reconcile_check(
        &self,
        _environment: &Environment,
        _project: &Project,
        _workspace: &TaskWorkspace,
        _generation: i64,
    ) -> Result<CheckObservation, ExecutionError> {
        Ok(CheckObservation::Unknown)
    }

    /// The runtime must identify the workspace and attempt even when the launch
    /// response, including its private process receipt, was lost.
    async fn reconcile_task(
        &self,
        environment: &Environment,
        project: &Project,
        attempt: &TaskAttempt,
    ) -> Result<TaskObservation, ExecutionError>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum WorkspaceError {
    #[error("The workspace operation was invalid")]
    Invalid,
    #[error("The workspace changed")]
    Conflict,
    #[error("The task changed files outside its approved scope")]
    ScopeViolation,
    #[error(
        "The combined diff exceeds Factory's preview limits or cannot be displayed. Reduce the task's changes and retry."
    )]
    DiffUnrenderable,
    #[error("The workspace operation is unavailable")]
    Unavailable,
    #[error("The workspace operation may still be active")]
    Uncertain,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FeatureBranch {
    pub base_ref: String,
    pub base_oid: String,
    pub head_ref: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceDiff {
    pub git: GitSnapshot,
    pub patch: String,
    pub changed_paths: Vec<String>,
}

#[async_trait]
pub trait FeatureWorkspaceRuntime: Send + Sync {
    /// Fetch a new candidate that is private until the origin has been rechecked.
    async fn prepare_initialization(
        &self,
        environment: &Environment,
        project: &Project,
        feature_id: Uuid,
        candidate_id: Uuid,
    ) -> Result<FeatureBranch, WorkspaceError>;

    /// Promote only the exact candidate observed before the origin recheck.
    async fn confirm_initialization(
        &self,
        environment: &Environment,
        project: &Project,
        feature_id: Uuid,
        candidate_id: Uuid,
        expected: &FeatureBranch,
    ) -> Result<FeatureBranch, WorkspaceError>;

    async fn prepare_task(
        &self,
        environment: &Environment,
        project: &Project,
        workspace: &TaskWorkspace,
    ) -> Result<(), WorkspaceError>;

    /// Verify scope and produce an attributable commit without publishing it.
    async fn checkpoint(
        &self,
        environment: &Environment,
        project: &Project,
        attempt: &TaskAttempt,
    ) -> Result<String, WorkspaceError>;

    /// Serialize integration and refuse unexpected changes or merge conflicts.
    async fn integrate(
        &self,
        environment: &Environment,
        project: &Project,
        attempt: &TaskAttempt,
        expected_head: &str,
    ) -> Result<String, WorkspaceError>;

    async fn snapshot(
        &self,
        environment: &Environment,
        project: &Project,
        feature_id: Uuid,
    ) -> Result<WorkspaceDiff, WorkspaceError>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FeatureRemoteRefs {
    pub base_oid: Option<String>,
    pub head_oid: Option<String>,
    pub managed_head_oid: Option<String>,
}

/// Trusted Git operations used only by authenticated draft delivery.
#[async_trait]
pub trait FeatureDeliveryRuntime: Send + Sync {
    async fn remote(
        &self,
        environment: &Environment,
        project: &Project,
        feature_id: Uuid,
    ) -> Result<FeatureRemoteRefs, WorkspaceError>;

    /// Verify the snapshot and create the destination ref without force.
    async fn push_reviewed(
        &self,
        environment: &Environment,
        project: &Project,
        feature_id: Uuid,
        snapshot: &GitSnapshot,
    ) -> Result<(), WorkspaceError>;

    /// Compute a candidate in isolation. It is not usable by reviewers.
    async fn stage_refresh(
        &self,
        environment: &Environment,
        project: &Project,
        feature_id: Uuid,
        candidate_id: Uuid,
        expected: &GitSnapshot,
    ) -> Result<GitSnapshot, WorkspaceError>;

    /// Promote the exact candidate after the application rechecks the origin.
    async fn confirm_refresh(
        &self,
        environment: &Environment,
        project: &Project,
        feature_id: Uuid,
        candidate_id: Uuid,
        expected: &GitSnapshot,
        proposed: &GitSnapshot,
    ) -> Result<WorkspaceDiff, WorkspaceError>;
}
