mod mapping;
mod requests;
mod supervision;

use factory_core::environments::*;
use sqlx::PgPool;
use uuid::Uuid;

pub use supervision::PgEnvironmentSupervisor;

#[derive(Clone)]
pub struct PgEnvironmentStore {
    pub(crate) pool: PgPool,
}

fn unavailable(_: impl std::fmt::Display) -> EnvironmentError {
    EnvironmentError::Unavailable
}

#[async_trait::async_trait]
impl EnvironmentStore for PgEnvironmentStore {
    async fn get(&self, actor: EnvironmentActor) -> Result<Option<Environment>, EnvironmentError> {
        requests::get(&self.pool, actor).await
    }
    async fn start(
        &self,
        actor: EnvironmentActor,
        request: EnvironmentStart,
    ) -> Result<Environment, EnvironmentError> {
        requests::start(&self.pool, actor, request).await
    }
    async fn stop(&self, actor: EnvironmentActor) -> Result<Option<Environment>, EnvironmentError> {
        requests::stop(&self.pool, actor).await
    }
    async fn reconcile_batch(
        &self,
        installation_id: Uuid,
        limit: u32,
    ) -> Result<Vec<Environment>, EnvironmentError> {
        supervision::batch(&self.pool, installation_id, limit).await
    }
    async fn begin_provisioning(
        &self,
        id: Uuid,
        generation: i64,
    ) -> Result<bool, EnvironmentError> {
        let result = sqlx::query("UPDATE factory_identity.environments SET provisioning_state='volume_creating',updated_at=clock_timestamp() WHERE id=$1 AND generation=$2 AND desired_state='running' AND provisioning_state='unprovisioned'")
            .bind(id).bind(generation).execute(&self.pool).await.map_err(unavailable)?;
        Ok(result.rows_affected() == 1)
    }
    async fn record_observation(
        &self,
        id: Uuid,
        generation: i64,
        observation: EnvironmentObservation,
        retry_seconds: u32,
    ) -> Result<bool, EnvironmentError> {
        let result = sqlx::query("UPDATE factory_identity.environments SET observed_state=$3,error_code=$4,observed_at=clock_timestamp(),next_reconcile_at=clock_timestamp()+make_interval(secs => $5),updated_at=clock_timestamp(),provisioning_state=CASE WHEN $6 AND provisioning_state='volume_creating' THEN 'volume_ready' ELSE provisioning_state END WHERE id=$1 AND generation=$2")
            .bind(id).bind(generation).bind(mapping::observed(observation.state))
            .bind(observation.error.map(EnvironmentError::code)).bind(f64::from(retry_seconds))
            .bind(observation.volume_confirmed)
            .execute(&self.pool).await.map_err(unavailable)?;
        Ok(result.rows_affected() == 1)
    }
}
