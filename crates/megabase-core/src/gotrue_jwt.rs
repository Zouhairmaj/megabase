// Ported from golang-jwt/jwt parser.go, errors.go, validator.go, hmac.go and
// types.go (MIT), v5.3.1 (the version pinned in vendor/auth/go.mod at auth
// v2.197.0), and supabase/auth internal/api/auth.go `parseJWTClaims` (MIT),
// pin v2.197.0.

//! HS256 verification with GoTrue's error text.
//!
//! GoTrue parses bearer tokens with golang-jwt v5 and
//! `WithValidMethods(config.JWT.ValidMethods)`. With only `JWT_SECRET`
//! configured that list is `["HS256"]`. The parser's error is embedded in
//! `invalid JWT: unable to parse or verify signature, <error>`, so Auth needs
//! golang-jwt's check order and messages, not PostgREST's.

use std::fmt;

use hmac::Mac;
use serde::de::{Deserializer, MapAccess, Visitor};
use serde_json::{Map, Value};

use crate::ids::UserId;
use crate::jwt::{HmacSha256, Hs256, JwtClaims};

/// Algorithms golang-jwt v5 registers with `RegisterSigningMethod`.
const REGISTERED_ALGS: &[&str] = &[
    "HS256", "HS384", "HS512", "RS256", "RS384", "RS512", "PS256", "PS384", "PS512", "ES256",
    "ES384", "ES512", "EdDSA", "none",
];

/// `AccessTokenClaims` fields (including the embedded `RegisteredClaims`)
/// that Go decodes as `string`.
const STRING_CLAIMS: &[&str] = &[
    "iss",
    "sub",
    "jti",
    "email",
    "phone",
    "role",
    "aal",
    "session_id",
    "client_id",
    "scope",
];

/// `AccessTokenClaims` fields Go decodes as `map[string]interface {}`.
const MAP_CLAIMS: &[&str] = &["app_metadata", "user_metadata"];

/// `RegisteredClaims` fields decoded through `NumericDate.UnmarshalJSON`.
const DATE_CLAIMS: &[&str] = &["exp", "nbf", "iat"];

/// Why golang-jwt rejected a token. `Display` is the exact Go error string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GoTrueJwtError {
    /// `token contains an invalid number of segments`.
    Segments,
    /// A segment is not RawURL base64. Carries the segment name and Go's
    /// `CorruptInputError` offset.
    Base64 { segment: Segment, offset: usize },
    /// `json.Unmarshal` failed on the header or the claims.
    Json { segment: Segment, detail: String },
    /// `alg` is missing or not a string.
    AlgUnspecified,
    /// `alg` is not a registered golang-jwt signing method.
    AlgUnavailable,
    /// `alg` is registered but not in `ValidMethods` (`["HS256"]`).
    MethodInvalid(String),
    /// HMAC mismatch.
    Signature,
    /// The validator rejected `exp` and/or `nbf`.
    Claims(Vec<ClaimProblem>),
}

/// The segment a decode error refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Segment {
    Header,
    Claim,
    Signature,
}

impl Segment {
    fn name(self) -> &'static str {
        match self {
            Self::Header => "header",
            Self::Claim => "claim",
            Self::Signature => "signature",
        }
    }
}

/// One `Validator.Validate` failure, in Go's order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimProblem {
    Expired,
    NotValidYet,
}

impl fmt::Display for GoTrueJwtError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Segments => {
                f.write_str("token is malformed: token contains an invalid number of segments")
            }
            Self::Base64 { segment, offset } => write!(
                f,
                "token is malformed: could not base64 decode {}: illegal base64 data at input byte {offset}",
                segment.name()
            ),
            Self::Json { segment, detail } => write!(
                f,
                "token is malformed: could not JSON decode {}: {detail}",
                segment.name()
            ),
            Self::AlgUnspecified => {
                f.write_str("token is unverifiable: signing method (alg) is unspecified")
            }
            Self::AlgUnavailable => {
                f.write_str("token is unverifiable: signing method (alg) is unavailable")
            }
            Self::MethodInvalid(alg) => {
                write!(f, "token signature is invalid: signing method {alg} is invalid")
            }
            Self::Signature => f.write_str("token signature is invalid: signature is invalid"),
            Self::Claims(problems) => {
                f.write_str("token has invalid claims: ")?;
                for (index, problem) in problems.iter().enumerate() {
                    if index > 0 {
                        f.write_str(", ")?;
                    }
                    f.write_str(match problem {
                        ClaimProblem::Expired => "token is expired",
                        ClaimProblem::NotValidYet => "token is not valid yet",
                    })?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for GoTrueJwtError {}

impl Hs256 {
    /// Verify `token` the way GoTrue's `parseJWTClaims` does, at `now_unix`.
    ///
    /// # Errors
    ///
    /// Returns the golang-jwt failure, in golang-jwt's check order.
    pub fn verify_gotrue_at(
        &self,
        token: &str,
        now_unix: i64,
    ) -> Result<JwtClaims, GoTrueJwtError> {
        let mut parts = token.split('.');
        let (Some(header_b64), Some(claims_b64), Some(signature_b64), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(GoTrueJwtError::Segments);
        };

        let header_bytes =
            go_raw_url_decode(header_b64).map_err(|offset| GoTrueJwtError::Base64 {
                segment: Segment::Header,
                offset,
            })?;
        let header = decode_header(&header_bytes)?;

        let claim_bytes =
            go_raw_url_decode(claims_b64).map_err(|offset| GoTrueJwtError::Base64 {
                segment: Segment::Claim,
                offset,
            })?;
        let raw = decode_claims(&claim_bytes)?;

        let alg = match header.get("alg") {
            Some(Value::String(alg)) => alg.as_str(),
            _ => return Err(GoTrueJwtError::AlgUnspecified),
        };
        if !REGISTERED_ALGS.contains(&alg) {
            return Err(GoTrueJwtError::AlgUnavailable);
        }
        let signature =
            go_raw_url_decode(signature_b64).map_err(|offset| GoTrueJwtError::Base64 {
                segment: Segment::Signature,
                offset,
            })?;
        if alg != "HS256" {
            return Err(GoTrueJwtError::MethodInvalid(alg.to_string()));
        }

        let mut mac =
            HmacSha256::new_from_slice(self.secret()).map_err(|_| GoTrueJwtError::Signature)?;
        mac.update(header_b64.as_bytes());
        mac.update(b".");
        mac.update(claims_b64.as_bytes());
        mac.verify_slice(&signature)
            .map_err(|_| GoTrueJwtError::Signature)?;

        let exp = numeric_date(&raw, "exp");
        let nbf = numeric_date(&raw, "nbf");
        let mut problems = Vec::new();
        if exp.is_some_and(|exp| now_unix >= exp) {
            problems.push(ClaimProblem::Expired);
        }
        if nbf.is_some_and(|nbf| now_unix < nbf) {
            problems.push(ClaimProblem::NotValidYet);
        }
        if !problems.is_empty() {
            return Err(GoTrueJwtError::Claims(problems));
        }

        let string_claim = |key: &str| raw.get(key).and_then(Value::as_str).map(str::to_string);
        Ok(JwtClaims {
            role: string_claim("role"),
            sub: string_claim("sub").map(UserId::new),
            exp,
            raw,
        })
    }

    /// [`Hs256::verify_gotrue_at`] with the current Unix time.
    ///
    /// # Errors
    ///
    /// Same as [`Hs256::verify_gotrue_at`].
    pub fn verify_gotrue(&self, token: &str) -> Result<JwtClaims, GoTrueJwtError> {
        self.verify_gotrue_at(token, crate::jwt::unix_now())
    }
}

/// `base64.RawURLEncoding.DecodeString` (non-strict). `Err` is Go's
/// `CorruptInputError` offset. CR and LF are skipped, as in Go.
pub fn go_raw_url_decode(input: &str) -> Result<Vec<u8>, usize> {
    let src = input.as_bytes();
    let mut out = Vec::with_capacity(src.len() * 3 / 4);
    let mut si = 0usize;
    while si < src.len() {
        let mut dbuf = [0u8; 4];
        let mut dlen = 4usize;
        let mut j = 0usize;
        while j < 4 {
            if si == src.len() {
                if j == 0 {
                    return Ok(out);
                }
                if j == 1 {
                    return Err(si - j);
                }
                dlen = j;
                break;
            }
            let byte = src[si];
            si += 1;
            if let Some(value) = url_alphabet(byte) {
                dbuf[j] = value;
                j += 1;
                continue;
            }
            if byte == b'\n' || byte == b'\r' {
                continue;
            }
            return Err(si - 1);
        }
        let quantum = (u32::from(dbuf[0]) << 18)
            | (u32::from(dbuf[1]) << 12)
            | (u32::from(dbuf[2]) << 6)
            | u32::from(dbuf[3]);
        let bytes = quantum.to_be_bytes();
        out.extend_from_slice(&bytes[1..dlen]);
    }
    Ok(out)
}

fn url_alphabet(byte: u8) -> Option<u8> {
    match byte {
        b'A'..=b'Z' => Some(byte - b'A'),
        b'a'..=b'z' => Some(byte - b'a' + 26),
        b'0'..=b'9' => Some(byte - b'0' + 52),
        b'-' => Some(62),
        b'_' => Some(63),
        _ => None,
    }
}

/// `json.Unmarshal(headerBytes, &token.Header)` with a `map[string]interface{}`.
fn decode_header(bytes: &[u8]) -> Result<Map<String, Value>, GoTrueJwtError> {
    let segment = Segment::Header;
    match parse_json(bytes, segment)? {
        Value::Object(map) => Ok(map),
        Value::Null => Ok(Map::new()),
        other => Err(GoTrueJwtError::Json {
            segment,
            detail: format!(
                "json: cannot unmarshal {} into Go value of type map[string]interface {{}}",
                go_kind(&other)
            ),
        }),
    }
}

/// `json.Unmarshal(claimBytes, &claims)` into GoTrue's `AccessTokenClaims`.
///
/// Type mismatches are reported for the first offending field in document
/// order, after the whole document parsed, as Go's decoder does.
fn decode_claims(bytes: &[u8]) -> Result<Map<String, Value>, GoTrueJwtError> {
    let segment = Segment::Claim;
    let value = parse_json(bytes, segment)?;
    match value {
        Value::Object(_) => {}
        Value::Null => return Ok(Map::new()),
        other => {
            return Err(GoTrueJwtError::Json {
                segment,
                detail: format!(
                    "json: cannot unmarshal {} into Go value of type tokens.AccessTokenClaims",
                    go_kind(&other)
                ),
            })
        }
    }
    let ordered = ordered_entries(bytes).unwrap_or_default();
    let mut type_error = None;
    for (key, field) in &ordered {
        if DATE_CLAIMS.contains(&key.as_str()) {
            if let Some(detail) = numeric_date_error(field) {
                return Err(GoTrueJwtError::Json { segment, detail });
            }
            continue;
        }
        if type_error.is_some() || field.is_null() {
            continue;
        }
        let expected = if STRING_CLAIMS.contains(&key.as_str()) {
            (!field.is_string()).then_some("string")
        } else if MAP_CLAIMS.contains(&key.as_str()) {
            (!field.is_object()).then_some("map[string]interface {}")
        } else if key == "is_anonymous" {
            (!field.is_boolean()).then_some("bool")
        } else {
            None
        };
        if let Some(go_type) = expected {
            type_error = Some(format!(
                "json: cannot unmarshal {} into Go struct field Claims.{key} of type {go_type}",
                go_kind(field)
            ));
        }
    }
    if let Some(detail) = type_error {
        return Err(GoTrueJwtError::Json { segment, detail });
    }
    match value {
        Value::Object(map) => Ok(map),
        _ => Ok(Map::new()),
    }
}

/// `NumericDate.UnmarshalJSON` failure text for a non-null value.
fn numeric_date_error(value: &Value) -> Option<String> {
    match value {
        Value::Null | Value::Number(_) => None,
        Value::String(text) => {
            if is_json_number(text) {
                return None;
            }
            let quoted = serde_json::to_string(text).ok()?;
            Some(format!(
                "could not parse NumericData: json: cannot unmarshal string {quoted} into Go value of type json.Number: invalid syntax"
            ))
        }
        other => Some(format!(
            "could not parse NumericData: json: cannot unmarshal {} into Go value of type json.Number",
            go_kind(other)
        )),
    }
}

/// Go `isValidNumber`: a bare JSON number literal, no surrounding whitespace.
fn is_json_number(text: &str) -> bool {
    text.trim() == text && matches!(serde_json::from_str::<Value>(text), Ok(Value::Number(_)))
}

/// Whole seconds of a `NumericDate` claim. A string holding a number literal
/// is accepted by `json.Number`, as in Go.
fn numeric_date(raw: &Map<String, Value>, key: &str) -> Option<i64> {
    let seconds = match raw.get(key)? {
        Value::Number(number) => number.as_f64()?,
        Value::String(text) if is_json_number(text) => text.parse::<f64>().ok()?,
        _ => return None,
    };
    if !seconds.is_finite() {
        return None;
    }
    let truncated = seconds.trunc();
    if truncated < i64::MIN as f64 || truncated > i64::MAX as f64 {
        return None;
    }
    Some(truncated as i64)
}

fn parse_json(bytes: &[u8], segment: Segment) -> Result<Value, GoTrueJwtError> {
    serde_json::from_slice::<Value>(bytes).map_err(|error| GoTrueJwtError::Json {
        segment,
        detail: go_syntax_detail(bytes, &error),
    })
}

/// Go `encoding/json` syntax error text for the cases a JWT segment usually
/// hits: empty or truncated input, and an invalid first byte.
fn go_syntax_detail(bytes: &[u8], error: &serde_json::Error) -> String {
    let first = bytes
        .iter()
        .copied()
        .find(|byte| !matches!(byte, b' ' | b'\t' | b'\n' | b'\r'));
    match first {
        None => "unexpected end of JSON input".to_string(),
        Some(byte)
            if !matches!(
                byte,
                b'{' | b'[' | b'"' | b'-' | b'0'..=b'9' | b't' | b'f' | b'n'
            ) =>
        {
            format!(
                "invalid character {} looking for beginning of value",
                go_quote_char(byte)
            )
        }
        Some(_) if error.is_eof() => "unexpected end of JSON input".to_string(),
        Some(_) => {
            let index = error_offset(bytes, error);
            let byte = bytes.get(index).copied().unwrap_or(b' ');
            format!(
                "invalid character {} after top-level value",
                go_quote_char(byte)
            )
        }
    }
}

fn error_offset(bytes: &[u8], error: &serde_json::Error) -> usize {
    let (line, column) = (error.line(), error.column());
    let mut current = 1usize;
    let mut start = 0usize;
    for (index, byte) in bytes.iter().enumerate() {
        if current == line {
            break;
        }
        if *byte == b'\n' {
            current += 1;
            start = index + 1;
        }
    }
    (start + column).saturating_sub(1)
}

/// Go `quoteChar` in `encoding/json/scanner.go`.
fn go_quote_char(byte: u8) -> String {
    match byte {
        b'\'' => "'\\''".to_string(),
        b'"' => "'\"'".to_string(),
        0x20..=0x7e => format!("'{}'", byte as char),
        b'\t' => "'\\t'".to_string(),
        b'\n' => "'\\n'".to_string(),
        b'\r' => "'\\r'".to_string(),
        0x07 => "'\\a'".to_string(),
        0x08 => "'\\b'".to_string(),
        0x0c => "'\\f'".to_string(),
        0x0b => "'\\v'".to_string(),
        other if other < 0x80 => format!("'\\x{other:02x}'"),
        _ => "'\\ufffd'".to_string(),
    }
}

fn go_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Top-level object entries in document order (duplicates kept).
fn ordered_entries(bytes: &[u8]) -> Option<Vec<(String, Value)>> {
    struct Entries;
    impl<'de> Visitor<'de> for Entries {
        type Value = Vec<(String, Value)>;
        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("a JSON object")
        }
        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
            let mut entries = Vec::new();
            while let Some(entry) = map.next_entry::<String, Value>()? {
                entries.push(entry);
            }
            Ok(entries)
        }
    }
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    deserializer.deserialize_map(Entries).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine;
    use serde_json::json;

    const DEMO_SECRET: &str = "your-super-secret-jwt-token-with-at-least-32-characters-long";
    const NOW: i64 = 1_791_504_000;

    fn verifier() -> Hs256 {
        Hs256::new(DEMO_SECRET.as_bytes()).unwrap()
    }

    fn b64(value: &Value) -> String {
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(value).unwrap())
    }

    fn signed(header: &Value, payload: &Value) -> String {
        let input = format!("{}.{}", b64(header), b64(payload));
        let mut mac = HmacSha256::new_from_slice(DEMO_SECRET.as_bytes()).unwrap();
        mac.update(input.as_bytes());
        format!(
            "{input}.{}",
            URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
        )
    }

    fn message(token: &str) -> String {
        verifier()
            .verify_gotrue_at(token, NOW)
            .unwrap_err()
            .to_string()
    }

    #[test]
    fn alg_none_spellings_follow_golang_jwt() {
        let payload = b64(&json!({"role": "anon", "sub": "x"}));
        let none = format!("{}.{payload}.", b64(&json!({"alg": "none", "typ": "JWT"})));
        assert_eq!(
            message(&none),
            "token signature is invalid: signing method none is invalid"
        );
        for alg in ["None", "NONE", "hs256", "HS257"] {
            let token = format!("{}.{payload}.", b64(&json!({"alg": alg, "typ": "JWT"})));
            assert_eq!(
                message(&token),
                "token is unverifiable: signing method (alg) is unavailable",
                "{alg}"
            );
        }
        let rs = format!("{}.{payload}.c2ln", b64(&json!({"alg": "RS256"})));
        assert_eq!(
            message(&rs),
            "token signature is invalid: signing method RS256 is invalid"
        );
    }

    #[test]
    fn forged_hs256_signature_is_signature_invalid() {
        let token = format!(
            "{}.{}.{}",
            b64(&json!({"alg": "HS256", "typ": "JWT"})),
            b64(&json!({"role": "service_role", "sub": "abc"})),
            URL_SAFE_NO_PAD.encode([7u8; 16])
        );
        assert_eq!(
            message(&token),
            "token signature is invalid: signature is invalid"
        );
    }

    #[test]
    fn missing_alg_and_segment_count() {
        let payload = b64(&json!({}));
        let token = format!("{}.{payload}.", b64(&json!({"typ": "JWT"})));
        assert_eq!(
            message(&token),
            "token is unverifiable: signing method (alg) is unspecified"
        );
        for token in ["", "a", "a.b", "a.b.c.d"] {
            assert_eq!(
                message(token),
                "token is malformed: token contains an invalid number of segments"
            );
        }
    }

    #[test]
    fn base64_offsets_match_go() {
        assert_eq!(go_raw_url_decode("eyJ9"), Ok(b"{\"}".to_vec()));
        assert_eq!(go_raw_url_decode("a"), Err(0));
        assert_eq!(go_raw_url_decode("abcde"), Err(4));
        assert_eq!(go_raw_url_decode("ab=c"), Err(2));
        assert_eq!(go_raw_url_decode("ab+c"), Err(2));
        assert_eq!(go_raw_url_decode("YQ\n=="), Err(3));
        assert_eq!(go_raw_url_decode("YR"), Ok(b"a".to_vec()));
        assert_eq!(
            message("!!!.e30.sig"),
            "token is malformed: could not base64 decode header: illegal base64 data at input byte 0"
        );
    }

    #[test]
    fn json_errors_match_go() {
        let header = URL_SAFE_NO_PAD.encode(b"[1]");
        assert_eq!(
            message(&format!("{header}.e30.")),
            "token is malformed: could not JSON decode header: json: cannot unmarshal array into Go value of type map[string]interface {}"
        );
        let header = URL_SAFE_NO_PAD.encode(b"x");
        assert_eq!(
            message(&format!("{header}.e30.")),
            "token is malformed: could not JSON decode header: invalid character 'x' looking for beginning of value"
        );
        let hs = b64(&json!({"alg": "HS256"}));
        assert_eq!(
            message(&format!("{hs}..")),
            "token is malformed: could not JSON decode claim: unexpected end of JSON input"
        );
        let claims = URL_SAFE_NO_PAD.encode(br#"{"sub": 5, "role": true}"#);
        assert_eq!(
            message(&format!("{hs}.{claims}.")),
            "token is malformed: could not JSON decode claim: json: cannot unmarshal number into Go struct field Claims.sub of type string"
        );
        let claims = b64(&json!({"exp": "soon"}));
        assert_eq!(
            message(&format!("{hs}.{claims}.")),
            "token is malformed: could not JSON decode claim: could not parse NumericData: json: cannot unmarshal string \"soon\" into Go value of type json.Number: invalid syntax"
        );
    }

    #[test]
    fn numeric_string_dates_follow_json_number() {
        let header = json!({"alg": "HS256", "typ": "JWT"});
        let token = signed(&header, &json!({"sub": "u", "exp": "1e2"}));
        assert_eq!(
            message(&token),
            "token has invalid claims: token is expired"
        );
        let token = signed(&header, &json!({"sub": "u", "exp": " 5"}));
        assert_eq!(
            message(&token),
            "token is malformed: could not JSON decode claim: could not parse NumericData: json: cannot unmarshal string \" 5\" into Go value of type json.Number: invalid syntax"
        );
    }

    #[test]
    fn null_header_means_no_alg() {
        let header = URL_SAFE_NO_PAD.encode(b"null");
        assert_eq!(
            message(&format!("{header}.e30.")),
            "token is unverifiable: signing method (alg) is unspecified"
        );
    }

    #[test]
    fn expiry_and_not_before() {
        let header = json!({"alg": "HS256", "typ": "JWT"});
        let token = signed(&header, &json!({"sub": "u", "exp": NOW}));
        assert_eq!(
            message(&token),
            "token has invalid claims: token is expired"
        );
        let token = signed(&header, &json!({"sub": "u", "exp": NOW + 1}));
        let claims = verifier().verify_gotrue_at(&token, NOW).unwrap();
        assert_eq!(claims.sub.as_deref(), Some("u"));
        let token = signed(&header, &json!({"exp": NOW - 1, "nbf": NOW + 5}));
        assert_eq!(
            message(&token),
            "token has invalid claims: token is expired, token is not valid yet"
        );
    }

    #[test]
    fn kid_falls_back_to_the_secret() {
        let token = signed(
            &json!({"alg": "HS256", "kid": "unknown"}),
            &json!({"role": "authenticated", "sub": "s", "exp": NOW + 60}),
        );
        let claims = verifier().verify_gotrue_at(&token, NOW).unwrap();
        assert_eq!(claims.role.as_deref(), Some("authenticated"));
    }
}

#[cfg(test)]
mod properties {
    use super::*;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine;
    use proptest::prelude::*;

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        #[test]
        fn go_decoder_agrees_with_canonical_base64(bytes in proptest::collection::vec(any::<u8>(), 0..64)) {
            let encoded = URL_SAFE_NO_PAD.encode(&bytes);
            prop_assert_eq!(go_raw_url_decode(&encoded), Ok(bytes));
        }

        #[test]
        fn verify_gotrue_never_panics(token in "\\PC{0,96}") {
            let verifier = Hs256::new(b"your-super-secret-jwt-token-with-at-least-32-characters-long".to_vec()).unwrap();
            let _ = verifier.verify_gotrue_at(&token, 0);
        }

        #[test]
        fn forged_tokens_never_verify(
            alg in prop::sample::select(vec!["none", "None", "NONE", "HS256"]),
            role in prop::sample::select(vec!["anon", "authenticated", "service_role", "postgres"]),
            sub in "[a-z0-9]{16}",
            sig in proptest::collection::vec(any::<u8>(), 16),
        ) {
            let verifier = Hs256::new(b"your-super-secret-jwt-token-with-at-least-32-characters-long".to_vec()).unwrap();
            let header = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&serde_json::json!({"alg": alg, "typ": "JWT"})).unwrap());
            let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&serde_json::json!({"role": role, "sub": sub})).unwrap());
            let signature = if alg.eq_ignore_ascii_case("none") { String::new() } else { URL_SAFE_NO_PAD.encode(sig) };
            let error = verifier.verify_gotrue_at(&format!("{header}.{payload}.{signature}"), 0).unwrap_err();
            let expected = match alg {
                "none" => GoTrueJwtError::MethodInvalid("none".into()),
                "HS256" => GoTrueJwtError::Signature,
                _ => GoTrueJwtError::AlgUnavailable,
            };
            prop_assert_eq!(error, expected);
        }
    }
}
