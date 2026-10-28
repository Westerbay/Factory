use crate::{
    admin,
    auth::{self, AuthState},
    config::RuntimeConfig,
    health, http,
};
use axum::{
    Router,
    http::{HeaderValue, Method, header},
    middleware,
};
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

pub fn router(
    config: &RuntimeConfig,
    identity: Option<IdentityRuntime>,
) -> Result<Router, &'static str> {
    let origin = HeaderValue::from_str(config.web_origin()).map_err(|_| "WEB_ORIGIN_INVALID")?;
    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::list([origin]))
        .allow_credentials(true)
        .allow_methods([Method::GET, Method::POST, Method::PATCH])
        .allow_headers([
            header::CONTENT_TYPE,
            axum::http::HeaderName::from_static("x-csrf-token"),
        ])
        .max_age(Duration::from_secs(60));
    let (routes, _) = documented_router(identity).split_for_parts();
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
pub(crate) fn documented_router(identity: Option<IdentityRuntime>) -> OpenApiRouter {
    let database = identity.as_ref().map(|runtime| runtime.database.clone());
    let auth = identity.map(|runtime| runtime.auth);
    auth::documented_router(auth.clone())
        .merge(admin::documented_router(auth))
        .merge(health::documented_router(database))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;

    async fn request(method: Method, path: &str, origin: &str) -> axum::response::Response {
        let config = RuntimeConfig::from_values(|_| None).unwrap();
        router(&config, None)
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
}
