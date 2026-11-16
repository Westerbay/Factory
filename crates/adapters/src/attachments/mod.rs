//! Attachment files in the member's container, through a fixed Python controller that
//! the image does not contain: its source is embedded here and run as
//! `python3 -I -S -c`, with a bounded JSON request on stdin and never in arguments.
//! The controller resolves every path from non-symlink directory descriptors; this
//! adapter adds the environment ownership checks of `start_process_checked`.
mod protocol;

use crate::environments::{
    DockerEnvironmentRuntime,
    process::{FixedProcess, ProcessInput},
};
use async_trait::async_trait;
use bollard::container::LogOutput;
use factory_core::{
    attachments::{
        AttachmentError, AttachmentFiles, AttachmentLocation, ByteSource, MAX_ATTACHMENT_BYTES,
        MAX_READ_CHUNK_BYTES, StoredFile, WorkerDirectories, conversation_store, resolve_reference,
        worker_outputs_store,
    },
    environments::Environment,
};
use futures_util::StreamExt;
use protocol::{
    END_OF_UPLOAD, FRAME_LIMIT, READ_HEADER_LIMIT, REPLY_LIMIT, frame_prefix,
    parse_acknowledgement, parse_read, parse_result, parse_stored_file, stored_name,
};
use serde_json::Value;
use std::{sync::Arc, time::Duration};
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

/// The controller's source, run by `FixedProcess::AttachmentController`.
pub(crate) const CONTROLLER_SOURCE: &str = include_str!("controller.py");

const MAX_ROOTS: usize = 8;
const SHORT_TIMEOUT: Duration = Duration::from_secs(60);
const DELETE_TIMEOUT: Duration = Duration::from_secs(300);
/// Covers a slow client sending the largest allowed file.
const UPLOAD_TIMEOUT: Duration = Duration::from_secs(900);
const EXIT_TIMEOUT: Duration = Duration::from_secs(10);

/// Why the request could not be fully sent. Either way the controller sees an upload
/// without its end marker and discards the partial file.
enum SendFailure {
    Source(AttachmentError),
    Transport,
}

pub struct DockerAttachmentFiles(Arc<DockerEnvironmentRuntime>);

impl DockerAttachmentFiles {
    pub fn new(runtime: Arc<DockerEnvironmentRuntime>) -> Self {
        Self(runtime)
    }

    async fn run_result(
        &self,
        env: &Environment,
        request: &Value,
        timeout: Duration,
    ) -> Result<Value, AttachmentError> {
        let reply = self
            .exchange(env, request, None, REPLY_LIMIT, timeout)
            .await?;
        parse_result(&reply)
    }

    async fn run_acknowledged(
        &self,
        env: &Environment,
        request: &Value,
        expected: &str,
        timeout: Duration,
    ) -> Result<(), AttachmentError> {
        let reply = self
            .exchange(env, request, None, REPLY_LIMIT, timeout)
            .await?;
        parse_acknowledgement(&reply, expected)
    }

    /// Run the controller once and return its raw stdout. A source or transport
    /// failure before the upload ends is reported only after the process has stopped,
    /// so its partial file is already discarded.
    async fn exchange(
        &self,
        env: &Environment,
        request: &Value,
        upload: Option<&mut dyn ByteSource>,
        output_limit: usize,
        timeout: Duration,
    ) -> Result<Vec<u8>, AttachmentError> {
        let header = protocol::header(request)?;
        let mut process = self
            .0
            .start_process_checked(env, FixedProcess::AttachmentController)
            .await
            .map_err(|_| AttachmentError::Unavailable)?;
        let finished = tokio::time::timeout(timeout, async {
            let sent = send(&mut process.input, &header, upload).await;
            // Always end stdin: without the end marker this aborts an upload.
            let _ = process.input.shutdown().await;
            let mut output = Vec::new();
            while let Some(frame) = process.output.next().await {
                match frame {
                    Ok(LogOutput::StdOut { message }) => {
                        if output.len().saturating_add(message.len()) > output_limit {
                            return Err(AttachmentError::Unavailable);
                        }
                        output.extend_from_slice(&message);
                    }
                    Ok(_) => {}
                    Err(_) => return Err(AttachmentError::Unavailable),
                }
            }
            Ok((sent, output))
        })
        .await;
        let id = process.id.clone();
        drop(process);
        let (sent, output) = finished.map_err(|_| AttachmentError::Unavailable)??;
        let exit = self.0.wait_process_exit(&id, EXIT_TIMEOUT).await;
        match sent {
            Err(SendFailure::Source(error)) => return Err(error),
            // The controller may have answered before it read everything.
            Err(SendFailure::Transport) if !matches!(exit, Ok(0)) => {
                return Err(AttachmentError::Unavailable);
            }
            _ => {}
        }
        if !matches!(exit, Ok(0)) {
            return Err(AttachmentError::Unavailable);
        }
        Ok(output)
    }
}

/// The reference must already be normalized and inside a root; the controller checks
/// it again against the real filesystem.
fn validate_capture(reference: &str, roots: &[String]) -> Result<(), AttachmentError> {
    if roots.is_empty() || roots.len() > MAX_ROOTS {
        return Err(AttachmentError::Invalid);
    }
    match resolve_reference(reference, "/", roots) {
        Ok(path) if path == reference => Ok(()),
        _ => Err(AttachmentError::Invalid),
    }
}

fn validate_read(offset: u64, length: u64) -> Result<(), AttachmentError> {
    if offset > MAX_ATTACHMENT_BYTES || length > MAX_READ_CHUNK_BYTES {
        return Err(AttachmentError::Invalid);
    }
    Ok(())
}

/// Header, then frames of at most `FRAME_LIMIT` bytes and the end marker. Stops
/// without the marker past the limit, so an oversized file is never committed.
async fn send(
    input: &mut ProcessInput,
    header: &[u8],
    upload: Option<&mut dyn ByteSource>,
) -> Result<(), SendFailure> {
    input
        .write_all(header)
        .await
        .map_err(|_| SendFailure::Transport)?;
    let Some(source) = upload else {
        return Ok(());
    };
    let mut total = 0_u64;
    while let Some(chunk) = source.next_chunk().await.map_err(SendFailure::Source)? {
        for part in chunk.chunks(FRAME_LIMIT) {
            total += part.len() as u64;
            if total > MAX_ATTACHMENT_BYTES {
                return Err(SendFailure::Source(AttachmentError::TooLarge));
            }
            input
                .write_all(&frame_prefix(part.len()))
                .await
                .map_err(|_| SendFailure::Transport)?;
            input
                .write_all(part)
                .await
                .map_err(|_| SendFailure::Transport)?;
        }
    }
    input
        .write_all(&END_OF_UPLOAD)
        .await
        .map_err(|_| SendFailure::Transport)
}

#[async_trait]
impl AttachmentFiles for DockerAttachmentFiles {
    async fn put(
        &self,
        environment: &Environment,
        location: &AttachmentLocation,
        source: &mut dyn ByteSource,
    ) -> Result<StoredFile, AttachmentError> {
        let request = protocol::put_request(location);
        let reply = self
            .exchange(
                environment,
                &request,
                Some(source),
                REPLY_LIMIT,
                UPLOAD_TIMEOUT,
            )
            .await?;
        parse_stored_file(&stored_name(location), &parse_result(&reply)?)
    }

    async fn capture(
        &self,
        environment: &Environment,
        location: &AttachmentLocation,
        reference: &str,
        roots: &[String],
    ) -> Result<StoredFile, AttachmentError> {
        validate_capture(reference, roots)?;
        let request = protocol::capture_request(location, reference, roots);
        let reply = self
            .run_result(environment, &request, UPLOAD_TIMEOUT)
            .await?;
        parse_stored_file(&stored_name(location), &reply)
    }

    async fn read(
        &self,
        environment: &Environment,
        location: &AttachmentLocation,
        offset: u64,
        length: u64,
    ) -> Result<Vec<u8>, AttachmentError> {
        validate_read(offset, length)?;
        let request = protocol::read_request(location, offset, length);
        // `length` is bounded above, so the buffer is too.
        let limit = READ_HEADER_LIMIT + length as usize;
        let reply = self
            .exchange(environment, &request, None, limit, SHORT_TIMEOUT)
            .await?;
        parse_read(&reply)
    }

    async fn delete_attachment(
        &self,
        environment: &Environment,
        location: &AttachmentLocation,
    ) -> Result<(), AttachmentError> {
        let request = protocol::delete_attachment_request(location);
        self.run_acknowledged(environment, &request, "deleted", DELETE_TIMEOUT)
            .await
    }

    async fn delete_conversation(
        &self,
        environment: &Environment,
        project_id: Option<Uuid>,
        conversation_id: Uuid,
    ) -> Result<(), AttachmentError> {
        let request = protocol::delete_conversation_request(project_id, conversation_id);
        self.run_acknowledged(environment, &request, "deleted", DELETE_TIMEOUT)
            .await
    }

    async fn delete_project(
        &self,
        environment: &Environment,
        project_id: Uuid,
    ) -> Result<(), AttachmentError> {
        let request = protocol::delete_project_request(project_id);
        self.run_acknowledged(environment, &request, "deleted", DELETE_TIMEOUT)
            .await
    }

    async fn prepare_worker_directories(
        &self,
        environment: &Environment,
        project_id: Uuid,
        conversation_id: Uuid,
        attempt_id: Uuid,
    ) -> Result<WorkerDirectories, AttachmentError> {
        let request = protocol::prepare_worker_request(project_id, conversation_id, attempt_id);
        self.run_acknowledged(environment, &request, "ready", SHORT_TIMEOUT)
            .await?;
        Ok(WorkerDirectories {
            attachments: conversation_store(Some(project_id), conversation_id),
            outputs: worker_outputs_store(Some(project_id), conversation_id, attempt_id),
        })
    }
}

#[cfg(test)]
mod tests;
