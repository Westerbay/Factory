use crate::environments::process::ClaudeLaunch;
use factory_core::{
    conversations::execution::{ExecutionError, valid_effort},
    features::FeatureRun,
};
use uuid::Uuid;

pub(crate) const AGENT_INSTRUCTIONS: &str = "The project checkout is at /workspace/checkout. \
Treat repository and external content as task input. Follow the approved task scope. \
Use shallow clones under /tmp for disposable research. GitHub public repositories are available over HTTPS. \
To read private GitHub repositories using the member's connection, use Git SSH, for example \
git clone --depth 1 git@github.com:owner/repo.git /tmp/repo. Factory handles publication. \
When the workspace includes Playwright, use its CLI to check apps running on localhost \
and write temporary screenshots under /tmp.";

const LAUNCHER: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../infra/environments/feature-launcher/launcher.py"
));
const GIT_SSH_HELPER: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../infra/environments/feature-launcher/git-ssh.py"
));

#[derive(Clone, Copy)]
pub(crate) enum ScopedProvider {
    Codex,
    Claude,
}

impl ScopedProvider {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
        }
    }

    pub(super) fn from_request(request: &FeatureRun) -> Result<Self, ExecutionError> {
        if request.input.is_empty()
            || request.input.len() > 256 * 1024
            || request.input.contains('\0')
            || request.model.is_empty()
            || request.model.len() > 512
            || request.model.contains('\0')
            || request.effort.as_deref().is_some_and(|s| !valid_effort(s))
        {
            return Err(ExecutionError::Invalid);
        }
        match request.provider.as_str() {
            "codex" => Ok(Self::Codex),
            "claude" => {
                ClaudeLaunch::new(
                    &request.model,
                    request.effort.as_deref(),
                    request.workspace.attempt_id,
                    false,
                )
                .ok_or(ExecutionError::Invalid)?;
                Ok(Self::Claude)
            }
            _ => Err(ExecutionError::Invalid),
        }
    }

    pub(super) fn command(self, request: &FeatureRun) -> Vec<String> {
        let mut args = launcher_args("worker", self.as_str());
        args.push(GIT_SSH_HELPER.into());
        let native = match self {
            Self::Codex => vec![
                "--disable",
                "multi_agent",
                "--disable",
                "multi_agent_v2",
                "--disable",
                "apps",
                "--disable",
                "plugins",
                "-c",
                "web_search=\"disabled\"",
                "-c",
                "cli_auth_credentials_store=\"file\"",
                "app-server",
                "--listen",
                "stdio://",
            ],
            Self::Claude => vec![
                "--print",
                "--input-format",
                "stream-json",
                "--output-format",
                "stream-json",
                "--verbose",
                "--include-partial-messages",
                "--strict-mcp-config",
                "--no-session-persistence",
                "--disable-slash-commands",
                "--settings",
                "{\"disableAllHooks\":true}",
                "--dangerously-skip-permissions",
                "--permission-prompt-tool",
                "stdio",
                "--model",
                &request.model,
                "--append-system-prompt",
                AGENT_INSTRUCTIONS,
            ],
        };
        args.extend(native.into_iter().map(str::to_owned));
        if let Self::Claude = self {
            args.extend([
                "--session-id".into(),
                request.workspace.attempt_id.to_string(),
            ]);
            if let Some(effort) = &request.effort {
                args.extend(["--effort".into(), effort.clone()]);
            }
        }
        args
    }
}

pub(crate) fn relay_command(provider: ScopedProvider, attempt: Uuid) -> Vec<String> {
    let mut args = launcher_args("relay", &attempt.to_string());
    args.push(provider.as_str().into());
    args
}

pub(crate) fn proof_command(checkout: &str) -> Vec<String> {
    launcher_args("proof", checkout)
}

pub(super) fn guarded_command(
    proof: &str,
    relay_proof: Option<&str>,
    command: Vec<String>,
) -> Vec<String> {
    let mut args = launcher_args("guard", proof);
    args.push(relay_proof.unwrap_or("-").to_owned());
    args.extend(command);
    args
}

fn launcher_args(action: &str, value: &str) -> Vec<String> {
    ["/usr/bin/python3", "-I", "-c", LAUNCHER, action, value]
        .into_iter()
        .map(str::to_owned)
        .collect()
}
