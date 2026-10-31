//! User-owned conversation records; provider protocol and credentials remain private.
use crate::environments::EnvironmentActor;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uuid::Uuid;

pub mod execution;

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ConversationError {
    #[error("An active session is required")]
    Unauthorized,
    #[error("Conversation not found")]
    NotFound,
    #[error("Conversation state changed")]
    Conflict,
    #[error("Invalid conversation input")]
    Invalid,
    #[error("Conversation service is unavailable")]
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conversation {
    pub id: Uuid,
    pub user_id: Uuid,
    /// Internal provider reference; never include this field in HTTP projections.
    pub provider_thread: Option<String>,
    pub created_at: DateTime<Utc>,
}
/// A stable keyset page, ordered newest first. The cursor is an owned conversation ID.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConversationPage {
    pub items: Vec<Conversation>,
    pub next_cursor: Option<Uuid>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunStatus {
    Active,
    Completed,
    Failed,
    Interrupted,
    Uncertain,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Run {
    pub id: Uuid,
    pub conversation_id: Uuid,
    pub operation_id: Uuid,
    pub environment_generation: i64,
    pub model: String,
    pub status: RunStatus,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    pub id: Uuid,
    pub run_id: Uuid,
    pub role: MessageRole,
    pub content: String,
    pub created_at: DateTime<Utc>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageRole {
    User,
    Assistant,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestKind {
    Permission,
    Question,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestState {
    Pending,
    Claimed,
    Uncertain,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingRequest {
    pub id: Uuid,
    pub run_id: Uuid,
    pub kind: RequestKind,
    pub state: RequestState,
    /// Validated application-owned representation, not raw provider frames.
    pub details: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResponseClaim {
    /// Private provider correlation; never project to HTTP.
    pub provider_reference: String,
    /// Only a fresh claim authorizes one delivery attempt.
    pub fresh: bool,
    pub state: ResponseState,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResponseState {
    Claimed,
    Delivered,
    Uncertain,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoverableRun {
    pub conversation_id: Uuid,
    pub run_id: Uuid,
    /// False means a launch request may still reach Docker; stop alone cannot recover it.
    pub launch_settled: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConversationHistory {
    pub conversation: Conversation,
    pub runs: Vec<Run>,
    pub messages: Vec<Message>,
    pub requests: Vec<PendingRequest>,
    /// Older records remain durable but are outside this bounded recent window.
    pub runs_truncated: bool,
    pub messages_truncated: bool,
    pub requests_truncated: bool,
}
#[derive(Clone, Debug)]
pub struct StartTurn {
    pub conversation_id: Uuid,
    pub operation_id: Uuid,
    pub environment_id: Uuid,
    pub generation: i64,
    pub model: String,
    pub input: String,
}

/// Every method checks current actor and environment in the same transaction as the operation.
/// Provider callbacks must not treat failed authorization as successful persistence.
#[async_trait]
pub trait ConversationStore: Send + Sync {
    /// Reserve before starting a provider process. The global limit is durable across restarts.
    async fn reserve_model_probe(
        &self,
        actor: EnvironmentActor,
        environment_id: Uuid,
        generation: i64,
        lease: Uuid,
    ) -> Result<(), ConversationError>;
    /// Internal only: the runtime has conclusively finished issuing its launch request.
    /// Never call this on timeout, lost transport, or process death alone.
    async fn settle_model_probe_launch(
        &self,
        lease: Uuid,
        environment_id: Uuid,
        generation: i64,
    ) -> Result<(), ConversationError>;
    /// Called only after the runtime proved that the owned process exited, or never started.
    async fn release_model_probe(
        &self,
        actor: EnvironmentActor,
        lease: Uuid,
    ) -> Result<(), ConversationError>;
    /// A later generation, freshly observed stopped, proves the old container cannot run.
    async fn recover_model_probe_stopped(
        &self,
        actor: EnvironmentActor,
        environment_id: Uuid,
        generation: i64,
        observed_at: DateTime<Utc>,
    ) -> Result<bool, ConversationError>;
    /// Includes persisted uncertain work, including across API restart.
    async fn has_open_runs(&self, actor: EnvironmentActor) -> Result<bool, ConversationError>;
    async fn create(
        &self,
        actor: EnvironmentActor,
        environment_id: Uuid,
        generation: i64,
    ) -> Result<Conversation, ConversationError>;
    async fn list(
        &self,
        actor: EnvironmentActor,
        environment_id: Uuid,
        generation: i64,
        before: Option<Uuid>,
    ) -> Result<ConversationPage, ConversationError>;
    async fn history(
        &self,
        actor: EnvironmentActor,
        environment_id: Uuid,
        generation: i64,
        id: Uuid,
    ) -> Result<ConversationHistory, ConversationError>;
    /// Repeating an operation with identical inputs returns its original run, never re-executes it.
    async fn begin_turn(
        &self,
        actor: EnvironmentActor,
        turn: StartTurn,
    ) -> Result<(Run, bool), ConversationError>;
    /// Internal only: a provider launch completed its Docker start request.
    async fn settle_run_launch(
        &self,
        run_id: Uuid,
        environment_id: Uuid,
        generation: i64,
    ) -> Result<(), ConversationError>;
    async fn set_thread(
        &self,
        actor: EnvironmentActor,
        environment_id: Uuid,
        generation: i64,
        run_id: Uuid,
        thread: &str,
    ) -> Result<(), ConversationError>;
    /// Store each completed provider item exactly once. Correlation stays private.
    /// Repeated identical items return the existing record; changed text conflicts.
    async fn append_assistant(
        &self,
        actor: EnvironmentActor,
        environment_id: Uuid,
        generation: i64,
        run_id: Uuid,
        provider_item: &str,
        text: &str,
    ) -> Result<(Message, bool), ConversationError>;
    async fn add_request(
        &self,
        actor: EnvironmentActor,
        environment_id: Uuid,
        generation: i64,
        request: PendingRequest,
        provider_reference: &str,
    ) -> Result<(), ConversationError>;
    /// A fresh durable claim allows one delivery attempt; existing claims must never be replayed.
    async fn claim_response(
        &self,
        actor: EnvironmentActor,
        environment_id: Uuid,
        generation: i64,
        request_id: Uuid,
        operation_id: Uuid,
        answer: &str,
    ) -> Result<ResponseClaim, ConversationError>;
    /// Only a provider acknowledgement correlated by the owning runtime establishes
    /// delivery; writing to the process alone does not. Uncertain must not be retried.
    async fn record_response(
        &self,
        actor: EnvironmentActor,
        environment_id: Uuid,
        generation: i64,
        request_id: Uuid,
        operation_id: Uuid,
        state: ResponseState,
    ) -> Result<(), ConversationError>;
    /// List only this owner's runs with proof that a later generation is stopped.
    async fn recoverable_stopped(
        &self,
        actor: EnvironmentActor,
        environment_id: Uuid,
        generation: i64,
        observed_at: DateTime<Utc>,
    ) -> Result<Vec<RecoverableRun>, ConversationError>;
    /// Release only after a fresh supervisor observation proves a later generation stopped.
    async fn recover_stopped(
        &self,
        actor: EnvironmentActor,
        environment_id: Uuid,
        generation: i64,
        observed_at: DateTime<Utc>,
        run_id: Uuid,
    ) -> Result<(), ConversationError>;
    /// Record a confirmed terminal outcome; completed items are persisted separately.
    /// Uncertain retains the execution reservation until an explicit later resolution.
    async fn finish_turn(
        &self,
        actor: EnvironmentActor,
        environment_id: Uuid,
        generation: i64,
        run_id: Uuid,
        status: RunStatus,
    ) -> Result<(), ConversationError>;
}
