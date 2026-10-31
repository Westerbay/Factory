use super::*;
use crate::environments::{EnvironmentAvailability, EnvironmentReconciler};
use async_trait::async_trait;
use factory_core::environments::*;
use factory_core::identity::CredentialHash;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

struct Store {
    row: Mutex<Environment>,
    revoked: AtomicBool,
}

#[async_trait]
impl EnvironmentStore for Store {
    async fn get(&self, actor: EnvironmentActor) -> Result<Option<Environment>, EnvironmentError> {
        let row = self.row.lock().unwrap();
        if self.revoked.load(Ordering::SeqCst) || row.user_id != actor.user_id {
            return Err(EnvironmentError::Unauthorized);
        }
        Ok(Some(row.clone()))
    }
    async fn initialize_if_absent(
        &self,
        _: EnvironmentActor,
        _: EnvironmentStart,
    ) -> Result<Environment, EnvironmentError> {
        unreachable!()
    }
    async fn start(
        &self,
        _: EnvironmentActor,
        _: EnvironmentStart,
    ) -> Result<Environment, EnvironmentError> {
        unreachable!()
    }
    async fn stop(&self, _: EnvironmentActor) -> Result<Option<Environment>, EnvironmentError> {
        unreachable!()
    }
    async fn reconcile_batch(&self, _: Uuid, _: u32) -> Result<Vec<Environment>, EnvironmentError> {
        Ok(vec![])
    }
    async fn begin_provisioning(&self, _: Uuid, _: i64) -> Result<bool, EnvironmentError> {
        unreachable!()
    }
    async fn record_observation(
        &self,
        _: Uuid,
        _: i64,
        _: EnvironmentObservation,
        _: u32,
    ) -> Result<bool, EnvironmentError> {
        unreachable!()
    }
}

struct NoLifecycle;
#[async_trait]
impl EnvironmentRuntime for NoLifecycle {
    async fn inspect(&self, _: &Environment) -> Result<RuntimeInspection, EnvironmentError> {
        unreachable!()
    }
    async fn create_volume(&self, _: &Environment) -> Result<(), EnvironmentError> {
        unreachable!()
    }
    async fn start(&self, _: &Environment) -> Result<(), EnvironmentError> {
        unreachable!()
    }
    async fn stop(&self, _: &Environment) -> Result<(), EnvironmentError> {
        unreachable!()
    }
}
struct Guard;
#[async_trait]
impl EnvironmentSupervisor for Guard {
    async fn check(&mut self) -> Result<(), EnvironmentError> {
        Ok(())
    }
}

#[derive(Default)]
struct Port {
    state: Mutex<Option<ConnectionSnapshot>>,
    cancels: AtomicUsize,
    calls: AtomicUsize,
    after_cancel: Mutex<Option<Box<dyn Fn() + Send + Sync>>>,
}
#[async_trait]
impl AgentConnectionPort for Port {
    async fn read(&self, _: &Environment) -> Result<ConnectionSnapshot, AgentConnectionError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let mut state = self.state.lock().unwrap();
        let snapshot = state
            .clone()
            .unwrap_or_else(|| ConnectionSnapshot::account(None, Utc::now()));
        if snapshot.status != ConnectionStatus::Connecting {
            *state = None;
        }
        Ok(snapshot)
    }
    async fn begin_login(
        &self,
        _: &Environment,
        operation_id: Uuid,
        expires_at: DateTime<Utc>,
    ) -> Result<ConnectionSnapshot, AgentConnectionError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let state = ConnectionSnapshot {
            status: ConnectionStatus::Connecting,
            account: None,
            error: None,
            observed_at: Utc::now(),
            challenge: Some(DeviceChallenge {
                operation_id,
                expires_at,
                verification_url: "https://auth.openai.com/codex/device".into(),
                user_code: "SYNTHETIC".into(),
            }),
        };
        *self.state.lock().unwrap() = Some(state.clone());
        Ok(state)
    }
    async fn cancel_login(
        &self,
        _: &Environment,
        operation_id: Uuid,
    ) -> Result<ConnectionSnapshot, AgentConnectionError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let mut state = self.state.lock().unwrap();
        assert_eq!(
            state
                .as_ref()
                .unwrap()
                .challenge
                .as_ref()
                .unwrap()
                .operation_id,
            operation_id
        );
        *state = None;
        self.cancels.fetch_add(1, Ordering::SeqCst);
        if let Some(callback) = self.after_cancel.lock().unwrap().take() {
            callback();
        }
        Ok(ConnectionSnapshot::failure(
            AgentConnectionError::Cancelled,
            Utc::now(),
        ))
    }
    async fn logout(&self, _: &Environment) -> Result<ConnectionSnapshot, AgentConnectionError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(ConnectionSnapshot::account(None, Utc::now()))
    }
}
struct Fixture {
    service: AgentConnectionService,
    store: Arc<Store>,
    port: Arc<Port>,
    actor: EnvironmentActor,
}
impl Fixture {
    async fn new() -> Self {
        let availability = EnvironmentAvailability::default();
        let actor = EnvironmentActor {
            user_id: Uuid::new_v4(),
            session_hash: CredentialHash([1; 32]),
        };
        let config = EnvironmentStart {
            installation_id: Uuid::new_v4(),
            image_id: "fixture-image".into(),
            policy_version: "3".into(),
        };
        let store = Arc::new(Store {
            revoked: AtomicBool::new(false),
            row: Mutex::new(Environment {
                id: Uuid::new_v4(),
                user_id: actor.user_id,
                installation_id: config.installation_id,
                desired_state: DesiredState::Running,
                observed_state: ObservedState::Running,
                generation: 1,
                provisioning_state: ProvisioningState::VolumeReady,
                volume_name: "fixture-home".into(),
                container_name: "fixture-container".into(),
                image_id: config.image_id.clone(),
                policy_version: config.policy_version.clone(),
                observed_at: Some(Utc::now()),
                error_code: None,
                next_reconcile_at: Utc::now(),
            }),
        });
        EnvironmentReconciler::new(
            store.clone(),
            Arc::new(NoLifecycle),
            Box::new(Guard),
            config.clone(),
            availability.clone(),
        )
        .unwrap()
        .next_batch()
        .await
        .unwrap();
        let environments =
            Arc::new(EnvironmentService::new(store.clone(), config, availability).unwrap());
        let port = Arc::new(Port::default());
        Self {
            service: AgentConnectionService::new(environments, port.clone()),
            store,
            port,
            actor,
        }
    }
    async fn begin(&self) -> Uuid {
        self.service
            .begin_login(self.actor, Utc::now() + Duration::minutes(5))
            .await
            .unwrap()
            .challenge
            .unwrap()
            .operation_id
    }
}

#[tokio::test]
async fn codes_are_session_private_and_stale_cancellation_cannot_cancel_a_new_login() {
    let f = Fixture::new().await;
    let id = f.begin().await;
    let other = EnvironmentActor {
        session_hash: CredentialHash([2; 32]),
        ..f.actor
    };
    assert!(matches!(
        f.service.read(other).await,
        Err(AgentConnectionError::Conflict)
    ));
    assert!(matches!(
        f.service.cancel_login(f.actor, Uuid::new_v4()).await,
        Err(AgentConnectionError::Conflict)
    ));
    assert_eq!(f.port.cancels.load(Ordering::SeqCst), 0);
    f.service.cancel_login(f.actor, id).await.unwrap();
    let new = f.begin().await;
    f.service.interrupt(f.actor, id).await.unwrap();
    assert_eq!(f.port.cancels.load(Ordering::SeqCst), 1);
    assert_eq!(
        f.service
            .read(f.actor)
            .await
            .unwrap()
            .challenge
            .unwrap()
            .operation_id,
        new
    );
}

#[tokio::test]
async fn revocation_denies_reads_but_interrupts_the_previously_authorized_operation() {
    let f = Fixture::new().await;
    let id = f.begin().await;
    f.store.revoked.store(true, Ordering::SeqCst);
    assert!(matches!(
        f.service.read(f.actor).await,
        Err(AgentConnectionError::Unauthorized)
    ));
    assert!(matches!(
        f.service.revalidate_pending(f.actor, id).await,
        Err(AgentConnectionError::Unauthorized)
    ));
    f.service.interrupt(f.actor, id).await.unwrap();
    assert_eq!(f.port.cancels.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn duplicate_start_expired_sessions_and_generation_changes_are_refused() {
    let f = Fixture::new().await;
    assert!(matches!(
        f.service
            .begin_login(f.actor, Utc::now() - Duration::seconds(1))
            .await,
        Err(AgentConnectionError::Unauthorized)
    ));
    let (a, b) = tokio::join!(
        f.service
            .begin_login(f.actor, Utc::now() + Duration::minutes(1)),
        f.service
            .begin_login(f.actor, Utc::now() + Duration::minutes(1))
    );
    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
    let id = a.or(b).unwrap().challenge.unwrap().operation_id;
    f.store.row.lock().unwrap().generation += 1;
    assert!(matches!(
        f.service.revalidate_pending(f.actor, id).await,
        Err(AgentConnectionError::Conflict)
    ));
    f.service.interrupt(f.actor, id).await.unwrap();
}

#[tokio::test]
async fn queued_operations_revalidate_revocation_and_stop_before_contacting_the_provider() {
    use std::{future::Future, pin::Pin, task::Poll};

    for revoked in [false, true] {
        for operation in ["read", "begin", "cancel", "logout", "revalidate"] {
            let f = Fixture::new().await;
            let id = if operation == "begin" {
                Uuid::new_v4()
            } else {
                f.begin().await
            };
            let calls_before = f.port.calls.load(Ordering::SeqCst);
            let slot = f.service.slot(f.actor.user_id).unwrap();
            let guard = slot.lock().await;
            let mut queued: Pin<Box<dyn Future<Output = Result<(), AgentConnectionError>>>> =
                Box::pin(async {
                    match operation {
                        "read" => f.service.read(f.actor).await.map(|_| ()),
                        "begin" => f
                            .service
                            .begin_login(f.actor, Utc::now() + Duration::minutes(1))
                            .await
                            .map(|_| ()),
                        "cancel" => f.service.cancel_login(f.actor, id).await.map(|_| ()),
                        "logout" => f.service.logout(f.actor).await.map(|_| ()),
                        "revalidate" => f.service.revalidate_pending(f.actor, id).await.map(|_| ()),
                        _ => unreachable!(),
                    }
                });
            // Poll once while the slot is held, proving the request is queued without sleeps.
            std::future::poll_fn(|cx| {
                assert!(queued.as_mut().poll(cx).is_pending());
                Poll::Ready(())
            })
            .await;
            let expected = if revoked {
                f.store.revoked.store(true, Ordering::SeqCst);
                AgentConnectionError::Unauthorized
            } else {
                f.store.row.lock().unwrap().desired_state = DesiredState::Stopped;
                AgentConnectionError::Conflict
            };
            drop(guard);
            assert_eq!(
                queued.await,
                Err(expected),
                "{operation}, revoked={revoked}"
            );
            assert_eq!(f.port.calls.load(Ordering::SeqCst), calls_before);
        }
    }
}

#[tokio::test]
async fn logout_revalidates_after_waiting_for_pending_login_cancellation() {
    for revoked in [false, true] {
        let f = Fixture::new().await;
        f.begin().await;
        let store = f.store.clone();
        *f.port.after_cancel.lock().unwrap() = Some(Box::new(move || {
            if revoked {
                store.revoked.store(true, Ordering::SeqCst);
            } else {
                store.row.lock().unwrap().desired_state = DesiredState::Stopped;
            }
        }));
        let result = f.service.logout(f.actor).await;
        assert!(matches!(
            result,
            Err(AgentConnectionError::Unauthorized | AgentConnectionError::Conflict)
        ));
        assert_eq!(f.port.cancels.load(Ordering::SeqCst), 1);
        assert_eq!(
            f.port.calls.load(Ordering::SeqCst),
            2,
            "only begin and cancel are authorized"
        );
    }
}

async fn monitor_failure(f: &Fixture) -> Uuid {
    let id = f.begin().await;
    *f.port.state.lock().unwrap() = Some(ConnectionSnapshot::failure(
        AgentConnectionError::CleanupUncertain,
        Utc::now(),
    ));
    assert!(!f.service.revalidate_pending(f.actor, id).await.unwrap());
    assert!(f.port.state.lock().unwrap().is_none());
    id
}

#[tokio::test]
async fn monitor_preserves_one_final_error_for_the_browser_then_refresh_reads_actual_account() {
    let f = Fixture::new().await;
    let id = monitor_failure(&f).await;
    let calls = f.port.calls.load(Ordering::SeqCst);
    assert!(!f.service.revalidate_pending(f.actor, id).await.unwrap());
    assert_eq!(f.port.calls.load(Ordering::SeqCst), calls);
    let error = f.service.read(f.actor).await.unwrap();
    assert_eq!(error.error, Some(AgentConnectionError::CleanupUncertain));
    assert!(error.challenge.is_none());
    assert_eq!(f.port.calls.load(Ordering::SeqCst), calls);
    let refreshed = f.service.read(f.actor).await.unwrap();
    assert_eq!(refreshed.status, ConnectionStatus::Disconnected);
    assert_eq!(f.port.calls.load(Ordering::SeqCst), calls + 1);
}

#[tokio::test]
async fn completed_errors_do_not_cross_sessions_generations_or_the_original_deadline() {
    for change in ["session", "generation", "expiry"] {
        let f = Fixture::new().await;
        monitor_failure(&f).await;
        let mut actor = f.actor;
        match change {
            "session" => actor.session_hash = CredentialHash([8; 32]),
            "generation" => f.store.row.lock().unwrap().generation += 2,
            "expiry" => {
                let slot = f.service.slot(actor.user_id).unwrap();
                slot.lock().await.as_mut().unwrap().expires_at = Utc::now() - Duration::seconds(1);
            }
            _ => unreachable!(),
        }
        let calls = f.port.calls.load(Ordering::SeqCst);
        let fresh = f.service.read(actor).await.unwrap();
        assert_eq!(fresh.status, ConnectionStatus::Disconnected, "{change}");
        assert_eq!(f.port.calls.load(Ordering::SeqCst), calls + 1);
        assert!(fresh.error.is_none());
    }
}

#[tokio::test]
async fn new_intents_and_interrupts_do_not_cancel_a_finished_operation() {
    for action in ["begin", "logout", "interrupt", "cancel"] {
        let f = Fixture::new().await;
        let old = monitor_failure(&f).await;
        match action {
            "begin" => assert_ne!(f.begin().await, old),
            "logout" => assert_eq!(
                f.service.logout(f.actor).await.unwrap().status,
                ConnectionStatus::Disconnected
            ),
            "interrupt" => f.service.interrupt(f.actor, old).await.unwrap(),
            "cancel" => assert!(matches!(
                f.service.cancel_login(f.actor, old).await,
                Err(AgentConnectionError::Conflict)
            )),
            _ => unreachable!(),
        }
        assert_eq!(f.port.cancels.load(Ordering::SeqCst), 0, "{action}");
    }
}
