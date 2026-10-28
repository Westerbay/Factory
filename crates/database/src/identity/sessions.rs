use super::*;

pub(super) async fn get<'a>(
    executor: impl sqlx::Executor<'a, Database = sqlx::Postgres>,
    hash: CredentialHash,
) -> Result<Option<ActiveSession>, IdentityError> {
    let sql = format!(
        "SELECT {USER_COLUMNS},s.csrf_token,s.expires_at FROM factory_identity.sessions s JOIN factory_identity.users u ON u.id=s.user_id JOIN factory_identity.external_identities e ON e.user_id=u.id WHERE s.token_hash=$1 AND s.expires_at > clock_timestamp() AND u.status='active'"
    );
    sqlx::query(&sql)
        .bind(hash.0.as_slice())
        .fetch_optional(executor)
        .await
        .map_err(unavailable)?
        .map(|row| {
            Ok(ActiveSession {
                user: user(&row)?,
                csrf_token: row.try_get("csrf_token").map_err(unavailable)?,
                expires_at: row.try_get("expires_at").map_err(unavailable)?,
            })
        })
        .transpose()
}

pub(super) async fn admin(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    hash: CredentialHash,
) -> Result<User, IdentityError> {
    let session = get(&mut **tx, hash)
        .await?
        .ok_or(IdentityError::Unauthorized)?;
    require_admin(&session.user)?;
    Ok(session.user)
}
