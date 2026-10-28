use super::*;
use chrono::{DateTime, Utc};

pub fn require_admin(user: &User) -> Result<(), IdentityError> {
    if user.status != AccessStatus::Active {
        return Err(IdentityError::Unauthorized);
    }
    if user.role != Role::Admin {
        return Err(IdentityError::Forbidden);
    }
    Ok(())
}

/// Evaluate under the store's locked user row. Every login must begin after activation.
pub fn admission_decision(user: &User, login_started_at: DateTime<Utc>) -> AdmissionDecision {
    match user.status {
        AccessStatus::Pending => AdmissionDecision::Pending,
        AccessStatus::Suspended => AdmissionDecision::Suspended,
        AccessStatus::Active => {
            if user
                .access_enabled_at
                .is_some_and(|at| login_started_at >= at)
            {
                AdmissionDecision::Active
            } else {
                AdmissionDecision::StaleLogin
            }
        }
    }
}

/// Returns whether a real transition is required. No-op active writes must not
/// reset access_enabled_at. Call after validating the live session and locking target.
pub fn authorize_access_change(
    actor: &User,
    target: &User,
    request: &AccessRequest,
) -> Result<bool, IdentityError> {
    require_admin(actor)?;
    if request.expected_version < 1 || target.id != request.target_id {
        return Err(IdentityError::InvalidInput);
    }
    if target.role == Role::Admin {
        return Err(IdentityError::ProtectedAdministrator);
    }
    if target.version != request.expected_version {
        return Err(IdentityError::Conflict);
    }
    Ok(target.status != AccessStatus::from(request.status))
}
