//! Store layout and validation of agent-written file references.
use super::{AttachmentError, MAX_REFERENCE_BYTES, validate_name};
use uuid::Uuid;

/// Relative to the member's home directory.
pub const STORE_ROOT: &str = ".factory/attachments";
/// Read-only in feature discussion workers.
pub const WORKER_ATTACHMENTS_DIR: &str = "/workspace/attachments";
/// Writable per attempt in feature discussion workers.
pub const WORKER_OUTPUTS_DIR: &str = "/workspace/outputs";

/// Directory of one conversation, relative to the member's home: chats share
/// `chats`, project conversations live under their project.
pub fn conversation_store(project_id: Option<Uuid>, conversation_id: Uuid) -> String {
    let scope = project_id.map_or_else(|| "chats".to_owned(), |id| id.to_string());
    format!("{STORE_ROOT}/{scope}/{conversation_id}")
}

/// Directory of the project's conversations, relative to the member's home.
pub fn project_store(project_id: Uuid) -> String {
    format!("{STORE_ROOT}/{project_id}")
}

/// Writable output directory of one worker attempt, relative to the member's home.
/// The leading dot keeps it apart from attachment directories.
pub fn worker_outputs_store(
    project_id: Option<Uuid>,
    conversation_id: Uuid,
    attempt_id: Uuid,
) -> String {
    format!(
        "{}/.outputs/{attempt_id}",
        conversation_store(project_id, conversation_id)
    )
}

/// Where one attachment lives; the name has been validated as a single component.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttachmentLocation {
    project_id: Option<Uuid>,
    conversation_id: Uuid,
    attachment_id: Uuid,
    name: String,
}

impl AttachmentLocation {
    pub fn new(
        project_id: Option<Uuid>,
        conversation_id: Uuid,
        attachment_id: Uuid,
        name: &str,
    ) -> Result<Self, AttachmentError> {
        validate_name(name)?;
        Ok(Self {
            project_id,
            conversation_id,
            attachment_id,
            name: name.to_owned(),
        })
    }

    pub fn project_id(&self) -> Option<Uuid> {
        self.project_id
    }

    pub fn conversation_id(&self) -> Uuid {
        self.conversation_id
    }

    pub fn attachment_id(&self) -> Uuid {
        self.attachment_id
    }

    /// Relative to the member's home.
    pub fn directory(&self) -> String {
        format!(
            "{}/{}",
            conversation_store(self.project_id, self.conversation_id),
            self.attachment_id
        )
    }

    /// Relative to the member's home.
    pub fn file(&self) -> String {
        format!("{}/{}", self.directory(), self.name)
    }
}

/// Resolve an agent reference lexically and require it to name a file inside one of
/// the absolute `roots`. Relative references are taken from the absolute `base`.
/// `..` may not climb above `/`, and the result is returned normalized. Symlinks
/// are resolved and rechecked by the file adapter when it reads the file.
pub fn resolve_reference<R: AsRef<str>>(
    reference: &str,
    base: &str,
    roots: &[R],
) -> Result<String, AttachmentError> {
    if reference.is_empty()
        || reference.len() > MAX_REFERENCE_BYTES
        || reference.chars().any(char::is_control)
    {
        return Err(AttachmentError::Invalid);
    }
    let joined;
    let absolute = if reference.starts_with('/') {
        reference
    } else {
        joined = format!("{base}/{reference}");
        &joined
    };
    let components = normalize(absolute).ok_or(AttachmentError::Invalid)?;
    let inside = roots
        .iter()
        .filter_map(|root| normalize(root.as_ref()))
        .filter(|root| !root.is_empty())
        .any(|root| components.len() > root.len() && components.starts_with(&root));
    if !inside {
        return Err(AttachmentError::Invalid);
    }
    let path = format!("/{}", components.join("/"));
    if path.len() > MAX_REFERENCE_BYTES {
        return Err(AttachmentError::Invalid);
    }
    Ok(path)
}

fn normalize(path: &str) -> Option<Vec<&str>> {
    if !path.starts_with('/') {
        return None;
    }
    let mut components = Vec::new();
    for component in path.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                components.pop()?;
            }
            other => components.push(other),
        }
    }
    Some(components)
}
