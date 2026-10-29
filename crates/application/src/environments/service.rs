use chrono::{DateTime, Utc};
use factory_core::environments::*;
use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
};

/// Shared readiness only. PostgreSQL and the process installation lock own exclusion.
/// A lost supervisor is terminal: only a new process may acquire ownership again.
#[derive(Clone)]
pub struct EnvironmentAvailability {
    state: Arc<AtomicU8>,
    started_at: DateTime<Utc>,
}

impl Default for EnvironmentAvailability {
    fn default() -> Self {
        Self {
            state: Arc::new(AtomicU8::new(0)),
            started_at: Utc::now(),
        }
    }
}

impl EnvironmentAvailability {
    pub fn is_available(&self) -> bool {
        self.state.load(Ordering::SeqCst) == 1
    }

    pub fn fail_closed(&self) {
        self.state.store(2, Ordering::SeqCst);
    }

    pub fn is_failed(&self) -> bool {
        self.state.load(Ordering::SeqCst) == 2
    }

    pub(super) fn ready(&self) {
        let _ = self
            .state
            .compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst);
    }
}

pub struct EnvironmentService {
    store: Arc<dyn EnvironmentStore>,
    config: EnvironmentStart,
    availability: EnvironmentAvailability,
}

impl EnvironmentService {
    pub fn new(
        store: Arc<dyn EnvironmentStore>,
        config: EnvironmentStart,
        availability: EnvironmentAvailability,
    ) -> Result<Self, EnvironmentError> {
        config.validate()?;
        Ok(Self {
            store,
            config,
            availability,
        })
    }

    pub fn is_available(&self) -> bool {
        self.availability.is_available()
    }

    /// Do not republish a terminal observation from a previous process before
    /// this supervisor has inspected it. The last observation time stays visible.
    pub fn can_report(&self, environment: Option<&Environment>) -> bool {
        self.is_available()
            && environment.is_none_or(|environment| {
                !matches!(
                    environment.observed_state,
                    ObservedState::Running | ObservedState::Stopped
                ) || environment
                    .observed_at
                    .is_some_and(|at| at >= self.availability.started_at)
            })
    }

    pub async fn get(
        &self,
        actor: EnvironmentActor,
    ) -> Result<Option<Environment>, EnvironmentError> {
        // Always revalidate the actor, even while the supervisor is unavailable.
        self.store.get(actor).await
    }

    pub async fn start(&self, actor: EnvironmentActor) -> Result<Environment, EnvironmentError> {
        self.require_available()?;
        self.store.start(actor, self.config.clone()).await
    }

    pub async fn stop(
        &self,
        actor: EnvironmentActor,
    ) -> Result<Option<Environment>, EnvironmentError> {
        // A stop remains useful while Docker is down: persist it for recovery.
        self.store.stop(actor).await
    }

    fn require_available(&self) -> Result<(), EnvironmentError> {
        if self.is_available() {
            Ok(())
        } else {
            Err(EnvironmentError::Unavailable)
        }
    }
}
