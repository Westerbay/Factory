use async_trait::async_trait;
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Method, Request, header},
    response::Response,
};
use factory_api::{
    app::{self, IdentityRuntime},
    auth::AuthState,
    config::RuntimeConfig,
};
use factory_application::identity::IdentityService;
use factory_core::identity::{IdentityError, IdentityProvider, VerifiedIdentity};
use factory_database::{
    identity::IdentityDatabase,
    local_target::{VerifiedTarget, from_fixture},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;
use tower::ServiceExt;
use url::Url;

const WEB_ORIGIN: &str = "http://localhost:3000";

// This provider exists only in the integration-test binary. The production
// composition has no fixture switch, alternate OAuth endpoint or fake login.
struct FixtureProvider;

#[async_trait]
impl IdentityProvider for FixtureProvider {
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
    async fn exchange(&self, code: &str, _: &str) -> Result<VerifiedIdentity, IdentityError> {
        if !["1", "2"].contains(&code) {
            return Err(IdentityError::InvalidLogin);
        }
        Ok(VerifiedIdentity {
            provider: "github".into(),
            issuer: "https://github.com".into(),
            subject: code.into(),
            handle: format!("fixture{code}"),
            display_name: format!("Fixture {code}"),
        })
    }
}

#[derive(Deserialize)]
struct Fixture {
    project: String,
    owner: String,
    port: u16,
}

async fn target() -> VerifiedTarget {
    let path = std::env::var("FACTORY_TEST_TARGET_PATH").expect("explicit owned fixture target");
    assert!(std::fs::metadata(&path).unwrap().len() <= 4096);
    let fixture: Fixture = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    from_fixture(&root, &fixture.project, &fixture.owner, fixture.port)
        .await
        .unwrap()
}

fn configured_app(database: Arc<IdentityDatabase>) -> Router {
    let config = RuntimeConfig::from_values(|_| None).unwrap();
    let service = Arc::new(IdentityService::new(
        Arc::new(database.store()),
        Arc::new(FixtureProvider),
    ));
    app::router(
        &config,
        Some(IdentityRuntime {
            auth: AuthState::new(service, &config),
            database,
        }),
    )
    .unwrap()
}

async fn request(
    app: &Router,
    method: Method,
    path: &str,
    cookie: &str,
    csrf: Option<&str>,
    body: Option<Value>,
) -> Response {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header(header::ORIGIN, WEB_ORIGIN);
    if !cookie.is_empty() {
        request = request.header(header::COOKIE, cookie);
    }
    if let Some(csrf) = csrf {
        request = request.header("x-csrf-token", csrf);
    }
    let body = if let Some(body) = body {
        request = request.header(header::CONTENT_TYPE, "application/json");
        Body::from(body.to_string())
    } else {
        Body::empty()
    };
    app.clone()
        .oneshot(request.body(body).unwrap())
        .await
        .unwrap()
}

async fn response_json(response: Response) -> Value {
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    serde_json::from_slice(&to_bytes(response.into_body(), 16384).await.unwrap()).unwrap()
}

fn cookie(response: &Response, name: &str) -> String {
    response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .find_map(|header| {
            let cookie = cookie::Cookie::parse(header.to_str().unwrap()).unwrap();
            (cookie.name() == name).then(|| format!("{}={}", cookie.name(), cookie.value()))
        })
        .unwrap()
}

async fn start(app: &Router) -> (String, String) {
    let response = request(app, Method::GET, "/api/auth/github/start", "", None, None).await;
    assert_eq!(response.status(), 302);
    assert_eq!(
        response.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN],
        WEB_ORIGIN
    );
    let flags = response.headers()[header::SET_COOKIE].to_str().unwrap();
    for flag in ["HttpOnly", "SameSite=Lax", "Path=/", "Max-Age=600"] {
        assert!(flags.contains(flag));
    }
    assert!(!flags.contains("Domain="));
    let url = Url::parse(response.headers()[header::LOCATION].to_str().unwrap()).unwrap();
    let state = url
        .query_pairs()
        .find(|(key, _)| key == "state")
        .unwrap()
        .1
        .into_owned();
    (state, cookie(&response, "factory_login"))
}

async fn callback(app: &Router, attempt: &(String, String), subject: &str) -> Response {
    let response = request(
        app,
        Method::GET,
        &format!(
            "/api/auth/github/callback?state={}&code={subject}",
            attempt.0
        ),
        &attempt.1,
        None,
        None,
    )
    .await;
    assert_eq!(response.status(), 302);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    assert_eq!(response.headers()[header::REFERRER_POLICY], "no-referrer");
    assert_eq!(cookie(&response, "factory_login"), "factory_login=");
    response
}

async fn login(app: &Router, subject: &str) -> String {
    let response = callback(app, &start(app).await, subject).await;
    assert_eq!(
        response.headers()[header::LOCATION],
        "http://localhost:3000/app"
    );
    let session_flags = response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .find(|value| value.to_str().unwrap().starts_with("factory_session="))
        .unwrap()
        .to_str()
        .unwrap();
    for flag in ["HttpOnly", "SameSite=Lax", "Path=/", "Max-Age=604800"] {
        assert!(session_flags.contains(flag));
    }
    assert!(!session_flags.contains("Domain="));
    assert!(!session_flags.contains("Secure"));
    cookie(&response, "factory_session")
}

async fn session(app: &Router, cookie: &str) -> Value {
    let response = request(app, Method::GET, "/api/session", cookie, None, None).await;
    assert_eq!(response.status(), 200);
    response_json(response).await
}

async fn change(
    app: &Router,
    admin: &str,
    csrf: &str,
    id: &str,
    status: &str,
    version: i64,
) -> Value {
    let response = request(
        app,
        Method::PATCH,
        &format!("/api/admin/users/{id}/access"),
        admin,
        Some(csrf),
        Some(json!({"status":status,"expectedVersion":version})),
    )
    .await;
    assert_eq!(response.status(), 200);
    response_json(response).await
}

#[tokio::test]
async fn unavailable_identity_keeps_api_contracts_and_exact_write_cors() {
    let app = app::router(&RuntimeConfig::from_values(|_| None).unwrap(), None).unwrap();
    for (path, status) in [
        ("/health/live", 200),
        ("/health/ready", 503),
        ("/api/session", 503),
        ("/api/auth/github/start", 503),
        ("/api/admin/users?status=pending", 503),
        ("/api/missing", 404),
        ("/app", 404),
    ] {
        let response = request(&app, Method::GET, path, "", None, None).await;
        assert_eq!(response.status(), status, "{path}");
        assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");
        response_json(response).await;
    }
    // Every registered identity operation must fail closed before extracting input
    // when no runtime exists, including mutations with missing or malformed bodies.
    let contract = serde_json::to_value(factory_api::openapi::document()).unwrap();
    let paths = contract["paths"].as_object().unwrap();
    assert!(!paths.is_empty());
    for (path, item) in paths.iter().filter(|(path, _)| path.starts_with("/api/")) {
        for (method, operation) in item.as_object().unwrap() {
            if operation.get("responses").is_none() {
                continue;
            }
            let method = Method::from_bytes(method.to_uppercase().as_bytes()).unwrap();
            let path = path.replace("{id}", "invalid-id");
            let response = request(&app, method, &path, "", None, None).await;
            assert_eq!(response.status(), 503, "{path}");
            assert!(operation["responses"].get("503").is_some(), "{path}");
            assert_eq!(response_json(response).await["code"], "unavailable");
        }
    }
    for method in ["POST", "PATCH", "DELETE"] {
        for origin in [
            WEB_ORIGIN,
            "http://localhost:4000",
            "https://evil.test",
            "null",
        ] {
            let path = if method == "DELETE" {
                "/api/projects/00000000-0000-0000-0000-000000000001"
            } else {
                "/api/admin/users/00000000-0000-0000-0000-000000000001/access"
            };
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(Method::OPTIONS)
                        .uri(path)
                        .header(header::ORIGIN, origin)
                        .header(header::ACCESS_CONTROL_REQUEST_METHOD, method)
                        .header(
                            header::ACCESS_CONTROL_REQUEST_HEADERS,
                            "content-type,x-csrf-token",
                        )
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            if origin == WEB_ORIGIN {
                assert_eq!(
                    response.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN],
                    WEB_ORIGIN
                );
                assert_eq!(
                    response.headers()[header::ACCESS_CONTROL_ALLOW_CREDENTIALS],
                    "true"
                );
                assert!(
                    response.headers()[header::ACCESS_CONTROL_ALLOW_METHODS]
                        .to_str()
                        .unwrap()
                        .split(',')
                        .any(|v| v.trim() == method)
                );
                assert!(
                    response.headers()[header::ACCESS_CONTROL_ALLOW_HEADERS]
                        .to_str()
                        .unwrap()
                        .contains("x-csrf-token")
                );
            } else {
                assert!(
                    !response
                        .headers()
                        .contains_key(header::ACCESS_CONTROL_ALLOW_ORIGIN)
                );
            }
        }
    }
}

#[tokio::test]
async fn oauth_start_negotiates_browser_recovery_without_changing_api_errors() {
    let config = RuntimeConfig::from_values(|name| {
        (name == "FACTORY_WEB_ORIGIN").then(|| "http://localhost:3100".into())
    })
    .unwrap();
    let app = app::router(&config, None).unwrap();
    for (accept, expected) in [
        (None, 503),
        (Some("application/json"), 503),
        (Some("*/*"), 503),
        (Some("text/html;q=0,application/json"), 503),
        (Some("text/html;q=invalid"), 503),
        (Some("text/html;q=NaN"), 503),
        (Some("text/html;q=0;q=1"), 503),
        (Some("text/html"), 302),
        (
            Some("text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8"),
            302,
        ),
        (Some("application/json,text/html;q=0.5"), 302),
    ] {
        let mut request = Request::builder()
            .uri("/api/auth/github/start")
            .header(header::HOST, "untrusted.example")
            .header("x-forwarded-host", "untrusted.example");
        if let Some(accept) = accept {
            request = request.header(header::ACCEPT, accept);
        }
        let response = app
            .clone()
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), expected, "{accept:?}");
        if expected == 302 {
            assert_eq!(
                response.headers()[header::LOCATION],
                "http://localhost:3100/sign-in?reason=unavailable"
            );
        } else {
            assert!(!response.headers().contains_key(header::LOCATION));
        }
        assert_eq!(response.headers()[header::REFERRER_POLICY], "no-referrer");
        assert!(
            response
                .headers()
                .get_all(header::VARY)
                .iter()
                .any(|value| value == "Accept")
        );
        assert_eq!(response_json(response).await["code"], "unavailable");
    }
    for path in ["/api/session", "/api/admin/users?status=pending"] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(path)
                    .header(header::ACCEPT, "text/html")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 503);
        assert!(!response.headers().contains_key(header::LOCATION));
        assert_eq!(response_json(response).await["code"], "unavailable");
    }
}

#[tokio::test]
#[ignore = "requires the freshly initialized, explicitly owned PostgreSQL integration fixture"]
async fn owned_composition_preserves_identity_across_restart_and_revokes_access() {
    let target = target().await;
    let database = Arc::new(IdentityDatabase::connect(&target).await.unwrap());
    database
        .setup_administrator(&FixtureProvider.exchange("1", "").await.unwrap())
        .await
        .unwrap();
    let app = configured_app(database.clone());
    assert_eq!(
        request(&app, Method::GET, "/health/ready", "", None, None)
            .await
            .status(),
        200
    );
    let pending = callback(&app, &start(&app).await, "2").await;
    assert_eq!(
        pending.headers()[header::LOCATION],
        "http://localhost:3000/sign-in?reason=pending"
    );
    assert_eq!(cookie(&pending, "factory_session"), "factory_session=");
    let before_activation = start(&app).await;
    let admin = login(&app, "1").await;
    let admin_session = session(&app, &admin).await;
    assert_eq!(admin_session["user"]["role"], "admin");
    let csrf = admin_session["csrfToken"].as_str().unwrap();
    let listed = request(
        &app,
        Method::GET,
        "/api/admin/users?status=pending",
        &admin,
        None,
        None,
    )
    .await;
    assert_eq!(listed.status(), 200);
    let listed = response_json(listed).await;
    assert_eq!(listed["items"].as_array().unwrap().len(), 1);
    let id = listed["items"][0]["id"].as_str().unwrap();
    let active = change(
        &app,
        &admin,
        csrf,
        id,
        "active",
        listed["items"][0]["version"].as_i64().unwrap(),
    )
    .await;
    let old_attempt = callback(&app, &before_activation, "2").await;
    assert_eq!(
        old_attempt.headers()[header::LOCATION],
        "http://localhost:3000/sign-in?reason=invalid-login"
    );
    let member = login(&app, "2").await;
    assert_eq!(session(&app, &member).await["user"]["role"], "member");
    assert_eq!(
        request(
            &app,
            Method::GET,
            "/api/admin/users?status=active",
            &member,
            None,
            None
        )
        .await
        .status(),
        403
    );

    // Recreate the runtime and connection pool, keeping only browser cookies.
    drop(app);
    database.close().await;
    let restarted_database = Arc::new(IdentityDatabase::connect(&target).await.unwrap());
    let app = configured_app(restarted_database.clone());
    assert_eq!(session(&app, &member).await["user"]["id"], id);
    assert_eq!(session(&app, &admin).await["csrfToken"], csrf);
    let suspended = change(
        &app,
        &admin,
        csrf,
        id,
        "suspended",
        active["version"].as_i64().unwrap(),
    )
    .await;
    assert_eq!(
        request(&app, Method::GET, "/api/session", &member, None, None)
            .await
            .status(),
        401
    );
    let denied = callback(&app, &start(&app).await, "2").await;
    assert_eq!(
        denied.headers()[header::LOCATION],
        "http://localhost:3000/sign-in?reason=suspended"
    );
    change(
        &app,
        &admin,
        csrf,
        id,
        "active",
        suspended["version"].as_i64().unwrap(),
    )
    .await;
    assert_eq!(
        request(&app, Method::GET, "/api/session", &member, None, None)
            .await
            .status(),
        401
    );
    let member = login(&app, "2").await;
    let member_session = session(&app, &member).await;
    let logout = request(
        &app,
        Method::POST,
        "/api/auth/logout",
        &member,
        member_session["csrfToken"].as_str(),
        None,
    )
    .await;
    assert_eq!(logout.status(), 204);
    assert_eq!(cookie(&logout, "factory_session"), "factory_session=");
    assert_eq!(
        request(&app, Method::GET, "/api/session", &member, None, None)
            .await
            .status(),
        401
    );
    restarted_database.close().await;
    assert_eq!(
        request(&app, Method::GET, "/health/ready", "", None, None)
            .await
            .status(),
        503
    );
    assert_eq!(
        request(&app, Method::GET, "/api/session", &admin, None, None)
            .await
            .status(),
        503
    );
}
