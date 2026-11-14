use chrono::{DateTime, Utc};
use factory_core::environments::*;
use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
};

/// Shared readiness only. PostgreSQL and the process installation lock own exclusion.
/// Failure is terminal for this host. Recovery must compose a new host with fresh
/// readiness and observations; old services and runtime permissions stay closed.
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

    pub(crate) fn ready(&self) {
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

    /// A container recorded for a different image or policy cannot be used.
    pub fn has_policy_mismatch(&self, environment: &Environment) -> bool {
        environment.image_id != self.config.image_id
            || environment.policy_version != self.config.policy_version
    }

    /// Do not republish a terminal observation from a previous host before
    /// this supervisor has inspected it. The last observation time stays visible.
    pub fn can_report(&self, environment: Option<&Environment>) -> bool {
        self.is_available()
            && environment.is_none_or(|environment| !self.has_policy_mismatch(environment))
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

    /// First access may only create an absent record, never restart an existing one.
    pub async fn initialize_if_absent(
        &self,
        actor: EnvironmentActor,
    ) -> Result<Environment, EnvironmentError> {
        self.require_available()?;
        self.store
            .initialize_if_absent(actor, self.config.clone())
            .await
    }

    pub async fn start(&self, actor: EnvironmentActor) -> Result<Environment, EnvironmentError> {
        self.require_available()?;
        if self
            .get(actor)
            .await?
            .as_ref()
            .is_some_and(|environment| self.has_policy_mismatch(environment))
        {
            return Err(EnvironmentError::PolicyMismatch);
        }
        self.store.start(actor, self.config.clone()).await
    }

    /// Revalidate the owner before interactive work; a cached Running state alone
    /// must never grant access while the supervisor or intent has changed.
    pub async fn require_running(
        &self,
        actor: EnvironmentActor,
    ) -> Result<Environment, EnvironmentError> {
        let environment = self
            .get(actor)
            .await?
            .ok_or(EnvironmentError::Unavailable)?;
        if self.has_policy_mismatch(&environment) {
            return Err(EnvironmentError::PolicyMismatch);
        }
        if !self.can_report(Some(&environment)) {
            return Err(EnvironmentError::Unavailable);
        }
        if environment.desired_state != DesiredState::Running
            || environment.observed_state != ObservedState::Running
            || environment.error_code.is_some()
        {
            return Err(EnvironmentError::Conflict);
        }
        Ok(environment)
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
