use chrono::{DateTime, Utc};
use factory_core::identity as domain;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum UserRole {
    Member,
    Admin,
}

impl From<domain::Role> for UserRole {
    fn from(role: domain::Role) -> Self {
        match role {
            domain::Role::Member => Self::Member,
            domain::Role::Admin => Self::Admin,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum UserStatus {
    Pending,
    Active,
    Suspended,
}

impl From<domain::AccessStatus> for UserStatus {
    fn from(status: domain::AccessStatus) -> Self {
        match status {
            domain::AccessStatus::Pending => Self::Pending,
            domain::AccessStatus::Active => Self::Active,
            domain::AccessStatus::Suspended => Self::Suspended,
        }
    }
}

impl From<UserStatus> for domain::AccessStatus {
    fn from(status: UserStatus) -> Self {
        match status {
            UserStatus::Pending => Self::Pending,
            UserStatus::Active => Self::Active,
            UserStatus::Suspended => Self::Suspended,
        }
    }
}

#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SessionUser {
    pub id: Uuid,
    pub display_name: String,
    pub handle: String,
    pub role: UserRole,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SessionResponse {
    pub user: SessionUser,
    pub expires_at: DateTime<Utc>,
    pub csrf_token: String,
}

impl From<domain::ActiveSession> for SessionResponse {
    fn from(session: domain::ActiveSession) -> Self {
        Self {
            user: SessionUser {
                id: session.user.id,
                display_name: session.user.display_name,
                handle: session.user.handle,
                role: session.user.role.into(),
            },
            expires_at: session.expires_at,
            csrf_token: session.csrf_token,
        }
    }
}

#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AdminUser {
    pub id: Uuid,
    pub display_name: String,
    pub handle: String,
    pub provider: String,
    pub role: UserRole,
    pub status: UserStatus,
    pub version: i64,
}

impl From<domain::User> for AdminUser {
    fn from(user: domain::User) -> Self {
        Self {
            id: user.id,
            display_name: user.display_name,
            handle: user.handle,
            provider: user.provider,
            role: user.role.into(),
            status: user.status.into(),
            version: user.version,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UsersResponse {
    pub items: Vec<AdminUser>,
    pub next_cursor: Option<Uuid>,
}

impl From<domain::UserPage> for UsersResponse {
    fn from(page: domain::UserPage) -> Self {
        Self {
            items: page.items.into_iter().map(Into::into).collect(),
            next_cursor: page.next_cursor,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum AccessAction {
    Active,
    Suspended,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AccessBody {
    pub status: AccessAction,
    #[schema(minimum = 1)]
    pub expected_version: i64,
}

impl AccessBody {
    pub fn into_request(
        self,
        target_id: Uuid,
    ) -> Result<domain::AccessRequest, domain::IdentityError> {
        if self.expected_version < 1 {
            return Err(domain::IdentityError::InvalidInput);
        }
        Ok(domain::AccessRequest {
            target_id,
            status: match self.status {
                AccessAction::Active => domain::AccessDecision::Active,
                AccessAction::Suspended => domain::AccessDecision::Suspended,
            },
            expected_version: self.expected_version,
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsersQuery {
    pub status: UserStatus,
    pub cursor: Option<Uuid>,
    #[serde(default = "default_limit")]
    pub limit: u8,
}

fn default_limit() -> u8 {
    20
}

impl UsersQuery {
    pub fn into_domain(self) -> Result<domain::UserListQuery, domain::IdentityError> {
        let query = domain::UserListQuery {
            status: self.status.into(),
            after: self.cursor,
            limit: self.limit,
        };
        query.validate()?;
        Ok(query)
    }
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ApiError {
    pub code: String,
    pub message: String,
    pub request_id: Uuid,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_cannot_choose_actor_role_or_invalid_version() {
        for input in [
            r#"{"status":"active","expectedVersion":1,"role":"admin"}"#,
            r#"{"status":"active","expectedVersion":1,"actor":"forged"}"#,
            r#"{"status":"pending","expectedVersion":1}"#,
            r#"{"status":"active","expectedVersion":1.5}"#,
        ] {
            assert!(serde_json::from_str::<AccessBody>(input).is_err());
        }
        let body = serde_json::from_str::<AccessBody>(r#"{"status":"active","expectedVersion":0}"#)
            .unwrap();
        assert!(body.into_request(Uuid::nil()).is_err());
    }

    #[test]
    fn user_listing_limits_are_bounded_and_cursor_is_typed() {
        for limit in [0, 51, 255] {
            assert!(
                UsersQuery {
                    status: UserStatus::Pending,
                    cursor: None,
                    limit
                }
                .into_domain()
                .is_err()
            );
        }
        assert!(
            serde_json::from_str::<UsersQuery>(r#"{"status":"pending","cursor":"not-a-uuid"}"#)
                .is_err()
        );
        let query = serde_json::from_str::<UsersQuery>(r#"{"status":"pending"}"#)
            .unwrap()
            .into_domain()
            .unwrap();
        assert_eq!(query.limit, 20);
    }
}
