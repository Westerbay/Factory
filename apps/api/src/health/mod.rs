use axum::{Json, extract::State, http::StatusCode};
use factory_database::{identity::IdentityDatabase, schema::Readiness};
use serde::Serialize;
use std::sync::Arc;
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};

pub(crate) fn documented_router(database: Option<Arc<IdentityDatabase>>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(live))
        .routes(routes!(ready))
        .with_state(database)
}

#[derive(Serialize, ToSchema)]
pub struct Live {
    #[schema(inline)]
    status: LiveStatus,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
enum LiveStatus {
    Ok,
}
#[utoipa::path(
    get, path = "/health/live", operation_id = "healthLive",
    responses((status = 200, description = "Process is alive", body = Live))
)]
pub async fn live() -> Json<Live> {
    Json(Live {
        status: LiveStatus::Ok,
    })
}

#[derive(Serialize, ToSchema)]
pub struct Ready {
    status: &'static str,
    code: &'static str,
}
#[utoipa::path(
    get, path = "/health/ready", operation_id = "healthReady",
    responses(
        (status = 200, description = "Required services ready", body = Ready),
        (status = 503, description = "Required services unavailable", body = Ready)
    )
)]
pub async fn ready(
    State(database): State<Option<Arc<IdentityDatabase>>>,
) -> (StatusCode, Json<Ready>) {
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
            Ok(Readiness::Missing) => "identity-schema-required",
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
