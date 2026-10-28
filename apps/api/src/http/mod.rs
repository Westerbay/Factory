use crate::contracts::ApiError;
use axum::{
    Json,
    extract::{Request, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    middleware::Next,
    response::Response,
};
use uuid::Uuid;

/// OAuth starts are both API requests and top-level browser navigations. Keep
/// machine errors intact, but return browser failures to the public sign-in UI.
pub async fn oauth_navigation_errors(
    State(web_origin): State<String>,
    request: Request,
    next: Next,
) -> Response {
    let start = request.method() == Method::GET && request.uri().path() == "/api/auth/github/start";
    let browser = start && accepts_html(request.headers());
    let mut response = next.run(request).await;
    if start {
        response
            .headers_mut()
            .append(header::VARY, HeaderValue::from_static("Accept"));
    }
    if browser && (response.status().is_client_error() || response.status().is_server_error()) {
        let reason = if response.status() == StatusCode::BAD_REQUEST {
            "invalid-login"
        } else {
            "unavailable"
        };
        let location = HeaderValue::from_str(&format!("{web_origin}/sign-in?reason={reason}"))
            .expect("RuntimeConfig validates the local frontend origin before composition");
        *response.status_mut() = StatusCode::FOUND;
        response.headers_mut().insert(header::LOCATION, location);
        // Retain the safe error envelope and headers (notably Retry-After).
    }
    response
}

fn accepts_html(headers: &HeaderMap) -> bool {
    headers
        .get_all(header::ACCEPT)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .any(|value| {
            value.split(',').any(|range| {
                let mut parts = range.split(';');
                if !parts
                    .next()
                    .is_some_and(|media| media.trim().eq_ignore_ascii_case("text/html"))
                {
                    return false;
                }
                let mut quality = None;
                for parameter in parts {
                    if let Some((key, value)) = parameter.trim().split_once('=')
                        && key.trim().eq_ignore_ascii_case("q")
                    {
                        if quality.is_some() {
                            return false;
                        }
                        let Ok(value) = value.trim().parse::<f32>() else {
                            return false;
                        };
                        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                            return false;
                        }
                        quality = Some(value);
                    }
                }
                quality.unwrap_or(1.0) > 0.0
            })
        })
}

pub async fn not_found() -> (StatusCode, Json<ApiError>) {
    (
        StatusCode::NOT_FOUND,
        Json(ApiError {
            code: "not-found".into(),
            message: "No API route matches this request.".into(),
            request_id: Uuid::new_v4(),
        }),
    )
}

pub async fn response_headers(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response.headers_mut().insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    response
}
