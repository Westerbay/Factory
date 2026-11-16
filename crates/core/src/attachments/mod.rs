//! Conversation attachments: limits, media classification, store paths, agent file
//! references and the ports implemented by persistence and container adapters.
use chrono::{DateTime, Duration, Utc};
use uuid::Uuid;

mod block;
mod media;
mod paths;
mod ports;

#[cfg(test)]
mod tests;

pub use block::*;
pub use media::*;
pub use paths::*;
pub use ports::*;

pub const MAX_ATTACHMENT_BYTES: u64 = 52_428_800;
pub const MAX_ATTACHMENTS_PER_MESSAGE: usize = 10;
pub const MAX_NAME_BYTES: usize = 255;
pub const MAX_REFERENCE_BYTES: usize = 1024;
/// An upload that is not attached to a turn within this window is purged.
pub const STAGED_TTL_HOURS: i64 = 24;

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AttachmentError {
    #[error("An active session is required")]
    Unauthorized,
    #[error("Attachment not found")]
    NotFound,
    #[error("Attachment state changed")]
    Conflict,
    #[error("Invalid attachment input")]
    Invalid,
    #[error("The file is larger than the attachment limit")]
    TooLarge,
    #[error("Too many attachments in one message")]
    TooMany,
    #[error("Attachment service is unavailable")]
    Unavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttachmentSource {
    User,
    Agent,
}

impl AttachmentSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Agent => "agent",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "user" => Some(Self::User),
            "agent" => Some(Self::Agent),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttachmentState {
    /// Bytes are being written; the file is not readable yet.
    Uploading,
    /// Stored and classified, not yet attached to a turn.
    Staged,
    Attached,
    /// An agent reference could not be captured; only metadata remains.
    Unavailable,
}

impl AttachmentState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Uploading => "uploading",
            Self::Staged => "staged",
            Self::Attached => "attached",
            Self::Unavailable => "unavailable",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "uploading" => Some(Self::Uploading),
            "staged" => Some(Self::Staged),
            "attached" => Some(Self::Attached),
            "unavailable" => Some(Self::Unavailable),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaKind {
    Image,
    /// Rendered only through an isolated image, never inline in the page.
    Svg,
    Video,
    Pdf,
    Text,
    Html,
    File,
}

impl MediaKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::Svg => "svg",
            Self::Video => "video",
            Self::Pdf => "pdf",
            Self::Text => "text",
            Self::Html => "html",
            Self::File => "file",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "image" => Some(Self::Image),
            "svg" => Some(Self::Svg),
            "video" => Some(Self::Video),
            "pdf" => Some(Self::Pdf),
            "text" => Some(Self::Text),
            "html" => Some(Self::Html),
            "file" => Some(Self::File),
            _ => None,
        }
    }
}

/// Metadata of one file. Until bytes are stored (`Uploading`, `Unavailable`) the
/// type is `application/octet-stream`/`File` and the size is zero.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attachment {
    pub id: Uuid,
    pub conversation_id: Uuid,
    pub run_id: Option<Uuid>,
    pub message_id: Option<Uuid>,
    pub source: AttachmentSource,
    pub state: AttachmentState,
    pub name: String,
    /// Classified from content by Factory; the client-declared type is never kept.
    pub media_type: String,
    pub kind: MediaKind,
    pub size: u64,
    pub created_at: DateTime<Utc>,
}

/// A display name is also the stored file name, so it must be a single path component.
pub fn validate_name(name: &str) -> Result<(), AttachmentError> {
    if name.is_empty()
        || name.len() > MAX_NAME_BYTES
        || name == "."
        || name == ".."
        || name.chars().any(|c| c == '/' || c.is_control())
    {
        return Err(AttachmentError::Invalid);
    }
    Ok(())
}

/// Reject duplicates and more than the per-message limit before any transaction.
pub fn validate_attachment_ids(ids: &[Uuid]) -> Result<(), AttachmentError> {
    if ids.len() > MAX_ATTACHMENTS_PER_MESSAGE {
        return Err(AttachmentError::TooMany);
    }
    if ids
        .iter()
        .enumerate()
        .any(|(index, id)| ids[..index].contains(id))
    {
        return Err(AttachmentError::Invalid);
    }
    Ok(())
}

/// Staged uploads created before this instant are expired.
pub fn staged_cutoff(now: DateTime<Utc>) -> DateTime<Utc> {
    now - Duration::hours(STAGED_TTL_HOURS)
}

/// A valid display name for an agent-referenced file, derived from its last path
/// component. Invalid characters become `_`; the result is never empty.
pub fn display_name_from_reference(reference: &str) -> String {
    let last = reference.rsplit('/').next().unwrap_or_default();
    let mut name = String::new();
    for c in last.chars() {
        let c = if c.is_control() { '_' } else { c };
        if name.len() + c.len_utf8() > MAX_NAME_BYTES {
            break;
        }
        name.push(c);
    }
    if validate_name(&name).is_err() {
        return "file".to_owned();
    }
    name
}
