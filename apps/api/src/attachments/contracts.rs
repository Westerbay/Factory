use chrono::{DateTime, Utc};
use factory_core::attachments::{Attachment, AttachmentSource, AttachmentState, MediaKind};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum AttachmentSourceView {
    User,
    Agent,
}

#[derive(Clone, Copy, Debug, Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum AttachmentStateView {
    Uploading,
    Staged,
    Attached,
    /// An agent reference that could not be captured; only metadata remains.
    Unavailable,
}

/// How the interface presents the file; classified by Factory from the stored bytes.
#[derive(Clone, Copy, Debug, Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum MediaKindView {
    Image,
    Svg,
    Video,
    Pdf,
    Text,
    Html,
    File,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentView {
    pub id: Uuid,
    pub conversation_id: Uuid,
    pub run_id: Option<Uuid>,
    pub message_id: Option<Uuid>,
    pub source: AttachmentSourceView,
    pub state: AttachmentStateView,
    pub name: String,
    /// The stored type, never the type the client declared.
    pub media_type: String,
    pub kind: MediaKindView,
    pub size: u64,
    pub created_at: String,
}

impl From<AttachmentSource> for AttachmentSourceView {
    fn from(source: AttachmentSource) -> Self {
        match source {
            AttachmentSource::User => Self::User,
            AttachmentSource::Agent => Self::Agent,
        }
    }
}

impl From<AttachmentState> for AttachmentStateView {
    fn from(state: AttachmentState) -> Self {
        match state {
            AttachmentState::Uploading => Self::Uploading,
            AttachmentState::Staged => Self::Staged,
            AttachmentState::Attached => Self::Attached,
            AttachmentState::Unavailable => Self::Unavailable,
        }
    }
}

impl From<MediaKind> for MediaKindView {
    fn from(kind: MediaKind) -> Self {
        match kind {
            MediaKind::Image => Self::Image,
            MediaKind::Svg => Self::Svg,
            MediaKind::Video => Self::Video,
            MediaKind::Pdf => Self::Pdf,
            MediaKind::Text => Self::Text,
            MediaKind::Html => Self::Html,
            MediaKind::File => Self::File,
        }
    }
}

impl From<Attachment> for AttachmentView {
    fn from(attachment: Attachment) -> Self {
        Self {
            id: attachment.id,
            conversation_id: attachment.conversation_id,
            run_id: attachment.run_id,
            message_id: attachment.message_id,
            source: attachment.source.into(),
            state: attachment.state.into(),
            name: attachment.name,
            media_type: attachment.media_type,
            kind: attachment.kind.into(),
            size: attachment.size,
            created_at: DateTime::<Utc>::to_rfc3339(&attachment.created_at),
        }
    }
}

/// Raw file bytes, up to 50 MiB.
#[derive(ToSchema)]
#[schema(value_type = String, format = Binary)]
#[allow(dead_code)]
pub struct BinaryBody(Vec<u8>);

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UploadQuery {
    pub name: String,
    /// The type the client declared. It is validated for size and otherwise ignored.
    #[serde(rename = "type")]
    pub declared_type: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContentQuery {
    pub download: Option<String>,
}
