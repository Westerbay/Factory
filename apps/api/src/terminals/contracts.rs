use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

pub const SUBPROTOCOL: &str = "factory.terminal.v1";
pub const MAX_CONTROL_BYTES: usize = 2048;
pub const MAX_MESSAGE_BYTES: usize = 128 * 1024;

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AttachmentRequest {
    pub take_over: bool,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentResponse {
    pub ticket: String,
    pub expires_at: DateTime<Utc>,
}

/// UTF-8 JSON inputs for factory.terminal.v1. Output data uses binary frames.
#[derive(Deserialize, ToSchema)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum TerminalClientMessage {
    Authenticate { ticket: String },
    Input { data: String },
    Resize { cols: u16, rows: u16 },
    Ack { bytes: u32 },
}

#[derive(Clone, Copy, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum TerminalErrorCode {
    Unauthorized,
    Unavailable,
    Conflict,
    InvalidTicket,
    InvalidInput,
    SlowClient,
    CleanupUncertain,
}

impl From<factory_core::terminals::TerminalError> for TerminalErrorCode {
    fn from(error: factory_core::terminals::TerminalError) -> Self {
        use factory_core::terminals::TerminalError;
        match error {
            TerminalError::Unauthorized => Self::Unauthorized,
            TerminalError::Unavailable => Self::Unavailable,
            TerminalError::Conflict => Self::Conflict,
            TerminalError::InvalidTicket => Self::InvalidTicket,
            TerminalError::InvalidInput => Self::InvalidInput,
            TerminalError::CleanupUncertain => Self::CleanupUncertain,
        }
    }
}

#[derive(Serialize, ToSchema)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum TerminalServerMessage {
    Ready,
    Detached { code: TerminalErrorCode },
    Exit,
    Error { code: TerminalErrorCode },
}
