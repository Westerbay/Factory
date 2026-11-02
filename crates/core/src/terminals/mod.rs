//! Interactive terminal contracts, independent of HTTP and Docker.
use crate::{
    environments::{Environment, EnvironmentError},
    projects::Project,
};
use async_trait::async_trait;
use uuid::Uuid;

pub const MAX_INPUT_BYTES: usize = 16 * 1024;
pub const OUTPUT_CHUNK_BYTES: usize = 16 * 1024;
pub const OUTPUT_WINDOW_BYTES: usize = 256 * 1024;
pub const MAX_OUTPUT_BUFFER_BYTES: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TerminalError {
    #[error("An active session is required")]
    Unauthorized,
    #[error("Terminal unavailable; check the environment")]
    Unavailable,
    #[error("Terminal already attached or environment changed")]
    Conflict,
    #[error("The attachment ticket is invalid or expired")]
    InvalidTicket,
    #[error("Invalid terminal message")]
    InvalidInput,
    #[error("Terminal detachment could not be verified")]
    CleanupUncertain,
}

impl From<EnvironmentError> for TerminalError {
    fn from(value: EnvironmentError) -> Self {
        match value {
            EnvironmentError::Unauthorized => Self::Unauthorized,
            EnvironmentError::Conflict => Self::Conflict,
            _ => Self::Unavailable,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct TerminalSize {
    pub cols: u16,
    pub rows: u16,
}

impl TerminalSize {
    pub fn new(cols: u16, rows: u16) -> Result<Self, TerminalError> {
        if !(2..=500).contains(&cols) || !(1..=200).contains(&rows) {
            return Err(TerminalError::InvalidInput);
        }
        Ok(Self { cols, rows })
    }
}

#[async_trait]
pub trait TerminalInput: Send {
    /// Implementations must bound transport writes to at most three seconds.
    async fn write(&mut self, input: &[u8]) -> Result<(), TerminalError>;
}

#[async_trait]
pub trait TerminalOutput: Send {
    /// At most OUTPUT_CHUNK_BYTES per read, without retaining an unbounded queue.
    async fn read(&mut self) -> Result<Option<Vec<u8>>, TerminalError>;
}

pub struct TerminalProcess {
    /// Internal runtime handle. Never projected into browser contracts.
    pub handle: String,
    pub input: Box<dyn TerminalInput>,
    pub output: Box<dyn TerminalOutput>,
}

#[async_trait]
pub trait TerminalRuntime: Send + Sync {
    async fn attach(&self, environment: &Environment) -> Result<TerminalProcess, TerminalError>;
    /// Attach a conversation-specific shell after validating its project directory.
    async fn attach_conversation(
        &self,
        _environment: &Environment,
        _conversation_id: Uuid,
        _project: Option<&Project>,
    ) -> Result<TerminalProcess, TerminalError> {
        Err(TerminalError::Unavailable)
    }
    /// Detach only the specified conversation's tmux client.
    async fn detach_conversation(
        &self,
        _environment: &Environment,
        _conversation_id: Uuid,
        _handle: &str,
    ) -> Result<(), TerminalError> {
        Err(TerminalError::Unavailable)
    }
    async fn resize(&self, handle: &str, size: TerminalSize) -> Result<(), TerminalError>;
    /// Caller serializes generations. Detach only this environment's fixed tmux
    /// session, and prove that the specified attachment process has exited.
    async fn detach(&self, environment: &Environment, handle: &str) -> Result<(), TerminalError>;
}
