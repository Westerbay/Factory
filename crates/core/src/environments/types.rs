use crate::identity::CredentialHash;
use chrono::{DateTime, Utc};
use uuid::Uuid;

#[derive(Clone, Copy)]
pub struct EnvironmentActor {
    pub user_id: Uuid,
    pub session_hash: CredentialHash,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DesiredState {
    Stopped,
    Running,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObservedState {
    NotCreated,
    Starting,
    Running,
    Stopping,
    Stopped,
    Error,
    Unavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProvisioningState {
    Unprovisioned,
    VolumeCreating,
    VolumeReady,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum EnvironmentError {
    #[error("An active session is required")]
    Unauthorized,
    #[error("Environment capacity is reached")]
    CapacityReached,
    #[error("The environment changed; refresh and retry")]
    Conflict,
    #[error("Environment service is unavailable")]
    Unavailable,
    #[error("Invalid environment configuration")]
    InvalidConfiguration,
    #[error("The resource ownership does not match")]
    OwnershipConflict,
    #[error("The persistent volume is missing")]
    VolumeMissing,
    #[error("Interrupted volume provisioning requires operator repair")]
    ProvisioningUncertain,
    #[error("The resource does not match the required policy")]
    PolicyMismatch,
    #[error("The runtime operation timed out")]
    Timeout,
}

impl EnvironmentError {
    pub fn code(self) -> &'static str {
        match self {
            Self::Unauthorized => "unauthorized",
            Self::CapacityReached => "capacity_reached",
            Self::Conflict => "conflict",
            Self::Unavailable => "unavailable",
            Self::InvalidConfiguration => "invalid_configuration",
            Self::OwnershipConflict => "ownership_conflict",
            Self::VolumeMissing => "volume_missing",
            Self::ProvisioningUncertain => "provisioning_uncertain",
            Self::PolicyMismatch => "policy_mismatch",
            Self::Timeout => "timeout",
        }
    }
}

/// Internal record. HTTP must project only status, observation time, lease and error code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Environment {
    pub id: Uuid,
    pub user_id: Uuid,
    pub installation_id: Uuid,
    pub desired_state: DesiredState,
    pub observed_state: ObservedState,
    pub generation: i64,
    pub provisioning_state: ProvisioningState,
    pub volume_name: String,
    pub container_name: String,
    pub image_id: String,
    pub policy_version: String,
    pub capacity_reserved: bool,
    pub lease_expires_at: Option<DateTime<Utc>>,
    pub observed_at: Option<DateTime<Utc>>,
    pub error_code: Option<EnvironmentError>,
    pub next_reconcile_at: DateTime<Utc>,
}

#[derive(Clone, Debug)]
pub struct EnvironmentStart {
    pub installation_id: Uuid,
    pub image_id: String,
    pub policy_version: String,
    pub max_running: u32,
    pub lease_seconds: u32,
}

impl EnvironmentStart {
    pub fn validate(&self) -> Result<(), EnvironmentError> {
        if self.installation_id.is_nil()
            || self.max_running == 0
            || self.lease_seconds == 0
            || self.image_id.is_empty()
            || self.image_id.len() > 256
            || self.policy_version.is_empty()
            || self.policy_version.len() > 64
            || self.image_id.chars().any(char::is_control)
            || self.policy_version.chars().any(char::is_control)
        {
            return Err(EnvironmentError::InvalidConfiguration);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VolumePresence {
    Missing,
    Present,
}

/// A successful inspection has verified ownership of every resource it found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RuntimeInspection {
    pub state: ObservedState,
    pub volume: VolumePresence,
}

#[derive(Clone, Copy, Debug)]
pub struct EnvironmentObservation {
    pub state: ObservedState,
    pub error: Option<EnvironmentError>,
    /// True only after successful ownership-verified volume inspection/creation.
    pub volume_confirmed: bool,
}

impl EnvironmentObservation {
    pub fn confirms_stopped(self) -> bool {
        self.error.is_none()
            && matches!(
                self.state,
                ObservedState::NotCreated | ObservedState::Stopped
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uncertain_or_in_progress_observations_never_release_capacity() {
        for state in [
            ObservedState::Starting,
            ObservedState::Running,
            ObservedState::Stopping,
            ObservedState::Error,
            ObservedState::Unavailable,
        ] {
            assert!(
                !EnvironmentObservation {
                    state,
                    error: None,
                    volume_confirmed: false
                }
                .confirms_stopped()
            );
        }
        assert!(
            !EnvironmentObservation {
                state: ObservedState::Stopped,
                error: Some(EnvironmentError::Timeout),
                volume_confirmed: true
            }
            .confirms_stopped()
        );
        assert!(
            EnvironmentObservation {
                state: ObservedState::NotCreated,
                error: None,
                volume_confirmed: false
            }
            .confirms_stopped()
        );
    }
}
