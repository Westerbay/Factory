use chrono::{DateTime, Utc};
use factory_core::environments::{DesiredState, Environment, ObservedState};
use serde::Serialize;
use utoipa::ToSchema;

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentStatus {
    NotCreated,
    Starting,
    Running,
    Stopping,
    Stopped,
    Error,
    Unavailable,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentResponse {
    pub status: EnvironmentStatus,
    pub observed_at: Option<DateTime<Utc>>,
    pub idle_expires_at: Option<DateTime<Utc>>,
    /// Bounded public error code; never includes provider responses or resource names.
    pub error_code: Option<String>,
}

impl EnvironmentResponse {
    pub(super) fn project(environment: Option<&Environment>, available: bool) -> Self {
        let mut response = Self {
            status: EnvironmentStatus::NotCreated,
            observed_at: environment.and_then(|e| e.observed_at),
            idle_expires_at: environment.and_then(|e| e.lease_expires_at),
            error_code: environment.and_then(|e| e.error_code.map(|error| error.code().to_owned())),
        };
        if !available {
            response.status = EnvironmentStatus::Unavailable;
            response.error_code = Some("unavailable".into());
        } else if let Some(environment) = environment {
            response.status = match environment.observed_state {
                ObservedState::Error => EnvironmentStatus::Error,
                ObservedState::Unavailable => EnvironmentStatus::Unavailable,
                ObservedState::NotCreated if environment.desired_state == DesiredState::Stopped => {
                    EnvironmentStatus::NotCreated
                }
                ObservedState::Stopped if environment.desired_state == DesiredState::Stopped => {
                    EnvironmentStatus::Stopped
                }
                ObservedState::Running if environment.desired_state == DesiredState::Running => {
                    EnvironmentStatus::Running
                }
                _ if environment.desired_state == DesiredState::Running => {
                    EnvironmentStatus::Starting
                }
                _ => EnvironmentStatus::Stopping,
            };
        }
        response
    }
}
