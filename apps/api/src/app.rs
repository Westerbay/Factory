use crate::{
    admin, agent_connections, attachments,
    auth::{self, AuthState},
    config::RuntimeConfig,
    conversations, environments, features, health, http, projects, terminals,
};
use axum::{
    Router,
    http::{HeaderValue, Method, header},
    middleware,
};
use factory_application::agent_connections::AgentConnectionService;
use factory_application::environments::EnvironmentService;
use factory_application::terminals::TerminalService;
use factory_database::identity::IdentityDatabase;
use std::{sync::Arc, time::Duration};
use tower_http::cors::{AllowOrigin, CorsLayer};
use utoipa_axum::router::OpenApiRouter;

/// Composition has no provider, database, filesystem or environment side effects.
#[derive(Clone)]
pub struct IdentityRuntime {
    pub auth: AuthState,
    pub database: Arc<IdentityDatabase>,
}

#[derive(Default)]
pub(crate) struct WorkspaceRuntime {
    pub projects: Option<Arc<projects::runtime::ProjectsRuntime>>,
    pub features: Option<features::FeatureRuntime>,
    pub attachments: Option<Arc<factory_application::attachments::AttachmentService>>,
}

pub fn router(
    config: &RuntimeConfig,
    identity: Option<IdentityRuntime>,
) -> Result<Router, &'static str> {
    router_with_environments(config, identity, None)
}

pub fn router_with_environments(
    config: &RuntimeConfig,
    identity: Option<IdentityRuntime>,
    environments: Option<Arc<EnvironmentService>>,
) -> Result<Router, &'static str> {
    router_with_user_services(config, identity, environments, None, None)
}

pub fn router_with_user_services(
    config: &RuntimeConfig,
    identity: Option<IdentityRuntime>,
    environments: Option<Arc<EnvironmentService>>,
    terminal: Option<Arc<TerminalService>>,
    accounts: Option<Arc<AgentConnectionService>>,
) -> Result<Router, &'static str> {
    router_with_conversations(
        config,
        identity,
        environments,
        terminal,
        accounts,
        None,
        None,
    )
}

pub(crate) fn router_with_conversations(
    config: &RuntimeConfig,
    identity: Option<IdentityRuntime>,
    environments: Option<Arc<EnvironmentService>>,
    terminal: Option<Arc<TerminalService>>,
    accounts: Option<Arc<AgentConnectionService>>,
    claude_accounts: Option<Arc<AgentConnectionService>>,
    conversations: Option<Arc<conversations::runtime::ConversationRuntime>>,
) -> Result<Router, &'static str> {
    router_with_workspace(
        config,
        identity,
        environments,
        terminal,
        (accounts, claude_accounts),
        conversations,
        WorkspaceRuntime::default(),
    )
}

pub(crate) fn router_with_workspace(
    config: &RuntimeConfig,
    identity: Option<IdentityRuntime>,
    environments: Option<Arc<EnvironmentService>>,
    terminal: Option<Arc<TerminalService>>,
    accounts: (
        Option<Arc<AgentConnectionService>>,
        Option<Arc<AgentConnectionService>>,
    ),
    conversations: Option<Arc<conversations::runtime::ConversationRuntime>>,
    workspace: WorkspaceRuntime,
) -> Result<Router, &'static str> {
    let (accounts, claude_accounts) = accounts;
    let origin = HeaderValue::from_str(config.web_origin()).map_err(|_| "WEB_ORIGIN_INVALID")?;
    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::list([origin]))
        .allow_credentials(true)
        .allow_methods([Method::GET, Method::POST, Method::PATCH, Method::DELETE])
        .allow_headers([
            header::CONTENT_TYPE,
            header::RANGE,
            axum::http::HeaderName::from_static("x-csrf-token"),
        ])
        // The browser app reads text and HTML attachments and honors ranges itself.
        .expose_headers([
            header::ACCEPT_RANGES,
            header::CONTENT_LENGTH,
            header::CONTENT_RANGE,
            header::CONTENT_DISPOSITION,
        ])
        .max_age(Duration::from_secs(60));
    let (routes, _) = documented_router_with_workspace(
        identity,
        environments,
        terminal,
        accounts,
        claude_accounts,
        conversations,
        workspace,
    )
    .split_for_parts();
    Ok(routes
        .fallback(http::not_found)
        .layer(middleware::from_fn_with_state(
            config.web_origin().to_owned(),
            http::oauth_navigation_errors,
        ))
        .layer(cors)
        .layer(middleware::from_fn(http::response_headers)))
}

/// Runtime and inert contract export share the same route registration.
pub(crate) fn documented_router(
    identity: Option<IdentityRuntime>,
    environments: Option<Arc<EnvironmentService>>,
    terminal: Option<Arc<TerminalService>>,
    accounts: Option<Arc<AgentConnectionService>>,
    claude_accounts: Option<Arc<AgentConnectionService>>,
    conversations: Option<Arc<conversations::runtime::ConversationRuntime>>,
) -> OpenApiRouter {
    documented_router_with_workspace(
        identity,
        environments,
        terminal,
        accounts,
        claude_accounts,
        conversations,
        WorkspaceRuntime::default(),
    )
}

fn documented_router_with_workspace(
    identity: Option<IdentityRuntime>,
    environments: Option<Arc<EnvironmentService>>,
    terminal: Option<Arc<TerminalService>>,
    accounts: Option<Arc<AgentConnectionService>>,
    claude_accounts: Option<Arc<AgentConnectionService>>,
    conversations: Option<Arc<conversations::runtime::ConversationRuntime>>,
    workspace: WorkspaceRuntime,
) -> OpenApiRouter {
    let database = identity.as_ref().map(|runtime| runtime.database.clone());
    let auth = identity.map(|runtime| runtime.auth.with_projects(workspace.projects.clone()));
    auth::documented_router(auth.clone())
        .merge(admin::documented_router(auth.clone()))
        .merge(environments::documented_router(auth.clone(), environments))
        .merge(terminals::documented_router(auth.clone(), terminal))
        .merge(agent_connections::documented_router(
            auth.clone(),
            accounts,
            claude_accounts,
            conversations
                .clone()
                .map(|v| v as Arc<dyn agent_connections::DisconnectGuard>),
        ))
        .merge(conversations::documented_router(
            auth.clone(),
            conversations,
        ))
        .merge(attachments::documented_router(
            auth.clone(),
            workspace.attachments,
        ))
        .merge(features::documented_router(
            auth.clone(),
            workspace.features,
        ))
        .merge(projects::documented_router(auth, workspace.projects))
        .merge(health::documented_router(database))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;

    async fn request(method: Method, path: &str, origin: &str) -> axum::response::Response {
        let config = RuntimeConfig::from_values(|_| None).unwrap();
        request_with_config(&config, method, path, origin).await
    }

    async fn request_with_config(
        config: &RuntimeConfig,
        method: Method,
        path: &str,
        origin: &str,
    ) -> axum::response::Response {
        router(config, None)
            .unwrap()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(path)
                    .header(header::ORIGIN, origin)
                    .header(header::ACCESS_CONTROL_REQUEST_METHOD, "GET")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn health_is_honest_and_api_routes_never_fall_back_to_html() {
        let live = request(Method::GET, "/health/live", "http://localhost:3000").await;
        assert_eq!(live.status(), 200);
        let body = axum::body::to_bytes(live.into_body(), 1024).await.unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&body).unwrap()["status"],
            "ok"
        );
        for (path, status) in [("/health/ready", 503), ("/api/missing", 404), ("/app", 404)] {
            let response = request(Method::GET, path, "http://localhost:3000").await;
            assert_eq!(response.status(), status);
            assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
            assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");
        }
    }

    #[tokio::test]
    async fn credentials_are_allowed_only_for_the_configured_frontend_origin() {
        for method in [Method::GET, Method::OPTIONS] {
            let allowed = request(method.clone(), "/health/live", "http://localhost:3000").await;
            assert_eq!(
                allowed.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN],
                "http://localhost:3000"
            );
            assert_eq!(
                allowed.headers()[header::ACCESS_CONTROL_ALLOW_CREDENTIALS],
                "true"
            );
            for origin in ["http://localhost:4000", "https://evil.example", "null"] {
                let denied = request(method.clone(), "/health/live", origin).await;
                assert!(
                    !denied
                        .headers()
                        .contains_key(header::ACCESS_CONTROL_ALLOW_ORIGIN)
                );
            }
        }
    }

    #[tokio::test]
    async fn production_custom_web_origin_is_the_only_credentialed_cors_origin() {
        let config = RuntimeConfig::from_values(|name| match name {
            "FACTORY_DEPLOYMENT" => Some("production".into()),
            "FACTORY_WEB_ORIGIN" => Some("https://factory.example.com".into()),
            "FACTORY_DB_PASSWORD" => Some("synthetic-password".into()),
            "FACTORY_GITHUB_CLIENT_ID" => Some("synthetic-client".into()),
            "FACTORY_GITHUB_CLIENT_SECRET" => Some("synthetic-secret".into()),
            _ => None,
        })
        .unwrap();
        for (origin, allowed) in [
            ("https://factory.example.com", true),
            ("https://other.example.com", false),
            ("https://factory-api.example.com", false),
            ("http://factory.example.com", false),
        ] {
            let response =
                request_with_config(&config, Method::OPTIONS, "/health/live", origin).await;
            let headers = response.headers();
            assert_eq!(
                headers
                    .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
                    .and_then(|value| value.to_str().ok()),
                allowed.then_some(origin)
            );
            if allowed {
                assert_eq!(headers[header::ACCESS_CONTROL_ALLOW_CREDENTIALS], "true");
            }
        }
    }

    #[tokio::test]
    async fn browser_app_may_send_ranges_and_read_attachment_headers() {
        let config = RuntimeConfig::from_values(|_| None).unwrap();
        let response = router(&config, None)
            .unwrap()
            .oneshot(
                Request::builder()
                    .method(Method::OPTIONS)
                    .uri("/api/conversations/x/attachments/y/content")
                    .header(header::ORIGIN, "http://localhost:3000")
                    .header(header::ACCESS_CONTROL_REQUEST_METHOD, "GET")
                    .header(header::ACCESS_CONTROL_REQUEST_HEADERS, "range")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let allowed = response.headers()[header::ACCESS_CONTROL_ALLOW_HEADERS]
            .to_str()
            .unwrap();
        assert!(allowed.split(',').any(|name| name.trim() == "range"));
        let live = request(Method::GET, "/health/live", "http://localhost:3000").await;
        let exposed = live.headers()[header::ACCESS_CONTROL_EXPOSE_HEADERS]
            .to_str()
            .unwrap()
            .to_ascii_lowercase();
        for name in ["content-range", "content-length", "accept-ranges"] {
            assert!(exposed.contains(name), "{name}");
        }
    }
}
