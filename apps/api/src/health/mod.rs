use axum::{Json, http::StatusCode};
use factory_database::{identity::IdentityDatabase, migrations::Readiness};
use serde::Serialize;
use std::sync::Arc;

#[derive(Serialize)]
pub struct Live {
    status: &'static str,
}
pub async fn live() -> Json<Live> {
    Json(Live { status: "ok" })
}

#[derive(Serialize)]
pub struct Ready {
    status: &'static str,
    code: &'static str,
}
pub async fn ready(database: Option<Arc<IdentityDatabase>>) -> (StatusCode, Json<Ready>) {
    let code = match database {
        None => "identity-unavailable",
        Some(database) => match database.ready().await {
            Ok(Readiness::Ready) => {
                return (
                    StatusCode::OK,
                    Json(Ready {
                        status: "ok",
                        code: "ready",
                    }),
                );
            }
            Ok(Readiness::Missing | Readiness::Pending) => "identity-migration-required",
            Ok(Readiness::Invalid) => "identity-schema-invalid",
            Err(_) => "identity-storage-unavailable",
        },
    };
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(Ready {
            status: "unavailable",
            code,
        }),
    )
}
