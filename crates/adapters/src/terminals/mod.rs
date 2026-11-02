//! tmux attachment through the owned local Docker runtime.
use crate::environments::{
    DockerEnvironmentRuntime,
    process::{FixedProcess, ProcessInput, ProcessOutput},
};
use crate::projects::{DockerProjectRuntime, ProjectDirectory};
use async_trait::async_trait;
use factory_core::{
    environments::Environment,
    projects::{Project, ProjectState},
    terminals::*,
};
use futures_util::StreamExt;
use std::{sync::Arc, time::Duration};
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

pub struct DockerTerminalRuntime(Arc<DockerEnvironmentRuntime>);

impl DockerTerminalRuntime {
    pub fn new(runtime: Arc<DockerEnvironmentRuntime>) -> Self {
        Self(runtime)
    }
}

struct Input(ProcessInput);
struct Output {
    stream: ProcessOutput,
    pending: Vec<u8>,
    offset: usize,
}

#[async_trait]
impl TerminalInput for Input {
    async fn write(&mut self, input: &[u8]) -> Result<(), TerminalError> {
        tokio::time::timeout(Duration::from_secs(3), async {
            self.0
                .write_all(input)
                .await
                .map_err(|_| TerminalError::Unavailable)?;
            self.0.flush().await.map_err(|_| TerminalError::Unavailable)
        })
        .await
        .map_err(|_| TerminalError::Unavailable)?
    }
}

#[async_trait]
impl TerminalOutput for Output {
    async fn read(&mut self) -> Result<Option<Vec<u8>>, TerminalError> {
        while self.offset == self.pending.len() {
            self.pending.clear();
            self.offset = 0;
            let Some(chunk) = self.stream.next().await else {
                return Ok(None);
            };
            let chunk = chunk.map_err(|_| TerminalError::Unavailable)?;
            if chunk.as_ref().len() > MAX_OUTPUT_BUFFER_BYTES {
                return Err(TerminalError::Unavailable);
            }
            self.pending.extend_from_slice(chunk.as_ref());
        }
        let end = (self.offset + OUTPUT_CHUNK_BYTES).min(self.pending.len());
        let chunk = self.pending[self.offset..end].to_vec();
        self.offset = end;
        Ok(Some(chunk))
    }
}

#[async_trait]
impl TerminalRuntime for DockerTerminalRuntime {
    async fn attach(&self, environment: &Environment) -> Result<TerminalProcess, TerminalError> {
        let process = self
            .0
            .start_process(environment, FixedProcess::TerminalAttach)
            .await?;
        Ok(TerminalProcess {
            handle: process.id,
            input: Box::new(Input(process.input)),
            output: Box::new(Output {
                stream: process.output,
                pending: Vec::new(),
                offset: 0,
            }),
        })
    }

    async fn attach_conversation(
        &self,
        environment: &Environment,
        conversation_id: Uuid,
        project: Option<&Project>,
    ) -> Result<TerminalProcess, TerminalError> {
        if conversation_id.is_nil() {
            return Err(TerminalError::InvalidInput);
        }
        if let Some(project) = project
            && (project.state != ProjectState::Ready
                || DockerProjectRuntime::new(self.0.clone())
                    .inspect_project(environment, project)
                    .await
                    != Ok(ProjectDirectory::Ready))
        {
            return Err(TerminalError::Unavailable);
        }
        let process = self
            .0
            .start_process(
                environment,
                FixedProcess::TerminalAttachConversation {
                    conversation: conversation_id,
                    project: project.map(|p| p.directory.clone()),
                },
            )
            .await?;
        Ok(TerminalProcess {
            handle: process.id,
            input: Box::new(Input(process.input)),
            output: Box::new(Output {
                stream: process.output,
                pending: Vec::new(),
                offset: 0,
            }),
        })
    }

    async fn detach_conversation(
        &self,
        environment: &Environment,
        conversation_id: Uuid,
        handle: &str,
    ) -> Result<(), TerminalError> {
        self.detach_process(
            environment,
            handle,
            FixedProcess::TerminalDetachConversation {
                conversation: conversation_id,
            },
        )
        .await
    }

    async fn resize(&self, handle: &str, size: TerminalSize) -> Result<(), TerminalError> {
        self.0
            .resize_process(handle, size.cols, size.rows)
            .await
            .map_err(Into::into)
    }

    async fn detach(&self, environment: &Environment, handle: &str) -> Result<(), TerminalError> {
        self.detach_process(environment, handle, FixedProcess::TerminalDetach)
            .await
    }
}

impl DockerTerminalRuntime {
    async fn detach_process(
        &self,
        environment: &Environment,
        handle: &str,
        command: FixedProcess,
    ) -> Result<(), TerminalError> {
        // A natural shell exit or a fully closed Docker stream already detaches.
        if self
            .0
            .wait_process_exit(handle, Duration::from_millis(250))
            .await
            .is_ok()
        {
            return Ok(());
        }
        let detach = self
            .0
            .start_process(environment, command)
            .await
            .map_err(|_| TerminalError::CleanupUncertain)?;
        let detach_id = detach.id.clone();
        drop(detach);
        self.0
            .wait_process_exit(&detach_id, Duration::from_secs(5))
            .await
            .map_err(|_| TerminalError::CleanupUncertain)?;
        self.0
            .wait_process_exit(handle, Duration::from_secs(5))
            .await
            .map_err(|_| TerminalError::CleanupUncertain)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
