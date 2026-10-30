//! Inert HTTP contract: generation never constructs services or connects dependencies.
use crate::{app, auth::SESSION_COOKIE};
use utoipa::OpenApi as _;

#[derive(utoipa::OpenApi)]
#[openapi(components(schemas(
    crate::terminals::TerminalClientMessage,
    crate::terminals::TerminalServerMessage
)))]
struct TerminalProtocol;
use utoipa::openapi::{
    Info, OpenApi,
    security::{ApiKey, ApiKeyValue, SecurityScheme},
};

pub fn document() -> OpenApi {
    let (_, mut document) = app::documented_router(None, None, None).split_for_parts();
    document.merge(TerminalProtocol::openapi());
    document.info = Info::builder()
        .title("Factory API")
        .version(env!("CARGO_PKG_VERSION"))
        .description(Some(
            "Local private identity API. Generated without runtime configuration.",
        ))
        .build();
    document
        .components
        .get_or_insert_default()
        .add_security_scheme(
            "sessionCookie",
            SecurityScheme::ApiKey(ApiKey::Cookie(ApiKeyValue::new(SESSION_COOKIE))),
        );
    document
}
