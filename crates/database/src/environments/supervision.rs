use super::*;
use sqlx::{Connection, PgConnection, postgres::PgConnectOptions};
use std::time::Duration;

const OWNERSHIP_TIMEOUT: Duration = Duration::from_secs(5);

pub struct PgEnvironmentSupervisor {
    connection: Option<PgConnection>,
}

impl PgEnvironmentSupervisor {
    pub(crate) async fn acquire(
        options: &PgConnectOptions,
        installation: Uuid,
    ) -> Result<Self, EnvironmentError> {
        // The lock query shares the connection deadline. If acquisition times out
        // or is cancelled, its owned connection drops and cannot retain the lock.
        tokio::time::timeout(OWNERSHIP_TIMEOUT, async {
            let mut connection = PgConnection::connect_with(options)
                .await
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
        })
        .await
        .map_err(unavailable)?
    }

    pub async fn close(mut self) -> Result<(), EnvironmentError> {
        if let Some(connection) = self.connection.take() {
            tokio::time::timeout(OWNERSHIP_TIMEOUT, connection.close())
                .await
                .map_err(unavailable)?
                .map_err(unavailable)?;
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
    with_connection(pool, || EnvironmentError::Unavailable, |connection| {
        Box::pin(async move {
            let users: Vec<Uuid> = sqlx::query_scalar("SELECT user_id FROM factory_identity.environments e WHERE installation_id=$1 AND next_reconcile_at<=clock_timestamp() ORDER BY next_reconcile_at,id LIMIT $2")
                .bind(installation).bind(i64::from(limit)).fetch_all(&mut *connection).await.map_err(unavailable)?;
            let mut records = Vec::with_capacity(users.len());
            for user_id in users {
                let mut tx = connection.begin().await.map_err(unavailable)?;
                let status: String =
                    sqlx::query_scalar("SELECT status FROM factory_identity.users WHERE id=$1 FOR UPDATE")
                        .bind(user_id)
                        .fetch_one(&mut *tx)
                        .await
                        .map_err(unavailable)?;
                sqlx::query("UPDATE factory_identity.environments SET desired_state='stopped',generation=generation+1,next_reconcile_at=clock_timestamp(),updated_at=clock_timestamp() WHERE user_id=$1 AND desired_state='running' AND $2<>'active'")
                    .bind(user_id).bind(status).execute(&mut *tx).await.map_err(unavailable)?;
                let row =
                    sqlx::query("SELECT * FROM factory_identity.environments WHERE user_id=$1 FOR UPDATE")
                        .bind(user_id)
                        .fetch_one(&mut *tx)
                        .await
                        .map_err(unavailable)?;
                let env = mapping::environment(&row)?;
                records.push(env);
                tx.commit().await.map_err(unavailable)?;
            }
            Ok(records)
        })
    }).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connection::test_support::{accept, options, wait_for_disconnect, wait_for_query};
    use tokio::{net::TcpListener, sync::oneshot, task::JoinHandle};

    async fn stalled_acquisition() -> (
        JoinHandle<Result<PgEnvironmentSupervisor, EnvironmentError>>,
        JoinHandle<()>,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let options = options(&listener);
        let (reached, ready) = oneshot::channel();
        let peer = tokio::spawn(async move {
            let mut socket = accept(&listener).await;
            wait_for_query(&mut socket, b"pg_try_advisory_lock").await;
            reached.send(()).unwrap();
            // Cancellation must close the dedicated socket, releasing any server lock.
            wait_for_disconnect(&mut socket).await;
        });
        let acquisition = tokio::spawn(async move {
            PgEnvironmentSupervisor::acquire(&options, Uuid::new_v4()).await
        });
        tokio::time::timeout(Duration::from_secs(2), ready)
            .await
            .expect("lock query must be sent")
            .unwrap();
        (acquisition, peer)
    }

    #[tokio::test]
    async fn acquisition_deadline_includes_the_lock_query_and_drops_the_socket() {
        let (acquisition, peer) = stalled_acquisition().await;
        let result = tokio::time::timeout(Duration::from_secs(7), acquisition)
            .await
            .expect("a stalled lock query must not block recovery indefinitely")
            .unwrap();
        assert!(matches!(result, Err(EnvironmentError::Unavailable)));
        tokio::time::timeout(Duration::from_secs(1), peer)
            .await
            .expect("timed out acquisition must close its connection")
            .unwrap();
    }

    #[tokio::test]
    async fn cancelling_acquisition_drops_the_socket() {
        let (acquisition, peer) = stalled_acquisition().await;
        acquisition.abort();
        assert!(acquisition.await.is_err_and(|error| error.is_cancelled()));
        tokio::time::timeout(Duration::from_secs(1), peer)
            .await
            .expect("cancelled acquisition must close its connection")
            .unwrap();
    }
}
