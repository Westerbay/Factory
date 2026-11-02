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
use uuid::Uuid;

pub(crate) type ProcessOutput = Pin<Box<dyn Stream<Item = Result<LogOutput, Error>> + Send>>;
pub(crate) type ProcessInput = Pin<Box<dyn AsyncWrite + Send>>;

/// Account cleanup must shut down stdin and drop BOTH stream halves before waiting
/// on the saved ID. A half-closed Docker connection can otherwise retain exec stdin.
pub(crate) struct DockerProcess {
    pub id: String,
    pub input: ProcessInput,
    pub output: ProcessOutput,
}

/// The phase is determined structurally: the preflight future completes before
/// the create/start future is constructed. No mutable flag survives cancellation.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ProcessStartError {
    PreSubmission(EnvironmentError),
    Submitted(EnvironmentError),
}
impl From<ProcessStartError> for EnvironmentError {
    fn from(error: ProcessStartError) -> Self {
        match error {
            ProcessStartError::PreSubmission(error) | ProcessStartError::Submitted(error) => error,
        }
    }
}

/// No string command or Docker identifier supplied by a browser can reach this enum.
#[derive(Clone)]
pub(crate) enum FixedProcess {
    TerminalAttach,
    TerminalDetach,
    TerminalAttachConversation {
        conversation: Uuid,
        project: Option<String>,
    },
    TerminalDetachConversation {
        conversation: Uuid,
    },
    CodexAccount,
    CodexExecution,
    ClaudeStatus,
    ClaudeLogout,
    ClaudeLogin,
    ClaudeProbe,
    ClaudeExecution(ClaudeLaunch),
    /// Fixed, immutable controller; its bounded JSON input never selects an executable.
    ProjectController,
    #[cfg(test)]
    CodexSchema,
}

/// Arguments are individual Docker exec arguments, never a shell command.
#[derive(Clone)]
pub(crate) struct ClaudeLaunch {
    model: String,
    effort: Option<String>,
    session: uuid::Uuid,
    resume: bool,
    cwd: Option<String>,
}

impl ClaudeLaunch {
    pub(crate) fn new(
        model: &str,
        effort: Option<&str>,
        session: uuid::Uuid,
        resume: bool,
    ) -> Option<Self> {
        if model.is_empty()
            || model.len() > 256
            || !model.as_bytes()[0].is_ascii_alphanumeric()
            || !model
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"-_.:/[]".contains(&c))
            || effort
                .is_some_and(|value| !matches!(value, "low" | "medium" | "high" | "xhigh" | "max"))
            || session.is_nil()
        {
            return None;
        }
        Some(Self {
            model: model.to_owned(),
            effort: effort.map(str::to_owned),
            session,
            resume,
            cwd: None,
        })
    }

    pub(crate) fn with_cwd(mut self, cwd: String) -> Self {
        self.cwd = Some(cwd);
        self
    }
}

impl FixedProcess {
    fn options(self) -> CreateExecOptions<String> {
        let working_dir = match &self {
            Self::ClaudeExecution(launch) => {
                launch.cwd.as_deref().unwrap_or("/home/factory/Documents")
            }
            _ => "/home/factory/Documents",
        }
        .to_owned();
        let terminal = matches!(
            self,
            Self::TerminalAttach | Self::TerminalAttachConversation { .. } | Self::ClaudeLogin
        );
        let (session, cwd) = match &self {
            Self::TerminalAttachConversation {
                conversation,
                project,
            } => (
                format!("factory-{conversation}"),
                project.as_ref().map_or_else(
                    || "/home/factory/Documents".to_owned(),
                    |name| format!("/home/factory/Documents/{name}"),
                ),
            ),
            Self::TerminalDetachConversation { conversation } => {
                (format!("factory-{conversation}"), String::new())
            }
            _ => (String::new(), String::new()),
        };
        let mut cmd: Vec<String> = match &self {
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
            Self::TerminalAttachConversation { .. } => vec![
                "/usr/bin/tmux",
                "-f",
                "/etc/factory/tmux.conf",
                "-S",
                "/tmp/factory-tmux.sock",
                "new-session",
                "-A",
                "-D",
                "-s",
                &session,
                "-c",
                &cwd,
            ],
            Self::TerminalDetachConversation { .. } => vec![
                "/usr/bin/tmux",
                "-S",
                "/tmp/factory-tmux.sock",
                "detach-client",
                "-s",
                &session,
            ],
            Self::CodexAccount => vec!["/usr/local/bin/factory-auth-controller"],
            Self::ProjectController => vec!["/usr/local/bin/factory-project-controller"],
            // The fixed executable and fixed arguments are never selected by the browser.
            // A watchdog bounds orphan lifetime; it does not prove descendant cleanup.
            Self::CodexExecution => vec![
                "/usr/bin/timeout",
                "--signal=TERM",
                "--kill-after=5s",
                "86400s",
                "/opt/codex/bin/codex",
                "-c",
                "cli_auth_credentials_store=\"file\"",
                "app-server",
                "--listen",
                "stdio://",
            ],
            Self::ClaudeStatus => vec![
                "/usr/bin/timeout", "--signal=TERM", "--kill-after=5s", "30s",
                "/opt/claude/claude", "auth", "status",
            ],
            Self::ClaudeLogout => vec![
                "/usr/bin/timeout", "--signal=TERM", "--kill-after=5s", "30s",
                "/usr/bin/flock", "--nonblock", "--conflict-exit-code=75", "--no-fork",
                "/tmp/factory-claude-auth.lock", "/opt/claude/claude", "auth", "logout",
            ],
            // The CLI needs a private PTY for Ctrl-C. Disable echo before the
            // browser-issued code reaches stdin; no caller input enters the command.
            Self::ClaudeLogin => vec![
                "/bin/sh", "-c",
                "stty -echo; exec /usr/bin/timeout --signal=TERM --kill-after=5s 595s /usr/bin/flock --nonblock --conflict-exit-code=75 --no-fork /tmp/factory-claude-auth.lock /opt/claude/claude auth login --claudeai",
            ],
            Self::ClaudeProbe => vec![
                "/usr/bin/timeout", "--signal=TERM", "--kill-after=5s", "60s",
                "/opt/claude/claude", "--print", "--input-format", "stream-json",
                "--output-format", "stream-json", "--verbose", "--no-session-persistence",
                "--strict-mcp-config", "--settings", "{\"disableAllHooks\":true}",
            ],
            Self::ClaudeExecution(_) => vec![
                "/usr/bin/timeout", "--signal=TERM", "--kill-after=5s", "86400s",
                "/opt/claude/claude", "--print", "--input-format", "stream-json",
                "--output-format", "stream-json", "--verbose", "--include-partial-messages",
                "--dangerously-skip-permissions", "--permission-prompt-tool", "stdio",
            ],
            #[cfg(test)]
            Self::CodexSchema => vec![
                "/bin/sh",
                "-c",
                "/opt/codex/bin/codex app-server generate-json-schema --out /tmp/factory-schema && cat /tmp/factory-schema/v2/TurnStartParams.json",
            ],
        }.into_iter().map(str::to_owned).collect();
        if let Self::ClaudeExecution(launch) = self {
            cmd.push(
                if launch.resume {
                    "--resume"
                } else {
                    "--session-id"
                }
                .into(),
            );
            cmd.push(launch.session.to_string());
            cmd.push("--model".into());
            cmd.push(launch.model);
            if let Some(effort) = launch.effort {
                cmd.push("--effort".into());
                cmd.push(effort);
            }
        }
        CreateExecOptions {
            attach_stdin: Some(true),
            attach_stdout: Some(true),
            attach_stderr: Some(true),
            tty: Some(terminal),
            privileged: Some(false),
            user: Some("10000:10000".into()),
            working_dir: Some(working_dir),
            env: Some(vec![
                "HOME=/home/factory".into(),
                "CODEX_HOME=/home/factory/.codex".into(),
                "TERM=xterm-256color".into(),
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
        self.start_process_checked(env, command)
            .await
            .map_err(Into::into)
    }

    /// Only a completed preflight may construct a create/start future. Any
    /// error after its construction is conservative: Docker may have received
    /// a request even if the caller observed a timeout or transport failure.
    pub(crate) async fn start_process_checked(
        &self,
        env: &Environment,
        command: FixedProcess,
    ) -> Result<DockerProcess, ProcessStartError> {
        let deadline = tokio::time::Instant::now() + self.timeout;
        let container_id = tokio::time::timeout_at(deadline, async {
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
            container["Id"]
                .as_str()
                .map(str::to_owned)
                .ok_or(EnvironmentError::OwnershipConflict)
        })
        .await
        .map_err(|_| ProcessStartError::PreSubmission(EnvironmentError::Timeout))?
        .map_err(ProcessStartError::PreSubmission)?;

        self.require_mutation_permission()
            .map_err(ProcessStartError::PreSubmission)?;
        tokio::time::timeout_at(deadline, async {
            let options = command.options();
            let tty = options.tty == Some(true);
            let exec = self
                .docker
                .create_exec(&container_id, options)
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
        .map_err(|_| ProcessStartError::Submitted(EnvironmentError::Timeout))?
        .map_err(ProcessStartError::Submitted)
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
    #[test]
    fn terminal_commands_are_scoped_to_each_conversation() {
        let one = uuid::Uuid::new_v4();
        let two = uuid::Uuid::new_v4();
        let project = "repo".to_owned();
        let args = super::FixedProcess::TerminalAttachConversation {
            conversation: one,
            project: Some(project.clone()),
        }
        .options()
        .cmd
        .unwrap();
        assert!(
            args.windows(2)
                .any(|pair| pair == ["-s", &format!("factory-{one}")])
        );
        assert!(
            args.windows(2)
                .any(|pair| pair == ["-c", &format!("/home/factory/Documents/{project}")])
        );
        let detach = super::FixedProcess::TerminalDetachConversation { conversation: two }
            .options()
            .cmd
            .unwrap();
        assert!(
            detach
                .windows(2)
                .any(|pair| pair == ["-s", &format!("factory-{two}")])
        );
        assert!(!detach.iter().any(|arg| arg == &format!("factory-{one}")));
    }
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

    #[test]
    fn claude_effort_is_a_validated_native_argument() {
        let session = Uuid::new_v4();
        let launch = ClaudeLaunch::new("opus", Some("high"), session, false).unwrap();
        let command = FixedProcess::ClaudeExecution(launch).options().cmd.unwrap();
        assert!(command.windows(2).any(|part| part == ["--effort", "high"]));
        assert!(ClaudeLaunch::new("opus", Some("--help"), session, false).is_none());
        let default = ClaudeLaunch::new("opus", None, session, false).unwrap();
        assert!(
            !FixedProcess::ClaudeExecution(default)
                .options()
                .cmd
                .unwrap()
                .contains(&"--effort".to_owned())
        );
    }

    #[test]
    fn claude_project_launch_uses_project_directory() {
        let cwd = format!("/home/factory/Documents/project-{}", Uuid::new_v4());
        for resume in [false, true] {
            let launch = ClaudeLaunch::new("opus", Some("high"), Uuid::new_v4(), resume)
                .unwrap()
                .with_cwd(cwd.clone());
            let options = FixedProcess::ClaudeExecution(launch).options();
            assert_eq!(options.working_dir.as_deref(), Some(cwd.as_str()));
            assert!(
                options
                    .cmd
                    .unwrap()
                    .windows(2)
                    .any(|part| part == ["--effort", "high"])
            );
        }
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
        let mut text = String::new();
        tokio::time::timeout(Duration::from_secs(15), async {
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
        .await
        .map_err(|_| format!("Timed out waiting for fixture marker: {marker}; output: {text:?}"))?
    }

    async fn exercise(
        runtime: &Arc<DockerEnvironmentRuntime>,
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
        first.input.write_all(b"claude --version\r").await?;
        until(&mut first.output, "2.1.284 (Claude Code)").await?;
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
            .write_all(b"printf '%s-restored\\n' \"$(cat ~/Documents/witness)\"\r")
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
        // Exercise the exact fixed app-server launch without credentials or a provider turn.
        let mut execution = runtime
            .start_process(env, FixedProcess::CodexExecution)
            .await?;
        execution.input.write_all(b"{\"id\":9,\"method\":\"initialize\",\"params\":{\"clientInfo\":{\"name\":\"factory_fixture\",\"version\":\"0\"}}}\n").await?;
        until(&mut execution.output, "\"id\":9").await?;
        execution.input.shutdown().await?;
        let execution_id = execution.id.clone();
        drop(execution);
        assert_eq!(
            runtime
                .wait_process_exit(&execution_id, Duration::from_secs(15))
                .await?,
            0
        );

        // Exercise the unmodified Claude binary in the isolated, unsigned-in home.
        let mut status = runtime
            .start_process(env, FixedProcess::ClaudeStatus)
            .await?;
        until(&mut status.output, "\"loggedIn\": false").await?;
        let status_id = status.id.clone();
        drop(status);
        assert_eq!(
            runtime
                .wait_process_exit(&status_id, Duration::from_secs(15))
                .await?,
            1
        );
        let mut probe = runtime
            .start_process(env, FixedProcess::ClaudeProbe)
            .await?;
        probe.input.write_all(b"{\"type\":\"control_request\",\"request_id\":\"factory-init\",\"request\":{\"subtype\":\"initialize\",\"hooks\":{},\"mcpServers\":{}}}\n").await?;
        until(&mut probe.output, "\"models\"").await?;
        probe.input.shutdown().await?;
        let probe_id = probe.id.clone();
        drop(probe);
        assert_eq!(
            runtime
                .wait_process_exit(&probe_id, Duration::from_secs(15))
                .await?,
            0
        );
        // The real login runs in an echo-disabled PTY and must stop on cancellation.
        let mut login = runtime
            .start_process(env, FixedProcess::ClaudeLogin)
            .await?;
        let mut output = Vec::new();
        tokio::time::timeout(Duration::from_secs(20), async {
            while let Some(chunk) = login.output.next().await {
                output.extend_from_slice(chunk?.as_ref());
                if output
                    .windows(b"Paste code here".len())
                    .any(|part| part == b"Paste code here")
                {
                    return Ok::<(), Box<dyn std::error::Error>>(());
                }
                if output.len() > 65536 {
                    return Err("Login output exceeded limit".into());
                }
            }
            Err("Login exited before the browser prompt".into())
        })
        .await??;
        let synthetic_code = b"factory-synthetic-no-echo";
        login.input.write_all(synthetic_code).await?;
        login.input.flush().await?;
        tokio::time::sleep(Duration::from_millis(100)).await;
        login.input.write_all(b"\x03").await?;
        login.input.flush().await?;
        tokio::time::timeout(Duration::from_secs(10), async {
            while let Some(chunk) = login.output.next().await {
                output.extend_from_slice(chunk?.as_ref());
                if output.len() > 65536 {
                    return Err::<(), Box<dyn std::error::Error>>(
                        "Login output exceeded limit".into(),
                    );
                }
            }
            Ok(())
        })
        .await??;
        assert!(
            !output
                .windows(synthetic_code.len())
                .any(|part| part == synthetic_code)
        );
        let login_id = login.id.clone();
        drop(login);
        assert_ne!(
            runtime
                .wait_process_exit(&login_id, Duration::from_secs(15))
                .await?,
            0
        );
        // Exercise the adapter's real URL parsing and confirmed cancellation twice.
        let claude = crate::claude_account::ClaudeAccountAdapter::new(runtime.clone());
        for _ in 0..2 {
            let operation = Uuid::new_v4();
            let result = factory_core::agent_connections::AgentConnectionPort::begin_login(
                &claude,
                env,
                operation,
                Utc::now() + chrono::Duration::minutes(2),
            )
            .await?;
            let challenge = result
                .challenge
                .ok_or("Missing Claude authorization challenge")?;
            assert_eq!(challenge.operation_id, operation);
            assert!(
                challenge
                    .verification_url
                    .starts_with("https://claude.com/cai/oauth/authorize?")
            );
            assert!(challenge.user_code.is_none());
            factory_core::agent_connections::AgentConnectionPort::cancel_login(
                &claude, env, operation,
            )
            .await?;
        }

        // The image's pinned binary owns the contract, not a copied documentation enum.
        let mut schema = runtime
            .start_process(env, FixedProcess::CodexSchema)
            .await?;
        schema.input.shutdown().await?;
        let mut bytes = Vec::new();
        while let Some(chunk) =
            tokio::time::timeout(Duration::from_secs(30), schema.output.next()).await?
        {
            if let LogOutput::StdOut { message } = chunk? {
                bytes.extend_from_slice(&message);
            }
            if bytes.len() > 1024 * 1024 {
                return Err("Schema fixture exceeded output limit".into());
            }
        }
        let schema_id = schema.id.clone();
        drop(schema);
        assert_eq!(
            runtime
                .wait_process_exit(&schema_id, Duration::from_secs(15))
                .await?,
            0
        );
        let value: serde_json::Value = serde_json::from_slice(&bytes)?;
        let rendered = serde_json::to_string(&value)?;
        assert!(rendered.contains("approvalPolicy") && rendered.contains("sandboxPolicy"));
        assert!(rendered.contains("dangerFullAccess") && rendered.contains("never"));
        Ok(())
    }

    #[tokio::test]
    #[ignore = "Build the image and set FACTORY_PROCESS_TEST_IMAGE to its immutable image ID; local Docker only"]
    async fn local_docker_terminal_and_account_lifecycle() {
        let image =
            std::env::var("FACTORY_PROCESS_TEST_IMAGE").expect("Explicit fixture image required");
        let (metadata, mut env) = environment(&image);
        let egress = crate::environments::fixture::egress_metadata(metadata.installation_id());
        let runtime = Arc::new(
            DockerEnvironmentRuntime::connect(metadata, egress, Arc::new(|| true)).unwrap(),
        );
        let result = std::panic::AssertUnwindSafe(exercise(&runtime, &mut env))
            .catch_unwind()
            .await;
        crate::environments::fixture::cleanup(&runtime, &[&env]).await;
        result.unwrap().unwrap();
    }
}
