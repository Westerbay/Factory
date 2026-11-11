use super::*;

impl PgEnvironmentStore {
    pub async fn upgrade_inventory(
        &self,
        installation: Uuid,
    ) -> Result<Vec<Environment>, EnvironmentError> {
        with_connection(&self.pool, || EnvironmentError::Unavailable, |connection| {
            Box::pin(async move {
                sqlx::query("SELECT * FROM factory_identity.environments WHERE installation_id=$1 ORDER BY id")
                    .bind(installation)
                    .fetch_all(connection)
                    .await
                    .map_err(unavailable)?
                    .iter()
                    .map(mapping::environment)
                    .collect()
            })
        }).await
    }

    pub async fn upgrade_has_active_work(
        &self,
        installation: Uuid,
    ) -> Result<bool, EnvironmentError> {
        with_connection(&self.pool, || EnvironmentError::Unavailable, |connection| {
            Box::pin(async move {
                sqlx::query_scalar(
                    "SELECT EXISTS (
                        SELECT 1 FROM factory_identity.environments e
                        WHERE e.installation_id=$1 AND (
                            EXISTS (SELECT 1 FROM factory_identity.conversation_runs r WHERE r.environment_id=e.id AND r.status='active')
                            OR EXISTS (SELECT 1 FROM factory_identity.model_probe_leases p WHERE p.environment_id=e.id)
                            OR EXISTS (SELECT 1 FROM factory_identity.features f JOIN factory_identity.feature_attempts a ON a.feature_id=f.id WHERE f.environment_id=e.id AND a.reserved)
                        )
                    )",
                )
                .bind(installation)
                .fetch_one(connection)
                .await
                .map_err(unavailable)
            })
        }).await
    }

    /// The caller holds the exclusive supervisor and has stopped the owned runtime.
    /// Preserve desired state and persistent volume identity across the transition.
    pub async fn complete_runtime_upgrade(
        &self,
        source: EnvironmentStart,
        target_image: String,
        environment_count: usize,
    ) -> Result<(), EnvironmentError> {
        with_connection(&self.pool, || EnvironmentError::Unavailable, |connection| {
            Box::pin(async move {
                let mut tx = connection.begin().await.map_err(unavailable)?;
                let result = sqlx::query(
                    "UPDATE factory_identity.environments
                     SET image_id=$4,
                         generation=generation+CASE WHEN image_id<>$4 OR observed_at IS NOT NULL THEN 1 ELSE 0 END,
                         observed_state='unavailable',observed_generation=NULL,observed_at=NULL,error_code=NULL,
                         next_reconcile_at=clock_timestamp(),updated_at=clock_timestamp()
                     WHERE installation_id=$1 AND policy_version=$2 AND image_id IN ($3,$4)",
                )
                .bind(source.installation_id)
                .bind(source.policy_version)
                .bind(source.image_id)
                .bind(target_image)
                .execute(&mut *tx)
                .await
                .map_err(unavailable)?;
                if result.rows_affected() != environment_count as u64 {
                    return Err(EnvironmentError::Conflict);
                }
                tx.commit().await.map_err(unavailable)
            })
        }).await
    }
}
