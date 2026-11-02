//! Project requests are fenced by the current environment before persistence.
use crate::environments::EnvironmentService;
use factory_core::{
    environments::{EnvironmentActor, EnvironmentError},
    projects::*,
};
use std::sync::Arc;
use uuid::Uuid;

pub struct ProjectService {
    environments: Arc<EnvironmentService>,
    store: Arc<dyn ProjectStore>,
}
impl ProjectService {
    pub fn new(environments: Arc<EnvironmentService>, store: Arc<dyn ProjectStore>) -> Self {
        Self {
            environments,
            store,
        }
    }
    async fn current(&self, actor: EnvironmentActor) -> Result<(Uuid, i64), ProjectError> {
        let env = self
            .environments
            .require_running(actor)
            .await
            .map_err(|error| match error {
                EnvironmentError::Unauthorized => ProjectError::Unauthorized,
                EnvironmentError::Conflict | EnvironmentError::PolicyMismatch => {
                    ProjectError::Conflict
                }
                _ => ProjectError::Unavailable,
            })?;
        Ok((env.id, env.generation))
    }
    pub async fn reserve(
        &self,
        actor: EnvironmentActor,
        input: ImportProject,
    ) -> Result<(Project, bool), ProjectError> {
        input.validate()?;
        let (env, generation) = self.current(actor).await?;
        self.store.reserve(actor, env, generation, input).await
    }
    pub async fn get(&self, actor: EnvironmentActor, id: Uuid) -> Result<Project, ProjectError> {
        let (env, generation) = self.current(actor).await?;
        self.store.get(actor, env, generation, id).await
    }
    pub async fn list(
        &self,
        actor: EnvironmentActor,
        before: Option<Uuid>,
    ) -> Result<ProjectPage, ProjectError> {
        let (env, generation) = self.current(actor).await?;
        self.store.list(actor, env, generation, before).await
    }
    /// Synchronize one provider-verified page and return already-present IDs.
    /// Caller must never pass repository metadata supplied by a browser.
    pub async fn sync_repository_metadata(
        &self,
        actor: EnvironmentActor,
        repositories: &[RepositoryMetadata],
    ) -> Result<Vec<i64>, ProjectError> {
        validate_repository_metadata(repositories)?;
        let (env, generation) = self.current(actor).await?;
        self.store
            .sync_repository_metadata(actor, env, generation, repositories)
            .await
    }
    /// Inspect one bounded batch before workspace start, including when stopped.
    /// Reload after processing every batch; only an empty result permits startup.
    pub async fn recoverable(
        &self,
        actor: EnvironmentActor,
    ) -> Result<Vec<RecoverableImport>, ProjectError> {
        self.store.recoverable(actor).await
    }
}
