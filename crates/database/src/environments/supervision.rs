use super::*;
use sqlx::{Connection, PgConnection, postgres::PgConnectOptions};
use std::time::Duration;

pub struct PgEnvironmentSupervisor {
    connection: Option<PgConnection>,
}

impl PgEnvironmentSupervisor {
    pub(crate) async fn acquire(
        options: &PgConnectOptions,
        installation: Uuid,
    ) -> Result<Self, EnvironmentError> {
        let mut connection =
            tokio::time::timeout(Duration::from_secs(5), PgConnection::connect_with(options))
                .await
                .map_err(unavailable)?
                .map_err(unavailable)?;
        let acquired: bool =
            sqlx::query_scalar("SELECT pg_try_advisory_lock(hashtextextended($1, 61925))")
                .bind(installation.to_string())
                .fetch_one(&mut connection)
                .await
                .map_err(unavailable)?;
        if !acquired {
            connection.close().await.map_err(unavailable)?;
            return Err(EnvironmentError::Conflict);
        }
        Ok(Self {
            connection: Some(connection),
        })
    }

    pub async fn close(mut self) -> Result<(), EnvironmentError> {
        if let Some(connection) = self.connection.take() {
            connection.close().await.map_err(unavailable)?;
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl EnvironmentSupervisor for PgEnvironmentSupervisor {
    async fn check(&mut self) -> Result<(), EnvironmentError> {
        let connection = self
            .connection
            .as_mut()
            .ok_or(EnvironmentError::Unavailable)?;
        let check = tokio::time::timeout(
            Duration::from_secs(2),
            sqlx::query("SELECT 1").execute(connection),
        )
        .await;
        if matches!(check, Ok(Ok(_))) {
            return Ok(());
        }
        // Discard the dedicated connection permanently: never reconnect implicitly.
        self.connection.take();
        Err(EnvironmentError::Unavailable)
    }
}

pub(super) async fn batch(
    pool: &PgPool,
    installation: Uuid,
    limit: u32,
) -> Result<Vec<Environment>, EnvironmentError> {
    if !(1..=100).contains(&limit) {
        return Err(EnvironmentError::InvalidConfiguration);
    }
    let users: Vec<Uuid> = sqlx::query_scalar("SELECT user_id FROM factory_identity.environments WHERE installation_id=$1 AND (next_reconcile_at<=clock_timestamp() OR (desired_state='running' AND lease_expires_at<=clock_timestamp())) ORDER BY next_reconcile_at,id LIMIT $2")
        .bind(installation).bind(i64::from(limit)).fetch_all(pool).await.map_err(unavailable)?;
    let mut records = Vec::with_capacity(users.len());
    for user_id in users {
        let mut tx = pool.begin().await.map_err(unavailable)?;
        let status: String =
            sqlx::query_scalar("SELECT status FROM factory_identity.users WHERE id=$1 FOR UPDATE")
                .bind(user_id)
                .fetch_one(&mut *tx)
                .await
                .map_err(unavailable)?;
        sqlx::query("UPDATE factory_identity.environments SET desired_state='stopped',lease_expires_at=NULL,generation=generation+1,next_reconcile_at=clock_timestamp(),updated_at=clock_timestamp() WHERE user_id=$1 AND desired_state='running' AND ($2<>'active' OR lease_expires_at<=clock_timestamp())")
            .bind(user_id).bind(status).execute(&mut *tx).await.map_err(unavailable)?;
        let row =
            sqlx::query("SELECT * FROM factory_identity.environments WHERE user_id=$1 FOR UPDATE")
                .bind(user_id)
                .fetch_one(&mut *tx)
                .await
                .map_err(unavailable)?;
        records.push(mapping::environment(&row)?);
        tx.commit().await.map_err(unavailable)?;
    }
    Ok(records)
}
