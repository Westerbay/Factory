use super::*;
use chrono::Utc;
use factory_core::environments::{DesiredState, ObservedState, ProvisioningState};
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::{Notify, oneshot};

fn environment() -> Environment {
    Environment {
        id: Uuid::new_v4(),
        user_id: Uuid::new_v4(),
        installation_id: Uuid::new_v4(),
        desired_state: DesiredState::Running,
        observed_state: ObservedState::Running,
        generation: 1,
        provisioning_state: ProvisioningState::VolumeReady,
        volume_name: "volume".into(),
        container_name: "container".into(),
        image_id: "image".into(),
        policy_version: "policy".into(),
        observed_at: Some(Utc::now()),
        error_code: None,
        next_reconcile_at: Utc::now(),
    }
}

const SESSION: CredentialHash = CredentialHash([1; 32]);

fn discovered(models: Result<Vec<ExecutionModel>, ConversationError>) -> ModelProbeResult {
    ModelProbeResult {
        initiator: SESSION,
        models,
    }
}

fn models() -> Vec<ExecutionModel> {
    vec![ExecutionModel {
        id: "model".into(),
        name: "Model".into(),
    }]
}

#[tokio::test]
async fn concurrent_readers_share_one_discovery_and_its_result() {
    let probes = ModelProbes::default();
    let env = environment();
    let calls = AtomicUsize::new(0);
    let joined = Notify::new();
    let (first, second) = tokio::join!(
        async {
            let probe = probes.for_environment(&env).unwrap();
            probe
                .result
                .get_or_init(|| async {
                    calls.fetch_add(1, Ordering::SeqCst);
                    joined.notified().await;
                    discovered(Ok(models()))
                })
                .await
                .for_session(SESSION)
        },
        async {
            let probe = probes.for_environment(&env).unwrap();
            joined.notify_one();
            probe
                .result
                .get_or_init(|| async {
                    calls.fetch_add(1, Ordering::SeqCst);
                    discovered(Err(ConversationError::Conflict))
                })
                .await
                .for_session(SESSION)
        }
    );
    assert_eq!(first, Ok(models()));
    assert_eq!(second, first);
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    // There is no stale server cache after the last reader releases its result.
    let next = probes.for_environment(&env).unwrap();
    assert!(next.result.get().is_none());
}

#[tokio::test]
async fn failed_discovery_is_shared_but_a_later_request_can_retry() {
    let probes = ModelProbes::default();
    let env = environment();
    let joined = Notify::new();
    let (first, second) = tokio::join!(
        async {
            let probe = probes.for_environment(&env).unwrap();
            probe
                .result
                .get_or_init(|| async {
                    joined.notified().await;
                    discovered(Err(ConversationError::Unavailable))
                })
                .await
                .for_session(SESSION)
        },
        async {
            let probe = probes.for_environment(&env).unwrap();
            joined.notify_one();
            probe
                .result
                .get_or_init(|| async { discovered(Ok(models())) })
                .await
                .for_session(SESSION)
        }
    );
    assert_eq!(first, Err(ConversationError::Unavailable));
    assert_eq!(second, first);
    let next = probes.for_environment(&env).unwrap();
    assert_eq!(
        next.result
            .get_or_init(|| async { discovered(Ok(models())) })
            .await
            .for_session(SESSION),
        Ok(models())
    );
}

#[tokio::test]
async fn dropping_a_waiter_does_not_cancel_or_replace_the_running_discovery() {
    let probes = ModelProbes::default();
    let env = environment();
    let first = probes.for_environment(&env).unwrap();
    let (started, ready) = oneshot::channel();
    let (finish, completed) = oneshot::channel();
    let task = tokio::spawn(async move {
        first
            .result
            .get_or_init(|| async {
                started.send(()).unwrap();
                completed.await.unwrap();
                discovered(Ok(models()))
            })
            .await
            .for_session(SESSION)
    });
    ready.await.unwrap();
    {
        let waiting = probes.for_environment(&env).unwrap();
        let read = waiting
            .result
            .get_or_init(|| async { panic!("a waiting reader must not start another discovery") });
        tokio::pin!(read);
        tokio::select! {
            biased;
            _ = &mut read => panic!("discovery is still pending"),
            () = std::future::ready(()) => {}
        }
    }
    let later = probes.for_environment(&env).unwrap();
    finish.send(()).unwrap();
    assert_eq!(task.await.unwrap(), Ok(models()));
    assert_eq!(
        later.result.get().unwrap().for_session(SESSION),
        Ok(models())
    );
}

#[tokio::test]
async fn a_revoked_initiator_stays_unauthorized_without_revoking_an_independent_follower() {
    let probes = ModelProbes::default();
    let env = environment();
    let joined = Notify::new();
    let other_session = CredentialHash([2; 32]);
    let (initiator, follower) = tokio::join!(
        async {
            let probe = probes.for_environment(&env).unwrap();
            probe
                .result
                .get_or_init(|| async {
                    joined.notified().await;
                    discovered(Err(ConversationError::Unauthorized))
                })
                .await
                .for_session(SESSION)
        },
        async {
            let probe = probes.for_environment(&env).unwrap();
            joined.notify_one();
            probe
                .result
                .get_or_init(|| async { panic!("the follower must share the initial probe") })
                .await
                .for_session(other_session)
        }
    );
    assert_eq!(initiator, Err(ConversationError::Unauthorized));
    // The coordinator independently reauthorizes the follower before this result.
    // Another session's error is neither permission nor revocation for this caller.
    assert_eq!(follower, Err(ConversationError::Conflict));
}

#[test]
fn users_and_environment_revisions_never_share_a_result() {
    let probes = ModelProbes::default();
    let env = environment();
    let first = probes.for_environment(&env).unwrap();
    let other = probes.for_environment(&environment()).unwrap();
    assert!(!Arc::ptr_eq(&first, &other));
    for changed in [
        Environment {
            id: Uuid::new_v4(),
            ..env.clone()
        },
        Environment {
            generation: 2,
            ..env.clone()
        },
        Environment {
            image_id: "new-image".into(),
            ..env.clone()
        },
        Environment {
            policy_version: "new-policy".into(),
            ..env.clone()
        },
    ] {
        assert!(matches!(
            probes.for_environment(&changed),
            Err(ConversationError::Conflict)
        ));
    }
    drop(first);
    assert!(
        probes
            .for_environment(&Environment {
                generation: 2,
                ..env
            })
            .is_ok()
    );
}

#[test]
fn inflight_users_are_bounded_and_finished_entries_are_reclaimed() {
    let probes = ModelProbes::default();
    let held = (0..256)
        .map(|_| probes.for_environment(&environment()).unwrap())
        .collect::<Vec<_>>();
    assert!(matches!(
        probes.for_environment(&environment()),
        Err(ConversationError::Unavailable)
    ));
    drop(held);
    let next = probes.for_environment(&environment()).unwrap();
    assert_eq!(probes.0.lock().unwrap().len(), 1);
    assert!(next.result.get().is_none());
}
