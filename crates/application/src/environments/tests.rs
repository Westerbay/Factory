use super::*;
use async_trait::async_trait;
use chrono::{Duration, Utc};
use factory_core::{environments::*, identity::CredentialHash};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use uuid::Uuid;

mod concurrency;

struct Store {
    row: Mutex<Environment>,
    valid_session: AtomicBool,
    writes: AtomicUsize,
    fail_record: AtomicBool,
}

impl Store {
    fn row(&self) -> Environment {
        self.row.lock().unwrap().clone()
    }
    fn authorize(&self, actor: EnvironmentActor) -> Result<(), EnvironmentError> {
        if self.valid_session.load(Ordering::SeqCst) && actor.user_id == self.row().user_id {
            Ok(())
        } else {
            Err(EnvironmentError::Unauthorized)
        }
    }
}

#[async_trait]
impl EnvironmentStore for Store {
    async fn get(&self, actor: EnvironmentActor) -> Result<Option<Environment>, EnvironmentError> {
        self.authorize(actor)?;
        Ok(Some(self.row()))
    }
    async fn start(
        &self,
        actor: EnvironmentActor,
        _: EnvironmentStart,
    ) -> Result<Environment, EnvironmentError> {
        self.authorize(actor)?;
        let mut row = self.row.lock().unwrap();
        if row.desired_state != DesiredState::Running {
            row.generation += 1;
        }
        row.desired_state = DesiredState::Running;
        row.capacity_reserved = true;
        Ok(row.clone())
    }
    async fn stop(&self, actor: EnvironmentActor) -> Result<Option<Environment>, EnvironmentError> {
        self.authorize(actor)?;
        let mut row = self.row.lock().unwrap();
        row.generation += 1;
        row.desired_state = DesiredState::Stopped;
        row.lease_expires_at = None;
        Ok(Some(row.clone()))
    }
    async fn heartbeat(
        &self,
        actor: EnvironmentActor,
        _: u32,
    ) -> Result<Environment, EnvironmentError> {
        self.authorize(actor)?;
        let row = self.row();
        if row.desired_state != DesiredState::Running
            || row.lease_expires_at.is_none_or(|lease| lease <= Utc::now())
        {
            Err(EnvironmentError::Conflict)
        } else {
            Ok(row)
        }
    }
    async fn reconcile_batch(&self, _: Uuid, _: u32) -> Result<Vec<Environment>, EnvironmentError> {
        let mut row = self.row.lock().unwrap();
        if !self.valid_session.load(Ordering::SeqCst)
            || row
                .lease_expires_at
                .is_some_and(|lease| lease <= Utc::now())
        {
            row.desired_state = DesiredState::Stopped;
            row.generation += 1;
            row.lease_expires_at = None;
        }
        Ok(vec![row.clone()])
    }
    async fn begin_provisioning(&self, _: Uuid, generation: i64) -> Result<bool, EnvironmentError> {
        let mut row = self.row.lock().unwrap();
        if row.generation != generation
            || row.provisioning_state != ProvisioningState::Unprovisioned
        {
            return Ok(false);
        }
        row.provisioning_state = ProvisioningState::VolumeCreating;
        Ok(true)
    }
    async fn record_observation(
        &self,
        _: Uuid,
        generation: i64,
        observation: EnvironmentObservation,
        retry: u32,
    ) -> Result<bool, EnvironmentError> {
        assert!(retry <= 15);
        if self.fail_record.load(Ordering::SeqCst) {
            return Err(EnvironmentError::Unavailable);
        }
        let mut row = self.row.lock().unwrap();
        if row.generation != generation {
            return Ok(false);
        }
        row.observed_state = observation.state;
        row.error_code = observation.error;
        row.observed_at = Some(Utc::now());
        if observation.volume_confirmed
            && row.provisioning_state == ProvisioningState::VolumeCreating
        {
            row.provisioning_state = ProvisioningState::VolumeReady;
        }
        if row.desired_state == DesiredState::Stopped && observation.confirms_stopped() {
            row.capacity_reserved = false;
        }
        self.writes.fetch_add(1, Ordering::SeqCst);
        Ok(true)
    }
}

struct Guard(Arc<AtomicBool>);
#[async_trait]
impl EnvironmentSupervisor for Guard {
    async fn check(&mut self) -> Result<(), EnvironmentError> {
        if self.0.load(Ordering::SeqCst) {
            Ok(())
        } else {
            Err(EnvironmentError::Unavailable)
        }
    }
}

#[derive(Clone, Copy, Default)]
enum DuringEffect {
    #[default]
    Nothing,
    LoseGuard,
    OppositeIntent,
    Timeout,
    FailPersistence,
}

struct Runtime {
    inspection: Mutex<RuntimeInspection>,
    store: Arc<Store>,
    guard: Arc<AtomicBool>,
    effect: Mutex<DuringEffect>,
    calls: Mutex<Vec<&'static str>>,
    inspect_error: Mutex<Option<EnvironmentError>>,
}

impl Runtime {
    fn effect(&self) -> Result<(), EnvironmentError> {
        match *self.effect.lock().unwrap() {
            DuringEffect::Nothing => {}
            DuringEffect::LoseGuard => {
                self.guard.store(false, Ordering::SeqCst);
            }
            DuringEffect::OppositeIntent => {
                let mut row = self.store.row.lock().unwrap();
                row.generation += 1;
                row.desired_state = if row.desired_state == DesiredState::Running {
                    DesiredState::Stopped
                } else {
                    DesiredState::Running
                };
            }
            DuringEffect::Timeout => return Err(EnvironmentError::Timeout),
            DuringEffect::FailPersistence => {
                self.store.fail_record.store(true, Ordering::SeqCst);
            }
        }
        Ok(())
    }
}

#[async_trait]
impl EnvironmentRuntime for Runtime {
    async fn inspect(&self, _: &Environment) -> Result<RuntimeInspection, EnvironmentError> {
        self.calls.lock().unwrap().push("inspect");
        if let Some(error) = *self.inspect_error.lock().unwrap() {
            return Err(error);
        }
        Ok(*self.inspection.lock().unwrap())
    }
    async fn create_volume(&self, environment: &Environment) -> Result<(), EnvironmentError> {
        assert_eq!(
            environment.provisioning_state,
            ProvisioningState::VolumeCreating
        );
        assert_eq!(
            self.store.row().provisioning_state,
            ProvisioningState::VolumeCreating
        );
        self.calls.lock().unwrap().push("create");
        self.inspection.lock().unwrap().volume = VolumePresence::Present;
        self.effect()
    }
    async fn start(&self, environment: &Environment) -> Result<(), EnvironmentError> {
        assert_eq!(
            environment.provisioning_state,
            ProvisioningState::VolumeReady
        );
        assert_eq!(
            self.store.row().provisioning_state,
            ProvisioningState::VolumeReady
        );
        self.calls.lock().unwrap().push("start");
        self.inspection.lock().unwrap().state = ObservedState::Running;
        self.effect()
    }
    async fn stop(&self, _: &Environment) -> Result<(), EnvironmentError> {
        self.calls.lock().unwrap().push("stop");
        // Capacity remains reserved throughout the external operation.
        assert!(self.store.row().capacity_reserved);
        self.inspection.lock().unwrap().state = ObservedState::Stopped;
        self.effect()
    }
}

struct Fixture {
    store: Arc<Store>,
    runtime: Arc<Runtime>,
    guard: Arc<AtomicBool>,
    availability: EnvironmentAvailability,
    config: EnvironmentStart,
}

impl Fixture {
    fn new() -> Self {
        let config = EnvironmentStart {
            installation_id: Uuid::new_v4(),
            image_id: "sha256:fixture".into(),
            policy_version: "1".into(),
            max_running: 2,
            lease_seconds: 900,
        };
        let store = Arc::new(Store {
            row: Mutex::new(Environment {
                id: Uuid::new_v4(),
                user_id: Uuid::new_v4(),
                installation_id: config.installation_id,
                desired_state: DesiredState::Running,
                observed_state: ObservedState::Starting,
                generation: 1,
                provisioning_state: ProvisioningState::Unprovisioned,
                volume_name: "fixture-volume".into(),
                container_name: "fixture-container".into(),
                image_id: config.image_id.clone(),
                policy_version: "1".into(),
                capacity_reserved: true,
                lease_expires_at: Some(Utc::now() + Duration::minutes(15)),
                observed_at: None,
                error_code: None,
                next_reconcile_at: Utc::now(),
            }),
            valid_session: AtomicBool::new(true),
            writes: AtomicUsize::new(0),
            fail_record: AtomicBool::new(false),
        });
        let guard = Arc::new(AtomicBool::new(true));
        let runtime = Arc::new(Runtime {
            inspection: Mutex::new(RuntimeInspection {
                state: ObservedState::NotCreated,
                volume: VolumePresence::Missing,
            }),
            store: store.clone(),
            guard: guard.clone(),
            effect: Mutex::new(DuringEffect::Nothing),
            calls: Mutex::new(vec![]),
            inspect_error: Mutex::new(None),
        });
        Self {
            store,
            runtime,
            guard,
            availability: EnvironmentAvailability::default(),
            config,
        }
    }
    fn reconciler(&self) -> EnvironmentReconciler {
        EnvironmentReconciler::new(
            self.store.clone(),
            self.runtime.clone(),
            Box::new(Guard(self.guard.clone())),
            self.config.clone(),
            self.availability.clone(),
        )
        .unwrap()
    }
    fn actor(&self) -> EnvironmentActor {
        EnvironmentActor {
            user_id: self.store.row().user_id,
            session_hash: CredentialHash([1; 32]),
        }
    }
    fn service(&self) -> EnvironmentService {
        EnvironmentService::new(
            self.store.clone(),
            self.config.clone(),
            self.availability.clone(),
        )
        .unwrap()
    }
    fn provisioned(&self, desired: DesiredState) {
        let mut row = self.store.row.lock().unwrap();
        row.desired_state = desired;
        row.observed_state = ObservedState::Running;
        row.observed_at = Some(Utc::now());
        row.provisioning_state = ProvisioningState::VolumeReady;
        *self.runtime.inspection.lock().unwrap() = RuntimeInspection {
            state: ObservedState::Running,
            volume: VolumePresence::Present,
        };
    }
}

#[tokio::test]
async fn read_never_provisions_and_requests_revalidate_the_actor() {
    let f = Fixture::new();
    f.service().get(f.actor()).await.unwrap();
    assert!(f.runtime.calls.lock().unwrap().is_empty());
    assert_eq!(
        f.store.row().provisioning_state,
        ProvisioningState::Unprovisioned
    );
    f.availability.ready();
    f.store.valid_session.store(false, Ordering::SeqCst);
    assert_eq!(
        f.service().start(f.actor()).await.unwrap_err(),
        EnvironmentError::Unauthorized
    );
    assert_eq!(
        f.service().heartbeat(f.actor()).await.unwrap_err(),
        EnvironmentError::Unauthorized
    );
}

#[tokio::test]
async fn provisioning_is_durable_and_repeated_steps_do_not_create_duplicates() {
    let f = Fixture::new();
    let mut worker = f.reconciler();
    worker.reconcile_once().await.unwrap();
    worker.reconcile_once().await.unwrap();
    assert_eq!(f.store.row().observed_state, ObservedState::Running);
    assert_eq!(
        f.store.row().provisioning_state,
        ProvisioningState::VolumeReady
    );
    assert_eq!(
        f.runtime
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|call| **call == "create")
            .count(),
        1
    );
    assert_eq!(
        f.runtime
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|call| **call == "start")
            .count(),
        1
    );
}

#[tokio::test]
async fn interrupted_or_missing_volumes_are_never_replaced() {
    for (phase, error) in [
        (
            ProvisioningState::VolumeCreating,
            EnvironmentError::ProvisioningUncertain,
        ),
        (
            ProvisioningState::VolumeReady,
            EnvironmentError::VolumeMissing,
        ),
    ] {
        let f = Fixture::new();
        f.store.row.lock().unwrap().provisioning_state = phase;
        f.reconciler().reconcile_once().await.unwrap();
        assert_eq!(f.store.row().error_code, Some(error));
        assert_eq!(*f.runtime.calls.lock().unwrap(), vec!["inspect"]);
        assert!(f.store.row().capacity_reserved);
    }
}

#[tokio::test]
async fn volume_created_before_crash_is_recovered_by_inspection() {
    let f = Fixture::new();
    *f.runtime.effect.lock().unwrap() = DuringEffect::FailPersistence;
    assert!(f.reconciler().reconcile_once().await.is_err());
    assert_eq!(
        f.store.row().provisioning_state,
        ProvisioningState::VolumeCreating
    );
    f.store.fail_record.store(false, Ordering::SeqCst);
    *f.runtime.effect.lock().unwrap() = DuringEffect::Nothing;
    f.reconciler().reconcile_once().await.unwrap();
    assert_eq!(f.store.row().observed_state, ObservedState::Running);
    assert_eq!(
        f.runtime
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|call| **call == "create")
            .count(),
        1
    );
}

#[tokio::test]
async fn guard_loss_during_start_or_stop_retains_capacity_and_is_terminal() {
    for desired in [DesiredState::Running, DesiredState::Stopped] {
        let f = Fixture::new();
        f.provisioned(desired);
        if desired == DesiredState::Running {
            f.runtime.inspection.lock().unwrap().state = ObservedState::Stopped;
        }
        *f.runtime.effect.lock().unwrap() = DuringEffect::LoseGuard;
        let mut worker = f.reconciler();
        assert_eq!(
            worker.reconcile_once().await.unwrap_err(),
            EnvironmentError::Unavailable
        );
        assert!(f.store.row().capacity_reserved);
        assert!(!f.availability.is_available());
        let count = f.runtime.calls.lock().unwrap().len();
        f.guard.store(true, Ordering::SeqCst);
        assert!(worker.reconcile_once().await.is_err());
        assert_eq!(f.runtime.calls.lock().unwrap().len(), count);
    }
}

#[tokio::test]
async fn timeout_does_not_confirm_stop_and_recovery_inspects_before_releasing_capacity() {
    let f = Fixture::new();
    f.provisioned(DesiredState::Stopped);
    *f.runtime.effect.lock().unwrap() = DuringEffect::Timeout;
    f.reconciler().reconcile_once().await.unwrap();
    assert!(f.store.row().capacity_reserved);
    assert_eq!(f.store.row().observed_state, ObservedState::Unavailable);
    *f.runtime.effect.lock().unwrap() = DuringEffect::Nothing;
    f.reconciler().reconcile_once().await.unwrap();
    assert!(!f.store.row().capacity_reserved);
    assert_eq!(f.store.row().observed_state, ObservedState::Stopped);
}

#[tokio::test]
async fn opposing_intents_during_effects_are_not_overwritten_by_stale_completion() {
    for desired in [DesiredState::Running, DesiredState::Stopped] {
        let f = Fixture::new();
        f.provisioned(desired);
        if desired == DesiredState::Running {
            f.runtime.inspection.lock().unwrap().state = ObservedState::Stopped;
        }
        *f.runtime.effect.lock().unwrap() = DuringEffect::OppositeIntent;
        let mut worker = f.reconciler();
        worker.reconcile_once().await.unwrap();
        assert!(f.store.row().capacity_reserved);
        assert_ne!(f.store.row().desired_state, desired);
        assert_eq!(f.store.row().generation, 2);
        assert_eq!(
            f.store.row().observed_state,
            if desired == DesiredState::Running {
                ObservedState::Starting
            } else {
                ObservedState::Stopping
            }
        );
        *f.runtime.effect.lock().unwrap() = DuringEffect::Nothing;
        worker.reconcile_once().await.unwrap();
        assert_eq!(
            f.store.row().observed_state,
            if desired == DesiredState::Running {
                ObservedState::Stopped
            } else {
                ObservedState::Running
            }
        );
    }
}

#[tokio::test]
async fn expiry_or_suspension_stops_and_late_heartbeat_cannot_revive() {
    for suspended in [false, true] {
        let f = Fixture::new();
        f.provisioned(DesiredState::Running);
        if suspended {
            f.store.valid_session.store(false, Ordering::SeqCst);
        } else {
            f.store.row.lock().unwrap().lease_expires_at = Some(Utc::now() - Duration::seconds(1));
        }
        f.reconciler().reconcile_once().await.unwrap();
        assert_eq!(f.store.row().observed_state, ObservedState::Stopped);
        assert!(!f.store.row().capacity_reserved);
        assert!(f.service().heartbeat(f.actor()).await.is_err());
    }
}

#[tokio::test]
async fn foreign_resources_are_never_mutated_and_failure_codes_remain_bounded() {
    let f = Fixture::new();
    *f.runtime.inspect_error.lock().unwrap() = Some(EnvironmentError::OwnershipConflict);
    f.reconciler().reconcile_once().await.unwrap();
    assert_eq!(*f.runtime.calls.lock().unwrap(), vec!["inspect"]);
    assert_eq!(
        f.store.row().error_code,
        Some(EnvironmentError::OwnershipConflict)
    );
}

#[tokio::test]
async fn missing_recorded_volume_does_not_prevent_stopping_its_owned_container() {
    let f = Fixture::new();
    f.provisioned(DesiredState::Stopped);
    f.runtime.inspection.lock().unwrap().volume = VolumePresence::Missing;
    f.reconciler().reconcile_once().await.unwrap();
    assert_eq!(
        *f.runtime.calls.lock().unwrap(),
        vec!["inspect", "stop", "inspect"]
    );
    assert_eq!(
        f.store.row().error_code,
        Some(EnvironmentError::VolumeMissing)
    );
    assert_eq!(
        f.runtime.inspection.lock().unwrap().state,
        ObservedState::Stopped
    );
}

#[tokio::test]
async fn previous_process_terminal_observations_stay_unavailable_until_inspected() {
    let f = Fixture::new();
    f.provisioned(DesiredState::Running);
    {
        let mut row = f.store.row.lock().unwrap();
        row.observed_state = ObservedState::Running;
        row.observed_at = Some(Utc::now() - Duration::minutes(1));
    }
    f.availability.ready();
    assert!(!f.service().can_report(Some(&f.store.row())));
    f.reconciler().reconcile_once().await.unwrap();
    assert!(f.service().can_report(Some(&f.store.row())));
}
