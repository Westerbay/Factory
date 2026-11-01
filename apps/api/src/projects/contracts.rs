use factory_core::{
    identity::{AccessibleRepository, RepositoryConnection},
    projects::{Project, ProjectState, RecoverableImport},
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProjectView {
    pub id: Uuid,
    pub full_name: String,
    pub state: ProjectStateView,
    /// Comparison with the provider name used at clone; this does not inspect Git remotes.
    pub renamed_since_clone: bool,
}
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProjectVerificationView {
    pub status: ProjectVerificationStatus,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProjectVerificationStatus {
    Matched,
    Changed,
}
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProjectStateView {
    Importing,
    Uncertain,
    Ready,
    Failed,
    Deleting,
}
impl From<Project> for ProjectView {
    fn from(value: Project) -> Self {
        let renamed_since_clone = value.full_name != value.cloned_full_name;
        Self {
            id: value.id,
            full_name: value.full_name,
            renamed_since_clone,
            state: match value.state {
                ProjectState::Importing => ProjectStateView::Importing,
                ProjectState::Ready => ProjectStateView::Ready,
                ProjectState::Failed => ProjectStateView::Failed,
                ProjectState::Deleting => ProjectStateView::Deleting,
            },
        }
    }
}
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProjectPageView {
    pub items: Vec<ProjectView>,
    pub next_cursor: Option<Uuid>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectQuery {
    pub before: Option<Uuid>,
}
#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImportBody {
    pub repository_id: i64,
    pub page: u32,
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RepositoryPageBody {
    pub page: u32,
}
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryView {
    pub id: i64,
    pub full_name: String,
    pub private: bool,
}
impl From<AccessibleRepository> for RepositoryView {
    fn from(value: AccessibleRepository) -> Self {
        Self {
            id: value.id,
            full_name: value.full_name,
            private: value.private,
        }
    }
}
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryPageView {
    pub items: Vec<RepositoryView>,
    pub next_page: Option<u32>,
}
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AuthorizationView {
    pub authorization_url: String,
}
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionView {
    pub status: &'static str,
    pub login: Option<String>,
    pub email: Option<String>,
}
impl From<Option<RepositoryConnection>> for ConnectionView {
    fn from(value: Option<RepositoryConnection>) -> Self {
        match value {
            Some(v) => Self {
                status: "connected",
                login: Some(v.login),
                email: Some(v.email),
            },
            None => Self {
                status: "disconnected",
                login: None,
                email: None,
            },
        }
    }
}
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RecoverableImportView {
    pub project_id: Uuid,
}
impl From<RecoverableImport> for RecoverableImportView {
    fn from(value: RecoverableImport) -> Self {
        Self {
            project_id: value.project_id,
        }
    }
}
