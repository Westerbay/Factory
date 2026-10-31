use super::*;

pub(super) async fn create(
    connection: &mut PgConnection,
    attempt: NewLoginAttempt,
) -> Result<(), IdentityError> {
    let mut tx = connection.begin().await.map_err(unavailable)?;
    sqlx::query("DELETE FROM factory_identity.login_attempts WHERE state_hash IN (SELECT state_hash FROM factory_identity.login_attempts WHERE expires_at <= clock_timestamp() LIMIT 100 FOR UPDATE SKIP LOCKED)")
        .execute(&mut *tx).await.map_err(unavailable)?;
    sqlx::query("INSERT INTO factory_identity.login_attempts(state_hash,binding_hash,pkce_verifier,expires_at) VALUES($1,$2,$3,clock_timestamp()+make_interval(secs => $4))")
        .bind(attempt.state_hash.0.as_slice()).bind(attempt.binding_hash.0.as_slice()).bind(attempt.pkce_verifier)
        .bind(LOGIN_ATTEMPT_TTL_SECONDS as f64).execute(&mut *tx).await.map_err(unavailable)?;
    tx.commit().await.map_err(unavailable)
}

pub(super) async fn take(
    connection: &mut PgConnection,
    state: CredentialHash,
    binding: CredentialHash,
) -> Result<LoginAttempt, IdentityError> {
    let row = sqlx::query("DELETE FROM factory_identity.login_attempts WHERE state_hash=$1 AND binding_hash=$2 AND expires_at > clock_timestamp() RETURNING pkce_verifier,created_at")
        .bind(state.0.as_slice()).bind(binding.0.as_slice()).fetch_optional(connection).await.map_err(unavailable)?.ok_or(IdentityError::InvalidLogin)?;
    Ok(LoginAttempt {
        pkce_verifier: row.try_get("pkce_verifier").map_err(unavailable)?,
        created_at: row.try_get("created_at").map_err(unavailable)?,
    })
}
