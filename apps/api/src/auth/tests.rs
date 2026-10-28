use super::{credentials::*, *};
use crate::config::RuntimeConfig;
use async_trait::async_trait;
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Method, Request, header},
    response::Response,
};
use chrono::{Duration, Utc};
use factory_application::identity::IdentityService;
use factory_core::identity::*;
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};
use tower::ServiceExt;
use url::Url;
use uuid::Uuid;

type Attempts = HashMap<CredentialHash, (CredentialHash, String)>;

#[derive(Default)]
struct Store {
    attempts: Mutex<Attempts>,
    sessions: Mutex<HashMap<CredentialHash, ActiveSession>>,
    unavailable: AtomicBool,
    admission: AtomicUsize,
}

fn user() -> User {
    User {
        id: Uuid::new_v4(),
        display_name: "Fixture user".into(),
        handle: "fixture".into(),
        provider: "github".into(),
        role: Role::Member,
        status: AccessStatus::Active,
        version: 1,
        access_enabled_at: Some(Utc::now() - Duration::days(1)),
    }
}

#[async_trait]
impl IdentityStore for Store {
    async fn create_login_attempt(&self, attempt: NewLoginAttempt) -> Result<(), IdentityError> {
        if self.unavailable.load(Ordering::SeqCst) {
            return Err(IdentityError::Unavailable);
        }
        self.attempts.lock().unwrap().insert(
            attempt.state_hash,
            (attempt.binding_hash, attempt.pkce_verifier),
        );
        Ok(())
    }
    async fn take_login_attempt(
        &self,
        state: CredentialHash,
        binding: CredentialHash,
    ) -> Result<LoginAttempt, IdentityError> {
        let mut attempts = self.attempts.lock().unwrap();
        if !attempts
            .get(&state)
            .is_some_and(|(stored, _)| stored == &binding)
        {
            return Err(IdentityError::InvalidLogin);
        }
        let (_, pkce_verifier) = attempts.remove(&state).unwrap();
        Ok(LoginAttempt {
            pkce_verifier,
            created_at: Utc::now(),
        })
    }
    async fn admit(&self, admission: LoginAdmission) -> Result<AdmissionOutcome, IdentityError> {
        match self.admission.load(Ordering::SeqCst) {
            1 => return Ok(AdmissionOutcome::Pending),
            2 => return Ok(AdmissionOutcome::Suspended),
            3 => return Ok(AdmissionOutcome::StaleLogin),
            _ => {}
        }
        let session = ActiveSession {
            user: user(),
            csrf_token: admission.session.csrf_token,
            expires_at: Utc::now() + Duration::days(7),
        };
        let mut sessions = self.sessions.lock().unwrap();
        if let Some(old) = admission.session.previous_token_hash {
            sessions.remove(&old);
        }
        sessions.insert(admission.session.token_hash, session.clone());
        Ok(AdmissionOutcome::Active(session))
    }
    async fn active_session(
        &self,
        hash: CredentialHash,
    ) -> Result<Option<ActiveSession>, IdentityError> {
        if self.unavailable.load(Ordering::SeqCst) {
            return Err(IdentityError::Unavailable);
        }
        Ok(self
            .sessions
            .lock()
            .unwrap()
            .get(&hash)
            .filter(|s| s.expires_at > Utc::now() && s.user.status == AccessStatus::Active)
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
        Err(IdentityError::Unavailable)
    }
    async fn change_access(
        &self,
        _: CredentialHash,
        _: AccessRequest,
    ) -> Result<User, IdentityError> {
        Err(IdentityError::Unavailable)
    }
}

#[derive(Default)]
struct Provider {
    calls: AtomicUsize,
    unavailable: AtomicBool,
}

#[async_trait]
impl IdentityProvider for Provider {
    fn provider(&self) -> &str {
        "github"
    }
    fn issuer(&self) -> &str {
        "https://github.com"
    }
    fn authorization_url(&self, state: &str, challenge: &str) -> Result<String, IdentityError> {
        let mut url = Url::parse("https://github.com/login/oauth/authorize").unwrap();
        url.query_pairs_mut()
            .append_pair("state", state)
            .append_pair("code_challenge", challenge);
        Ok(url.into())
    }
    async fn exchange(&self, _: &str, _: &str) -> Result<VerifiedIdentity, IdentityError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.unavailable.load(Ordering::SeqCst) {
            return Err(IdentityError::Unavailable);
        }
        Ok(VerifiedIdentity {
            provider: "github".into(),
            issuer: "https://github.com".into(),
            subject: "123".into(),
            display_name: "Fixture user".into(),
            handle: "fixture".into(),
        })
    }
}

fn fixture() -> (Router, Arc<Store>, Arc<Provider>) {
    let store = Arc::new(Store::default());
    let provider = Arc::new(Provider::default());
    let service = Arc::new(IdentityService::new(
        store.clone(),
        provider.clone(),
        BootstrapAdmin::github("123").unwrap(),
    ));
    (
        router(AuthState::new(
            service,
            &RuntimeConfig::from_values(|_| None).unwrap(),
        )),
        store,
        provider,
    )
}

async fn call(
    app: &Router,
    method: Method,
    uri: &str,
    cookies: &str,
    origin: Option<&str>,
    csrf: Option<&str>,
) -> Response {
    let mut request = Request::builder().method(method).uri(uri);
    if !cookies.is_empty() {
        request = request.header(header::COOKIE, cookies);
    }
    if let Some(origin) = origin {
        request = request.header(header::ORIGIN, origin);
    }
    if let Some(csrf) = csrf {
        request = request.header("x-csrf-token", csrf);
    }
    app.clone()
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap()
}

fn returned_cookie(response: &Response, name: &str) -> String {
    response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .find_map(|h| {
            let cookie = cookie::Cookie::parse(h.to_str().unwrap()).unwrap();
            (cookie.name() == name).then(|| format!("{}={}", cookie.name(), cookie.value()))
        })
        .unwrap()
}

async fn start(app: &Router) -> (String, String) {
    let response = call(app, Method::GET, "/api/auth/github/start", "", None, None).await;
    assert_eq!(response.status(), 302);
    let location = Url::parse(response.headers()[header::LOCATION].to_str().unwrap()).unwrap();
    let state = location
        .query_pairs()
        .find(|(k, _)| k == "state")
        .unwrap()
        .1
        .into_owned();
    (state, returned_cookie(&response, LOGIN_COOKIE))
}

async fn sign_in(app: &Router, previous: &str) -> String {
    let (state, binding) = start(app).await;
    let cookies = if previous.is_empty() {
        binding
    } else {
        format!("{binding}; {previous}")
    };
    let response = call(
        app,
        Method::GET,
        &format!("/api/auth/github/callback?state={state}&code=fixture"),
        &cookies,
        None,
        None,
    )
    .await;
    assert_eq!(
        response.headers()[header::LOCATION],
        "http://localhost:3000/app"
    );
    returned_cookie(&response, SESSION_COOKIE)
}

#[tokio::test]
async fn browser_binding_pkce_cookie_and_replay_are_enforced() {
    let (app, store, provider) = fixture();
    let initial = call(&app, Method::GET, "/api/auth/github/start", "", None, None).await;
    let binding = returned_cookie(&initial, LOGIN_COOKIE);
    let flags = initial.headers()[header::SET_COOKIE].to_str().unwrap();
    for expected in ["HttpOnly", "SameSite=Lax", "Path=/", "Max-Age=600"] {
        assert!(flags.contains(expected));
    }
    assert!(!flags.contains("Domain="));
    assert!(!flags.contains("Secure"));
    let url = Url::parse(initial.headers()[header::LOCATION].to_str().unwrap()).unwrap();
    let state = url
        .query_pairs()
        .find(|(k, _)| k == "state")
        .unwrap()
        .1
        .into_owned();
    assert_eq!(state.len(), 43);
    let verifier = store
        .attempts
        .lock()
        .unwrap()
        .get(&hash_token(&state).unwrap())
        .unwrap()
        .1
        .clone();
    let expected_challenge = oauth2::PkceCodeChallenge::from_code_verifier_sha256(
        &oauth2::PkceCodeVerifier::new(verifier),
    );
    assert_eq!(
        url.query_pairs()
            .find(|(k, _)| k == "code_challenge")
            .unwrap()
            .1
            .len(),
        43
    );
    assert_eq!(
        url.query_pairs()
            .find(|(key, _)| key == "code_challenge")
            .unwrap()
            .1,
        expected_challenge.as_str()
    );
    let callback = format!("/api/auth/github/callback?state={state}&code=fixture");
    let mismatch = call(
        &app,
        Method::GET,
        &callback,
        &format!("factory_login={}", random_token().unwrap()),
        None,
        None,
    )
    .await;
    assert_eq!(
        mismatch.headers()[header::LOCATION],
        "http://localhost:3000/sign-in?reason=invalid-login"
    );
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
    let success = call(&app, Method::GET, &callback, &binding, None, None).await;
    assert_eq!(
        success.headers()[header::LOCATION],
        "http://localhost:3000/app"
    );
    assert!(returned_cookie(&success, LOGIN_COOKIE).ends_with('='));
    let replay = call(&app, Method::GET, &callback, &binding, None, None).await;
    assert_eq!(
        replay.headers()[header::LOCATION],
        "http://localhost:3000/sign-in?reason=invalid-login"
    );
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn session_rotation_csrf_origin_and_logout_are_enforced() {
    let (app, _, _) = fixture();
    let old = sign_in(&app, "").await;
    let new = sign_in(&app, &old).await;
    assert_ne!(new, old);
    assert_eq!(
        call(&app, Method::GET, "/api/session", &old, None, None)
            .await
            .status(),
        401
    );
    let response = call(&app, Method::GET, "/api/session", &new, None, None).await;
    assert_eq!(response.status(), 200);
    let json: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap()).unwrap();
    let csrf = json["csrfToken"].as_str().unwrap();
    for (origin, token) in [
        (None, Some(csrf)),
        (Some("http://evil.test"), Some(csrf)),
        (Some("http://localhost:3000"), Some("wrong")),
    ] {
        assert_eq!(
            call(&app, Method::POST, "/api/auth/logout", &new, origin, token)
                .await
                .status(),
            403
        );
    }
    let logout = call(
        &app,
        Method::POST,
        "/api/auth/logout",
        &new,
        Some("http://localhost:3000"),
        Some(csrf),
    )
    .await;
    assert_eq!(logout.status(), 204);
    assert_eq!(returned_cookie(&logout, SESSION_COOKIE), "factory_session=");
    assert_eq!(
        call(&app, Method::GET, "/api/session", &new, None, None)
            .await
            .status(),
        401
    );
}

#[tokio::test]
async fn pending_suspended_or_stale_admission_revokes_the_previous_browser_session() {
    for mode in [1, 2, 3] {
        let (app, store, _) = fixture();
        let old = sign_in(&app, "").await;
        store.admission.store(mode, Ordering::SeqCst);
        let (state, binding) = start(&app).await;
        let response = call(
            &app,
            Method::GET,
            &format!("/api/auth/github/callback?state={state}&code=fixture"),
            &format!("{binding}; {old}"),
            None,
            None,
        )
        .await;
        assert_eq!(
            returned_cookie(&response, SESSION_COOKIE),
            "factory_session="
        );
        assert_eq!(
            call(&app, Method::GET, "/api/session", &old, None, None)
                .await
                .status(),
            401
        );
    }
}

#[tokio::test]
async fn bad_callback_does_not_log_out_an_existing_session() {
    let (app, _, _) = fixture();
    let session = sign_in(&app, "").await;
    let response = call(
        &app,
        Method::GET,
        "/api/auth/github/callback?state=invalid&code=forged",
        &session,
        None,
        None,
    )
    .await;
    assert_eq!(returned_cookie(&response, LOGIN_COOKIE), "factory_login=");
    assert_eq!(
        call(&app, Method::GET, "/api/session", &session, None, None)
            .await
            .status(),
        200
    );
}

#[tokio::test]
async fn provider_failure_consumes_the_attempt_without_issuing_a_session() {
    let (app, store, provider) = fixture();
    provider.unavailable.store(true, Ordering::SeqCst);
    let (state, binding) = start(&app).await;
    let callback = format!("/api/auth/github/callback?state={state}&code=fixture");
    let failure = call(&app, Method::GET, &callback, &binding, None, None).await;
    assert_eq!(
        failure.headers()[header::LOCATION],
        "http://localhost:3000/sign-in?reason=unavailable"
    );
    assert_eq!(returned_cookie(&failure, LOGIN_COOKIE), "factory_login=");
    assert!(store.sessions.lock().unwrap().is_empty());
    assert!(store.attempts.lock().unwrap().is_empty());
    let replay = call(&app, Method::GET, &callback, &binding, None, None).await;
    assert_eq!(
        replay.headers()[header::LOCATION],
        "http://localhost:3000/sign-in?reason=invalid-login"
    );
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn denied_login_validates_binding_and_consumes_the_attempt_without_exchange() {
    let (app, store, provider) = fixture();
    let (state, binding) = start(&app).await;
    let callback = format!("/api/auth/github/callback?state={state}&error=access_denied");
    let wrong_binding = format!("factory_login={}", random_token().unwrap());
    let mismatch = call(&app, Method::GET, &callback, &wrong_binding, None, None).await;
    assert_eq!(
        mismatch.headers()[header::LOCATION],
        "http://localhost:3000/sign-in?reason=invalid-login"
    );
    assert_eq!(store.attempts.lock().unwrap().len(), 1);
    let denial = call(&app, Method::GET, &callback, &binding, None, None).await;
    assert_eq!(
        denial.headers()[header::LOCATION],
        "http://localhost:3000/sign-in?reason=denied"
    );
    assert_eq!(returned_cookie(&denial, LOGIN_COOKIE), "factory_login=");
    assert!(store.attempts.lock().unwrap().is_empty());
    assert!(store.sessions.lock().unwrap().is_empty());
    let replay = call(&app, Method::GET, &callback, &binding, None, None).await;
    assert_eq!(
        replay.headers()[header::LOCATION],
        "http://localhost:3000/sign-in?reason=invalid-login"
    );
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn expired_or_duplicate_credentials_fail_closed_and_store_failure_is_safe() {
    let (app, store, _) = fixture();
    let session = sign_in(&app, "").await;
    assert_eq!(
        call(
            &app,
            Method::GET,
            "/api/session",
            &format!("{session}; {session}"),
            None,
            None
        )
        .await
        .status(),
        401
    );
    store
        .sessions
        .lock()
        .unwrap()
        .values_mut()
        .for_each(|s| s.expires_at = Utc::now() - Duration::seconds(1));
    assert_eq!(
        call(&app, Method::GET, "/api/session", &session, None, None)
            .await
            .status(),
        401
    );
    store.unavailable.store(true, Ordering::SeqCst);
    let response = call(&app, Method::GET, "/api/session", &session, None, None).await;
    assert_eq!(response.status(), 503);
    let json: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap()).unwrap();
    assert_eq!(json["code"], "unavailable");
    assert!(Uuid::parse_str(json["requestId"].as_str().unwrap()).is_ok());
}

#[tokio::test]
async fn anonymous_login_work_has_a_bounded_process_quota() {
    let (app, _, _) = fixture();
    for _ in 0..60 {
        assert_eq!(
            call(&app, Method::GET, "/api/auth/github/start", "", None, None)
                .await
                .status(),
            302
        );
    }
    let response = call(&app, Method::GET, "/api/auth/github/start", "", None, None).await;
    assert_eq!(response.status(), 429);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
}

#[tokio::test]
async fn ambiguous_origins_and_oversized_bodies_have_safe_json_errors() {
    let (app, store, _) = fixture();
    let session = sign_in(&app, "").await;
    let csrf = store
        .sessions
        .lock()
        .unwrap()
        .values()
        .next()
        .unwrap()
        .csrf_token
        .clone();
    let request = Request::builder()
        .method(Method::POST)
        .uri("/api/auth/logout")
        .header(header::COOKIE, &session)
        .header(header::ORIGIN, "http://localhost:3000")
        .header(header::ORIGIN, "http://evil.test")
        .header("x-csrf-token", &csrf)
        .body(Body::empty())
        .unwrap();
    assert_eq!(app.clone().oneshot(request).await.unwrap().status(), 403);
    let request = Request::builder()
        .method(Method::POST)
        .uri("/api/auth/logout")
        .header(header::COOKIE, &session)
        .header(header::ORIGIN, "http://localhost:3000")
        .header("x-csrf-token", &csrf)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from("private-content".repeat(1024)))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), 400);
    let bytes = to_bytes(response.into_body(), 4096).await.unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["code"], "invalid-input");
    assert!(!String::from_utf8_lossy(&bytes).contains("private-content"));
    assert_eq!(
        call(&app, Method::GET, "/api/session", &session, None, None)
            .await
            .status(),
        200
    );
}
