use super::*;
use async_trait::async_trait;

#[async_trait]
pub trait IdentityProvider: Send + Sync {
    fn provider(&self) -> &str;
    fn issuer(&self) -> &str;
    fn authorization_url(&self, state: &str, pkce_challenge: &str)
    -> Result<String, IdentityError>;
    async fn exchange(
        &self,
        code: &str,
        pkce_verifier: &str,
    ) -> Result<VerifiedIdentity, IdentityError>;
}

#[async_trait]
pub trait IdentityStore: Send + Sync {
    /// Database clock owns created_at and the ten-minute expiry. Cleanup is bounded.
    async fn create_login_attempt(&self, attempt: NewLoginAttempt) -> Result<(), IdentityError>;

    /// Match both hashes and expiry, then consume atomically before provider exchange.
    /// Missing, expired, mismatched or replayed attempts return InvalidLogin.
    async fn take_login_attempt(
        &self,
        state_hash: CredentialHash,
        binding_hash: CredentialHash,
    ) -> Result<LoginAttempt, IdentityError>;

    /// One transaction upserts a pending member, locks the user, applies
    /// admission_decision and issues a fixed seven-day session. HTTP login never
    /// creates or promotes an administrator. Revoke previous token when
    /// issuing the replacement, and store only the new token hash. No raw token input.
    async fn admit(&self, admission: LoginAdmission) -> Result<AdmissionOutcome, IdentityError>;

    /// Recheck current user status and DB-clock expiry on every read. No role snapshots.
    async fn active_session(
        &self,
        token_hash: CredentialHash,
    ) -> Result<Option<ActiveSession>, IdentityError>;

    /// Idempotent revocation; an already-authorized request may still finish.
    async fn logout(&self, token_hash: CredentialHash) -> Result<(), IdentityError>;

    /// Revalidate live admin session in the read transaction before listing users.
    async fn list_users(
        &self,
        actor_session_hash: CredentialHash,
        query: UserListQuery,
    ) -> Result<UserPage, IdentityError>;

    /// Revalidate live admin, lock target, call authorize_access_change, compare version,
    /// update status, revoke target sessions on suspension and append access event atomically.
    /// Activation time uses clock_timestamp() AFTER obtaining the user lock, never now().
    async fn change_access(
        &self,
        actor_session_hash: CredentialHash,
        request: AccessRequest,
    ) -> Result<User, IdentityError>;
}
