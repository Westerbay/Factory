use super::*;
use axum::{
    Router,
    extract::{Path, Query, State},
    http::HeaderMap,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use std::collections::HashMap;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

struct Guard;
impl RepositoryOperationGuard for Guard {
    fn check(
        &self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), Error>> + Send + '_>> {
        Box::pin(async { Ok(()) })
    }
}

const KEY: &str =
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA factory";

#[derive(Default)]
struct Fixture {
    keys: Mutex<Vec<String>>,
    posts: AtomicUsize,
    fail_post_response: bool,
    no_verified_email: bool,
    denied_scope: bool,
    forbidden: bool,
    quota: bool,
    forbidden_message: Option<String>,
    origin_status: u16,
    origin_id: Option<u64>,
    origin_name: Option<String>,
    draft: Mutex<Option<serde_json::Value>>,
    draft_posts: AtomicUsize,
    draft_forbidden: bool,
    hold_draft_response: bool,
    draft_head_after_post: Option<String>,
    move_base_on_diff: bool,
}

async fn token(State(state): State<Arc<Fixture>>) -> impl IntoResponse {
    let scopes = if state.denied_scope {
        "repo"
    } else {
        "repo,user:email,write:public_key"
    };
    (
        StatusCode::OK,
        axum::Json(
            serde_json::json!({ "access_token":"fixture-token", "token_type":"bearer", "scope":scopes }),
        ),
    )
}
async fn profile() -> impl IntoResponse {
    axum::Json(serde_json::json!({ "id":123, "login":"user", "name":null }))
}
async fn emails(State(state): State<Arc<Fixture>>) -> impl IntoResponse {
    axum::Json(
        serde_json::json!([{ "email":"user@example.test", "primary":true, "verified":!state.no_verified_email }]),
    )
}
async fn repos(State(state): State<Arc<Fixture>>) -> impl IntoResponse {
    if state.forbidden {
        return (
            StatusCode::FORBIDDEN,
            [(
                "x-ratelimit-remaining",
                if state.quota { "0" } else { "12" },
            )],
            axum::Json(serde_json::json!({"message":state.forbidden_message.as_deref().unwrap_or("denied")})),
        )
            .into_response();
    }
    (
        StatusCode::OK,
        [("x-ratelimit-remaining", "12")],
        axum::Json(serde_json::json!([{ "id":3, "full_name":"user/example", "private":true }])),
    )
        .into_response()
}
async fn repo_at_origin(State(state): State<Arc<Fixture>>) -> impl IntoResponse {
    let status = StatusCode::from_u16(if state.origin_status == 0 {
        200
    } else {
        state.origin_status
    })
    .unwrap();
    let body = serde_json::json!({"id":state.origin_id.unwrap_or(3), "full_name":state.origin_name.as_deref().unwrap_or("user/example"), "private":true});
    (status, [(header::LOCATION, "/user")], axum::Json(body))
}
async fn keys(State(state): State<Arc<Fixture>>) -> impl IntoResponse {
    let keys = state
        .keys
        .lock()
        .unwrap()
        .iter()
        .map(|key| serde_json::json!({"key":key}))
        .collect::<Vec<_>>();
    axum::Json(keys)
}
async fn add_key(
    State(state): State<Arc<Fixture>>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> impl IntoResponse {
    state.posts.fetch_add(1, Ordering::SeqCst);
    state
        .keys
        .lock()
        .unwrap()
        .push(body["key"].as_str().unwrap().into());
    if state.fail_post_response {
        StatusCode::INTERNAL_SERVER_ERROR
    } else {
        StatusCode::CREATED
    }
}

async fn drafts(
    State(state): State<Arc<Fixture>>,
    Query(query): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    assert_eq!(query["state"], "all");
    assert!(query["head"].starts_with("user:factory/features/"));
    axum::Json(
        state
            .draft
            .lock()
            .unwrap()
            .iter()
            .cloned()
            .collect::<Vec<_>>(),
    )
}

async fn add_draft(
    State(state): State<Arc<Fixture>>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> impl IntoResponse {
    state.draft_posts.fetch_add(1, Ordering::SeqCst);
    assert_eq!(body["draft"], true);
    assert_eq!(body["base"], "main");
    assert!(
        body["head"]
            .as_str()
            .unwrap()
            .starts_with("factory/features/")
    );
    if state.draft_forbidden {
        return StatusCode::FORBIDDEN.into_response();
    }
    let draft = serde_json::json!({
        "number": 17,
        "state": "open",
        "draft": true,
        "body": body["body"],
        "changed_files": 1,
        "updated_at": "2026-01-01T00:00:00Z",
        "head": {
            "ref": body["head"],
            "sha": state.draft_head_after_post.clone().unwrap_or_else(|| "b".repeat(40)),
            "repo": {"id":3,"full_name":"user/example"}
        },
        "base": {
            "ref": body["base"],
            "sha": "a".repeat(40),
            "repo": {"id":3,"full_name":"user/example"}
        }
    });
    *state.draft.lock().unwrap() = Some(draft.clone());
    if state.hold_draft_response {
        std::future::pending::<()>().await;
    }
    (StatusCode::CREATED, axum::Json(draft)).into_response()
}

const REMOTE_PATCH: &str = "diff --git a/remote.txt b/remote.txt\n--- a/remote.txt\n+++ b/remote.txt\n@@ -1 +1 @@\n-before\n+after\n";

async fn draft_by_number(
    State(state): State<Arc<Fixture>>,
    Path(number): Path<u64>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let mut draft = state.draft.lock().unwrap();
    let Some(draft) = draft.as_mut().filter(|draft| draft["number"] == number) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if headers[header::ACCEPT] == "application/vnd.github.diff" {
        if state.move_base_on_diff {
            draft["base"]["sha"] = "e".repeat(40).into();
        }
        return REMOTE_PATCH.into_response();
    }
    assert_eq!(headers[header::ACCEPT], "application/vnd.github+json");
    axum::Json(draft.clone()).into_response()
}

async fn draft_files(Path(number): Path<u64>) -> impl IntoResponse {
    assert_eq!(number, 17);
    axum::Json(serde_json::json!([{"filename": "remote.txt"}]))
}
struct Server(tokio::task::JoinHandle<()>);
impl Drop for Server {
    fn drop(&mut self) {
        self.0.abort();
    }
}
async fn fixture(state: Fixture) -> (GitHubRepositoryProvider, Arc<Fixture>, Server) {
    let state = Arc::new(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = Router::new()
        .route("/token", post(token))
        .route("/user", get(profile))
        .route("/user/emails", get(emails))
        .route("/user/repos", get(repos))
        .route("/repos/{owner}/{repo}", get(repo_at_origin))
        .route("/repos/user/example/pulls", get(drafts).post(add_draft))
        .route("/repos/user/example/pulls/{number}", get(draft_by_number))
        .route("/repos/user/example/pulls/{number}/files", get(draft_files))
        .route("/user/keys", get(keys).post(add_key))
        .with_state(state.clone());
    let server = Server(tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    }));
    let mut provider = GitHubRepositoryProvider::new(
        "fixture-client".into(),
        "fixture-secret".into(),
        "http://localhost:3001/api/auth/github/callback".into(),
    )
    .unwrap();
    provider.inner.oauth = provider
        .inner
        .oauth
        .set_token_uri(TokenUrl::new(format!("http://{address}/token")).unwrap());
    provider.inner.profile = Url::parse(&format!("http://{address}/user")).unwrap();
    (provider, state, server)
}

#[tokio::test]
async fn lost_draft_response_is_reconciled_only_for_the_reviewed_identity() {
    let (mut provider, state, _server) = fixture(Fixture {
        hold_draft_response: true,
        ..Default::default()
    })
    .await;
    let token = RepositoryAccessToken::new("fixture-token".into()).unwrap();
    let target = RepositoryDraftTarget {
        repository_id: 3,
        full_name: "user/example",
        base: "main",
        head: &format!("factory/features/{}", Uuid::new_v4()),
        head_oid: &"b".repeat(40),
        operation_id: Uuid::new_v4(),
    };
    assert_eq!(provider.find_draft(&token, &target, &Guard).await, Ok(None));
    provider.inner.http = http_client(Duration::from_millis(400)).unwrap();
    assert_eq!(
        provider
            .create_draft(&token, &target, "Reviewed", &Guard)
            .await,
        Err(Error::Unavailable)
    );
    provider.inner.http = http_client(REQUEST_TIMEOUT).unwrap();
    let draft = provider
        .find_draft(&token, &target, &Guard)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(draft.url, "https://github.com/user/example/pull/17");
    assert_eq!(draft.base_oid, "a".repeat(40));
    assert_eq!(state.draft_posts.load(Ordering::SeqCst), 1);

    let unrelated = RepositoryDraftTarget {
        operation_id: Uuid::new_v4(),
        ..target
    };
    assert_eq!(
        provider.find_draft(&token, &unrelated, &Guard).await,
        Err(Error::Conflict)
    );
    state.draft.lock().unwrap().as_mut().unwrap()["base"]["sha"] = "d".repeat(40).into();
    let moved = provider
        .find_draft(&token, &target, &Guard)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(moved.base_oid, "d".repeat(40));

    state.draft.lock().unwrap().as_mut().unwrap()["head"]["sha"] = "c".repeat(40).into();
    let moved_head = provider
        .find_draft(&token, &target, &Guard)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(moved_head.head_oid, "c".repeat(40));
    assert_eq!(moved_head.url, draft.url);
    assert_eq!(state.draft_posts.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn draft_receipt_and_current_diff_survive_a_moved_head_at_creation() {
    let (provider, state, _server) = fixture(Fixture {
        draft_head_after_post: Some("c".repeat(40)),
        ..Default::default()
    })
    .await;
    let token = RepositoryAccessToken::new("fixture-token".into()).unwrap();
    let feature_id = Uuid::new_v4();
    let target = RepositoryDraftTarget {
        repository_id: 3,
        full_name: "user/example",
        base: "main",
        head: &format!("factory/features/{feature_id}"),
        head_oid: &"b".repeat(40),
        operation_id: Uuid::new_v4(),
    };
    let receipt = provider
        .create_draft(&token, &target, "Reviewed", &Guard)
        .await
        .unwrap();
    assert_eq!(receipt.head_oid, "c".repeat(40));
    assert_ne!(receipt.head_oid, target.head_oid);
    state.draft.lock().unwrap().as_mut().unwrap()["base"]["sha"] = "d".repeat(40).into();
    let read_target = RepositoryDraftDiffTarget {
        repository_id: target.repository_id,
        full_name: target.full_name,
        feature_id,
        operation_id: target.operation_id,
        draft_url: &receipt.url,
    };
    let diff = provider
        .draft_diff(&token, &read_target, &Guard)
        .await
        .unwrap();
    assert_eq!(diff.base_oid, "d".repeat(40));
    assert_eq!(diff.head_oid, "c".repeat(40));
    assert_eq!(diff.patch, REMOTE_PATCH);
    assert_eq!(diff.changed_paths, ["remote.txt"]);
    assert_eq!(state.draft_posts.load(Ordering::SeqCst), 1);
    let wrong_repo = RepositoryDraftDiffTarget {
        repository_id: 4,
        ..read_target
    };
    assert!(matches!(
        provider.draft_diff(&token, &wrong_repo, &Guard).await,
        Err(Error::Conflict)
    ));
}

#[tokio::test]
async fn rejects_a_remote_diff_if_the_base_moves_while_it_is_read() {
    let (provider, state, _server) = fixture(Fixture {
        move_base_on_diff: true,
        ..Default::default()
    })
    .await;
    let token = RepositoryAccessToken::new("fixture-token".into()).unwrap();
    let feature_id = Uuid::new_v4();
    let target = RepositoryDraftTarget {
        repository_id: 3,
        full_name: "user/example",
        base: "main",
        head: &format!("factory/features/{feature_id}"),
        head_oid: &"b".repeat(40),
        operation_id: Uuid::new_v4(),
    };
    let receipt = provider
        .create_draft(&token, &target, "Reviewed", &Guard)
        .await
        .unwrap();
    let read_target = RepositoryDraftDiffTarget {
        repository_id: target.repository_id,
        full_name: target.full_name,
        feature_id,
        operation_id: target.operation_id,
        draft_url: &receipt.url,
    };
    assert!(matches!(
        provider.draft_diff(&token, &read_target, &Guard).await,
        Err(Error::Conflict)
    ));
    assert_eq!(state.draft_posts.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn draft_permission_denial_does_not_create_a_receipt() {
    let (provider, state, _server) = fixture(Fixture {
        draft_forbidden: true,
        ..Default::default()
    })
    .await;
    let token = RepositoryAccessToken::new("fixture-token".into()).unwrap();
    let target = RepositoryDraftTarget {
        repository_id: 3,
        full_name: "user/example",
        base: "main",
        head: &format!("factory/features/{}", Uuid::new_v4()),
        head_oid: &"b".repeat(40),
        operation_id: Uuid::new_v4(),
    };
    assert_eq!(
        provider
            .create_draft(&token, &target, "Reviewed", &Guard)
            .await,
        Err(Error::ProviderForbidden)
    );
    assert!(state.draft.lock().unwrap().is_none());
}

#[tokio::test]
async fn requests_required_scopes_and_lists_verified_data() {
    let (provider, _, _server) = fixture(Fixture::default()).await;
    let url = provider
        .authorization_url(&format!("repo.{}", "s".repeat(43)), &"c".repeat(43))
        .unwrap();
    let url = Url::parse(&url).unwrap();
    let scope = url
        .query_pairs()
        .find(|(name, _)| name == "scope")
        .unwrap()
        .1;
    for value in ["repo", "write:public_key", "user:email"] {
        assert!(scope.split(' ').any(|s| s == value));
    }
    let (token, identity) = provider.exchange("code", &"v".repeat(43)).await.unwrap();
    assert_eq!(identity.subject, "123");
    assert_eq!(
        provider.primary_email(&token, &Guard).await.unwrap(),
        "user@example.test"
    );
    let page = provider.repositories(&token, 1).await.unwrap();
    assert_eq!(page.items[0].full_name, "user/example");
    assert!(page.items[0].private);
    assert_eq!(page.next_page, None);
    assert!(provider.repositories(&token, 0).await.is_err());
}

#[tokio::test]
async fn missing_scope_and_unverified_email_fail_closed() {
    let (provider, _, _server) = fixture(Fixture {
        denied_scope: true,
        ..Default::default()
    })
    .await;
    assert!(matches!(
        provider.exchange("code", &"v".repeat(43)).await,
        Err(Error::Forbidden)
    ));
    let (provider, _, _server) = fixture(Fixture {
        no_verified_email: true,
        ..Default::default()
    })
    .await;
    let token = RepositoryAccessToken::new("fixture".into()).unwrap();
    assert!(matches!(
        provider.primary_email(&token, &Guard).await,
        Err(Error::EmailUnavailable)
    ));
}

#[tokio::test]
async fn lost_registration_response_reconciles_without_second_post() {
    let (provider, state, _server) = fixture(Fixture {
        fail_post_response: true,
        ..Default::default()
    })
    .await;
    let token = RepositoryAccessToken::new("fixture".into()).unwrap();
    provider
        .ensure_public_key(&token, KEY, &Guard)
        .await
        .unwrap();
    provider
        .ensure_public_key(&token, KEY, &Guard)
        .await
        .unwrap();
    assert_eq!(state.posts.load(Ordering::SeqCst), 1);
    assert!(matches!(
        provider
            .ensure_public_key(&token, "not-a-key", &Guard)
            .await,
        Err(Error::InvalidInput)
    ));
}

#[tokio::test]
async fn github_rate_limit_header_distinguishes_quota_from_denied_permissions() {
    let token = RepositoryAccessToken::new("fixture".into()).unwrap();
    let (quota, _, _server) = fixture(Fixture {
        forbidden: true,
        quota: true,
        ..Default::default()
    })
    .await;
    assert!(matches!(
        quota.repositories(&token, 1).await,
        Err(Error::RateLimited)
    ));
    let (denied, _, _server) = fixture(Fixture {
        forbidden: true,
        ..Default::default()
    })
    .await;
    assert!(matches!(
        denied.repositories(&token, 1).await,
        Err(Error::ProviderForbidden)
    ));
}

struct RevokedGuard(std::sync::atomic::AtomicUsize);
impl RepositoryOperationGuard for RevokedGuard {
    fn check(
        &self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), Error>> + Send + '_>> {
        Box::pin(async move {
            if self.0.fetch_add(1, Ordering::SeqCst) == 0 {
                Ok(())
            } else {
                Err(Error::ProviderUnauthorized)
            }
        })
    }
}

#[tokio::test]
async fn revoked_epoch_prevents_a_post_after_the_initial_key_listing() {
    let (provider, state, _server) = fixture(Fixture::default()).await;
    let token = RepositoryAccessToken::new("fixture".into()).unwrap();
    let guard = RevokedGuard(AtomicUsize::new(0));
    assert!(matches!(
        provider.ensure_public_key(&token, KEY, &guard).await,
        Err(Error::ProviderUnauthorized)
    ));
    assert_eq!(state.posts.load(Ordering::SeqCst), 0);
}

#[test]
fn retry_after_on_403_reports_secondary_rate_limit() {
    let mut headers = header::HeaderMap::new();
    headers.insert(header::RETRY_AFTER, header::HeaderValue::from_static("60"));
    assert_eq!(
        check_status(StatusCode::FORBIDDEN, &headers),
        Err(Error::RateLimited)
    );
    headers.insert(
        header::RETRY_AFTER,
        header::HeaderValue::from_static("not-a-delay"),
    );
    assert_eq!(
        check_status(StatusCode::FORBIDDEN, &headers),
        Err(Error::ProviderForbidden)
    );
}

#[tokio::test]
async fn secondary_limit_body_is_bounded_and_distinct_from_permission_denial() {
    let token = RepositoryAccessToken::new("fixture".into()).unwrap();
    let (quota, _, _server) = fixture(Fixture {
        forbidden: true,
        forbidden_message: Some(
            "You have exceeded a secondary rate limit. Please wait before retrying.".into(),
        ),
        ..Default::default()
    })
    .await;
    assert!(matches!(
        quota.repositories(&token, 1).await,
        Err(Error::RateLimited)
    ));
    let (denied, _, _server) = fixture(Fixture {
        forbidden: true,
        forbidden_message: Some("User has no access to this repository".into()),
        ..Default::default()
    })
    .await;
    assert!(matches!(
        denied.repositories(&token, 1).await,
        Err(Error::ProviderForbidden)
    ));
    let (oversized, _, _server) = fixture(Fixture {
        forbidden: true,
        forbidden_message: Some("x".repeat(ERROR_BODY_LIMIT + 1)),
        ..Default::default()
    })
    .await;
    assert!(matches!(
        oversized.repositories(&token, 1).await,
        Err(Error::ProviderForbidden)
    ));
}

#[tokio::test]
async fn clone_origin_is_matched_only_for_same_id_and_name_without_redirects() {
    let token = RepositoryAccessToken::new("fixture".into()).unwrap();
    for (state, expected) in [
        (Fixture::default(), RepositoryOriginVerification::Matched),
        (
            Fixture {
                origin_id: Some(4),
                ..Default::default()
            },
            RepositoryOriginVerification::Changed,
        ),
        (
            Fixture {
                origin_name: Some("another/name".into()),
                ..Default::default()
            },
            RepositoryOriginVerification::Changed,
        ),
        (
            Fixture {
                origin_status: 301,
                ..Default::default()
            },
            RepositoryOriginVerification::Changed,
        ),
        (
            Fixture {
                origin_status: 404,
                ..Default::default()
            },
            RepositoryOriginVerification::Changed,
        ),
    ] {
        let (provider, _, _server) = fixture(state).await;
        assert_eq!(
            provider
                .verify_clone_origin(&token, 3, "user/example")
                .await
                .unwrap(),
            expected
        );
    }
    for (status, expected) in [
        (401, Error::ProviderUnauthorized),
        (403, Error::ProviderForbidden),
        (429, Error::RateLimited),
        (500, Error::Unavailable),
    ] {
        let (provider, _, _server) = fixture(Fixture {
            origin_status: status,
            ..Default::default()
        })
        .await;
        assert_eq!(
            provider
                .verify_clone_origin(&token, 3, "user/example")
                .await,
            Err(expected)
        );
    }
    let (provider, _, server) = fixture(Fixture::default()).await;
    drop(server);
    assert!(matches!(
        provider
            .verify_clone_origin(&token, 3, "user/example")
            .await,
        Err(Error::Unavailable)
    ));
    assert!(matches!(
        provider.verify_clone_origin(&token, 3, "../example").await,
        Err(Error::InvalidInput)
    ));
}
