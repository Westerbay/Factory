//! Local Docker ownership must be established before any PostgreSQL connection.
mod docker;
mod inspection;
mod proof;

pub use proof::{TargetProof, check_proof};
use std::{collections::BTreeMap, fmt, path::Path};
use tokio::io::AsyncReadExt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetError {
    InvalidProof,
    WorkspaceUnavailable,
    MarkerUnavailable,
    DockerUnavailable,
    DockerTimeout,
    DockerOutputLimit,
    InspectionMismatch,
}

impl fmt::Display for TargetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidProof => "Refusing database operation: invalid local target proof",
            Self::WorkspaceUnavailable => "Factory workspace is unavailable or unsupported",
            Self::MarkerUnavailable => {
                "Factory ownership marker is missing or invalid; run explicit database setup"
            }
            Self::DockerUnavailable => "Local Docker inspection failed",
            Self::DockerTimeout => "Local Docker inspection timed out",
            Self::DockerOutputLimit => "Local Docker inspection exceeded its output limit",
            Self::InspectionMismatch => {
                "Refusing database operation: live resource ownership differs"
            }
        })
    }
}
impl std::error::Error for TargetError {}

/// Created only after live container and volume inspection. Never deserialize this type.
pub struct VerifiedTarget(TargetProof);

impl VerifiedTarget {
    pub fn host(&self) -> &str {
        &self.0.host
    }
    pub fn port(&self) -> u16 {
        self.0.port as u16
    }
    pub fn database(&self) -> &str {
        &self.0.database
    }
    pub fn username(&self) -> &str {
        &self.0.username
    }
    pub fn password(&self) -> &str {
        &self.0.password
    }
}

pub async fn from_workspace(root: &Path) -> Result<VerifiedTarget, TargetError> {
    let directory = workspace_directory(root).await?;
    let state = Path::new(&directory).join(".factory");
    let state_metadata = tokio::fs::symlink_metadata(&state)
        .await
        .map_err(|_| TargetError::MarkerUnavailable)?;
    let marker = state.join("database-owner");
    let metadata = tokio::fs::symlink_metadata(&marker)
        .await
        .map_err(|_| TargetError::MarkerUnavailable)?;
    if !state_metadata.is_dir()
        || state_metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() > 128
    {
        return Err(TargetError::MarkerUnavailable);
    }
    let file = tokio::fs::File::open(marker)
        .await
        .map_err(|_| TargetError::MarkerUnavailable)?;
    let mut bytes = Vec::new();
    file.take(129)
        .read_to_end(&mut bytes)
        .await
        .map_err(|_| TargetError::MarkerUnavailable)?;
    if bytes.len() > 128 {
        return Err(TargetError::MarkerUnavailable);
    }
    let owner = std::str::from_utf8(&bytes)
        .map_err(|_| TargetError::MarkerUnavailable)?
        .trim();
    if !proof::uuid(owner) {
        return Err(TargetError::MarkerUnavailable);
    }
    inspect(expected(directory, "factory-local", owner, 55432)).await
}

/// Test harnesses supply only their unique owned project identity, never a database URL.
/// This still requires fresh live Docker inspection, including its owned volume.
pub async fn from_fixture(
    root: &Path,
    project: &str,
    owner: &str,
    port: u16,
) -> Result<VerifiedTarget, TargetError> {
    if !proof::fixture_project(project) {
        return Err(TargetError::InvalidProof);
    }
    inspect(expected(
        workspace_directory(root).await?,
        project,
        owner,
        port,
    ))
    .await
}

async fn workspace_directory(root: &Path) -> Result<String, TargetError> {
    let canonical = tokio::fs::canonicalize(root)
        .await
        .map_err(|_| TargetError::WorkspaceUnavailable)?;
    if !canonical.is_dir() {
        return Err(TargetError::WorkspaceUnavailable);
    }
    let directory = canonical
        .to_str()
        .ok_or(TargetError::WorkspaceUnavailable)?;
    // Windows canonicalization adds a verbatim prefix absent from Node's resolve().
    let directory = directory.strip_prefix("\\\\?\\").unwrap_or(directory);
    Ok(directory.to_owned())
}

fn expected(directory: String, project: &str, owner: &str, port: u16) -> TargetProof {
    TargetProof {
        project: project.into(),
        owner: owner.into(),
        directory: directory.clone(),
        database: if project == "factory-local" {
            "factory_local"
        } else {
            "factory_test"
        }
        .into(),
        username: "factory".into(),
        password: "local-fixture-only".into(),
        host: "127.0.0.1".into(),
        port: port.into(),
        container: format!("{project}-postgres-1"),
        labels: BTreeMap::from([
            ("com.docker.compose.project".into(), project.into()),
            ("io.factory.owner".into(), owner.into()),
            ("io.factory.directory".into(), directory),
        ]),
        published_host: "127.0.0.1".into(),
        published_port: port.into(),
    }
}

async fn inspect(proof: TargetProof) -> Result<VerifiedTarget, TargetError> {
    check_proof(&proof)?;
    let container = docker::inspect(&["inspect", &proof.container]).await?;
    let volume_name = format!("{}_postgres_data", proof.project);
    let volume = docker::inspect(&["volume", "inspect", &volume_name]).await?;
    inspection::verify(&proof, &container, &volume)?;
    Ok(VerifiedTarget(proof))
}
