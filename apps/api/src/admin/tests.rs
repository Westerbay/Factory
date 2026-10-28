use super::*;
use crate::config::RuntimeConfig;
use async_trait::async_trait;
use axum::{
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{Duration, Utc};
use factory_application::identity::IdentityService;
use factory_core::identity::*;
use sha2::{Digest, Sha256};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use tower::ServiceExt;

const CSRF: &str = "fixture-csrf-token";

fn user(id: u128, role: Role, status: AccessStatus) -> User {
    User {
        id: Uuid::from_u128(id),
        display_name: "Fixture user".into(),
        handle: "fixture".into(),
        provider: "github".into(),
        role,
        status,
        version: 1,
        access_enabled_at: Some(Utc::now() - Duration::days(1)),
    }
}

fn token(kind: u8) -> String {
    URL_SAFE_NO_PAD.encode([kind; 32])
}
fn hash(kind: u8) -> CredentialHash {
    CredentialHash(Sha256::digest(token(kind).as_bytes()).into())
}

struct Store {
    administrator: Mutex<User>,
    target: Mutex<User>,
    query: Mutex<Option<UserListQuery>>,
    unavailable: AtomicBool,
    writes: AtomicUsize,
}

#[async_trait]
impl IdentityStore for Store {
    async fn create_login_attempt(&self, _: NewLoginAttempt) -> Result<(), IdentityError> {
        Err(IdentityError::Unavailable)
    }
    async fn take_login_attempt(
        &self,
        _: CredentialHash,
        _: CredentialHash,
    ) -> Result<LoginAttempt, IdentityError> {
        Err(IdentityError::Unavailable)
    }
    async fn admit(&self, _: LoginAdmission) -> Result<AdmissionOutcome, IdentityError> {
        Err(IdentityError::Unavailable)
    }
    async fn active_session(
        &self,
        token: CredentialHash,
    ) -> Result<Option<ActiveSession>, IdentityError> {
        if self.unavailable.load(Ordering::SeqCst) {
            return Err(IdentityError::Unavailable);
        }
        let user = if token == hash(1) {
            self.administrator.lock().unwrap().clone()
        } else if token == hash(2) {
            self.target.lock().unwrap().clone()
        } else {
            return Ok(None);
        };
        Ok(Some(ActiveSession {
            user,
            csrf_token: CSRF.into(),
            expires_at: Utc::now() + Duration::days(1),
        }))
    }
    async fn logout(&self, _: CredentialHash) -> Result<(), IdentityError> {
        Err(IdentityError::Unavailable)
    }
    async fn list_users(
        &self,
        actor: CredentialHash,
        query: UserListQuery,
    ) -> Result<UserPage, IdentityError> {
        require_admin(
            &self
                .active_session(actor)
                .await?
                .ok_or(IdentityError::Unauthorized)?
                .user,
        )?;
        *self.query.lock().unwrap() = Some(query);
        Ok(UserPage {
            items: vec![self.target.lock().unwrap().clone()],
            next_cursor: Some(Uuid::from_u128(2)),
        })
    }
    async fn change_access(
        &self,
        actor: CredentialHash,
        request: AccessRequest,
    ) -> Result<User, IdentityError> {
        let actor = self
            .active_session(actor)
            .await?
            .ok_or(IdentityError::Unauthorized)?
            .user;
        let mut target = self.target.lock().unwrap();
        let selected = if request.target_id == actor.id {
            &actor
        } else {
            &*target
        };
        if request.target_id != selected.id {
            return Err(IdentityError::NotFound);
        }
        if authorize_access_change(&actor, selected, &request)? {
            target.status = request.status.into();
            target.version += 1;
            self.writes.fetch_add(1, Ordering::SeqCst);
        }
        Ok(target.clone())
    }
}

struct UnusedProvider;
#[async_trait]
impl IdentityProvider for UnusedProvider {
    fn provider(&self) -> &str {
        "github"
    }
    fn issuer(&self) -> &str {
        "https://github.com"
    }
    fn authorization_url(&self, _: &str, _: &str) -> Result<String, IdentityError> {
        Err(IdentityError::Unavailable)
    }
    async fn exchange(&self, _: &str, _: &str) -> Result<VerifiedIdentity, IdentityError> {
        Err(IdentityError::Unavailable)
    }
}

fn fixture() -> (Router, Arc<Store>) {
    let store = Arc::new(Store {
        administrator: Mutex::new(user(1, Role::Admin, AccessStatus::Active)),
        target: Mutex::new(user(2, Role::Member, AccessStatus::Active)),
        query: Mutex::new(None),
        unavailable: AtomicBool::new(false),
        writes: AtomicUsize::new(0),
    });
    let service = Arc::new(IdentityService::new(
        store.clone(),
        Arc::new(UnusedProvider),
    ));
    (
        router(AuthState::new(
            service,
            &RuntimeConfig::from_values(|_| None).unwrap(),
        )),
        store,
    )
}

async fn call(
    app: &Router,
    method: Method,
    path: &str,
    actor: Option<u8>,
    origin: Option<&str>,
    csrf: Option<&str>,
    body: &str,
) -> Response {
    let mut request = Request::builder().method(method).uri(path);
    if let Some(actor) = actor {
        request = request.header(header::COOKIE, format!("factory_session={}", token(actor)));
    }
    if let Some(origin) = origin {
        request = request.header(header::ORIGIN, origin);
    }
    if let Some(csrf) = csrf {
        request = request.header("x-csrf-token", csrf);
    }
    if !body.is_empty() {
        request = request.header(header::CONTENT_TYPE, "application/json");
    }
    app.clone()
        .oneshot(request.body(Body::from(body.to_owned())).unwrap())
        .await
        .unwrap()
}

async fn json(response: Response) -> serde_json::Value {
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    serde_json::from_slice(&to_bytes(response.into_body(), 16384).await.unwrap()).unwrap()
}

#[tokio::test]
async fn direct_requests_require_a_live_active_administrator() {
    let (app, store) = fixture();
    let patch = format!("/api/admin/users/{}/access", Uuid::from_u128(2));
    for (actor, expected) in [(None, 401), (Some(2), 403), (Some(3), 401)] {
        for (method, path, body) in [
            (Method::GET, "/api/admin/users?status=active", ""),
            (
                Method::PATCH,
                patch.as_str(),
                r#"{"status":"suspended","expectedVersion":1}"#,
            ),
        ] {
            let response = call(
                &app,
                method,
                path,
                actor,
                Some("http://localhost:3000"),
                Some(CSRF),
                body,
            )
            .await;
            assert_eq!(response.status(), expected);
            let error = json(response).await;
            assert!(Uuid::parse_str(error["requestId"].as_str().unwrap()).is_ok());
            assert!(error.get("items").is_none());
        }
    }
    store.administrator.lock().unwrap().status = AccessStatus::Suspended;
    assert_eq!(
        call(
            &app,
            Method::GET,
            "/api/admin/users?status=active",
            Some(1),
            None,
            None,
            ""
        )
        .await
        .status(),
        401
    );
    assert_eq!(store.writes.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn listing_passes_bounded_filters_and_exposes_only_the_contract() {
    let (app, store) = fixture();
    let cursor = Uuid::from_u128(4);
    let response = call(
        &app,
        Method::GET,
        &format!("/api/admin/users?status=pending&cursor={cursor}&limit=5"),
        Some(1),
        None,
        None,
        "",
    )
    .await;
    assert_eq!(response.status(), 200);
    let value = json(response).await;
    assert_eq!(value["nextCursor"], Uuid::from_u128(2).to_string());
    assert_eq!(value["items"][0].as_object().unwrap().len(), 7);
    let query = store.query.lock().unwrap().take().unwrap();
    assert_eq!(query.status, AccessStatus::Pending);
    assert_eq!(query.after, Some(cursor));
    assert_eq!(query.limit, 5);
    for query in [
        "status=active&limit=0",
        "status=active&limit=51",
        "status=active&limit=-1",
        "status=active&cursor=secret-invalid",
        "status=unknown",
        "status=active&role=admin",
        "status=active&status=pending",
        "",
    ] {
        let response = call(
            &app,
            Method::GET,
            &format!("/api/admin/users?{query}"),
            Some(1),
            None,
            None,
            "",
        )
        .await;
        assert_eq!(response.status(), 400, "{query}");
        assert_eq!(json(response).await["code"], "invalid-input");
    }
}

#[tokio::test]
async fn mutations_require_origin_csrf_and_strict_typed_payloads() {
    let (app, store) = fixture();
    let path = format!("/api/admin/users/{}/access", Uuid::from_u128(2));
    let valid = r#"{"status":"suspended","expectedVersion":1}"#;
    let non_json = Request::builder()
        .method(Method::PATCH)
        .uri(&path)
        .header(header::COOKIE, format!("factory_session={}", token(1)))
        .header(header::ORIGIN, "http://localhost:3000")
        .header("x-csrf-token", CSRF)
        .header(header::CONTENT_TYPE, "text/plain")
        .body(Body::from(valid))
        .unwrap();
    assert_eq!(app.clone().oneshot(non_json).await.unwrap().status(), 400);
    for (origin, csrf) in [
        (None, Some(CSRF)),
        (Some("http://evil.test"), Some(CSRF)),
        (Some("http://localhost:3000"), None),
        (Some("http://localhost:3000"), Some("wrong")),
    ] {
        assert_eq!(
            call(&app, Method::PATCH, &path, Some(1), origin, csrf, valid)
                .await
                .status(),
            403
        );
    }
    for body in [
        "",
        "not-json",
        r#"{"status":"active","expectedVersion":1,"role":"admin"}"#,
        r#"{"status":"active","expectedVersion":1,"actor":"forged"}"#,
        r#"{"status":"active","expectedVersion":0}"#,
        r#"{"status":"active","expectedVersion":1.5}"#,
        r#"{"status":"pending","expectedVersion":1}"#,
        r#"{"status":"active","status":"suspended","expectedVersion":1}"#,
    ] {
        let response = call(
            &app,
            Method::PATCH,
            &path,
            Some(1),
            Some("http://localhost:3000"),
            Some(CSRF),
            body,
        )
        .await;
        assert_eq!(response.status(), 400, "{body}");
        assert_eq!(json(response).await["code"], "invalid-input");
    }
    let response = call(
        &app,
        Method::PATCH,
        "/api/admin/users/private-invalid/access",
        Some(1),
        Some("http://localhost:3000"),
        Some(CSRF),
        valid,
    )
    .await;
    assert_eq!(response.status(), 400);
    assert!(!json(response).await.to_string().contains("private-invalid"));
    assert_eq!(store.writes.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn mutation_versions_protected_admin_and_dependency_failures_map_safely() {
    let (app, store) = fixture();
    let path = format!("/api/admin/users/{}/access", Uuid::from_u128(2));
    let body = r#"{"status":"suspended","expectedVersion":1}"#;
    let response = call(
        &app,
        Method::PATCH,
        &path,
        Some(1),
        Some("http://localhost:3000"),
        Some(CSRF),
        body,
    )
    .await;
    assert_eq!(response.status(), 200);
    let value = json(response).await;
    assert_eq!(value["status"], "suspended");
    assert_eq!(value["version"], 2);
    let stale = call(
        &app,
        Method::PATCH,
        &path,
        Some(1),
        Some("http://localhost:3000"),
        Some(CSRF),
        body,
    )
    .await;
    assert_eq!(stale.status(), StatusCode::CONFLICT);
    assert_eq!(json(stale).await["code"], "version-conflict");
    let admin_path = format!("/api/admin/users/{}/access", Uuid::from_u128(1));
    let protected = call(
        &app,
        Method::PATCH,
        &admin_path,
        Some(1),
        Some("http://localhost:3000"),
        Some(CSRF),
        body,
    )
    .await;
    assert_eq!(protected.status(), 409);
    assert_eq!(json(protected).await["code"], "protected-administrator");
    let missing_path = format!("/api/admin/users/{}/access", Uuid::from_u128(99));
    let missing = call(
        &app,
        Method::PATCH,
        &missing_path,
        Some(1),
        Some("http://localhost:3000"),
        Some(CSRF),
        body,
    )
    .await;
    assert_eq!(missing.status(), 404);
    assert_eq!(json(missing).await["code"], "not-found");
    assert_eq!(store.writes.load(Ordering::SeqCst), 1);
    store.unavailable.store(true, Ordering::SeqCst);
    let unavailable = call(
        &app,
        Method::GET,
        "/api/admin/users?status=active",
        Some(1),
        None,
        None,
        "",
    )
    .await;
    assert_eq!(unavailable.status(), 503);
    assert_eq!(json(unavailable).await["code"], "unavailable");
}

#[tokio::test]
async fn request_bounds_and_process_quota_reject_without_writes() {
    let (app, store) = fixture();
    let path = format!("/api/admin/users/{}/access", Uuid::from_u128(2));
    let oversized = call(
        &app,
        Method::PATCH,
        &path,
        Some(1),
        Some("http://localhost:3000"),
        Some(CSRF),
        &"private-payload".repeat(1024),
    )
    .await;
    assert_eq!(oversized.status(), 400);
    assert!(
        !json(oversized)
            .await
            .to_string()
            .contains("private-payload")
    );
    let long_uri = format!("/api/admin/users?status=active&cursor={}", "x".repeat(8192));
    assert_eq!(
        call(&app, Method::GET, &long_uri, Some(1), None, None, "")
            .await
            .status(),
        400
    );
    for _ in 0..598 {
        assert_eq!(
            call(
                &app,
                Method::GET,
                "/api/admin/users?status=active",
                Some(1),
                None,
                None,
                ""
            )
            .await
            .status(),
            200
        );
    }
    let limited = call(
        &app,
        Method::GET,
        "/api/admin/users?status=active",
        Some(1),
        None,
        None,
        "",
    )
    .await;
    assert_eq!(limited.status(), 429);
    assert_eq!(limited.headers()[header::RETRY_AFTER], "60");
    assert_eq!(json(limited).await["code"], "rate-limited");
    assert_eq!(store.writes.load(Ordering::SeqCst), 0);
}
