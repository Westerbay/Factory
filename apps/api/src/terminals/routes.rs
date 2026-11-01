use super::{contracts::*, socket};
use crate::{
    auth::{AuthError, AuthState, hash_token, random_token},
    contracts::ApiError,
};
use axum::{
    Json, Router,
    body::Bytes,
    extract::{
        DefaultBodyLimit, State, WebSocketUpgrade, rejection::BytesRejection,
        ws::rejection::WebSocketUpgradeRejection,
    },
    http::{HeaderMap, StatusCode, Uri, header},
    response::{IntoResponse, Response},
};
use factory_application::terminals::TerminalService;
use factory_core::{
    environments::EnvironmentActor, identity::IdentityError, terminals::TerminalError,
};
use std::sync::Arc;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

#[derive(Clone)]
struct TerminalState {
    auth: Option<AuthState>,
    service: Option<Arc<TerminalService>>,
}

pub fn router(auth: AuthState, service: Option<Arc<TerminalService>>) -> Router {
    documented_router(Some(auth), service).into()
}

pub(crate) fn documented_router(
    auth: Option<AuthState>,
    service: Option<Arc<TerminalService>>,
) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(issue))
        .routes(routes!(connect))
        .layer(DefaultBodyLimit::max(MAX_CONTROL_BYTES))
        .layer(axum::middleware::from_fn(crate::http::response_headers))
        .with_state(TerminalState { auth, service })
}

impl TerminalState {
    fn dependencies(&self) -> Result<(&AuthState, Arc<TerminalService>), AuthError> {
        let auth = self
            .auth
            .as_ref()
            .ok_or_else(|| AuthError::from(IdentityError::Unavailable))?;
        let service = self
            .service
            .clone()
            .ok_or_else(|| AuthError::from(IdentityError::Unavailable))?;
        Ok((auth, service))
    }
}

#[utoipa::path(post, path = "/api/terminal/attachments", operation_id = "createTerminalAttachment",
    security(("sessionCookie" = [])), request_body = AttachmentRequest,
    params(("X-CSRF-Token" = String, Header, description = "Session CSRF token; exact frontend Origin required")),
    responses(
        (status = 201, description = "Single-use terminal ticket, expires within 30 seconds", body = AttachmentResponse),
        (status = 400, description = "Invalid body", body = ApiError),
        (status = 401, description = "Active session required", body = ApiError),
        (status = 403, description = "Origin or CSRF rejected", body = ApiError),
        (status = 409, description = "Already attached or environment changed", body = ApiError),
        (status = 429, description = "Rate limit", body = ApiError),
        (status = 503, description = "Terminal unavailable", body = ApiError)
    )
)]
async fn issue(
    State(state): State<TerminalState>,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Result<(StatusCode, Json<AttachmentResponse>), Response> {
    let (auth, service) = state.dependencies().map_err(IntoResponse::into_response)?;
    auth.check_quota(false)
        .map_err(IntoResponse::into_response)?;
    let authenticated = auth
        .authenticate(&headers)
        .await
        .map_err(IntoResponse::into_response)?;
    auth.authorize_mutation(&headers, &authenticated, true)
        .map_err(IntoResponse::into_response)?;
    let body = body.map_err(|_| AuthError::invalid().into_response())?;
    let request: AttachmentRequest =
        serde_json::from_slice(&body).map_err(|_| AuthError::invalid().into_response())?;
    let ticket = random_token().map_err(|error| AuthError::from(error).into_response())?;
    let hash = hash_token(&ticket).map_err(|error| AuthError::from(error).into_response())?;
    let actor = EnvironmentActor {
        user_id: authenticated.session.user.id,
        session_hash: authenticated.token_hash,
    };
    let expires_at = if let Some(conversation) = request.conversation_id {
        service
            .issue_for_conversation(
                actor,
                hash,
                request.take_over,
                authenticated.session.expires_at,
                conversation,
            )
            .await
    } else {
        service
            .issue(
                actor,
                hash,
                request.take_over,
                authenticated.session.expires_at,
            )
            .await
    }
    .map_err(error)?;
    Ok((
        StatusCode::CREATED,
        Json(AttachmentResponse { ticket, expires_at }),
    ))
}

#[utoipa::path(get, path = "/api/terminal/socket", operation_id = "connectTerminal",
    security(("sessionCookie" = [])),
    description = "Upgrade with exact Origin and factory.terminal.v1 subprotocol. Within five seconds send TerminalClientMessage.authenticate containing the single-use ticket. Further JSON controls use TerminalClientMessage; data output is binary and controls use TerminalServerMessage. No credentials in URL or subprotocol.",
    responses(
        (status = 101, description = "Authenticated WebSocket upgrade; ticket still required before exec"),
        (status = 400, description = "Invalid upgrade or subprotocol", body = ApiError),
        (status = 401, description = "Active session required", body = ApiError),
        (status = 403, description = "Origin rejected", body = ApiError),
        (status = 429, description = "Rate limit", body = ApiError),
        (status = 503, description = "Terminal unavailable", body = ApiError)
    )
)]
async fn connect(
    State(state): State<TerminalState>,
    headers: HeaderMap,
    uri: Uri,
    upgrade: Result<WebSocketUpgrade, WebSocketUpgradeRejection>,
) -> Result<Response, Response> {
    let (auth, service) = state.dependencies().map_err(IntoResponse::into_response)?;
    auth.check_quota(false)
        .map_err(IntoResponse::into_response)?;
    auth.authorize_origin(&headers)
        .map_err(IntoResponse::into_response)?;
    if uri.query().is_some()
        || !headers
            .get_all(header::SEC_WEBSOCKET_PROTOCOL)
            .iter()
            .any(|value| {
                value.to_str().is_ok_and(|value| {
                    value
                        .split(',')
                        .any(|protocol| protocol.trim() == SUBPROTOCOL)
                })
            })
    {
        return Err(AuthError::invalid().into_response());
    }
    // Subscribe before authentication to close the race with a committed logout.
    let revocations = auth.service().subscribe_revocations();
    let authenticated = auth
        .authenticate(&headers)
        .await
        .map_err(IntoResponse::into_response)?;
    let upgrade = upgrade.map_err(|_| AuthError::invalid().into_response())?;
    let auth = auth.clone();
    Ok(upgrade
        .protocols([SUBPROTOCOL])
        .max_message_size(MAX_MESSAGE_BYTES)
        .max_frame_size(MAX_MESSAGE_BYTES)
        .max_write_buffer_size(OUTPUT_BUFFER_BYTES)
        .write_buffer_size(0)
        .on_upgrade(move |socket| socket::run(socket, auth, authenticated, service, revocations)))
}

const OUTPUT_BUFFER_BYTES: usize = factory_core::terminals::MAX_OUTPUT_BUFFER_BYTES;

pub(super) fn error(error: TerminalError) -> Response {
    let status = match error {
        TerminalError::Unauthorized | TerminalError::InvalidTicket => StatusCode::UNAUTHORIZED,
        TerminalError::Conflict => StatusCode::CONFLICT,
        TerminalError::InvalidInput => StatusCode::BAD_REQUEST,
        TerminalError::Unavailable | TerminalError::CleanupUncertain => {
            StatusCode::SERVICE_UNAVAILABLE
        }
    };
    let code = match error {
        TerminalError::Unauthorized => "unauthorized",
        TerminalError::Unavailable => "unavailable",
        TerminalError::Conflict => "conflict",
        TerminalError::InvalidTicket => "invalid_ticket",
        TerminalError::InvalidInput => "invalid_input",
        TerminalError::CleanupUncertain => "cleanup_uncertain",
    };
    (
        status,
        Json(ApiError {
            code: code.into(),
            message: error.to_string(),
            request_id: Uuid::new_v4(),
        }),
    )
        .into_response()
}
