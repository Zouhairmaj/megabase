//! Realtime's JWT check for socket connects and channel joins.

// Ported from supabase/realtime lib/realtime_web/channels/auth/jwt_verification.ex and
// lib/realtime_web/channels/auth/channels_authorization.ex (Apache-2.0), pin v2.143.3.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use megabase_core::{Hs256, JwtError};
use serde_json::{Map, Value};

/// Why a token was rejected, named after the upstream error atoms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TokenError {
    /// `:token_malformed`: not three segments, or claims that are not JSON.
    Malformed,
    /// `:signature_error`.
    Signature,
    /// `:error_generating_signer`: an `alg` with no signer for the secret.
    Signer,
    /// `:expired_token`, with how many seconds ago `exp` passed.
    Expired(i64),
    /// `:missing_claims`: `role` or `exp` absent.
    MissingClaims,
    /// `HS384`/`HS512`: upstream verifies them, Megabase does not yet.
    UnsupportedAlgorithm,
    /// Every other failure (`:expected_claims_map`, `:expected_header_map`,
    /// a failed `exp`/`iat` validator on a non-integer, a bad URI escape).
    Other,
}

/// Verified claims, with `exp` and `iat` rounded as upstream does.
pub(crate) type Claims = Map<String, Value>;

/// `ChannelsAuthorization.authorize_conn/3` with the tenant secret.
pub(crate) fn authorize_conn(jwt: &Hs256, token: &str, now: i64) -> Result<Claims, TokenError> {
    let token = clean_token(token).ok_or(TokenError::Other)?;
    let claims = verify(jwt, &token, now)?;
    if claims.contains_key("role") && claims.contains_key("exp") {
        Ok(claims)
    } else {
        Err(TokenError::MissingClaims)
    }
}

/// `clean_token/1`: `URI.decode/1`, then drop whitespace. `None` where
/// `URI.decode/1` raises on a bad escape.
fn clean_token(token: &str) -> Option<String> {
    let bytes = token.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = bytes.get(i + 1..i + 3)?;
            let hex = std::str::from_utf8(hex).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    let decoded = String::from_utf8(out).ok()?;
    // Erlang `\s` without the unicode flag.
    Some(
        decoded
            .chars()
            .filter(|c| !matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0B' | '\x0C'))
            .collect(),
    )
}

fn decode_json(segment: &str) -> Option<Value> {
    let bytes = URL_SAFE_NO_PAD.decode(segment.trim_end_matches('=')).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// `JwtVerification.verify/3` with a secret and no JWKS.
fn verify(jwt: &Hs256, token: &str, now: i64) -> Result<Claims, TokenError> {
    // `Joken.peek_claims/1`, then `Joken.peek_header/1`.
    let segments: Vec<&str> = token.split('.').collect();
    let [header, payload, _signature] = segments[..] else {
        return Err(TokenError::Malformed);
    };
    let Some(payload) = decode_json(payload) else {
        return Err(TokenError::Malformed);
    };
    let Value::Object(mut claims) = payload else {
        return Err(TokenError::Other);
    };
    let Some(Value::Object(header)) = decode_json(header) else {
        return Err(TokenError::Other);
    };
    match header.get("alg").and_then(Value::as_str) {
        Some("HS256") => {}
        Some("HS384" | "HS512") => return Err(TokenError::UnsupportedAlgorithm),
        _ => return Err(TokenError::Signer),
    }
    // Signature only: `exp` is validated below with Joken's rule, not
    // megabase-core's leeway, so the core check is pushed out of reach.
    match jwt.verify_at(token, i64::MIN) {
        Ok(_) => {}
        Err(JwtError::KeyError) => return Err(TokenError::Signature),
        Err(JwtError::MalformedHeader | JwtError::MalformedPayload) => {
            return Err(TokenError::Malformed)
        }
        Err(_) => return Err(TokenError::Other),
    }
    // Joken validates `exp` then `iat`, each only when present.
    match claims.get("exp") {
        None => {}
        Some(Value::Number(exp)) => {
            let valid = exp.as_f64().is_some_and(|exp| exp > now as f64);
            if !valid {
                return Err(match exp.as_i64() {
                    Some(exp) => TokenError::Expired(now.saturating_sub(exp)),
                    None => TokenError::Other,
                });
            }
        }
        Some(_) => return Err(TokenError::Other),
    }
    if claims.get("iat").is_some_and(|iat| !iat.is_number()) {
        return Err(TokenError::Other);
    }
    for key in ["exp", "iat"] {
        if let Some(rounded) = claims
            .get(key)
            .and_then(Value::as_f64)
            .filter(|_| claims.get(key).and_then(Value::as_i64).is_none())
            .map(f64::round)
        {
            claims.insert(key.to_owned(), Value::from(rounded as i64));
        }
    }
    Ok(claims)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const SECRET: &str = "super-secret-jwt-token-with-at-least-32-characters-long";
    const NOW: i64 = 1_700_000_000;

    fn jwt() -> Hs256 {
        Hs256::new(SECRET).unwrap()
    }

    fn sign(claims: &Value) -> String {
        jwt().sign(claims).unwrap().as_str().to_owned()
    }

    #[test]
    fn accepts_role_and_future_exp() {
        let token = sign(&json!({"role": "anon", "exp": NOW + 60}));
        let claims = authorize_conn(&jwt(), &token, NOW).unwrap();
        assert_eq!(claims["role"], "anon");
    }

    #[test]
    fn rejects_expired_with_seconds_ago() {
        let token = sign(&json!({"role": "anon", "exp": NOW - 5}));
        assert_eq!(
            authorize_conn(&jwt(), &token, NOW),
            Err(TokenError::Expired(5))
        );
        let token = sign(&json!({"role": "anon", "exp": NOW}));
        assert_eq!(
            authorize_conn(&jwt(), &token, NOW),
            Err(TokenError::Expired(0))
        );
    }

    #[test]
    fn requires_role_and_exp() {
        let token = sign(&json!({"exp": NOW + 60}));
        assert_eq!(
            authorize_conn(&jwt(), &token, NOW),
            Err(TokenError::MissingClaims)
        );
        let token = sign(&json!({"role": "anon"}));
        assert_eq!(
            authorize_conn(&jwt(), &token, NOW),
            Err(TokenError::MissingClaims)
        );
    }

    #[test]
    fn rejects_malformed_and_bad_signature() {
        assert_eq!(
            authorize_conn(&jwt(), "not-a-jwt", NOW),
            Err(TokenError::Malformed)
        );
        assert_eq!(
            authorize_conn(&jwt(), "a.b.c", NOW),
            Err(TokenError::Malformed)
        );
        let other = Hs256::new("another-secret-that-is-at-least-32-bytes!!")
            .unwrap()
            .sign(&json!({"role": "anon", "exp": NOW + 60}))
            .unwrap();
        assert_eq!(
            authorize_conn(&jwt(), other.as_str(), NOW),
            Err(TokenError::Signature)
        );
    }

    #[test]
    fn maps_algorithms() {
        let claims = URL_SAFE_NO_PAD.encode(br#"{"role":"anon","exp":1800000000}"#);
        for (alg, expected) in [
            ("none", TokenError::Signer),
            ("RS256", TokenError::Signer),
            ("HS512", TokenError::UnsupportedAlgorithm),
        ] {
            let header = URL_SAFE_NO_PAD.encode(format!(r#"{{"alg":"{alg}"}}"#));
            let token = format!("{header}.{claims}.sig");
            assert_eq!(authorize_conn(&jwt(), &token, NOW), Err(expected));
        }
    }

    #[test]
    fn non_object_claims_and_bad_escapes_are_other() {
        let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"HS256"}"#);
        let claims = URL_SAFE_NO_PAD.encode(b"[1]");
        let token = format!("{header}.{claims}.sig");
        assert_eq!(authorize_conn(&jwt(), &token, NOW), Err(TokenError::Other));
        assert_eq!(authorize_conn(&jwt(), "%zz", NOW), Err(TokenError::Other));
        let token = sign(&json!({"role": "anon", "exp": "soon"}));
        assert_eq!(authorize_conn(&jwt(), &token, NOW), Err(TokenError::Other));
    }

    #[test]
    fn cleans_escapes_and_whitespace() {
        let token = sign(&json!({"role": "anon", "exp": NOW + 60}));
        let (head, tail) = token.split_at(10);
        let dirty = format!(" {head}%0A{tail}\n");
        assert!(authorize_conn(&jwt(), &dirty, NOW).is_ok());
    }

    #[test]
    fn rounds_float_exp() {
        let token = sign(&json!({"role": "anon", "exp": (NOW + 60) as f64 + 0.6}));
        let claims = authorize_conn(&jwt(), &token, NOW).unwrap();
        assert_eq!(claims["exp"], json!(NOW + 61));
    }
}
