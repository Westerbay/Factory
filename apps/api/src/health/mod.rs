use axum::{Json, http::StatusCode};
use serde::Serialize;

#[derive(Serialize)]
pub struct Live {
    status: &'static str,
}

pub async fn live() -> Json<Live> {
    Json(Live { status: "ok" })
}

#[derive(Serialize)]
pub struct Readiness {
    status: &'static str,
    code: &'static str,
}

/// The shell must not claim that unimplemented identity/storage is operational.
pub async fn ready() -> (StatusCode, Json<Readiness>) {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(Readiness {
            status: "unavailable",
            code: "foundation-incomplete",
        }),
    )
}
