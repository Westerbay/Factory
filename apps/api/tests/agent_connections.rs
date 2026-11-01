use async_trait::async_trait;
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use factory_api::{agent_connections, auth::AuthState, config::RuntimeConfig};
use factory_application::{
    agent_connections::AgentConnectionService,
    environments::{EnvironmentAvailability, EnvironmentReconciler, EnvironmentService},
    identity::IdentityService,
};
use factory_core::{agent_connections::*, environments::*, identity::*};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    sync::Notify,
    task::JoinHandle,
};
use tower::ServiceExt;
use uuid::Uuid;
const ORIGIN: &str = "http://localhost:3000";
const CSRF: &str = "fixture-account-csrf";
const BASE: &str = "/api/agent-connections/codex";
struct Store {
    sessions: Mutex<HashMap<CredentialHash, ActiveSession>>,
    environment: Mutex<Environment>,
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
        let sessions = self.sessions.lock().unwrap();
        if !sessions.get(&actor.session_hash).is_some_and(|session| {
            session.user.id == actor.user_id
                && session.user.status == AccessStatus::Active
                && session.expires_at > Utc::now()
        }) {
            return Err(EnvironmentError::Unauthorized);
        }
        let environment = self.environment.lock().unwrap();
        Ok((environment.user_id == actor.user_id).then(|| environment.clone()))
    }
    async fn initialize_if_absent(
        &self,
        _: EnvironmentActor,
        _: EnvironmentStart,
    ) -> Result<Environment, EnvironmentError> {
        unreachable!()
    }
    async fn start(
        &self,
        _: EnvironmentActor,
        _: EnvironmentStart,
    ) -> Result<Environment, EnvironmentError> {
        unreachable!()
    }
    async fn stop(&self, _: EnvironmentActor) -> Result<Option<Environment>, EnvironmentError> {
        unreachable!()
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

struct Supervisor;
#[async_trait]
impl EnvironmentSupervisor for Supervisor {
    async fn check(&mut self) -> Result<(), EnvironmentError> {
        Ok(())
    }
}
struct NoEnvironmentRuntime;
#[async_trait]
impl EnvironmentRuntime for NoEnvironmentRuntime {
    async fn inspect(&self, _: &Environment) -> Result<RuntimeInspection, EnvironmentError> {
        unreachable!()
    }
    async fn create_volume(&self, _: &Environment) -> Result<(), EnvironmentError> {
        unreachable!()
    }
    async fn start(&self, _: &Environment) -> Result<(), EnvironmentError> {
        unreachable!()
    }
    async fn stop(&self, _: &Environment) -> Result<(), EnvironmentError> {
        unreachable!()
    }
}

#[derive(Default)]
struct Runtime {
    claude: bool,
    starts: AtomicUsize,
    hold_start: AtomicBool,
    release_start: Notify,
    cancelled: Mutex<Vec<Uuid>>,
    logouts: AtomicUsize,
    snapshot: Mutex<Option<ConnectionSnapshot>>,
    read_error: Mutex<Option<AgentConnectionError>>,
    hold_read: AtomicBool,
    reads: AtomicUsize,
    release_read: Notify,
    codes: AtomicUsize,
    hold_code: AtomicBool,
    release_code: Notify,
}
#[async_trait]
impl AgentConnectionPort for Runtime {
    async fn read(&self, _: &Environment) -> Result<ConnectionSnapshot, AgentConnectionError> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        if self.hold_read.load(Ordering::SeqCst) {
            self.release_read.notified().await;
        }
        if let Some(error) = *self.read_error.lock().unwrap() {
            return Err(error);
        }
        Ok(self
            .snapshot
            .lock()
            .unwrap()
            .clone()
            .unwrap_or_else(|| ConnectionSnapshot::account(None, Utc::now())))
    }
    async fn begin_login(
        &self,
        _: &Environment,
        operation_id: Uuid,
        expires_at: DateTime<Utc>,
    ) -> Result<ConnectionSnapshot, AgentConnectionError> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        if self.hold_start.load(Ordering::SeqCst) {
            self.release_start.notified().await;
        }
        let snapshot = ConnectionSnapshot {
            status: ConnectionStatus::Connecting,
            account: None,
            challenge: Some(LoginChallenge {
                operation_id,
                verification_url: if self.claude {
                    "https://claude.com/cai/oauth/authorize?redirect_uri=https%3A%2F%2Fplatform.claude.com%2Foauth%2Fcode%2Fcallback&response_type=code&code_challenge_method=S256&state=fixture&code_challenge=fixture"
                } else {
                    "https://auth.openai.com/codex/device"
                }
                .into(),
                user_code: (!self.claude).then(|| "FIXTURE-ONLY".into()),
                expires_at,
            }),
            error: None,
            observed_at: Utc::now(),
        };
        *self.snapshot.lock().unwrap() = Some(snapshot.clone());
        Ok(snapshot)
    }
    async fn cancel_login(
        &self,
        _: &Environment,
        operation_id: Uuid,
    ) -> Result<ConnectionSnapshot, AgentConnectionError> {
        self.cancelled.lock().unwrap().push(operation_id);
        let snapshot = ConnectionSnapshot::failure(AgentConnectionError::Cancelled, Utc::now());
        *self.snapshot.lock().unwrap() = Some(snapshot.clone());
        Ok(snapshot)
    }
    async fn submit_login_code(
        &self,
        _: &Environment,
        operation_id: Uuid,
        code: &str,
    ) -> Result<ConnectionSnapshot, AgentConnectionError> {
        let snapshot = self
            .snapshot
            .lock()
            .unwrap()
            .clone()
            .ok_or(AgentConnectionError::Conflict)?;
        if !self.claude
            || snapshot
                .challenge
                .as_ref()
                .is_none_or(|challenge| challenge.operation_id != operation_id)
            || code != "fixture-code"
        {
            return Err(AgentConnectionError::Conflict);
        }
        self.codes.fetch_add(1, Ordering::SeqCst);
        if self.hold_code.load(Ordering::SeqCst) {
            self.release_code.notified().await;
        }
        Ok(snapshot)
    }
    async fn logout(&self, _: &Environment) -> Result<ConnectionSnapshot, AgentConnectionError> {
        self.logouts.fetch_add(1, Ordering::SeqCst);
        let snapshot = ConnectionSnapshot::account(None, Utc::now());
        *self.snapshot.lock().unwrap() = Some(snapshot.clone());
        Ok(snapshot)
    }
}
struct Fixture {
    router: Router,
    auth: AuthState,
    store: Arc<Store>,
    runtime: Arc<Runtime>,
    claude_runtime: Arc<Runtime>,
    hash: CredentialHash,
    token: String,
    url: String,
    server: JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}
impl Fixture {
    async fn new(lifetime: ChronoDuration) -> Self {
        let token = URL_SAFE_NO_PAD.encode([7u8; 32]);
        let hash = CredentialHash(Sha256::digest(token.as_bytes()).into());
        let user = User {
            id: Uuid::new_v4(),
            display_name: "Fixture".into(),
            handle: "fixture".into(),
            provider: "github".into(),
            role: Role::Member,
            status: AccessStatus::Active,
            version: 1,
            access_enabled_at: Some(Utc::now()),
        };
        let availability = EnvironmentAvailability::default();
        let config = EnvironmentStart {
            installation_id: Uuid::new_v4(),
            image_id: "fixture-image".into(),
            policy_version: "2".into(),
        };
        let store = Arc::new(Store {
            sessions: Mutex::new(HashMap::from([(
                hash,
                ActiveSession {
                    user: user.clone(),
                    csrf_token: CSRF.into(),
                    expires_at: Utc::now() + lifetime,
                },
            )])),
            environment: Mutex::new(Environment {
                id: Uuid::new_v4(),
                user_id: user.id,
                installation_id: config.installation_id,
                desired_state: DesiredState::Running,
                observed_state: ObservedState::Running,
                generation: 1,
                provisioning_state: ProvisioningState::VolumeReady,
                volume_name: "private-volume".into(),
                container_name: "private-container".into(),
                image_id: config.image_id.clone(),
                policy_version: config.policy_version.clone(),
                observed_at: Some(Utc::now()),
                error_code: None,
                next_reconcile_at: Utc::now(),
            }),
        });
        EnvironmentReconciler::new(
            store.clone(),
            Arc::new(NoEnvironmentRuntime),
            Box::new(Supervisor),
            config.clone(),
            availability.clone(),
        )
        .unwrap()
        .next_batch()
        .await
        .unwrap();
        let environments =
            Arc::new(EnvironmentService::new(store.clone(), config, availability).unwrap());
        let runtime = Arc::new(Runtime::default());
        let service = Arc::new(AgentConnectionService::new(
            environments.clone(),
            runtime.clone(),
        ));
        let claude_runtime = Arc::new(Runtime {
            claude: true,
            ..Runtime::default()
        });
        let claude_service = Arc::new(AgentConnectionService::new(
            environments,
            claude_runtime.clone(),
        ));
        let auth = AuthState::new(
            Arc::new(IdentityService::new(store.clone(), Arc::new(NoProvider))),
            &RuntimeConfig::from_values(|_| None).unwrap(),
        );
        struct FixtureGuard(Arc<AgentConnectionService>);
        #[async_trait::async_trait]
        impl agent_connections::DisconnectGuard for FixtureGuard {
            async fn acquire_login(
                &self,
                _: factory_core::environments::EnvironmentActor,
            ) -> Result<Box<dyn Send>, factory_core::agent_connections::AgentConnectionError>
            {
                Ok(Box::new(()))
            }
            async fn logout(
                &self,
                actor: factory_core::environments::EnvironmentActor,
            ) -> Result<
                factory_core::agent_connections::ConnectionSnapshot,
                factory_core::agent_connections::AgentConnectionError,
            > {
                self.0.logout(actor).await
            }
        }
        let guard = Arc::new(FixtureGuard(service.clone()));
        let router = agent_connections::router_with_providers(
            auth.clone(),
            Some(service),
            Some(claude_service),
            guard,
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("{}", listener.local_addr().unwrap());
        let server_router = router.clone();
        let server = tokio::spawn(async move {
            axum::serve(listener, server_router).await.unwrap();
        });
        Self {
            router,
            auth,
            store,
            runtime,
            claude_runtime,
            hash,
            token,
            url,
            server,
        }
    }

    fn request(&self, method: &str, suffix: &str, body: &str) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(format!("{BASE}{suffix}"))
            .header("cookie", format!("factory_session={}", self.token))
            .header("origin", ORIGIN)
            .header("x-csrf-token", CSRF)
            .header("content-type", "application/json")
            .body(Body::from(body.to_owned()))
            .unwrap()
    }
    async fn call(&self, method: &str, suffix: &str, body: &str) -> (StatusCode, Value) {
        response(
            self.router
                .clone()
                .oneshot(self.request(method, suffix, body))
                .await
                .unwrap(),
        )
        .await
    }
    async fn login(&self) -> Value {
        let (status, body) = self.call("POST", "/login", "").await;
        assert_eq!(status, 202, "{body}");
        body
    }
    async fn start_http(&self) -> TcpStream {
        let mut stream = TcpStream::connect(&self.url).await.unwrap();
        stream.write_all(format!("POST {BASE}/login HTTP/1.1\r\nHost: localhost\r\nOrigin: {ORIGIN}\r\nX-CSRF-Token: {CSRF}\r\nCookie: factory_session={}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n", self.token).as_bytes()).await.unwrap();
        stream
    }
    fn second_session(&self, other_user: bool) -> String {
        let token = URL_SAFE_NO_PAD.encode([8u8; 32]);
        let hash = CredentialHash(Sha256::digest(token.as_bytes()).into());
        let mut sessions = self.store.sessions.lock().unwrap();
        let mut session = sessions.get(&self.hash).unwrap().clone();
        if other_user {
            session.user.id = Uuid::new_v4();
        }
        sessions.insert(hash, session);
        token
    }
}
async fn response(response: axum::response::Response) -> (StatusCode, Value) {
    assert_eq!(response.headers()["cache-control"], "no-store");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 8192).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}
async fn wait_for(mut condition: impl FnMut() -> bool, seconds: u64) {
    tokio::time::timeout(Duration::from_secs(seconds), async {
        while !condition() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("bounded completion");
}

#[tokio::test]
async fn mutations_require_live_identity_exact_origin_csrf_and_strict_bodies() {
    let fixture = Fixture::new(ChronoDuration::minutes(5)).await;
    for (name, value, status) in [
        ("origin", "https://evil.example", 403),
        ("x-csrf-token", "wrong", 403),
        ("cookie", "factory_session=invalid", 401),
    ] {
        let mut request = fixture.request("POST", "/login", "");
        request.headers_mut().insert(name, value.parse().unwrap());
        assert_eq!(
            response(fixture.router.clone().oneshot(request).await.unwrap())
                .await
                .0,
            status
        );
    }
    for (path, body) in [
        ("/login", "{}"),
        ("/logout", "{}"),
        ("/login/cancel", "{}"),
        ("/login/cancel", r#"{"operationId":"invalid"}"#),
    ] {
        assert_eq!(fixture.call("POST", path, body).await.0, 400);
    }
    assert_eq!(fixture.runtime.starts.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.runtime.logouts.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn challenge_is_session_owned_cancel_is_operation_specific_and_logout_is_confirmed() {
    let fixture = Fixture::new(ChronoDuration::minutes(5)).await;
    assert_eq!(
        fixture.call("GET", "", "").await.1["status"],
        "disconnected"
    );
    let login = fixture.login().await;
    assert_eq!(login["provider"], "codex");
    assert_eq!(login["status"], "connecting");
    let operation = login["challenge"]["operationId"].as_str().unwrap();
    let (_, read) = fixture.call("GET", "", "").await;
    assert_eq!(read["challenge"], login["challenge"]);
    assert_eq!(fixture.call("POST", "/login", "").await.0, 409);
    let other = fixture.second_session(false);
    for (method, path, body) in [
        ("GET", "", ""),
        ("POST", "/logout", ""),
        (
            "POST",
            "/login/cancel",
            &json!({"operationId":operation}).to_string(),
        ),
    ] {
        let mut request = fixture.request(method, path, body);
        request.headers_mut().insert(
            "cookie",
            format!("factory_session={other}").parse().unwrap(),
        );
        let (status, body) = response(fixture.router.clone().oneshot(request).await.unwrap()).await;
        assert_eq!(status, 409);
        assert!(!body.to_string().contains("FIXTURE-ONLY"));
    }
    assert_eq!(
        fixture
            .call(
                "POST",
                "/login/cancel",
                &json!({"operationId":Uuid::new_v4()}).to_string()
            )
            .await
            .0,
        409
    );
    let (status, cancelled) = fixture
        .call(
            "POST",
            "/login/cancel",
            &json!({"operationId":operation}).to_string(),
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(cancelled["errorCode"], "cancelled");
    assert!(cancelled["challenge"].is_null());
    assert_eq!(fixture.runtime.cancelled.lock().unwrap().len(), 1);
    assert_eq!(
        fixture.call("POST", "/logout", "").await.1["status"],
        "disconnected"
    );
    assert_eq!(fixture.runtime.logouts.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn another_user_never_reaches_the_owned_environment() {
    let fixture = Fixture::new(ChronoDuration::minutes(5)).await;
    fixture.login().await;
    let other = fixture.second_session(true);
    let mut request = fixture.request("POST", "/login", "");
    request.headers_mut().insert(
        "cookie",
        format!("factory_session={other}").parse().unwrap(),
    );
    let (status, body) = response(fixture.router.clone().oneshot(request).await.unwrap()).await;
    assert_eq!(status, 503);
    assert!(!body.to_string().contains("FIXTURE-ONLY"));
    assert_eq!(fixture.runtime.starts.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn revocation_during_http_start_responds_without_abandoning_controller_cleanup() {
    let fixture = Fixture::new(ChronoDuration::minutes(5)).await;
    fixture.runtime.hold_start.store(true, Ordering::SeqCst);
    let mut connection = fixture.start_http().await;
    wait_for(|| fixture.runtime.starts.load(Ordering::SeqCst) == 1, 2).await;
    fixture.auth.service().logout(fixture.hash).await.unwrap();
    let mut response = String::new();
    tokio::time::timeout(
        Duration::from_secs(2),
        connection.read_to_string(&mut response),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(response.starts_with("HTTP/1.1 401"), "{response}");
    assert!(!response.contains("FIXTURE-ONLY"));
    fixture.runtime.release_start.notify_one();
    wait_for(|| fixture.runtime.cancelled.lock().unwrap().len() == 1, 2).await;
}

#[tokio::test]
async fn disconnect_during_http_start_still_cleans_up_the_created_controller() {
    let fixture = Fixture::new(ChronoDuration::minutes(5)).await;
    fixture.runtime.hold_start.store(true, Ordering::SeqCst);
    let connection = fixture.start_http().await;
    wait_for(|| fixture.runtime.starts.load(Ordering::SeqCst) == 1, 2).await;
    drop(connection);
    tokio::time::sleep(Duration::from_millis(100)).await;
    fixture.runtime.release_start.notify_one();
    wait_for(|| fixture.runtime.cancelled.lock().unwrap().len() == 1, 2).await;
}

#[tokio::test]
async fn exact_session_expiry_interrupts_pending_login_before_the_poll_interval() {
    let fixture = Fixture::new(ChronoDuration::milliseconds(500)).await;
    let login = fixture.login().await;
    assert!(login["challenge"]["expiresAt"].is_string());
    wait_for(|| fixture.runtime.cancelled.lock().unwrap().len() == 1, 2).await;
    assert_eq!(fixture.call("GET", "", "").await.0, 401);
}

#[tokio::test]
async fn stopped_or_replaced_environment_interrupts_pending_login_and_blocks_new_actions() {
    for replacement in [false, true] {
        let fixture = Fixture::new(ChronoDuration::minutes(5)).await;
        fixture.login().await;
        {
            let mut environment = fixture.store.environment.lock().unwrap();
            if replacement {
                environment.generation += 1;
            } else {
                environment.desired_state = DesiredState::Stopped;
            }
        }
        assert!(
            fixture
                .call("POST", "/logout", "")
                .await
                .0
                .is_client_error()
        );
        wait_for(|| fixture.runtime.cancelled.lock().unwrap().len() == 1, 6).await;
        assert_eq!(fixture.runtime.logouts.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn provider_failures_are_bounded_projections_and_never_claim_disconnected() {
    let fixture = Fixture::new(ChronoDuration::minutes(5)).await;
    *fixture.runtime.read_error.lock().unwrap() = Some(AgentConnectionError::UnsupportedMethod);
    let (status, body) = fixture.call("GET", "", "").await;
    assert_eq!(status, 200);
    assert_eq!(body["status"], "error");
    assert_eq!(body["errorCode"], "unsupported_method");
    assert!(body["account"].is_null());
    assert!(body["challenge"].is_null());
}

#[tokio::test]
async fn revocation_during_account_read_never_returns_the_account() {
    let fixture = Fixture::new(ChronoDuration::minutes(5)).await;
    *fixture.runtime.snapshot.lock().unwrap() = Some(ConnectionSnapshot::account(
        Some(AccountSummary {
            email: Some("fixture@example.invalid".into()),
            plan: "plus".into(),
        }),
        Utc::now(),
    ));
    fixture.runtime.hold_read.store(true, Ordering::SeqCst);
    let request = fixture.request("GET", "", "");
    let pending = tokio::spawn(fixture.router.clone().oneshot(request));
    wait_for(|| fixture.runtime.reads.load(Ordering::SeqCst) == 1, 2).await;
    fixture.auth.service().logout(fixture.hash).await.unwrap();
    fixture.runtime.release_read.notify_one();
    let (status, body) = response(pending.await.unwrap().unwrap()).await;
    assert_eq!(status, 401);
    assert!(!body.to_string().contains("fixture@example.invalid"));
}

#[tokio::test]
async fn stop_during_start_never_publishes_a_challenge_and_cleans_up() {
    let fixture = Fixture::new(ChronoDuration::minutes(5)).await;
    fixture.runtime.hold_start.store(true, Ordering::SeqCst);
    let request = fixture.request("POST", "/login", "");
    let pending = tokio::spawn(fixture.router.clone().oneshot(request));
    wait_for(|| fixture.runtime.starts.load(Ordering::SeqCst) == 1, 2).await;
    fixture.store.environment.lock().unwrap().desired_state = DesiredState::Stopped;
    fixture.runtime.release_start.notify_one();
    let (status, body) = response(pending.await.unwrap().unwrap()).await;
    assert_eq!(status, 409);
    assert!(!body.to_string().contains("FIXTURE-ONLY"));
    wait_for(|| fixture.runtime.cancelled.lock().unwrap().len() == 1, 2).await;
}

#[tokio::test]
async fn confirmed_account_replaces_the_pending_challenge() {
    let fixture = Fixture::new(ChronoDuration::minutes(5)).await;
    fixture.login().await;
    *fixture.runtime.snapshot.lock().unwrap() = Some(ConnectionSnapshot::account(
        Some(AccountSummary {
            email: None,
            plan: "plus".into(),
        }),
        Utc::now(),
    ));
    let (status, body) = fixture.call("GET", "", "").await;
    assert_eq!(status, 200);
    assert_eq!(body["status"], "connected");
    assert_eq!(body["account"]["plan"], "plus");
    assert!(body["challenge"].is_null());
    assert!(body["errorCode"].is_null());
}

#[tokio::test]
async fn claude_connection_is_private_and_disconnects_independently() {
    let fixture = Fixture::new(ChronoDuration::minutes(5)).await;
    *fixture.runtime.snapshot.lock().unwrap() = Some(ConnectionSnapshot::account(
        Some(AccountSummary {
            email: None,
            plan: "codex".into(),
        }),
        Utc::now(),
    ));
    *fixture.claude_runtime.snapshot.lock().unwrap() = Some(ConnectionSnapshot::account(
        Some(AccountSummary {
            email: None,
            plan: "claude".into(),
        }),
        Utc::now(),
    ));
    let make = |method: &str, path: &str, csrf: bool| {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header("cookie", format!("factory_session={}", fixture.token))
            .header("origin", ORIGIN);
        if csrf {
            request = request.header("x-csrf-token", CSRF);
        }
        request.body(Body::empty()).unwrap()
    };
    let (status, value) = response(
        fixture
            .router
            .clone()
            .oneshot(make("GET", "/api/agent-connections/claude", false))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(value["provider"], "claude");
    assert_eq!(value["status"], "connected");
    let (status, _) = response(
        fixture
            .router
            .clone()
            .oneshot(make("POST", "/api/agent-connections/claude/logout", false))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(fixture.claude_runtime.logouts.load(Ordering::SeqCst), 0);
    let (status, value) = response(
        fixture
            .router
            .clone()
            .oneshot(make("POST", "/api/agent-connections/claude/logout", true))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(value["status"], "disconnected");
    assert_eq!(fixture.claude_runtime.logouts.load(Ordering::SeqCst), 1);
    let (status, codex) = fixture.call("GET", "", "").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(codex["status"], "connected");
}

#[tokio::test]
async fn claude_browser_code_is_session_scoped_and_requires_csrf() {
    let fixture = Fixture::new(ChronoDuration::minutes(5)).await;
    let request = |path: &str, body: &str| {
        let mut request = fixture.request("POST", "", body);
        *request.uri_mut() = path.parse().unwrap();
        request
    };
    let (status, started) = response(
        fixture
            .router
            .clone()
            .oneshot(request("/api/agent-connections/claude/login", ""))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(started["status"], "connecting");
    assert!(started["challenge"]["userCode"].is_null());
    let id = started["challenge"]["operationId"].as_str().unwrap();
    let body = json!({"operationId": id, "code": "fixture-code"}).to_string();
    let mut unauthorized = request("/api/agent-connections/claude/login/code", &body);
    unauthorized.headers_mut().insert(
        "cookie",
        format!("factory_session={}", fixture.second_session(false))
            .parse()
            .unwrap(),
    );
    assert_eq!(
        response(fixture.router.clone().oneshot(unauthorized).await.unwrap())
            .await
            .0,
        StatusCode::CONFLICT
    );
    let mut wrong_csrf = request("/api/agent-connections/claude/login/code", &body);
    wrong_csrf
        .headers_mut()
        .insert("x-csrf-token", "wrong".parse().unwrap());
    assert_eq!(
        response(fixture.router.clone().oneshot(wrong_csrf).await.unwrap())
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(fixture.claude_runtime.codes.load(Ordering::SeqCst), 0);
    let (status, result) = response(
        fixture
            .router
            .clone()
            .oneshot(request("/api/agent-connections/claude/login/code", &body))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(!result.to_string().contains("fixture-code"));
    assert_eq!(fixture.claude_runtime.codes.load(Ordering::SeqCst), 1);
    let cancel = json!({"operationId": id}).to_string();
    assert_eq!(
        response(
            fixture
                .router
                .clone()
                .oneshot(request(
                    "/api/agent-connections/claude/login/cancel",
                    &cancel
                ))
                .await
                .unwrap()
        )
        .await
        .0,
        StatusCode::OK
    );
}

#[tokio::test]
async fn revoked_session_never_receives_the_claude_challenge_after_submitting_a_code() {
    let fixture = Fixture::new(ChronoDuration::minutes(5)).await;
    let mut start = fixture.request("POST", "", "");
    *start.uri_mut() = "/api/agent-connections/claude/login".parse().unwrap();
    let (_, started) = response(fixture.router.clone().oneshot(start).await.unwrap()).await;
    let id = started["challenge"]["operationId"].as_str().unwrap();
    fixture
        .claude_runtime
        .hold_code
        .store(true, Ordering::SeqCst);
    let body = json!({"operationId": id, "code": "fixture-code"}).to_string();
    let mut submit = fixture.request("POST", "", &body);
    *submit.uri_mut() = "/api/agent-connections/claude/login/code".parse().unwrap();
    let router = fixture.router.clone();
    let pending =
        tokio::spawn(async move { response(router.oneshot(submit).await.unwrap()).await });
    wait_for(
        || fixture.claude_runtime.codes.load(Ordering::SeqCst) == 1,
        3,
    )
    .await;
    fixture.store.sessions.lock().unwrap().remove(&fixture.hash);
    fixture.claude_runtime.release_code.notify_one();
    let (status, value) = pending.await.unwrap();
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(value.get("challenge").is_none());
}
