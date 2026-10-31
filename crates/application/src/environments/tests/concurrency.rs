use super::*;
use tokio::sync::Notify;

struct MultipleEnvironments(Vec<Arc<Store>>);

impl MultipleEnvironments {
    fn store(&self, id: Uuid) -> &Store {
        self.0.iter().find(|store| store.row().id == id).unwrap()
    }
}

#[async_trait]
impl EnvironmentStore for MultipleEnvironments {
    async fn get(&self, _: EnvironmentActor) -> Result<Option<Environment>, EnvironmentError> {
        unreachable!()
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
    async fn reconcile_batch(
        &self,
        installation: Uuid,
        limit: u32,
    ) -> Result<Vec<Environment>, EnvironmentError> {
        let mut rows = vec![];
        for store in &self.0 {
            rows.extend(store.reconcile_batch(installation, limit).await?);
        }
        Ok(rows)
    }
    async fn begin_provisioning(
        &self,
        id: Uuid,
        generation: i64,
    ) -> Result<bool, EnvironmentError> {
        self.store(id).begin_provisioning(id, generation).await
    }
    async fn record_observation(
        &self,
        id: Uuid,
        generation: i64,
        observation: EnvironmentObservation,
        retry: u32,
    ) -> Result<bool, EnvironmentError> {
        self.store(id)
            .record_observation(id, generation, observation, retry)
            .await
    }
}

struct BlockingRuntime {
    runtimes: Vec<Arc<Runtime>>,
    provisioning_entered: Notify,
    release_provisioning: Notify,
}

impl BlockingRuntime {
    fn runtime(&self, environment: &Environment) -> &Runtime {
        self.runtimes
            .iter()
            .find(|runtime| runtime.store.row().id == environment.id)
            .unwrap()
    }
}

#[async_trait]
impl EnvironmentRuntime for BlockingRuntime {
    async fn inspect(
        &self,
        environment: &Environment,
    ) -> Result<RuntimeInspection, EnvironmentError> {
        self.runtime(environment).inspect(environment).await
    }
    async fn create_volume(&self, environment: &Environment) -> Result<(), EnvironmentError> {
        self.provisioning_entered.notify_one();
        self.release_provisioning.notified().await;
        self.runtime(environment).create_volume(environment).await
    }
    async fn start(&self, environment: &Environment) -> Result<(), EnvironmentError> {
        self.runtime(environment).start(environment).await
    }
    async fn stop(&self, environment: &Environment) -> Result<(), EnvironmentError> {
        self.runtime(environment).stop(environment).await
    }
}

#[tokio::test]
async fn blocked_provisioning_does_not_block_another_environments_suspension_and_stop() {
    let slow = Fixture::new();
    let suspended = Fixture::new();
    suspended.provisioned(DesiredState::Running);
    suspended.store.row.lock().unwrap().installation_id = slow.config.installation_id;
    let store = Arc::new(MultipleEnvironments(vec![
        slow.store.clone(),
        suspended.store.clone(),
    ]));
    let runtime = Arc::new(BlockingRuntime {
        runtimes: vec![slow.runtime.clone(), suspended.runtime.clone()],
        provisioning_entered: Notify::new(),
        release_provisioning: Notify::new(),
    });
    let worker = || {
        EnvironmentReconciler::new(
            store.clone(),
            runtime.clone(),
            Box::new(Guard(slow.guard.clone())),
            slow.config.clone(),
            slow.availability.clone(),
        )
        .unwrap()
    };
    let mut selector = worker();
    let initial = selector.next_batch().await.unwrap();
    assert_eq!(initial.len(), 2);
    let slow_environment = initial
        .into_iter()
        .find(|row| row.id == slow.store.row().id)
        .unwrap();
    let mut slow_worker = worker();
    let mut suspended_worker = worker();

    // No sleeps: B must finish before the test permits A's volume call to return.
    // A global serial queue would deadlock this handshake and fail the timeout.
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        tokio::join!(
            async {
                slow_worker
                    .reconcile_environment(slow_environment)
                    .await
                    .unwrap();
            },
            async {
                runtime.provisioning_entered.notified().await;
                suspended.store.active_user.store(false, Ordering::SeqCst);
                let batch = selector.next_batch().await.unwrap();
                let suspended_environment = batch
                    .into_iter()
                    .find(|row| row.id == suspended.store.row().id)
                    .unwrap();
                assert_eq!(suspended_environment.desired_state, DesiredState::Stopped);
                suspended_worker
                    .reconcile_environment(suspended_environment)
                    .await
                    .unwrap();
                assert_eq!(suspended.store.row().observed_state, ObservedState::Stopped);
                assert_eq!(
                    slow.store.row().provisioning_state,
                    ProvisioningState::VolumeCreating
                );
                assert!(!slow.runtime.calls.lock().unwrap().contains(&"create"));
                runtime.release_provisioning.notify_one();
            }
        );
    })
    .await
    .expect("an unrelated stop must finish while provisioning remains blocked");
    assert_eq!(slow.store.row().observed_state, ObservedState::Running);
}
