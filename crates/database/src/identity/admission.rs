use super::*;
use uuid::Uuid;

pub(super) async fn admit(
    connection: &mut PgConnection,
    admission: LoginAdmission,
) -> Result<AdmissionOutcome, IdentityError> {
    let mut tx = connection.begin().await.map_err(unavailable)?;
    let user = upsert_identity(&mut tx, &admission.identity).await?;
    let id = user.id;
    let outcome = match admission_decision(&user, admission.login_started_at) {
        AdmissionDecision::Pending => AdmissionOutcome::Pending,
        AdmissionDecision::Suspended => AdmissionOutcome::Suspended,
        AdmissionDecision::StaleLogin => AdmissionOutcome::StaleLogin,
        AdmissionDecision::Active => {
            if let Some(previous) = admission.session.previous_token_hash {
                sqlx::query("DELETE FROM factory_identity.sessions WHERE token_hash=$1")
                    .bind(previous.0.as_slice())
                    .execute(&mut *tx)
                    .await
                    .map_err(unavailable)?;
            }
            sqlx::query("DELETE FROM factory_identity.sessions WHERE token_hash IN (SELECT token_hash FROM factory_identity.sessions WHERE expires_at <= clock_timestamp() LIMIT 100 FOR UPDATE SKIP LOCKED)")
                .execute(&mut *tx).await.map_err(unavailable)?;
            let expires_at = sqlx::query_scalar("INSERT INTO factory_identity.sessions(token_hash,user_id,csrf_token,expires_at) VALUES($1,$2,$3,clock_timestamp()+make_interval(secs => $4)) RETURNING expires_at")
                .bind(admission.session.token_hash.0.as_slice()).bind(id).bind(&admission.session.csrf_token).bind(SESSION_TTL_SECONDS as f64)
                .fetch_one(&mut *tx).await.map_err(unavailable)?;
            AdmissionOutcome::Active(ActiveSession {
                user,
                csrf_token: admission.session.csrf_token,
                expires_at,
            })
        }
    };
    tx.commit().await.map_err(unavailable)?;
    Ok(outcome)
}

/// Share the identity lock with operator setup so a concurrent first login cannot
/// duplicate the account or overwrite the operator's role/status decision.
pub(super) async fn upsert_identity(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    identity: &VerifiedIdentity,
) -> Result<User, IdentityError> {
    // Serialize creation of the same external identity without exposing provider-specific policy.
    let key = serde_json::to_string(&[&identity.provider, &identity.issuer, &identity.subject])
        .map_err(unavailable)?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 813357))")
        .bind(key)
        .execute(&mut **tx)
        .await
        .map_err(unavailable)?;
    let sql = format!(
        "SELECT {USER_COLUMNS} FROM factory_identity.users u JOIN factory_identity.external_identities e ON e.user_id=u.id WHERE e.provider=$1 AND e.issuer=$2 AND e.subject=$3 FOR UPDATE OF u"
    );
    let existing = sqlx::query(&sql)
        .bind(&identity.provider)
        .bind(&identity.issuer)
        .bind(&identity.subject)
        .fetch_optional(&mut **tx)
        .await
        .map_err(unavailable)?;
    let id = existing
        .as_ref()
        .map(user)
        .transpose()?
        .map(|u| u.id)
        .unwrap_or_else(Uuid::new_v4);
    if existing.is_none() {
        sqlx::query("INSERT INTO factory_identity.users(id,display_name,role,status) VALUES($1,$2,'member','pending')")
            .bind(id).bind(&identity.display_name).execute(&mut **tx).await.map_err(unavailable)?;
        sqlx::query("INSERT INTO factory_identity.external_identities(user_id,provider,issuer,subject,handle) VALUES($1,$2,$3,$4,$5)")
            .bind(id).bind(&identity.provider).bind(&identity.issuer).bind(&identity.subject).bind(&identity.handle)
            .execute(&mut **tx).await.map_err(unavailable)?;
    } else {
        sqlx::query("UPDATE factory_identity.users SET display_name=$2,updated_at=clock_timestamp() WHERE id=$1")
            .bind(id).bind(&identity.display_name).execute(&mut **tx).await.map_err(unavailable)?;
        sqlx::query("UPDATE factory_identity.external_identities SET handle=$2 WHERE user_id=$1")
            .bind(id)
            .bind(&identity.handle)
            .execute(&mut **tx)
            .await
            .map_err(unavailable)?;
    }
    let row = sqlx::query(&format!("SELECT {USER_COLUMNS} FROM factory_identity.users u JOIN factory_identity.external_identities e ON e.user_id=u.id WHERE u.id=$1"))
        .bind(id).fetch_one(&mut **tx).await.map_err(unavailable)?;
    user(&row)
}
