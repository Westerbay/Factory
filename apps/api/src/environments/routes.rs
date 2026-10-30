use super::EnvironmentResponse;
use crate::{
    auth::{AuthError, AuthState},
    contracts::ApiError,
};
use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, State, rejection::BytesRejection},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use factory_application::environments::EnvironmentService;
use factory_core::{
    environments::{EnvironmentActor, EnvironmentError},
    identity::IdentityError,
};
use std::sync::Arc;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

#[derive(Clone)]
struct EnvironmentState {
    auth: Option<AuthState>,
    service: Option<Arc<EnvironmentService>>,
}

pub fn router(auth: AuthState, service: Option<Arc<EnvironmentService>>) -> Router {
    documented_router(Some(auth), service).into()
}

pub(crate) fn documented_router(
    auth: Option<AuthState>,
    service: Option<Arc<EnvironmentService>>,
) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(get))
        .routes(routes!(start))
        .routes(routes!(stop))
        .layer(DefaultBodyLimit::max(0))
        .layer(axum::middleware::from_fn(crate::http::response_headers))
        .with_state(EnvironmentState { auth, service })
}

impl EnvironmentState {
    async fn actor(
        &self,
        headers: &HeaderMap,
        mutation: bool,
    ) -> Result<EnvironmentActor, Response> {
        let auth = self
            .auth
            .as_ref()
            .ok_or_else(|| AuthError::from(IdentityError::Unavailable).into_response())?;
        auth.check_quota(false)
            .map_err(IntoResponse::into_response)?;
        let authenticated = auth
            .authenticate(headers)
            .await
            .map_err(IntoResponse::into_response)?;
        if mutation {
            auth.authorize_mutation(headers, &authenticated, false)
                .map_err(IntoResponse::into_response)?;
        }
        Ok(EnvironmentActor {
            user_id: authenticated.session.user.id,
            session_hash: authenticated.token_hash,
        })
    }

    fn service(&self) -> Result<&EnvironmentService, EnvironmentError> {
        self.service.as_deref().ok_or(EnvironmentError::Unavailable)
    }
}

#[utoipa::path(
    get, path = "/api/environment", operation_id = "getEnvironment",
    security(("sessionCookie" = [])),
    description = "Read only the current user's environment. Does not create or start resources.",
    responses(
        (status = 200, description = "Environment state, including unavailable when not prepared", body = EnvironmentResponse),
        (status = 401, description = "Active session required", body = ApiError),
        (status = 429, description = "Rate limit", body = ApiError),
        (status = 503, description = "Identity or persistence unavailable", body = ApiError)
    )
)]
async fn get(
    State(state): State<EnvironmentState>,
    headers: HeaderMap,
) -> Result<Json<EnvironmentResponse>, Response> {
    let actor = state.actor(&headers, false).await?;
    let Some(service) = &state.service else {
        return Ok(Json(EnvironmentResponse::project(None, false, false)));
    };
    let environment = service.get(actor).await.map_err(error)?;
    Ok(Json(EnvironmentResponse::project(
        environment.as_ref(),
        service.can_report(environment.as_ref()),
        environment
            .as_ref()
            .is_some_and(|environment| service.needs_upgrade(environment)),
    )))
}

#[utoipa::path(
    post, path = "/api/environment/start", operation_id = "startEnvironment",
    security(("sessionCookie" = [])),
    params(("X-CSRF-Token" = String, Header, description = "Session CSRF token. Exact frontend Origin and an empty body are required.")),
    description = "Persist a persistent running intent for the current user. Opposite intents are queued by generation.",
    responses(
        (status = 202, description = "Running intent accepted", body = EnvironmentResponse),
        (status = 400, description = "Body must be empty", body = ApiError),
        (status = 401, description = "Active session required", body = ApiError),
        (status = 403, description = "Origin or CSRF rejected", body = ApiError),
        (status = 409, description = "Conflicting intent", body = ApiError),
        (status = 429, description = "Rate limit", body = ApiError),
        (status = 503, description = "Environment service unavailable", body = ApiError)
    )
)]
async fn start(
    State(state): State<EnvironmentState>,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Result<(StatusCode, Json<EnvironmentResponse>), Response> {
    let actor = state.actor(&headers, true).await?;
    empty_body(body).map_err(IntoResponse::into_response)?;
    let service = state.service().map_err(error)?;
    let environment = service.start(actor).await.map_err(error)?;
    Ok((
        StatusCode::ACCEPTED,
        Json(EnvironmentResponse::project(
            Some(&environment),
            service.can_report(Some(&environment)),
            service.needs_upgrade(&environment),
        )),
    ))
}

#[utoipa::path(
    post, path = "/api/environment/stop", operation_id = "stopEnvironment",
    security(("sessionCookie" = [])),
    params(("X-CSRF-Token" = String, Header, description = "Session CSRF token. Exact frontend Origin and an empty body are required.")),
    description = "Persist a stopped intent; no volume is removed.",
    responses(
        (status = 202, description = "Stopped intent accepted", body = EnvironmentResponse),
        (status = 400, description = "Body must be empty", body = ApiError),
        (status = 401, description = "Active session required", body = ApiError),
        (status = 403, description = "Origin or CSRF rejected", body = ApiError),
        (status = 409, description = "Conflicting intent", body = ApiError),
        (status = 429, description = "Rate limit", body = ApiError),
        (status = 503, description = "Environment service unavailable", body = ApiError)
    )
)]
async fn stop(
    State(state): State<EnvironmentState>,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Result<(StatusCode, Json<EnvironmentResponse>), Response> {
    let actor = state.actor(&headers, true).await?;
    empty_body(body).map_err(IntoResponse::into_response)?;
    let service = state.service().map_err(error)?;
    let environment = service.stop(actor).await.map_err(error)?;
    Ok((
        StatusCode::ACCEPTED,
        Json(EnvironmentResponse::project(
            environment.as_ref(),
            service.can_report(environment.as_ref()),
            environment
                .as_ref()
                .is_some_and(|environment| service.needs_upgrade(environment)),
        )),
    ))
}

fn empty_body(body: Result<Bytes, BytesRejection>) -> Result<(), AuthError> {
    if body.is_ok_and(|body| body.is_empty()) {
        Ok(())
    } else {
        Err(AuthError::invalid())
    }
}

fn error(error: EnvironmentError) -> Response {
    let status = match error {
        EnvironmentError::Unauthorized => StatusCode::UNAUTHORIZED,
        EnvironmentError::Conflict => StatusCode::CONFLICT,
        _ => StatusCode::SERVICE_UNAVAILABLE,
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
