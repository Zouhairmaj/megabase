// Ported from supabase/auth internal/api/api.go, internal/api/middleware.go
// (`requireScimServerEnabled`), internal/api/scim/server.go and
// internal/api/scim/{core,protocol} (MIT), pin v2.197.0.

//! SCIM 2.0 discovery endpoints under `/auth/v1/scim/v2`.
//!
//! GoTrue mounts the SCIM sub-router behind `requireScimServerEnabled`, so
//! every method on every path under `/scim/v2` is 404 `feature_disabled`
//! while `GOTRUE_EXPERIMENTAL_SCIM_ENABLED` is off (the reference stack).
//! With it on, `ServiceProviderConfig` describes the server, `ResourceTypes`
//! and `Schemas` list nothing, and any other path is a SCIM 404. Spec:
//! `specs/auth/scim.md`.

use axum::extract::{RawQuery, State};
use axum::http::{header::ALLOW, header::CONTENT_TYPE, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use axum::Router;
use serde::Serialize;

use crate::http::AuthError;
use crate::routes::query_param;
use crate::state::AuthState;
use crate::well_known::go_marshal;

const BASE_PATH: &str = "/auth/v1/scim/v2";
const MEDIA_TYPE: &str = "application/scim+json";
const SCHEMA_SERVICE_PROVIDER_CONFIG: &str =
    "urn:ietf:params:scim:schemas:core:2.0:ServiceProviderConfig";
const SCHEMA_LIST_RESPONSE: &str = "urn:ietf:params:scim:api:messages:2.0:ListResponse";
const SCHEMA_ERROR: &str = "urn:ietf:params:scim:api:messages:2.0:Error";

pub(crate) fn router(state: AuthState) -> Router {
    Router::new()
        .route(
            "/auth/v1/scim/v2/ServiceProviderConfig",
            any(service_provider_config),
        )
        .route("/auth/v1/scim/v2/ResourceTypes", any(resource_types))
        .route("/auth/v1/scim/v2/Schemas", any(schemas))
        .with_state(state)
}

/// The gate, or the SCIM 404, for paths under the SCIM base that no route
/// serves. `None` for every other path.
pub(crate) fn fallback_gate(scim_enabled: bool, path: &str) -> Option<Response> {
    let under = path == BASE_PATH || path.starts_with("/auth/v1/scim/v2/");
    if !under {
        return None;
    }
    Some(if scim_enabled {
        scim_error(StatusCode::NOT_FOUND, "Endpoint or resource does not exist")
    } else {
        disabled()
    })
}

fn disabled() -> Response {
    AuthError::feature_disabled("SCIM server is disabled").into_response()
}

/// `requireScimServerEnabled`, then chi's method handling (GET only).
fn gate(state: &AuthState, method: &Method) -> Option<Response> {
    if !state.scim_enabled {
        return Some(disabled());
    }
    if method != Method::GET {
        return Some((StatusCode::METHOD_NOT_ALLOWED, [(ALLOW, "GET")]).into_response());
    }
    None
}

fn scim_json(status: StatusCode, value: &impl Serialize) -> Response {
    let Ok(bytes) = go_marshal(value) else {
        return AuthError::internal("Error encoding json response").into_response();
    };
    (
        status,
        [(CONTENT_TYPE, HeaderValue::from_static(MEDIA_TYPE))],
        bytes,
    )
        .into_response()
}

#[derive(Serialize)]
struct ScimError {
    schemas: [&'static str; 1],
    detail: &'static str,
    status: String,
}

fn scim_error(status: StatusCode, detail: &'static str) -> Response {
    scim_json(
        status,
        &ScimError {
            schemas: [SCHEMA_ERROR],
            detail,
            status: status.as_u16().to_string(),
        },
    )
}

#[derive(Serialize)]
struct Supported {
    supported: bool,
}

#[derive(Serialize)]
struct Bulk {
    supported: bool,
    #[serde(rename = "maxOperations")]
    max_operations: u32,
    #[serde(rename = "maxPayloadSize")]
    max_payload_size: u32,
}

#[derive(Serialize)]
struct Filter {
    supported: bool,
    #[serde(rename = "maxResults")]
    max_results: u32,
}

#[derive(Serialize)]
struct AuthenticationScheme {
    #[serde(rename = "type")]
    kind: &'static str,
    name: &'static str,
    description: &'static str,
    #[serde(rename = "specUri")]
    spec_uri: &'static str,
    primary: bool,
}

#[derive(Serialize)]
struct Meta {
    #[serde(rename = "resourceType")]
    resource_type: &'static str,
    location: String,
}

#[derive(Serialize)]
struct ServiceProviderConfig {
    schemas: [&'static str; 1],
    patch: Supported,
    bulk: Bulk,
    filter: Filter,
    #[serde(rename = "changePassword")]
    change_password: Supported,
    sort: Supported,
    etag: Supported,
    #[serde(rename = "authenticationSchemes")]
    authentication_schemes: [AuthenticationScheme; 1],
    meta: Meta,
}

// megabase:unit auth:route:GET /auth/v1/scim/v2/ServiceProviderConfig
async fn service_provider_config(State(state): State<AuthState>, method: Method) -> Response {
    if let Some(response) = gate(&state, &method) {
        return response;
    }
    let base = format!("{}/scim/v2", state.api_external_url.trim_end_matches('/'));
    scim_json(
        StatusCode::OK,
        &ServiceProviderConfig {
            schemas: [SCHEMA_SERVICE_PROVIDER_CONFIG],
            patch: Supported { supported: false },
            bulk: Bulk {
                supported: false,
                max_operations: 0,
                max_payload_size: 0,
            },
            filter: Filter {
                supported: false,
                max_results: 0,
            },
            change_password: Supported { supported: false },
            sort: Supported { supported: false },
            etag: Supported { supported: false },
            authentication_schemes: [AuthenticationScheme {
                kind: "oauthbearertoken",
                name: "OAuth Bearer Token",
                description: "Authentication scheme using the OAuth Bearer Token Standard",
                spec_uri: "http://www.rfc-editor.org/info/rfc6750",
                primary: true,
            }],
            meta: Meta {
                resource_type: "ServiceProviderConfig",
                location: format!("{base}/ServiceProviderConfig"),
            },
        },
    )
}

#[derive(Serialize)]
struct EmptyList {
    schemas: [&'static str; 1],
    #[serde(rename = "totalResults")]
    total_results: u32,
    #[serde(rename = "startIndex")]
    start_index: u32,
    #[serde(rename = "itemsPerPage")]
    items_per_page: u32,
    #[serde(rename = "Resources")]
    resources: [(); 0],
}

/// `list` in server.go with no resources.
fn empty_list(query: Option<&str>) -> Response {
    if query_param(query, "filter").is_some() {
        return scim_error(
            StatusCode::FORBIDDEN,
            "Filtering is not supported on this endpoint",
        );
    }
    scim_json(
        StatusCode::OK,
        &EmptyList {
            schemas: [SCHEMA_LIST_RESPONSE],
            total_results: 0,
            start_index: 1,
            items_per_page: 0,
            resources: [],
        },
    )
}

// megabase:unit auth:route:GET /auth/v1/scim/v2/ResourceTypes
async fn resource_types(
    State(state): State<AuthState>,
    method: Method,
    RawQuery(query): RawQuery,
) -> Response {
    gate(&state, &method).unwrap_or_else(|| empty_list(query.as_deref()))
}

// megabase:unit auth:route:GET /auth/v1/scim/v2/Schemas
async fn schemas(
    State(state): State<AuthState>,
    method: Method,
    RawQuery(query): RawQuery,
) -> Response {
    gate(&state, &method).unwrap_or_else(|| empty_list(query.as_deref()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use serde_json::{json, Value};
    use tower::ServiceExt;

    fn enabled() -> AuthState {
        let mut state = AuthState::reference();
        state.scim_enabled = true;
        state.api_external_url = "http://localhost:9999/".into();
        state
    }

    /// Runs the request through the full auth router so the fallback applies.
    async fn call(
        state: AuthState,
        method: &str,
        uri: &str,
    ) -> (StatusCode, HeaderMapOwned, String) {
        let request = Request::builder()
            .method(method)
            .uri(uri)
            .body(Body::empty())
            .unwrap();
        let response = crate::router_with_state(state)
            .oneshot(request)
            .await
            .unwrap();
        let (parts, body) = response.into_parts();
        let text = String::from_utf8(to_bytes(body, usize::MAX).await.unwrap().to_vec()).unwrap();
        (parts.status, parts.headers, text)
    }

    type HeaderMapOwned = axum::http::HeaderMap;

    #[tokio::test]
    async fn disabled_is_feature_disabled_for_every_method_and_path() {
        for (method, path) in [
            ("GET", "/auth/v1/scim/v2/ServiceProviderConfig"),
            ("POST", "/auth/v1/scim/v2/Schemas"),
            ("GET", "/auth/v1/scim/v2/Users"),
        ] {
            let (status, _, text) = call(AuthState::reference(), method, path).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{method} {path}");
            let body: Value = serde_json::from_str(&text).unwrap();
            assert_eq!(body["error_code"], "feature_disabled");
            assert_eq!(body["msg"], "SCIM server is disabled");
        }
    }

    #[tokio::test]
    async fn service_provider_config_matches_upstream_fixture() {
        let (status, headers, text) =
            call(enabled(), "GET", "/auth/v1/scim/v2/ServiceProviderConfig").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(headers[CONTENT_TYPE], MEDIA_TYPE);
        let expected = json!({
            "schemas": ["urn:ietf:params:scim:schemas:core:2.0:ServiceProviderConfig"],
            "patch": {"supported": false},
            "bulk": {"supported": false, "maxOperations": 0, "maxPayloadSize": 0},
            "filter": {"supported": false, "maxResults": 0},
            "changePassword": {"supported": false},
            "sort": {"supported": false},
            "etag": {"supported": false},
            "authenticationSchemes": [{
                "type": "oauthbearertoken",
                "name": "OAuth Bearer Token",
                "description": "Authentication scheme using the OAuth Bearer Token Standard",
                "specUri": "http://www.rfc-editor.org/info/rfc6750",
                "primary": true
            }],
            "meta": {
                "resourceType": "ServiceProviderConfig",
                "location": "http://localhost:9999/scim/v2/ServiceProviderConfig"
            }
        });
        assert_eq!(serde_json::from_str::<Value>(&text).unwrap(), expected);
    }

    #[tokio::test]
    async fn lists_are_empty_and_filter_is_forbidden() {
        for path in ["ResourceTypes", "Schemas"] {
            let uri = format!("/auth/v1/scim/v2/{path}");
            let (status, _, text) = call(enabled(), "GET", &uri).await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(
                text,
                r#"{"schemas":["urn:ietf:params:scim:api:messages:2.0:ListResponse"],"totalResults":0,"startIndex":1,"itemsPerPage":0,"Resources":[]}"#
            );
            let (status, _, text) = call(enabled(), "GET", &format!("{uri}?filter=x")).await;
            assert_eq!(status, StatusCode::FORBIDDEN);
            assert_eq!(
                text,
                r#"{"schemas":["urn:ietf:params:scim:api:messages:2.0:Error"],"detail":"Filtering is not supported on this endpoint","status":"403"}"#
            );
        }
    }

    #[tokio::test]
    async fn unknown_path_and_wrong_method_when_enabled() {
        let (status, headers, text) = call(enabled(), "GET", "/auth/v1/scim/v2/Users").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(headers[CONTENT_TYPE], MEDIA_TYPE);
        assert_eq!(
            text,
            r#"{"schemas":["urn:ietf:params:scim:api:messages:2.0:Error"],"detail":"Endpoint or resource does not exist","status":"404"}"#
        );
        let (status, headers, _) = call(enabled(), "POST", "/auth/v1/scim/v2/Schemas").await;
        assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(headers[ALLOW], "GET");
    }

    #[test]
    fn other_paths_are_not_gated() {
        assert!(fallback_gate(false, "/auth/v1/scimx").is_none());
        assert!(fallback_gate(false, "/auth/v1/health").is_none());
    }
}
