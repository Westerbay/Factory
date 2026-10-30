//! Internal, fixed-command Docker streams shared by terminal and account adapters.
use super::{
    DockerEnvironmentRuntime,
    policy::{require_current_policy, validate_record},
};
use bollard::{
    container::LogOutput,
    errors::Error,
    exec::{CreateExecOptions, StartExecOptions, StartExecResults},
    query_parameters::ResizeExecOptions,
};
use factory_core::environments::{DesiredState, Environment, EnvironmentError, VolumePresence};
use futures_util::Stream;
use std::{pin::Pin, time::Duration};
use tokio::io::AsyncWrite;

pub(crate) type ProcessOutput = Pin<Box<dyn Stream<Item = Result<LogOutput, Error>> + Send>>;
pub(crate) type ProcessInput = Pin<Box<dyn AsyncWrite + Send>>;

/// Account cleanup must shut down stdin and drop BOTH stream halves before waiting
/// on the saved ID. A half-closed Docker connection can otherwise retain exec stdin.
pub(crate) struct DockerProcess {
    pub id: String,
    pub input: ProcessInput,
    pub output: ProcessOutput,
}

/// No string command or Docker identifier supplied by a browser can reach this enum.
#[derive(Clone, Copy)]
pub(crate) enum FixedProcess {
    TerminalAttach,
    TerminalDetach,
    CodexAccount,
}

impl FixedProcess {
    fn options(self) -> CreateExecOptions<&'static str> {
        let terminal = matches!(self, Self::TerminalAttach);
        let cmd = match self {
            Self::TerminalAttach => vec![
                "/usr/bin/tmux",
                "-f",
                "/etc/factory/tmux.conf",
                "-S",
                "/tmp/factory-tmux.sock",
                "new-session",
                "-A",
                "-D",
                "-s",
                "factory",
                "-c",
                "/home/factory/Documents",
            ],
            Self::TerminalDetach => vec![
                "/usr/bin/tmux",
                "-S",
                "/tmp/factory-tmux.sock",
                "detach-client",
                "-s",
                "factory",
            ],
            Self::CodexAccount => vec!["/usr/local/bin/factory-auth-controller"],
        };
        CreateExecOptions {
            attach_stdin: Some(true),
            attach_stdout: Some(true),
            attach_stderr: Some(true),
            tty: Some(terminal),
            privileged: Some(false),
            user: Some("10000:10000"),
            working_dir: Some("/home/factory/Documents"),
            env: Some(vec![
                "HOME=/home/factory",
                "CODEX_HOME=/home/factory/.codex",
                "TERM=xterm-256color",
            ]),
            cmd: Some(cmd),
            ..Default::default()
        }
    }
}

impl DockerEnvironmentRuntime {
    /// Callers serialize terminal generations before attach/detach, and authorize the owner.
    /// This boundary independently proves the recorded container before every exec.
    pub(crate) async fn start_process(
        &self,
        env: &Environment,
        command: FixedProcess,
    ) -> Result<DockerProcess, EnvironmentError> {
        tokio::time::timeout(self.timeout, async {
            validate_record(env, self.metadata.installation_id())?;
            require_current_policy(env)?;
            if env.desired_state != DesiredState::Running {
                return Err(EnvironmentError::Conflict);
            }
            if !(self.mutation_permitted)() {
                return Err(EnvironmentError::Unavailable);
            }
            if self.volume(env).await? != VolumePresence::Present {
                return Err(EnvironmentError::VolumeMissing);
            }
            let container = self
                .container(env)
                .await?
                .ok_or(EnvironmentError::Unavailable)?;
            self.verify_current_container(env, &container).await?;
            if container["State"]["Running"] != true
                || container["State"]["Paused"] == true
                || container["State"]["Restarting"] == true
            {
                return Err(EnvironmentError::Unavailable);
            }
            let container_id = container["Id"]
                .as_str()
                .ok_or(EnvironmentError::OwnershipConflict)?;
            let options = command.options();
            let tty = options.tty == Some(true);
            let exec = self
                .docker
                .create_exec(container_id, options)
                .await
                .map_err(map_error)?;
            if !(self.mutation_permitted)() {
                return Err(EnvironmentError::Unavailable);
            }
            match self
                .docker
                .start_exec(
                    &exec.id,
                    Some(StartExecOptions {
                        detach: false,
                        tty,
                        output_capacity: Some(64 * 1024),
                    }),
                )
                .await
                .map_err(map_error)?
            {
                StartExecResults::Attached { input, output } => Ok(DockerProcess {
                    id: exec.id,
                    input,
                    output,
                }),
                StartExecResults::Detached => Err(EnvironmentError::Unavailable),
            }
        })
        .await
        .map_err(|_| EnvironmentError::Timeout)?
    }

    /// Exec IDs originate only from start_process and are retained by the owning adapter.
    pub(crate) async fn resize_process(
        &self,
        exec_id: &str,
        cols: u16,
        rows: u16,
    ) -> Result<(), EnvironmentError> {
        if !(2..=500).contains(&cols) || !(1..=200).contains(&rows) {
            return Err(EnvironmentError::Conflict);
        }
        if !(self.mutation_permitted)() {
            return Err(EnvironmentError::Unavailable);
        }
        tokio::time::timeout(
            self.timeout,
            self.docker.resize_exec(
                exec_id,
                ResizeExecOptions {
                    h: i32::from(rows),
                    w: i32::from(cols),
                },
            ),
        )
        .await
        .map_err(|_| EnvironmentError::Timeout)?
        .map_err(map_error)
    }

    /// A closed transport is not proof that Docker's exec client has terminated.
    pub(crate) async fn wait_process_exit(
        &self,
        exec_id: &str,
        timeout: Duration,
    ) -> Result<i64, EnvironmentError> {
        tokio::time::timeout(timeout, async {
            loop {
                let status = self.docker.inspect_exec(exec_id).await.map_err(map_error)?;
                if status.running == Some(false) {
                    return status.exit_code.ok_or(EnvironmentError::Unavailable);
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .map_err(|_| EnvironmentError::Timeout)?
    }
}

fn map_error(error: Error) -> EnvironmentError {
    match error {
        Error::RequestTimeoutError => EnvironmentError::Timeout,
        _ => EnvironmentError::Unavailable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::environments::{POLICY_VERSION, PreparationMetadata};
    use chrono::Utc;
    use factory_core::environments::{EnvironmentRuntime, ObservedState, ProvisioningState};
    use futures_util::{FutureExt, StreamExt};
    use serde_json::json;
    use std::sync::Arc;
    use tokio::io::AsyncWriteExt;
    use uuid::Uuid;

    fn environment(image: &str) -> (PreparationMetadata, Environment) {
        let installation = Uuid::new_v4();
        let id = Uuid::new_v4();
        let name = format!("factory-{}-{}", installation.simple(), id.simple());
        let metadata = serde_json::from_value(json!({"schemaVersion":1,
            "installationId":installation.to_string(),"imageId":image,"policyVersion":POLICY_VERSION}))
        .unwrap();
        (
            metadata,
            Environment {
                id,
                user_id: Uuid::new_v4(),
                installation_id: installation,
                desired_state: DesiredState::Running,
                observed_state: ObservedState::NotCreated,
                generation: 1,
                provisioning_state: ProvisioningState::VolumeCreating,
                volume_name: format!("{name}-home"),
                container_name: name,
                image_id: image.into(),
                policy_version: POLICY_VERSION.into(),
                observed_at: None,
                error_code: None,
                next_reconcile_at: Utc::now(),
            },
        )
    }

    #[tokio::test]
    async fn stopped_or_foreign_records_cannot_open_docker_processes() {
        let (metadata, mut env) = environment(&format!("sha256:{}", "a".repeat(64)));
        // No daemon is contacted: both refusals must occur before any Docker operation.
        let egress = crate::environments::fixture::synthetic_egress(metadata.installation_id());
        let runtime =
            DockerEnvironmentRuntime::connect(metadata, egress, Arc::new(|| true)).unwrap();
        env.desired_state = DesiredState::Stopped;
        assert!(matches!(
            runtime
                .start_process(&env, FixedProcess::TerminalAttach)
                .await,
            Err(EnvironmentError::Conflict)
        ));
        env.desired_state = DesiredState::Running;
        env.user_id = Uuid::nil();
        assert!(matches!(
            runtime
                .start_process(&env, FixedProcess::CodexAccount)
                .await,
            Err(EnvironmentError::OwnershipConflict)
        ));
    }

    async fn until(
        output: &mut ProcessOutput,
        marker: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        tokio::time::timeout(Duration::from_secs(15), async {
            let mut text = String::new();
            while let Some(chunk) = output.next().await {
                text.push_str(&String::from_utf8_lossy(chunk?.as_ref()));
                if text.contains(marker) {
                    return Ok(());
                }
                if text.len() > 65536 {
                    return Err("Unexpected fixture output".into());
                }
            }
            Err(format!("Process exited before fixture marker {marker:?}: {text:?}").into())
        })
        .await?
    }

    async fn exercise(
        runtime: &DockerEnvironmentRuntime,
        env: &mut Environment,
    ) -> Result<(), Box<dyn std::error::Error>> {
        runtime.create_volume(env).await?;
        env.provisioning_state = ProvisioningState::VolumeReady;
        runtime.start(env).await?;
        let mut first = runtime
            .start_process(env, FixedProcess::TerminalAttach)
            .await?;
        first
            .input
            .write_all(b"printf 'factory-%s\\n' ready\r")
            .await?;
        until(&mut first.output, "factory-ready").await?;
        runtime.resize_process(&first.id, 123, 37).await?;
        first.input.write_all(b"stty size\r").await?;
        until(&mut first.output, "37 123").await?;
        first.input.write_all(b"sleep 60\r").await?;
        tokio::time::sleep(Duration::from_millis(100)).await;
        first
            .input
            .write_all(b"\x03printf 'interrupt-%s\\n' ok\r")
            .await?;
        until(&mut first.output, "interrupt-ok").await?;
        first
            .input
            .write_all(b"sleep 1; printf persistent > ~/Documents/witness\r")
            .await?;
        let detached = runtime
            .start_process(env, FixedProcess::TerminalDetach)
            .await?;
        runtime
            .wait_process_exit(&detached.id, Duration::from_secs(5))
            .await?;
        runtime
            .wait_process_exit(&first.id, Duration::from_secs(5))
            .await?;
        drop((detached, first));
        // Reconnect with a new Docker client, as after an API process restart.
        tokio::time::sleep(Duration::from_secs(2)).await;
        let mut second = runtime
            .start_process(env, FixedProcess::TerminalAttach)
            .await?;
        second
            .input
            .write_all(b"cat ~/Documents/witness; printf -- '-%s\\n' restored\r")
            .await?;
        until(&mut second.output, "persistent-restored").await?;
        let mut account = runtime
            .start_process(env, FixedProcess::CodexAccount)
            .await?;
        account.input.write_all(b"{\"id\":1,\"method\":\"initialize\",\"params\":{\"clientInfo\":{\"name\":\"factory_fixture\",\"version\":\"0\"}}}\n").await?;
        until(&mut account.output, "\"id\":1").await?;
        let duplicate = runtime
            .start_process(env, FixedProcess::CodexAccount)
            .await?;
        assert_eq!(
            runtime
                .wait_process_exit(&duplicate.id, Duration::from_secs(5))
                .await?,
            75
        );
        account.input.shutdown().await?;
        let account_id = account.id.clone();
        drop(account);
        runtime
            .wait_process_exit(&account_id, Duration::from_secs(15))
            .await?;
        drop(duplicate);
        // EOF releases the lock, allowing a fresh controller without starting OAuth.
        let mut fresh = runtime
            .start_process(env, FixedProcess::CodexAccount)
            .await?;
        fresh.input.shutdown().await?;
        let fresh_id = fresh.id.clone();
        drop(fresh);
        assert_ne!(
            runtime
                .wait_process_exit(&fresh_id, Duration::from_secs(15))
                .await?,
            75
        );
        Ok(())
    }

    #[tokio::test]
    #[ignore = "Build the image and set FACTORY_PROCESS_TEST_IMAGE to its immutable image ID; local Docker only"]
    async fn local_docker_terminal_and_account_lifecycle() {
        let image =
            std::env::var("FACTORY_PROCESS_TEST_IMAGE").expect("Explicit fixture image required");
        let (metadata, mut env) = environment(&image);
        let egress = crate::environments::fixture::egress_metadata(metadata.installation_id());
        let runtime =
            DockerEnvironmentRuntime::connect(metadata, egress, Arc::new(|| true)).unwrap();
        let result = std::panic::AssertUnwindSafe(exercise(&runtime, &mut env))
            .catch_unwind()
            .await;
        crate::environments::fixture::cleanup(&runtime, &[&env]).await;
        result.unwrap().unwrap();
    }
}
