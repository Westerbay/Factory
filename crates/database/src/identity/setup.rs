use super::*;

// Keep the existing bootstrap lock key for interoperability with older runtimes.
const ADMIN_SETUP_LOCK: i64 = 0x4641435441444d;

impl IdentityDatabase {
    /// Operator-only check. Never expose this capability through the HTTP store port.
    pub async fn has_administrator(&self) -> Result<bool, IdentityError> {
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM factory_identity.users WHERE role='admin')")
            .fetch_one(&self.pool)
            .await
            .map_err(unavailable)
    }

    /// Configure exactly one operator-selected account, without issuing a session.
    /// Existing administrators are preserved, including on repeated/concurrent setup.
    pub async fn setup_administrator(
        &self,
        identity: &VerifiedIdentity,
    ) -> Result<User, IdentityError> {
        identity.validate()?;
        let mut tx = self.pool.begin().await.map_err(unavailable)?;
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(ADMIN_SETUP_LOCK)
            .execute(&mut *tx)
            .await
            .map_err(unavailable)?;
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM factory_identity.users WHERE role='admin')",
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(unavailable)?;
        if exists {
            return Err(IdentityError::ProtectedAdministrator);
        }
        let selected = admission::upsert_identity(&mut tx, identity).await?;
        sqlx::query("UPDATE factory_identity.users SET role='admin',status='active',version=version+1,access_enabled_at=clock_timestamp(),updated_at=clock_timestamp() WHERE id=$1")
            .bind(selected.id).execute(&mut *tx).await.map_err(unavailable)?;
        // A previously registered account must authenticate again after promotion.
        sqlx::query("DELETE FROM factory_identity.sessions WHERE user_id=$1")
            .bind(selected.id)
            .execute(&mut *tx)
            .await
            .map_err(unavailable)?;
        let row = sqlx::query(&format!("SELECT {USER_COLUMNS} FROM factory_identity.users u JOIN factory_identity.external_identities e ON e.user_id=u.id WHERE u.id=$1"))
            .bind(selected.id).fetch_one(&mut *tx).await.map_err(unavailable)?;
        let admin = user(&row)?;
        tx.commit().await.map_err(unavailable)?;
        Ok(admin)
    }
}
