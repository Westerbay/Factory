use super::{contracts::*, lifecycle};
use crate::{
    auth::{AuthError, AuthState, Authenticated},
    contracts::ApiError,
};
use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, State, rejection::BytesRejection},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use factory_application::agent_connections::AgentConnectionService;
use factory_core::{
    agent_connections::{AgentConnectionError, ConnectionSnapshot},
    environments::EnvironmentActor,
    identity::IdentityError,
};
use std::sync::Arc;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

#[derive(Clone)]
struct ConnectionState {
    auth: Option<AuthState>,
    service: Option<Arc<AgentConnectionService>>,
    disconnect: Option<Arc<dyn DisconnectGuard>>,
}

#[async_trait::async_trait]
pub trait DisconnectGuard: Send + Sync {
    async fn acquire_login(
        &self,
        actor: EnvironmentActor,
    ) -> Result<Box<dyn Send>, AgentConnectionError>;
    async fn logout(
        &self,
        actor: EnvironmentActor,
    ) -> Result<ConnectionSnapshot, AgentConnectionError>;
}

#[async_trait::async_trait]
impl DisconnectGuard for crate::conversations::runtime::ConversationRuntime {
    async fn acquire_login(
        &self,
        actor: EnvironmentActor,
    ) -> Result<Box<dyn Send>, AgentConnectionError> {
        self.acquire_account_change(actor).await
    }
    async fn logout(
        &self,
        actor: EnvironmentActor,
    ) -> Result<ConnectionSnapshot, AgentConnectionError> {
        crate::conversations::runtime::ConversationRuntime::logout(self, actor).await
    }
}

pub fn router(auth: AuthState, service: Option<Arc<AgentConnectionService>>) -> Router {
    documented_router(Some(auth), service, None).into()
}

pub fn router_with_disconnect_guard(
    auth: AuthState,
    service: Option<Arc<AgentConnectionService>>,
    guard: Arc<dyn DisconnectGuard>,
) -> Router {
    documented_router(Some(auth), service, Some(guard)).into()
}

pub(crate) fn documented_router(
    auth: Option<AuthState>,
    service: Option<Arc<AgentConnectionService>>,
    disconnect: Option<Arc<dyn DisconnectGuard>>,
) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(read))
        .routes(routes!(login))
        .routes(routes!(cancel))
        .routes(routes!(logout))
        .layer(DefaultBodyLimit::max(1024))
        .layer(axum::middleware::from_fn(crate::http::response_headers))
        .with_state(ConnectionState {
            auth,
            service,
            disconnect,
        })
}

impl ConnectionState {
    async fn authenticate(
        &self,
        headers: &HeaderMap,
        mutation: Option<bool>,
    ) -> Result<(AuthState, Authenticated), Response> {
        let auth = self
            .auth
            .clone()
            .ok_or_else(|| AuthError::from(IdentityError::Unavailable).into_response())?;
        auth.check_quota(false)
            .map_err(IntoResponse::into_response)?;
        let authenticated = auth
            .authenticate(headers)
            .await
            .map_err(IntoResponse::into_response)?;
        if let Some(has_body) = mutation {
            auth.authorize_mutation(headers, &authenticated, has_body)
                .map_err(IntoResponse::into_response)?;
        }
        Ok((auth, authenticated))
    }

    fn service(&self) -> Result<Arc<AgentConnectionService>, AgentConnectionError> {
        self.service
            .clone()
            .ok_or(AgentConnectionError::Unavailable)
    }
}

#[utoipa::path(get, path = "/api/agent-connections/codex", operation_id = "getCodexConnection",
    security(("sessionCookie" = [])), responses(
        (status = 200, description = "Current account or unavailable state; pending codes belong only to their initiating session", body = AgentConnectionResponse),
        (status = 401, description = "Active session required", body = ApiError),
        (status = 409, description = "Operation owned by another session or environment changed", body = ApiError),
        (status = 429, description = "Rate limit", body = ApiError),
        (status = 503, description = "Identity unavailable", body = ApiError)
    )
)]
async fn read(
    State(state): State<ConnectionState>,
    headers: HeaderMap,
) -> Result<Json<AgentConnectionResponse>, Response> {
    let (auth, authenticated) = state.authenticate(&headers, None).await?;
    let Some(service) = state.service else {
        return Ok(Json(AgentConnectionResponse::unavailable()));
    };
    let actor = actor(&authenticated);
    let result = lifecycle::settle(async move { service.read(actor).await }).await;
    // A provider read may outlive the session that authorized it.
    auth.authenticate(&headers)
        .await
        .map_err(IntoResponse::into_response)?;
    match result {
        Ok(snapshot) => Ok(Json(snapshot.into())),
        Err(problem @ (AgentConnectionError::Unauthorized | AgentConnectionError::Conflict)) => {
            Err(error(problem))
        }
        Err(problem) => Ok(Json(
            ConnectionSnapshot::failure(problem, chrono::Utc::now()).into(),
        )),
    }
}

#[utoipa::path(post, path = "/api/agent-connections/codex/login", operation_id = "beginCodexLogin",
    security(("sessionCookie" = [])),
    params(("X-CSRF-Token" = String, Header, description = "Session CSRF token; exact frontend Origin required; empty request body")),
    responses(
        (status = 202, description = "Device authorization started; may already be connected", body = AgentConnectionResponse),
        (status = 400, description = "Body must be empty", body = ApiError),
        (status = 401, description = "Active session required", body = ApiError),
        (status = 403, description = "Origin or CSRF rejected", body = ApiError),
        (status = 409, description = "Already pending, expired or environment changed", body = ApiError),
        (status = 422, description = "Unsupported authentication method", body = ApiError),
        (status = 429, description = "Rate limit", body = ApiError),
        (status = 502, description = "Invalid provider response or login failure", body = ApiError),
        (status = 503, description = "Provider unavailable or cleanup uncertain", body = ApiError)
    )
)]
async fn login(
    State(state): State<ConnectionState>,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Result<(StatusCode, Json<AgentConnectionResponse>), Response> {
    let (auth, authenticated) = state.authenticate(&headers, Some(false)).await?;
    require_empty(body).map_err(IntoResponse::into_response)?;
    let service = state.service().map_err(error)?;
    let guard = state
        .disconnect
        .ok_or_else(|| error(AgentConnectionError::Unavailable))?;
    let actor = actor(&authenticated);
    let held = guard.acquire_login(actor).await.map_err(error)?;
    let snapshot = lifecycle::begin(auth, service, authenticated, held)
        .await
        .map_err(error)?;
    Ok((StatusCode::ACCEPTED, Json(snapshot.into())))
}

#[utoipa::path(post, path = "/api/agent-connections/codex/login/cancel", operation_id = "cancelCodexLogin",
    security(("sessionCookie" = [])), request_body = CancelAgentLoginBody,
    params(("X-CSRF-Token" = String, Header, description = "Session CSRF token; exact frontend Origin required")),
    responses(
        (status = 200, description = "Operation cancellation confirmed", body = AgentConnectionResponse),
        (status = 400, description = "Invalid body", body = ApiError),
        (status = 401, description = "Active session required", body = ApiError),
        (status = 403, description = "Origin or CSRF rejected", body = ApiError),
        (status = 409, description = "Operation differs or environment changed", body = ApiError),
        (status = 422, description = "Unsupported authentication method", body = ApiError),
        (status = 429, description = "Rate limit", body = ApiError),
        (status = 502, description = "Invalid provider response or login failure", body = ApiError),
        (status = 503, description = "Provider unavailable or cleanup uncertain", body = ApiError)
    )
)]
async fn cancel(
    State(state): State<ConnectionState>,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Result<Json<AgentConnectionResponse>, Response> {
    let (_, authenticated) = state.authenticate(&headers, Some(true)).await?;
    let body = body.map_err(|_| AuthError::invalid().into_response())?;
    let request: CancelAgentLoginBody =
        serde_json::from_slice(&body).map_err(|_| AuthError::invalid().into_response())?;
    let service = state.service().map_err(error)?;
    let actor = actor(&authenticated);
    lifecycle::settle(async move { service.cancel_login(actor, request.operation_id).await })
        .await
        .map(|snapshot| Json(snapshot.into()))
        .map_err(error)
}

#[utoipa::path(post, path = "/api/agent-connections/codex/logout", operation_id = "logoutCodex",
    security(("sessionCookie" = [])),
    params(("X-CSRF-Token" = String, Header, description = "Session CSRF token; exact frontend Origin required; empty request body")),
    responses(
        (status = 200, description = "Provider logout confirmed", body = AgentConnectionResponse),
        (status = 400, description = "Body must be empty", body = ApiError),
        (status = 401, description = "Active session required", body = ApiError),
        (status = 403, description = "Origin or CSRF rejected", body = ApiError),
        (status = 409, description = "Operation owned by another session or environment changed", body = ApiError),
        (status = 422, description = "Unsupported authentication method", body = ApiError),
        (status = 429, description = "Rate limit", body = ApiError),
        (status = 502, description = "Invalid provider response or login failure", body = ApiError),
        (status = 503, description = "Provider unavailable or cleanup uncertain", body = ApiError)
    )
)]
async fn logout(
    State(state): State<ConnectionState>,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Result<Json<AgentConnectionResponse>, Response> {
    let (_, authenticated) = state.authenticate(&headers, Some(false)).await?;
    require_empty(body).map_err(IntoResponse::into_response)?;
    let conversations = state
        .disconnect
        .ok_or_else(|| error(AgentConnectionError::Unavailable))?;
    let actor = actor(&authenticated);
    lifecycle::settle(async move { conversations.logout(actor).await })
        .await
        .map(|snapshot| Json(snapshot.into()))
        .map_err(error)
}

fn actor(authenticated: &Authenticated) -> EnvironmentActor {
    EnvironmentActor {
        user_id: authenticated.session.user.id,
        session_hash: authenticated.token_hash,
    }
}

fn require_empty(body: Result<Bytes, BytesRejection>) -> Result<(), AuthError> {
    match body {
        Ok(body) if body.is_empty() => Ok(()),
        _ => Err(AuthError::invalid()),
    }
}

fn error(error: AgentConnectionError) -> Response {
    let status = match error {
        AgentConnectionError::Unauthorized => StatusCode::UNAUTHORIZED,
        AgentConnectionError::Conflict
        | AgentConnectionError::Cancelled
        | AgentConnectionError::Expired => StatusCode::CONFLICT,
        AgentConnectionError::UnsupportedMethod => StatusCode::UNPROCESSABLE_ENTITY,
        AgentConnectionError::Protocol | AgentConnectionError::LoginFailed => {
            StatusCode::BAD_GATEWAY
        }
        AgentConnectionError::Unavailable
        | AgentConnectionError::Timeout
        | AgentConnectionError::CleanupUncertain => StatusCode::SERVICE_UNAVAILABLE,
    };
    (
        status,
        Json(ApiError {
            code: error.code().into(),
            message: error.to_string(),
            request_id: Uuid::new_v4(),
        }),
    )
        .into_response()
}
