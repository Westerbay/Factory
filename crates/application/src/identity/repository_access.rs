//! Session-bound repository authorization, token storage and SSH key setup.
use factory_core::identity::*;
use factory_core::projects::{
    Project, ProjectState, RepositoryMetadata, validate_repository_metadata,
};
use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex, Weak},
};
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};
use uuid::Uuid;

const TOKEN_RENEWAL_MARGIN_SECONDS: i64 = 60;

struct AccessGuard {
    store: Arc<dyn RepositoryAccessStore>,
    actor: RepositoryActor,
    epoch: i64,
}

/// Holds the admitted actor's repository credential inside the application boundary.
pub(crate) struct RepositoryDraftAccess<'a> {
    provider: &'a dyn RepositoryAccessProvider,
    token: RepositoryAccessToken,
    guard: AccessGuard,
    project: &'a Project,
}

impl RepositoryDraftAccess<'_> {
    pub(crate) async fn recheck(&self) -> Result<(), RepositoryAccessError> {
        self.guard.check().await?;
        let origin = self
            .provider
            .verify_clone_origin(
                &self.token,
                self.project.repository_id,
                &self.project.cloned_full_name,
            )
            .await?;
        self.guard.check().await?;
        match origin {
            RepositoryOriginVerification::Matched => Ok(()),
            RepositoryOriginVerification::Changed => Err(RepositoryAccessError::Conflict),
        }
    }

    pub(crate) async fn find_draft(
        &self,
        target: &RepositoryDraftTarget<'_>,
    ) -> Result<Option<RepositoryDraft>, RepositoryAccessError> {
        self.check_target(target.repository_id, target.full_name)?;
        self.recheck().await?;
        let result = self
            .provider
            .find_draft(&self.token, target, &self.guard)
            .await?;
        self.guard.check().await?;
        Ok(result)
    }

    pub(crate) async fn create_draft(
        &self,
        target: &RepositoryDraftTarget<'_>,
        title: &str,
    ) -> Result<RepositoryDraft, RepositoryAccessError> {
        self.check_target(target.repository_id, target.full_name)?;
        let result = self
            .provider
            .create_draft(&self.token, target, title, &self.guard)
            .await?;
        // The write already happened. If this actor is no longer current, the
        // durable claim remains available for reconciliation by an active session.
        self.guard
            .check()
            .await
            .map_err(|_| RepositoryAccessError::Unavailable)?;
        Ok(result)
    }

    pub(crate) async fn draft_diff(
        &self,
        target: &RepositoryDraftDiffTarget<'_>,
    ) -> Result<RepositoryDraftDiff, RepositoryAccessError> {
        self.check_target(target.repository_id, target.full_name)?;
        let result = self
            .provider
            .draft_diff(&self.token, target, &self.guard)
            .await?;
        self.guard.check().await?;
        Ok(result)
    }

    fn check_target(
        &self,
        repository_id: i64,
        full_name: &str,
    ) -> Result<(), RepositoryAccessError> {
        if repository_id != self.project.repository_id || full_name != self.project.cloned_full_name
        {
            return Err(RepositoryAccessError::Conflict);
        }
        Ok(())
    }
}

struct LoginReadGuard {
    store: Arc<dyn RepositoryAccessStore>,
    actor: factory_core::environments::EnvironmentActor,
    epoch: i64,
}
impl RepositoryOperationGuard for LoginReadGuard {
    fn check(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<(), RepositoryAccessError>> + Send + '_>> {
        Box::pin(async {
            if self.store.login_credential_epoch(self.actor).await? != self.epoch {
                return Err(RepositoryAccessError::ProviderUnauthorized);
            }
            Ok(())
        })
    }
}
impl RepositoryOperationGuard for AccessGuard {
    fn check(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<(), RepositoryAccessError>> + Send + '_>> {
        Box::pin(self.store.validate_repository_epoch(self.actor, self.epoch))
    }
}

pub struct RepositoryAccessService {
    store: Arc<dyn RepositoryAccessStore>,
    provider: Arc<dyn RepositoryAccessProvider>,
    cipher: Arc<dyn RepositoryTokenCipher>,
    keys: Arc<dyn RepositoryPublicKeySource>,
    // Weak entries are pruned on acquisition; users do not block one another.
    registration: Mutex<HashMap<Uuid, Weak<AsyncMutex<()>>>>,
}

impl RepositoryAccessService {
    pub fn new(
        store: Arc<dyn RepositoryAccessStore>,
        provider: Arc<dyn RepositoryAccessProvider>,
        cipher: Arc<dyn RepositoryTokenCipher>,
        keys: Arc<dyn RepositoryPublicKeySource>,
    ) -> Self {
        Self {
            store,
            provider,
            cipher,
            keys,
            registration: Mutex::new(HashMap::new()),
        }
    }

    fn registration_lock(&self, user: Uuid) -> Result<Arc<AsyncMutex<()>>, RepositoryAccessError> {
        let mut locks = self
            .registration
            .lock()
            .map_err(|_| RepositoryAccessError::Unavailable)?;
        locks.retain(|_, lock| lock.strong_count() > 0);
        if let Some(lock) = locks.get(&user).and_then(Weak::upgrade) {
            return Ok(lock);
        }
        let lock = Arc::new(AsyncMutex::new(()));
        locks.insert(user, Arc::downgrade(&lock));
        Ok(lock)
    }

    async fn authorized_token(
        &self,
        actor: RepositoryActor,
    ) -> Result<(RepositoryAccessToken, AccessGuard), RepositoryAccessError> {
        let (_, token, guard) = self.read_authorized_token(actor).await?;
        if !needs_renewal(&token) {
            return Ok((token, guard));
        }
        let lock = self
            .registration_lock(actor.actor.user_id)?
            .lock_owned()
            .await;
        let (_lock, token, guard) = self.authorized_token_locked(actor, lock).await?;
        Ok((token, guard))
    }

    async fn read_authorized_token(
        &self,
        actor: RepositoryActor,
    ) -> Result<(EncryptedRepositoryToken, RepositoryAccessToken, AccessGuard), RepositoryAccessError>
    {
        let credential = self
            .store
            .repository_credential(actor)
            .await?
            .ok_or(RepositoryAccessError::ProviderUnauthorized)?;
        let token = self
            .cipher
            .open(actor.actor.user_id, &credential.encrypted)?;
        let guard = AccessGuard {
            store: self.store.clone(),
            actor,
            epoch: credential.authorization_epoch,
        };
        guard.check().await?;
        Ok((credential.encrypted, token, guard))
    }

    /// Serialize the rotation with other renewals and key registration for this user.
    async fn authorized_token_locked(
        &self,
        actor: RepositoryActor,
        lock: OwnedMutexGuard<()>,
    ) -> Result<(OwnedMutexGuard<()>, RepositoryAccessToken, AccessGuard), RepositoryAccessError>
    {
        let (previous, token, guard) = self.read_authorized_token(actor).await?;
        if !needs_renewal(&token) {
            return Ok((lock, token, guard));
        }
        let provider = self.provider.clone();
        let cipher = self.cipher.clone();
        // GitHub consumes the previous refresh token on success. The caller
        // can disconnect, but an in-flight rotation must reach the guarded save.
        tokio::spawn(async move {
            let renewal = token.renewal().ok_or(RepositoryAccessError::Unavailable)?;
            let refreshed = provider.refresh(renewal).await?;
            let next = cipher.seal(actor.actor.user_id, &refreshed)?;
            guard
                .store
                .rotate_repository_token(actor, guard.epoch, &previous, next)
                .await?;
            Ok((lock, refreshed, guard))
        })
        .await
        .map_err(|_| RepositoryAccessError::Unavailable)?
    }

    /// Complete sign-in without requiring a running environment. Persistence
    /// rechecks the session after the read-only email request and encryption.
    pub async fn save_login(
        &self,
        actor: factory_core::environments::EnvironmentActor,
        identity: &VerifiedIdentity,
        token: RepositoryAccessToken,
    ) -> Result<(), RepositoryAccessError> {
        let epoch = self.store.login_credential_epoch(actor).await?;
        let guard = LoginReadGuard {
            store: self.store.clone(),
            actor,
            epoch,
        };
        let email = self.provider.primary_email(&token, &guard).await?;
        let encrypted = self.cipher.seal(actor.user_id, &token)?;
        self.store
            .save_login_credential(
                actor,
                identity,
                RepositoryCredential {
                    subject: identity.subject.clone(),
                    login: identity.handle.clone(),
                    email,
                    encrypted,
                    authorization_epoch: epoch,
                },
            )
            .await
    }

    /// Register the environment key only after the environment is available.
    pub async fn register_existing_key(
        &self,
        actor: RepositoryActor,
    ) -> Result<(), RepositoryAccessError> {
        let lock = self
            .registration_lock(actor.actor.user_id)?
            .lock_owned()
            .await;
        let (_registration, token, guard) = self.authorized_token_locked(actor, lock).await?;
        let mut key = self.keys.public_key_status(actor).await?;
        let exists = self
            .provider
            .public_key_registered(&token, &key.public_key, &guard)
            .await?;
        guard.check().await?;
        if !exists && key.registered {
            key = self.keys.rotate_missing_key(actor, &key.public_key).await?;
        }
        if !exists {
            guard.check().await?;
            self.provider
                .ensure_public_key(&token, &key.public_key, &guard)
                .await?;
        }
        guard.check().await?;
        self.keys.mark_registered(actor, &key.public_key).await?;
        guard.check().await
    }

    pub async fn begin(
        &self,
        attempt: RepositoryAuthorizationAttempt,
        state: &str,
        challenge: &str,
    ) -> Result<String, RepositoryAccessError> {
        if !(43..=128).contains(&attempt.verifier.len()) || challenge.len() != 43 {
            return Err(RepositoryAccessError::InvalidInput);
        }
        let url = self.provider.authorization_url(state, challenge)?;
        self.store.begin_repository_authorization(attempt).await?;
        Ok(url)
    }

    /// The caller must supply the current session hash from its cookie. The store
    /// consumes the attempt atomically and rechecks session and generation.
    pub async fn complete(
        &self,
        state: CredentialHash,
        binding: CredentialHash,
        session: CredentialHash,
        code: &str,
    ) -> Result<(), RepositoryAccessError> {
        let attempt = self
            .store
            .take_repository_authorization(state, binding, session)
            .await?;
        let actor = attempt.actor;
        let epoch = attempt.authorization_epoch;
        let guard = AccessGuard {
            store: self.store.clone(),
            actor,
            epoch,
        };
        guard.check().await?;
        let (token, identity) = self.provider.exchange(code, &attempt.verifier).await?;
        identity
            .validate()
            .map_err(|_| RepositoryAccessError::InvalidLogin)?;
        self.store
            .verify_repository_identity(actor, &identity, epoch)
            .await?;
        let email = self.provider.primary_email(&token, &guard).await?;
        let user_lock = self.registration_lock(actor.actor.user_id)?;
        let _registration = user_lock.lock().await;
        self.store
            .verify_repository_identity(actor, &identity, epoch)
            .await?;
        let mut key = self.keys.public_key_status(actor).await?;
        let exists = self
            .provider
            .public_key_registered(&token, &key.public_key, &guard)
            .await?;
        // A remote absence is trustworthy only after a successful full listing.
        // Rotate only when durable evidence proves this exact key was registered.
        self.store
            .verify_repository_identity(actor, &identity, epoch)
            .await?;
        if !exists && key.registered {
            key = self.keys.rotate_missing_key(actor, &key.public_key).await?;
        }
        if !exists {
            self.store
                .verify_repository_identity(actor, &identity, epoch)
                .await?;
            self.provider
                .ensure_public_key(&token, &key.public_key, &guard)
                .await?;
        }
        self.store
            .verify_repository_identity(actor, &identity, epoch)
            .await?;
        self.keys.mark_registered(actor, &key.public_key).await?;
        let encrypted = self.cipher.seal(actor.actor.user_id, &token)?;
        let credential = RepositoryCredential {
            subject: identity.subject.clone(),
            login: identity.handle.clone(),
            email,
            encrypted,
            authorization_epoch: epoch,
        };
        self.store
            .save_repository_credential(actor, &identity, credential)
            .await
    }

    pub async fn cancel(
        &self,
        state: CredentialHash,
        binding: CredentialHash,
        session: CredentialHash,
    ) -> Result<(), RepositoryAccessError> {
        self.store
            .take_repository_authorization(state, binding, session)
            .await?;
        Ok(())
    }

    pub async fn connection(
        &self,
        actor: RepositoryActor,
    ) -> Result<Option<RepositoryConnection>, RepositoryAccessError> {
        Ok(self
            .store
            .repository_credential(actor)
            .await?
            .map(|credential| RepositoryConnection {
                login: credential.login,
                email: credential.email,
            }))
    }

    pub async fn repositories(
        &self,
        actor: RepositoryActor,
        page: u32,
    ) -> Result<AccessibleRepositoryPage, RepositoryAccessError> {
        if page == 0 || page > 1000 {
            return Err(RepositoryAccessError::InvalidInput);
        }
        let (token, guard) = self.authorized_token(actor).await?;
        let result = self.provider.repositories(&token, page).await?;
        guard.check().await?;
        Ok(result)
    }

    /// Verify the persisted clone-time provider address, not the local Git remote.
    /// The caller must first load the project through the owned project store.
    pub async fn verify_clone_origin(
        &self,
        actor: RepositoryActor,
        project: &Project,
    ) -> Result<RepositoryOriginVerification, RepositoryAccessError> {
        check_project_binding(actor, project)?;
        let (token, guard) = self.authorized_token(actor).await?;
        let result = self
            .provider
            .verify_clone_origin(&token, project.repository_id, &project.cloned_full_name)
            .await?;
        guard.check().await?;
        Ok(result)
    }

    pub(crate) async fn draft_access<'a>(
        &'a self,
        actor: RepositoryActor,
        project: &'a Project,
    ) -> Result<RepositoryDraftAccess<'a>, RepositoryAccessError> {
        check_project_binding(actor, project)?;
        let (token, guard) = self.authorized_token(actor).await?;
        let access = RepositoryDraftAccess {
            provider: self.provider.as_ref(),
            token,
            guard,
            project,
        };
        access.recheck().await?;
        Ok(access)
    }

    pub async fn disconnect(&self, actor: RepositoryActor) -> Result<(), RepositoryAccessError> {
        self.store.delete_repository_credential(actor).await
    }
}

fn needs_renewal(token: &RepositoryAccessToken) -> bool {
    token.renewal().is_some_and(|renewal| {
        renewal.expires_at() <= chrono::Utc::now().timestamp() + TOKEN_RENEWAL_MARGIN_SECONDS
    })
}

fn check_project_binding(
    actor: RepositoryActor,
    project: &Project,
) -> Result<(), RepositoryAccessError> {
    if project.user_id != actor.actor.user_id || project.environment_id != actor.environment_id {
        return Err(RepositoryAccessError::Unauthorized);
    }
    if project.state != ProjectState::Ready {
        return Err(RepositoryAccessError::Conflict);
    }
    validate_repository_metadata(&[RepositoryMetadata {
        repository_id: project.repository_id,
        full_name: project.cloned_full_name.clone(),
    }])
    .map_err(|_| RepositoryAccessError::InvalidInput)
}

#[cfg(test)]
mod tests;
