use super::EnvironmentAvailability;
use factory_core::environments::*;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct EnvironmentUpgradePreview {
    pub snapshot: EnvironmentUpgradeSnapshot,
    pub old_container_id: Option<String>,
    pub target: EnvironmentStart,
    pub already_current: bool,
}

/// The composition owner must retain the installation process lock for this service's
/// lifetime. The dedicated database supervisor additionally excludes another host.
pub struct EnvironmentUpgradeService {
    store: Arc<dyn EnvironmentUpgradeStore>,
    runtime: Arc<dyn EnvironmentUpgradeRuntime>,
    supervisor: Box<dyn EnvironmentSupervisor>,
    availability: EnvironmentAvailability,
    target: EnvironmentStart,
}

impl EnvironmentUpgradeService {
    pub fn new(
        store: Arc<dyn EnvironmentUpgradeStore>,
        runtime: Arc<dyn EnvironmentUpgradeRuntime>,
        supervisor: Box<dyn EnvironmentSupervisor>,
        availability: EnvironmentAvailability,
        target: EnvironmentStart,
    ) -> Result<Self, EnvironmentError> {
        target.validate()?;
        Ok(Self {
            store,
            runtime,
            supervisor,
            availability,
            target,
        })
    }

    async fn guard(&mut self) -> Result<(), EnvironmentError> {
        if self.availability.is_failed() || self.supervisor.check().await.is_err() {
            self.availability.fail_closed();
            return Err(EnvironmentError::Unavailable);
        }
        Ok(())
    }

    pub async fn preview(
        &mut self,
        user_id: Uuid,
    ) -> Result<EnvironmentUpgradePreview, EnvironmentError> {
        self.guard().await?;
        let snapshot = self
            .store
            .upgrade_snapshot(user_id, self.target.installation_id)
            .await?;
        let env = &snapshot.environment;
        if env.desired_state != DesiredState::Stopped {
            return Err(EnvironmentError::Conflict);
        }
        if env.provisioning_state != ProvisioningState::VolumeReady {
            return Err(EnvironmentError::ProvisioningUncertain);
        }
        let current = env.image_id == self.target.image_id
            && env.policy_version == self.target.policy_version;
        if let Some(intent) = &snapshot.pending {
            require_intent(env, intent, &self.target)?;
        }
        let old_container_id = self.runtime.inspect_upgrade(env).await?;
        self.guard().await?;
        match (&snapshot.pending, &old_container_id) {
            (Some(intent), Some(id)) if intent.old_container_id != *id => {
                return Err(EnvironmentError::OwnershipConflict);
            }
            (None, None) if !current => return Err(EnvironmentError::Conflict),
            _ => {}
        }
        Ok(EnvironmentUpgradePreview {
            already_current: current && snapshot.pending.is_none(),
            snapshot,
            old_container_id,
            target: self.target.clone(),
        })
    }

    /// Execute only after the operator has confirmed the displayed stable identity.
    pub async fn execute(
        &mut self,
        preview: &EnvironmentUpgradePreview,
    ) -> Result<(), EnvironmentError> {
        self.guard().await?;
        let fresh = self.preview(preview.snapshot.environment.user_id).await?;
        if !same_preview(preview, &fresh) {
            return Err(EnvironmentError::Conflict);
        }
        if fresh.already_current {
            return Ok(());
        }
        let env = fresh.snapshot.environment;
        let old_id = fresh
            .snapshot
            .pending
            .as_ref()
            .map(|intent| intent.old_container_id.as_str())
            .or(fresh.old_container_id.as_deref())
            .ok_or(EnvironmentError::Conflict)?;
        let intent = self.store.begin_upgrade(&env, old_id, &self.target).await?;
        self.guard().await?;
        let mut reserved = env;
        reserved.generation = intent.generation;
        require_intent(&reserved, &intent, &self.target)?;
        let mut target = reserved.clone();
        target.image_id.clone_from(&intent.target_image_id);
        target
            .policy_version
            .clone_from(&intent.target_policy_version);
        self.runtime.prepare_upgrade(&target).await?;
        self.guard().await?;
        // Re-read the durable generation before the only destructive step.
        let check = self
            .store
            .upgrade_snapshot(reserved.user_id, reserved.installation_id)
            .await?;
        if check.pending.as_ref() != Some(&intent)
            || !same_environment(&reserved, &check.environment)
        {
            return Err(EnvironmentError::Conflict);
        }
        self.guard().await?;
        self.runtime
            .remove_upgrade_container(&reserved, &intent.old_container_id)
            .await?;
        self.guard().await?;
        if self.runtime.inspect_upgrade(&reserved).await?.is_some() {
            return Err(EnvironmentError::OwnershipConflict);
        }
        self.guard().await?;
        self.store.finish_upgrade(&intent).await?;
        self.guard().await
    }
}

fn require_intent(
    env: &Environment,
    intent: &EnvironmentUpgradeIntent,
    target: &EnvironmentStart,
) -> Result<(), EnvironmentError> {
    if intent.environment_id != env.id
        || intent.generation != env.generation
        || intent.target_image_id != target.image_id
        || intent.target_policy_version != target.policy_version
    {
        return Err(EnvironmentError::Conflict);
    }
    Ok(())
}

fn same_environment(a: &Environment, b: &Environment) -> bool {
    a.id == b.id
        && a.user_id == b.user_id
        && a.installation_id == b.installation_id
        && a.generation == b.generation
        && a.desired_state == b.desired_state
        && a.provisioning_state == b.provisioning_state
        && a.image_id == b.image_id
        && a.policy_version == b.policy_version
        && a.container_name == b.container_name
        && a.volume_name == b.volume_name
}

fn same_preview(a: &EnvironmentUpgradePreview, b: &EnvironmentUpgradePreview) -> bool {
    same_environment(&a.snapshot.environment, &b.snapshot.environment)
        && a.snapshot.pending == b.snapshot.pending
        && a.old_container_id == b.old_container_id
        && a.target.installation_id == b.target.installation_id
        && a.target.image_id == b.target.image_id
        && a.target.policy_version == b.target.policy_version
        && a.already_current == b.already_current
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    };

    struct Store {
        snapshot: Mutex<EnvironmentUpgradeSnapshot>,
        fail_finish: AtomicBool,
    }
    #[async_trait::async_trait]
    impl EnvironmentUpgradeStore for Store {
        async fn upgrade_snapshot(
            &self,
            _: Uuid,
            _: Uuid,
        ) -> Result<EnvironmentUpgradeSnapshot, EnvironmentError> {
            Ok(self.snapshot.lock().unwrap().clone())
        }
        async fn begin_upgrade(
            &self,
            expected: &Environment,
            id: &str,
            target: &EnvironmentStart,
        ) -> Result<EnvironmentUpgradeIntent, EnvironmentError> {
            let mut state = self.snapshot.lock().unwrap();
            assert!(same_environment(expected, &state.environment));
            if let Some(intent) = &state.pending {
                return Ok(intent.clone());
            }
            state.environment.generation += 1;
            let intent = EnvironmentUpgradeIntent {
                operation_id: Uuid::new_v4(),
                environment_id: expected.id,
                generation: state.environment.generation,
                old_container_id: id.into(),
                target_image_id: target.image_id.clone(),
                target_policy_version: target.policy_version.clone(),
            };
            state.pending = Some(intent.clone());
            Ok(intent)
        }
        async fn finish_upgrade(
            &self,
            intent: &EnvironmentUpgradeIntent,
        ) -> Result<(), EnvironmentError> {
            if self.fail_finish.swap(false, Ordering::SeqCst) {
                return Err(EnvironmentError::Unavailable);
            }
            let mut state = self.snapshot.lock().unwrap();
            assert_eq!(state.pending.as_ref(), Some(intent));
            state
                .environment
                .image_id
                .clone_from(&intent.target_image_id);
            state
                .environment
                .policy_version
                .clone_from(&intent.target_policy_version);
            state.environment.observed_state = ObservedState::NotCreated;
            state.pending = None;
            Ok(())
        }
    }
    struct Runtime {
        container: Mutex<Option<String>>,
        volume: AtomicBool,
        running: AtomicBool,
        fail_prepare: AtomicBool,
        fail_after_remove: AtomicBool,
        lose_guard: AtomicBool,
        guard: Arc<AtomicBool>,
        store: Arc<Store>,
    }
    #[async_trait::async_trait]
    impl EnvironmentUpgradeRuntime for Runtime {
        async fn inspect_upgrade(
            &self,
            _: &Environment,
        ) -> Result<Option<String>, EnvironmentError> {
            if !self.volume.load(Ordering::SeqCst) {
                return Err(EnvironmentError::VolumeMissing);
            }
            if self.running.load(Ordering::SeqCst) {
                return Err(EnvironmentError::Conflict);
            }
            Ok(self.container.lock().unwrap().clone())
        }
        async fn prepare_upgrade(&self, _: &Environment) -> Result<(), EnvironmentError> {
            assert!(self.store.snapshot.lock().unwrap().pending.is_some());
            if self.lose_guard.swap(false, Ordering::SeqCst) {
                self.guard.store(false, Ordering::SeqCst);
            }
            if self.fail_prepare.swap(false, Ordering::SeqCst) {
                return Err(EnvironmentError::Unavailable);
            }
            Ok(())
        }
        async fn remove_upgrade_container(
            &self,
            _: &Environment,
            id: &str,
        ) -> Result<(), EnvironmentError> {
            assert_eq!(
                self.store
                    .snapshot
                    .lock()
                    .unwrap()
                    .pending
                    .as_ref()
                    .unwrap()
                    .old_container_id,
                id
            );
            let removed = self.container.lock().unwrap().take();
            if let Some(removed) = removed {
                assert_eq!(removed, id);
            }
            if self.fail_after_remove.swap(false, Ordering::SeqCst) {
                return Err(EnvironmentError::Timeout);
            }
            Ok(())
        }
    }
    struct Guard(Arc<AtomicBool>);
    #[async_trait::async_trait]
    impl EnvironmentSupervisor for Guard {
        async fn check(&mut self) -> Result<(), EnvironmentError> {
            if self.0.load(Ordering::SeqCst) {
                Ok(())
            } else {
                Err(EnvironmentError::Unavailable)
            }
        }
    }
    struct Fixture {
        store: Arc<Store>,
        runtime: Arc<Runtime>,
        target: EnvironmentStart,
    }
    impl Fixture {
        fn new() -> Self {
            let env = Environment {
                id: Uuid::new_v4(),
                user_id: Uuid::new_v4(),
                installation_id: Uuid::new_v4(),
                desired_state: DesiredState::Stopped,
                observed_state: ObservedState::Stopped,
                generation: 4,
                provisioning_state: ProvisioningState::VolumeReady,
                volume_name: "persistent-home".into(),
                container_name: "owned-container".into(),
                image_id: format!("sha256:{}", "a".repeat(64)),
                policy_version: "2".into(),
                observed_at: None,
                error_code: None,
                next_reconcile_at: chrono::Utc::now(),
            };
            let target = EnvironmentStart {
                installation_id: env.installation_id,
                image_id: format!("sha256:{}", "b".repeat(64)),
                policy_version: "3".into(),
            };
            let store = Arc::new(Store {
                snapshot: Mutex::new(EnvironmentUpgradeSnapshot {
                    environment: env,
                    pending: None,
                }),
                fail_finish: AtomicBool::new(false),
            });
            let runtime = Arc::new(Runtime {
                container: Mutex::new(Some("c".repeat(64))),
                volume: AtomicBool::new(true),
                running: AtomicBool::new(false),
                fail_prepare: AtomicBool::new(false),
                fail_after_remove: AtomicBool::new(false),
                lose_guard: AtomicBool::new(false),
                guard: Arc::new(AtomicBool::new(true)),
                store: store.clone(),
            });
            Self {
                store,
                runtime,
                target,
            }
        }
        fn service(&self) -> EnvironmentUpgradeService {
            EnvironmentUpgradeService::new(
                self.store.clone(),
                self.runtime.clone(),
                Box::new(Guard(self.runtime.guard.clone())),
                EnvironmentAvailability::default(),
                self.target.clone(),
            )
            .unwrap()
        }
        fn user(&self) -> Uuid {
            self.store.snapshot.lock().unwrap().environment.user_id
        }
    }

    #[tokio::test]
    async fn interrupted_replacement_resumes_same_intent_before_and_after_removal() {
        for phase in 0..3 {
            let fixture = Fixture::new();
            match phase {
                0 => fixture.runtime.fail_prepare.store(true, Ordering::SeqCst),
                1 => fixture
                    .runtime
                    .fail_after_remove
                    .store(true, Ordering::SeqCst),
                _ => fixture.store.fail_finish.store(true, Ordering::SeqCst),
            }
            let mut service = fixture.service();
            let preview = service.preview(fixture.user()).await.unwrap();
            assert!(service.execute(&preview).await.is_err());
            let intent = fixture
                .store
                .snapshot
                .lock()
                .unwrap()
                .pending
                .clone()
                .unwrap();
            assert_eq!(intent.generation, 5);
            drop(service);
            let mut resumed = fixture.service();
            let preview = resumed.preview(fixture.user()).await.unwrap();
            assert_eq!(preview.snapshot.pending.as_ref(), Some(&intent));
            resumed.execute(&preview).await.unwrap();
            let done = resumed.preview(fixture.user()).await.unwrap();
            assert!(done.already_current);
            resumed.execute(&done).await.unwrap();
            assert_eq!(done.snapshot.environment.volume_name, "persistent-home");
            assert_eq!(
                done.snapshot.environment.desired_state,
                DesiredState::Stopped
            );
            assert_eq!(done.snapshot.environment.generation, 5);
        }
    }

    #[tokio::test]
    async fn stale_confirmation_running_missing_home_and_replacement_id_are_rejected() {
        let fixture = Fixture::new();
        let mut service = fixture.service();
        let preview = service.preview(fixture.user()).await.unwrap();
        fixture
            .store
            .snapshot
            .lock()
            .unwrap()
            .environment
            .generation += 1;
        assert_eq!(
            service.execute(&preview).await,
            Err(EnvironmentError::Conflict)
        );
        assert!(fixture.store.snapshot.lock().unwrap().pending.is_none());
        fixture.runtime.running.store(true, Ordering::SeqCst);
        assert!(matches!(
            service.preview(fixture.user()).await,
            Err(EnvironmentError::Conflict)
        ));
        fixture.runtime.running.store(false, Ordering::SeqCst);
        fixture.runtime.volume.store(false, Ordering::SeqCst);
        assert!(matches!(
            service.preview(fixture.user()).await,
            Err(EnvironmentError::VolumeMissing)
        ));
        fixture.runtime.volume.store(true, Ordering::SeqCst);
        fixture.runtime.fail_prepare.store(true, Ordering::SeqCst);
        let preview = service.preview(fixture.user()).await.unwrap();
        assert!(service.execute(&preview).await.is_err());
        *fixture.runtime.container.lock().unwrap() = Some("d".repeat(64));
        assert!(matches!(
            service.preview(fixture.user()).await,
            Err(EnvironmentError::OwnershipConflict)
        ));
    }

    #[tokio::test]
    async fn lost_supervisor_permanently_prevents_removal_and_keeps_recovery_intent() {
        let fixture = Fixture::new();
        let mut service = fixture.service();
        let preview = service.preview(fixture.user()).await.unwrap();
        fixture.runtime.lose_guard.store(true, Ordering::SeqCst);
        assert_eq!(
            service.execute(&preview).await,
            Err(EnvironmentError::Unavailable)
        );
        assert!(fixture.runtime.container.lock().unwrap().is_some());
        assert!(fixture.store.snapshot.lock().unwrap().pending.is_some());
        fixture.runtime.guard.store(true, Ordering::SeqCst);
        assert!(matches!(
            service.preview(fixture.user()).await,
            Err(EnvironmentError::Unavailable)
        ));
    }
}
