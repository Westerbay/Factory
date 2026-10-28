use factory_core::environments::EnvironmentError;
use serde::Deserialize;
use std::{
    fs::{File, OpenOptions},
    io::Read,
    path::Path,
};
use uuid::Uuid;

pub const POLICY_VERSION: &str = "1";

/// Trusted operator preparation, read once. Never discovers Docker contexts or credentials.
#[derive(Clone, Debug, Deserialize)]
#[serde(try_from = "RawPreparationMetadata")]
pub struct PreparationMetadata {
    installation_id: Uuid,
    image_id: String,
    policy_version: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawPreparationMetadata {
    schema_version: u32,
    installation_id: String,
    image_id: String,
    policy_version: String,
}

impl TryFrom<RawPreparationMetadata> for PreparationMetadata {
    type Error = EnvironmentError;
    fn try_from(raw: RawPreparationMetadata) -> Result<Self, Self::Error> {
        if raw.schema_version != 1 {
            return Err(EnvironmentError::InvalidConfiguration);
        }
        let metadata = Self {
            installation_id: Uuid::parse_str(&raw.installation_id)
                .map_err(|_| EnvironmentError::InvalidConfiguration)?,
            image_id: raw.image_id,
            policy_version: raw.policy_version,
        };
        metadata.validate()?;
        Ok(metadata)
    }
}

impl PreparationMetadata {
    pub fn load(directory: impl AsRef<Path>) -> Result<Self, EnvironmentError> {
        let file = File::open(directory.as_ref().join("prepared.json"))
            .map_err(|_| EnvironmentError::Unavailable)?;
        let mut bytes = Vec::new();
        file.take(4097)
            .read_to_end(&mut bytes)
            .map_err(|_| EnvironmentError::Unavailable)?;
        if bytes.len() > 4096 {
            return Err(EnvironmentError::InvalidConfiguration);
        }
        let metadata: Self =
            serde_json::from_slice(&bytes).map_err(|_| EnvironmentError::InvalidConfiguration)?;
        metadata.validate()?;
        Ok(metadata)
    }

    pub(super) fn validate(&self) -> Result<(), EnvironmentError> {
        if self.installation_id.is_nil()
            || self.policy_version != POLICY_VERSION
            || !immutable_image(&self.image_id)
        {
            return Err(EnvironmentError::InvalidConfiguration);
        }
        Ok(())
    }
    pub fn installation_id(&self) -> Uuid {
        self.installation_id
    }
    pub fn image_id(&self) -> &str {
        &self.image_id
    }
    pub fn policy_version(&self) -> &str {
        &self.policy_version
    }
}

pub(super) fn immutable_image(image: &str) -> bool {
    image.strip_prefix("sha256:").is_some_and(|hash| {
        hash.len() == 64
            && hash
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    })
}

/// Hold for the entire process, including after loss of the PostgreSQL supervisor.
/// Never unlink this file: another process may hold its inode open.
pub struct InstallationGuard {
    _file: File,
}

impl InstallationGuard {
    pub fn acquire(directory: impl AsRef<Path>) -> Result<Self, EnvironmentError> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(directory.as_ref().join("supervisor.lock"))
            .map_err(|_| EnvironmentError::Unavailable)?;
        file.try_lock().map_err(|_| EnvironmentError::Unavailable)?;
        Ok(Self { _file: file })
    }
}
