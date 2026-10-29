use super::*;
use async_trait::async_trait;
use uuid::Uuid;

#[async_trait]
pub trait EnvironmentStore: Send + Sync {
    /// Revalidate user/session in the transaction; reads never create records.
    async fn get(&self, actor: EnvironmentActor) -> Result<Option<Environment>, EnvironmentError>;
    async fn start(
        &self,
        actor: EnvironmentActor,
        request: EnvironmentStart,
    ) -> Result<Environment, EnvironmentError>;
    async fn stop(&self, actor: EnvironmentActor) -> Result<Option<Environment>, EnvironmentError>;
    /// Repair suspended intent under the user lock, then return bounded due records.
    async fn reconcile_batch(
        &self,
        installation_id: Uuid,
        limit: u32,
    ) -> Result<Vec<Environment>, EnvironmentError>;
    /// Commit VolumeCreating before an external creation. False means stale generation/phase.
    async fn begin_provisioning(&self, id: Uuid, generation: i64)
    -> Result<bool, EnvironmentError>;
    /// Generation-conditional write; stale runtime results cannot overwrite a newer intent.
    async fn record_observation(
        &self,
        id: Uuid,
        generation: i64,
        observation: EnvironmentObservation,
        retry_seconds: u32,
    ) -> Result<bool, EnvironmentError>;
}

#[async_trait]
pub trait EnvironmentRuntime: Send + Sync {
    async fn inspect(
        &self,
        environment: &Environment,
    ) -> Result<RuntimeInspection, EnvironmentError>;
    /// Caller must first persist VolumeCreating. Never replaces an existing volume.
    async fn create_volume(&self, environment: &Environment) -> Result<(), EnvironmentError>;
    /// Verify complete ownership, image, volume and policy before creating/starting.
    async fn start(&self, environment: &Environment) -> Result<(), EnvironmentError>;
    /// Verify ownership before stopping. Never removes the volume.
    async fn stop(&self, environment: &Environment) -> Result<(), EnvironmentError>;
}

#[async_trait]
pub trait EnvironmentSupervisor: Send {
    /// A failed check is terminal for this guard. No new runtime operation may follow it.
    async fn check(&mut self) -> Result<(), EnvironmentError>;
}
