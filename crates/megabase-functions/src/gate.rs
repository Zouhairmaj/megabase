// Ported from supabase/supabase docker/volumes/functions/main/index.ts (Apache-2.0), pin v1.26.08.

//! The `VERIFY_JWT` gate the self-hosted `main` service runs before it
//! starts a user worker.
//!
//! Upstream verifies with `jose` (`jwtVerify`, no clock tolerance): HS256
//! tokens against `JWT_SECRET`, ES256 / RS256 tokens against
//! `SUPABASE_JWKS`. The pinned compose file leaves `SUPABASE_JWKS` unset, so
//! asymmetric tokens are always rejected there, and here.

use axum::http::{header, HeaderMap};
use base64::alphabet::URL_SAFE;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};
use base64::Engine;
use megabase_core::Hs256;
use serde_json::{Map, Value};

/// jose `decodeProtectedHeader` decodes base64url with or without padding
/// and ignores non-zero trailing bits, like the forgiving WHATWG decoder.
const FORGIVING_B64URL: GeneralPurpose = GeneralPurpose::new(
    &URL_SAFE,
    GeneralPurposeConfig::new()
        .with_decode_padding_mode(DecodePaddingMode::Indifferent)
        .with_decode_allow_trailing_bits(true),
);

/// Why the gate rejected a request. `Display` is the `msg` upstream returns
/// with status 401 (`e.toString()` for thrown errors).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GateError {
    /// `getAuthToken`: no (or an empty) `Authorization` header.
    #[error("Error: Missing authorization header")]
    MissingHeader,
    /// `getAuthToken`: the first space-separated word is not `Bearer`.
    #[error("Error: Auth header is not 'Bearer {{token}}'")]
    NotBearer,
    /// jose `decodeProtectedHeader` could not read the token's header.
    #[error("TypeError: Invalid Token or Protected Header formatting")]
    MalformedHeader,
    /// `isValidHybridJWT` returned false.
    #[error("Invalid JWT")]
    Invalid,
}

/// Run the upstream gate on `headers` as of `now_unix`.
///
/// `secret` is `JWT_SECRET`; without it every HS256 token is invalid, as
/// upstream `isValidLegacyJWT` returns false when the variable is unset.
///
/// # Errors
///
/// The [`GateError`] whose message upstream sends back with status 401.
pub fn check(headers: &HeaderMap, secret: Option<&Hs256>, now_unix: i64) -> Result<(), GateError> {
    let token = auth_token(headers)?;
    let protected = decode_protected_header(token)?;
    let valid = match protected.get("alg").and_then(Value::as_str) {
        Some("HS256") => secret.is_some_and(|key| legacy_jwt_is_valid(key, token, now_unix)),
        // `SUPABASE_JWKS` is not configured: `isValidJWT` returns false.
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(GateError::Invalid)
    }
}

/// `getAuthToken`: `authHeader.split(' ')` destructured as `[bearer, token]`.
/// A missing second word is `undefined`, which jose then rejects.
fn auth_token(headers: &HeaderMap) -> Result<&str, GateError> {
    let value = headers
        .get(header::AUTHORIZATION)
        .map(|value| value.to_str().unwrap_or_default())
        .unwrap_or_default();
    if value.is_empty() {
        return Err(GateError::MissingHeader);
    }
    let mut words = value.split(' ');
    if words.next() != Some("Bearer") {
        return Err(GateError::NotBearer);
    }
    words.next().ok_or(GateError::MalformedHeader)
}

/// jose `decodeProtectedHeader` for a compact string: the first of 3 (JWS)
/// or 5 (JWE) dot-separated parts must decode to a JSON object.
fn decode_protected_header(token: &str) -> Result<Map<String, Value>, GateError> {
    let parts: Vec<&str> = token.split('.').collect();
    let first = match parts.len() {
        3 | 5 => parts[0],
        _ => return Err(GateError::MalformedHeader),
    };
    if first.is_empty() {
        return Err(GateError::MalformedHeader);
    }
    let bytes = FORGIVING_B64URL
        .decode(first)
        .map_err(|_| GateError::MalformedHeader)?;
    let text = String::from_utf8_lossy(&bytes);
    match serde_json::from_str::<Value>(&text) {
        Ok(Value::Object(map)) => Ok(map),
        _ => Err(GateError::MalformedHeader),
    }
}

/// `isValidLegacyJWT`: jose `jwtVerify` with the raw secret. Signature and
/// payload checks are `megabase_core`'s; jose's claim checks run with zero
/// clock tolerance, so `exp`, `nbf` and `iat` are rechecked here.
fn legacy_jwt_is_valid(secret: &Hs256, token: &str, now_unix: i64) -> bool {
    let Ok(claims) = secret.verify_at(token, now_unix) else {
        return false;
    };
    if claims.exp.is_some_and(|exp| exp <= now_unix) {
        return false;
    }
    match claims.raw.get("nbf") {
        None => {}
        Some(Value::Number(nbf)) if nbf.as_f64().is_some_and(|nbf| nbf <= now_unix as f64) => {}
        Some(_) => return false,
    }
    !matches!(claims.raw.get("iat"), Some(iat) if !iat.is_number())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;
    use serde_json::json;

    const SECRET: &str = "super-secret-jwt-token-with-at-least-32-characters-long";
    const NOW: i64 = 1_700_000_000;

    fn key() -> Hs256 {
        Hs256::new(SECRET).unwrap()
    }

    fn token(payload: &Value) -> String {
        key().sign(payload).unwrap().as_str().to_string()
    }

    fn headers(authorization: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_str(authorization).unwrap(),
        );
        headers
    }

    fn run(authorization: Option<&str>) -> Result<(), GateError> {
        let headers = authorization.map(headers).unwrap_or_default();
        check(&headers, Some(&key()), NOW)
    }

    #[test]
    fn accepts_a_valid_hs256_token() {
        let jwt = token(&json!({"role": "anon", "exp": NOW + 60}));
        assert_eq!(run(Some(&format!("Bearer {jwt}"))), Ok(()));
    }

    #[test]
    fn missing_or_empty_header() {
        assert_eq!(run(None), Err(GateError::MissingHeader));
        assert_eq!(run(Some("")), Err(GateError::MissingHeader));
        assert_eq!(
            GateError::MissingHeader.to_string(),
            "Error: Missing authorization header"
        );
    }

    #[test]
    fn scheme_must_be_exactly_bearer() {
        for value in ["bearer x", "Basic x", "Bearerx", " Bearer x"] {
            assert_eq!(run(Some(value)), Err(GateError::NotBearer), "{value}");
        }
        assert_eq!(
            GateError::NotBearer.to_string(),
            "Error: Auth header is not 'Bearer {token}'"
        );
    }

    #[test]
    fn unreadable_tokens_fail_header_decoding() {
        for value in [
            "Bearer",
            "Bearer ",
            "Bearer  x.y.z",
            "Bearer abc",
            "Bearer a.b",
            "Bearer .b.c",
            "Bearer !!.b.c",
            "Bearer WzFd.b.c",
        ] {
            assert_eq!(run(Some(value)), Err(GateError::MalformedHeader), "{value}");
        }
        assert_eq!(
            GateError::MalformedHeader.to_string(),
            "TypeError: Invalid Token or Protected Header formatting"
        );
    }

    #[test]
    fn invalid_tokens() {
        let wrong_key = Hs256::new("another-secret-that-is-at-least-32-bytes-long")
            .unwrap()
            .sign(&json!({"role": "anon"}))
            .unwrap();
        let expired = token(&json!({"exp": NOW}));
        let not_yet = token(&json!({"nbf": NOW + 1}));
        let bad_iat = token(&json!({"iat": "yesterday"}));
        // {"alg":"ES256"} and {"alg":"none"} headers.
        let es256 = "eyJhbGciOiJFUzI1NiJ9.e30.c2ln";
        let none = "eyJhbGciOiJub25lIn0.e30.";
        for jwt in [
            wrong_key.as_str(),
            &expired,
            &not_yet,
            &bad_iat,
            es256,
            none,
        ] {
            assert_eq!(
                run(Some(&format!("Bearer {jwt}"))),
                Err(GateError::Invalid),
                "{jwt}"
            );
        }
        assert_eq!(GateError::Invalid.to_string(), "Invalid JWT");
    }

    #[test]
    fn extra_words_after_the_token_are_ignored() {
        let jwt = token(&json!({"role": "anon"}));
        assert_eq!(run(Some(&format!("Bearer {jwt} trailing"))), Ok(()));
    }

    #[test]
    fn without_a_secret_every_hs256_token_is_invalid() {
        let jwt = token(&json!({"role": "anon"}));
        assert_eq!(
            check(&headers(&format!("Bearer {jwt}")), None, NOW),
            Err(GateError::Invalid)
        );
    }

    #[test]
    fn padded_and_five_part_headers_decode() {
        // `{"alg":"HS256"}` with padding, as a 5-part (JWE-shaped) token.
        assert!(decode_protected_header("eyJhbGciOiJIUzI1NiJ9.a.b.c.d").is_ok());
        assert!(decode_protected_header("eyJhIjoxfQ==.b.c").is_ok());
    }
}
