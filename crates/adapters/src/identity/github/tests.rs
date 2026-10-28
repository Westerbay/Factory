use super::*;
use axum::{
    Router,
    extract::State,
    http::HeaderMap,
    response::IntoResponse,
    routing::{get, post},
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

struct Fixture {
    token_body: String,
    token_status: StatusCode,
    profile_body: String,
    profile_status: StatusCode,
    token_delay: Duration,
    token_calls: AtomicUsize,
    profile_calls: AtomicUsize,
    token_request: Mutex<Option<(HeaderMap, String)>>,
    profile_headers: Mutex<Option<HeaderMap>>,
}

impl Default for Fixture {
    fn default() -> Self {
        Self {
            token_body:
                r#"{"access_token":"fixture-token","token_type":"bearer","scope":"read:user"}"#
                    .into(),
            token_status: StatusCode::OK,
            profile_body: r#"{"id":1234567890123456789,"login":"fixture-user","name":null}"#.into(),
            profile_status: StatusCode::OK,
            token_delay: Duration::ZERO,
            token_calls: AtomicUsize::new(0),
            profile_calls: AtomicUsize::new(0),
            token_request: Mutex::new(None),
            profile_headers: Mutex::new(None),
        }
    }
}

async fn token(
    State(state): State<Arc<Fixture>>,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    state.token_calls.fetch_add(1, Ordering::SeqCst);
    *state.token_request.lock().unwrap() = Some((headers, body));
    tokio::time::sleep(state.token_delay).await;
    (
        state.token_status,
        [
            (header::CONTENT_TYPE, "application/json"),
            (header::LOCATION, "/profile"),
        ],
        state.token_body.clone(),
    )
}

async fn profile(State(state): State<Arc<Fixture>>, headers: HeaderMap) -> impl IntoResponse {
    state.profile_calls.fetch_add(1, Ordering::SeqCst);
    *state.profile_headers.lock().unwrap() = Some(headers);
    (
        state.profile_status,
        [(header::CONTENT_TYPE, "application/json")],
        state.profile_body.clone(),
    )
}

struct Server(tokio::task::JoinHandle<()>);
impl Drop for Server {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn fixture(state: Fixture) -> (GitHubProvider, Arc<Fixture>, Server) {
    let state = Arc::new(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = Router::new()
        .route("/token", post(token))
        .route("/profile", get(profile))
        .with_state(state.clone());
    let server = Server(tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    }));
    let mut provider = GitHubProvider::new(
        "fixture-client".into(),
        "fixture-secret".into(),
        "http://localhost:3001/api/auth/github/callback".into(),
    )
    .unwrap();
    // Endpoint substitution exists only inside this private cfg(test) module.
    provider.oauth = provider
        .oauth
        .set_token_uri(TokenUrl::new(format!("http://{address}/token")).unwrap());
    provider.profile = Url::parse(&format!("http://{address}/profile")).unwrap();
    (provider, state, server)
}

#[tokio::test]
async fn operator_lookup_uses_public_identity_without_credentials_and_rejects_non_users() {
    let (provider, state, _server) = fixture(Fixture {
        profile_body:
            r#"{"id":1234567890123456789,"login":"renamed-user","name":null,"type":"User"}"#.into(),
        ..Default::default()
    })
    .await;
    let identity = lookup_account_at(&provider.http, provider.profile.clone())
        .await
        .unwrap();
    assert_eq!(identity.subject, "1234567890123456789");
    assert_eq!(identity.handle, "renamed-user");
    assert!(
        !state
            .profile_headers
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .contains_key(header::AUTHORIZATION)
    );
    assert_eq!(state.token_calls.load(Ordering::SeqCst), 0);
    for body in [
        r#"{"id":1,"login":"org","type":"Organization"}"#,
        r#"{"id":1,"login":"robot","type":"Bot"}"#,
        r#"{"id":0,"login":"user","type":"User"}"#,
        r#"{"id":1,"login":"bad\nuser","type":"User"}"#,
        r#"{"id":1,"login":"user"}"#,
    ] {
        let (provider, _, _server) = fixture(Fixture {
            profile_body: body.into(),
            ..Default::default()
        })
        .await;
        assert!(
            lookup_account_at(&provider.http, provider.profile.clone())
                .await
                .is_err()
        );
    }
    for (status, expected) in [
        (StatusCode::NOT_FOUND, IdentityError::NotFound),
        (StatusCode::TOO_MANY_REQUESTS, IdentityError::Unavailable),
        (StatusCode::FOUND, IdentityError::Unavailable),
    ] {
        let (provider, _, _server) = fixture(Fixture {
            profile_status: status,
            ..Default::default()
        })
        .await;
        assert!(
            matches!(lookup_account_at(&provider.http, provider.profile.clone()).await, Err(error) if error == expected)
        );
    }
    for username in [
        "../user",
        "user?token=x",
        "user@example.com",
        "-user",
        "user-",
        "a--b",
        "",
    ] {
        assert!(matches!(
            lookup_github_account(username).await,
            Err(IdentityError::InvalidInput)
        ));
    }
}

#[test]
fn authorization_requests_only_identity_with_state_and_s256() {
    let provider = GitHubProvider::new(
        "client".into(),
        "secret".into(),
        "http://localhost:3001/api/auth/github/callback".into(),
    )
    .unwrap();
    let (challenge, _) = oauth2::PkceCodeChallenge::new_random_sha256();
    let state = "s".repeat(43);
    let url = Url::parse(
        &provider
            .authorization_url(&state, challenge.as_str())
            .unwrap(),
    )
    .unwrap();
    let params: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
    assert_eq!(url.origin().ascii_serialization(), "https://github.com");
    assert_eq!(url.path(), "/login/oauth/authorize");
    assert_eq!(params["scope"], "read:user");
    assert_eq!(params["code_challenge_method"], "S256");
    assert_eq!(params["code_challenge"], challenge.as_str());
    assert_eq!(params["state"], state);
    assert_eq!(
        params["redirect_uri"],
        "http://localhost:3001/api/auth/github/callback"
    );
    assert!(provider.authorization_url("injected&state", "bad").is_err());
    assert!(
        GitHubProvider::new(
            "client".into(),
            "secret".into(),
            "https://evil.test/callback".into()
        )
        .is_err()
    );
}

#[tokio::test]
async fn exchange_posts_secret_and_pkce_then_returns_only_stable_identity() {
    let (provider, state, _server) = fixture(Fixture::default()).await;
    let result = provider
        .exchange("fixture-code", &"v".repeat(43))
        .await
        .unwrap();
    assert_eq!(result.subject, "1234567890123456789");
    assert_eq!(result.provider, "github");
    assert_eq!(result.issuer, "https://github.com");
    assert_eq!(result.display_name, "fixture-user");
    let request = state.token_request.lock().unwrap();
    let (headers, body) = request.as_ref().unwrap();
    let values: std::collections::HashMap<_, _> = url::form_urlencoded::parse(body.as_bytes())
        .into_owned()
        .collect();
    assert!(!headers.contains_key(header::AUTHORIZATION));
    assert_eq!(values["client_id"], "fixture-client");
    assert_eq!(values["client_secret"], "fixture-secret");
    assert_eq!(values["code"], "fixture-code");
    assert_eq!(values["code_verifier"], "v".repeat(43));
    assert_eq!(
        state.profile_headers.lock().unwrap().as_ref().unwrap()[header::AUTHORIZATION],
        "Bearer fixture-token"
    );
    assert_eq!(state.token_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn redirects_and_provider_errors_never_follow_or_retry() {
    for (status, body, expected) in [
        (StatusCode::FOUND, "{}", IdentityError::Unavailable),
        (
            StatusCode::BAD_GATEWAY,
            "upstream private detail",
            IdentityError::Unavailable,
        ),
        (
            StatusCode::OK,
            r#"{"error":"bad_verification_code","error_description":"private"}"#,
            IdentityError::InvalidLogin,
        ),
    ] {
        let (provider, state, _server) = fixture(Fixture {
            token_status: status,
            token_body: body.into(),
            ..Fixture::default()
        })
        .await;
        assert_eq!(
            provider
                .exchange("code", &"v".repeat(43))
                .await
                .unwrap_err(),
            expected
        );
        assert_eq!(state.token_calls.load(Ordering::SeqCst), 1);
        assert_eq!(state.profile_calls.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn token_and_profile_responses_are_bounded() {
    for oversized_token in [true, false] {
        let state = if oversized_token {
            Fixture {
                token_body: "x".repeat(TOKEN_BODY_LIMIT + 1),
                ..Fixture::default()
            }
        } else {
            Fixture {
                profile_body: "x".repeat(PROFILE_BODY_LIMIT + 1),
                ..Fixture::default()
            }
        };
        let (provider, _, _server) = fixture(state).await;
        assert_eq!(
            provider
                .exchange("code", &"v".repeat(43))
                .await
                .unwrap_err(),
            IdentityError::Unavailable
        );
    }
}

#[tokio::test]
async fn chunked_profile_is_bounded_without_content_length() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let (mut provider, _, _token_server) = fixture(Fixture::default()).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    provider.profile = Url::parse(&format!(
        "http://{}/profile",
        listener.local_addr().unwrap()
    ))
    .unwrap();
    let _profile_server = Server(tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        while !request.ends_with(b"\r\n\r\n") {
            let byte = socket.read_u8().await.unwrap();
            request.push(byte);
            assert!(request.len() <= 8192);
        }
        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\n\r\n").await.unwrap();
        // Separate HTTP chunks exceed the budget cumulatively, not in one header.
        for size in [40 * 1024, 30 * 1024] {
            socket
                .write_all(format!("{size:x}\r\n").as_bytes())
                .await
                .unwrap();
            socket.write_all(&vec![b'x'; size]).await.unwrap();
            socket.write_all(b"\r\n").await.unwrap();
        }
        socket.write_all(b"0\r\n\r\n").await.unwrap();
    }));
    assert_eq!(
        provider
            .exchange("code", &"v".repeat(43))
            .await
            .unwrap_err(),
        IdentityError::Unavailable
    );
}

#[tokio::test]
async fn untrusted_profile_id_or_control_characters_never_become_identity() {
    for profile_body in [
        r#"{"id":"123","login":"name"}"#,
        r#"{"id":0,"login":"name"}"#,
        r#"{"id":123.4,"login":"name"}"#,
        r#"{"id":123,"login":"bad\nname"}"#,
        r#"{"id":123,"login":""}"#,
    ] {
        let (provider, _, _server) = fixture(Fixture {
            profile_body: profile_body.into(),
            ..Fixture::default()
        })
        .await;
        assert_eq!(
            provider
                .exchange("code", &"v".repeat(43))
                .await
                .unwrap_err(),
            IdentityError::InvalidLogin
        );
    }
}

#[tokio::test]
async fn stalled_token_request_times_out_without_retry_or_profile_fetch() {
    let (mut provider, state, _server) = fixture(Fixture {
        token_delay: Duration::from_secs(1),
        ..Fixture::default()
    })
    .await;
    provider.http = http_client(Duration::from_millis(30)).unwrap();
    assert_eq!(
        provider
            .exchange("code", &"v".repeat(43))
            .await
            .unwrap_err(),
        IdentityError::Unavailable
    );
    assert_eq!(state.token_calls.load(Ordering::SeqCst), 1);
    assert_eq!(state.profile_calls.load(Ordering::SeqCst), 0);
}
