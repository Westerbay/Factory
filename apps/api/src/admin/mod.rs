use crate::{
    auth::{AuthError, AuthState, Authenticated},
    contracts::{AccessBody, AdminUser, UsersQuery, UsersResponse},
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
    routing::{get, patch},
};
use factory_core::identity::require_admin;
use uuid::Uuid;

pub fn router(state: AuthState) -> Router {
    Router::new()
        .route("/api/admin/users", get(list_users))
        .route("/api/admin/users/{id}/access", patch(change_access))
        .layer(DefaultBodyLimit::max(8192))
        .layer(middleware::from_fn_with_state(state.clone(), protect))
        .with_state(state)
}

async fn protect(
    State(state): State<AuthState>,
    request: axum::extract::Request,
    next: Next,
) -> Response {
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

async fn list_users(
    State(state): State<AuthState>,
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

async fn change_access(
    State(state): State<AuthState>,
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
