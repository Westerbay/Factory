use super::*;
use uuid::Uuid;

pub(super) async fn list(
    pool: &PgPool,
    hash: CredentialHash,
    query: UserListQuery,
) -> Result<UserPage, IdentityError> {
    query.validate()?;
    let mut tx = pool.begin().await.map_err(unavailable)?;
    sessions::admin(&mut tx, hash).await?;
    let sql = format!(
        "SELECT {USER_COLUMNS} FROM factory_identity.users u JOIN factory_identity.external_identities e ON e.user_id=u.id WHERE u.status=$1 AND ($2::uuid IS NULL OR u.id>$2) ORDER BY u.id LIMIT $3"
    );
    let rows = sqlx::query(&sql)
        .bind(status(query.status))
        .bind(query.after)
        .bind(i64::from(query.limit) + 1)
        .fetch_all(&mut *tx)
        .await
        .map_err(unavailable)?;
    let mut items = rows.iter().map(user).collect::<Result<Vec<_>, _>>()?;
    let next_cursor = if items.len() > usize::from(query.limit) {
        items.truncate(usize::from(query.limit));
        items.last().map(|u| u.id)
    } else {
        None
    };
    tx.commit().await.map_err(unavailable)?;
    Ok(UserPage { items, next_cursor })
}

pub(super) async fn change(
    pool: &PgPool,
    hash: CredentialHash,
    request: AccessRequest,
) -> Result<User, IdentityError> {
    let mut tx = pool.begin().await.map_err(unavailable)?;
    let actor = sessions::admin(&mut tx, hash).await?;
    let sql = format!(
        "SELECT {USER_COLUMNS} FROM factory_identity.users u JOIN factory_identity.external_identities e ON e.user_id=u.id WHERE u.id=$1 FOR UPDATE OF u"
    );
    let row = sqlx::query(&sql)
        .bind(request.target_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(unavailable)?
        .ok_or(IdentityError::NotFound)?;
    let mut target = user(&row)?;
    if authorize_access_change(&actor, &target, &request)? {
        let next = AccessStatus::from(request.status);
        let row = sqlx::query("UPDATE factory_identity.users SET status=$2,version=version+1,updated_at=clock_timestamp(),access_enabled_at=CASE WHEN $2='active' THEN clock_timestamp() ELSE access_enabled_at END WHERE id=$1 RETURNING version,access_enabled_at")
            .bind(target.id).bind(status(next)).fetch_one(&mut *tx).await.map_err(unavailable)?;
        if next == AccessStatus::Suspended {
            sqlx::query("UPDATE factory_identity.environments SET desired_state='stopped',lease_expires_at=NULL,generation=generation+CASE WHEN desired_state='running' THEN 1 ELSE 0 END,next_reconcile_at=clock_timestamp(),updated_at=clock_timestamp() WHERE user_id=$1")
                .bind(target.id).execute(&mut *tx).await.map_err(unavailable)?;
            sqlx::query("DELETE FROM factory_identity.sessions WHERE user_id=$1")
                .bind(target.id)
                .execute(&mut *tx)
                .await
                .map_err(unavailable)?;
        }
        sqlx::query("INSERT INTO factory_identity.access_events(id,actor_user_id,target_user_id,prior_status,next_status) VALUES($1,$2,$3,$4,$5)")
            .bind(Uuid::new_v4()).bind(actor.id).bind(target.id).bind(status(target.status)).bind(status(next))
            .execute(&mut *tx).await.map_err(unavailable)?;
        target.status = next;
        target.version = row.try_get("version").map_err(unavailable)?;
        target.access_enabled_at = row.try_get("access_enabled_at").map_err(unavailable)?;
    }
    tx.commit().await.map_err(unavailable)?;
    Ok(target)
}
