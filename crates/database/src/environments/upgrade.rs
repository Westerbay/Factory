use super::*;
use sqlx::{Postgres, Row, Transaction, postgres::PgRow};

fn intent(row: &PgRow) -> Result<EnvironmentUpgradeIntent, EnvironmentError> {
    Ok(EnvironmentUpgradeIntent {
        operation_id: row.try_get("operation_id").map_err(unavailable)?,
        environment_id: row.try_get("environment_id").map_err(unavailable)?,
        generation: row.try_get("generation").map_err(unavailable)?,
        old_container_id: row.try_get("old_container_id").map_err(unavailable)?,
        target_image_id: row.try_get("target_image_id").map_err(unavailable)?,
        target_policy_version: row.try_get("target_policy_version").map_err(unavailable)?,
    })
}

async fn locked_environment(
    tx: &mut Transaction<'_, Postgres>,
    user: Uuid,
) -> Result<Environment, EnvironmentError> {
    // Match admission, suspension, Start and Stop: always lock user before environment.
    let exists = sqlx::query("SELECT id FROM factory_identity.users WHERE id=$1 FOR UPDATE")
        .bind(user)
        .fetch_optional(&mut **tx)
        .await
        .map_err(unavailable)?;
    if exists.is_none() {
        return Err(EnvironmentError::Conflict);
    }
    let row =
        sqlx::query("SELECT * FROM factory_identity.environments WHERE user_id=$1 FOR UPDATE")
            .bind(user)
            .fetch_optional(&mut **tx)
            .await
            .map_err(unavailable)?
            .ok_or(EnvironmentError::Conflict)?;
    mapping::environment(&row)
}

async fn pending(
    tx: &mut Transaction<'_, Postgres>,
    environment_id: Uuid,
) -> Result<Option<EnvironmentUpgradeIntent>, EnvironmentError> {
    sqlx::query("SELECT * FROM factory_identity.environment_upgrades WHERE environment_id=$1")
        .bind(environment_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(unavailable)?
        .as_ref()
        .map(intent)
        .transpose()
}

fn unchanged(a: &Environment, b: &Environment) -> bool {
    a.id == b.id
        && a.user_id == b.user_id
        && a.installation_id == b.installation_id
        && a.generation == b.generation
        && a.desired_state == b.desired_state
        && a.provisioning_state == b.provisioning_state
        && a.volume_name == b.volume_name
        && a.container_name == b.container_name
        && a.image_id == b.image_id
        && a.policy_version == b.policy_version
}

#[async_trait::async_trait]
impl EnvironmentUpgradeStore for PgEnvironmentStore {
    async fn upgrade_snapshot(
        &self,
        user_id: Uuid,
        installation_id: Uuid,
    ) -> Result<EnvironmentUpgradeSnapshot, EnvironmentError> {
        let mut tx = self.pool.begin().await.map_err(unavailable)?;
        let environment = locked_environment(&mut tx, user_id).await?;
        if environment.installation_id != installation_id {
            return Err(EnvironmentError::OwnershipConflict);
        }
        let pending = pending(&mut tx, environment.id).await?;
        tx.commit().await.map_err(unavailable)?;
        Ok(EnvironmentUpgradeSnapshot {
            environment,
            pending,
        })
    }

    async fn begin_upgrade(
        &self,
        expected: &Environment,
        old_container_id: &str,
        target: &EnvironmentStart,
    ) -> Result<EnvironmentUpgradeIntent, EnvironmentError> {
        target.validate()?;
        if old_container_id.len() != 64
            || !old_container_id
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        {
            return Err(EnvironmentError::InvalidConfiguration);
        }
        let mut tx = self.pool.begin().await.map_err(unavailable)?;
        let env = locked_environment(&mut tx, expected.user_id).await?;
        if !unchanged(expected, &env)
            || env.desired_state != DesiredState::Stopped
            || env.provisioning_state != ProvisioningState::VolumeReady
            || target.installation_id != env.installation_id
        {
            return Err(EnvironmentError::Conflict);
        }
        if let Some(intent) = pending(&mut tx, env.id).await? {
            if intent.generation != env.generation
                || intent.old_container_id != old_container_id
                || intent.target_image_id != target.image_id
                || intent.target_policy_version != target.policy_version
            {
                return Err(EnvironmentError::Conflict);
            }
            tx.commit().await.map_err(unavailable)?;
            return Ok(intent);
        }
        if env.image_id == target.image_id && env.policy_version == target.policy_version {
            return Err(EnvironmentError::Conflict);
        }
        let generation = env
            .generation
            .checked_add(1)
            .ok_or(EnvironmentError::Conflict)?;
        let operation_id = Uuid::new_v4();
        sqlx::query("UPDATE factory_identity.environments SET generation=$2,updated_at=clock_timestamp() WHERE id=$1")
            .bind(env.id).bind(generation).execute(&mut *tx).await.map_err(unavailable)?;
        let row = sqlx::query("INSERT INTO factory_identity.environment_upgrades(environment_id,operation_id,generation,old_container_id,target_image_id,target_policy_version) VALUES($1,$2,$3,$4,$5,$6) RETURNING *")
            .bind(env.id).bind(operation_id).bind(generation).bind(old_container_id).bind(&target.image_id).bind(&target.policy_version)
            .fetch_one(&mut *tx).await.map_err(unavailable)?;
        let intent = intent(&row)?;
        tx.commit().await.map_err(unavailable)?;
        Ok(intent)
    }

    async fn finish_upgrade(
        &self,
        expected: &EnvironmentUpgradeIntent,
    ) -> Result<(), EnvironmentError> {
        let mut tx = self.pool.begin().await.map_err(unavailable)?;
        let user: Uuid =
            sqlx::query_scalar("SELECT user_id FROM factory_identity.environments WHERE id=$1")
                .bind(expected.environment_id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(unavailable)?
                .ok_or(EnvironmentError::Conflict)?;
        let env = locked_environment(&mut tx, user).await?;
        if env.id != expected.environment_id
            || env.generation != expected.generation
            || env.desired_state != DesiredState::Stopped
            || env.provisioning_state != ProvisioningState::VolumeReady
            || pending(&mut tx, env.id).await?.as_ref() != Some(expected)
        {
            return Err(EnvironmentError::Conflict);
        }
        sqlx::query("UPDATE factory_identity.environments SET image_id=$2,policy_version=$3,observed_state='not_created',error_code=NULL,observed_at=NULL,next_reconcile_at=clock_timestamp(),updated_at=clock_timestamp() WHERE id=$1")
            .bind(env.id).bind(&expected.target_image_id).bind(&expected.target_policy_version).execute(&mut *tx).await.map_err(unavailable)?;
        sqlx::query("DELETE FROM factory_identity.environment_upgrades WHERE environment_id=$1")
            .bind(env.id)
            .execute(&mut *tx)
            .await
            .map_err(unavailable)?;
        tx.commit().await.map_err(unavailable)?;
        Ok(())
    }
}
