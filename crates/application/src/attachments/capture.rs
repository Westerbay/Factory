//! A capture remains owned until its metadata is committed or its cleanup settles.
use super::{AttachmentService, agent::derived_id};
use factory_core::{
    attachments::*,
    environments::{Environment, EnvironmentActor},
};
use std::{future::Future, pin::Pin, sync::Arc};
use tokio::sync::{OwnedSemaphorePermit, oneshot};
use uuid::Uuid;

/// Only identifiers and a classified error; no file paths, content or credentials.
#[derive(Debug)]
pub struct CaptureCleanupFailure {
    pub environment_id: Uuid,
    pub attachment_id: Uuid,
    pub error: AttachmentError,
}

pub type CaptureTask =
    Pin<Box<dyn Future<Output = Result<(), CaptureCleanupFailure>> + Send + 'static>>;

/// Run through caller cancellation and report failed or uncertain cleanup.
pub trait CaptureHost: Send + Sync {
    fn spawn(&self, task: CaptureTask);
}

pub(super) struct CaptureJob {
    pub actor: EnvironmentActor,
    pub environment: Environment,
    pub location: AttachmentLocation,
    pub reference: String,
    pub roots: Vec<String>,
    pub lease: Arc<OwnedSemaphorePermit>,
}

pub(super) struct CaptureReceipt {
    id: Uuid,
    accepted: oneshot::Sender<()>,
}

impl CaptureReceipt {
    /// Retain bytes only after the store confirms that this file is attached.
    pub fn accept(self, recorded: &[Attachment]) {
        if recorded
            .iter()
            .any(|file| file.id == self.id && file.state == AttachmentState::Attached)
        {
            let _ = self.accepted.send(());
        }
        // Otherwise dropping the sender leaves cleanup to the still-owned job.
    }
}

#[derive(Default)]
pub(super) struct PendingFiles {
    pub files: Vec<AgentAttachment>,
    pub receipts: Vec<CaptureReceipt>,
}

impl AttachmentService {
    /// Attachment rows for one assistant message are inserted atomically. A replay
    /// uses these canonical rows, including Unavailable, without reading files again.
    pub(super) async fn recorded_message(
        &self,
        actor: EnvironmentActor,
        conversation: Uuid,
        run: Uuid,
        item: &str,
        references: &[String],
    ) -> Result<Option<Uuid>, AttachmentError> {
        let mut message = None;
        let mut found = 0;
        for (index, reference) in references.iter().enumerate() {
            let row = match self.store.get(actor, derived_id(run, item, index)).await {
                Ok(row) => row,
                Err(AttachmentError::NotFound) => continue,
                Err(error) => return Err(error),
            };
            if row.conversation_id != conversation
                || row.run_id != Some(run)
                || row.source != AttachmentSource::Agent
                || row.name != display_name_from_reference(reference)
                || !matches!(
                    row.state,
                    AttachmentState::Attached | AttachmentState::Unavailable
                )
                || row.message_id.is_none()
                || message.is_some_and(|id| row.message_id != Some(id))
            {
                return Err(AttachmentError::Conflict);
            }
            message = row.message_id;
            found += 1;
        }
        if found != 0 && found != references.len() {
            return Err(AttachmentError::Conflict);
        }
        Ok(message)
    }

    pub(super) fn launch_capture(
        &self,
        job: CaptureJob,
    ) -> (
        oneshot::Receiver<Result<StoredFile, AttachmentError>>,
        CaptureReceipt,
    ) {
        let files = self.files.clone();
        let store = self.store.clone();
        let id = job.location.attachment_id();
        let (send, receive) = oneshot::channel();
        let (accept, accepted) = oneshot::channel();
        self.capture_host.spawn(Box::pin(async move {
            // A replay cannot race this job's late completion and cleanup.
            let _lease = job.lease;
            if send.is_closed() {
                return Ok(());
            }
            let copied = files
                .capture(&job.environment, &job.location, &job.reference, &job.roots)
                .await;
            let completed = copied.is_ok();
            let uncertain = copied == Err(AttachmentError::Unavailable);
            let delivered = send.send(copied).is_ok();
            let failure = |error| CaptureCleanupFailure {
                environment_id: job.environment.id,
                attachment_id: id,
                error,
            };
            if !completed {
                // Normal controller errors abort their partial write. A transport
                // failure is not proof that its process has stopped: never race it
                // with deletion, and make that uncertainty visible to the host.
                return if uncertain {
                    Err(failure(AttachmentError::Unavailable))
                } else {
                    Ok(())
                };
            }
            if delivered && accepted.await.is_ok() {
                return Ok(());
            }
            // Successful capture means the adapter confirmed process exit. Check
            // canonical metadata again in case persistence won the cancellation race.
            match store.get(job.actor, id).await {
                Ok(row)
                    if row.conversation_id == job.location.conversation_id()
                        && row.source == AttachmentSource::Agent =>
                {
                    match row.state {
                        AttachmentState::Attached => return Ok(()),
                        AttachmentState::Unavailable => {}
                        _ => return Err(failure(AttachmentError::Conflict)),
                    }
                }
                Ok(_) => return Err(failure(AttachmentError::Conflict)),
                Err(AttachmentError::NotFound) => {}
                Err(error) => return Err(failure(error)),
            }
            files
                .delete_attachment(&job.environment, &job.location)
                .await
                .map_err(failure)
        }));
        (
            receive,
            CaptureReceipt {
                id,
                accepted: accept,
            },
        )
    }
}
