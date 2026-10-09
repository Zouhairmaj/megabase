// Ported from supabase/auth internal/api/auth.go (MIT), pin v2.197.0,
// and PostgREST src/library/PostgREST/Auth/Jwt.hs (MIT), pin v16.4.

//! HS256 JWT verification shared by Auth and REST.
//!
//! Level 1 verifies the symmetric tokens the demo `JWT_SECRET` issues.
//! Asymmetric keys and JWKS rotation are out of scope.

use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use hmac::{Hmac, Mac};
use serde_json::{Map, Value};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// PostgREST `allowedSkewSeconds` in `PostgREST.Auth.Jwt`.
pub const EXP_LEEWAY_SECS: i64 = 30;

const HS256: &str = "HS256";

/// Compact JWT claims Auth and REST both need from a verified token.
#[derive(Debug, Clone, PartialEq)]
pub struct JwtClaims {
    /// Payload `role` when it is a JSON string (`anon`, `authenticated`, `service_role`).
    pub role: Option<String>,
    /// Payload `sub` when it is a JSON string. User access tokens set this;
    /// the demo anon / service_role keys do not.
    pub sub: Option<String>,
    /// Payload `exp` as Unix seconds when it is a JSON number.
    pub exp: Option<i64>,
    /// Full payload object (`auth.jwt()` / `request.jwt.claims`).
    pub raw: Map<String, Value>,
}

/// Why HS256 verification rejected a token. Messages match PostgREST JWT errors
/// (`PGRST301` / `PGRST303`) so REST can reuse them; Auth maps the same kinds
/// to GoTrue `bad_jwt`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum JwtError {
    #[error("Server lacks JWT secret")]
    SecretMissing,
    #[error("Empty JWT is sent in Authorization header")]
    Empty,
    #[error("Expected 3 parts in JWT; got {0}")]
    UnexpectedParts(usize),
    #[error("Wrong or unsupported encoding algorithm")]
    BadAlgorithm(String),
    #[error("JWT cryptographic operation failed")]
    BadCrypto,
    #[error("Parsing claims failed")]
    MalformedHeader,
    #[error("Parsing claims failed")]
    MalformedPayload,
    #[error("The JWT 'exp' claim must be a number")]
    ExpNotNumber,
    #[error("JWT expired")]
    Expired,
}

/// HMAC-SHA256 verifier bound to one `JWT_SECRET`.
#[derive(Clone)]
pub struct Hs256 {
    secret: Vec<u8>,
}

impl fmt::Debug for Hs256 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Hs256")
            .field("secret", &"<redacted>")
            .finish()
    }
}

impl Hs256 {
    /// Rejects an empty secret so callers never fall back to a default key.
    pub fn new(secret: impl Into<Vec<u8>>) -> Result<Self, JwtError> {
        let secret = secret.into();
        if secret.is_empty() {
            return Err(JwtError::SecretMissing);
        }
        Ok(Self { secret })
    }

    pub fn verify(&self, token: &str) -> Result<JwtClaims, JwtError> {
        self.verify_at(token, unix_now())
    }

    pub fn verify_at(&self, token: &str, now_unix: i64) -> Result<JwtClaims, JwtError> {
        if token.is_empty() {
            return Err(JwtError::Empty);
        }

        let parts: Vec<&str> = token.split('.').collect();
        if parts.len() != 3 {
            return Err(JwtError::UnexpectedParts(parts.len()));
        }
        let header_b64 = parts[0];
        let payload_b64 = parts[1];
        let signature_b64 = parts[2];

        let header_json = decode_segment(header_b64).map_err(|_| JwtError::MalformedHeader)?;
        let header: Value =
            serde_json::from_slice(&header_json).map_err(|_| JwtError::MalformedHeader)?;
        let alg = header
            .as_object()
            .and_then(|o| o.get("alg"))
            .and_then(Value::as_str)
            .unwrap_or("");
        if alg != HS256 {
            return Err(JwtError::BadAlgorithm(alg.to_string()));
        }

        let signature = decode_segment(signature_b64).map_err(|_| JwtError::BadCrypto)?;
        let signing_input = format!("{header_b64}.{payload_b64}");
        let mut mac =
            HmacSha256::new_from_slice(&self.secret).map_err(|_| JwtError::SecretMissing)?;
        mac.update(signing_input.as_bytes());
        mac.verify_slice(&signature)
            .map_err(|_| JwtError::BadCrypto)?;

        let payload_json = decode_segment(payload_b64).map_err(|_| JwtError::MalformedPayload)?;
        let payload: Value =
            serde_json::from_slice(&payload_json).map_err(|_| JwtError::MalformedPayload)?;
        let raw = payload
            .as_object()
            .cloned()
            .ok_or(JwtError::MalformedPayload)?;

        let exp = match raw.get("exp") {
            None | Some(Value::Null) => None,
            Some(Value::Number(number)) => Some(json_unix(number).ok_or(JwtError::ExpNotNumber)?),
            Some(_) => return Err(JwtError::ExpNotNumber),
        };
        if let Some(exp) = exp {
            if now_unix.saturating_sub(EXP_LEEWAY_SECS) > exp {
                return Err(JwtError::Expired);
            }
        }

        Ok(JwtClaims {
            role: string_claim(&raw, "role"),
            sub: string_claim(&raw, "sub"),
            exp,
            raw,
        })
    }
}

/// GoTrue `bearerRegexp` (`(?i)^bearer (\S+$)`) in `internal/api/api.go`.
pub fn bearer_token(authorization: &str) -> Option<&str> {
    const PREFIX: &str = "bearer ";
    if authorization.len() < PREFIX.len() {
        return None;
    }
    if !authorization[..PREFIX.len()].eq_ignore_ascii_case(PREFIX) {
        return None;
    }
    let token = &authorization[PREFIX.len()..];
    if token.is_empty() || token.contains(char::is_whitespace) {
        return None;
    }
    Some(token)
}

fn string_claim(raw: &Map<String, Value>, key: &str) -> Option<String> {
    raw.get(key).and_then(Value::as_str).map(str::to_string)
}

fn json_unix(number: &serde_json::Number) -> Option<i64> {
    if let Some(i) = number.as_i64() {
        return Some(i);
    }
    if let Some(u) = number.as_u64() {
        return i64::try_from(u).ok();
    }
    let f = number.as_f64()?;
    if !f.is_finite() {
        return None;
    }
    let truncated = f.trunc();
    if truncated < i64::MIN as f64 || truncated > i64::MAX as f64 {
        return None;
    }
    Some(truncated as i64)
}

fn decode_segment(segment: &str) -> Result<Vec<u8>, ()> {
    URL_SAFE_NO_PAD.decode(segment).map_err(|_| ())
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// `JWT_SECRET` from `vendor/supabase/docker/.env.example`.
    const DEMO_SECRET: &str = "your-super-secret-jwt-token-with-at-least-32-characters-long";

    /// Demo `ANON_KEY` (payload contains whitespace; no `sub`).
    const DEMO_ANON: &str = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyAgCiAgICAicm9sZSI6ICJhbm9uIiwKICAgICJpc3MiOiAic3VwYWJhc2UtZGVtbyIsCiAgICAiaWF0IjogMTY0MTc2OTIwMCwKICAgICJleHAiOiAxNzk5NTM1NjAwCn0.dc_X5iR_VP_qT0zsiyj_I_OZ2T9FtRU2BBNWN8Bu4GE";

    /// Demo `SERVICE_ROLE_KEY`.
    const DEMO_SERVICE: &str = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyAgCiAgICAicm9sZSI6ICJzZXJ2aWNlX3JvbGUiLAogICAgImlzcyI6ICJzdXBhYmFzZS1kZW1vIiwKICAgICJpYXQiOiAxNjQxNzY5MjAwLAogICAgImV4cCI6IDE3OTk1MzU2MDAKfQ.DaYlNEoUrrEn2Ig7tqibS-PHK5vgusbcbo7X36XVt4Q";

    /// After demo `iat` (1641769200), before demo `exp` (1799535600).
    const DURING_DEMO: i64 = 1_791_504_000;

    fn verifier() -> Hs256 {
        Hs256::new(DEMO_SECRET.as_bytes()).unwrap()
    }

    fn sign(secret: &[u8], header: Value, payload: Value) -> String {
        let header_b64 = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&header).unwrap());
        let payload_b64 = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&payload).unwrap());
        let signing_input = format!("{header_b64}.{payload_b64}");
        let mut mac = HmacSha256::new_from_slice(secret).unwrap();
        mac.update(signing_input.as_bytes());
        let sig = URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());
        format!("{signing_input}.{sig}")
    }

    fn hs256_header() -> Value {
        json!({"alg": "HS256", "typ": "JWT"})
    }

    #[test]
    fn demo_anon_key_is_accepted() {
        let claims = verifier().verify_at(DEMO_ANON, DURING_DEMO).unwrap();
        assert_eq!(claims.role.as_deref(), Some("anon"));
        assert_eq!(claims.sub, None);
        assert_eq!(claims.exp, Some(1_799_535_600));
        assert_eq!(claims.raw["iss"], "supabase-demo");
    }

    #[test]
    fn demo_service_role_key_is_accepted() {
        let claims = verifier().verify_at(DEMO_SERVICE, DURING_DEMO).unwrap();
        assert_eq!(claims.role.as_deref(), Some("service_role"));
        assert_eq!(claims.sub, None);
        assert_eq!(claims.exp, Some(1_799_535_600));
    }

    #[test]
    fn user_token_exposes_role_sub_exp() {
        let token = sign(
            DEMO_SECRET.as_bytes(),
            hs256_header(),
            json!({
                "role": "authenticated",
                "sub": "11111111-1111-1111-1111-111111111111",
                "exp": DURING_DEMO + 3600,
            }),
        );
        let claims = verifier().verify_at(&token, DURING_DEMO).unwrap();
        assert_eq!(claims.role.as_deref(), Some("authenticated"));
        assert_eq!(
            claims.sub.as_deref(),
            Some("11111111-1111-1111-1111-111111111111")
        );
        assert_eq!(claims.exp, Some(DURING_DEMO + 3600));
    }

    #[test]
    fn forged_signature_is_rejected() {
        let token = sign(
            b"some-other-secret-that-is-not-the-demo-secret!!",
            hs256_header(),
            json!({"role": "authenticated", "exp": DURING_DEMO + 3600}),
        );
        assert_eq!(
            verifier().verify_at(&token, DURING_DEMO).unwrap_err(),
            JwtError::BadCrypto
        );
    }

    #[test]
    fn tampered_payload_is_rejected() {
        let token = sign(
            DEMO_SECRET.as_bytes(),
            hs256_header(),
            json!({"role": "anon", "exp": DURING_DEMO + 3600}),
        );
        let (h, rest) = token.split_once('.').unwrap();
        let (_, sig) = rest.rsplit_once('.').unwrap();
        let forged_payload = URL_SAFE_NO_PAD.encode(br#"{"role":"service_role"}"#);
        let forged = format!("{h}.{forged_payload}.{sig}");
        assert_eq!(
            verifier().verify_at(&forged, DURING_DEMO).unwrap_err(),
            JwtError::BadCrypto
        );
    }

    #[test]
    fn alg_none_is_rejected() {
        let payload = URL_SAFE_NO_PAD.encode(
            serde_json::to_vec(&json!({"role": "authenticated", "exp": DURING_DEMO + 3600}))
                .unwrap(),
        );
        for header in [
            json!({"alg": "none", "typ": "JWT"}),
            json!({"alg": "None", "typ": "JWT"}),
            json!({"alg": "NONE", "typ": "JWT"}),
        ] {
            let header_b64 = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&header).unwrap());
            let none_token = format!("{header_b64}.{payload}.");
            assert_eq!(
                verifier().verify_at(&none_token, DURING_DEMO).unwrap_err(),
                JwtError::BadAlgorithm(header["alg"].as_str().unwrap().to_string()),
                "{header}"
            );
        }
    }

    #[test]
    fn alg_none_with_hs256_signature_is_still_rejected() {
        let token = sign(
            DEMO_SECRET.as_bytes(),
            json!({"alg": "none", "typ": "JWT"}),
            json!({"role": "authenticated", "exp": DURING_DEMO + 3600}),
        );
        assert_eq!(
            verifier().verify_at(&token, DURING_DEMO).unwrap_err(),
            JwtError::BadAlgorithm("none".into())
        );
    }

    #[test]
    fn expired_token_is_rejected() {
        let token = sign(
            DEMO_SECRET.as_bytes(),
            hs256_header(),
            json!({"role": "authenticated", "exp": DURING_DEMO - 3600}),
        );
        assert_eq!(
            verifier().verify_at(&token, DURING_DEMO).unwrap_err(),
            JwtError::Expired
        );
    }

    #[test]
    fn exp_leeway_matches_postgrest() {
        let token = sign(
            DEMO_SECRET.as_bytes(),
            hs256_header(),
            json!({"role": "anon", "exp": DURING_DEMO}),
        );
        verifier()
            .verify_at(&token, DURING_DEMO + EXP_LEEWAY_SECS)
            .unwrap();
        assert_eq!(
            verifier()
                .verify_at(&token, DURING_DEMO + EXP_LEEWAY_SECS + 1)
                .unwrap_err(),
            JwtError::Expired
        );
    }

    #[test]
    fn rs256_header_is_rejected_without_trying_asymmetric() {
        let token = sign(
            DEMO_SECRET.as_bytes(),
            json!({"alg": "RS256", "typ": "JWT"}),
            json!({"role": "authenticated", "exp": DURING_DEMO + 3600}),
        );
        assert_eq!(
            verifier().verify_at(&token, DURING_DEMO).unwrap_err(),
            JwtError::BadAlgorithm("RS256".into())
        );
    }

    #[test]
    fn kid_is_ignored_and_hs256_secret_is_used() {
        let token = sign(
            DEMO_SECRET.as_bytes(),
            json!({"alg": "HS256", "typ": "JWT", "kid": "legacy"}),
            json!({"role": "authenticated", "sub": "user-1", "exp": DURING_DEMO + 60}),
        );
        let claims = verifier().verify_at(&token, DURING_DEMO).unwrap();
        assert_eq!(claims.sub.as_deref(), Some("user-1"));
    }

    #[test]
    fn empty_token_and_wrong_part_count() {
        assert_eq!(
            verifier().verify_at("", DURING_DEMO).unwrap_err(),
            JwtError::Empty
        );
        assert_eq!(
            verifier().verify_at("onlyone", DURING_DEMO).unwrap_err(),
            JwtError::UnexpectedParts(1)
        );
        assert_eq!(
            verifier().verify_at("two.parts", DURING_DEMO).unwrap_err(),
            JwtError::UnexpectedParts(2)
        );
        assert_eq!(
            verifier().verify_at("a.b.c.d", DURING_DEMO).unwrap_err(),
            JwtError::UnexpectedParts(4)
        );
    }

    #[test]
    fn exp_must_be_a_number_when_present() {
        let token = sign(
            DEMO_SECRET.as_bytes(),
            hs256_header(),
            json!({"role": "anon", "exp": "soon"}),
        );
        assert_eq!(
            verifier().verify_at(&token, DURING_DEMO).unwrap_err(),
            JwtError::ExpNotNumber
        );
    }

    #[test]
    fn empty_secret_is_rejected() {
        assert_eq!(
            Hs256::new(&[] as &[u8]).unwrap_err(),
            JwtError::SecretMissing
        );
    }

    #[test]
    fn bearer_token_matches_gotrue() {
        assert_eq!(bearer_token("Bearer abc"), Some("abc"));
        assert_eq!(bearer_token("bearer abc"), Some("abc"));
        assert_eq!(bearer_token("BEARER abc"), Some("abc"));
        assert_eq!(bearer_token("Bearer"), None);
        assert_eq!(bearer_token("Bearer "), None);
        assert_eq!(bearer_token("Bearer abc def"), None);
        assert_eq!(bearer_token("Basic abc"), None);
        assert_eq!(bearer_token(" Bearer abc"), None);
    }

    #[test]
    fn debug_redacts_secret() {
        let debug = format!("{:?}", verifier());
        assert!(debug.contains("<redacted>"));
        assert!(!debug.contains(DEMO_SECRET));
    }
}
