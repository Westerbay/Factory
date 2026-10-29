use super::*;
use sqlx::{Row, postgres::PgRow};

pub(super) fn observed(state: ObservedState) -> &'static str {
    match state {
        ObservedState::NotCreated => "not_created",
        ObservedState::Starting => "starting",
        ObservedState::Running => "running",
        ObservedState::Stopping => "stopping",
        ObservedState::Stopped => "stopped",
        ObservedState::Error => "error",
        ObservedState::Unavailable => "unavailable",
    }
}

pub(super) fn environment(row: &PgRow) -> Result<Environment, EnvironmentError> {
    let desired: String = row.try_get("desired_state").map_err(unavailable)?;
    let observed: String = row.try_get("observed_state").map_err(unavailable)?;
    let provisioning: String = row.try_get("provisioning_state").map_err(unavailable)?;
    let error: Option<String> = row.try_get("error_code").map_err(unavailable)?;
    Ok(Environment {
        id: row.try_get("id").map_err(unavailable)?,
        user_id: row.try_get("user_id").map_err(unavailable)?,
        installation_id: row.try_get("installation_id").map_err(unavailable)?,
        desired_state: match desired.as_str() {
            "running" => DesiredState::Running,
            "stopped" => DesiredState::Stopped,
            _ => return Err(EnvironmentError::Unavailable),
        },
        observed_state: match observed.as_str() {
            "not_created" => ObservedState::NotCreated,
            "starting" => ObservedState::Starting,
            "running" => ObservedState::Running,
            "stopping" => ObservedState::Stopping,
            "stopped" => ObservedState::Stopped,
            "error" => ObservedState::Error,
            "unavailable" => ObservedState::Unavailable,
            _ => return Err(EnvironmentError::Unavailable),
        },
        provisioning_state: match provisioning.as_str() {
            "unprovisioned" => ProvisioningState::Unprovisioned,
            "volume_creating" => ProvisioningState::VolumeCreating,
            "volume_ready" => ProvisioningState::VolumeReady,
            _ => return Err(EnvironmentError::Unavailable),
        },
        generation: row.try_get("generation").map_err(unavailable)?,
        volume_name: row.try_get("volume_name").map_err(unavailable)?,
        container_name: row.try_get("container_name").map_err(unavailable)?,
        image_id: row.try_get("image_id").map_err(unavailable)?,
        policy_version: row.try_get("policy_version").map_err(unavailable)?,
        observed_at: row.try_get("observed_at").map_err(unavailable)?,
        next_reconcile_at: row.try_get("next_reconcile_at").map_err(unavailable)?,
        error_code: error
            .map(|code| match code.as_str() {
                "unauthorized" => Ok(EnvironmentError::Unauthorized),
                "capacity_reached" => Ok(EnvironmentError::Conflict),
                "conflict" => Ok(EnvironmentError::Conflict),
                "unavailable" => Ok(EnvironmentError::Unavailable),
                "invalid_configuration" => Ok(EnvironmentError::InvalidConfiguration),
                "ownership_conflict" => Ok(EnvironmentError::OwnershipConflict),
                "volume_missing" => Ok(EnvironmentError::VolumeMissing),
                "provisioning_uncertain" => Ok(EnvironmentError::ProvisioningUncertain),
                "policy_mismatch" => Ok(EnvironmentError::PolicyMismatch),
                "timeout" => Ok(EnvironmentError::Timeout),
                _ => Err(EnvironmentError::Unavailable),
            })
            .transpose()?,
    })
}
