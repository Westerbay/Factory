use crate::{
    auth::{AuthError, AuthState, Authenticated, RequiredIdentity},
    contracts::{AccessBody, AdminUser, ApiError, UsersQuery, UsersResponse},
};
use axum::{
    Json, Router,
    body::Bytes,
    extract::{
        DefaultBodyLimit, Path, Query, State,
        rejection::{BytesRejection, PathRejection, QueryRejection},
    },
    http::{HeaderMap, HeaderValue, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
};
use factory_core::identity::{IdentityError, require_admin};
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

pub fn router(state: AuthState) -> Router {
    documented_router(Some(state)).into()
}

pub(crate) fn documented_router(state: Option<AuthState>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(list_users))
        .routes(routes!(change_access))
        .layer(DefaultBodyLimit::max(8192))
        .layer(middleware::from_fn_with_state(state.clone(), protect))
        .with_state(state)
}

async fn protect(
    State(state): State<Option<AuthState>>,
    request: axum::extract::Request,
    next: Next,
) -> Response {
    let Some(state) = state else {
        return AuthError::from(IdentityError::Unavailable).into_response();
    };
    let mut response = match state.check_quota(false) {
        Ok(()) if request.uri().to_string().len() <= 8192 => next.run(request).await,
        Ok(()) => AuthError::invalid().into_response(),
        Err(error) => error.into_response(),
    };
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

async fn administrator(state: &AuthState, headers: &HeaderMap) -> Result<Authenticated, AuthError> {
    let auth = state.authenticate(headers).await?;
    require_admin(&auth.session.user)?;
    Ok(auth)
}

#[utoipa::path(
    get, path = "/api/admin/users", operation_id = "listUsers",
    security(("sessionCookie" = [])),
    params(UsersQuery),
    responses(
        (status = 200, description = "Users in stable UUID order", body = UsersResponse),
        (status = 400, description = "Invalid input", body = ApiError),
        (status = 401, description = "Session required", body = ApiError),
        (status = 403, description = "Access denied", body = ApiError),
        (status = 404, description = "User not found", body = ApiError),
        (status = 409, description = "Conflict or protected administrator", body = ApiError),
        (status = 429, description = "Rate limit", body = ApiError),
        (status = 503, description = "Dependency unavailable", body = ApiError)
    )
)]
async fn list_users(
    RequiredIdentity(state): RequiredIdentity,
    headers: HeaderMap,
    query: Result<Query<UsersQuery>, QueryRejection>,
) -> Result<Json<UsersResponse>, AuthError> {
    let auth = administrator(&state, &headers).await?;
    let Query(query) = query.map_err(|_| AuthError::invalid())?;
    let page = state
        .service()
        .list_users(auth.token_hash, query.into_domain()?)
        .await?;
    Ok(Json(page.into()))
}

#[utoipa::path(
    patch, path = "/api/admin/users/{id}/access", operation_id = "changeAccess",
    security(("sessionCookie" = [])),
    params(
        ("id" = Uuid, Path),
        ("X-CSRF-Token" = String, Header, description = "Session-bound CSRF value; exact frontend Origin also required.")
    ),
    request_body = AccessBody,
    responses(
        (status = 200, description = "Updated member access", body = AdminUser),
        (status = 400, description = "Invalid input", body = ApiError),
        (status = 401, description = "Session required", body = ApiError),
        (status = 403, description = "Access denied", body = ApiError),
        (status = 404, description = "User not found", body = ApiError),
        (status = 409, description = "Conflict or protected administrator", body = ApiError),
        (status = 429, description = "Rate limit", body = ApiError),
        (status = 503, description = "Dependency unavailable", body = ApiError)
    )
)]
async fn change_access(
    RequiredIdentity(state): RequiredIdentity,
    headers: HeaderMap,
    path: Result<Path<Uuid>, PathRejection>,
    body: Result<Bytes, BytesRejection>,
) -> Result<Json<AdminUser>, AuthError> {
    let auth = administrator(&state, &headers).await?;
    let body = body.map_err(|_| AuthError::invalid())?;
    state.authorize_mutation(&headers, &auth, !body.is_empty())?;
    let Path(id) = path.map_err(|_| AuthError::invalid())?;
    let body: AccessBody = serde_json::from_slice(&body).map_err(|_| AuthError::invalid())?;
    let user = state
        .service()
        .change_access(auth.token_hash, body.into_request(id)?)
        .await?;
    Ok(Json(user.into()))
}

#[cfg(test)]
mod tests;
