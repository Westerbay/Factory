//! Inert HTTP contract: generation never constructs services or connects dependencies.
use crate::{app, auth::SESSION_COOKIE};
use utoipa::openapi::{
    Info, OpenApi,
    security::{ApiKey, ApiKeyValue, SecurityScheme},
};

pub fn document() -> OpenApi {
    let (_, mut document) = app::documented_router(None, None).split_for_parts();
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
