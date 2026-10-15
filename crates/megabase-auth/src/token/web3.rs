// Ported from supabase/auth internal/api/web3.go (`Web3Grant`) (MIT),
// pin v2.197.0.

//! `POST /auth/v1/token?grant_type=web3`: Sign in with Solana or Ethereum.
//!
//! The provider switches, the chain dispatch, and the message and signature
//! length checks are ported. Message parsing
//! and signature verification (SIWS ed25519, SIWE secp256k1) are not, so a
//! request for an enabled chain is HTTP 501. The reference stack enables
//! neither chain.

use axum::body::Body;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use megabase_core::MegabaseNotImplemented;
use serde::Deserialize;

use super::id_token::raw_url_decode;
use super::{grant_json, null_as_default};
use crate::error::{validation, GoTrueError};
use crate::gojson::WEB3_GRANT_PARAMS;
use crate::state::AuthState;
use crate::COMPONENT;

const UNIT: &str = "auth:grant-type:web3";

#[derive(Deserialize)]
struct Web3Body {
    #[serde(default, deserialize_with = "null_as_default")]
    message: String,
    #[serde(default, deserialize_with = "null_as_default")]
    signature: String,
    #[serde(default, deserialize_with = "null_as_default")]
    chain: String,
}

// Not counted as served yet: an enabled chain is still HTTP 501.
pub(super) async fn grant(state: AuthState, body: Body) -> Response {
    let config = &state.config;
    if !config.web3_solana && !config.web3_ethereum {
        return disabled("Web3 provider is disabled");
    }
    let params = match grant_json::<Web3Body>(body, &WEB3_GRANT_PARAMS).await {
        Ok(params) => params,
        Err(error) => return error.into_response(),
    };
    let enabled = match params.chain.as_str() {
        "solana" => config.web3_solana,
        "ethereum" => config.web3_ethereum,
        _ => {
            return GoTrueError::new(
                StatusCode::BAD_REQUEST,
                "web3_unsupported_chain",
                "Unsupported chain",
            )
            .into_response();
        }
    };
    if !enabled {
        return if params.chain == "solana" {
            disabled("Solana Web3 provider is disabled")
        } else {
            disabled("Ethereum Web3 provider is disabled")
        };
    }
    if let Err(message) = check_fields(&params) {
        return validation(message).into_response();
    }
    // SIWS / SIWE message parsing and signature verification.
    MegabaseNotImplemented::new(COMPONENT, UNIT).into_response()
}

/// The length and encoding checks `web3GrantSolana` and
/// `web3GrantEthereum` run before parsing the message. Lengths are bytes,
/// as Go `len`.
fn check_fields(params: &Web3Body) -> Result<(), &'static str> {
    if params.message.len() < 64 {
        return Err("message is too short");
    }
    if params.message.len() > 20 * 1024 {
        return Err("message must not exceed 20KB");
    }
    if params.chain == "solana" {
        if params.signature.len() != 86 && params.signature.len() != 88 {
            return Err("signature must be 64 bytes encoded as base64 with or without padding");
        }
        let url_safe = params
            .signature
            .trim_end_matches('=')
            .replace('+', "-")
            .replace('/', "_");
        if raw_url_decode(&url_safe).is_none() {
            return Err("signature does not contain valid base64 characters");
        }
    } else {
        let hex = params
            .signature
            .strip_prefix("0x")
            .unwrap_or(&params.signature);
        if hex.len() != 130 {
            return Err("signature must be 65 bytes long, encoded as a 130 character-long hexadecimal string");
        }
    }
    Ok(())
}

fn disabled(message: &'static str) -> Response {
    GoTrueError::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        "web3_provider_disabled",
        message,
    )
    .into_response()
}

#[cfg(test)]
mod tests {
    use axum::http::Request;
    use serde_json::Value;
    use tower::ServiceExt;

    use super::*;
    use crate::config::AuthConfig;
    use crate::routes::router;
    use crate::store::Backend;

    async fn post(config: AuthConfig, body: &str) -> (StatusCode, Value) {
        let state = AuthState::new(config, None, Backend::memory());
        let request = Request::builder()
            .method("POST")
            .uri("/auth/v1/token?grant_type=web3")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let response = router(state).oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    #[tokio::test]
    async fn disabled_before_the_body_is_read() {
        let (status, body) = post(AuthConfig::reference_defaults(), "not-json").await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body["error_code"], "web3_provider_disabled");
        assert_eq!(body["msg"], "Web3 provider is disabled");
    }

    #[tokio::test]
    async fn chain_dispatch() {
        let mut solana = AuthConfig::reference_defaults();
        solana.web3_solana = true;
        let (status, body) = post(solana.clone(), r#"{"chain":"bitcoin"}"#).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error_code"], "web3_unsupported_chain");
        assert_eq!(body["msg"], "Unsupported chain");

        let (status, body) = post(solana.clone(), r#"{"chain":"ethereum"}"#).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body["msg"], "Ethereum Web3 provider is disabled");

        let (status, body) = post(solana.clone(), r#"{"chain":7}"#).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(
            body["msg"],
            "Could not parse request body as JSON: json: cannot unmarshal number into Go struct field Web3GrantParams.chain of type string"
        );

        let (status, body) = post(solana, r#"{"chain":"solana"}"#).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error_code"], "validation_failed");
        assert_eq!(body["msg"], "message is too short");

        let mut ethereum = AuthConfig::reference_defaults();
        ethereum.web3_ethereum = true;
        let (_, body) = post(ethereum, r#"{"chain":"solana"}"#).await;
        assert_eq!(body["msg"], "Solana Web3 provider is disabled");
    }

    async fn fields(chain: &str, message: &str, signature: &str) -> (StatusCode, Value) {
        let mut config = AuthConfig::reference_defaults();
        config.web3_solana = true;
        config.web3_ethereum = true;
        let body = serde_json::json!({
            "chain": chain,
            "message": message,
            "signature": signature,
        });
        post(config, &body.to_string()).await
    }

    #[tokio::test]
    async fn field_checks_before_verification() {
        let message = "m".repeat(64);
        let (status, body) = fields("ethereum", &"m".repeat(63), "").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error_code"], "validation_failed");
        assert_eq!(body["msg"], "message is too short");

        let (_, body) = fields("solana", &"m".repeat(20 * 1024 + 1), "").await;
        assert_eq!(body["msg"], "message must not exceed 20KB");

        let (_, body) = fields("solana", &message, "short").await;
        assert_eq!(
            body["msg"],
            "signature must be 64 bytes encoded as base64 with or without padding"
        );
        let (_, body) = fields("solana", &message, &"!".repeat(86)).await;
        assert_eq!(
            body["msg"],
            "signature does not contain valid base64 characters"
        );
        let padded = format!("{}==", "A+/".repeat(28) + "AA");
        let (status, body) = fields("solana", &message, &padded).await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED, "{body}");
        assert_eq!(body["unit"], UNIT);

        let (_, body) = fields("ethereum", &message, "0xabc").await;
        assert_eq!(
            body["msg"],
            "signature must be 65 bytes long, encoded as a 130 character-long hexadecimal string"
        );
        let hex = format!("0x{}", "a".repeat(130));
        let (status, body) = fields("ethereum", &message, &hex).await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED, "{body}");
        assert_eq!(body["unit"], UNIT);
    }
}
