use super::*;
use crate::connection::test_support::{finish_replacement, stalled_pool};

#[tokio::test]
async fn stalled_session_lookup_is_bounded_and_cancellation_releases_the_pool() {
    for cancel in [false, true] {
        let (pool, ready, peer) = stalled_pool(b"FROM factory_identity.sessions", false).await;
        let store = PgIdentityStore { pool: pool.clone() };
        let operation =
            tokio::spawn(async move { store.active_session(CredentialHash([7; 32])).await });
        tokio::time::timeout(Duration::from_secs(2), ready)
            .await
            .expect("session lookup must send its query")
            .unwrap();
        if cancel {
            operation.abort();
            assert!(operation.await.is_err_and(|error| error.is_cancelled()));
        } else {
            let result = tokio::time::timeout(Duration::from_secs(12), operation)
                .await
                .expect("silent PostgreSQL must not hold session lookup indefinitely")
                .unwrap();
            assert!(matches!(result, Err(IdentityError::Unavailable)));
        }
        finish_replacement(pool, peer).await;
    }
}
