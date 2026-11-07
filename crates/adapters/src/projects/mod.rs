//! Typed, bounded project operations inside an owned Docker environment.
//!
//! The caller must authorize the user and current environment generation before
//! calling this adapter and must retain durable import receipts for uncertain work.
mod feature_delivery;
mod features;

pub use features::FeatureRemote;

use crate::environments::{
    DockerEnvironmentRuntime,
    process::{FixedProcess, ProcessStartError},
};
use factory_core::{
    environments::Environment,
    projects::{ImportProject, Project, validate_project_directory},
};
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use tokio::io::AsyncWriteExt;

const LIMIT: usize = 8192;
const SHORT_TIMEOUT: Duration = Duration::from_secs(30);
const CLONE_TIMEOUT: Duration = Duration::from_secs(270);

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ProjectRuntimeError {
    #[error("Invalid project request")]
    Invalid,
    #[error("Project destination or key conflicts with existing files")]
    Conflict,
    #[error("Task changes exceed the approved scope")]
    ScopeViolation,
    #[error("The combined diff cannot be displayed")]
    DiffUnrenderable,
    #[error("Project operation is unavailable")]
    Unavailable,
    #[error("Project operation may still be running")]
    Uncertain,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProjectDirectory {
    Ready,
    Missing,
    /// An operation-owned staging receipt exists; this does not prove it stopped.
    Staged,
}

#[derive(Debug, Deserialize)]
pub struct SshKeyStatus {
    pub public_key: String,
    pub registered: bool,
}

pub struct DockerProjectRuntime(Arc<DockerEnvironmentRuntime>);

impl DockerProjectRuntime {
    pub fn new(runtime: Arc<DockerEnvironmentRuntime>) -> Self {
        Self(runtime)
    }

    /// Read-only readiness check; it never creates keys, locks, markers or config.
    pub async fn setup_status(&self, env: &Environment) -> Result<bool, ProjectRuntimeError> {
        parse_setup_status(
            self.run(env, json!({"op":"setup_status"}), SHORT_TIMEOUT)
                .await?,
        )
    }

    /// Return only the public key and whether this exact key has a durable
    /// confirmed-registration receipt. The private key remains in the owned home.
    pub async fn public_key_status(
        &self,
        env: &Environment,
    ) -> Result<SshKeyStatus, ProjectRuntimeError> {
        let value = self.run(env, json!({"op":"key"}), SHORT_TIMEOUT).await?;
        parse_key(value)
    }

    /// Record registration only after GitHub confirms this exact public key.
    pub async fn mark_key_registered(
        &self,
        env: &Environment,
        expected: &str,
    ) -> Result<(), ProjectRuntimeError> {
        let state = parse_key(
            self.run(
                env,
                json!({"op":"mark", "expected":expected}),
                SHORT_TIMEOUT,
            )
            .await?,
        )?;
        if state.registered && state.public_key == expected {
            Ok(())
        } else {
            Err(ProjectRuntimeError::Uncertain)
        }
    }

    /// Use only after exhaustive GitHub inspection confirms this exact previously
    /// registered key is absent. A successful replacement is not yet registered.
    pub async fn rotate_missing_key(
        &self,
        env: &Environment,
        expected: &str,
    ) -> Result<SshKeyStatus, ProjectRuntimeError> {
        let state = parse_key(
            self.run(
                env,
                json!({"op":"rotate", "expected":expected}),
                SHORT_TIMEOUT,
            )
            .await?,
        )?;
        if !state.registered && state.public_key != expected {
            Ok(state)
        } else {
            Err(ProjectRuntimeError::Uncertain)
        }
    }

    /// Set defaults in an app-owned included Git config; explicit user config wins.
    pub async fn configure_git_identity(
        &self,
        env: &Environment,
        login: &str,
        email: &str,
    ) -> Result<(), ProjectRuntimeError> {
        if login.is_empty() || login.len() > 39 || email.is_empty() || email.len() > 320 {
            return Err(ProjectRuntimeError::Invalid);
        }
        self.run(
            env,
            json!({"op":"identity", "name":login, "email":email}),
            SHORT_TIMEOUT,
        )
        .await
        .and_then(|value| {
            if value.as_str() == Some("configured") {
                Ok(())
            } else {
                Err(ProjectRuntimeError::Uncertain)
            }
        })
    }

    /// Read-only check for any existing entry, including symlinks.
    pub async fn destination_available(
        &self,
        env: &Environment,
        directory: &str,
    ) -> Result<bool, ProjectRuntimeError> {
        validate_project_directory(directory).map_err(|_| ProjectRuntimeError::Invalid)?;
        match self
            .run(
                env,
                json!({"op":"destination_status", "directory":directory}),
                SHORT_TIMEOUT,
            )
            .await?
            .as_str()
        {
            Some("available") => Ok(true),
            Some("exists") => Ok(false),
            _ => Err(ProjectRuntimeError::Uncertain),
        }
    }

    /// A fresh, durable reservation is required. Never retry an uncertain launch
    /// without the separate observed-stopped recovery and staging reconciliation.
    pub async fn clone_project(
        &self,
        env: &Environment,
        project: &Project,
    ) -> Result<(), ProjectRuntimeError> {
        self.run(env, clone_request(env, project)?, CLONE_TIMEOUT)
            .await
            .and_then(|value| {
                if value.as_str() == Some("ready") {
                    Ok(())
                } else {
                    Err(ProjectRuntimeError::Uncertain)
                }
            })
    }

    /// Deletes only the verified, application-owned clone and import staging area.
    pub async fn delete_project(
        &self,
        env: &Environment,
        project: &Project,
    ) -> Result<(), ProjectRuntimeError> {
        validate_project(env, project)?;
        if project.state != factory_core::projects::ProjectState::Deleting {
            return Err(ProjectRuntimeError::Invalid);
        }
        let value = self.run(env, json!({"op":"delete", "project":project.id.to_string(), "operation":project.operation_id.to_string(), "directory":project.directory}), CLONE_TIMEOUT).await?;
        if value.as_str() == Some("deleted") {
            Ok(())
        } else {
            Err(ProjectRuntimeError::Uncertain)
        }
    }

    /// Remove only the stopped import's marked temporary checkout before retry.
    pub async fn clean_import_stage(
        &self,
        env: &Environment,
        project: &Project,
    ) -> Result<(), ProjectRuntimeError> {
        validate_project(env, project)?;
        if project.state != factory_core::projects::ProjectState::Failed {
            return Err(ProjectRuntimeError::Invalid);
        }
        let value = self.run(env, json!({"op":"clean_stage", "project":project.id.to_string(), "operation":project.operation_id.to_string()}), CLONE_TIMEOUT).await?;
        if value.as_str() == Some("cleaned") {
            Ok(())
        } else {
            Err(ProjectRuntimeError::Uncertain)
        }
    }

    /// Checks both the directory and its operation marker without following links.
    pub async fn inspect_project(
        &self,
        env: &Environment,
        project: &Project,
    ) -> Result<ProjectDirectory, ProjectRuntimeError> {
        validate_project(env, project)?;
        match self
            .run(
                env,
                json!({"op":"inspect", "project":project.id.to_string(), "operation":project.operation_id.to_string(), "directory":project.directory}),
                SHORT_TIMEOUT,
            )
            .await?
            .as_str()
        {
            Some("ready") => Ok(ProjectDirectory::Ready),
            Some("missing") => Ok(ProjectDirectory::Missing),
            Some("staged") => Ok(ProjectDirectory::Staged),
            _ => Err(ProjectRuntimeError::Uncertain),
        }
    }

    async fn run(
        &self,
        env: &Environment,
        request: Value,
        timeout: Duration,
    ) -> Result<Value, ProjectRuntimeError> {
        self.run_bounded(env, request, timeout, LIMIT, LIMIT).await
    }

    async fn run_bounded(
        &self,
        env: &Environment,
        request: Value,
        timeout: Duration,
        request_limit: usize,
        response_limit: usize,
    ) -> Result<Value, ProjectRuntimeError> {
        let payload = serde_json::to_vec(&request).map_err(|_| ProjectRuntimeError::Invalid)?;
        if payload.len() > request_limit {
            return Err(ProjectRuntimeError::Invalid);
        }
        let mut process = self
            .0
            .start_process_checked(env, FixedProcess::ProjectController)
            .await
            .map_err(|error| match error {
                ProcessStartError::PreSubmission(_) => ProjectRuntimeError::Unavailable,
                ProcessStartError::Submitted(_) => ProjectRuntimeError::Uncertain,
            })?;
        let result = tokio::time::timeout(timeout, async {
            process
                .input
                .write_all(&payload)
                .await
                .map_err(|_| ProjectRuntimeError::Uncertain)?;
            process
                .input
                .shutdown()
                .await
                .map_err(|_| ProjectRuntimeError::Uncertain)?;
            let mut output = Vec::new();
            while let Some(chunk) = process.output.next().await {
                let chunk = chunk.map_err(|_| ProjectRuntimeError::Uncertain)?;
                let bytes = chunk.as_ref();
                if output.len().saturating_add(bytes.len()) > response_limit {
                    return Err(ProjectRuntimeError::Uncertain);
                }
                output.extend_from_slice(bytes);
            }
            Ok(output)
        })
        .await
        .map_err(|_| ProjectRuntimeError::Uncertain)??;
        drop(process.input);
        drop(process.output);
        let exit = self
            .0
            .wait_process_exit(&process.id, Duration::from_secs(10))
            .await
            .map_err(|_| ProjectRuntimeError::Uncertain)?;
        if exit != 0 {
            return Err(ProjectRuntimeError::Uncertain);
        }
        parse_response(&result)
    }
}

fn validate_project(env: &Environment, project: &Project) -> Result<(), ProjectRuntimeError> {
    if project.user_id != env.user_id
        || project.environment_id != env.id
        || validate_project_directory(&project.directory).is_err()
    {
        return Err(ProjectRuntimeError::Invalid);
    }
    ImportProject {
        repository_id: project.repository_id,
        full_name: project.cloned_full_name.clone(),
        operation_id: project.operation_id,
    }
    .validate()
    .map_err(|_| ProjectRuntimeError::Invalid)
}

fn clone_request(env: &Environment, project: &Project) -> Result<Value, ProjectRuntimeError> {
    validate_project(env, project)?;
    if project.environment_generation != env.generation
        || project.state != factory_core::projects::ProjectState::Importing
    {
        return Err(ProjectRuntimeError::Invalid);
    }
    Ok(json!({
        "op": "clone",
        "project": project.id.to_string(),
        "operation": project.operation_id.to_string(),
        "repository": project.cloned_full_name,
        "directory": project.directory,
    }))
}

fn parse_setup_status(value: Value) -> Result<bool, ProjectRuntimeError> {
    value.as_bool().ok_or(ProjectRuntimeError::Uncertain)
}

fn parse_key(value: Value) -> Result<SshKeyStatus, ProjectRuntimeError> {
    let state: SshKeyStatus =
        serde_json::from_value(value).map_err(|_| ProjectRuntimeError::Uncertain)?;
    if !state.public_key.starts_with("ssh-ed25519 ")
        || state.public_key.len() > 1024
        || state.public_key.chars().any(char::is_control)
    {
        return Err(ProjectRuntimeError::Uncertain);
    }
    Ok(state)
}

fn parse_response(bytes: &[u8]) -> Result<Value, ProjectRuntimeError> {
    let response: Value =
        serde_json::from_slice(bytes).map_err(|_| ProjectRuntimeError::Uncertain)?;
    if response["ok"] == true {
        return response
            .get("result")
            .filter(|v| !v.is_null())
            .cloned()
            .ok_or(ProjectRuntimeError::Uncertain);
    }
    Err(match response["error"].as_str() {
        Some("invalid") => ProjectRuntimeError::Invalid,
        Some("conflict") => ProjectRuntimeError::Conflict,
        Some("scope_violation") => ProjectRuntimeError::ScopeViolation,
        Some("diff_unrenderable") => ProjectRuntimeError::DiffUnrenderable,
        Some("unavailable") => ProjectRuntimeError::Unavailable,
        _ => ProjectRuntimeError::Uncertain,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use factory_core::{
        environments::{DesiredState, ObservedState, ProvisioningState},
        projects::ProjectState,
    };
    use uuid::Uuid;

    #[test]
    fn clone_uses_validated_repository_name_frozen_for_the_operation() {
        let environment_id = Uuid::new_v4();
        let user_id = Uuid::new_v4();
        let project_id = Uuid::new_v4();
        let env = Environment {
            id: environment_id,
            user_id,
            installation_id: Uuid::new_v4(),
            desired_state: DesiredState::Running,
            observed_state: ObservedState::Running,
            generation: 1,
            provisioning_state: ProvisioningState::VolumeReady,
            volume_name: "fixture-volume".into(),
            container_name: "fixture-container".into(),
            image_id: "fixture-image".into(),
            policy_version: "fixture".into(),
            observed_at: None,
            error_code: None,
            next_reconcile_at: Utc::now(),
        };
        let mut project = Project {
            id: project_id,
            user_id,
            repository_id: 42,
            full_name: "new-owner/renamed".into(),
            cloned_full_name: "original-owner/original-repo".into(),
            directory: "original-repo".into(),
            state: ProjectState::Importing,
            operation_id: Uuid::new_v4(),
            environment_id,
            environment_generation: env.generation,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let request = clone_request(&env, &project).unwrap();
        assert_eq!(request["op"], "clone");
        assert_eq!(request["repository"], "original-owner/original-repo");
        assert_eq!(request["operation"], project.operation_id.to_string());

        project.directory = "../outside".into();
        assert_eq!(
            clone_request(&env, &project),
            Err(ProjectRuntimeError::Invalid)
        );
        project.directory = "original-repo".into();
        project.cloned_full_name = "owner/../../invalid".into();
        assert_eq!(
            clone_request(&env, &project),
            Err(ProjectRuntimeError::Invalid)
        );
    }

    #[test]
    fn malformed_and_uncertain_process_responses_never_report_success() {
        assert_eq!(
            parse_response(b"not json"),
            Err(ProjectRuntimeError::Uncertain)
        );
        assert_eq!(
            parse_response(br#"{"ok":false,"error":"uncertain"}"#),
            Err(ProjectRuntimeError::Uncertain)
        );
        assert_eq!(
            parse_response(br#"{"ok":true,"result":"ready"}"#),
            Ok(json!("ready"))
        );
    }

    #[test]
    fn setup_readiness_accepts_only_explicit_booleans() {
        assert_eq!(parse_setup_status(json!(true)), Ok(true));
        assert_eq!(parse_setup_status(json!(false)), Ok(false));
        assert_eq!(
            parse_setup_status(json!("true")),
            Err(ProjectRuntimeError::Uncertain)
        );
        assert_eq!(
            parse_setup_status(Value::Null),
            Err(ProjectRuntimeError::Uncertain)
        );
    }
}
