use factory_core::identity::*;
use std::sync::Arc;

pub struct IdentityService {
    store: Arc<dyn IdentityStore>,
    provider: Arc<dyn IdentityProvider>,
    bootstrap: BootstrapAdmin,
}

impl IdentityService {
    pub fn new(
        store: Arc<dyn IdentityStore>,
        provider: Arc<dyn IdentityProvider>,
        bootstrap: BootstrapAdmin,
    ) -> Self {
        Self {
            store,
            provider,
            bootstrap,
        }
    }

    pub async fn begin_login(
        &self,
        attempt: NewLoginAttempt,
        state: &str,
        challenge: &str,
    ) -> Result<String, IdentityError> {
        if state.is_empty()
            || state.len() > 256
            || challenge.len() != 43
            || !(43..=128).contains(&attempt.pkce_verifier.len())
        {
            return Err(IdentityError::InvalidInput);
        }
        let url = self.provider.authorization_url(state, challenge)?;
        self.store.create_login_attempt(attempt).await?;
        Ok(url)
    }

    pub async fn cancel_login(
        &self,
        state_hash: CredentialHash,
        binding_hash: CredentialHash,
    ) -> Result<(), IdentityError> {
        self.store
            .take_login_attempt(state_hash, binding_hash)
            .await?;
        Ok(())
    }

    pub async fn complete_login(
        &self,
        state_hash: CredentialHash,
        binding_hash: CredentialHash,
        code: &str,
        session: NewSession,
    ) -> Result<AdmissionOutcome, IdentityError> {
        if code.is_empty()
            || code.len() > 4096
            || session.csrf_token.len() < 32
            || session.csrf_token.len() > 256
        {
            return Err(IdentityError::InvalidLogin);
        }
        let attempt = self
            .store
            .take_login_attempt(state_hash, binding_hash)
            .await?;
        // The attempt is consumed even when the exchange fails. Never retry a code.
        let identity = self.provider.exchange(code, &attempt.pkce_verifier).await?;
        identity.validate()?;
        if identity.provider != self.provider.provider()
            || identity.issuer != self.provider.issuer()
        {
            return Err(IdentityError::InvalidLogin);
        }
        self.store
            .admit(LoginAdmission {
                identity,
                login_started_at: attempt.created_at,
                bootstrap: self.bootstrap.clone(),
                session,
            })
            .await
    }

    pub async fn session(&self, hash: CredentialHash) -> Result<ActiveSession, IdentityError> {
        self.store
            .active_session(hash)
            .await?
            .filter(|s| s.user.status == AccessStatus::Active)
            .ok_or(IdentityError::Unauthorized)
    }

    pub async fn logout(&self, hash: CredentialHash) -> Result<(), IdentityError> {
        self.store.logout(hash).await
    }

    pub async fn list_users(
        &self,
        hash: CredentialHash,
        query: UserListQuery,
    ) -> Result<UserPage, IdentityError> {
        query.validate()?;
        require_admin(&self.session(hash).await?.user)?;
        self.store.list_users(hash, query).await
    }

    pub async fn change_access(
        &self,
        hash: CredentialHash,
        request: AccessRequest,
    ) -> Result<User, IdentityError> {
        if request.expected_version < 1 {
            return Err(IdentityError::InvalidInput);
        }
        require_admin(&self.session(hash).await?.user)?;
        self.store.change_access(hash, request).await
    }
}
