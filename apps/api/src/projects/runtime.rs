//! Composition and bounded execution of repository operations.
use factory_adapters::projects::{
    DockerProjectRuntime, ProjectDirectory, ProjectRuntimeError, SshKeyStatus,
};
use factory_application::{
    environments::{EnvironmentAvailability, EnvironmentService},
    identity::RepositoryAccessService,
    projects::ProjectService,
};
use factory_core::{
    environments::{Environment, EnvironmentActor},
    identity::{
        RepositoryAccessError, RepositoryActor, RepositoryPublicKey, RepositoryPublicKeySource,
    },
    projects::{
        ImportProject, Project, ProjectError, ProjectState, ProjectStore, RepositoryMetadata,
    },
};
use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
};
use tokio::sync::Semaphore;
use uuid::Uuid;

pub struct KeySource {
    pub environments: Arc<EnvironmentService>,
    pub runtime: Arc<dyn ProjectDriver>,
}

/// Container operations are a boundary: HTTP tests can exercise authorization and
/// durable orchestration without starting Docker or contacting a provider.
#[async_trait::async_trait]
pub trait ProjectDriver: Send + Sync {
    /// Read-only inspection: never creates a key, lock, directory or Git config.
    async fn setup_status(&self, env: &Environment) -> Result<bool, ProjectRuntimeError>;
    async fn public_key_status(
        &self,
        env: &Environment,
    ) -> Result<SshKeyStatus, ProjectRuntimeError>;
    async fn mark_key_registered(
        &self,
        env: &Environment,
        expected: &str,
    ) -> Result<(), ProjectRuntimeError>;
    async fn rotate_missing_key(
        &self,
        env: &Environment,
        expected: &str,
    ) -> Result<SshKeyStatus, ProjectRuntimeError>;
    async fn configure_git_identity(
        &self,
        env: &Environment,
        login: &str,
        email: &str,
    ) -> Result<(), ProjectRuntimeError>;
    async fn delete_project(
        &self,
        _env: &Environment,
        _project: &Project,
    ) -> Result<(), ProjectRuntimeError> {
        Err(ProjectRuntimeError::Unavailable)
    }
    async fn clean_import_stage(
        &self,
        _env: &Environment,
        _project: &Project,
    ) -> Result<(), ProjectRuntimeError> {
        Err(ProjectRuntimeError::Unavailable)
    }
    async fn destination_available(
        &self,
        env: &Environment,
        directory: &str,
    ) -> Result<bool, ProjectRuntimeError>;
    async fn clone_project(
        &self,
        env: &Environment,
        project: &Project,
    ) -> Result<(), ProjectRuntimeError>;
    async fn inspect_project(
        &self,
        env: &Environment,
        project: &Project,
    ) -> Result<ProjectDirectory, ProjectRuntimeError>;
}
#[async_trait::async_trait]
impl ProjectDriver for DockerProjectRuntime {
    async fn setup_status(&self, env: &Environment) -> Result<bool, ProjectRuntimeError> {
        self.setup_status(env).await
    }
    async fn public_key_status(
        &self,
        env: &Environment,
    ) -> Result<SshKeyStatus, ProjectRuntimeError> {
        self.public_key_status(env).await
    }
    async fn mark_key_registered(
        &self,
        env: &Environment,
        expected: &str,
    ) -> Result<(), ProjectRuntimeError> {
        self.mark_key_registered(env, expected).await
    }
    async fn rotate_missing_key(
        &self,
        env: &Environment,
        expected: &str,
    ) -> Result<SshKeyStatus, ProjectRuntimeError> {
        self.rotate_missing_key(env, expected).await
    }
    async fn configure_git_identity(
        &self,
        env: &Environment,
        login: &str,
        email: &str,
    ) -> Result<(), ProjectRuntimeError> {
        self.configure_git_identity(env, login, email).await
    }
    async fn delete_project(
        &self,
        env: &Environment,
        project: &Project,
    ) -> Result<(), ProjectRuntimeError> {
        self.delete_project(env, project).await
    }
    async fn clean_import_stage(
        &self,
        env: &Environment,
        project: &Project,
    ) -> Result<(), ProjectRuntimeError> {
        self.clean_import_stage(env, project).await
    }
    async fn destination_available(
        &self,
        env: &Environment,
        directory: &str,
    ) -> Result<bool, ProjectRuntimeError> {
        self.destination_available(env, directory).await
    }
    async fn clone_project(
        &self,
        env: &Environment,
        project: &Project,
    ) -> Result<(), ProjectRuntimeError> {
        self.clone_project(env, project).await
    }
    async fn inspect_project(
        &self,
        env: &Environment,
        project: &Project,
    ) -> Result<ProjectDirectory, ProjectRuntimeError> {
        self.inspect_project(env, project).await
    }
}
impl KeySource {
    async fn environment(
        &self,
        actor: RepositoryActor,
    ) -> Result<Environment, RepositoryAccessError> {
        let env = self
            .environments
            .require_running(actor.actor)
            .await
            .map_err(|_| RepositoryAccessError::Unavailable)?;
        if env.id != actor.environment_id || env.generation != actor.generation {
            return Err(RepositoryAccessError::Conflict);
        }
        Ok(env)
    }
}
fn key_error(e: ProjectRuntimeError) -> RepositoryAccessError {
    match e {
        ProjectRuntimeError::Invalid => RepositoryAccessError::InvalidInput,
        ProjectRuntimeError::Conflict => RepositoryAccessError::Conflict,
        _ => RepositoryAccessError::Unavailable,
    }
}
#[async_trait::async_trait]
impl RepositoryPublicKeySource for KeySource {
    async fn public_key_status(
        &self,
        actor: RepositoryActor,
    ) -> Result<RepositoryPublicKey, RepositoryAccessError> {
        let env = self.environment(actor).await?;
        let value = self
            .runtime
            .public_key_status(&env)
            .await
            .map_err(key_error)?;
        self.environment(actor).await?;
        Ok(RepositoryPublicKey {
            public_key: value.public_key,
            registered: value.registered,
        })
    }
    async fn mark_registered(
        &self,
        actor: RepositoryActor,
        expected_key: &str,
    ) -> Result<(), RepositoryAccessError> {
        let env = self.environment(actor).await?;
        self.runtime
            .mark_key_registered(&env, expected_key)
            .await
            .map_err(key_error)?;
        self.environment(actor).await?;
        Ok(())
    }
    async fn rotate_missing_key(
        &self,
        actor: RepositoryActor,
        expected_key: &str,
    ) -> Result<RepositoryPublicKey, RepositoryAccessError> {
        let env = self.environment(actor).await?;
        let value = self
            .runtime
            .rotate_missing_key(&env, expected_key)
            .await
            .map_err(key_error)?;
        self.environment(actor).await?;
        Ok(RepositoryPublicKey {
            public_key: value.public_key,
            registered: value.registered,
        })
    }
}

pub struct ProjectsRuntime {
    pub environments: Arc<EnvironmentService>,
    pub projects: Arc<ProjectService>,
    pub store: Arc<dyn ProjectStore>,
    pub docker: Arc<dyn ProjectDriver>,
    pub access: Option<Arc<RepositoryAccessService>>,
    pub availability: EnvironmentAvailability,
    pub installation: Arc<dyn Send + Sync>,
    pub jobs: Arc<Semaphore>,
    pub active: Arc<Mutex<HashSet<Uuid>>>,
}
struct ActiveImport {
    id: Uuid,
    active: Arc<Mutex<HashSet<Uuid>>>,
}
impl Drop for ActiveImport {
    fn drop(&mut self) {
        if let Ok(mut active) = self.active.lock() {
            active.remove(&self.id);
        }
    }
}
impl ProjectsRuntime {
    pub fn view(&self, project: Project) -> super::contracts::ProjectView {
        let uncertain = project.state == ProjectState::Importing
            && self
                .active
                .lock()
                .map_or(true, |active| !active.contains(&project.id));
        let mut view: super::contracts::ProjectView = project.into();
        if uncertain {
            view.state = super::contracts::ProjectStateView::Uncertain;
        }
        view
    }
    pub async fn actor(
        &self,
        actor: EnvironmentActor,
    ) -> Result<(Environment, RepositoryActor), RepositoryAccessError> {
        let env = self
            .environments
            .require_running(actor)
            .await
            .map_err(|_| RepositoryAccessError::Unavailable)?;
        Ok((
            env.clone(),
            RepositoryActor {
                actor,
                environment_id: env.id,
                generation: env.generation,
            },
        ))
    }
    pub fn access(&self) -> Result<&Arc<RepositoryAccessService>, RepositoryAccessError> {
        self.access
            .as_ref()
            .ok_or(RepositoryAccessError::Unavailable)
    }
    /// A connection is reported only after a read-only inspection confirms the
    /// key receipt and Git identity. Repair requires a separate explicit POST.
    pub async fn connection(
        &self,
        actor: EnvironmentActor,
    ) -> Result<Option<factory_core::identity::RepositoryConnection>, RepositoryAccessError> {
        let (env, repo_actor) = self.actor(actor).await?;
        let connection = self.access()?.connection(repo_actor).await?;
        if connection.is_none() {
            return Ok(None);
        }
        let ready = self.docker.setup_status(&env).await.map_err(key_error)?;
        let (current, _) = self.actor(actor).await?;
        if current.id != env.id || current.generation != env.generation {
            return Err(RepositoryAccessError::Conflict);
        }
        // A disconnect or reconnect may race the filesystem inspection. Never
        // publish the old connection after its credential has changed.
        if self.access()?.connection(repo_actor).await? != connection {
            return Err(RepositoryAccessError::Conflict);
        }
        Ok(if ready { connection } else { None })
    }
    pub async fn ensure_identity(
        &self,
        actor: EnvironmentActor,
    ) -> Result<(), RepositoryAccessError> {
        let (env, repo_actor) = self.actor(actor).await?;
        let access = self.access()?;
        let connection = access
            .connection(repo_actor)
            .await?
            .ok_or(RepositoryAccessError::ProviderUnauthorized)?;
        let key = self
            .docker
            .public_key_status(&env)
            .await
            .map_err(key_error)?;
        if !key.registered {
            return Err(RepositoryAccessError::Conflict);
        }
        self.docker
            .configure_git_identity(&env, &connection.login, &connection.email)
            .await
            .map_err(key_error)?;
        self.actor(actor).await.and_then(|(current, _)| {
            if current.id == env.id && current.generation == env.generation {
                Ok(())
            } else {
                Err(RepositoryAccessError::Conflict)
            }
        })
    }
    pub async fn import(
        self: &Arc<Self>,
        actor: EnvironmentActor,
        repository_id: i64,
        page: u32,
    ) -> Result<Project, RepositoryAccessError> {
        let (env, repo_actor) = self.actor(actor).await?;
        let repositories = self.access()?.repositories(repo_actor, page).await?;
        let repository = repositories
            .items
            .into_iter()
            .find(|item| item.id == repository_id)
            .ok_or(RepositoryAccessError::Conflict)?;
        let input = ImportProject {
            repository_id,
            full_name: repository.full_name,
            operation_id: Uuid::new_v4(),
        };
        let directory = input.directory().map_err(project_access_error)?;
        let already_imported = self
            .projects
            .sync_repository_metadata(
                actor,
                &[RepositoryMetadata {
                    repository_id,
                    full_name: input.full_name.clone(),
                }],
            )
            .await
            .map_err(project_access_error)?
            .contains(&repository_id);
        if !already_imported
            && !self
                .docker
                .destination_available(&env, directory)
                .await
                .map_err(key_error)?
        {
            return Err(RepositoryAccessError::DestinationExists);
        }
        self.ensure_identity(actor).await?;
        let (current, _) = self.actor(actor).await?;
        if current.id != env.id || current.generation != env.generation {
            return Err(RepositoryAccessError::Conflict);
        }
        let permit = self
            .jobs
            .clone()
            .try_acquire_owned()
            .map_err(|_| RepositoryAccessError::Unavailable)?;
        let (project, fresh) = self
            .projects
            .reserve(actor, input)
            .await
            .map_err(project_access_error)?;
        if fresh {
            self.spawn_clone(actor, project.clone(), permit).await?;
        }
        Ok(project)
    }
    async fn spawn_clone(
        self: &Arc<Self>,
        actor: EnvironmentActor,
        project: Project,
        permit: tokio::sync::OwnedSemaphorePermit,
    ) -> Result<(), RepositoryAccessError> {
        let env = self
            .environments
            .require_running(actor)
            .await
            .map_err(|_| RepositoryAccessError::Unavailable)?;
        if env.id != project.environment_id
            || env.generation != project.environment_generation
            || project.state != ProjectState::Importing
        {
            return Err(RepositoryAccessError::Conflict);
        }
        self.active
            .lock()
            .map_err(|_| RepositoryAccessError::Unavailable)?
            .insert(project.id);
        let active = ActiveImport {
            id: project.id,
            active: self.active.clone(),
        };
        let owner = self.clone();
        tokio::spawn(async move {
            let _active = active;
            let _permit = permit;
            let _installation = owner.installation.clone();
            let result = owner.docker.clone_project(&env, &project).await;
            // Never settle an uncertain or failed clone: it can have descendants.
            let current = owner.environments.require_running(actor).await;
            let verified = current
                .is_ok_and(|current| current.id == env.id && current.generation == env.generation);
            if result.is_ok() && !owner.availability.is_failed() && verified {
                if owner
                    .store
                    .settle(
                        actor,
                        env.id,
                        env.generation,
                        project.id,
                        project.operation_id,
                        true,
                    )
                    .await
                    .is_err()
                {
                    tracing::warn!(event = "projects.import.settlement_unconfirmed");
                }
            } else {
                tracing::warn!(event = "projects.import.unconfirmed");
            }
        });
        Ok(())
    }
    pub async fn delete(
        self: &Arc<Self>,
        actor: EnvironmentActor,
        id: Uuid,
    ) -> Result<(), ProjectError> {
        let env = self
            .environments
            .require_running(actor)
            .await
            .map_err(|_| ProjectError::Unavailable)?;
        let project = self
            .store
            .begin_delete(actor, env.id, env.generation, id)
            .await?;
        self.docker
            .delete_project(&env, &project)
            .await
            .map_err(|error| match error {
                ProjectRuntimeError::Conflict => ProjectError::Conflict,
                ProjectRuntimeError::Invalid => ProjectError::Invalid,
                _ => ProjectError::Unavailable,
            })?;
        if self.availability.is_failed() {
            return Err(ProjectError::Unavailable);
        }
        self.store
            .finish_delete(actor, env.id, env.generation, id)
            .await
    }
    pub async fn retry(
        self: &Arc<Self>,
        actor: EnvironmentActor,
        id: Uuid,
    ) -> Result<Project, ProjectError> {
        let env = self
            .environments
            .require_running(actor)
            .await
            .map_err(|_| ProjectError::Unavailable)?;
        let project = self.projects.get(actor, id).await?;
        if project.state != ProjectState::Failed {
            return Err(ProjectError::Conflict);
        }
        match self
            .docker
            .inspect_project(&env, &project)
            .await
            .map_err(|_| ProjectError::Conflict)?
        {
            ProjectDirectory::Ready => {
                self.store
                    .confirm_recovered(actor, env.id, env.generation, id, project.operation_id)
                    .await
            }
            status @ (ProjectDirectory::Missing | ProjectDirectory::Staged) => {
                if status == ProjectDirectory::Staged {
                    self.docker
                        .clean_import_stage(&env, &project)
                        .await
                        .map_err(|_| ProjectError::Unavailable)?;
                }
                self.ensure_identity(actor)
                    .await
                    .map_err(|_| ProjectError::Unavailable)?;
                let permit = self
                    .jobs
                    .clone()
                    .try_acquire_owned()
                    .map_err(|_| ProjectError::Unavailable)?;
                let next = self
                    .store
                    .retry(
                        actor,
                        env.id,
                        env.generation,
                        id,
                        project.operation_id,
                        Uuid::new_v4(),
                    )
                    .await?;
                self.spawn_clone(actor, next.clone(), permit)
                    .await
                    .map_err(|_| ProjectError::Unavailable)?;
                Ok(next)
            }
        }
    }
}
fn project_access_error(e: ProjectError) -> RepositoryAccessError {
    match e {
        ProjectError::Unauthorized => RepositoryAccessError::Unauthorized,
        ProjectError::Invalid => RepositoryAccessError::InvalidInput,
        ProjectError::Conflict | ProjectError::NotFound => RepositoryAccessError::Conflict,
        ProjectError::DestinationExists => RepositoryAccessError::DestinationExists,
        ProjectError::InUse => RepositoryAccessError::Conflict,
        ProjectError::Unavailable => RepositoryAccessError::Unavailable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn aborted_or_panicking_job_does_not_remain_reported_as_active() {
        let id = Uuid::new_v4();
        let active = Arc::new(Mutex::new(HashSet::from([id])));
        let guard = ActiveImport {
            id,
            active: active.clone(),
        };
        let task = tokio::spawn(async move {
            let _guard = guard;
            panic!("synthetic task failure");
        });
        assert!(task.await.is_err());
        assert!(!active.lock().unwrap().contains(&id));
    }
}
