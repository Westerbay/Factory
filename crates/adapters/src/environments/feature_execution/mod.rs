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
use crate::attachments::DockerAttachmentFiles;
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
    attachments::AttachmentFiles,
    conversations::execution::{ExecutionControl, ExecutionError, QuestionAnswer},
    environments::{DesiredState, Environment, EnvironmentError, VolumePresence},
    features::{
        CheckObservation, FeatureCheckOutput, FeatureExecution, FeatureExecutionRuntime,
        FeatureRun, TaskAttempt, TaskObservation, TaskWorkspace,
    },
    projects::{Project, ProjectState},
};
use futures_util::StreamExt;
use policy::{
    ROLE, Role, WorkerIdentity, inspect_id, verify_identity, verify_worker, verify_worker_owner,
};
pub(crate) use provider::{
    AGENT_INSTRUCTIONS, RelayAccess, ScopedProvider, proof_command, relay_command,
};
use provider::{guarded_command, prepare_command, prepared_command};
use serde_json::Value;
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, OnceLock, Weak,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};
use uuid::Uuid;

const MAX_CHECK_OUTPUT: usize = 32 * 1024;
const CHECK_TIMEOUT: Duration = Duration::from_secs(600);
pub(crate) const CHECKOUT: &str = "/workspace/checkout";
const TOOLING: &str = "/workspace/tooling";
const PREPARATION_STEPS: &[&str] = &[
    "configuration",
    "node",
    "npm",
    "pnpm",
    "yarn",
    "rust",
    "python",
    "java",
    "go",
    "ruby",
    "uv",
    "maven",
    "gradle",
    "jq",
    "ripgrep",
    "python-dependencies",
    "cargo-dependencies",
    "node-dependencies",
];

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

    async fn prepare_repository(
        &self,
        env: &Environment,
        project: &Project,
        workspace: &TaskWorkspace,
    ) -> Result<(), ExecutionError> {
        let identity = WorkerIdentity::new(env, project, workspace, env.generation, Role::Prepare)?;
        let body = identity.body(env, CHECKOUT, prepare_command())?;
        let (process, exit) = self
            .start(
                env,
                &identity,
                body,
                Some((RelayAccess::Repository, workspace.attempt_id)),
            )
            .await
            .map_err(|error| {
                tracing::warn!(event = "repository.environment.prepare_failed",
                    attempt_id = %workspace.attempt_id, execution_error = %error, step = "worker");
                ExecutionError::RepositoryPreparationFailed("worker")
            })?;
        // No application receipt is needed for this disposable, idempotent setup.
        exit.launch_ack.store(true, Ordering::Release);
        let result = collect_check(process, exit).await.map_err(|error| {
            tracing::warn!(event = "repository.environment.prepare_failed",
                attempt_id = %workspace.attempt_id, execution_error = %error, step = "worker");
            ExecutionError::RepositoryPreparationFailed("worker")
        })?;
        if result.exit_code != 0 {
            let reported_step = result
                .output
                .trim()
                .strip_prefix("Repository environment: ");
            let step = PREPARATION_STEPS
                .iter()
                .copied()
                .find(|step| Some(*step) == reported_step)
                .unwrap_or("setup");
            tracing::warn!(event = "repository.environment.prepare_failed",
                attempt_id = %workspace.attempt_id, step, exit_code = result.exit_code);
            return Err(ExecutionError::RepositoryPreparationFailed(step));
        }
        Ok(())
    }

    /// Call with the environment launch lock held. Missing containers do not
    /// establish whether an earlier Docker request reached the daemon.
    async fn inspect_check(
        &self,
        env: &Environment,
        project: &Project,
        workspace: &TaskWorkspace,
        generation: i64,
    ) -> Result<Option<Value>, ExecutionError> {
        let identity = WorkerIdentity::new(env, project, workspace, generation, Role::Check)?;
        let value = self
            .runtime
            .inspect_feature_worker(&identity.name)
            .await
            .map_err(|_| ExecutionError::CleanupUncertain)?;
        if let Some(value) = &value {
            verify_identity(env, &identity, value)?;
            inspect_id(value).map_err(|_| ExecutionError::CleanupUncertain)?;
        }
        Ok(value)
    }

    async fn start(
        &self,
        env: &Environment,
        identity: &WorkerIdentity,
        body: ContainerCreateBody,
        relay: Option<(RelayAccess, Uuid)>,
    ) -> Result<(DockerProcess, WorkerExit), ExecutionError> {
        tokio::time::timeout(self.runtime.timeout, async {
            let _gate = launch_lock(env).await.map_err(preflight)?;
            self.runtime.feature_preflight(env).await?;
            let proof = self.runtime.checkout_proof(env, identity).await?;
            let relay = match relay {
                Some((access, attempt)) => {
                    Some(self.runtime.feature_relay(env, access, attempt).await?)
                }
                None => None,
            };
            self.runtime
                .create_feature_worker(env, identity, body, relay, &proof)
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
        let mut identity = WorkerIdentity::new(
            env,
            project,
            &request.workspace,
            env.generation,
            Role::Agent,
        )?;
        if let Some(conversation) = request.conversation_id {
            // Mount sources must exist, and the call is idempotent.
            DockerAttachmentFiles::new(Arc::clone(&self.runtime))
                .prepare_worker_directories(
                    env,
                    project.id,
                    conversation,
                    request.workspace.attempt_id,
                )
                .await
                .map_err(|_| ExecutionError::PreflightUnavailable)?;
            identity = identity.with_conversation(project, conversation);
        }
        let cwd = match provider {
            ScopedProvider::Codex => "/workspace",
            ScopedProvider::Claude => CHECKOUT,
        };
        self.prepare_repository(env, project, &request.workspace)
            .await?;
        let body = identity.body(
            env,
            cwd,
            prepared_command(identity.readonly, provider.command(request)),
        )?;
        let (process, exit) = self
            .start(
                env,
                &identity,
                body,
                Some((
                    RelayAccess::Provider(provider),
                    request.workspace.attempt_id,
                )),
            )
            .await
            .inspect_err(|error| {
                tracing::warn!(
                    event = "agent.execution.launch_failed",
                    provider = provider.as_str(),
                    attempt_id = %request.workspace.attempt_id,
                    execution_error = %error,
                );
            })?;
        let process_receipt = process.id.clone();
        let launch_ack = Arc::clone(&exit.launch_ack);
        let mut session = match provider {
            ScopedProvider::Codex => crate::codex_execution::scoped_session(process, exit, request),
            ScopedProvider::Claude => {
                crate::claude_execution::scoped_session(process, exit, request)
            }
        };
        session.control = Box::new(ScopedControl {
            delegate: session.control,
            launch_ack,
        });
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
        self.prepare_repository(env, project, workspace).await?;
        let body = check_body(&identity, env, command)?;
        let (process, exit) = self.start(env, &identity, body, None).await?;
        collect_check(process, exit).await
    }

    async fn acknowledge_check(
        &self,
        env: &Environment,
        project: &Project,
        workspace: &TaskWorkspace,
        generation: i64,
    ) -> Result<(), ExecutionError> {
        tokio::time::timeout(self.runtime.timeout, async {
            validate_record(env, self.runtime.metadata.installation_id()).map_err(preflight)?;
            let _gate = launch_lock(env).await.map_err(preflight)?;
            let Some(value) = self
                .inspect_check(env, project, workspace, generation)
                .await?
            else {
                // Only the caller's already durable acknowledgement makes
                // cleanup idempotent. Recovery never treats this as proof.
                return Ok(());
            };
            if value["State"]["Status"] != "exited" || value["State"]["Running"] != false {
                return Err(ExecutionError::CleanupUncertain);
            }
            let id = inspect_id(&value).map_err(|_| ExecutionError::CleanupUncertain)?;
            self.runtime
                .remove_feature_worker(id)
                .await
                .map_err(|_| ExecutionError::CleanupUncertain)
        })
        .await
        .map_err(|_| ExecutionError::CleanupUncertain)?
    }

    async fn reconcile_check(
        &self,
        env: &Environment,
        project: &Project,
        workspace: &TaskWorkspace,
        generation: i64,
    ) -> Result<CheckObservation, ExecutionError> {
        tokio::time::timeout(self.runtime.timeout, async {
            validate_record(env, self.runtime.metadata.installation_id()).map_err(preflight)?;
            let _gate = launch_lock(env).await.map_err(preflight)?;
            let Some(value) = self
                .inspect_check(env, project, workspace, generation)
                .await?
            else {
                return Ok(CheckObservation::Unknown);
            };
            let state = &value["State"];
            match (state["Status"].as_str(), state["Running"].as_bool()) {
                (Some("running"), Some(true)) => Ok(CheckObservation::Running),
                (Some("exited"), Some(false)) => state["ExitCode"]
                    .as_i64()
                    .filter(|code| (0..=255).contains(code))
                    .map(|exit_code| CheckObservation::Exited { exit_code })
                    .ok_or(ExecutionError::CleanupUncertain),
                _ => Ok(CheckObservation::Unknown),
            }
        })
        .await
        .map_err(|_| ExecutionError::CleanupUncertain)?
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
            let checking = attempt.pending_verification == Some(attempt.id());
            let launch_settled = attempt.launch_settled && !checking;
            let mut running = false;
            let mut unknown = false;
            let mut stopped = Vec::new();
            for role in [Role::Agent, Role::Check, Role::Prepare] {
                let identity = WorkerIdentity::new(
                    env,
                    project,
                    &attempt.claim.workspace,
                    attempt.generation,
                    role,
                )?;
                let Some(value) = self
                    .runtime
                    .inspect_feature_worker(&identity.name)
                    .await
                    .map_err(preflight)?
                else {
                    continue;
                };
                verify_identity(env, &identity, &value)?;
                let id = inspect_id(&value).map_err(preflight)?;
                match (
                    value["State"]["Status"].as_str(),
                    value["State"]["Running"].as_bool(),
                ) {
                    (Some("running"), Some(true)) => running = true,
                    (Some("exited"), Some(false)) => stopped.push((role, id.to_owned())),
                    (Some("created"), Some(false)) if launch_settled => {
                        stopped.push((role, id.to_owned()));
                    }
                    _ => unknown = true,
                }
            }
            if running {
                return Ok(TaskObservation::Running);
            }
            if unknown {
                return Ok(TaskObservation::Unknown);
            }
            if launch_settled {
                for (_, id) in stopped {
                    // Removing a verified created container fences any pending Start.
                    self.runtime
                        .remove_feature_worker(&id)
                        .await
                        .map_err(|_| ExecutionError::CleanupUncertain)?;
                }
                return Ok(TaskObservation::Stopped);
            }
            // An old agent cannot prove a subsequently uncertain check stopped.
            let proof = stopped
                .into_iter()
                .find(|(role, _)| *role == if checking { Role::Check } else { Role::Agent });
            // Keep the exited object until its exact identity is durably recorded.
            Ok(proof.map_or(TaskObservation::Unknown, |(_, worker_id)| {
                TaskObservation::StoppedWithWorkerProof { worker_id }
            }))
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
        prepared_command(
            identity.readonly,
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
        ),
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
            if bytes.len() >= MAX_CHECK_OUTPUT {
                log.clear();
                log.extend_from_slice(&bytes[bytes.len() - MAX_CHECK_OUTPUT..]);
            } else {
                let discarded = (log.len() + bytes.len()).saturating_sub(MAX_CHECK_OUTPUT);
                log.drain(..discarded);
                log.extend_from_slice(&bytes);
            }
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

fn relay_proof(value: &[u8]) -> Result<&str, ExecutionError> {
    if value.len() > 54 {
        return Err(ExecutionError::PreflightUnavailable);
    }
    let proof = value
        .strip_prefix(b"ready ")
        .ok_or(ExecutionError::PreflightUnavailable)?;
    parse_proof(std::str::from_utf8(proof).map_err(|_| ExecutionError::PreflightUnavailable)?)
}

fn parse_proof(value: &str) -> Result<&str, ExecutionError> {
    let proof = value
        .strip_suffix('\n')
        .ok_or(ExecutionError::PreflightUnavailable)?;
    let (device, inode) = proof
        .split_once(':')
        .ok_or(ExecutionError::PreflightUnavailable)?;
    if device.parse::<u64>().is_err() || inode.parse::<u64>().is_err() {
        return Err(ExecutionError::PreflightUnavailable);
    }
    Ok(proof)
}

fn preflight(_: EnvironmentError) -> ExecutionError {
    ExecutionError::PreflightUnavailable
}

struct ScopedControl {
    delegate: Box<dyn ExecutionControl>,
    launch_ack: Arc<AtomicBool>,
}

#[async_trait]
impl ExecutionControl for ScopedControl {
    async fn begin_turn(&mut self) -> Result<(), ExecutionError> {
        // The coordinator persists the process receipt before calling begin_turn.
        self.launch_ack.store(true, Ordering::Release);
        self.delegate.begin_turn().await
    }

    async fn answer(
        &mut self,
        request_id: Uuid,
        answers: Vec<QuestionAnswer>,
    ) -> Result<(), ExecutionError> {
        self.delegate.answer(request_id, answers).await
    }

    async fn interrupt(&mut self) -> Result<(), ExecutionError> {
        self.delegate.interrupt().await
    }
}

pub(crate) struct WorkerExit {
    runtime: Arc<DockerEnvironmentRuntime>,
    id: String,
    relay: Option<DockerProcess>,
    launch_ack: Arc<AtomicBool>,
}

impl WorkerExit {
    /// Before reporting a terminal provider event, prove the whole container exited.
    pub(crate) async fn finish(self) -> Result<i64, ExecutionError> {
        let Self {
            runtime,
            id,
            relay,
            launch_ack,
        } = self;
        // Closing both relay stream halves tears down the Unix listener and tunnels.
        drop(relay);
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
        let launch_recorded = launch_ack.load(Ordering::Acquire);
        let cleanup = tokio::time::timeout(runtime.timeout, async {
            if launch_recorded {
                runtime.remove_feature_worker(&id).await
            } else if matches!(observed, Ok(Ok(_))) {
                // No durable acknowledgement arrived. Keep the exited worker
                // as evidence for both provider launches and checks.
                Ok(())
            } else {
                runtime.stop_feature_worker(&id).await
            }
        })
        .await;
        if !matches!(cleanup, Ok(Ok(()))) {
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
        mut body: ContainerCreateBody,
        relay: Option<(DockerProcess, String)>,
        proof: &str,
    ) -> Result<(DockerProcess, WorkerExit), ExecutionError> {
        self.require_mutation_permission().map_err(preflight)?;
        let command = body.entrypoint.take().ok_or(ExecutionError::Invalid)?;
        body.entrypoint = Some(guarded_command(
            proof,
            relay.as_ref().map(|(_, proof)| proof.as_str()),
            command,
        ));
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
            Err(ExecutionError::LaunchUncertain) => {
                // A possibly accepted start may leave the only durable proof.
                return Err(ExecutionError::LaunchUncertain);
            }
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
                launch_ack: Arc::new(AtomicBool::new(false)),
                relay: relay.map(|(process, _)| process),
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

    async fn checkout_proof(
        &self,
        env: &Environment,
        identity: &WorkerIdentity,
    ) -> Result<String, ExecutionError> {
        let mut process = self
            .start_process_checked(
                env,
                FixedProcess::FeatureProof {
                    checkout: identity.checkout.clone(),
                },
            )
            .await
            .map_err(|_| ExecutionError::PreflightUnavailable)?;
        let proof = match process.output.next().await {
            Some(Ok(LogOutput::StdOut { message })) if message.len() <= 48 => {
                String::from_utf8(message.to_vec())
                    .map_err(|_| ExecutionError::PreflightUnavailable)?
            }
            _ => return Err(ExecutionError::PreflightUnavailable),
        };
        let id = process.id.clone();
        drop(process);
        if self
            .wait_process_exit(&id, Duration::from_secs(3))
            .await
            .map_err(preflight)?
            != 0
        {
            return Err(ExecutionError::PreflightUnavailable);
        }
        parse_proof(&proof).map(str::to_owned)
    }

    async fn feature_relay(
        &self,
        env: &Environment,
        access: RelayAccess,
        attempt: Uuid,
    ) -> Result<(DockerProcess, String), ExecutionError> {
        if matches!(access, RelayAccess::Provider(_))
            && !self.egress.available().await.map_err(preflight)?
        {
            return Err(ExecutionError::PreflightUnavailable);
        }
        let mut relay = self
            .start_process_checked(env, FixedProcess::FeatureRelay { access, attempt })
            .await
            .map_err(|error| match error {
                ProcessStartError::PreSubmission(_) => ExecutionError::PreflightUnavailable,
                ProcessStartError::Submitted(_) => ExecutionError::LaunchUncertain,
            })?;
        let proof = match relay.output.next().await {
            Some(Ok(LogOutput::StdOut { message })) => relay_proof(&message)?.to_owned(),
            _ => return Err(ExecutionError::PreflightUnavailable),
        };
        Ok((relay, proof))
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

    async fn stop_feature_worker(&self, id: &str) -> Result<(), EnvironmentError> {
        self.require_mutation_permission()?;
        match self
            .docker
            .stop_container(
                id,
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
            }) => Ok(()),
            Err(_) => Err(EnvironmentError::Unavailable),
        }
    }

    async fn list_feature_worker_ids(
        &self,
        env: &Environment,
    ) -> Result<Vec<String>, EnvironmentError> {
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
        matching
            .into_iter()
            .map(|entry| entry.id.ok_or(EnvironmentError::OwnershipConflict))
            .collect()
    }

    pub(super) async fn feature_workers_need_stop(
        &self,
        env: &Environment,
    ) -> Result<bool, EnvironmentError> {
        let mut needs_stop = false;
        for id in self.list_feature_worker_ids(env).await? {
            let Some(value) = self.inspect_feature_worker(&id).await? else {
                continue;
            };
            verify_worker_owner(env, &value, &id)?;
            needs_stop |=
                value["State"]["Status"] != "exited" || value["State"]["Running"] != false;
        }
        Ok(needs_stop)
    }

    /// Stop is serialized against create/attach/start. Inspect every match and
    /// mutate only immutable IDs. Exited workers may be pending launch proof.
    pub(super) async fn stop_feature_workers(
        &self,
        env: &Environment,
    ) -> Result<(), EnvironmentError> {
        for id in self.list_feature_worker_ids(env).await? {
            let Some(value) = self.inspect_feature_worker(&id).await? else {
                continue;
            };
            verify_worker_owner(env, &value, &id)?;
            self.stop_feature_worker(&id).await?;
            let Some(stopped) = self.inspect_feature_worker(&id).await? else {
                continue;
            };
            verify_worker_owner(env, &stopped, &id)?;
            match (
                stopped["State"]["Status"].as_str(),
                stopped["State"]["Running"].as_bool(),
            ) {
                (Some("exited"), Some(false)) if stopped["Config"]["Labels"][ROLE] == "prepare" => {
                    self.remove_feature_worker(&id).await?;
                }
                (Some("exited"), Some(false)) => (),
                (Some("created"), Some(false)) => {
                    // A created worker provides no proof that Start settled. Fence
                    // a late daemon request by removing the exact unstarted object.
                    self.remove_feature_worker(&id).await?;
                }
                _ => return Err(EnvironmentError::Unavailable),
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
