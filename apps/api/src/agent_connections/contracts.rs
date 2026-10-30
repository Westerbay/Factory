use chrono::{DateTime, Utc};
use factory_core::agent_connections::{ConnectionSnapshot, ConnectionStatus};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum AgentProvider {
    Codex,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum AgentConnectionStatus {
    Unavailable,
    Disconnected,
    Connecting,
    Connected,
    Error,
}

#[derive(Serialize, ToSchema)]
pub struct AgentAccount {
    pub email: Option<String>,
    pub plan: String,
}

/// Device authorization instructions, never provider credentials or a provider login ID.
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AgentLoginChallenge {
    pub operation_id: Uuid,
    pub verification_url: String,
    pub user_code: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AgentConnectionResponse {
    pub provider: AgentProvider,
    pub status: AgentConnectionStatus,
    pub account: Option<AgentAccount>,
    pub challenge: Option<AgentLoginChallenge>,
    /// Bounded application error code, never the raw provider response.
    pub error_code: Option<String>,
    pub observed_at: Option<DateTime<Utc>>,
}

impl AgentConnectionResponse {
    pub fn unavailable() -> Self {
        Self {
            provider: AgentProvider::Codex,
            status: AgentConnectionStatus::Unavailable,
            account: None,
            challenge: None,
            error_code: Some("unavailable".into()),
            observed_at: None,
        }
    }
}

impl From<ConnectionSnapshot> for AgentConnectionResponse {
    fn from(snapshot: ConnectionSnapshot) -> Self {
        Self {
            provider: AgentProvider::Codex,
            status: match snapshot.status {
                ConnectionStatus::Unavailable => AgentConnectionStatus::Unavailable,
                ConnectionStatus::Disconnected => AgentConnectionStatus::Disconnected,
                ConnectionStatus::Connecting => AgentConnectionStatus::Connecting,
                ConnectionStatus::Connected => AgentConnectionStatus::Connected,
                ConnectionStatus::Error => AgentConnectionStatus::Error,
            },
            account: snapshot.account.map(|account| AgentAccount {
                email: account.email,
                plan: account.plan,
            }),
            challenge: snapshot.challenge.map(|challenge| AgentLoginChallenge {
                operation_id: challenge.operation_id,
                verification_url: challenge.verification_url,
                user_code: challenge.user_code,
                expires_at: challenge.expires_at,
            }),
            error_code: snapshot.error.map(|error| error.code().into()),
            observed_at: Some(snapshot.observed_at),
        }
    }
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CancelAgentLoginBody {
    pub operation_id: Uuid,
}
