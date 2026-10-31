mod mapping;
mod requests;
mod supervision;

#[cfg(test)]
mod reconciliation_tests;

use crate::connection::with_connection;
use factory_core::environments::*;
use sqlx::{Connection, PgPool};
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
        with_connection(
            &self.pool,
            || EnvironmentError::Unavailable,
            |connection| Box::pin(requests::get(connection, actor)),
        )
        .await
    }
    async fn initialize_if_absent(
        &self,
        actor: EnvironmentActor,
        request: EnvironmentStart,
    ) -> Result<Environment, EnvironmentError> {
        request.validate()?;
        with_connection(
            &self.pool,
            || EnvironmentError::Unavailable,
            |connection| Box::pin(requests::initialize_if_absent(connection, actor, request)),
        )
        .await
    }
    async fn start(
        &self,
        actor: EnvironmentActor,
        request: EnvironmentStart,
    ) -> Result<Environment, EnvironmentError> {
        request.validate()?;
        with_connection(
            &self.pool,
            || EnvironmentError::Unavailable,
            |connection| Box::pin(requests::start(connection, actor, request)),
        )
        .await
    }
    async fn stop(&self, actor: EnvironmentActor) -> Result<Option<Environment>, EnvironmentError> {
        with_connection(
            &self.pool,
            || EnvironmentError::Unavailable,
            |connection| Box::pin(requests::stop(connection, actor)),
        )
        .await
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
        with_connection(&self.pool, || EnvironmentError::Unavailable, |connection| {
            Box::pin(async move {
                let result = sqlx::query("UPDATE factory_identity.environments SET provisioning_state='volume_creating',updated_at=clock_timestamp() WHERE id=$1 AND generation=$2 AND desired_state='running' AND provisioning_state='unprovisioned'")
                    .bind(id).bind(generation).execute(connection).await.map_err(unavailable)?;
                Ok(result.rows_affected() == 1)
            })
        }).await
    }
    async fn record_observation(
        &self,
        id: Uuid,
        generation: i64,
        observation: EnvironmentObservation,
        retry_seconds: u32,
    ) -> Result<bool, EnvironmentError> {
        with_connection(&self.pool, || EnvironmentError::Unavailable, |connection| {
            Box::pin(async move {
                let mut tx = connection.begin().await.map_err(unavailable)?;
                let result = sqlx::query("UPDATE factory_identity.environments SET observed_state=$3,error_code=$4,observed_generation=$2,observed_at=clock_timestamp(),next_reconcile_at=clock_timestamp()+make_interval(secs => $5),updated_at=clock_timestamp(),provisioning_state=CASE WHEN $6 AND provisioning_state='volume_creating' THEN 'volume_ready' ELSE provisioning_state END WHERE id=$1 AND generation=$2")
                    .bind(id).bind(generation).bind(mapping::observed(observation.state))
                    .bind(observation.error.map(EnvironmentError::code)).bind(f64::from(retry_seconds))
                    .bind(observation.volume_confirmed)
                    .execute(&mut *tx).await.map_err(unavailable)?;
                if result.rows_affected() == 1
                    && observation.state == ObservedState::Stopped
                    && observation.error.is_none()
                {
                    // This method is called only after the exclusive supervisor's owned
                    // runtime inspection. A stopped observation of a newer generation
                    // proves that old container processes no longer run, even when the
                    // owner is suspended and cannot possess a session. Keep the row
                    // locked until deletion commits so a concurrent start cannot race it.
                    sqlx::query("DELETE FROM factory_identity.model_probe_leases p USING factory_identity.environments e WHERE e.id=$1 AND e.generation=$2 AND e.desired_state='stopped' AND e.observed_state='stopped' AND e.observed_generation=$2 AND e.error_code IS NULL AND p.environment_id=e.id AND p.user_id=e.user_id AND p.environment_generation < $2 AND p.launch_settled")
                        .bind(id).bind(generation).execute(&mut *tx).await.map_err(unavailable)?;
                }
                tx.commit().await.map_err(unavailable)?;
                Ok(result.rows_affected() == 1)
            })
        }).await
    }
}
