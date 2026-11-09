//! Inert HTTP contract: generation never constructs services or connects dependencies.
use crate::{app, auth::SESSION_COOKIE};
use utoipa::OpenApi as _;

#[derive(utoipa::OpenApi)]
#[openapi(components(schemas(
    crate::terminals::TerminalClientMessage,
    crate::terminals::TerminalServerMessage,
    crate::conversations::contracts::EventView
)))]
struct TerminalProtocol;
use utoipa::openapi::{
    Info, OpenApi,
    security::{ApiKey, ApiKeyValue, SecurityScheme},
};

pub fn document() -> OpenApi {
    let (_, mut document) =
        app::documented_router(None, None, None, None, None, None).split_for_parts();
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

#[cfg(test)]
mod conversation_contract_tests {
    #[test]
    fn conversation_mutations_document_required_security_and_errors() {
        let doc = serde_json::to_value(super::document()).unwrap();
        let paths = doc["paths"].as_object().unwrap();
        let expected = [
            "/api/conversations",
            "/api/conversations/{id}/runs",
            "/api/conversations/{id}/runs/{run}/cancel",
            "/api/conversations/{id}/runs/{run}/questions/{request}",
            "/api/conversations/{id}/runs/{run}/recover",
            "/api/conversations/model-probe/recover",
        ];
        let mut found = Vec::new();
        for (path, value) in paths {
            if !path.starts_with("/api/conversations") {
                continue;
            }
            let Some(operation) = value.get("post") else {
                continue;
            };
            found.push(path.as_str());
            assert!(
                operation["security"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|s| s.get("sessionCookie").is_some()),
                "{path}: cookie security"
            );
            for name in ["Origin", "X-CSRF-Token"] {
                let parameter = operation["parameters"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|p| p["in"] == "header" && p["name"] == name)
                    .unwrap_or_else(|| panic!("{path}: missing {name}"));
                assert_eq!(
                    parameter["required"], true,
                    "{path}: {name} must be required"
                );
            }
            for status in ["400", "401", "403", "409", "429", "503"] {
                assert!(
                    operation["responses"].get(status).is_some(),
                    "{path}: missing {status}"
                );
            }
        }
        let mut expected = expected.to_vec();
        expected.sort_unstable();
        found.sort_unstable();
        assert_eq!(found, expected, "cover every conversation mutation");
    }

    #[test]
    fn event_stream_documents_conditional_origin_and_access_errors() {
        let doc = serde_json::to_value(super::document()).unwrap();
        let operation = &doc["paths"]["/api/conversations/{id}/runs/{run}/events"]["get"];
        let origin = operation["parameters"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["in"] == "header" && p["name"] == "Origin")
            .unwrap();
        assert_eq!(origin["required"], false);
        assert!(
            origin["description"]
                .as_str()
                .unwrap()
                .contains("Sec-Fetch-Site: same-origin")
        );
        for status in ["400", "401", "403", "404", "409", "429", "503"] {
            assert!(
                operation["responses"].get(status).is_some(),
                "missing {status}"
            );
        }
    }
}
