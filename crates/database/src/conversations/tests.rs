use super::*;
use crate::connection::test_support::{finish_replacement, stalled_pool};
use factory_core::identity::CredentialHash;
use std::time::Duration;

#[tokio::test]
async fn cancelling_a_stalled_chat_transaction_releases_the_pool_connection() {
    let (pool, ready, peer) =
        stalled_pool(b"SELECT status FROM factory_identity.users", true).await;
    let store = PgConversationStore { pool: pool.clone() };
    let operation = tokio::spawn(async move {
        store
            .has_open_runs(EnvironmentActor {
                user_id: Uuid::new_v4(),
                session_hash: CredentialHash([7; 32]),
            })
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), ready)
        .await
        .expect("the chat transaction must reach authorization")
        .unwrap();
    operation.abort();
    assert!(operation.await.is_err_and(|error| error.is_cancelled()));
    finish_replacement(pool, peer).await;
}

#[tokio::test]
async fn cancelling_a_stalled_probe_settlement_releases_the_pool_connection() {
    let (pool, ready, peer) =
        stalled_pool(b"UPDATE factory_identity.model_probe_leases", false).await;
    let store = PgConversationStore { pool: pool.clone() };
    let operation = tokio::spawn(async move {
        store
            .settle_model_probe_launch(Uuid::new_v4(), Uuid::new_v4(), 1)
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), ready)
        .await
        .expect("probe settlement must reach its update")
        .unwrap();
    operation.abort();
    assert!(operation.await.is_err_and(|error| error.is_cancelled()));
    finish_replacement(pool, peer).await;
}
