use super::{Environment, EnvironmentError, EnvironmentStart};
use async_trait::async_trait;
use uuid::Uuid;

/// A durable replacement intent. The old home is never part of the removal operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnvironmentUpgradeIntent {
    pub operation_id: Uuid,
    pub environment_id: Uuid,
    pub generation: i64,
    pub old_container_id: String,
    pub target_image_id: String,
    pub target_policy_version: String,
}

#[derive(Clone, Debug)]
pub struct EnvironmentUpgradeSnapshot {
    pub environment: Environment,
    pub pending: Option<EnvironmentUpgradeIntent>,
}

#[async_trait]
pub trait EnvironmentUpgradeStore: Send + Sync {
    /// Operator-only access by stable Factory user identity, without a browser session.
    async fn upgrade_snapshot(
        &self,
        user_id: Uuid,
        installation_id: Uuid,
    ) -> Result<EnvironmentUpgradeSnapshot, EnvironmentError>;

    /// Compare the preview under user/environment locks, then reserve a generation.
    /// An existing matching intent is returned unchanged for crash recovery.
    async fn begin_upgrade(
        &self,
        expected: &Environment,
        old_container_id: &str,
        target: &EnvironmentStart,
    ) -> Result<EnvironmentUpgradeIntent, EnvironmentError>;

    /// Atomically publish the target and clear the matching intent; keep Stopped.
    async fn finish_upgrade(
        &self,
        intent: &EnvironmentUpgradeIntent,
    ) -> Result<(), EnvironmentError>;
}

#[async_trait]
pub trait EnvironmentUpgradeRuntime: Send + Sync {
    /// Require the existing owned home and an owned stopped container, if present.
    /// Absence is interpreted by application policy, never as permission to recreate data.
    async fn inspect_upgrade(
        &self,
        environment: &Environment,
    ) -> Result<Option<String>, EnvironmentError>;

    async fn prepare_upgrade(&self, target: &Environment) -> Result<(), EnvironmentError>;

    /// Recheck ownership/state, then remove this exact stopped ID without its volume.
    async fn remove_upgrade_container(
        &self,
        environment: &Environment,
        container_id: &str,
    ) -> Result<(), EnvironmentError>;
}
