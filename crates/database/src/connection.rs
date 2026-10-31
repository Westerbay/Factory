use sqlx::{PgConnection, PgPool, Postgres, pool::PoolConnection};
use std::{future::Future, pin::Pin, time::Duration};
use tokio::time::{Instant, timeout_at};

#[cfg(test)]
pub(crate) mod test_support;

#[cfg(test)]
mod tests;

const OPERATION_TIMEOUT: Duration = Duration::from_secs(10);

pub(crate) type Operation<'connection, T, E> =
    Pin<Box<dyn Future<Output = Result<T, E>> + Send + 'connection>>;

struct OwnedConnection(Option<PoolConnection<Postgres>>);

impl Drop for OwnedConnection {
    fn drop(&mut self) {
        if let Some(connection) = self.0.take() {
            // Interrupted SQL must not enter SQLx's unbounded pool-return ping.
            // Detaching releases the permit, and dropping closes the socket.
            drop(connection.detach());
        }
    }
}

pub(crate) async fn with_connection<T: Send, E: Send>(
    pool: &PgPool,
    unavailable: impl Fn() -> E + Send + Sync,
    operation: impl for<'connection> FnOnce(
        &'connection mut PgConnection,
    ) -> Operation<'connection, T, E>
    + Send,
) -> Result<T, E> {
    let deadline = Instant::now() + OPERATION_TIMEOUT;
    let (result, mut owned) = timeout_at(deadline, async {
        let connection = pool.acquire().await.map_err(|_| unavailable())?;
        let mut owned = OwnedConnection(Some(connection));
        let result = operation(owned.0.as_mut().expect("connection is owned")).await;
        Ok::<_, E>((result, owned))
    })
    .await
    .map_err(|_| unavailable())??;

    if result.is_ok() {
        let mut connection = owned.0.take().expect("connection is owned");
        // This future owns the socket and permit. Timing out or cancelling it
        // discards both instead of leaving an unbounded background ping.
        let returning = connection.return_to_pool();
        drop(connection);
        let _ = timeout_at(deadline, returning).await;
        // A confirmed SQL/COMMIT result remains successful even if recycling
        // fails. It must not become a retryable error after the write committed.
    }
    result
}
