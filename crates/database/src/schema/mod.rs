use crate::identity::unavailable;
use factory_core::identity::IdentityError;
use sqlx::PgPool;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Readiness {
    Ready,
    Missing,
    Invalid,
}

pub(crate) async fn ready(pool: &PgPool) -> Result<Readiness, IdentityError> {
    let present: bool =
        sqlx::query_scalar("SELECT to_regclass('factory_identity.schema_info') IS NOT NULL")
            .fetch_one(pool)
            .await
            .map_err(unavailable)?;
    if !present {
        let exists: bool =
            sqlx::query_scalar("SELECT to_regnamespace('factory_identity') IS NOT NULL")
                .fetch_one(pool)
                .await
                .map_err(unavailable)?;
        return Ok(if exists {
            Readiness::Invalid
        } else {
            Readiness::Missing
        });
    }
    let valid: bool = sqlx::query_scalar("SELECT (SELECT count(*) = 1 AND bool_and(version = 1) FROM factory_identity.schema_info) AND to_regclass('factory_identity.users') IS NOT NULL AND to_regclass('factory_identity.environments') IS NOT NULL AND to_regclass('factory_identity.conversations') IS NOT NULL AND to_regclass('factory_identity.model_probe_leases') IS NOT NULL AND to_regclass('public.runs') IS NOT NULL")
        .fetch_one(pool).await.map_err(unavailable)?;
    Ok(if valid {
        Readiness::Ready
    } else {
        Readiness::Invalid
    })
}
