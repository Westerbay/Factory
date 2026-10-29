use async_trait::async_trait;
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Method, Request, header},
    response::Response,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{Duration, Utc};
use factory_api::{auth::AuthState, config::RuntimeConfig, environments};
use factory_application::{
    environments::{EnvironmentAvailability, EnvironmentReconciler, EnvironmentService},
    identity::IdentityService,
};
use factory_core::{environments::*, identity::*};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};
use tower::ServiceExt;
use uuid::Uuid;

const ORIGIN: &str = "http://localhost:3000";
const CSRF: &str = "fixture-csrf-value";

#[derive(Default)]
struct Store {
    sessions: Mutex<HashMap<CredentialHash, ActiveSession>>,
    rows: Mutex<HashMap<Uuid, Environment>>,
    requests: AtomicUsize,
    revoke_at_transaction: AtomicBool,
}

impl Store {
    fn authorize(&self, actor: EnvironmentActor) -> Result<(), EnvironmentError> {
        if self.revoke_at_transaction.load(Ordering::SeqCst) {
            return Err(EnvironmentError::Unauthorized);
        }
        let sessions = self.sessions.lock().unwrap();
        let session = sessions
            .get(&actor.session_hash)
            .ok_or(EnvironmentError::Unauthorized)?;
        if session.user.id != actor.user_id
            || session.user.status != AccessStatus::Active
            || session.expires_at <= Utc::now()
        {
            return Err(EnvironmentError::Unauthorized);
        }
        self.requests.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[async_trait]
impl IdentityStore for Store {
    async fn create_login_attempt(&self, _: NewLoginAttempt) -> Result<(), IdentityError> {
        unreachable!()
    }
    async fn take_login_attempt(
        &self,
        _: CredentialHash,
        _: CredentialHash,
    ) -> Result<LoginAttempt, IdentityError> {
        unreachable!()
    }
    async fn admit(&self, _: LoginAdmission) -> Result<AdmissionOutcome, IdentityError> {
        unreachable!()
    }
    async fn active_session(
        &self,
        hash: CredentialHash,
    ) -> Result<Option<ActiveSession>, IdentityError> {
        Ok(self
            .sessions
            .lock()
            .unwrap()
            .get(&hash)
            .filter(|session| session.expires_at > Utc::now())
            .cloned())
    }
    async fn logout(&self, hash: CredentialHash) -> Result<(), IdentityError> {
        self.sessions.lock().unwrap().remove(&hash);
        Ok(())
    }
    async fn list_users(
        &self,
        _: CredentialHash,
        _: UserListQuery,
    ) -> Result<UserPage, IdentityError> {
        unreachable!()
    }
    async fn change_access(
        &self,
        _: CredentialHash,
        _: AccessRequest,
    ) -> Result<User, IdentityError> {
        unreachable!()
    }
}

struct NoProvider;
#[async_trait]
impl IdentityProvider for NoProvider {
    fn provider(&self) -> &str {
        "github"
    }
    fn issuer(&self) -> &str {
        "https://github.com"
    }
    fn authorization_url(&self, _: &str, _: &str) -> Result<String, IdentityError> {
        unreachable!()
    }
    async fn exchange(&self, _: &str, _: &str) -> Result<VerifiedIdentity, IdentityError> {
        unreachable!()
    }
}

#[async_trait]
impl EnvironmentStore for Store {
    async fn get(&self, actor: EnvironmentActor) -> Result<Option<Environment>, EnvironmentError> {
        self.authorize(actor)?;
        Ok(self.rows.lock().unwrap().get(&actor.user_id).cloned())
    }
    async fn start(
        &self,
        actor: EnvironmentActor,
        config: EnvironmentStart,
    ) -> Result<Environment, EnvironmentError> {
        self.authorize(actor)?;
        let mut rows = self.rows.lock().unwrap();
        let row = rows.entry(actor.user_id).or_insert_with(|| Environment {
            id: Uuid::new_v4(),
            user_id: actor.user_id,
            installation_id: config.installation_id,
            desired_state: DesiredState::Stopped,
            observed_state: ObservedState::NotCreated,
            generation: 1,
            provisioning_state: ProvisioningState::Unprovisioned,
            volume_name: "private-volume-name".into(),
            container_name: "private-container-name".into(),
            image_id: config.image_id,
            policy_version: config.policy_version,
            observed_at: None,
            error_code: None,
            next_reconcile_at: Utc::now(),
        });
        if row.desired_state != DesiredState::Running {
            row.generation += 1;
        }
        row.desired_state = DesiredState::Running;
        Ok(row.clone())
    }
    async fn stop(&self, actor: EnvironmentActor) -> Result<Option<Environment>, EnvironmentError> {
        self.authorize(actor)?;
        let mut rows = self.rows.lock().unwrap();
        Ok(rows.get_mut(&actor.user_id).map(|row| {
            if row.desired_state != DesiredState::Stopped {
                row.generation += 1;
            }
            row.desired_state = DesiredState::Stopped;
            row.clone()
        }))
    }
    async fn reconcile_batch(&self, _: Uuid, _: u32) -> Result<Vec<Environment>, EnvironmentError> {
        Ok(vec![])
    }
    async fn begin_provisioning(&self, _: Uuid, _: i64) -> Result<bool, EnvironmentError> {
        unreachable!()
    }
    async fn record_observation(
        &self,
        _: Uuid,
        _: i64,
        _: EnvironmentObservation,
        _: u32,
    ) -> Result<bool, EnvironmentError> {
        unreachable!()
    }
}

struct NoRuntime;
#[async_trait]
impl EnvironmentRuntime for NoRuntime {
    async fn inspect(&self, _: &Environment) -> Result<RuntimeInspection, EnvironmentError> {
        panic!("HTTP must never inspect Docker")
    }
    async fn create_volume(&self, _: &Environment) -> Result<(), EnvironmentError> {
        panic!("HTTP must never create Docker resources")
    }
    async fn start(&self, _: &Environment) -> Result<(), EnvironmentError> {
        panic!("HTTP must never start Docker")
    }
    async fn stop(&self, _: &Environment) -> Result<(), EnvironmentError> {
        panic!("HTTP must never stop Docker")
    }
}
struct Guard;
#[async_trait]
impl EnvironmentSupervisor for Guard {
    async fn check(&mut self) -> Result<(), EnvironmentError> {
        Ok(())
    }
}

struct Fixture {
    app: Router,
    store: Arc<Store>,
    auth: AuthState,
    availability: EnvironmentAvailability,
    cookies: Vec<String>,
    users: Vec<Uuid>,
}

impl Fixture {
    async fn new() -> Self {
        let store = Arc::new(Store::default());
        let mut cookies = vec![];
        let mut users = vec![];
        for index in 1..=3 {
            let token = URL_SAFE_NO_PAD.encode([index; 32]);
            let hash = CredentialHash(Sha256::digest(token.as_bytes()).into());
            let id = Uuid::new_v4();
            users.push(id);
            cookies.push(format!("factory_session={token}"));
            store.sessions.lock().unwrap().insert(
                hash,
                ActiveSession {
                    user: User {
                        id,
                        display_name: "Fixture member".into(),
                        handle: "fixture".into(),
                        provider: "github".into(),
                        role: Role::Member,
                        status: AccessStatus::Active,
                        version: 1,
                        access_enabled_at: Some(Utc::now()),
                    },
                    csrf_token: CSRF.into(),
                    expires_at: Utc::now() + Duration::hours(1),
                },
            );
        }
        let config = RuntimeConfig::from_values(|_| None).unwrap();
        let auth = AuthState::new(
            Arc::new(IdentityService::new(store.clone(), Arc::new(NoProvider))),
            &config,
        );
        let availability = EnvironmentAvailability::default();
        let start = EnvironmentStart {
            installation_id: Uuid::new_v4(),
            image_id: "private-image-id".into(),
            policy_version: "1".into(),
        };
        EnvironmentReconciler::new(
            store.clone(),
            Arc::new(NoRuntime),
            Box::new(Guard),
            start.clone(),
            availability.clone(),
        )
        .unwrap()
        .next_batch()
        .await
        .unwrap();
        let service =
            Arc::new(EnvironmentService::new(store.clone(), start, availability.clone()).unwrap());
        let app = environments::router(auth.clone(), Some(service));
        Self {
            app,
            store,
            auth,
            availability,
            cookies,
            users,
        }
    }

    async fn call(&self, method: Method, path: &str, user: usize, body: &str) -> Response {
        let request = Request::builder()
            .method(method)
            .uri(path)
            .header(header::COOKIE, &self.cookies[user])
            .header(header::ORIGIN, ORIGIN)
            .header("x-csrf-token", CSRF)
            .body(Body::from(body.to_owned()))
            .unwrap();
        self.app.clone().oneshot(request).await.unwrap()
    }
}

async fn json(response: Response) -> Value {
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    serde_json::from_slice(&to_bytes(response.into_body(), 8192).await.unwrap()).unwrap()
}

#[tokio::test]
async fn reads_are_actor_scoped_and_never_create_or_expose_internal_identifiers() {
    let f = Fixture::new().await;
    assert_eq!(
        json(f.call(Method::GET, "/api/environment", 0, "").await).await["status"],
        "not_created"
    );
    assert!(f.store.rows.lock().unwrap().is_empty());
    let response = f.call(Method::POST, "/api/environment/start", 0, "").await;
    assert_eq!(response.status(), 202);
    let body = json(response).await;
    assert_eq!(body["status"], "starting");
    assert_eq!(body.as_object().unwrap().len(), 3);
    assert!(!body.to_string().contains("private-"));
    assert_eq!(
        json(f.call(Method::GET, "/api/environment", 1, "").await).await["status"],
        "not_created"
    );
    assert_eq!(f.store.rows.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn every_route_requires_a_live_active_session_and_store_rechecks_mutations() {
    let f = Fixture::new().await;
    for (method, path) in [
        (Method::GET, "/api/environment"),
        (Method::POST, "/api/environment/start"),
        (Method::POST, "/api/environment/stop"),
    ] {
        let response = f
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(path)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 401);
    }
    f.store.revoke_at_transaction.store(true, Ordering::SeqCst);
    assert_eq!(
        f.call(Method::POST, "/api/environment/start", 0, "")
            .await
            .status(),
        401
    );
    assert!(f.store.rows.lock().unwrap().is_empty());
    f.store.revoke_at_transaction.store(false, Ordering::SeqCst);
    for session in f.store.sessions.lock().unwrap().values_mut() {
        session.user.status = AccessStatus::Suspended;
    }
    assert_eq!(
        f.call(Method::GET, "/api/environment", 0, "")
            .await
            .status(),
        401
    );
}

#[tokio::test]
async fn mutations_reject_csrf_origin_and_all_nonempty_bodies_before_store_calls() {
    let f = Fixture::new().await;
    for path in ["/api/environment/start", "/api/environment/stop"] {
        for (origin, csrf) in [
            ("https://evil.example", CSRF),
            (ORIGIN, "wrong"),
            ("", CSRF),
        ] {
            let response = f
                .app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(Method::POST)
                        .uri(path)
                        .header(header::COOKIE, &f.cookies[0])
                        .header(header::ORIGIN, origin)
                        .header("x-csrf-token", csrf)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), 403);
        }
        for body in [
            "{}".to_owned(),
            format!("{{\"userId\":\"{}\"}}", f.users[1]),
            "secret".repeat(4096),
        ] {
            assert_eq!(f.call(Method::POST, path, 0, &body).await.status(), 400);
        }
    }
    assert_eq!(f.store.requests.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn repeated_start_is_idempotent_and_more_than_two_users_can_start() {
    let f = Fixture::new().await;
    f.call(Method::POST, "/api/environment/start", 0, "").await;
    let generation = f.store.rows.lock().unwrap()[&f.users[0]].generation;
    f.call(Method::POST, "/api/environment/start", 0, "").await;
    assert_eq!(
        f.store.rows.lock().unwrap()[&f.users[0]].generation,
        generation
    );
    for user in [1, 2] {
        assert_eq!(
            f.call(Method::POST, "/api/environment/start", user, "")
                .await
                .status(),
            202
        );
    }
    assert_eq!(f.store.rows.lock().unwrap().len(), 3);
    assert_eq!(
        f.call(Method::POST, "/api/environment/stop", 0, "")
            .await
            .status(),
        202
    );
    assert_eq!(
        f.store.rows.lock().unwrap()[&f.users[0]].desired_state,
        DesiredState::Stopped
    );
    assert_eq!(
        f.call(Method::POST, "/api/environment/heartbeat", 0, "")
            .await
            .status(),
        404
    );
}

#[tokio::test]
async fn unprepared_or_lost_supervisor_is_honest_and_preserves_last_observation() {
    let f = Fixture::new().await;
    let unprepared = environments::router(f.auth.clone(), None);
    let response = unprepared
        .oneshot(
            Request::builder()
                .uri("/api/environment")
                .header(header::COOKIE, &f.cookies[0])
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(json(response).await["status"], "unavailable");
    f.call(Method::POST, "/api/environment/start", 0, "").await;
    let observed = Utc::now();
    {
        let mut rows = f.store.rows.lock().unwrap();
        let row = rows.get_mut(&f.users[0]).unwrap();
        row.observed_state = ObservedState::Running;
        row.observed_at = Some(observed);
    }
    f.availability.fail_closed();
    let state = json(f.call(Method::GET, "/api/environment", 0, "").await).await;
    assert_eq!(state["status"], "unavailable");
    assert_eq!(state["observedAt"], serde_json::to_value(observed).unwrap());
    assert_eq!(
        f.call(Method::POST, "/api/environment/start", 0, "")
            .await
            .status(),
        503
    );
    // Persisting a stop remains safe and useful for the next owner.
    assert_eq!(
        f.call(Method::POST, "/api/environment/stop", 0, "")
            .await
            .status(),
        202
    );
}
