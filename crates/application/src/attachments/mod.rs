//! Attachment use cases. Files live in the member's container; the store keeps their
//! metadata. Every call is fenced by the current running environment.
mod agent;
mod gates;

#[cfg(test)]
mod tests;

pub use agent::{AgentScope, DOCUMENTS, Deadline, HOME, Vantage};

use crate::conversations::ConversationService;
use chrono::{Duration, Utc};
use factory_core::{
    attachments::*,
    conversations::ConversationError,
    environments::{Environment, EnvironmentActor},
};
use gates::Gates;
use std::{collections::HashMap, sync::Arc};
use uuid::Uuid;

const UPLOADS_PER_USER: usize = 4;
const READS_PER_USER: usize = 4;
/// An `Uploading` row younger than this is a transfer in progress; older ones were
/// abandoned and no longer count against the staged limit.
const UPLOADING_MINUTES: i64 = 15;

pub struct AttachmentService {
    conversations: Arc<ConversationService>,
    store: Arc<dyn AttachmentStore>,
    files: Arc<dyn AttachmentFiles>,
    /// Counting staged files and reserving a new one must not interleave.
    reservations: Gates,
    uploads: Gates,
    reads: Gates,
}

pub fn conversation_error(error: ConversationError) -> AttachmentError {
    match error {
        ConversationError::Unauthorized => AttachmentError::Unauthorized,
        ConversationError::NotFound => AttachmentError::NotFound,
        ConversationError::Conflict | ConversationError::ContextLimit => AttachmentError::Conflict,
        ConversationError::Invalid => AttachmentError::Invalid,
        ConversationError::Unavailable => AttachmentError::Unavailable,
    }
}

pub fn attachment_error(error: AttachmentError) -> ConversationError {
    match error {
        AttachmentError::Unauthorized => ConversationError::Unauthorized,
        AttachmentError::NotFound => ConversationError::NotFound,
        AttachmentError::Conflict => ConversationError::Conflict,
        AttachmentError::Invalid | AttachmentError::TooLarge | AttachmentError::TooMany => {
            ConversationError::Invalid
        }
        AttachmentError::Unavailable => ConversationError::Unavailable,
    }
}

impl AttachmentService {
    pub fn new(
        conversations: Arc<ConversationService>,
        store: Arc<dyn AttachmentStore>,
        files: Arc<dyn AttachmentFiles>,
    ) -> Self {
        Self {
            conversations,
            store,
            files,
            reservations: Gates::new(1),
            uploads: Gates::new(UPLOADS_PER_USER),
            reads: Gates::new(READS_PER_USER),
        }
    }

    async fn environment(&self, actor: EnvironmentActor) -> Result<Environment, AttachmentError> {
        self.conversations
            .current_environment(actor)
            .await
            .map_err(conversation_error)
    }

    async fn location(
        &self,
        actor: EnvironmentActor,
        attachment: &Attachment,
    ) -> Result<AttachmentLocation, AttachmentError> {
        let conversation = self
            .conversations
            .get(actor, attachment.conversation_id)
            .await
            .map_err(conversation_error)?;
        AttachmentLocation::new(
            conversation.project_id,
            attachment.conversation_id,
            attachment.id,
            &attachment.name,
        )
    }

    /// Store a file as a staged attachment of an owned conversation. A failure leaves
    /// no file behind; an `Uploading` row is purged with the other expired uploads.
    pub async fn upload(
        &self,
        actor: EnvironmentActor,
        conversation_id: Uuid,
        name: &str,
        source: &mut dyn ByteSource,
    ) -> Result<Attachment, AttachmentError> {
        validate_name(name)?;
        let _transfer = self.uploads.try_acquire(actor.user_id)?;
        let (env, conversation, _) = self
            .conversations
            .context(actor, conversation_id)
            .await
            .map_err(conversation_error)?;
        self.purge_expired(actor, &env).await;
        let id = Uuid::new_v4();
        let location = AttachmentLocation::new(conversation.project_id, conversation_id, id, name)?;
        {
            let _slot = self.reservations.acquire(actor.user_id).await?;
            let recent = Utc::now() - Duration::minutes(UPLOADING_MINUTES);
            let staged = self
                .store
                .list(actor, env.id, env.generation, conversation_id)
                .await?
                .iter()
                .filter(|a| {
                    a.state == AttachmentState::Staged
                        || (a.state == AttachmentState::Uploading && a.created_at > recent)
                })
                .count();
            if staged >= MAX_ATTACHMENTS_PER_MESSAGE {
                return Err(AttachmentError::TooMany);
            }
            let new = NewAttachment {
                id,
                conversation_id,
                name: name.to_owned(),
            };
            self.store
                .reserve(actor, env.id, env.generation, new)
                .await?;
        }
        let staged = match self.files.put(&env, &location, source).await {
            Ok(file) => {
                self.store
                    .mark_staged(actor, env.id, env.generation, id, file)
                    .await
            }
            Err(error) => Err(error),
        };
        if staged.is_err() {
            let _ = self.files.delete_attachment(&env, &location).await;
        }
        staged
    }

    /// Best effort: expiry runs inside someone else's upload and must not fail it.
    async fn purge_expired(&self, actor: EnvironmentActor, env: &Environment) {
        let Ok(expired) = self
            .store
            .expire_staged(actor, staged_cutoff(Utc::now()))
            .await
        else {
            return;
        };
        let mut projects: HashMap<Uuid, Option<Uuid>> = HashMap::new();
        for attachment in expired {
            let project = match projects.get(&attachment.conversation_id) {
                Some(project) => *project,
                None => {
                    let Ok(conversation) = self
                        .conversations
                        .get(actor, attachment.conversation_id)
                        .await
                    else {
                        continue;
                    };
                    projects.insert(attachment.conversation_id, conversation.project_id);
                    conversation.project_id
                }
            };
            if let Ok(location) = AttachmentLocation::new(
                project,
                attachment.conversation_id,
                attachment.id,
                &attachment.name,
            ) {
                let _ = self.files.delete_attachment(env, &location).await;
            }
        }
    }

    /// Remove a staged upload. The row goes first so a concurrent send cannot attach
    /// a file that is being deleted; files that cannot be deleted now are purged with
    /// the conversation.
    pub async fn remove_staged(
        &self,
        actor: EnvironmentActor,
        id: Uuid,
    ) -> Result<(), AttachmentError> {
        let env = self.environment(actor).await?;
        let removed = self
            .store
            .remove_staged(actor, env.id, env.generation, id)
            .await?;
        if let Ok(location) = self.location(actor, &removed).await {
            let _ = self.files.delete_attachment(&env, &location).await;
        }
        Ok(())
    }

    /// Metadata of an owned attachment.
    pub async fn get(
        &self,
        actor: EnvironmentActor,
        id: Uuid,
    ) -> Result<Attachment, AttachmentError> {
        self.environment(actor).await?;
        self.store.get(actor, id).await
    }

    /// Owned, staged uploads of this conversation, in the requested order, for a turn
    /// about to attach them. The turn's own transaction rechecks every condition.
    pub async fn staged(
        &self,
        actor: EnvironmentActor,
        conversation_id: Uuid,
        ids: &[Uuid],
    ) -> Result<Vec<Attachment>, AttachmentError> {
        validate_attachment_ids(ids)?;
        let mut found = Vec::with_capacity(ids.len());
        for id in ids {
            let attachment = self.store.get(actor, *id).await?;
            if attachment.conversation_id != conversation_id {
                return Err(AttachmentError::NotFound);
            }
            if attachment.state != AttachmentState::Staged {
                return Err(AttachmentError::Conflict);
            }
            found.push(attachment);
        }
        Ok(found)
    }

    /// Up to `MAX_READ_CHUNK_BYTES` from `offset`, shorter at the end of the file.
    /// Only stored files are readable; the caller serves them with `served_response`.
    pub async fn read(
        &self,
        actor: EnvironmentActor,
        id: Uuid,
        offset: u64,
        length: u64,
    ) -> Result<(Attachment, Vec<u8>), AttachmentError> {
        let _read = self.reads.acquire(actor.user_id).await?;
        let env = self.environment(actor).await?;
        let attachment = self.store.get(actor, id).await?;
        if !matches!(
            attachment.state,
            AttachmentState::Staged | AttachmentState::Attached
        ) {
            return Err(AttachmentError::NotFound);
        }
        if offset > attachment.size {
            return Err(AttachmentError::Invalid);
        }
        let length = length
            .min(MAX_READ_CHUNK_BYTES)
            .min(attachment.size - offset);
        if length == 0 {
            return Ok((attachment, Vec::new()));
        }
        let location = self.location(actor, &attachment).await?;
        let bytes = self.files.read(&env, &location, offset, length).await?;
        if bytes.len() as u64 > length {
            return Err(AttachmentError::Unavailable);
        }
        Ok((attachment, bytes))
    }

    /// Persist the agent's captured files with its message. See `capture_agent_files`.
    pub async fn record_agent_files(
        &self,
        actor: EnvironmentActor,
        env: &Environment,
        run_id: Uuid,
        message_id: Uuid,
        files: Vec<AgentAttachment>,
    ) -> Result<Vec<Attachment>, AttachmentError> {
        if files.is_empty() {
            return Ok(Vec::new());
        }
        self.store
            .record_agent_attachments(actor, env.id, env.generation, run_id, message_id, files)
            .await
    }

    /// Delete every attachment file of a project. Missing files count as deleted, so a
    /// failed project deletion can call this again. Call it after the project is fenced
    /// and before its database rows are removed.
    pub async fn purge_project(
        &self,
        actor: EnvironmentActor,
        project_id: Uuid,
    ) -> Result<(), AttachmentError> {
        let env = self.environment(actor).await?;
        self.files.delete_project(&env, project_id).await
    }

    /// Create the directories a discussion worker mounts for one attempt.
    pub async fn prepare_worker(
        &self,
        env: &Environment,
        project_id: Uuid,
        conversation_id: Uuid,
        attempt_id: Uuid,
    ) -> Result<WorkerDirectories, AttachmentError> {
        self.files
            .prepare_worker_directories(env, project_id, conversation_id, attempt_id)
            .await
    }
}
