use super::*;
use crate::environments::{EnvironmentAvailability, EnvironmentReconciler};
use async_trait::async_trait;
use factory_core::environments::*;
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
struct Runtime {
    attached: AtomicUsize,
    detached: Mutex<Vec<String>>,
    writes: Arc<Mutex<Vec<Vec<u8>>>>,
    fail_detach: AtomicBool,
}
struct Input(Arc<Mutex<Vec<Vec<u8>>>>);
struct Output;
#[async_trait]
impl TerminalInput for Input {
    async fn write(&mut self, bytes: &[u8]) -> Result<(), TerminalError> {
        self.0.lock().unwrap().push(bytes.to_vec());
        Ok(())
    }
}
#[async_trait]
impl TerminalOutput for Output {
    async fn read(&mut self) -> Result<Option<Vec<u8>>, TerminalError> {
        Ok(None)
    }
}
#[async_trait]
impl TerminalRuntime for Runtime {
    async fn attach(&self, _: &Environment) -> Result<TerminalProcess, TerminalError> {
        Ok(TerminalProcess {
            handle: self.attached.fetch_add(1, Ordering::SeqCst).to_string(),
            input: Box::new(Input(self.writes.clone())),
            output: Box::new(Output),
        })
    }
    async fn resize(&self, _: &str, _: TerminalSize) -> Result<(), TerminalError> {
        Ok(())
    }
    async fn detach(&self, _: &Environment, handle: &str) -> Result<(), TerminalError> {
        if self.fail_detach.load(Ordering::SeqCst) {
            return Err(TerminalError::CleanupUncertain);
        }
        self.detached.lock().unwrap().push(handle.to_owned());
        Ok(())
    }
}

struct Fixture {
    service: TerminalService,
    store: Arc<Store>,
    runtime: Arc<Runtime>,
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
        let runtime = Arc::new(Runtime::default());
        Self {
            service: TerminalService::new(environments, runtime.clone()),
            store,
            runtime,
            actor,
        }
    }
    async fn issue(&self, hash: u8, take_over: bool) -> Result<(), TerminalError> {
        self.service
            .issue(
                self.actor,
                CredentialHash([hash; 32]),
                take_over,
                Utc::now() + ChronoDuration::minutes(1),
            )
            .await
            .map(|_| ())
    }
    async fn attach(&self, hash: u8) -> Result<TerminalAttachment, TerminalError> {
        self.service
            .attach(self.actor, CredentialHash([hash; 32]))
            .await
    }
}

#[tokio::test]
async fn tickets_are_session_bound_consumed_once_and_expire() {
    let fixture = Fixture::new().await;
    fixture.issue(10, false).await.unwrap();
    let other = EnvironmentActor {
        session_hash: CredentialHash([2; 32]),
        ..fixture.actor
    };
    assert!(matches!(
        fixture
            .service
            .attach(other, CredentialHash([10; 32]))
            .await,
        Err(TerminalError::InvalidTicket)
    ));
    assert!(matches!(
        fixture.attach(10).await,
        Err(TerminalError::InvalidTicket)
    ));
    fixture.issue(11, false).await.unwrap();
    fixture
        .service
        .tickets
        .lock()
        .unwrap()
        .get_mut(&CredentialHash([11; 32]))
        .unwrap()
        .expires_at = Utc::now() - ChronoDuration::seconds(1);
    assert!(matches!(
        fixture.attach(11).await,
        Err(TerminalError::InvalidTicket)
    ));
    assert_eq!(fixture.runtime.attached.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn changed_generation_or_stopped_intent_never_reaches_exec() {
    for stopped in [true, false] {
        let fixture = Fixture::new().await;
        fixture.issue(10, false).await.unwrap();
        {
            let mut row = fixture.store.row.lock().unwrap();
            if stopped {
                row.desired_state = DesiredState::Stopped;
            } else {
                row.generation += 1;
            }
        }
        assert!(matches!(
            fixture.attach(10).await,
            Err(TerminalError::Conflict)
        ));
        assert_eq!(fixture.runtime.attached.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn takeover_invalidates_old_writer_and_stale_cleanup_cannot_detach_new_writer() {
    let fixture = Fixture::new().await;
    fixture.issue(10, false).await.unwrap();
    let mut first = fixture.attach(10).await.unwrap();
    assert_eq!(fixture.issue(11, false).await, Err(TerminalError::Conflict));
    fixture.issue(12, true).await.unwrap();
    let mut second = fixture.attach(12).await.unwrap();
    assert!(*first.replaced.borrow());
    assert_eq!(
        fixture.service.write(&mut first, b"old").await,
        Err(TerminalError::Conflict)
    );
    fixture.service.finish(first).await.unwrap();
    assert_eq!(*fixture.runtime.detached.lock().unwrap(), vec!["0"]);
    fixture.service.write(&mut second, b"new").await.unwrap();
    fixture.service.finish(second).await.unwrap();
    assert_eq!(*fixture.runtime.detached.lock().unwrap(), vec!["0", "1"]);
    assert_eq!(
        *fixture.runtime.writes.lock().unwrap(),
        vec![b"new".to_vec()]
    );
}

#[tokio::test]
async fn cleanup_uncertainty_is_retained_until_explicit_recovery() {
    let fixture = Fixture::new().await;
    fixture.issue(10, false).await.unwrap();
    let first = fixture.attach(10).await.unwrap();
    fixture.runtime.fail_detach.store(true, Ordering::SeqCst);
    assert_eq!(
        fixture.service.finish(first).await,
        Err(TerminalError::CleanupUncertain)
    );
    assert_eq!(fixture.issue(11, false).await, Err(TerminalError::Conflict));
    fixture.runtime.fail_detach.store(false, Ordering::SeqCst);
    fixture.issue(12, true).await.unwrap();
    let second = fixture.attach(12).await.unwrap();
    fixture.service.finish(second).await.unwrap();
}

#[tokio::test]
async fn inputs_are_reauthorized_and_bounded_before_writing() {
    let fixture = Fixture::new().await;
    fixture.issue(10, false).await.unwrap();
    let mut attachment = fixture.attach(10).await.unwrap();
    assert_eq!(
        fixture
            .service
            .write(&mut attachment, &vec![b'x'; MAX_INPUT_BYTES + 1])
            .await,
        Err(TerminalError::InvalidInput)
    );
    fixture.store.revoked.store(true, Ordering::SeqCst);
    assert_eq!(
        fixture
            .service
            .write(&mut attachment, b"touch forbidden")
            .await,
        Err(TerminalError::Unauthorized)
    );
    assert!(fixture.runtime.writes.lock().unwrap().is_empty());
    fixture.service.finish(attachment).await.unwrap();
}
