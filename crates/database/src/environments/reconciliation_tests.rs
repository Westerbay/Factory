use super::*;
use crate::connection::test_support::{finish_replacement, stalled_pool};
use std::time::Duration;

#[derive(Clone, Copy)]
enum ReconciliationOperation {
    Batch,
    BeginProvisioning,
    RecordObservation,
}

impl ReconciliationOperation {
    async fn run(self, store: PgEnvironmentStore) -> Result<(), EnvironmentError> {
        let id = Uuid::new_v4();
        match self {
            Self::Batch => store.reconcile_batch(id, 1).await.map(|_| ()),
            Self::BeginProvisioning => store.begin_provisioning(id, 1).await.map(|_| ()),
            Self::RecordObservation => store
                .record_observation(
                    id,
                    1,
                    EnvironmentObservation {
                        state: ObservedState::Running,
                        error: None,
                        volume_confirmed: true,
                    },
                    1,
                )
                .await
                .map(|_| ()),
        }
    }

    async fn verify_interruption(self, cancel: bool) {
        let (query, transaction): (&[u8], bool) = match self {
            Self::Batch => (b"SELECT user_id", false),
            Self::BeginProvisioning => (b"UPDATE factory_identity.environments", false),
            Self::RecordObservation => (b"UPDATE factory_identity.environments", true),
        };
        let (pool, ready, peer) = stalled_pool(query, transaction).await;
        let operation = tokio::spawn(self.run(PgEnvironmentStore { pool: pool.clone() }));
        tokio::time::timeout(Duration::from_secs(2), ready)
            .await
            .expect("the operation must send its query")
            .unwrap();
        if cancel {
            operation.abort();
            assert!(operation.await.is_err_and(|error| error.is_cancelled()));
        } else {
            let result = tokio::time::timeout(Duration::from_secs(12), operation)
                .await
                .expect("silent PostgreSQL must not block reconciliation drain")
                .unwrap();
            assert_eq!(result, Err(EnvironmentError::Unavailable));
        }
        finish_replacement(pool, peer).await;
    }

    async fn verify(self) {
        self.verify_interruption(false).await;
        self.verify_interruption(true).await;
    }
}

#[tokio::test]
async fn stalled_batch_is_bounded_and_cancellation_releases_the_pool() {
    ReconciliationOperation::Batch.verify().await;
}

#[tokio::test]
async fn stalled_provisioning_is_bounded_and_cancellation_releases_the_pool() {
    ReconciliationOperation::BeginProvisioning.verify().await;
}

#[tokio::test]
async fn stalled_observation_is_bounded_and_cancellation_releases_the_pool() {
    ReconciliationOperation::RecordObservation.verify().await;
}
