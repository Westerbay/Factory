//! Ports for attachment metadata and for files in the member's container.
use super::{Attachment, AttachmentError, AttachmentLocation, MediaKind};
use crate::environments::{Environment, EnvironmentActor};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uuid::Uuid;

/// Largest range one `read` call may return.
pub const MAX_READ_CHUNK_BYTES: u64 = 4 * 1024 * 1024;

/// A file whose bytes were stored and classified.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredFile {
    pub media_type: String,
    pub kind: MediaKind,
    pub size: u64,
    /// Lowercase hexadecimal SHA-256 of the stored bytes.
    pub sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewAttachment {
    pub id: Uuid,
    pub conversation_id: Uuid,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentCapture {
    Captured(StoredFile),
    /// The referenced file was missing, outside the allowed roots or too large.
    Unavailable,
}

/// One file the agent referenced in a message. The ID names the capture directory,
/// so it is chosen before the file is copied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentAttachment {
    pub id: Uuid,
    pub name: String,
    pub capture: AgentCapture,
}

/// Upload bytes chunk by chunk, so the file never has to fit in memory.
#[async_trait]
pub trait ByteSource: Send {
    /// The next non-empty chunk, or `None` at the end of the input.
    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, AttachmentError>;
}

/// Recheck ownership and the current environment generation in every transaction.
#[async_trait]
pub trait AttachmentStore: Send + Sync {
    /// Record an `Uploading` attachment in an owned, current conversation.
    async fn reserve(
        &self,
        actor: EnvironmentActor,
        environment_id: Uuid,
        generation: i64,
        attachment: NewAttachment,
    ) -> Result<Attachment, AttachmentError>;

    /// `Uploading` becomes `Staged` with its classified type and size.
    async fn mark_staged(
        &self,
        actor: EnvironmentActor,
        environment_id: Uuid,
        generation: i64,
        id: Uuid,
        file: StoredFile,
    ) -> Result<Attachment, AttachmentError>;

    /// Delete the row of a `Staged` attachment and return it; any other state
    /// conflicts. The caller then deletes its files.
    async fn remove_staged(
        &self,
        actor: EnvironmentActor,
        environment_id: Uuid,
        generation: i64,
        id: Uuid,
    ) -> Result<Attachment, AttachmentError>;

    /// Delete this owner's `Staged` and abandoned `Uploading` rows created before
    /// `before` (see `staged_cutoff`) and return them so their files can be deleted.
    async fn expire_staged(
        &self,
        actor: EnvironmentActor,
        before: DateTime<Utc>,
    ) -> Result<Vec<Attachment>, AttachmentError>;

    async fn get(&self, actor: EnvironmentActor, id: Uuid) -> Result<Attachment, AttachmentError>;

    /// Attachments of an owned conversation, oldest first.
    async fn list(
        &self,
        actor: EnvironmentActor,
        environment_id: Uuid,
        generation: i64,
        conversation_id: Uuid,
    ) -> Result<Vec<Attachment>, AttachmentError>;

    /// Attach the agent's files to its message in one transaction, `Attached` when
    /// captured and `Unavailable` otherwise. Repeating a run's capture is idempotent.
    async fn record_agent_attachments(
        &self,
        actor: EnvironmentActor,
        environment_id: Uuid,
        generation: i64,
        run_id: Uuid,
        message_id: Uuid,
        attachments: Vec<AgentAttachment>,
    ) -> Result<Vec<Attachment>, AttachmentError>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkerDirectories {
    /// Read-only source mounted at `WORKER_ATTACHMENTS_DIR`, relative to the home.
    pub attachments: String,
    /// Writable source mounted at `WORKER_OUTPUTS_DIR`, relative to the home.
    pub outputs: String,
}

/// File operations in the member's container. Paths are derived from
/// `AttachmentLocation`; implementations must never follow symlinks out of the store.
#[async_trait]
pub trait AttachmentFiles: Send + Sync {
    /// Write to a partial file, stop with `TooLarge` past `MAX_ATTACHMENT_BYTES`, then
    /// rename without replacing an existing file and classify the stored bytes.
    async fn put(
        &self,
        environment: &Environment,
        location: &AttachmentLocation,
        source: &mut dyn ByteSource,
    ) -> Result<StoredFile, AttachmentError>;

    /// Copy `reference` (already resolved by `resolve_reference`) into the store after
    /// resolving symlinks and rechecking it against `roots`. `NotFound` when absent,
    /// `Invalid` when outside the roots or not a regular file.
    async fn capture(
        &self,
        environment: &Environment,
        location: &AttachmentLocation,
        reference: &str,
        roots: &[String],
    ) -> Result<StoredFile, AttachmentError>;

    /// At most `MAX_READ_CHUNK_BYTES` from `offset`; shorter at the end of the file.
    async fn read(
        &self,
        environment: &Environment,
        location: &AttachmentLocation,
        offset: u64,
        length: u64,
    ) -> Result<Vec<u8>, AttachmentError>;

    /// Missing files are already deleted.
    async fn delete_attachment(
        &self,
        environment: &Environment,
        location: &AttachmentLocation,
    ) -> Result<(), AttachmentError>;

    async fn delete_conversation(
        &self,
        environment: &Environment,
        project_id: Option<Uuid>,
        conversation_id: Uuid,
    ) -> Result<(), AttachmentError>;

    /// Purge every conversation store of the project.
    async fn delete_project(
        &self,
        environment: &Environment,
        project_id: Uuid,
    ) -> Result<(), AttachmentError>;

    /// Create the worker's directories for one attempt, tolerating existing ones.
    async fn prepare_worker_directories(
        &self,
        environment: &Environment,
        project_id: Uuid,
        conversation_id: Uuid,
        attempt_id: Uuid,
    ) -> Result<WorkerDirectories, AttachmentError>;
}
