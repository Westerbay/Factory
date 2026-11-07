mod admission;
mod attempts;
mod repository_access;
mod sessions;
mod setup;
mod users;

#[cfg(test)]
mod tests;

use crate::{
    connection::with_connection,
    local_target::VerifiedTarget,
    schema::{self, Readiness},
};
use factory_core::identity::*;
use sqlx::{
    Connection, PgConnection, PgPool, Row,
    postgres::{PgConnectOptions, PgPoolOptions, PgRow},
};
use std::time::Duration;

#[derive(Clone)]
pub struct PgIdentityStore {
    pub(super) pool: PgPool,
}

pub struct IdentityDatabase {
    pool: PgPool,
    options: PgConnectOptions,
}

impl IdentityDatabase {
    /// A verified target is the sole connection input. No DATABASE_URL is consulted.
    pub async fn connect(target: &VerifiedTarget) -> Result<Self, IdentityError> {
        let options = connection_options(target)?;
        let pool = tokio::time::timeout(
            Duration::from_secs(5),
            PgPoolOptions::new()
                .max_connections(5)
                .acquire_timeout(Duration::from_secs(5))
                .connect_with(options.clone()),
        )
        .await
        .map_err(|_| IdentityError::Unavailable)?
        .map_err(unavailable)?;
        Ok(Self { pool, options })
    }
    pub fn store(&self) -> PgIdentityStore {
        PgIdentityStore {
            pool: self.pool.clone(),
        }
    }
    /// Prevent replacing a missing installation key while encrypted credentials exist.
    pub async fn has_repository_credentials(&self) -> Result<bool, IdentityError> {
        with_connection(
            &self.pool,
            || IdentityError::Unavailable,
            |connection| {
                Box::pin(async move {
                    sqlx::query_scalar(
                        "SELECT EXISTS(SELECT 1 FROM factory_identity.github_credentials)",
                    )
                    .fetch_one(connection)
                    .await
                    .map_err(unavailable)
                })
            },
        )
        .await
    }
    pub fn conversation_store(&self) -> crate::conversations::PgConversationStore {
        crate::conversations::PgConversationStore {
            pool: self.pool.clone(),
        }
    }
    pub fn feature_store(&self) -> crate::features::PgFeatureStore {
        crate::features::PgFeatureStore::new(self.pool.clone())
    }
    pub fn project_store(&self) -> crate::projects::PgProjectStore {
        crate::projects::PgProjectStore::new(self.pool.clone())
    }
    pub fn environment_store(&self) -> crate::environments::PgEnvironmentStore {
        crate::environments::PgEnvironmentStore {
            pool: self.pool.clone(),
        }
    }
    pub async fn environment_supervisor(
        &self,
        installation: uuid::Uuid,
    ) -> Result<
        crate::environments::PgEnvironmentSupervisor,
        factory_core::environments::EnvironmentError,
    > {
        if self.pool.is_closed() {
            return Err(factory_core::environments::EnvironmentError::Unavailable);
        }
        crate::environments::PgEnvironmentSupervisor::acquire(&self.options, installation).await
    }
    pub async fn ready(&self) -> Result<Readiness, IdentityError> {
        with_connection(
            &self.pool,
            || IdentityError::Unavailable,
            |connection| Box::pin(schema::ready(connection)),
        )
        .await
    }
    pub async fn close(&self) {
        self.pool.close().await;
    }
}

fn connection_options(target: &VerifiedTarget) -> Result<PgConnectOptions, IdentityError> {
    // SQLx otherwise inherits PostgreSQL environment settings, even with explicit
    // host/credentials. Reject their presence without exposing any value.
    for name in [
        "PGPORT",
        "PGHOSTADDR",
        "PGHOST",
        "PGUSER",
        "PGDATABASE",
        "PGPASSWORD",
        "PGSSLROOTCERT",
        "PGSSLCERT",
        "PGSSLKEY",
        "PGSSLMODE",
        "PGAPPNAME",
        "PGOPTIONS",
    ] {
        if std::env::var_os(name).is_some() {
            return Err(IdentityError::Unavailable);
        }
    }
    Ok(PgConnectOptions::new_without_pgpass()
        .host(target.host())
        .port(target.port())
        .database(target.database())
        .username(target.username())
        .password(target.password())
        .ssl_mode(sqlx::postgres::PgSslMode::Disable)
        .options([
            ("search_path", "factory_identity"),
            ("statement_timeout", "10000"),
            ("lock_timeout", "10000"),
        ]))
}

pub(super) fn unavailable(_: impl std::fmt::Display) -> IdentityError {
    IdentityError::Unavailable
}
pub(super) const USER_COLUMNS: &str =
    "u.id,u.display_name,u.role,u.status,u.version,u.access_enabled_at,e.handle,e.provider";

pub(super) fn status(status: AccessStatus) -> &'static str {
    match status {
        AccessStatus::Pending => "pending",
        AccessStatus::Active => "active",
        AccessStatus::Suspended => "suspended",
    }
}

pub(super) fn user(row: &PgRow) -> Result<User, IdentityError> {
    let role: String = row.try_get("role").map_err(unavailable)?;
    let status: String = row.try_get("status").map_err(unavailable)?;
    Ok(User {
        id: row.try_get("id").map_err(unavailable)?,
        display_name: row.try_get("display_name").map_err(unavailable)?,
        handle: row.try_get("handle").map_err(unavailable)?,
        provider: row.try_get("provider").map_err(unavailable)?,
        version: row.try_get("version").map_err(unavailable)?,
        access_enabled_at: row.try_get("access_enabled_at").map_err(unavailable)?,
        role: match role.as_str() {
            "member" => Role::Member,
            "admin" => Role::Admin,
            _ => return Err(IdentityError::Unavailable),
        },
        status: match status.as_str() {
            "pending" => AccessStatus::Pending,
            "active" => AccessStatus::Active,
            "suspended" => AccessStatus::Suspended,
            _ => return Err(IdentityError::Unavailable),
        },
    })
}

#[async_trait::async_trait]
impl IdentityStore for PgIdentityStore {
    async fn create_login_attempt(&self, attempt: NewLoginAttempt) -> Result<(), IdentityError> {
        if !(43..=128).contains(&attempt.pkce_verifier.len()) {
            return Err(IdentityError::InvalidInput);
        }
        with_connection(
            &self.pool,
            || IdentityError::Unavailable,
            |connection| Box::pin(attempts::create(connection, attempt)),
        )
        .await
    }
    async fn take_login_attempt(
        &self,
        state: CredentialHash,
        binding: CredentialHash,
    ) -> Result<LoginAttempt, IdentityError> {
        with_connection(
            &self.pool,
            || IdentityError::Unavailable,
            |connection| Box::pin(attempts::take(connection, state, binding)),
        )
        .await
    }
    async fn admit(&self, admission: LoginAdmission) -> Result<AdmissionOutcome, IdentityError> {
        admission.identity.validate()?;
        if !(32..=256).contains(&admission.session.csrf_token.len()) {
            return Err(IdentityError::InvalidInput);
        }
        with_connection(
            &self.pool,
            || IdentityError::Unavailable,
            |connection| Box::pin(admission::admit(connection, admission)),
        )
        .await
    }
    async fn active_session(
        &self,
        hash: CredentialHash,
    ) -> Result<Option<ActiveSession>, IdentityError> {
        with_connection(
            &self.pool,
            || IdentityError::Unavailable,
            |connection| Box::pin(sessions::get(connection, hash)),
        )
        .await
    }
    async fn logout(&self, hash: CredentialHash) -> Result<(), IdentityError> {
        with_connection(
            &self.pool,
            || IdentityError::Unavailable,
            |connection| {
                Box::pin(async move {
                    sqlx::query("DELETE FROM factory_identity.sessions WHERE token_hash=$1")
                        .bind(hash.0.as_slice())
                        .execute(connection)
                        .await
                        .map_err(unavailable)?;
                    Ok(())
                })
            },
        )
        .await
    }
    async fn list_users(
        &self,
        hash: CredentialHash,
        query: UserListQuery,
    ) -> Result<UserPage, IdentityError> {
        query.validate()?;
        with_connection(
            &self.pool,
            || IdentityError::Unavailable,
            |connection| Box::pin(users::list(connection, hash, query)),
        )
        .await
    }
    async fn change_access(
        &self,
        hash: CredentialHash,
        request: AccessRequest,
    ) -> Result<User, IdentityError> {
        with_connection(
            &self.pool,
            || IdentityError::Unavailable,
            |connection| Box::pin(users::change(connection, hash, request)),
        )
        .await
    }
}
