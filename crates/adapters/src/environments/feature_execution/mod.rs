//! Disposable worker containers for Features. The normal chat runtime stays in the
//! member container. Workers have no network namespace peer and mount only a checkout
//! and, for a provider turn, the read-only endpoint of an exact-domain CONNECT relay.
mod policy;
mod provider;

use super::{
    DockerEnvironmentRuntime,
    policy::{require_current_policy, validate_record},
    process::{DockerProcess, FixedProcess, ProcessStartError},
};
use async_trait::async_trait;
use bollard::{
    container::LogOutput,
    errors::Error as DockerError,
    models::ContainerCreateBody,
    query_parameters::{
        AttachContainerOptions, CreateContainerOptions, InspectContainerOptions,
        ListContainersOptions, RemoveContainerOptions, StartContainerOptions, StopContainerOptions,
    },
};
use factory_core::{
    conversations::execution::ExecutionError,
    environments::{DesiredState, Environment, EnvironmentError, VolumePresence},
    features::{
        FeatureCheckOutput, FeatureExecution, FeatureExecutionRuntime, FeatureRun, TaskAttempt,
        TaskObservation, TaskWorkspace,
    },
    projects::{Project, ProjectState},
};
use futures_util::StreamExt;
use policy::{
    ROLE, Role, WorkerIdentity, inspect_id, verify_identity, verify_worker, verify_worker_owner,
};
pub(crate) use provider::{ScopedProvider, relay_command};
use serde_json::Value;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock, Weak},
    time::Duration,
};
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};
use uuid::Uuid;

const MAX_CHECK_OUTPUT: usize = 32 * 1024;
const CHECK_TIMEOUT: Duration = Duration::from_secs(600);
pub(crate) const CHECKOUT: &str = "/workspace/checkout";

type LaunchLocks = HashMap<Uuid, Weak<AsyncMutex<()>>>;
static LAUNCH_LOCKS: OnceLock<Mutex<LaunchLocks>> = OnceLock::new();

pub(super) async fn launch_lock(
    env: &Environment,
) -> Result<OwnedMutexGuard<()>, EnvironmentError> {
    let lock = {
        let mut locks = LAUNCH_LOCKS
            .get_or_init(Mutex::default)
            .lock()
            .map_err(|_| EnvironmentError::Unavailable)?;
        locks.retain(|_, lock| lock.strong_count() > 0);
        if let Some(lock) = locks.get(&env.id).and_then(Weak::upgrade) {
            lock
        } else {
            let lock = Arc::new(AsyncMutex::new(()));
            locks.insert(env.id, Arc::downgrade(&lock));
            lock
        }
    };
    Ok(lock.lock_owned().await)
}

#[derive(Clone)]
pub struct DockerFeatureExecutionRuntime {
    runtime: Arc<DockerEnvironmentRuntime>,
}

impl DockerFeatureExecutionRuntime {
    pub fn new(runtime: Arc<DockerEnvironmentRuntime>) -> Self {
        Self { runtime }
    }

    async fn start(
        &self,
        env: &Environment,
        identity: &WorkerIdentity,
        body: ContainerCreateBody,
        relay: Option<(ScopedProvider, Uuid)>,
    ) -> Result<(DockerProcess, WorkerExit), ExecutionError> {
        tokio::time::timeout(self.runtime.timeout, async {
            let _gate = launch_lock(env).await.map_err(preflight)?;
            self.runtime.feature_preflight(env).await?;
            let relay = match relay {
                Some((provider, attempt)) => {
                    Some(self.runtime.feature_relay(env, provider, attempt).await?)
                }
                None => None,
            };
            self.runtime
                .create_feature_worker(env, identity, body, relay)
                .await
        })
        .await
        .map_err(|_| ExecutionError::LaunchUncertain)?
    }
}

#[async_trait]
impl FeatureExecutionRuntime for DockerFeatureExecutionRuntime {
    async fn open_task(
        &self,
        env: &Environment,
        project: &Project,
        request: &FeatureRun,
    ) -> Result<FeatureExecution, ExecutionError> {
        if project.state != ProjectState::Ready {
            return Err(ExecutionError::PreflightUnavailable);
        }
        let provider = ScopedProvider::from_request(request)?;
        let identity = WorkerIdentity::new(
            env,
            project,
            &request.workspace,
            env.generation,
            Role::Agent,
        )?;
        let cwd = match provider {
            ScopedProvider::Codex => "/workspace",
            ScopedProvider::Claude => CHECKOUT,
        };
        let body = identity.body(env, cwd, provider.command(request))?;
        let (process, exit) = self
            .start(
                env,
                &identity,
                body,
                Some((provider, request.workspace.attempt_id)),
            )
            .await?;
        let process_receipt = process.id.clone();
        let session = match provider {
            ScopedProvider::Codex => crate::codex_execution::scoped_session(process, exit, request),
            ScopedProvider::Claude => {
                crate::claude_execution::scoped_session(process, exit, request)
            }
        };
        Ok(FeatureExecution {
            session,
            process_receipt,
        })
    }

    async fn run_check(
        &self,
        env: &Environment,
        project: &Project,
        workspace: &TaskWorkspace,
        command: &str,
    ) -> Result<FeatureCheckOutput, ExecutionError> {
        if project.state != ProjectState::Ready
            || command.trim().is_empty()
            || command.len() > 8192
            || command.contains('\0')
        {
            return Err(ExecutionError::Invalid);
        }
        let identity = WorkerIdentity::new(env, project, workspace, env.generation, Role::Check)?;
        let body = check_body(&identity, env, command)?;
        let (process, exit) = self.start(env, &identity, body, None).await?;
        collect_check(process, exit).await
    }

    async fn reconcile_task(
        &self,
        env: &Environment,
        project: &Project,
        attempt: &TaskAttempt,
    ) -> Result<TaskObservation, ExecutionError> {
        tokio::time::timeout(self.runtime.timeout, async {
            validate_record(env, self.runtime.metadata.installation_id()).map_err(preflight)?;
            let _gate = launch_lock(env).await.map_err(preflight)?;
            let mut worker_was_observed = false;
            for role in [Role::Agent, Role::Check] {
                let identity = WorkerIdentity::new(
                    env,
                    project,
                    &attempt.claim.workspace,
                    attempt.generation,
                    role,
                )?;
                if let Some(value) = self
                    .runtime
                    .inspect_feature_worker(&identity.name)
                    .await
                    .map_err(preflight)?
                {
                    verify_identity(env, &identity, &value)?;
                    let id = inspect_id(&value).map_err(preflight)?;
                    match value["State"]["Status"].as_str() {
                        Some("running") => return Ok(TaskObservation::Running),
                        Some("created" | "exited") => {
                            // Remove even an unstarted worker to fence an uncertain Start request.
                            self.runtime
                                .remove_feature_worker(id)
                                .await
                                .map_err(|_| ExecutionError::CleanupUncertain)?;
                            worker_was_observed = true;
                        }
                        _ => return Ok(TaskObservation::Unknown),
                    }
                }
            }
            Ok(if worker_was_observed || attempt.launch_settled {
                TaskObservation::Stopped
            } else {
                TaskObservation::Unknown
            })
        })
        .await
        .map_err(|_| ExecutionError::CleanupUncertain)?
    }
}

fn check_body(
    identity: &WorkerIdentity,
    env: &Environment,
    command: &str,
) -> Result<ContainerCreateBody, ExecutionError> {
    identity.body(
        env,
        CHECKOUT,
        [
            "/usr/bin/timeout",
            "--signal=TERM",
            "--kill-after=5s",
            "590s",
            "/bin/sh",
            "-c",
            command,
        ]
        .into_iter()
        .map(str::to_owned)
        .collect(),
    )
}

async fn collect_check(
    process: DockerProcess,
    exit: WorkerExit,
) -> Result<FeatureCheckOutput, ExecutionError> {
    let DockerProcess {
        input, mut output, ..
    } = process;
    drop(input);
    let collected = tokio::time::timeout(CHECK_TIMEOUT, async {
        let mut log = Vec::new();
        while let Some(frame) = output.next().await {
            let bytes = match frame.map_err(|_| ExecutionError::Unavailable)? {
                LogOutput::StdOut { message } | LogOutput::StdErr { message } => message,
                _ => continue,
            };
            let remaining = MAX_CHECK_OUTPUT.saturating_sub(log.len());
            log.extend_from_slice(&bytes[..bytes.len().min(remaining)]);
        }
        Ok::<_, ExecutionError>(log)
    })
    .await;
    drop(output);
    let exit_code = exit.finish().await?;
    let log = collected.map_err(|_| ExecutionError::Unavailable)??;
    Ok(FeatureCheckOutput {
        exit_code,
        output: String::from_utf8_lossy(&log).into_owned(),
    })
}

fn preflight(_: EnvironmentError) -> ExecutionError {
    ExecutionError::PreflightUnavailable
}

pub(crate) struct WorkerExit {
    runtime: Arc<DockerEnvironmentRuntime>,
    id: String,
    relay: Option<DockerProcess>,
}

impl WorkerExit {
    /// Before reporting a terminal provider event, prove the whole container exited.
    pub(crate) async fn finish(self) -> Result<i64, ExecutionError> {
        let Self { runtime, id, relay } = self;
        let observed = tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                let value = runtime
                    .inspect_feature_worker(&id)
                    .await
                    .map_err(preflight)?
                    .ok_or(ExecutionError::CleanupUncertain)?;
                if value["Id"] != id {
                    return Err(ExecutionError::CleanupUncertain);
                }
                if value["State"]["Status"] == "exited" {
                    return value["State"]["ExitCode"]
                        .as_i64()
                        .ok_or(ExecutionError::CleanupUncertain);
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await;
        let removed =
            tokio::time::timeout(runtime.timeout, runtime.remove_feature_worker(&id)).await;
        // Closing both relay stream halves tears down the Unix listener and tunnels.
        drop(relay);
        if !matches!(removed, Ok(Ok(()))) {
            return Err(ExecutionError::CleanupUncertain);
        }
        // If the watchdog or shutdown was required, we proved cleanup but have no
        // trustworthy command exit code. Preserve that distinction for checks.
        observed.map_err(|_| ExecutionError::Unavailable)?
    }
}

impl DockerEnvironmentRuntime {
    async fn create_feature_worker(
        self: &Arc<Self>,
        env: &Environment,
        identity: &WorkerIdentity,
        body: ContainerCreateBody,
        relay: Option<DockerProcess>,
    ) -> Result<(DockerProcess, WorkerExit), ExecutionError> {
        self.require_mutation_permission().map_err(preflight)?;
        let expected =
            serde_json::to_value(&body).map_err(|_| ExecutionError::PreflightUnavailable)?;
        let created = self
            .docker
            .create_container(
                Some(CreateContainerOptions {
                    name: Some(identity.name.clone()),
                    ..Default::default()
                }),
                body,
            )
            .await
            .map_err(|_| ExecutionError::PreflightUnavailable)?;
        let id = created.id;
        let attached = self
            .attach_feature_worker(env, identity, &expected, &id, created.warnings)
            .await;
        let process = match attached {
            Ok(process) => process,
            Err(error) => {
                // A successful removal by proved immutable ID also fences any
                // Docker start request whose reply did not reach the caller.
                let Some(owned) = self
                    .inspect_feature_worker(&id)
                    .await
                    .map_err(|_| ExecutionError::CleanupUncertain)?
                else {
                    return Err(error);
                };
                verify_identity(env, identity, &owned)?;
                if inspect_id(&owned).map_err(preflight)? != id {
                    return Err(ExecutionError::CleanupUncertain);
                }
                return match self.remove_feature_worker(&id).await {
                    Ok(()) => Err(ExecutionError::PreflightUnavailable),
                    Err(_) => Err(error),
                };
            }
        };
        Ok((
            process,
            WorkerExit {
                runtime: Arc::clone(self),
                id,
                relay,
            },
        ))
    }

    async fn feature_preflight(&self, env: &Environment) -> Result<(), ExecutionError> {
        validate_record(env, self.metadata.installation_id()).map_err(preflight)?;
        require_current_policy(env).map_err(preflight)?;
        self.require_mutation_permission().map_err(preflight)?;
        if env.desired_state != DesiredState::Running
            || self.volume(env).await.map_err(preflight)? != VolumePresence::Present
        {
            return Err(ExecutionError::PreflightUnavailable);
        }
        let container = self
            .container(env)
            .await
            .map_err(preflight)?
            .ok_or(ExecutionError::PreflightUnavailable)?;
        self.verify_current_container(env, &container)
            .await
            .map_err(preflight)?;
        if container["State"]["Status"] != "running" || container["State"]["Paused"] == true {
            return Err(ExecutionError::PreflightUnavailable);
        }
        Ok(())
    }

    async fn feature_relay(
        &self,
        env: &Environment,
        provider: ScopedProvider,
        attempt: Uuid,
    ) -> Result<DockerProcess, ExecutionError> {
        if !self.egress.available().await.map_err(preflight)? {
            return Err(ExecutionError::PreflightUnavailable);
        }
        let mut relay = self
            .start_process_checked(env, FixedProcess::FeatureRelay { provider, attempt })
            .await
            .map_err(|error| match error {
                ProcessStartError::PreSubmission(_) => ExecutionError::PreflightUnavailable,
                ProcessStartError::Submitted(_) => ExecutionError::LaunchUncertain,
            })?;
        match relay.output.next().await {
            Some(Ok(LogOutput::StdOut { message })) if message.as_ref() == b"ready\n" => Ok(relay),
            _ => Err(ExecutionError::PreflightUnavailable),
        }
    }

    async fn inspect_feature_worker(
        &self,
        identity: &str,
    ) -> Result<Option<Value>, EnvironmentError> {
        match self
            .docker
            .inspect_container(identity, None::<InspectContainerOptions>)
            .await
        {
            Ok(worker) => serde_json::to_value(worker)
                .map(Some)
                .map_err(|_| EnvironmentError::Unavailable),
            Err(DockerError::DockerResponseServerError {
                status_code: 404, ..
            }) => Ok(None),
            Err(_) => Err(EnvironmentError::Unavailable),
        }
    }

    async fn attach_feature_worker(
        &self,
        env: &Environment,
        identity: &WorkerIdentity,
        expected: &Value,
        id: &str,
        warnings: Vec<String>,
    ) -> Result<DockerProcess, ExecutionError> {
        let value = self
            .inspect_feature_worker(id)
            .await
            .map_err(preflight)?
            .ok_or(ExecutionError::PreflightUnavailable)?;
        verify_identity(env, identity, &value)?;
        if inspect_id(&value).map_err(preflight)? != id || !warnings.is_empty() {
            return Err(ExecutionError::PreflightUnavailable);
        }
        verify_worker(&value, expected)?;
        self.require_mutation_permission().map_err(preflight)?;
        let stream = self
            .docker
            .attach_container(
                id,
                Some(AttachContainerOptions {
                    stdin: true,
                    stdout: true,
                    stderr: true,
                    stream: true,
                    ..Default::default()
                }),
            )
            .await
            .map_err(|_| ExecutionError::PreflightUnavailable)?;
        self.require_mutation_permission().map_err(preflight)?;
        self.docker
            .start_container(id, None::<StartContainerOptions>)
            .await
            .map_err(|_| ExecutionError::LaunchUncertain)?;
        Ok(DockerProcess {
            id: id.to_owned(),
            input: stream.input,
            output: stream.output,
        })
    }

    async fn remove_feature_worker(&self, id: &str) -> Result<(), EnvironmentError> {
        self.require_mutation_permission()?;
        match self
            .docker
            .remove_container(
                id,
                Some(RemoveContainerOptions {
                    force: true,
                    ..Default::default()
                }),
            )
            .await
        {
            Ok(())
            | Err(DockerError::DockerResponseServerError {
                status_code: 404, ..
            }) => Ok(()),
            Err(_) => Err(EnvironmentError::Unavailable),
        }
    }

    /// Stop is serialized against create/attach/start. The daemon is selected by
    /// the trusted installation. Inspect every match and mutate only immutable IDs.
    pub(super) async fn stop_feature_workers(
        &self,
        env: &Environment,
    ) -> Result<(), EnvironmentError> {
        let matching = self
            .docker
            .list_containers(Some(ListContainersOptions {
                all: true,
                filters: Some(HashMap::from([(
                    "label".to_owned(),
                    vec![
                        ROLE.to_owned(),
                        format!("io.factory.installation={}", env.installation_id),
                        format!("io.factory.environment={}", env.id),
                        format!("io.factory.user={}", env.user_id),
                    ],
                )])),
                ..Default::default()
            }))
            .await
            .map_err(|_| EnvironmentError::Unavailable)?;
        for entry in matching {
            let id = entry.id.ok_or(EnvironmentError::OwnershipConflict)?;
            let Some(value) = self.inspect_feature_worker(&id).await? else {
                continue;
            };
            verify_worker_owner(env, &value, &id)?;
            self.require_mutation_permission()?;
            match self
                .docker
                .stop_container(
                    &id,
                    Some(StopContainerOptions {
                        t: Some(2),
                        ..Default::default()
                    }),
                )
                .await
            {
                Ok(())
                | Err(DockerError::DockerResponseServerError {
                    status_code: 304 | 404,
                    ..
                }) => (),
                Err(_) => return Err(EnvironmentError::Unavailable),
            }
            self.remove_feature_worker(&id).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
