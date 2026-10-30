use super::*;
use sqlx::{Postgres, Transaction};

async fn authorize(
    tx: &mut Transaction<'_, Postgres>,
    actor: EnvironmentActor,
) -> Result<(), EnvironmentError> {
    // Lock users first, matching admission and suspension. Check session after the wait.
    let status: Option<String> = sqlx::query_scalar(
        "SELECT u.status FROM factory_identity.users u WHERE u.id=$1 FOR UPDATE OF u",
    )
    .bind(actor.user_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(unavailable)?;
    if status.as_deref() != Some("active") {
        return Err(EnvironmentError::Unauthorized);
    }
    let active: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM factory_identity.sessions WHERE user_id=$1 AND token_hash=$2 AND expires_at>clock_timestamp())")
        .bind(actor.user_id).bind(actor.session_hash.0.as_slice()).fetch_one(&mut **tx).await.map_err(unavailable)?;
    if !active {
        return Err(EnvironmentError::Unauthorized);
    }
    Ok(())
}

async fn current(
    tx: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
) -> Result<Option<Environment>, EnvironmentError> {
    sqlx::query("SELECT * FROM factory_identity.environments WHERE user_id=$1 FOR UPDATE")
        .bind(user_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(unavailable)?
        .as_ref()
        .map(mapping::environment)
        .transpose()
}

pub(super) async fn get(
    pool: &PgPool,
    actor: EnvironmentActor,
) -> Result<Option<Environment>, EnvironmentError> {
    let mut tx = pool.begin().await.map_err(unavailable)?;
    authorize(&mut tx, actor).await?;
    let record = current(&mut tx, actor.user_id).await?;
    tx.commit().await.map_err(unavailable)?;
    Ok(record)
}

pub(super) async fn start(
    pool: &PgPool,
    actor: EnvironmentActor,
    request: EnvironmentStart,
) -> Result<Environment, EnvironmentError> {
    request.validate()?;
    let mut tx = pool.begin().await.map_err(unavailable)?;
    authorize(&mut tx, actor).await?;
    let existing = current(&mut tx, actor.user_id).await?;
    let upgrading: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM factory_identity.environment_upgrades u JOIN factory_identity.environments e ON e.id=u.environment_id WHERE e.user_id=$1)")
        .bind(actor.user_id).fetch_one(&mut *tx).await.map_err(unavailable)?;
    if upgrading {
        return Err(EnvironmentError::Conflict);
    }
    if existing
        .as_ref()
        .is_some_and(|e| e.installation_id != request.installation_id)
    {
        return Err(EnvironmentError::OwnershipConflict);
    }
    // Enforce the same prepared target under the row lock, including a change
    // between the application's read and this transaction.
    if existing.as_ref().is_some_and(|environment| {
        environment.image_id != request.image_id
            || environment.policy_version != request.policy_version
    }) {
        return Err(EnvironmentError::PolicyMismatch);
    }
    if existing.is_none() {
        let id = Uuid::new_v4();
        let name = format!(
            "factory-{}-{}",
            request.installation_id.simple(),
            id.simple()
        );
        sqlx::query("INSERT INTO factory_identity.environments(id,user_id,installation_id,desired_state,observed_state,volume_name,container_name,image_id,policy_version) VALUES($1,$2,$3,'running','not_created',$4,$5,$6,$7)")
            .bind(id).bind(actor.user_id).bind(request.installation_id).bind(format!("{name}-home")).bind(name)
            .bind(request.image_id).bind(request.policy_version)
            .execute(&mut *tx).await.map_err(unavailable)?;
    } else {
        sqlx::query("UPDATE factory_identity.environments SET generation=generation+CASE WHEN desired_state='running' THEN 0 ELSE 1 END,desired_state='running',next_reconcile_at=clock_timestamp(),updated_at=clock_timestamp() WHERE user_id=$1")
            .bind(actor.user_id).execute(&mut *tx).await.map_err(unavailable)?;
    }
    let record = current(&mut tx, actor.user_id)
        .await?
        .ok_or(EnvironmentError::Unavailable)?;
    tx.commit().await.map_err(unavailable)?;
    Ok(record)
}

pub(super) async fn stop(
    pool: &PgPool,
    actor: EnvironmentActor,
) -> Result<Option<Environment>, EnvironmentError> {
    let mut tx = pool.begin().await.map_err(unavailable)?;
    authorize(&mut tx, actor).await?;
    sqlx::query("UPDATE factory_identity.environments SET generation=generation+CASE WHEN desired_state='stopped' THEN 0 ELSE 1 END,desired_state='stopped',next_reconcile_at=clock_timestamp(),updated_at=clock_timestamp() WHERE user_id=$1")
        .bind(actor.user_id).execute(&mut *tx).await.map_err(unavailable)?;
    let record = current(&mut tx, actor.user_id).await?;
    tx.commit().await.map_err(unavailable)?;
    Ok(record)
}
