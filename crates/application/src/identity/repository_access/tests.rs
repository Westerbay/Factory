use super::*;
use async_trait::async_trait;
use factory_core::environments::EnvironmentActor;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};
use uuid::Uuid;

struct Store {
    attempt: std::sync::Mutex<Option<RepositoryAuthorizationAttempt>>,
    credential: std::sync::Mutex<Option<RepositoryCredential>>,
    identity_matches: AtomicBool,
    active: AtomicBool,
    epoch: AtomicI64,
}
#[async_trait]
impl RepositoryAccessStore for Store {
    async fn login_credential_epoch(
        &self,
        _: EnvironmentActor,
    ) -> Result<i64, RepositoryAccessError> {
        if !self.active.load(Ordering::SeqCst) {
            return Err(RepositoryAccessError::Unauthorized);
        }
        Ok(self.epoch.load(Ordering::SeqCst))
    }
    async fn save_login_credential(
        &self,
        _: EnvironmentActor,
        _: &VerifiedIdentity,
        mut credential: RepositoryCredential,
    ) -> Result<(), RepositoryAccessError> {
        if !self.active.load(Ordering::SeqCst) {
            return Err(RepositoryAccessError::Unauthorized);
        }
        if self.epoch.load(Ordering::SeqCst) != credential.authorization_epoch {
            return Err(RepositoryAccessError::ProviderUnauthorized);
        }
        credential.authorization_epoch = self.epoch.fetch_add(1, Ordering::SeqCst) + 1;
        *self.credential.lock().unwrap() = Some(credential);
        Ok(())
    }
    async fn begin_repository_authorization(
        &self,
        mut attempt: RepositoryAuthorizationAttempt,
    ) -> Result<(), RepositoryAccessError> {
        attempt.authorization_epoch = self.epoch.load(Ordering::SeqCst);
        *self.attempt.lock().unwrap() = Some(attempt);
        Ok(())
    }
    async fn take_repository_authorization(
        &self,
        state: CredentialHash,
        binding: CredentialHash,
        session: CredentialHash,
    ) -> Result<RepositoryAuthorizationAttempt, RepositoryAccessError> {
        let attempt = {
            let mut attempt = self.attempt.lock().unwrap();
            if !attempt.as_ref().is_some_and(|a| {
                a.state_hash == state
                    && a.binding_hash == binding
                    && a.actor.actor.session_hash == session
            }) {
                return Err(RepositoryAccessError::InvalidLogin);
            }
            if !self.active.load(Ordering::SeqCst) {
                return Err(RepositoryAccessError::Unauthorized);
            }
            attempt.take().unwrap()
        };
        self.validate_repository_epoch(attempt.actor, attempt.authorization_epoch)
            .await?;
        Ok(attempt)
    }
    async fn validate_repository_epoch(
        &self,
        _: RepositoryActor,
        epoch: i64,
    ) -> Result<(), RepositoryAccessError> {
        if !self.active.load(Ordering::SeqCst) {
            return Err(RepositoryAccessError::Unauthorized);
        }
        if self.epoch.load(Ordering::SeqCst) != epoch {
            return Err(RepositoryAccessError::ProviderUnauthorized);
        }
        Ok(())
    }
    async fn verify_repository_identity(
        &self,
        actor: RepositoryActor,
        _: &VerifiedIdentity,
        epoch: i64,
    ) -> Result<(), RepositoryAccessError> {
        self.validate_repository_epoch(actor, epoch).await?;
        if self.identity_matches.load(Ordering::SeqCst) {
            Ok(())
        } else {
            Err(RepositoryAccessError::IdentityMismatch)
        }
    }
    async fn save_repository_credential(
        &self,
        actor: RepositoryActor,
        identity: &VerifiedIdentity,
        mut credential: RepositoryCredential,
    ) -> Result<(), RepositoryAccessError> {
        self.verify_repository_identity(actor, identity, credential.authorization_epoch)
            .await?;
        credential.authorization_epoch = self.epoch.fetch_add(1, Ordering::SeqCst) + 1;
        *self.credential.lock().unwrap() = Some(credential);
        Ok(())
    }
    async fn repository_credential(
        &self,
        actor: RepositoryActor,
    ) -> Result<Option<RepositoryCredential>, RepositoryAccessError> {
        self.validate_repository_epoch(actor, self.epoch.load(Ordering::SeqCst))
            .await?;
        Ok(self
            .credential
            .lock()
            .unwrap()
            .as_ref()
            .map(|c| RepositoryCredential {
                subject: c.subject.clone(),
                authorization_epoch: c.authorization_epoch,
                login: c.login.clone(),
                email: c.email.clone(),
                encrypted: c.encrypted.clone(),
            }))
    }
    async fn rotate_repository_token(
        &self,
        actor: RepositoryActor,
        epoch: i64,
        previous: &EncryptedRepositoryToken,
        next: EncryptedRepositoryToken,
    ) -> Result<(), RepositoryAccessError> {
        self.validate_repository_epoch(actor, epoch).await?;
        let mut credential = self.credential.lock().unwrap();
        let current = credential.as_mut().ok_or(RepositoryAccessError::Conflict)?;
        if current.encrypted.ciphertext != previous.ciphertext {
            return Err(RepositoryAccessError::Conflict);
        }
        current.encrypted = next;
        Ok(())
    }
    async fn delete_repository_credential(
        &self,
        actor: RepositoryActor,
    ) -> Result<(), RepositoryAccessError> {
        self.validate_repository_epoch(actor, self.epoch.load(Ordering::SeqCst))
            .await?;
        self.epoch.fetch_add(1, Ordering::SeqCst);
        *self.attempt.lock().unwrap() = None;
        *self.credential.lock().unwrap() = None;
        Ok(())
    }
}
struct Gate {
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}
impl Gate {
    fn new() -> Self {
        Self {
            entered: tokio::sync::Notify::new(),
            release: tokio::sync::Notify::new(),
        }
    }
}
struct Provider {
    exchange_gate: std::sync::Mutex<Option<Arc<Gate>>>,
    list_gate: std::sync::Mutex<Option<Arc<Gate>>>,
    origin_calls: AtomicUsize,
    refresh_calls: AtomicUsize,
    refresh_gate: std::sync::Mutex<Option<Arc<Gate>>>,
    reject_refresh: AtomicBool,
    registered: AtomicUsize,
    store: Arc<Store>,
    revoke_on_list: AtomicBool,
    revoke_on_email: AtomicBool,
    key_listing: AtomicUsize,
}
#[async_trait]
impl RepositoryAccessProvider for Provider {
    fn authorization_url(&self, _: &str, _: &str) -> Result<String, RepositoryAccessError> {
        Ok("https://github.test/authorize".into())
    }
    async fn exchange(
        &self,
        _: &str,
        _: &str,
    ) -> Result<(RepositoryAccessToken, VerifiedIdentity), RepositoryAccessError> {
        let gate = self.exchange_gate.lock().unwrap().clone();
        if let Some(gate) = gate {
            gate.entered.notify_one();
            gate.release.notified().await;
        }
        Ok((
            RepositoryAccessToken::new("fixture-token".into()).unwrap(),
            VerifiedIdentity {
                provider: "github".into(),
                issuer: "https://github.com".into(),
                subject: "123".into(),
                display_name: "user".into(),
                handle: "user".into(),
            },
        ))
    }
    async fn refresh(
        &self,
        renewal: &RepositoryTokenRenewal,
    ) -> Result<RepositoryAccessToken, RepositoryAccessError> {
        assert_eq!(renewal.secret(), "fixture-refresh");
        self.refresh_calls.fetch_add(1, Ordering::SeqCst);
        let gate = self.refresh_gate.lock().unwrap().clone();
        if let Some(gate) = gate {
            gate.entered.notify_one();
            gate.release.notified().await;
        }
        if self.reject_refresh.load(Ordering::SeqCst) {
            return Err(RepositoryAccessError::ProviderUnauthorized);
        }
        RepositoryAccessToken::renewable(
            "refreshed-token".into(),
            "refreshed-renewal".into(),
            chrono::Utc::now().timestamp() + 3600,
        )
    }
    async fn primary_email(
        &self,
        _: &RepositoryAccessToken,
        guard: &dyn RepositoryOperationGuard,
    ) -> Result<String, RepositoryAccessError> {
        guard.check().await?;
        if self.revoke_on_email.load(Ordering::SeqCst) {
            self.store.active.store(false, Ordering::SeqCst);
        }
        Ok("user@example.test".into())
    }
    async fn repositories(
        &self,
        _: &RepositoryAccessToken,
        _: u32,
    ) -> Result<AccessibleRepositoryPage, RepositoryAccessError> {
        let gate = self.list_gate.lock().unwrap().clone();
        if let Some(gate) = gate {
            gate.entered.notify_one();
            gate.release.notified().await;
        }
        if self.revoke_on_list.load(Ordering::SeqCst) {
            self.store.active.store(false, Ordering::SeqCst);
        }
        Ok(AccessibleRepositoryPage {
            items: vec![],
            next_page: None,
        })
    }
    async fn verify_clone_origin(
        &self,
        _: &RepositoryAccessToken,
        _: i64,
        _: &str,
    ) -> Result<RepositoryOriginVerification, RepositoryAccessError> {
        self.origin_calls.fetch_add(1, Ordering::SeqCst);
        let gate = self.list_gate.lock().unwrap().clone();
        if let Some(gate) = gate {
            gate.entered.notify_one();
            gate.release.notified().await;
        }
        Ok(RepositoryOriginVerification::Matched)
    }
    async fn public_key_registered(
        &self,
        _: &RepositoryAccessToken,
        _: &str,
        guard: &dyn RepositoryOperationGuard,
    ) -> Result<bool, RepositoryAccessError> {
        guard.check().await?;
        match self.key_listing.load(Ordering::SeqCst) {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(RepositoryAccessError::Unavailable),
        }
    }
    async fn ensure_public_key(
        &self,
        _: &RepositoryAccessToken,
        _: &str,
        guard: &dyn RepositoryOperationGuard,
    ) -> Result<(), RepositoryAccessError> {
        guard.check().await?;
        self.registered.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}
struct Cipher;
impl RepositoryTokenCipher for Cipher {
    fn seal(
        &self,
        _: Uuid,
        token: &RepositoryAccessToken,
    ) -> Result<EncryptedRepositoryToken, RepositoryAccessError> {
        Ok(EncryptedRepositoryToken {
            key_id: "test".into(),
            nonce: vec![0; 12],
            ciphertext: token.secret().as_bytes().to_vec(),
        })
    }
    fn open(
        &self,
        _: Uuid,
        token: &EncryptedRepositoryToken,
    ) -> Result<RepositoryAccessToken, RepositoryAccessError> {
        match token.ciphertext.as_slice() {
            b"expired-token" => RepositoryAccessToken::renewable(
                "expired-token".into(),
                "fixture-refresh".into(),
                chrono::Utc::now().timestamp() - 1,
            ),
            b"refreshed-token" => RepositoryAccessToken::renewable(
                "refreshed-token".into(),
                "refreshed-renewal".into(),
                chrono::Utc::now().timestamp() + 3600,
            ),
            _ => RepositoryAccessToken::new("fixture-token".into()),
        }
    }
}
struct Keys {
    calls: AtomicUsize,
    registered: AtomicBool,
    rotations: AtomicUsize,
    marked: AtomicUsize,
}
#[async_trait]
impl RepositoryPublicKeySource for Keys {
    async fn public_key_status(
        &self,
        _: RepositoryActor,
    ) -> Result<RepositoryPublicKey, RepositoryAccessError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(RepositoryPublicKey {
            public_key: "fixture-public-key".into(),
            registered: self.registered.load(Ordering::SeqCst),
        })
    }
    async fn mark_registered(
        &self,
        _: RepositoryActor,
        _: &str,
    ) -> Result<(), RepositoryAccessError> {
        self.marked.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    async fn rotate_missing_key(
        &self,
        _: RepositoryActor,
        _: &str,
    ) -> Result<RepositoryPublicKey, RepositoryAccessError> {
        self.rotations.fetch_add(1, Ordering::SeqCst);
        Ok(RepositoryPublicKey {
            public_key: "replacement-public-key".into(),
            registered: false,
        })
    }
}
fn fixture() -> (
    RepositoryAccessService,
    Arc<Store>,
    Arc<Provider>,
    Arc<Keys>,
    RepositoryActor,
) {
    let store = Arc::new(Store {
        attempt: std::sync::Mutex::new(None),
        credential: std::sync::Mutex::new(None),
        identity_matches: AtomicBool::new(true),
        active: AtomicBool::new(true),
        epoch: AtomicI64::new(1),
    });
    let provider = Arc::new(Provider {
        exchange_gate: std::sync::Mutex::new(None),
        list_gate: std::sync::Mutex::new(None),
        origin_calls: AtomicUsize::new(0),
        refresh_calls: AtomicUsize::new(0),
        refresh_gate: std::sync::Mutex::new(None),
        reject_refresh: AtomicBool::new(false),
        registered: AtomicUsize::new(0),
        store: store.clone(),
        revoke_on_list: AtomicBool::new(false),
        revoke_on_email: AtomicBool::new(false),
        key_listing: AtomicUsize::new(0),
    });
    let keys = Arc::new(Keys {
        calls: AtomicUsize::new(0),
        registered: AtomicBool::new(false),
        rotations: AtomicUsize::new(0),
        marked: AtomicUsize::new(0),
    });
    let service = RepositoryAccessService::new(
        store.clone(),
        provider.clone(),
        Arc::new(Cipher),
        keys.clone(),
    );
    let actor = RepositoryActor {
        actor: EnvironmentActor {
            user_id: Uuid::new_v4(),
            session_hash: CredentialHash([3; 32]),
        },
        environment_id: Uuid::new_v4(),
        generation: 1,
    };
    (service, store, provider, keys, actor)
}
async fn begin(service: &RepositoryAccessService, actor: RepositoryActor) {
    service
        .begin(
            RepositoryAuthorizationAttempt {
                state_hash: CredentialHash([1; 32]),
                binding_hash: CredentialHash([2; 32]),
                verifier: "v".repeat(43),
                actor,
                authorization_epoch: 0,
            },
            &"s".repeat(43),
            &"c".repeat(43),
        )
        .await
        .unwrap();
}

async fn save_expired_token(
    service: &RepositoryAccessService,
    provider: &Provider,
    actor: RepositoryActor,
) {
    let (_, identity) = provider.exchange("code", "verifier").await.unwrap();
    let token = RepositoryAccessToken::renewable(
        "expired-token".into(),
        "fixture-refresh".into(),
        chrono::Utc::now().timestamp() - 1,
    )
    .unwrap();
    service
        .save_login(actor.actor, &identity, token)
        .await
        .unwrap();
}

#[tokio::test]
async fn concurrent_repository_requests_share_one_renewal() {
    let (service, store, provider, _, actor) = fixture();
    save_expired_token(&service, &provider, actor).await;
    let epoch = store.epoch.load(Ordering::SeqCst);
    let (left, right) = tokio::join!(
        service.repositories(actor, 1),
        service.repositories(actor, 2),
    );
    assert!(left.is_ok() && right.is_ok());
    assert_eq!(provider.refresh_calls.load(Ordering::SeqCst), 1);
    assert_eq!(store.epoch.load(Ordering::SeqCst), epoch);
    assert_eq!(
        store
            .credential
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .encrypted
            .ciphertext,
        b"refreshed-token"
    );
}

#[tokio::test]
async fn disconnect_during_renewal_cannot_restore_the_grant() {
    let (service, store, provider, _, actor) = fixture();
    save_expired_token(&service, &provider, actor).await;
    let gate = Arc::new(Gate::new());
    *provider.refresh_gate.lock().unwrap() = Some(gate.clone());
    let (result, ()) = tokio::join!(service.repositories(actor, 1), async {
        gate.entered.notified().await;
        service.disconnect(actor).await.unwrap();
        gate.release.notify_one();
    });
    assert!(matches!(
        result,
        Err(RepositoryAccessError::ProviderUnauthorized)
    ));
    assert!(store.credential.lock().unwrap().is_none());
}

#[tokio::test]
async fn canceled_request_does_not_lose_rotated_credentials() {
    let (service, _, provider, _, actor) = fixture();
    let service = Arc::new(service);
    save_expired_token(&service, &provider, actor).await;
    let gate = Arc::new(Gate::new());
    *provider.refresh_gate.lock().unwrap() = Some(gate.clone());
    let caller = tokio::spawn({
        let service = service.clone();
        async move { service.repositories(actor, 1).await }
    });
    gate.entered.notified().await;
    caller.abort();
    assert!(caller.await.unwrap_err().is_cancelled());
    gate.release.notify_one();
    service.repositories(actor, 2).await.unwrap();
    assert_eq!(provider.refresh_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn rejected_renewal_requests_reconnection() {
    let (service, _, provider, _, actor) = fixture();
    save_expired_token(&service, &provider, actor).await;
    provider.reject_refresh.store(true, Ordering::SeqCst);
    assert!(matches!(
        service.repositories(actor, 1).await,
        Err(RepositoryAccessError::ProviderUnauthorized)
    ));
}

#[tokio::test]
async fn login_saves_token_then_registers_key_once_environment_is_available() {
    let (service, store, provider, keys, actor) = fixture();
    let (token, identity) = provider.exchange("code", "verifier").await.unwrap();
    service
        .save_login(actor.actor, &identity, token)
        .await
        .unwrap();
    assert!(store.credential.lock().unwrap().is_some());
    assert_eq!(keys.calls.load(Ordering::SeqCst), 0);
    service.register_existing_key(actor).await.unwrap();
    assert_eq!(provider.registered.load(Ordering::SeqCst), 1);
    assert_eq!(keys.marked.load(Ordering::SeqCst), 1);
    provider.key_listing.store(1, Ordering::SeqCst);
    service.register_existing_key(actor).await.unwrap();
    assert_eq!(provider.registered.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn login_does_not_save_token_after_session_revocation_during_email_lookup() {
    let (service, store, provider, keys, actor) = fixture();
    provider.revoke_on_email.store(true, Ordering::SeqCst);
    let (token, identity) = provider.exchange("code", "verifier").await.unwrap();
    assert!(matches!(
        service.save_login(actor.actor, &identity, token).await,
        Err(RepositoryAccessError::Unauthorized)
    ));
    assert!(store.credential.lock().unwrap().is_none());
    assert_eq!(keys.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn wrong_identity_is_rejected_before_creating_or_registering_key() {
    let (service, store, provider, keys, actor) = fixture();
    store.identity_matches.store(false, Ordering::SeqCst);
    begin(&service, actor).await;
    assert!(matches!(
        service
            .complete(
                CredentialHash([1; 32]),
                CredentialHash([2; 32]),
                actor.actor.session_hash,
                "code"
            )
            .await,
        Err(RepositoryAccessError::IdentityMismatch)
    ));
    assert_eq!(keys.calls.load(Ordering::SeqCst), 0);
    assert_eq!(provider.registered.load(Ordering::SeqCst), 0);
    assert!(store.credential.lock().unwrap().is_none());
}

#[tokio::test]
async fn session_bound_attempt_is_consumed_once_and_revocation_fences_reads() {
    let (service, store, provider, _, actor) = fixture();
    begin(&service, actor).await;
    assert!(matches!(
        service
            .complete(
                CredentialHash([1; 32]),
                CredentialHash([2; 32]),
                CredentialHash([9; 32]),
                "code"
            )
            .await,
        Err(RepositoryAccessError::InvalidLogin)
    ));
    service
        .complete(
            CredentialHash([1; 32]),
            CredentialHash([2; 32]),
            actor.actor.session_hash,
            "code",
        )
        .await
        .unwrap();
    assert_eq!(provider.registered.load(Ordering::SeqCst), 1);
    assert_eq!(
        service.connection(actor).await.unwrap().unwrap().email,
        "user@example.test"
    );
    assert!(matches!(
        service
            .complete(
                CredentialHash([1; 32]),
                CredentialHash([2; 32]),
                actor.actor.session_hash,
                "code"
            )
            .await,
        Err(RepositoryAccessError::InvalidLogin)
    ));
    provider.revoke_on_list.store(true, Ordering::SeqCst);
    assert!(matches!(
        service.repositories(actor, 1).await,
        Err(RepositoryAccessError::Unauthorized)
    ));
    assert!(!store.active.load(Ordering::SeqCst));
}

#[tokio::test]
async fn rotates_only_when_registered_key_is_confirmed_absent() {
    for (listing, should_rotate, succeeds) in [(0, true, true), (1, false, true), (2, false, false)]
    {
        let (service, _, provider, keys, actor) = fixture();
        keys.registered.store(true, Ordering::SeqCst);
        provider.key_listing.store(listing, Ordering::SeqCst);
        begin(&service, actor).await;
        let result = service
            .complete(
                CredentialHash([1; 32]),
                CredentialHash([2; 32]),
                actor.actor.session_hash,
                "code",
            )
            .await;
        assert_eq!(result.is_ok(), succeeds);
        assert_eq!(
            keys.rotations.load(Ordering::SeqCst),
            usize::from(should_rotate)
        );
        assert_eq!(keys.marked.load(Ordering::SeqCst), usize::from(succeeds));
        assert_eq!(
            provider.registered.load(Ordering::SeqCst),
            usize::from(should_rotate)
        );
    }
}

#[tokio::test]
async fn disconnect_invalidates_consumed_callback_and_pending_attempt() {
    let (service, store, provider, keys, actor) = fixture();
    let service = Arc::new(service);
    let gate = Arc::new(Gate::new());
    *provider.exchange_gate.lock().unwrap() = Some(gate.clone());
    begin(&service, actor).await;
    let task = {
        let service = service.clone();
        tokio::spawn(async move {
            service
                .complete(
                    CredentialHash([1; 32]),
                    CredentialHash([2; 32]),
                    actor.actor.session_hash,
                    "code",
                )
                .await
        })
    };
    gate.entered.notified().await;
    service.disconnect(actor).await.unwrap();
    gate.release.notify_one();
    assert!(matches!(
        task.await.unwrap(),
        Err(RepositoryAccessError::ProviderUnauthorized)
    ));
    assert_eq!(keys.calls.load(Ordering::SeqCst), 0);
    assert_eq!(provider.registered.load(Ordering::SeqCst), 0);
    assert!(store.credential.lock().unwrap().is_none());
    *provider.exchange_gate.lock().unwrap() = None;
    begin(&service, actor).await;
    service.disconnect(actor).await.unwrap();
    assert!(matches!(
        service
            .complete(
                CredentialHash([1; 32]),
                CredentialHash([2; 32]),
                actor.actor.session_hash,
                "code"
            )
            .await,
        Err(RepositoryAccessError::InvalidLogin)
    ));
}

#[tokio::test]
async fn disconnect_fences_in_flight_listing_and_missing_credential_keeps_session() {
    let (service, _, provider, _, actor) = fixture();
    let service = Arc::new(service);
    assert!(matches!(
        service.repositories(actor, 1).await,
        Err(RepositoryAccessError::ProviderUnauthorized)
    ));
    begin(&service, actor).await;
    service
        .complete(
            CredentialHash([1; 32]),
            CredentialHash([2; 32]),
            actor.actor.session_hash,
            "code",
        )
        .await
        .unwrap();
    let gate = Arc::new(Gate::new());
    *provider.list_gate.lock().unwrap() = Some(gate.clone());
    let task = {
        let service = service.clone();
        tokio::spawn(async move { service.repositories(actor, 1).await })
    };
    gate.entered.notified().await;
    service.disconnect(actor).await.unwrap();
    gate.release.notify_one();
    assert!(matches!(
        task.await.unwrap(),
        Err(RepositoryAccessError::ProviderUnauthorized)
    ));
}

#[tokio::test]
async fn registration_locks_are_per_user_and_reclaim_idle_entries() {
    let (service, _, _, _, actor) = fixture();
    let first = service.registration_lock(actor.actor.user_id).unwrap();
    let same = service.registration_lock(actor.actor.user_id).unwrap();
    let other = service.registration_lock(Uuid::new_v4()).unwrap();
    let held = first.lock().await;
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(20), same.lock())
            .await
            .is_err()
    );
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(20), other.lock())
            .await
            .is_ok()
    );
    drop(held);
    drop(same);
    drop(first);
    drop(other);
    let _next = service.registration_lock(Uuid::new_v4()).unwrap();
    assert_eq!(service.registration.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn clone_origin_check_revalidates_ownership_and_epoch_after_provider_response() {
    use factory_core::projects::{Project, ProjectState};
    let (service, _, provider, _, actor) = fixture();
    let service = Arc::new(service);
    begin(&service, actor).await;
    service
        .complete(
            CredentialHash([1; 32]),
            CredentialHash([2; 32]),
            actor.actor.session_hash,
            "code",
        )
        .await
        .unwrap();
    let mut project = Project {
        id: Uuid::new_v4(),
        user_id: Uuid::new_v4(),
        repository_id: 123,
        full_name: "owner/repo".into(),
        cloned_full_name: "owner/repo".into(),
        directory: "unused".into(),
        state: ProjectState::Ready,
        operation_id: Uuid::new_v4(),
        environment_id: actor.environment_id,
        environment_generation: 1,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    assert!(matches!(
        service.verify_clone_origin(actor, &project).await,
        Err(RepositoryAccessError::Unauthorized)
    ));
    assert_eq!(provider.origin_calls.load(Ordering::SeqCst), 0);
    project.user_id = actor.actor.user_id;
    assert_eq!(
        service.verify_clone_origin(actor, &project).await.unwrap(),
        RepositoryOriginVerification::Matched
    );
    let gate = Arc::new(Gate::new());
    *provider.list_gate.lock().unwrap() = Some(gate.clone());
    let task = {
        let service = service.clone();
        tokio::spawn(async move { service.verify_clone_origin(actor, &project).await })
    };
    gate.entered.notified().await;
    service.disconnect(actor).await.unwrap();
    gate.release.notify_one();
    assert!(matches!(
        task.await.unwrap(),
        Err(RepositoryAccessError::ProviderUnauthorized)
    ));
}
