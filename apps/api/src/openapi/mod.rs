//! Inert HTTP contract: generation never constructs a runtime or connects services.
use crate::contracts::*;
use serde_json::{Value, json};
use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(components(schemas(
    UserRole,
    UserStatus,
    SessionUser,
    SessionResponse,
    AdminUser,
    UsersResponse,
    AccessAction,
    AccessBody,
    ApiError
)))]
struct Schemas;

fn response(schema: &str, description: &str) -> Value {
    json!({"description": description,"content":{"application/json":{"schema":{"$ref":format!("#/components/schemas/{schema}")}}}})
}

fn protected_responses(success: Value) -> Value {
    let mut responses = json!({"200":success});
    for (status, meaning) in [
        ("400", "Invalid input"),
        ("401", "Session required"),
        ("403", "Access denied"),
        ("404", "User not found"),
        ("409", "Conflict or protected administrator"),
        ("429", "Rate limit"),
        ("503", "Dependency unavailable"),
    ] {
        responses[status] = response("ApiError", meaning);
    }
    responses
}

pub fn document() -> Value {
    let mut document =
        serde_json::to_value(Schemas::openapi()).expect("OpenAPI schema is serializable");
    document["info"] = json!({"title":"Factory API","version":"0.0.0","description":"Local private identity API. Generated without runtime configuration."});
    document["components"]["securitySchemes"] =
        json!({"sessionCookie":{"type":"apiKey","in":"cookie","name":"factory_session"}});
    let secured = json!([{"sessionCookie":[]}]);
    let csrf = json!({"name":"X-CSRF-Token","in":"header","required":true,"schema":{"type":"string"},"description":"Session-bound CSRF value; exact frontend Origin also required."});
    document["paths"] = json!({
        "/health/live":{"get":{"operationId":"healthLive","responses":{"200":{"description":"Process is alive","content":{"application/json":{"schema":{"type":"object","required":["status"],"properties":{"status":{"type":"string","const":"ok"}}}}}}}}},
        "/health/ready":{"get":{"operationId":"healthReady","responses":{"200":{"description":"Required services ready"},"503":{"description":"Required services unavailable"}}}},
        "/api/auth/github/start":{"get":{"operationId":"startGithubLogin","description":"An explicit Accept: text/html with nonzero quality returns start failures to the fixed frontend sign-in page. Other clients retain JSON errors and HTTP status. Retry-After is preserved for rate limits.","responses":{"302":{"description":"Redirect to GitHub with state and PKCE, or browser recovery at the fixed frontend sign-in page"},"429":response("ApiError","Rate limit"),"503":response("ApiError","Dependency unavailable")}}},
        "/api/auth/github/callback":{"get":{"operationId":"completeGithubLogin","parameters":[{"name":"state","in":"query","schema":{"type":"string"}},{"name":"code","in":"query","schema":{"type":"string"}},{"name":"error","in":"query","schema":{"type":"string"}}],"responses":{"302":{"description":"Fixed frontend redirect; issues session only for admitted user"}}}},
        "/api/session":{"get":{"operationId":"getSession","security":secured,"responses":protected_responses(response("SessionResponse","Current active session"))}},
        "/api/auth/logout":{"post":{"operationId":"logout","security":secured,"parameters":[csrf],"responses":{"204":{"description":"Session revoked and cookie cleared"},"401":response("ApiError","Session required"),"403":response("ApiError","Origin or CSRF denied"),"503":response("ApiError","Dependency unavailable")}}},
        "/api/admin/users":{"get":{"operationId":"listUsers","security":secured,"parameters":[{"name":"status","in":"query","required":true,"schema":{"$ref":"#/components/schemas/UserStatus"}},{"name":"cursor","in":"query","schema":{"type":"string","format":"uuid"}},{"name":"limit","in":"query","schema":{"type":"integer","minimum":1,"maximum":50,"default":20}}],"responses":protected_responses(response("UsersResponse","Users in stable UUID order"))}},
        "/api/admin/users/{id}/access":{"patch":{"operationId":"changeAccess","security":secured,"parameters":[{"name":"id","in":"path","required":true,"schema":{"type":"string","format":"uuid"}},csrf],"requestBody":{"required":true,"content":{"application/json":{"schema":{"$ref":"#/components/schemas/AccessBody"}}}},"responses":protected_responses(response("AdminUser","Updated member access"))}}
    });
    document
}
