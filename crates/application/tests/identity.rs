use async_trait::async_trait;
use chrono::{Duration, TimeZone, Utc};
use factory_application::identity::IdentityService;
use factory_core::identity::*;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use uuid::Uuid;

fn user(role: Role, status: AccessStatus) -> User {
    User {
        id: Uuid::new_v4(),
        display_name: "Test user".into(),
        handle: "test-user".into(),
        provider: "github".into(),
        role,
        status,
        version: 1,
        access_enabled_at: Some(Utc.timestamp_opt(1_000, 0).unwrap()),
    }
}

fn identity() -> VerifiedIdentity {
    VerifiedIdentity {
        provider: "github".into(),
        issuer: "https://github.com".into(),
        subject: "123".into(),
        display_name: "Test user".into(),
        handle: "test-user".into(),
    }
}

fn new_session() -> NewSession {
    NewSession {
        token_hash: CredentialHash([3; 32]),
        csrf_token: "x".repeat(43),
        previous_token_hash: None,
    }
}

#[derive(Default)]
struct MemoryStore {
    attempt: Mutex<Option<LoginAttempt>>,
    session: Mutex<Option<ActiveSession>>,
    admissions: AtomicUsize,
    admin_writes: AtomicUsize,
}

#[async_trait]
impl IdentityStore for MemoryStore {
    async fn create_login_attempt(&self, attempt: NewLoginAttempt) -> Result<(), IdentityError> {
        *self.attempt.lock().unwrap() = Some(LoginAttempt {
            pkce_verifier: attempt.pkce_verifier,
            created_at: Utc.timestamp_opt(2_000, 0).unwrap(),
        });
        Ok(())
    }
    async fn take_login_attempt(
        &self,
        _: CredentialHash,
        _: CredentialHash,
    ) -> Result<LoginAttempt, IdentityError> {
        self.attempt
            .lock()
            .unwrap()
            .take()
            .ok_or(IdentityError::InvalidLogin)
    }
    async fn admit(&self, _: LoginAdmission) -> Result<AdmissionOutcome, IdentityError> {
        self.admissions.fetch_add(1, Ordering::SeqCst);
        Ok(AdmissionOutcome::Pending)
    }
    async fn active_session(
        &self,
        _: CredentialHash,
    ) -> Result<Option<ActiveSession>, IdentityError> {
        Ok(self.session.lock().unwrap().clone())
    }
    async fn logout(&self, _: CredentialHash) -> Result<(), IdentityError> {
        *self.session.lock().unwrap() = None;
        Ok(())
    }
    async fn list_users(
        &self,
        _: CredentialHash,
        _: UserListQuery,
    ) -> Result<UserPage, IdentityError> {
        Ok(UserPage {
            items: vec![],
            next_cursor: None,
        })
    }
    async fn change_access(
        &self,
        _: CredentialHash,
        _: AccessRequest,
    ) -> Result<User, IdentityError> {
        self.admin_writes.fetch_add(1, Ordering::SeqCst);
        Err(IdentityError::Unavailable)
    }
}

#[derive(Default)]
struct Provider {
    fail: AtomicBool,
    spoof: AtomicBool,
    exchanges: AtomicUsize,
}

#[async_trait]
impl IdentityProvider for Provider {
    fn provider(&self) -> &str {
        "github"
    }
    fn issuer(&self) -> &str {
        "https://github.com"
    }
    fn authorization_url(&self, _: &str, _: &str) -> Result<String, IdentityError> {
        Ok("https://github.com/login/oauth/authorize".into())
    }
    async fn exchange(&self, _: &str, _: &str) -> Result<VerifiedIdentity, IdentityError> {
        self.exchanges.fetch_add(1, Ordering::SeqCst);
        if self.fail.load(Ordering::SeqCst) {
            return Err(IdentityError::Unavailable);
        }
        let mut value = identity();
        if self.spoof.load(Ordering::SeqCst) {
            value.issuer = "https://other.example".into();
        }
        Ok(value)
    }
}

async fn fixture() -> (IdentityService, Arc<MemoryStore>, Arc<Provider>) {
    let store = Arc::new(MemoryStore::default());
    let provider = Arc::new(Provider::default());
    let service = IdentityService::new(store.clone(), provider.clone());
    service
        .begin_login(
            NewLoginAttempt {
                state_hash: CredentialHash([1; 32]),
                binding_hash: CredentialHash([2; 32]),
                pkce_verifier: "v".repeat(43),
            },
            &"s".repeat(43),
            &"c".repeat(43),
        )
        .await
        .unwrap();
    (service, store, provider)
}

#[tokio::test]
async fn verified_identity_reaches_admission_once_and_pending_is_not_a_session() {
    let (service, store, provider) = fixture().await;
    assert!(matches!(
        service
            .complete_login(
                CredentialHash([1; 32]),
                CredentialHash([2; 32]),
                "code",
                new_session()
            )
            .await,
        Ok(AdmissionOutcome::Pending)
    ));
    assert_eq!(store.admissions.load(Ordering::SeqCst), 1);
    assert_eq!(provider.exchanges.load(Ordering::SeqCst), 1);
    assert!(matches!(
        service.session(CredentialHash([3; 32])).await,
        Err(IdentityError::Unauthorized)
    ));
}

#[tokio::test]
async fn failed_exchange_consumes_attempt_and_never_retries_code() {
    let (service, store, provider) = fixture().await;
    provider.fail.store(true, Ordering::SeqCst);
    assert!(matches!(
        service
            .complete_login(
                CredentialHash([1; 32]),
                CredentialHash([2; 32]),
                "code",
                new_session()
            )
            .await,
        Err(IdentityError::Unavailable)
    ));
    provider.fail.store(false, Ordering::SeqCst);
    assert!(matches!(
        service
            .complete_login(
                CredentialHash([1; 32]),
                CredentialHash([2; 32]),
                "code",
                new_session()
            )
            .await,
        Err(IdentityError::InvalidLogin)
    ));
    assert_eq!(provider.exchanges.load(Ordering::SeqCst), 1);
    assert_eq!(store.admissions.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn cancellation_consumes_the_attempt_without_exchange_or_admission() {
    let (service, store, provider) = fixture().await;
    service
        .cancel_login(CredentialHash([1; 32]), CredentialHash([2; 32]))
        .await
        .unwrap();
    assert_eq!(
        service
            .cancel_login(CredentialHash([1; 32]), CredentialHash([2; 32]))
            .await,
        Err(IdentityError::InvalidLogin)
    );
    assert!(matches!(
        service
            .complete_login(
                CredentialHash([1; 32]),
                CredentialHash([2; 32]),
                "code",
                new_session()
            )
            .await,
        Err(IdentityError::InvalidLogin)
    ));
    assert_eq!(provider.exchanges.load(Ordering::SeqCst), 0);
    assert_eq!(store.admissions.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn verified_profile_cannot_cross_provider_namespace() {
    let (service, store, provider) = fixture().await;
    provider.spoof.store(true, Ordering::SeqCst);
    assert!(matches!(
        service
            .complete_login(
                CredentialHash([1; 32]),
                CredentialHash([2; 32]),
                "code",
                new_session()
            )
            .await,
        Err(IdentityError::InvalidLogin)
    ));
    assert_eq!(store.admissions.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn member_or_revoked_session_cannot_request_access_changes() {
    let (service, store, _) = fixture().await;
    *store.session.lock().unwrap() = Some(ActiveSession {
        user: user(Role::Member, AccessStatus::Active),
        csrf_token: "csrf".into(),
        expires_at: Utc::now() + Duration::days(1),
    });
    let request = AccessRequest {
        target_id: Uuid::new_v4(),
        status: AccessDecision::Active,
        expected_version: 1,
    };
    assert!(matches!(
        service
            .change_access(CredentialHash([3; 32]), request.clone())
            .await,
        Err(IdentityError::Forbidden)
    ));
    service.logout(CredentialHash([3; 32])).await.unwrap();
    assert!(matches!(
        service
            .change_access(CredentialHash([3; 32]), request)
            .await,
        Err(IdentityError::Unauthorized)
    ));
    assert_eq!(store.admin_writes.load(Ordering::SeqCst), 0);
}

#[test]
fn admission_requires_latest_activation_even_for_the_administrator() {
    let old = Utc.timestamp_opt(999, 0).unwrap();
    for role in [Role::Member, Role::Admin] {
        let mut account = user(role, AccessStatus::Active);
        assert_eq!(
            admission_decision(&account, old),
            AdmissionDecision::StaleLogin
        );
        assert_eq!(
            admission_decision(&account, Utc::now()),
            AdmissionDecision::Active
        );
        account.status = AccessStatus::Suspended;
        assert_eq!(
            admission_decision(&account, Utc::now()),
            AdmissionDecision::Suspended
        );
    }
}

#[test]
fn access_policy_rejects_stale_versions_and_admin_targets_without_resetting_noops() {
    let actor = user(Role::Admin, AccessStatus::Active);
    let target = user(Role::Member, AccessStatus::Active);
    let mut request = AccessRequest {
        target_id: target.id,
        status: AccessDecision::Active,
        expected_version: 1,
    };
    assert_eq!(
        authorize_access_change(&actor, &target, &request),
        Ok(false)
    );
    request.expected_version = 2;
    assert_eq!(
        authorize_access_change(&actor, &target, &request),
        Err(IdentityError::Conflict)
    );
    request.target_id = actor.id;
    request.expected_version = 1;
    request.status = AccessDecision::Suspended;
    assert_eq!(
        authorize_access_change(&actor, &actor, &request),
        Err(IdentityError::ProtectedAdministrator)
    );
}
