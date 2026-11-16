//! How the agent sees attachments (prompt manifest) and how its files are captured.
use super::AttachmentService;
use factory_core::{
    attachments::*,
    conversations::ConversationError,
    environments::{Environment, EnvironmentActor},
    projects::Project,
};
use std::{
    future::{Future, poll_fn},
    pin::{Pin, pin},
    task::Poll,
};
use uuid::Uuid;

/// The member's home directory inside the container.
pub const HOME: &str = "/home/factory";
/// Working directory of conversations outside a project.
pub const DOCUMENTS: &str = "/home/factory/Documents";

/// Resolves once the time allowed for a capture is over.
pub type Deadline = Pin<Box<dyn Future<Output = ()> + Send>>;

/// Where an agent sees the attachment store.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Vantage {
    /// Chat and project agents run in the member's container.
    Container { project_id: Option<Uuid> },
    /// Feature workers see the conversation's files under `WORKER_ATTACHMENTS_DIR`.
    Worker,
}

impl Vantage {
    pub fn path(self, attachment: &Attachment) -> Result<String, AttachmentError> {
        match self {
            Self::Container { project_id } => {
                let location = AttachmentLocation::new(
                    project_id,
                    attachment.conversation_id,
                    attachment.id,
                    &attachment.name,
                )?;
                Ok(format!("{HOME}/{}", location.file()))
            }
            Self::Worker => {
                validate_name(&attachment.name)?;
                Ok(format!(
                    "{WORKER_ATTACHMENTS_DIR}/{}/{}",
                    attachment.id, attachment.name
                ))
            }
        }
    }

    fn lines(self, files: &[&Attachment]) -> String {
        files
            .iter()
            .filter_map(|file| {
                let path = self.path(file).ok()?;
                Some(format!(
                    "- {path} ({}, {} bytes)\n",
                    file.kind.as_str(),
                    file.size
                ))
            })
            .collect()
    }

    /// Appended to the prompt: the files of this message and how to show a file back.
    pub fn prompt_suffix(self, files: &[Attachment]) -> String {
        let mut suffix = String::new();
        if !files.is_empty() {
            let files: Vec<&Attachment> = files.iter().collect();
            suffix.push_str("\n\nFiles attached by the member (open them with your tools):\n");
            suffix.push_str(&self.lines(&files));
        }
        let (directory, order) = match self {
            Self::Container { .. } => ("your working directory", ""),
            Self::Worker => (WORKER_OUTPUTS_DIR, ", before any factory-proposal block"),
        };
        suffix.push_str(&format!(
            "\n\nTo show the member a file you created, write it under {directory} and list its path, one per line, in a {FILES_BLOCK_INFO} fenced block at the end of your message{order}. Only listed files are shown, at most {MAX_ATTACHMENTS_PER_MESSAGE}."
        ));
        suffix
    }

    /// Paths of the files attached to a past message, for transcripts replayed to a model.
    pub fn history_note(self, attachments: &[Attachment], message_id: Uuid) -> String {
        let files: Vec<&Attachment> = attachments
            .iter()
            .filter(|a| a.message_id == Some(message_id) && a.state == AttachmentState::Attached)
            .collect();
        if files.is_empty() {
            return String::new();
        }
        format!("Attached files:\n{}", self.lines(&files))
    }
}

/// Where an agent may reference files from, as the agent sees them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentScope {
    pub project_id: Option<Uuid>,
    pub conversation_id: Uuid,
    /// Base of relative references.
    pub base: String,
    pub roots: Vec<String>,
    /// `(agent path, container path)` of directories mounted at another location.
    pub mounts: Vec<(String, String)>,
}

impl AgentScope {
    /// The working directory and the conversation's store.
    pub fn chat(project: Option<&Project>, conversation_id: Uuid) -> Self {
        let project_id = project.map(|project| project.id);
        let base = project.map_or_else(
            || DOCUMENTS.to_owned(),
            |project| format!("{DOCUMENTS}/{}", project.directory),
        );
        Self {
            project_id,
            conversation_id,
            roots: vec![
                base.clone(),
                format!("{HOME}/{}", conversation_store(project_id, conversation_id)),
            ],
            base,
            mounts: Vec::new(),
        }
    }

    /// A discussion worker only reaches the two directories mounted for it.
    pub fn worker(
        project_id: Uuid,
        conversation_id: Uuid,
        directories: &WorkerDirectories,
    ) -> Self {
        Self {
            project_id: Some(project_id),
            conversation_id,
            base: WORKER_OUTPUTS_DIR.to_owned(),
            roots: vec![
                WORKER_OUTPUTS_DIR.to_owned(),
                WORKER_ATTACHMENTS_DIR.to_owned(),
            ],
            mounts: vec![
                (
                    WORKER_OUTPUTS_DIR.to_owned(),
                    format!("{HOME}/{}", directories.outputs),
                ),
                (
                    WORKER_ATTACHMENTS_DIR.to_owned(),
                    format!("{HOME}/{}", directories.attachments),
                ),
            ],
        }
    }

    /// The same path in the member's container, where the file adapter runs.
    fn container_path(&self, path: &str) -> String {
        for (agent, container) in &self.mounts {
            if path == agent {
                return container.clone();
            }
            if let Some(rest) = path.strip_prefix(agent.as_str())
                && rest.starts_with('/')
            {
                return format!("{container}{rest}");
            }
        }
        path.to_owned()
    }
}

/// A stable ID per referenced file, so replaying a message captures into the same
/// directories. FNV-1a with two offsets gives 128 bits; IDs are not secrets.
pub(super) fn derived_id(run_id: Uuid, provider_item: &str, index: usize) -> Uuid {
    let mut input = run_id.as_bytes().to_vec();
    input.extend_from_slice(provider_item.as_bytes());
    input.extend_from_slice(&(index as u64).to_be_bytes());
    let mut bytes = [0u8; 16];
    for (lane, half) in bytes.chunks_mut(8).enumerate() {
        let mut hash: u64 = if lane == 0 {
            0xcbf2_9ce4_8422_2325
        } else {
            0x8422_2325_cbf2_9ce4
        };
        for byte in &input {
            hash = (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3);
        }
        half.copy_from_slice(&hash.to_be_bytes());
    }
    uuid::Builder::from_random_bytes(bytes).into_uuid()
}

/// `None` when the deadline passes first.
async fn within<T>(deadline: &mut Deadline, work: impl Future<Output = T>) -> Option<T> {
    let mut work = pin!(work);
    poll_fn(|cx| {
        if let Poll::Ready(value) = work.as_mut().poll(cx) {
            return Poll::Ready(Some(value));
        }
        deadline.as_mut().poll(cx).map(|()| None)
    })
    .await
}

impl AttachmentService {
    /// Copy the files an agent referenced in one message into the store. Missing,
    /// out-of-scope, oversized or late files are returned as `Unavailable`, so the
    /// message can always be persisted. Run this before persisting the message.
    pub async fn capture_agent_files(
        &self,
        env: &Environment,
        scope: &AgentScope,
        run_id: Uuid,
        provider_item: &str,
        references: &[String],
        mut deadline: Deadline,
    ) -> Vec<AgentAttachment> {
        let roots: Vec<String> = scope
            .roots
            .iter()
            .map(|root| scope.container_path(root))
            .collect();
        let mut expired = false;
        let mut captured = Vec::with_capacity(references.len());
        for (index, reference) in references.iter().enumerate() {
            let id = derived_id(run_id, provider_item, index);
            let name = display_name_from_reference(reference);
            let mut capture = AgentCapture::Unavailable;
            let location =
                AttachmentLocation::new(scope.project_id, scope.conversation_id, id, &name);
            let path = resolve_reference(reference, &scope.base, &scope.roots);
            if let (false, Ok(location), Ok(path)) = (expired, location, path) {
                let source = scope.container_path(&path);
                let copy = self.files.capture(env, &location, &source, &roots);
                match within(&mut deadline, copy).await {
                    Some(Ok(file)) => capture = AgentCapture::Captured(file),
                    result => {
                        expired = result.is_none();
                        // Remove whatever a failed or abandoned copy left behind.
                        let _ = self.files.delete_attachment(env, &location).await;
                    }
                }
            }
            captured.push(AgentAttachment { id, name, capture });
        }
        captured
    }

    /// Persist a completed assistant message. Files it lists in a `factory-files`
    /// block are captured first, so the message and its files appear together; the
    /// block is not part of the stored text. Repeating the message is idempotent.
    #[allow(clippy::too_many_arguments)]
    pub async fn append_message(
        &self,
        actor: EnvironmentActor,
        env: &Environment,
        run_id: Uuid,
        scope: &AgentScope,
        provider_item: &str,
        visible: &str,
        deadline: Deadline,
    ) -> Result<(), ConversationError> {
        let extracted = extract_factory_files(visible);
        let files = self
            .capture_agent_files(
                env,
                scope,
                run_id,
                provider_item,
                &extracted.references,
                deadline,
            )
            .await;
        let text = if !extracted.text.trim().is_empty() {
            extracted.text
        } else if files.is_empty() {
            visible.to_owned()
        } else {
            // The store rejects empty messages; a message that only lists files
            // shows their names.
            let names: Vec<&str> = files.iter().map(|file| file.name.as_str()).collect();
            names.join("\n")
        };
        let (message, _) = self
            .conversations
            .append_assistant(actor, env, run_id, provider_item, &text)
            .await?;
        self.record_agent_files(actor, env, run_id, message.id, files)
            .await
            .map(|_| ())
            .map_err(super::attachment_error)
    }
}
