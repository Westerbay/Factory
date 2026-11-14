use crate::identity::unavailable;
use factory_core::identity::IdentityError;
use sqlx::PgConnection;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Readiness {
    Ready,
    Missing,
    Invalid,
}

pub(crate) async fn ready(connection: &mut PgConnection) -> Result<Readiness, IdentityError> {
    compatible(connection, false).await
}

pub(crate) async fn compatible(
    connection: &mut PgConnection,
    allow_previous: bool,
) -> Result<Readiness, IdentityError> {
    let present: bool =
        sqlx::query_scalar("SELECT to_regclass('factory_identity.schema_info') IS NOT NULL")
            .fetch_one(&mut *connection)
            .await
            .map_err(unavailable)?;
    if !present {
        let exists: bool =
            sqlx::query_scalar("SELECT to_regnamespace('factory_identity') IS NOT NULL")
                .fetch_one(&mut *connection)
                .await
                .map_err(unavailable)?;
        return Ok(if exists {
            Readiness::Invalid
        } else {
            Readiness::Missing
        });
    }
    let valid: bool = sqlx::query_scalar(
        "SELECT (SELECT count(*) = 1 AND bool_and(version = 6 OR ($1 AND version = 5)) FROM factory_identity.schema_info)
         AND to_regclass('factory_identity.users') IS NOT NULL
         AND to_regclass('factory_identity.environments') IS NOT NULL
         AND to_regclass('factory_identity.conversations') IS NOT NULL
         AND to_regclass('factory_identity.projects') IS NOT NULL
         AND to_regclass('factory_identity.github_authorization_epochs') IS NOT NULL
         AND to_regclass('factory_identity.github_credentials') IS NOT NULL
         AND to_regclass('factory_identity.github_authorization_attempts') IS NOT NULL
         AND EXISTS (SELECT 1 FROM information_schema.columns
             WHERE table_schema='factory_identity' AND table_name='conversations' AND column_name='project_id')
         AND EXISTS (SELECT 1 FROM information_schema.columns
             WHERE table_schema='factory_identity' AND table_name='projects' AND column_name='cloned_full_name')
         AND EXISTS (SELECT 1 FROM information_schema.columns
             WHERE table_schema='factory_identity' AND table_name='github_credentials' AND column_name='authorization_epoch')
         AND EXISTS (SELECT 1 FROM information_schema.columns
             WHERE table_schema='factory_identity' AND table_name='github_authorization_attempts' AND column_name='authorization_epoch')
         AND to_regclass('factory_identity.model_probe_leases') IS NOT NULL
         AND to_regclass('factory_identity.features') IS NOT NULL
         AND to_regclass('factory_identity.feature_revisions') IS NOT NULL
         AND to_regclass('factory_identity.feature_attempts') IS NOT NULL
         AND to_regclass('factory_identity.feature_reviews') IS NOT NULL
         AND to_regclass('factory_identity.feature_deliveries') IS NOT NULL
         AND to_regclass('factory_identity.feature_operations') IS NOT NULL
         AND (to_regclass('factory_identity.conversation_attachments') IS NOT NULL OR
              ($1 AND (SELECT version = 5 FROM factory_identity.schema_info LIMIT 1)))
         AND to_regclass('public.runs') IS NOT NULL",
    )
    .bind(allow_previous)
    .fetch_one(&mut *connection)
    .await
    .map_err(unavailable)?;
    Ok(if valid {
        Readiness::Ready
    } else {
        Readiness::Invalid
    })
}
