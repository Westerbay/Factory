use super::{AuthError, AuthState, credentials::*};
use crate::contracts::SessionResponse;
use axum::{
    Json, Router,
    body::Bytes,
    extract::{
        DefaultBodyLimit, Query, State,
        rejection::{BytesRejection, QueryRejection},
    },
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use factory_core::identity::{
    AdmissionOutcome, IdentityError, LOGIN_ATTEMPT_TTL_SECONDS, NewLoginAttempt, NewSession,
    SESSION_TTL_SECONDS,
};
use serde::Deserialize;

pub fn router(state: AuthState) -> Router {
    Router::new()
        .route("/api/auth/github/start", get(start))
        .route("/api/auth/github/callback", get(callback))
        .route("/api/session", get(session))
        .route("/api/auth/logout", post(logout))
        .layer(DefaultBodyLimit::max(8192))
        .layer(middleware::from_fn_with_state(state.clone(), protect))
        .with_state(state)
}

async fn protect(
    State(state): State<AuthState>,
    request: axum::extract::Request,
    next: Next,
) -> Response {
    let login = request.uri().path().starts_with("/api/auth/github/");
    let callback = request.uri().path() == "/api/auth/github/callback";
    let mut response = match state.check_quota(login) {
        Ok(()) if request.uri().to_string().len() <= 8192 => next.run(request).await,
        Ok(()) => AuthError::invalid().into_response(),
        Err(error) => error.into_response(),
    };
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response.headers_mut().insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    if callback && response.status().is_client_error() {
        response.headers_mut().append(
            header::SET_COOKIE,
            clear_cookie(LOGIN_COOKIE, state.secure_cookie),
        );
    }
    response
}

async fn start(State(state): State<AuthState>) -> Result<Response, AuthError> {
    let csrf_state = random_token()?;
    let binding = random_token()?;
    let verifier = oauth2::PkceCodeVerifier::new(random_token()?);
    let challenge = oauth2::PkceCodeChallenge::from_code_verifier_sha256(&verifier);
    let url = state
        .service()
        .begin_login(
            NewLoginAttempt {
                state_hash: hash_token(&csrf_state)?,
                binding_hash: hash_token(&binding)?,
                pkce_verifier: verifier.secret().to_owned(),
            },
            &csrf_state,
            challenge.as_str(),
        )
        .await?;
    let mut response = redirect(&url);
    response.headers_mut().append(
        header::SET_COOKIE,
        cookie(
            LOGIN_COOKIE,
            binding,
            LOGIN_ATTEMPT_TTL_SECONDS,
            state.secure_cookie,
        ),
    );
    Ok(response)
}

#[derive(Deserialize)]
struct Callback {
    state: Option<String>,
    code: Option<String>,
    error: Option<String>,
}

async fn callback(
    State(state): State<AuthState>,
    headers: HeaderMap,
    query: Result<Query<Callback>, QueryRejection>,
) -> Response {
    let outcome = complete_callback(&state, &headers, query).await;
    let mut response = match outcome {
        Ok(response) => response,
        Err(error) => callback_redirect(
            &state,
            if error == IdentityError::Unavailable {
                "unavailable"
            } else {
                "invalid-login"
            },
        ),
    };
    response.headers_mut().append(
        header::SET_COOKIE,
        clear_cookie(LOGIN_COOKIE, state.secure_cookie),
    );
    response
}

async fn complete_callback(
    state: &AuthState,
    headers: &HeaderMap,
    query: Result<Query<Callback>, QueryRejection>,
) -> Result<Response, IdentityError> {
    let Query(query) = query.map_err(|_| IdentityError::InvalidLogin)?;
    let raw_state = query.state.ok_or(IdentityError::InvalidLogin)?;
    let state_hash = hash_token(&raw_state)?;
    let binding_hash = credential(headers, LOGIN_COOKIE)
        .map_err(|_| IdentityError::InvalidLogin)?
        .ok_or(IdentityError::InvalidLogin)?;
    if query.error.is_some() {
        state
            .service()
            .cancel_login(state_hash, binding_hash)
            .await?;
        return Ok(callback_redirect(state, "denied"));
    }
    let code = query.code.ok_or(IdentityError::InvalidLogin)?;
    let previous = credential(headers, SESSION_COOKIE).map_err(|_| IdentityError::InvalidLogin)?;
    let raw_session = random_token()?;
    let outcome = state
        .service()
        .complete_login(
            state_hash,
            binding_hash,
            &code,
            NewSession {
                token_hash: hash_token(&raw_session)?,
                csrf_token: random_token()?,
                previous_token_hash: previous,
            },
        )
        .await?;
    let mut response = match outcome {
        AdmissionOutcome::Active(_) => {
            let mut response = redirect(&format!("{}/app", state.web_origin));
            response.headers_mut().append(
                header::SET_COOKIE,
                cookie(
                    SESSION_COOKIE,
                    raw_session,
                    SESSION_TTL_SECONDS,
                    state.secure_cookie,
                ),
            );
            return Ok(response);
        }
        AdmissionOutcome::Pending => callback_redirect(state, "pending"),
        AdmissionOutcome::Suspended => callback_redirect(state, "suspended"),
        AdmissionOutcome::StaleLogin => callback_redirect(state, "invalid-login"),
    };
    if let Some(hash) = previous
        && state.service().logout(hash).await.is_err()
    {
        response = callback_redirect(state, "unavailable");
    }
    response.headers_mut().append(
        header::SET_COOKIE,
        clear_cookie(SESSION_COOKIE, state.secure_cookie),
    );
    Ok(response)
}

fn callback_redirect(state: &AuthState, reason: &str) -> Response {
    redirect(&format!("{}/sign-in?reason={reason}", state.web_origin))
}

fn redirect(location: &str) -> Response {
    (StatusCode::FOUND, [(header::LOCATION, location)]).into_response()
}

async fn session(
    State(state): State<AuthState>,
    headers: HeaderMap,
) -> Result<Json<SessionResponse>, AuthError> {
    Ok(Json(state.authenticate(&headers).await?.session.into()))
}

async fn logout(
    State(state): State<AuthState>,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Result<Response, AuthError> {
    let auth = state.authenticate(&headers).await?;
    let body = body.map_err(|_| AuthError::invalid())?;
    state.authorize_mutation(&headers, &auth, !body.is_empty())?;
    if !body.is_empty() {
        return Err(AuthError::invalid());
    }
    state.service().logout(auth.token_hash).await?;
    let mut response = StatusCode::NO_CONTENT.into_response();
    response.headers_mut().append(
        header::SET_COOKIE,
        clear_cookie(SESSION_COOKIE, state.secure_cookie),
    );
    Ok(response)
}
