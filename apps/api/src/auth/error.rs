use crate::contracts::ApiError;
use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use factory_core::identity::IdentityError;
use uuid::Uuid;

pub struct AuthError {
    pub status: StatusCode,
    code: &'static str,
    message: &'static str,
}

impl AuthError {
    pub fn invalid() -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code: "invalid-input",
            message: "Invalid request.",
        }
    }
    pub fn forbidden() -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            code: "forbidden",
            message: "Access denied.",
        }
    }
    pub fn rate_limited() -> Self {
        Self {
            status: StatusCode::TOO_MANY_REQUESTS,
            code: "rate-limited",
            message: "Too many requests. Try again later.",
        }
    }
}

impl From<IdentityError> for AuthError {
    fn from(error: IdentityError) -> Self {
        let (status, code, message) = match error {
            IdentityError::InvalidInput => {
                (StatusCode::BAD_REQUEST, "invalid-input", "Invalid request.")
            }
            IdentityError::InvalidLogin => {
                (StatusCode::BAD_REQUEST, "invalid-login", "Restart sign-in.")
            }
            IdentityError::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                "session-required",
                "Sign in to continue.",
            ),
            IdentityError::Forbidden => (StatusCode::FORBIDDEN, "forbidden", "Access denied."),
            IdentityError::NotFound => (StatusCode::NOT_FOUND, "not-found", "User not found."),
            IdentityError::Conflict => (
                StatusCode::CONFLICT,
                "version-conflict",
                "The user changed. Refresh and retry.",
            ),
            IdentityError::ProtectedAdministrator => (
                StatusCode::CONFLICT,
                "protected-administrator",
                "Administrator access cannot be changed.",
            ),
            IdentityError::Unavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                "unavailable",
                "Service unavailable. Try again later.",
            ),
        };
        Self {
            status,
            code,
            message,
        }
    }
}

impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        let mut response = (
            self.status,
            Json(ApiError {
                code: self.code.into(),
                message: self.message.into(),
                request_id: Uuid::new_v4(),
            }),
        )
            .into_response();
        response.headers_mut().insert(
            axum::http::header::CACHE_CONTROL,
            axum::http::HeaderValue::from_static("no-store"),
        );
        if self.status == StatusCode::TOO_MANY_REQUESTS {
            response.headers_mut().insert(
                axum::http::header::RETRY_AFTER,
                axum::http::HeaderValue::from_static("60"),
            );
        }
        response
    }
}
