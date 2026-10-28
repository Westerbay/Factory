use crate::identity::unavailable;
use factory_core::identity::IdentityError;
use sqlx::{Connection, PgConnection, PgPool, Row, migrate::Migrator, postgres::PgConnectOptions};

static MIGRATOR: Migrator = sqlx::migrate!("./migrations");
const INITIALIZATION_LOCK: i64 = 0x464143544d4947;
const TABLES: &[&str] = &[
    "_sqlx_migrations",
    "access_events",
    "environments",
    "external_identities",
    "login_attempts",
    "sessions",
    "users",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Readiness {
    Ready,
    Missing,
    Pending,
    Invalid,
}

pub(crate) async fn ready(pool: &PgPool) -> Result<Readiness, IdentityError> {
    let mut connection = pool.acquire().await.map_err(unavailable)?;
    inspect(&mut connection).await
}

async fn inspect(connection: &mut PgConnection) -> Result<Readiness, IdentityError> {
    let schema: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM pg_catalog.pg_namespace WHERE nspname='factory_identity')",
    )
    .fetch_one(&mut *connection)
    .await
    .map_err(unavailable)?;
    if !schema {
        return Ok(Readiness::Missing);
    }
    let tables: Vec<String> = sqlx::query_scalar("SELECT c.relname::text FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='factory_identity' AND c.relkind IN ('r','p','v','m','f','S') ORDER BY c.relname")
        .fetch_all(&mut *connection).await.map_err(unavailable)?;
    if !tables.iter().any(|t| t == "_sqlx_migrations") {
        let objects: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_catalog.pg_depend d JOIN pg_catalog.pg_namespace n ON n.oid=d.refobjid WHERE d.refclassid='pg_catalog.pg_namespace'::regclass AND n.nspname='factory_identity')")
            .fetch_one(&mut *connection).await.map_err(unavailable)?;
        return Ok(if !objects {
            Readiness::Pending
        } else {
            Readiness::Invalid
        });
    }
    let ledger = sqlx::query(
        "SELECT version,success,checksum FROM factory_identity._sqlx_migrations ORDER BY version",
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(unavailable)?;
    for row in &ledger {
        let version: i64 = row.try_get("version").map_err(unavailable)?;
        let success: bool = row.try_get("success").map_err(unavailable)?;
        let checksum: Vec<u8> = row.try_get("checksum").map_err(unavailable)?;
        let expected = MIGRATOR
            .iter()
            .find(|migration| migration.version == version);
        if !success
            || expected.is_none_or(|migration| migration.checksum.as_ref() != checksum.as_slice())
        {
            return Ok(Readiness::Invalid);
        }
    }
    if ledger.len() != MIGRATOR.iter().count() {
        if ledger.is_empty() && tables.len() != 1 {
            return Ok(Readiness::Invalid);
        }
        return Ok(Readiness::Pending);
    }
    if tables.iter().map(String::as_str).collect::<Vec<_>>() != TABLES {
        return Ok(Readiness::Invalid);
    }
    Ok(Readiness::Ready)
}

pub(crate) async fn migrate(options: &PgConnectOptions) -> Result<(), IdentityError> {
    let mut connection = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        PgConnection::connect_with(options),
    )
    .await
    .map_err(|_| IdentityError::Unavailable)?
    .map_err(unavailable)?;
    let result = apply(&mut connection).await;
    // Closing this dedicated connection releases both the initialization and SQLx locks,
    // including every error path. They must never leak back into the runtime pool.
    let closed = connection.close().await.map_err(unavailable);
    result.and(closed)
}

async fn apply(connection: &mut PgConnection) -> Result<(), IdentityError> {
    sqlx::query("SELECT pg_advisory_lock($1)")
        .bind(INITIALIZATION_LOCK)
        .execute(&mut *connection)
        .await
        .map_err(unavailable)?;
    if inspect(connection).await? == Readiness::Invalid {
        return Err(IdentityError::Unavailable);
    }
    sqlx::query("CREATE SCHEMA IF NOT EXISTS factory_identity")
        .execute(&mut *connection)
        .await
        .map_err(unavailable)?;
    MIGRATOR.run(&mut *connection).await.map_err(unavailable)?;
    if inspect(connection).await? != Readiness::Ready {
        return Err(IdentityError::Unavailable);
    }
    Ok(())
}
