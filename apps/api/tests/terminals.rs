use async_trait::async_trait;
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{Duration as ChronoDuration, Utc};
use factory_api::{auth::AuthState, config::RuntimeConfig, terminals};
use factory_application::{
    environments::{EnvironmentAvailability, EnvironmentReconciler, EnvironmentService},
    identity::IdentityService,
    terminals::TerminalService,
};
use factory_core::{environments::*, identity::*, terminals::*};
use futures_util::{SinkExt, StreamExt};
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
    net::TcpStream,
    sync::{Notify, mpsc},
    task::JoinHandle,
};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream,
    tungstenite::{Message, client::IntoClientRequest},
};
use tower::ServiceExt;
use uuid::Uuid;

const ORIGIN: &str = "http://localhost:3000";
const CSRF: &str = "fixture-terminal-csrf";
type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

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
    attached: AtomicUsize,
    hold_attach: AtomicBool,
    release_attach: Notify,
    detached: Mutex<Vec<String>>,
    inputs: Arc<Mutex<Vec<Vec<u8>>>>,
    sizes: Mutex<Vec<(u16, u16)>>,
    outputs: Mutex<HashMap<String, mpsc::Sender<Vec<u8>>>>,
}
struct Input(Arc<Mutex<Vec<Vec<u8>>>>);
#[async_trait]
impl TerminalInput for Input {
    async fn write(&mut self, input: &[u8]) -> Result<(), TerminalError> {
        self.0.lock().unwrap().push(input.to_vec());
        Ok(())
    }
}
struct Output(mpsc::Receiver<Vec<u8>>);
#[async_trait]
impl TerminalOutput for Output {
    async fn read(&mut self) -> Result<Option<Vec<u8>>, TerminalError> {
        Ok(self.0.recv().await)
    }
}
#[async_trait]
impl TerminalRuntime for Runtime {
    async fn attach(&self, _: &Environment) -> Result<TerminalProcess, TerminalError> {
        let handle = self.attached.fetch_add(1, Ordering::SeqCst).to_string();
        let (sender, receiver) = mpsc::channel(32);
        self.outputs.lock().unwrap().insert(handle.clone(), sender);
        if self.hold_attach.load(Ordering::SeqCst) {
            self.release_attach.notified().await;
        }
        Ok(TerminalProcess {
            handle,
            input: Box::new(Input(self.inputs.clone())),
            output: Box::new(Output(receiver)),
        })
    }
    async fn resize(&self, _: &str, size: TerminalSize) -> Result<(), TerminalError> {
        self.sizes.lock().unwrap().push((size.cols, size.rows));
        Ok(())
    }
    async fn detach(&self, _: &Environment, handle: &str) -> Result<(), TerminalError> {
        self.detached.lock().unwrap().push(handle.to_owned());
        self.outputs.lock().unwrap().remove(handle);
        Ok(())
    }
}

struct Fixture {
    router: Router,
    auth: AuthState,
    store: Arc<Store>,
    runtime: Arc<Runtime>,
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
        let service = Arc::new(TerminalService::new(environments, runtime.clone()));
        let auth = AuthState::new(
            Arc::new(IdentityService::new(store.clone(), Arc::new(NoProvider))),
            &RuntimeConfig::from_values(|_| None).unwrap(),
        );
        let router = terminals::router(auth.clone(), Some(service));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!(
            "ws://{}/api/terminal/socket",
            listener.local_addr().unwrap()
        );
        let server_router = router.clone();
        let server = tokio::spawn(async move {
            axum::serve(listener, server_router).await.unwrap();
        });
        Self {
            router,
            auth,
            store,
            runtime,
            hash,
            token,
            url,
            server,
        }
    }

    async fn issue(&self, takeover: bool, origin: &str, csrf: &str) -> (StatusCode, Value) {
        let request = Request::builder()
            .method("POST")
            .uri("/api/terminal/attachments")
            .header("cookie", format!("factory_session={}", self.token))
            .header("origin", origin)
            .header("x-csrf-token", csrf)
            .header("content-type", "application/json")
            .body(Body::from(json!({"takeOver":takeover}).to_string()))
            .unwrap();
        let response = self.router.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let data =
            serde_json::from_slice(&to_bytes(response.into_body(), 8192).await.unwrap()).unwrap();
        (status, data)
    }

    async fn ticket(&self, takeover: bool) -> String {
        let (status, body) = self.issue(takeover, ORIGIN, CSRF).await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        body["ticket"].as_str().unwrap().to_owned()
    }

    async fn socket(&self) -> Socket {
        let mut request = self.url.clone().into_client_request().unwrap();
        request
            .headers_mut()
            .insert("origin", ORIGIN.parse().unwrap());
        request.headers_mut().insert(
            "cookie",
            format!("factory_session={}", self.token).parse().unwrap(),
        );
        request.headers_mut().insert(
            "sec-websocket-protocol",
            "factory.terminal.v1".parse().unwrap(),
        );
        let (socket, response) = tokio_tungstenite::connect_async(request).await.unwrap();
        assert_eq!(
            response.headers()["sec-websocket-protocol"],
            "factory.terminal.v1"
        );
        socket
    }

    async fn attach(&self, takeover: bool) -> Socket {
        let ticket = self.ticket(takeover).await;
        let mut socket = self.socket().await;
        send(&mut socket, json!({"type":"authenticate","ticket":ticket})).await;
        assert_eq!(control(&mut socket).await["type"], "ready");
        socket
    }
}

async fn send(socket: &mut Socket, value: Value) {
    socket
        .send(Message::Text(value.to_string().into()))
        .await
        .unwrap();
}
async fn next(socket: &mut Socket) -> Message {
    tokio::time::timeout(Duration::from_secs(7), socket.next())
        .await
        .expect("bounded server response")
        .expect("socket not closed yet")
        .unwrap()
}
async fn control(socket: &mut Socket) -> Value {
    let message = next(socket).await;
    serde_json::from_str(message.to_text().unwrap()).unwrap()
}

#[tokio::test]
async fn origin_csrf_and_websocket_headers_are_required_before_exec() {
    let fixture = Fixture::new(ChronoDuration::minutes(5)).await;
    for (origin, csrf) in [
        ("https://evil.example", CSRF),
        ("null", CSRF),
        (ORIGIN, "incorrect"),
    ] {
        assert_eq!(
            fixture.issue(false, origin, csrf).await.0,
            StatusCode::FORBIDDEN
        );
    }
    for (origin, protocol, query) in [
        ("https://evil.example", "factory.terminal.v1", ""),
        (ORIGIN, "invalid", ""),
        (ORIGIN, "factory.terminal.v1", "?ticket=forbidden"),
    ] {
        let mut request = format!("{}{query}", fixture.url)
            .into_client_request()
            .unwrap();
        request
            .headers_mut()
            .insert("origin", origin.parse().unwrap());
        request.headers_mut().insert(
            "cookie",
            format!("factory_session={}", fixture.token)
                .parse()
                .unwrap(),
        );
        request
            .headers_mut()
            .insert("sec-websocket-protocol", protocol.parse().unwrap());
        let expected = if origin == ORIGIN {
            StatusCode::BAD_REQUEST
        } else {
            StatusCode::FORBIDDEN
        };
        match tokio_tungstenite::connect_async(request).await {
            Err(tokio_tungstenite::tungstenite::Error::Http(response)) => {
                assert_eq!(response.status(), expected)
            }
            other => panic!("expected an HTTP rejection, got {other:?}"),
        }
    }
    assert_eq!(fixture.runtime.attached.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn ticket_is_single_use_and_no_process_starts_before_authentication() {
    let fixture = Fixture::new(ChronoDuration::minutes(5)).await;
    let ticket = fixture.ticket(false).await;
    let mut first = fixture.socket().await;
    assert_eq!(fixture.runtime.attached.load(Ordering::SeqCst), 0);
    send(&mut first, json!({"type":"authenticate","ticket":ticket})).await;
    assert_eq!(control(&mut first).await["type"], "ready");
    let mut replay = fixture.socket().await;
    send(&mut replay, json!({"type":"authenticate","ticket":ticket})).await;
    assert_eq!(control(&mut replay).await["code"], "invalid_ticket");
    assert_eq!(fixture.runtime.attached.load(Ordering::SeqCst), 1);
    let mut invalid = fixture.socket().await;
    send(&mut invalid, json!({"type":"input","data":"must not run"})).await;
    assert_eq!(control(&mut invalid).await["code"], "invalid_ticket");
    assert!(fixture.runtime.inputs.lock().unwrap().is_empty());
}

#[tokio::test]
async fn explicit_takeover_replaces_only_the_old_attachment_and_preserves_io() {
    let fixture = Fixture::new(ChronoDuration::minutes(5)).await;
    let mut first = fixture.attach(false).await;
    assert_eq!(
        fixture.issue(false, ORIGIN, CSRF).await.0,
        StatusCode::CONFLICT
    );
    let mut second = fixture.attach(true).await;
    let old = control(&mut first).await;
    assert_eq!(old["type"], "detached");
    send(
        &mut second,
        json!({"type":"input","data":"echo connected\r"}),
    )
    .await;
    send(&mut second, json!({"type":"resize","cols":120,"rows":35})).await;
    let output = fixture
        .runtime
        .outputs
        .lock()
        .unwrap()
        .get("1")
        .unwrap()
        .clone();
    output.send(b"connected\r\n".to_vec()).await.unwrap();
    assert_eq!(
        next(&mut second).await.into_data().as_ref(),
        b"connected\r\n"
    );
    send(&mut second, json!({"type":"ack","bytes":11})).await;
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if !fixture.runtime.sizes.lock().unwrap().is_empty() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        *fixture.runtime.inputs.lock().unwrap(),
        vec![b"echo connected\r".to_vec()]
    );
    assert_eq!(*fixture.runtime.sizes.lock().unwrap(), vec![(120, 35)]);
    assert_eq!(*fixture.runtime.detached.lock().unwrap(), vec!["0"]);
}

#[tokio::test]
async fn local_logout_closes_passive_socket_without_waiting_for_the_poll() {
    let fixture = Fixture::new(ChronoDuration::minutes(5)).await;
    let mut socket = fixture.attach(false).await;
    fixture.auth.service().logout(fixture.hash).await.unwrap();
    let message = tokio::time::timeout(Duration::from_secs(2), control(&mut socket))
        .await
        .unwrap();
    assert_eq!(message["code"], "unauthorized");
    assert!(fixture.runtime.inputs.lock().unwrap().is_empty());
}

#[tokio::test]
async fn logout_during_process_opening_does_not_abandon_the_created_attachment() {
    let fixture = Fixture::new(ChronoDuration::minutes(5)).await;
    fixture.runtime.hold_attach.store(true, Ordering::SeqCst);
    let ticket = fixture.ticket(false).await;
    let mut socket = fixture.socket().await;
    send(&mut socket, json!({"type":"authenticate","ticket":ticket})).await;
    tokio::time::timeout(Duration::from_secs(2), async {
        while fixture.runtime.attached.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    fixture.auth.service().logout(fixture.hash).await.unwrap();
    let message = tokio::time::timeout(Duration::from_secs(2), control(&mut socket))
        .await
        .unwrap();
    assert_eq!(message["code"], "unauthorized");
    fixture.runtime.release_attach.notify_one();
    tokio::time::timeout(Duration::from_secs(2), async {
        while fixture.runtime.detached.lock().unwrap().is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the in-flight process must still be collected and detached");
    assert_eq!(*fixture.runtime.detached.lock().unwrap(), vec!["0"]);
    assert!(fixture.runtime.inputs.lock().unwrap().is_empty());
}

#[tokio::test]
async fn database_revocation_and_generation_changes_are_checked_before_input() {
    for revoke in [true, false] {
        let fixture = Fixture::new(ChronoDuration::minutes(5)).await;
        let mut socket = fixture.attach(false).await;
        if revoke {
            fixture.store.sessions.lock().unwrap().clear();
        } else {
            fixture.store.environment.lock().unwrap().generation += 1;
        }
        send(&mut socket, json!({"type":"input","data":"must not run\r"})).await;
        let message = control(&mut socket).await;
        assert!(matches!(
            message["type"].as_str(),
            Some("error" | "detached")
        ));
        assert!(fixture.runtime.inputs.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn session_expiry_closes_before_the_five_second_poll() {
    let fixture = Fixture::new(ChronoDuration::seconds(2)).await;
    let mut socket = fixture.attach(false).await;
    let message = tokio::time::timeout(Duration::from_secs(3), control(&mut socket))
        .await
        .unwrap();
    assert_eq!(message["code"], "unauthorized");
}

#[tokio::test]
async fn unacknowledged_output_is_bounded_and_slow_client_is_detached() {
    let fixture = Fixture::new(ChronoDuration::minutes(5)).await;
    let mut socket = fixture.attach(false).await;
    let output = fixture
        .runtime
        .outputs
        .lock()
        .unwrap()
        .get("0")
        .unwrap()
        .clone();
    for _ in 0..17 {
        output.send(vec![b'x'; OUTPUT_CHUNK_BYTES]).await.unwrap();
    }
    let mut bytes = 0;
    loop {
        match next(&mut socket).await {
            Message::Binary(data) => {
                bytes += data.len();
                assert!(bytes <= OUTPUT_WINDOW_BYTES);
            }
            Message::Text(data) => {
                let message: Value = serde_json::from_str(&data).unwrap();
                assert_eq!(message["code"], "slow_client");
                break;
            }
            other => panic!("unexpected control: {other:?}"),
        }
    }
    assert_eq!(*fixture.runtime.detached.lock().unwrap(), vec!["0"]);
}
