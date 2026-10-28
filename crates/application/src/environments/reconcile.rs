use super::EnvironmentAvailability;
use factory_core::environments::*;
use std::sync::Arc;

const BATCH_SIZE: u32 = 32;
// Each attempt is bounded by the runtime port; retry no faster than the lease sweep.
const RECHECK_SECONDS: u32 = 15;

/// The host selects work independently of runtime effects and allows at most one
/// active job per environment. Jobs share the installation's dedicated database
/// guard and process lock; no runtime operation holds the selection loop open.
pub struct EnvironmentReconciler {
    store: Arc<dyn EnvironmentStore>,
    runtime: Arc<dyn EnvironmentRuntime>,
    guard: Box<dyn EnvironmentSupervisor>,
    config: EnvironmentStart,
    availability: EnvironmentAvailability,
}

impl EnvironmentReconciler {
    pub fn new(
        store: Arc<dyn EnvironmentStore>,
        runtime: Arc<dyn EnvironmentRuntime>,
        guard: Box<dyn EnvironmentSupervisor>,
        config: EnvironmentStart,
        availability: EnvironmentAvailability,
    ) -> Result<Self, EnvironmentError> {
        config.validate()?;
        Ok(Self {
            store,
            runtime,
            guard,
            config,
            availability,
        })
    }

    /// Repair expired/suspended intent and select bounded due work without Docker.
    /// Run this sweep even while other environments have runtime work in flight.
    pub async fn next_batch(&mut self) -> Result<Vec<Environment>, EnvironmentError> {
        self.check_guard().await?;
        let batch = self
            .store
            .reconcile_batch(self.config.installation_id, BATCH_SIZE)
            .await?;
        self.check_guard().await?;
        self.availability.ready();
        Ok(batch)
    }

    /// Execute one selected environment. The host must serialize jobs with the
    /// same environment ID, while allowing unrelated environments to progress.
    pub async fn reconcile_environment(
        &mut self,
        environment: Environment,
    ) -> Result<(), EnvironmentError> {
        self.check_guard().await?;
        if environment.installation_id != self.config.installation_id {
            return Err(EnvironmentError::OwnershipConflict);
        }
        if let Err(error) = self.reconcile(&environment).await {
            // Never record success or release capacity after ownership is lost.
            if self.availability.is_failed() {
                return Err(EnvironmentError::Unavailable);
            }
            self.check_guard().await?;
            self.store
                .record_observation(
                    environment.id,
                    environment.generation,
                    EnvironmentObservation {
                        state: if matches!(
                            error,
                            EnvironmentError::Unavailable | EnvironmentError::Timeout
                        ) {
                            ObservedState::Unavailable
                        } else {
                            ObservedState::Error
                        },
                        error: Some(error),
                        volume_confirmed: false,
                    },
                    RECHECK_SECONDS,
                )
                .await?;
        }
        self.check_guard().await?;
        Ok(())
    }

    /// Unit fixtures use one row; production scheduling is always per environment.
    #[cfg(test)]
    pub(super) async fn reconcile_once(&mut self) -> Result<usize, EnvironmentError> {
        let batch = self.next_batch().await?;
        let count = batch.len();
        for environment in batch {
            self.reconcile_environment(environment).await?;
        }
        Ok(count)
    }

    async fn check_guard(&mut self) -> Result<(), EnvironmentError> {
        if self.availability.is_failed() || self.guard.check().await.is_err() {
            self.availability.fail_closed();
            return Err(EnvironmentError::Unavailable);
        }
        Ok(())
    }

    async fn inspect(
        &mut self,
        environment: &Environment,
    ) -> Result<RuntimeInspection, EnvironmentError> {
        self.check_guard().await?;
        let result = self.runtime.inspect(environment).await;
        self.check_guard().await?;
        result
    }

    async fn reconcile(&mut self, environment: &Environment) -> Result<(), EnvironmentError> {
        let inspection = self.inspect(environment).await?;
        match environment.desired_state {
            DesiredState::Stopped => self.reconcile_stopped(environment, inspection).await,
            DesiredState::Running => self.reconcile_running(environment, inspection).await,
        }
    }

    async fn reconcile_stopped(
        &mut self,
        environment: &Environment,
        inspection: RuntimeInspection,
    ) -> Result<(), EnvironmentError> {
        if matches!(
            inspection.state,
            ObservedState::NotCreated | ObservedState::Stopped
        ) {
            return self.record(environment, inspection).await;
        }
        self.check_guard().await?;
        if !self
            .store
            .record_observation(
                environment.id,
                environment.generation,
                EnvironmentObservation {
                    state: ObservedState::Stopping,
                    error: None,
                    volume_confirmed: inspection.volume == VolumePresence::Present,
                },
                RECHECK_SECONDS,
            )
            .await?
        {
            return Ok(());
        }
        self.check_guard().await?;
        let result = self.runtime.stop(environment).await;
        self.check_guard().await?;
        result?;
        let inspection = self.inspect(environment).await?;
        self.record(environment, inspection).await
    }

    async fn reconcile_running(
        &mut self,
        environment: &Environment,
        mut inspection: RuntimeInspection,
    ) -> Result<(), EnvironmentError> {
        if inspection.volume == VolumePresence::Missing {
            match environment.provisioning_state {
                ProvisioningState::VolumeReady => return Err(EnvironmentError::VolumeMissing),
                ProvisioningState::VolumeCreating => {
                    return Err(EnvironmentError::ProvisioningUncertain);
                }
                ProvisioningState::Unprovisioned => {
                    self.check_guard().await?;
                    if !self
                        .store
                        .begin_provisioning(environment.id, environment.generation)
                        .await?
                    {
                        return Ok(());
                    }
                    self.check_guard().await?;
                    let mut provisioning = environment.clone();
                    provisioning.provisioning_state = ProvisioningState::VolumeCreating;
                    let result = self.runtime.create_volume(&provisioning).await;
                    self.check_guard().await?;
                    result?;
                    inspection = self.inspect(environment).await?;
                    if inspection.volume != VolumePresence::Present {
                        return Err(EnvironmentError::ProvisioningUncertain);
                    }
                }
            }
        } else if environment.provisioning_state == ProvisioningState::Unprovisioned
            && !self
                .store
                .begin_provisioning(environment.id, environment.generation)
                .await?
        {
            return Ok(());
        }

        // Commit the inspected volume before start; a crash cannot make missing data
        // look like an unprovisioned environment on the next process.
        self.check_guard().await?;
        let current = self
            .store
            .record_observation(
                environment.id,
                environment.generation,
                EnvironmentObservation {
                    state: if inspection.state == ObservedState::Running {
                        ObservedState::Running
                    } else {
                        ObservedState::Starting
                    },
                    error: None,
                    volume_confirmed: true,
                },
                RECHECK_SECONDS,
            )
            .await?;
        if !current || inspection.state == ObservedState::Running {
            return Ok(());
        }
        self.check_guard().await?;
        let mut provisioned = environment.clone();
        provisioned.provisioning_state = ProvisioningState::VolumeReady;
        let result = self.runtime.start(&provisioned).await;
        self.check_guard().await?;
        result?;
        let inspection = self.inspect(environment).await?;
        if inspection.volume != VolumePresence::Present {
            return Err(EnvironmentError::VolumeMissing);
        }
        self.record(environment, inspection).await
    }

    async fn record(
        &mut self,
        environment: &Environment,
        inspection: RuntimeInspection,
    ) -> Result<(), EnvironmentError> {
        if inspection.volume == VolumePresence::Missing {
            match environment.provisioning_state {
                ProvisioningState::VolumeReady => return Err(EnvironmentError::VolumeMissing),
                ProvisioningState::VolumeCreating => {
                    return Err(EnvironmentError::ProvisioningUncertain);
                }
                ProvisioningState::Unprovisioned => {}
            }
        }
        self.check_guard().await?;
        self.store
            .record_observation(
                environment.id,
                environment.generation,
                EnvironmentObservation {
                    state: inspection.state,
                    error: None,
                    volume_confirmed: inspection.volume == VolumePresence::Present,
                },
                RECHECK_SECONDS,
            )
            .await?;
        Ok(())
    }
}
