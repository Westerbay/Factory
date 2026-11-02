//! Provider-neutral account connection projections and short-lived sign-in operations.
use crate::environments::{Environment, EnvironmentError};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AgentConnectionError {
    #[error("An active session is required")]
    Unauthorized,
    #[error("An account operation is already active or the environment changed")]
    Conflict,
    #[error("The account service is unavailable")]
    Unavailable,
    #[error("The account operation timed out")]
    Timeout,
    #[error("The account provider returned an invalid response")]
    Protocol,
    #[error("The account authentication method is not supported")]
    UnsupportedMethod,
    #[error("The account provider did not complete sign-in")]
    LoginFailed,
    #[error("The account sign-in was cancelled")]
    Cancelled,
    #[error("The account sign-in expired")]
    Expired,
    #[error("Account controller cleanup could not be confirmed")]
    CleanupUncertain,
}
impl AgentConnectionError {
    pub fn code(self) -> &'static str {
        match self {
            Self::Unauthorized => "unauthorized",
            Self::Conflict => "conflict",
            Self::Unavailable => "unavailable",
            Self::Timeout => "timeout",
            Self::Protocol => "invalid_provider_response",
            Self::UnsupportedMethod => "unsupported_method",
            Self::LoginFailed => "login_failed",
            Self::Cancelled => "cancelled",
            Self::Expired => "expired",
            Self::CleanupUncertain => "cleanup_uncertain",
        }
    }
}
impl From<EnvironmentError> for AgentConnectionError {
    fn from(error: EnvironmentError) -> Self {
        match error {
            EnvironmentError::Unauthorized => Self::Unauthorized,
            EnvironmentError::Conflict => Self::Conflict,
            EnvironmentError::Timeout => Self::Timeout,
            _ => Self::Unavailable,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectionStatus {
    Unavailable,
    Disconnected,
    Connecting,
    Connected,
    Error,
}
#[derive(Clone, PartialEq, Eq)]
pub struct AccountSummary {
    pub email: Option<String>,
    pub plan: String,
}
/// Intentionally no Debug: a login code must not enter diagnostic logs.
#[derive(Clone)]
pub struct LoginChallenge {
    pub operation_id: Uuid,
    pub verification_url: String,
    /// A device code to display, or None when the provider returns a code in its browser flow.
    pub user_code: Option<String>,
    pub expires_at: DateTime<Utc>,
}
#[derive(Clone)]
pub struct ConnectionSnapshot {
    pub status: ConnectionStatus,
    pub account: Option<AccountSummary>,
    pub challenge: Option<LoginChallenge>,
    pub error: Option<AgentConnectionError>,
    pub observed_at: DateTime<Utc>,
}
impl ConnectionSnapshot {
    pub fn account(account: Option<AccountSummary>, observed_at: DateTime<Utc>) -> Self {
        Self {
            status: if account.is_some() {
                ConnectionStatus::Connected
            } else {
                ConnectionStatus::Disconnected
            },
            account,
            challenge: None,
            error: None,
            observed_at,
        }
    }
    pub fn failure(error: AgentConnectionError, observed_at: DateTime<Utc>) -> Self {
        Self {
            status: ConnectionStatus::Error,
            account: None,
            challenge: None,
            error: Some(error),
            observed_at,
        }
    }
}
#[async_trait]
pub trait AgentConnectionPort: Send + Sync {
    async fn read(
        &self,
        environment: &Environment,
    ) -> Result<ConnectionSnapshot, AgentConnectionError>;
    async fn begin_login(
        &self,
        environment: &Environment,
        operation_id: Uuid,
        expires_at: DateTime<Utc>,
    ) -> Result<ConnectionSnapshot, AgentConnectionError>;
    async fn cancel_login(
        &self,
        environment: &Environment,
        operation_id: Uuid,
    ) -> Result<ConnectionSnapshot, AgentConnectionError>;
    /// Submit a browser-issued code to the same native process that began sign-in.
    async fn submit_login_code(
        &self,
        _environment: &Environment,
        _operation_id: Uuid,
        _code: &str,
    ) -> Result<ConnectionSnapshot, AgentConnectionError> {
        Err(AgentConnectionError::UnsupportedMethod)
    }
    async fn logout(
        &self,
        environment: &Environment,
    ) -> Result<ConnectionSnapshot, AgentConnectionError>;
}
