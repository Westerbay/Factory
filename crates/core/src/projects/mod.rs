//! Private project records and durable import reservations.
use crate::environments::EnvironmentActor;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ProjectError {
    #[error("An active session is required")]
    Unauthorized,
    #[error("Project not found")]
    NotFound,
    #[error("Project state changed")]
    Conflict,
    #[error("A project directory with this name already exists")]
    DestinationExists,
    #[error("Project has an active or uncertain operation")]
    InUse,
    #[error("Invalid project input")]
    Invalid,
    #[error("Project service is unavailable")]
    Unavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProjectState {
    Importing,
    Ready,
    Failed,
    Deleting,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Project {
    pub id: Uuid,
    pub user_id: Uuid,
    /// Stable provider repository identity, independent of name and owner changes.
    pub repository_id: i64,
    pub full_name: String,
    /// Name used for the current import operation; it changes only on a new retry.
    /// Compare with full_name to flag a potentially stale managed Git origin.
    pub cloned_full_name: String,
    /// Stable repository directory name under the owner's Documents.
    pub directory: String,
    pub state: ProjectState,
    pub operation_id: Uuid,
    pub environment_id: Uuid,
    pub environment_generation: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug)]
pub struct ImportProject {
    pub repository_id: i64,
    pub full_name: String,
    pub operation_id: Uuid,
}
pub fn validate_project_directory(name: &str) -> Result<(), ProjectError> {
    if name.is_empty()
        || name.len() > 100
        || matches!(name, "." | "..")
        || !name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.'))
    {
        return Err(ProjectError::Invalid);
    }
    Ok(())
}

fn validate_repository(repository_id: i64, full_name: &str) -> Result<(), ProjectError> {
    let (owner, name) = full_name.split_once('/').ok_or(ProjectError::Invalid)?;
    if repository_id <= 0
        || full_name.len() > 256
        || owner.is_empty()
        || owner.len() > 39
        || validate_project_directory(name).is_err()
        || owner.starts_with('-')
        || owner.ends_with('-')
        || !owner
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-')
    {
        return Err(ProjectError::Invalid);
    }
    Ok(())
}
impl ImportProject {
    pub fn directory(&self) -> Result<&str, ProjectError> {
        self.validate()?;
        self.full_name
            .split_once('/')
            .map(|(_, name)| name)
            .ok_or(ProjectError::Invalid)
    }

    pub fn validate(&self) -> Result<(), ProjectError> {
        if self.operation_id.is_nil() {
            return Err(ProjectError::Invalid);
        }
        validate_repository(self.repository_id, &self.full_name)
    }
}

/// Provider-verified metadata; never construct from browser-supplied names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepositoryMetadata {
    pub repository_id: i64,
    pub full_name: String,
}

pub fn validate_repository_metadata(
    repositories: &[RepositoryMetadata],
) -> Result<(), ProjectError> {
    if repositories.len() > MAX_REPOSITORY_LOOKUP {
        return Err(ProjectError::Invalid);
    }
    let mut names = std::collections::BTreeMap::new();
    for repository in repositories {
        validate_repository(repository.repository_id, &repository.full_name)?;
        if names
            .insert(repository.repository_id, &repository.full_name)
            .is_some_and(|name| name != &repository.full_name)
        {
            return Err(ProjectError::Invalid);
        }
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectPage {
    pub items: Vec<Project>,
    pub next_cursor: Option<Uuid>,
}

/// Maximum provider page accepted by the internal import-picker lookup.
pub const MAX_REPOSITORY_LOOKUP: usize = 100;

/// Minimal internal receipt for deciding whether workspace recovery is required.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoverableImport {
    pub project_id: Uuid,
    pub operation_id: Uuid,
    pub environment_id: Uuid,
    pub environment_generation: i64,
}

/// Maximum receipts returned per recovery batch. Callers must reload after each batch.
pub const MAX_RECOVERABLE_IMPORTS: usize = 200;

#[async_trait]
pub trait ProjectStore: Send + Sync {
    /// Fence new project activity before deleting any files. Repeated calls resume deletion.
    async fn begin_delete(
        &self,
        _actor: EnvironmentActor,
        _environment_id: Uuid,
        _generation: i64,
        _id: Uuid,
    ) -> Result<Project, ProjectError> {
        Err(ProjectError::Unavailable)
    }
    /// Remove the record and its chats only after filesystem deletion is confirmed.
    async fn finish_delete(
        &self,
        _actor: EnvironmentActor,
        _environment_id: Uuid,
        _generation: i64,
        _id: Uuid,
    ) -> Result<(), ProjectError> {
        Err(ProjectError::Unavailable)
    }
    /// Only `fresh=true` permits starting a clone; retries return the existing receipt.
    async fn reserve(
        &self,
        actor: EnvironmentActor,
        environment_id: Uuid,
        generation: i64,
        input: ImportProject,
    ) -> Result<(Project, bool), ProjectError>;
    async fn get(
        &self,
        actor: EnvironmentActor,
        environment_id: Uuid,
        generation: i64,
        id: Uuid,
    ) -> Result<Project, ProjectError>;
    async fn list(
        &self,
        actor: EnvironmentActor,
        environment_id: Uuid,
        generation: i64,
        before: Option<Uuid>,
    ) -> Result<ProjectPage, ProjectError>;
    /// Synchronize owned project names from a bounded, provider-verified page.
    /// Returns existing IDs only; never call with browser-provided metadata.
    async fn sync_repository_metadata(
        &self,
        actor: EnvironmentActor,
        environment_id: Uuid,
        generation: i64,
        repositories: &[RepositoryMetadata],
    ) -> Result<Vec<i64>, ProjectError>;
    /// Settle only the still-current operation. A failure means cleanup is known safe.
    async fn settle(
        &self,
        actor: EnvironmentActor,
        environment_id: Uuid,
        generation: i64,
        id: Uuid,
        operation_id: Uuid,
        success: bool,
    ) -> Result<Project, ProjectError>;
    /// Recover an uncertain import only with a verified later stopped-environment observation.
    /// This marks the operation failed; callers must still reconcile the owned staging path
    /// before retrying, and must never use a mere generation change as stop proof.
    async fn recover_stopped(
        &self,
        actor: EnvironmentActor,
        environment_id: Uuid,
        stopped_generation: i64,
        observed_at: DateTime<Utc>,
        id: Uuid,
    ) -> Result<Project, ProjectError>;
    /// Inspect a bounded batch of unsettled imports for this authenticated owner,
    /// including while stopped. A non-empty batch is not the complete inventory:
    /// callers must process it and reload until an empty result before starting.
    /// Errors must block startup.
    async fn recoverable(
        &self,
        actor: EnvironmentActor,
    ) -> Result<Vec<RecoverableImport>, ProjectError>;
    /// Internal reconciliation only: the runtime must first verify the final directory
    /// and its owner marker against the original operation. Never expose this directly
    /// to a browser or accept an unverified marker as proof.
    async fn confirm_recovered(
        &self,
        actor: EnvironmentActor,
        environment_id: Uuid,
        generation: i64,
        id: Uuid,
        original_operation: Uuid,
    ) -> Result<Project, ProjectError>;
    /// Caller must reconcile its owned staging directory before retrying a failed import.
    async fn retry(
        &self,
        actor: EnvironmentActor,
        environment_id: Uuid,
        generation: i64,
        id: Uuid,
        previous_operation: Uuid,
        new_operation: Uuid,
    ) -> Result<Project, ProjectError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_repository_identity_and_untrusted_name() {
        let input = ImportProject {
            repository_id: 1,
            full_name: "owner/repo".into(),
            operation_id: Uuid::new_v4(),
        };
        assert!(input.validate().is_ok());
        for bad in [
            "",
            "/repo",
            "owner/",
            "owner/repo/nested",
            "../repo",
            "owner/..",
            "owner/repo?x",
            "owner/repo;touch",
            "--owner/repo",
            "owner/na\nme",
        ] {
            let mut candidate = input.clone();
            candidate.full_name = bad.into();
            assert_eq!(candidate.validate(), Err(ProjectError::Invalid));
        }
        let mut candidate = input;
        candidate.repository_id = 0;
        assert_eq!(candidate.validate(), Err(ProjectError::Invalid));
    }
}
