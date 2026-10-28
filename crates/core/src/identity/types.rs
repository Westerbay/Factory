use chrono::{DateTime, Utc};
use uuid::Uuid;

pub const LOGIN_ATTEMPT_TTL_SECONDS: i64 = 600;
pub const SESSION_TTL_SECONDS: i64 = 7 * 24 * 60 * 60;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct CredentialHash(pub [u8; 32]);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Member,
    Admin,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessStatus {
    Pending,
    Active,
    Suspended,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessDecision {
    Active,
    Suspended,
}

impl From<AccessDecision> for AccessStatus {
    fn from(value: AccessDecision) -> Self {
        match value {
            AccessDecision::Active => Self::Active,
            AccessDecision::Suspended => Self::Suspended,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct User {
    pub id: Uuid,
    pub display_name: String,
    pub handle: String,
    pub provider: String,
    pub role: Role,
    pub status: AccessStatus,
    pub version: i64,
    pub access_enabled_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedIdentity {
    pub provider: String,
    pub issuer: String,
    pub subject: String,
    pub display_name: String,
    pub handle: String,
}

impl VerifiedIdentity {
    pub fn validate(&self) -> Result<(), IdentityError> {
        let fields = [
            (&self.provider, 32),
            (&self.issuer, 256),
            (&self.subject, 256),
            (&self.display_name, 256),
            (&self.handle, 128),
        ];
        if fields.iter().any(|(value, max)| {
            value.trim().is_empty() || value.len() > *max || value.chars().any(char::is_control)
        }) {
            Err(IdentityError::InvalidLogin)
        } else {
            Ok(())
        }
    }
}

/// No Debug implementation: PKCE verifiers and session credentials must not enter logs.
pub struct NewLoginAttempt {
    pub state_hash: CredentialHash,
    pub binding_hash: CredentialHash,
    pub pkce_verifier: String,
}

pub struct LoginAttempt {
    pub pkce_verifier: String,
    pub created_at: DateTime<Utc>,
}

pub struct NewSession {
    pub token_hash: CredentialHash,
    pub csrf_token: String,
    pub previous_token_hash: Option<CredentialHash>,
}

pub struct LoginAdmission {
    pub identity: VerifiedIdentity,
    pub login_started_at: DateTime<Utc>,
    pub session: NewSession,
}

#[derive(Clone)]
pub struct ActiveSession {
    pub user: User,
    pub csrf_token: String,
    pub expires_at: DateTime<Utc>,
}

pub enum AdmissionOutcome {
    Active(ActiveSession),
    Pending,
    Suspended,
    StaleLogin,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdmissionDecision {
    Active,
    Pending,
    Suspended,
    StaleLogin,
}

#[derive(Clone, Debug)]
pub struct AccessRequest {
    pub target_id: Uuid,
    pub status: AccessDecision,
    pub expected_version: i64,
}

#[derive(Clone, Debug)]
pub struct UserListQuery {
    pub status: AccessStatus,
    pub after: Option<Uuid>,
    pub limit: u8,
}

impl UserListQuery {
    pub fn validate(&self) -> Result<(), IdentityError> {
        if !(1..=50).contains(&self.limit) {
            return Err(IdentityError::InvalidInput);
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct UserPage {
    pub items: Vec<User>,
    pub next_cursor: Option<Uuid>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum IdentityError {
    #[error("Invalid input")]
    InvalidInput,
    #[error("Invalid login")]
    InvalidLogin,
    #[error("An active session is required")]
    Unauthorized,
    #[error("Access denied")]
    Forbidden,
    #[error("User not found")]
    NotFound,
    #[error("The user changed; refresh and retry")]
    Conflict,
    #[error("The administrator cannot be suspended")]
    ProtectedAdministrator,
    #[error("Identity service is unavailable")]
    Unavailable,
}
