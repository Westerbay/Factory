use super::*;
use factory_core::environments::EnvironmentActor;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

type AccessError = RepositoryAccessError;

fn unavailable(_: impl std::fmt::Display) -> AccessError {
    AccessError::Unavailable
}

async fn authorize(
    tx: &mut Transaction<'_, Postgres>,
    actor: RepositoryActor,
) -> Result<(), AccessError> {
    let status: Option<String> =
        sqlx::query_scalar("SELECT status FROM factory_identity.users WHERE id=$1 FOR UPDATE")
            .bind(actor.actor.user_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(unavailable)?;
    if status.as_deref() != Some("active") {
        return Err(AccessError::Unauthorized);
    }
    // Session and environment observations remain stable until this transaction
    // finishes; logout's DELETE and supervisor updates must serialize with us.
    let session: Option<Uuid> = sqlx::query_scalar("SELECT user_id FROM factory_identity.sessions WHERE user_id=$1 AND token_hash=$2 AND expires_at>clock_timestamp() FOR UPDATE")
        .bind(actor.actor.user_id).bind(actor.actor.session_hash.0.as_slice()).fetch_optional(&mut **tx).await.map_err(unavailable)?;
    if session.is_none() {
        return Err(AccessError::Unauthorized);
    }
    let env: Option<Uuid> = sqlx::query_scalar("SELECT id FROM factory_identity.environments WHERE id=$1 AND user_id=$2 AND generation=$3 AND desired_state='running' AND observed_state='running' AND error_code IS NULL FOR UPDATE")
        .bind(actor.environment_id).bind(actor.actor.user_id).bind(actor.generation).fetch_optional(&mut **tx).await.map_err(unavailable)?;
    if env.is_none() {
        return Err(AccessError::Conflict);
    }
    Ok(())
}

async fn check_epoch(
    tx: &mut Transaction<'_, Postgres>,
    user: Uuid,
    expected: i64,
) -> Result<(), AccessError> {
    let current: Option<i64> = sqlx::query_scalar(
        "SELECT epoch FROM factory_identity.github_authorization_epochs WHERE user_id=$1",
    )
    .bind(user)
    .fetch_optional(&mut **tx)
    .await
    .map_err(unavailable)?;
    if expected > 0 && current == Some(expected) {
        Ok(())
    } else {
        Err(AccessError::ProviderUnauthorized)
    }
}

async fn identity_matches(
    tx: &mut Transaction<'_, Postgres>,
    actor: RepositoryActor,
    identity: &VerifiedIdentity,
) -> Result<(), AccessError> {
    let matches: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM factory_identity.external_identities WHERE user_id=$1 AND provider=$2 AND issuer=$3 AND subject=$4)")
        .bind(actor.actor.user_id).bind(&identity.provider).bind(&identity.issuer).bind(&identity.subject).fetch_one(&mut **tx).await.map_err(unavailable)?;
    if matches {
        Ok(())
    } else {
        Err(AccessError::IdentityMismatch)
    }
}

fn valid_actor(actor: RepositoryActor) -> bool {
    !actor.actor.user_id.is_nil() && !actor.environment_id.is_nil() && actor.generation > 0
}
fn valid_text(value: &str, max: usize) -> bool {
    !value.trim().is_empty() && value.len() <= max && !value.chars().any(char::is_control)
}

#[async_trait::async_trait]
impl RepositoryAccessStore for PgIdentityStore {
    async fn login_credential_epoch(&self, actor: EnvironmentActor) -> Result<i64, AccessError> {
        with_connection(&self.pool, || AccessError::Unavailable, |connection| Box::pin(async move {
            sqlx::query_scalar("SELECT COALESCE(e.epoch,0) FROM factory_identity.sessions s JOIN factory_identity.users u ON u.id=s.user_id LEFT JOIN factory_identity.github_authorization_epochs e ON e.user_id=u.id WHERE s.user_id=$1 AND s.token_hash=$2 AND s.expires_at>clock_timestamp() AND u.status='active'")
                .bind(actor.user_id).bind(actor.session_hash.0.as_slice()).fetch_optional(connection).await.map_err(unavailable)?.ok_or(AccessError::Unauthorized)
        })).await
    }
    async fn save_login_credential(
        &self,
        actor: EnvironmentActor,
        identity: &VerifiedIdentity,
        credential: RepositoryCredential,
    ) -> Result<(), AccessError> {
        let subject: i64 = credential
            .subject
            .parse()
            .map_err(|_| AccessError::InvalidInput)?;
        if actor.user_id.is_nil()
            || identity.validate().is_err()
            || identity.provider != "github"
            || identity.issuer != "https://github.com"
            || subject <= 0
            || credential.subject != identity.subject
            || credential.login != identity.handle
            || !valid_text(&credential.login, 128)
            || !valid_text(&credential.email, 320)
            || credential.email.len() < 3
            || !valid_text(&credential.encrypted.key_id, 128)
            || credential.encrypted.nonce.len() != 12
            || !(16..=8192).contains(&credential.encrypted.ciphertext.len())
        {
            return Err(AccessError::InvalidInput);
        }
        let identity = identity.clone();
        with_connection(&self.pool, || AccessError::Unavailable, |connection| Box::pin(async move {
            let mut tx = connection.begin().await.map_err(unavailable)?;
            let active: Option<String> = sqlx::query_scalar("SELECT status FROM factory_identity.users WHERE id=$1 FOR UPDATE")
                .bind(actor.user_id).fetch_optional(&mut *tx).await.map_err(unavailable)?;
            if active.as_deref() != Some("active") { return Err(AccessError::Unauthorized); }
            let session: Option<Uuid> = sqlx::query_scalar("SELECT user_id FROM factory_identity.sessions WHERE user_id=$1 AND token_hash=$2 AND expires_at>clock_timestamp() FOR UPDATE")
                .bind(actor.user_id).bind(actor.session_hash.0.as_slice()).fetch_optional(&mut *tx).await.map_err(unavailable)?;
            if session.is_none() { return Err(AccessError::Unauthorized); }
            let matches: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM factory_identity.external_identities WHERE user_id=$1 AND provider=$2 AND issuer=$3 AND subject=$4)")
                .bind(actor.user_id).bind(&identity.provider).bind(&identity.issuer).bind(&identity.subject).fetch_one(&mut *tx).await.map_err(unavailable)?;
            if !matches { return Err(AccessError::IdentityMismatch); }
            let current: Option<i64> = sqlx::query_scalar("SELECT epoch FROM factory_identity.github_authorization_epochs WHERE user_id=$1")
                .bind(actor.user_id).fetch_optional(&mut *tx).await.map_err(unavailable)?;
            if current.unwrap_or(0) != credential.authorization_epoch { return Err(AccessError::ProviderUnauthorized); }
            let epoch: i64 = sqlx::query_scalar("INSERT INTO factory_identity.github_authorization_epochs(user_id,epoch) VALUES($1,1) ON CONFLICT(user_id) DO UPDATE SET epoch=factory_identity.github_authorization_epochs.epoch+1 RETURNING epoch")
                .bind(actor.user_id).fetch_one(&mut *tx).await.map_err(unavailable)?;
            sqlx::query("INSERT INTO factory_identity.github_credentials(user_id,github_user_id,authorization_epoch,github_login,primary_email,key_id,nonce,ciphertext) VALUES($1,$2,$3,$4,$5,$6,$7,$8) ON CONFLICT(user_id) DO UPDATE SET github_user_id=EXCLUDED.github_user_id,authorization_epoch=EXCLUDED.authorization_epoch,github_login=EXCLUDED.github_login,primary_email=EXCLUDED.primary_email,key_id=EXCLUDED.key_id,nonce=EXCLUDED.nonce,ciphertext=EXCLUDED.ciphertext,updated_at=clock_timestamp()")
                .bind(actor.user_id).bind(subject).bind(epoch).bind(credential.login).bind(credential.email).bind(credential.encrypted.key_id).bind(credential.encrypted.nonce).bind(credential.encrypted.ciphertext).execute(&mut *tx).await.map_err(unavailable)?;
            tx.commit().await.map_err(unavailable)
        })).await
    }
    async fn begin_repository_authorization(
        &self,
        attempt: RepositoryAuthorizationAttempt,
    ) -> Result<(), AccessError> {
        if !valid_actor(attempt.actor)
            || !(43..=128).contains(&attempt.verifier.len())
            || !attempt
                .verifier
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"-._~".contains(&c))
        {
            return Err(AccessError::InvalidInput);
        }
        with_connection(&self.pool, || AccessError::Unavailable, |connection| Box::pin(async move {
            let mut tx = connection.begin().await.map_err(unavailable)?;
            authorize(&mut tx, attempt.actor).await?;
            sqlx::query("DELETE FROM factory_identity.github_authorization_attempts WHERE state_hash IN (SELECT state_hash FROM factory_identity.github_authorization_attempts WHERE expires_at<=clock_timestamp() LIMIT 100 FOR UPDATE SKIP LOCKED)")
                .execute(&mut *tx).await.map_err(unavailable)?;
            sqlx::query("INSERT INTO factory_identity.github_authorization_epochs(user_id,epoch) VALUES($1,1) ON CONFLICT (user_id) DO NOTHING").bind(attempt.actor.actor.user_id).execute(&mut *tx).await.map_err(unavailable)?;
            let epoch: i64 = sqlx::query_scalar("SELECT epoch FROM factory_identity.github_authorization_epochs WHERE user_id=$1").bind(attempt.actor.actor.user_id).fetch_one(&mut *tx).await.map_err(unavailable)?;
            sqlx::query("INSERT INTO factory_identity.github_authorization_attempts(state_hash,binding_hash,pkce_verifier,user_id,session_hash,environment_id,environment_generation,authorization_epoch,expires_at) VALUES($1,$2,$3,$4,$5,$6,$7,$8,clock_timestamp()+make_interval(secs => $9))")
                .bind(attempt.state_hash.0.as_slice()).bind(attempt.binding_hash.0.as_slice()).bind(attempt.verifier).bind(attempt.actor.actor.user_id)
                .bind(attempt.actor.actor.session_hash.0.as_slice()).bind(attempt.actor.environment_id).bind(attempt.actor.generation).bind(epoch).bind(LOGIN_ATTEMPT_TTL_SECONDS as f64)
                .execute(&mut *tx).await.map_err(unavailable)?;
            tx.commit().await.map_err(unavailable)
        })).await
    }

    async fn take_repository_authorization(
        &self,
        state: CredentialHash,
        binding: CredentialHash,
        session: CredentialHash,
    ) -> Result<RepositoryAuthorizationAttempt, AccessError> {
        with_connection(&self.pool, || AccessError::Unavailable, |connection| Box::pin(async move {
            let mut tx = connection.begin().await.map_err(unavailable)?;
            // Inspect the owner without taking a row lock, then lock the user first,
            // matching suspension and environment operations. The final DELETE is
            // still conditional and atomic, so a concurrent callback can win once.
            let row = sqlx::query("SELECT user_id,environment_id,environment_generation,authorization_epoch FROM factory_identity.github_authorization_attempts WHERE state_hash=$1 AND binding_hash=$2 AND session_hash=$3 AND expires_at>clock_timestamp()")
                .bind(state.0.as_slice()).bind(binding.0.as_slice()).bind(session.0.as_slice()).fetch_optional(&mut *tx).await.map_err(unavailable)?.ok_or(AccessError::InvalidLogin)?;
            let actor = RepositoryActor { actor: EnvironmentActor { user_id: row.try_get("user_id").map_err(unavailable)?, session_hash: session }, environment_id: row.try_get("environment_id").map_err(unavailable)?, generation: row.try_get("environment_generation").map_err(unavailable)? };
            authorize(&mut tx, actor).await?;
            let epoch: i64 = row.try_get("authorization_epoch").map_err(unavailable)?;
            check_epoch(&mut tx, actor.actor.user_id, epoch).await?;
            let verifier: String = sqlx::query_scalar("DELETE FROM factory_identity.github_authorization_attempts WHERE state_hash=$1 AND binding_hash=$2 AND session_hash=$3 AND expires_at>clock_timestamp() RETURNING pkce_verifier")
                .bind(state.0.as_slice()).bind(binding.0.as_slice()).bind(session.0.as_slice()).fetch_optional(&mut *tx).await.map_err(unavailable)?.ok_or(AccessError::InvalidLogin)?;
            tx.commit().await.map_err(unavailable)?;
            Ok(RepositoryAuthorizationAttempt { state_hash: state, binding_hash: binding, verifier, actor, authorization_epoch: epoch })
        })).await
    }

    async fn validate_repository_epoch(
        &self,
        actor: RepositoryActor,
        epoch: i64,
    ) -> Result<(), AccessError> {
        if !valid_actor(actor) {
            return Err(AccessError::InvalidInput);
        }
        with_connection(
            &self.pool,
            || AccessError::Unavailable,
            |connection| {
                Box::pin(async move {
                    let mut tx = connection.begin().await.map_err(unavailable)?;
                    authorize(&mut tx, actor).await?;
                    check_epoch(&mut tx, actor.actor.user_id, epoch).await?;
                    tx.commit().await.map_err(unavailable)
                })
            },
        )
        .await
    }

    async fn verify_repository_identity(
        &self,
        actor: RepositoryActor,
        identity: &VerifiedIdentity,
        epoch: i64,
    ) -> Result<(), AccessError> {
        if !valid_actor(actor)
            || identity.validate().is_err()
            || identity.provider != "github"
            || identity.issuer != "https://github.com"
        {
            return Err(AccessError::InvalidInput);
        }
        let identity = identity.clone();
        with_connection(
            &self.pool,
            || AccessError::Unavailable,
            |connection| {
                Box::pin(async move {
                    let mut tx = connection.begin().await.map_err(unavailable)?;
                    authorize(&mut tx, actor).await?;
                    check_epoch(&mut tx, actor.actor.user_id, epoch).await?;
                    identity_matches(&mut tx, actor, &identity).await?;
                    tx.commit().await.map_err(unavailable)
                })
            },
        )
        .await
    }

    async fn save_repository_credential(
        &self,
        actor: RepositoryActor,
        identity: &VerifiedIdentity,
        credential: RepositoryCredential,
    ) -> Result<(), AccessError> {
        let subject: i64 = credential
            .subject
            .parse()
            .map_err(|_| AccessError::InvalidInput)?;
        if !valid_actor(actor)
            || identity.validate().is_err()
            || identity.provider != "github"
            || identity.issuer != "https://github.com"
            || subject <= 0
            || credential.authorization_epoch <= 0
            || credential.subject != identity.subject
            || credential.login != identity.handle
            || !valid_text(&credential.login, 128)
            || !valid_text(&credential.email, 320)
            || credential.email.len() < 3
            || !valid_text(&credential.encrypted.key_id, 128)
            || credential.encrypted.nonce.len() != 12
            || !(16..=8192).contains(&credential.encrypted.ciphertext.len())
        {
            return Err(AccessError::InvalidInput);
        }
        let identity = identity.clone();
        with_connection(&self.pool, || AccessError::Unavailable, |connection| Box::pin(async move {
            let mut tx = connection.begin().await.map_err(unavailable)?;
            authorize(&mut tx, actor).await?;
            identity_matches(&mut tx, actor, &identity).await?;
            let next: Option<i64> = sqlx::query_scalar("UPDATE factory_identity.github_authorization_epochs SET epoch=epoch+1 WHERE user_id=$1 AND epoch=$2 RETURNING epoch")
                .bind(actor.actor.user_id).bind(credential.authorization_epoch).fetch_optional(&mut *tx).await.map_err(unavailable)?;
            let next = next.ok_or(AccessError::ProviderUnauthorized)?;
            sqlx::query("INSERT INTO factory_identity.github_credentials(user_id,github_user_id,authorization_epoch,github_login,primary_email,key_id,nonce,ciphertext) VALUES($1,$2,$3,$4,$5,$6,$7,$8) ON CONFLICT (user_id) DO UPDATE SET github_user_id=EXCLUDED.github_user_id,authorization_epoch=EXCLUDED.authorization_epoch,github_login=EXCLUDED.github_login,primary_email=EXCLUDED.primary_email,key_id=EXCLUDED.key_id,nonce=EXCLUDED.nonce,ciphertext=EXCLUDED.ciphertext,updated_at=clock_timestamp()")
                .bind(actor.actor.user_id).bind(subject).bind(next).bind(credential.login).bind(credential.email).bind(credential.encrypted.key_id).bind(credential.encrypted.nonce).bind(credential.encrypted.ciphertext).execute(&mut *tx).await.map_err(unavailable)?;
            tx.commit().await.map_err(unavailable)
        })).await
    }

    async fn repository_credential(
        &self,
        actor: RepositoryActor,
    ) -> Result<Option<RepositoryCredential>, AccessError> {
        if !valid_actor(actor) {
            return Err(AccessError::InvalidInput);
        }
        with_connection(&self.pool, || AccessError::Unavailable, |connection| Box::pin(async move {
            let mut tx = connection.begin().await.map_err(unavailable)?;
            authorize(&mut tx, actor).await?;
            let row = sqlx::query("SELECT c.github_user_id,c.github_login,c.primary_email,c.key_id,c.nonce,c.ciphertext,c.authorization_epoch FROM factory_identity.github_credentials c JOIN factory_identity.github_authorization_epochs e ON e.user_id=c.user_id AND e.epoch=c.authorization_epoch WHERE c.user_id=$1")
                .bind(actor.actor.user_id).fetch_optional(&mut *tx).await.map_err(unavailable)?;
            let result = row.map(|r| -> Result<_, AccessError> { Ok(RepositoryCredential {
                subject: r.try_get::<i64,_>("github_user_id").map_err(unavailable)?.to_string(),
                authorization_epoch: r.try_get("authorization_epoch").map_err(unavailable)?,
                login: r.try_get("github_login").map_err(unavailable)?, email: r.try_get("primary_email").map_err(unavailable)?,
                encrypted: EncryptedRepositoryToken { key_id: r.try_get("key_id").map_err(unavailable)?, nonce: r.try_get("nonce").map_err(unavailable)?, ciphertext: r.try_get("ciphertext").map_err(unavailable)? }
            }) }).transpose()?;
            tx.commit().await.map_err(unavailable)?;
            Ok(result)
        })).await
    }

    async fn rotate_repository_token(
        &self,
        actor: RepositoryActor,
        epoch: i64,
        previous: &EncryptedRepositoryToken,
        next: EncryptedRepositoryToken,
    ) -> Result<(), AccessError> {
        if !valid_actor(actor)
            || epoch <= 0
            || !valid_text(&next.key_id, 128)
            || next.nonce.len() != 12
            || !(16..=8192).contains(&next.ciphertext.len())
        {
            return Err(AccessError::InvalidInput);
        }
        let previous = previous.clone();
        with_connection(&self.pool, || AccessError::Unavailable, |connection| Box::pin(async move {
            let mut tx = connection.begin().await.map_err(unavailable)?;
            authorize(&mut tx, actor).await?;
            check_epoch(&mut tx, actor.actor.user_id, epoch).await?;
            let result = sqlx::query("UPDATE factory_identity.github_credentials SET key_id=$5,nonce=$6,ciphertext=$7,updated_at=clock_timestamp() WHERE user_id=$1 AND authorization_epoch=$2 AND key_id=$3 AND nonce=$4")
                .bind(actor.actor.user_id).bind(epoch).bind(previous.key_id).bind(previous.nonce)
                .bind(next.key_id).bind(next.nonce).bind(next.ciphertext)
                .execute(&mut *tx).await.map_err(unavailable)?;
            if result.rows_affected() != 1 { return Err(AccessError::Conflict); }
            tx.commit().await.map_err(unavailable)
        })).await
    }

    async fn delete_repository_credential(
        &self,
        actor: RepositoryActor,
    ) -> Result<(), AccessError> {
        if !valid_actor(actor) {
            return Err(AccessError::InvalidInput);
        }
        with_connection(
            &self.pool,
            || AccessError::Unavailable,
            |connection| {
                Box::pin(async move {
                    let mut tx = connection.begin().await.map_err(unavailable)?;
                    authorize(&mut tx, actor).await?;
                    sqlx::query("INSERT INTO factory_identity.github_authorization_epochs(user_id,epoch) VALUES($1,1) ON CONFLICT(user_id) DO UPDATE SET epoch=factory_identity.github_authorization_epochs.epoch+1")
                        .bind(actor.actor.user_id).execute(&mut *tx).await.map_err(unavailable)?;
                    sqlx::query("DELETE FROM factory_identity.github_authorization_attempts WHERE user_id=$1")
                        .bind(actor.actor.user_id).execute(&mut *tx).await.map_err(unavailable)?;
                    sqlx::query("DELETE FROM factory_identity.github_credentials WHERE user_id=$1")
                        .bind(actor.actor.user_id)
                        .execute(&mut *tx)
                        .await
                        .map_err(unavailable)?;
                    tx.commit().await.map_err(unavailable)
                })
            },
        )
        .await
    }
}
