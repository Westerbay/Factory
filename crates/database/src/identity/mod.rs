mod admission;
mod attempts;
mod sessions;
mod setup;
mod users;

use crate::{
    local_target::VerifiedTarget,
    migrations::{self, Readiness},
};
use factory_core::identity::*;
use sqlx::{
    PgPool, Row,
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
    pub async fn ready(&self) -> Result<Readiness, IdentityError> {
        migrations::ready(&self.pool).await
    }
    pub async fn migrate(&self) -> Result<(), IdentityError> {
        if self.pool.is_closed() {
            return Err(IdentityError::Unavailable);
        }
        migrations::migrate(&self.options).await
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
        attempts::create(&self.pool, attempt).await
    }
    async fn take_login_attempt(
        &self,
        state: CredentialHash,
        binding: CredentialHash,
    ) -> Result<LoginAttempt, IdentityError> {
        attempts::take(&self.pool, state, binding).await
    }
    async fn admit(&self, admission: LoginAdmission) -> Result<AdmissionOutcome, IdentityError> {
        admission::admit(&self.pool, admission).await
    }
    async fn active_session(
        &self,
        hash: CredentialHash,
    ) -> Result<Option<ActiveSession>, IdentityError> {
        sessions::get(&self.pool, hash).await
    }
    async fn logout(&self, hash: CredentialHash) -> Result<(), IdentityError> {
        sqlx::query("DELETE FROM factory_identity.sessions WHERE token_hash=$1")
            .bind(hash.0.as_slice())
            .execute(&self.pool)
            .await
            .map_err(unavailable)?;
        Ok(())
    }
    async fn list_users(
        &self,
        hash: CredentialHash,
        query: UserListQuery,
    ) -> Result<UserPage, IdentityError> {
        users::list(&self.pool, hash, query).await
    }
    async fn change_access(
        &self,
        hash: CredentialHash,
        request: AccessRequest,
    ) -> Result<User, IdentityError> {
        users::change(&self.pool, hash, request).await
    }
}
