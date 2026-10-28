use super::TargetError;
use std::{
    path::PathBuf,
    process::Stdio,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::Command,
};

const MAX_OUTPUT: u64 = 1024 * 1024;
const DEADLINE: Duration = Duration::from_secs(10);
static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

fn command(config: &std::path::Path) -> Command {
    let mut command = Command::new("docker");
    command
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if let Some(value) = std::env::var_os("PATH") {
        command.env("PATH", value);
    }
    #[cfg(windows)]
    {
        // Process startup needs these; Docker's config is separately fixed to an empty directory.
        for name in ["SystemRoot", "USERPROFILE", "ProgramFiles"] {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    let host = if cfg!(windows) {
        "npipe:////./pipe/docker_engine"
    } else {
        "unix:///var/run/docker.sock"
    };
    command.arg("--config").arg(config).args(["--host", host]);
    command
}

async fn read_limited(stream: impl AsyncRead + Unpin) -> Result<Vec<u8>, TargetError> {
    let mut bytes = Vec::new();
    stream
        .take(MAX_OUTPUT + 1)
        .read_to_end(&mut bytes)
        .await
        .map_err(|_| TargetError::DockerUnavailable)?;
    if bytes.len() as u64 > MAX_OUTPUT {
        return Err(TargetError::DockerOutputLimit);
    }
    Ok(bytes)
}

async fn execute(mut command: Command, deadline: Duration) -> Result<Vec<u8>, TargetError> {
    let mut child = command
        .spawn()
        .map_err(|_| TargetError::DockerUnavailable)?;
    let stdout = child.stdout.take().ok_or(TargetError::DockerUnavailable)?;
    let stderr = child.stderr.take().ok_or(TargetError::DockerUnavailable)?;
    let result = tokio::time::timeout(deadline, async {
        tokio::try_join!(read_limited(stdout), read_limited(stderr), async {
            child
                .wait()
                .await
                .map_err(|_| TargetError::DockerUnavailable)
        })
    })
    .await;
    match result {
        Ok(Ok((stdout, _, status))) if status.success() => Ok(stdout),
        other => {
            // Never retain process output in errors, even when Docker prints its environment.
            tokio::time::timeout(DEADLINE, child.kill())
                .await
                .map_err(|_| TargetError::DockerTimeout)?
                .map_err(|_| TargetError::DockerUnavailable)?;
            match other {
                Err(_) => Err(TargetError::DockerTimeout),
                Ok(Err(error)) => Err(error),
                _ => Err(TargetError::DockerUnavailable),
            }
        }
    }
}

struct EmptyConfig(PathBuf);
impl EmptyConfig {
    fn create() -> Result<Self, TargetError> {
        let time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| TargetError::DockerUnavailable)?
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "factory-docker-inspect-{}-{time}-{}",
            std::process::id(),
            NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        let builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        let builder = {
            use std::os::unix::fs::DirBuilderExt;
            let mut builder = builder;
            builder.mode(0o700);
            builder
        };
        builder
            .create(&path)
            .map_err(|_| TargetError::DockerUnavailable)?;
        Ok(Self(path))
    }
    fn remove(&self) -> Result<(), TargetError> {
        std::fs::remove_dir(&self.0).map_err(|_| TargetError::DockerUnavailable)
    }
}
impl Drop for EmptyConfig {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir(&self.0);
    }
}

pub(super) async fn inspect(args: &[&str]) -> Result<Vec<u8>, TargetError> {
    let config = EmptyConfig::create()?;
    let mut process = command(&config.0);
    process.args(args);
    let result = execute(process, DEADLINE).await;
    config.remove()?;
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn docker_uses_only_the_local_socket_and_allowlisted_environment() {
        let process = command(std::path::Path::new("/fixture/empty-config"));
        let args: Vec<_> = process
            .as_std()
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert_eq!(&args[..3], ["--config", "/fixture/empty-config", "--host"]);
        assert_eq!(
            args[3],
            if cfg!(windows) {
                "npipe:////./pipe/docker_engine"
            } else {
                "unix:///var/run/docker.sock"
            }
        );
        for (name, _) in process.as_std().get_envs() {
            let allowed = if cfg!(windows) {
                &["PATH", "SystemRoot", "USERPROFILE", "ProgramFiles"][..]
            } else {
                &["PATH"][..]
            };
            assert!(
                allowed
                    .iter()
                    .any(|allowed| name.eq_ignore_ascii_case(allowed))
            );
        }
    }

    #[tokio::test]
    async fn output_is_bounded_before_parsing() {
        assert_eq!(
            read_limited(&vec![b'x'; MAX_OUTPUT as usize + 1][..])
                .await
                .unwrap_err(),
            TargetError::DockerOutputLimit
        );
        assert_eq!(read_limited(&b"[]"[..]).await.unwrap(), b"[]");
    }

    // Re-enter this test binary as an inert process fixture; never invoke a shell.
    #[test]
    fn child_process_fixture() {
        match std::env::var("FACTORY_CHILD_FIXTURE").as_deref() {
            Ok("timeout") => std::thread::sleep(Duration::from_secs(30)),
            Ok("output") => {
                use std::io::Write;
                let _ = std::io::stdout().write_all(&vec![b'x'; MAX_OUTPUT as usize + 1024]);
            }
            Ok("failure") => panic!("synthetic-secret-must-not-escape"),
            _ => {}
        }
    }

    #[tokio::test]
    async fn process_deadline_output_limit_and_failures_are_enforced() {
        for (mode, expected) in [
            ("timeout", TargetError::DockerTimeout),
            ("output", TargetError::DockerOutputLimit),
            ("failure", TargetError::DockerUnavailable),
        ] {
            let mut process = Command::new(std::env::current_exe().unwrap());
            process
                .env_clear()
                .env("FACTORY_CHILD_FIXTURE", mode)
                .args([
                    "--exact",
                    "local_target::docker::tests::child_process_fixture",
                    "--nocapture",
                ])
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true);
            #[cfg(windows)]
            {
                if let Some(value) = std::env::var_os("SystemRoot") {
                    process.env("SystemRoot", value);
                }
                process.creation_flags(0x08000000);
            }
            let limit = if mode == "timeout" {
                Duration::from_millis(100)
            } else {
                Duration::from_secs(5)
            };
            let error = execute(process, limit).await.unwrap_err();
            assert_eq!(error, expected, "{mode}");
            assert!(!error.to_string().contains("synthetic-secret"));
        }
    }
}
