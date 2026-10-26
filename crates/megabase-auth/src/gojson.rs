// Ported from the Go standard library encoding/json (BSD-3-Clause) scanner and
// decoder error wording, as surfaced by supabase/auth internal/api/helpers.go
// `retrieveRequestParams` (MIT), pin v2.197.0.

//! Go-identical `bad_json` detail text for request bodies.
//!
//! GoTrue formats `Could not parse request body as JSON: %v` with the error
//! from `json.Unmarshal`. That error is either a `SyntaxError` from the
//! scanner or an `UnmarshalTypeError`. serde reports neither the same way, so
//! on failure this module re-derives the Go text from the body bytes.

use std::fmt;

use serde::de::{DeserializeOwned, MapAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::Value;

use crate::error::{bad_json, GoTrueError};

/// Go `maxNestingDepth` in `encoding/json`.
const MAX_DEPTH: usize = 10_000;

/// Go type of a struct field, as far as the auth params use them.
#[derive(Clone, Copy)]
pub(crate) enum Kind {
    /// `string`
    Str,
    /// `map[string]interface{}`
    Map,
    /// `bool`
    Bool,
    /// An `encoding.TextUnmarshaler` such as `uuid.UUID`: the key is matched
    /// like any field and `null` is a no-op, but the value is checked by the
    /// caller.
    Text,
}

/// A Go params struct: its name in `internal/api` and its JSON-tagged fields.
pub(crate) struct GoStruct {
    pub name: &'static str,
    pub fields: &'static [(&'static str, Kind)],
}

/// `SignupParams` in `internal/api/signup.go`.
pub(crate) const SIGNUP_PARAMS: GoStruct = GoStruct {
    name: "SignupParams",
    fields: &[
        ("email", Kind::Str),
        ("phone", Kind::Str),
        ("password", Kind::Str),
        ("data", Kind::Map),
        ("channel", Kind::Str),
        ("code_challenge_method", Kind::Str),
        ("code_challenge", Kind::Str),
    ],
};

/// `PasswordGrantParams` in `internal/api/token.go`.
pub(crate) const PASSWORD_GRANT_PARAMS: GoStruct = GoStruct {
    name: "PasswordGrantParams",
    fields: &[
        ("email", Kind::Str),
        ("phone", Kind::Str),
        ("password", Kind::Str),
    ],
};

/// `OtpParams` in `internal/api/otp.go`.
pub(crate) const OTP_PARAMS: GoStruct = GoStruct {
    name: "OtpParams",
    fields: &[
        ("email", Kind::Str),
        ("phone", Kind::Str),
        ("create_user", Kind::Bool),
        ("data", Kind::Map),
        ("channel", Kind::Str),
        ("code_challenge_method", Kind::Str),
        ("code_challenge", Kind::Str),
    ],
};

/// `MagicLinkParams` in `internal/api/magic_link.go`.
pub(crate) const MAGIC_LINK_PARAMS: GoStruct = GoStruct {
    name: "MagicLinkParams",
    fields: &[
        ("email", Kind::Str),
        ("data", Kind::Map),
        ("code_challenge_method", Kind::Str),
        ("code_challenge", Kind::Str),
    ],
};

/// `InviteParams` in `internal/api/invite.go`.
pub(crate) const INVITE_PARAMS: GoStruct = GoStruct {
    name: "InviteParams",
    fields: &[("email", Kind::Str), ("data", Kind::Map)],
};

/// `ResendConfirmationParams` in `internal/api/resend.go`.
pub(crate) const RESEND_PARAMS: GoStruct = GoStruct {
    name: "ResendConfirmationParams",
    fields: &[
        ("type", Kind::Str),
        ("email", Kind::Str),
        ("phone", Kind::Str),
        ("code_challenge", Kind::Str),
        ("code_challenge_method", Kind::Str),
    ],
};

/// `RefreshTokenGrantParams` in `internal/api/token_refresh.go`.
pub(crate) const REFRESH_TOKEN_GRANT_PARAMS: GoStruct = GoStruct {
    name: "RefreshTokenGrantParams",
    fields: &[("refresh_token", Kind::Str)],
};

/// `PKCEGrantParams` in `internal/api/token.go`.
pub(crate) const PKCE_GRANT_PARAMS: GoStruct = GoStruct {
    name: "PKCEGrantParams",
    fields: &[("auth_code", Kind::Str), ("code_verifier", Kind::Str)],
};

/// `IdTokenGrantParams` in `internal/api/token_oidc.go`.
pub(crate) const ID_TOKEN_GRANT_PARAMS: GoStruct = GoStruct {
    name: "IdTokenGrantParams",
    fields: &[
        ("id_token", Kind::Str),
        ("access_token", Kind::Str),
        ("nonce", Kind::Str),
        ("provider", Kind::Str),
        ("client_id", Kind::Str),
        ("issuer", Kind::Str),
        ("link_identity", Kind::Bool),
    ],
};

/// `Web3GrantParams` in `internal/api/web3.go`.
pub(crate) const WEB3_GRANT_PARAMS: GoStruct = GoStruct {
    name: "Web3GrantParams",
    fields: &[
        ("message", Kind::Str),
        ("signature", Kind::Str),
        ("chain", Kind::Str),
    ],
};

/// Decode `bytes` as `T`; on failure return GoTrue's `bad_json` error with
/// the text Go's `json.Unmarshal` would have produced for `target`.
///
/// # Errors
/// Returns the `bad_json` (400) error when the body is not valid for `T`.
pub(crate) fn decode<T: DeserializeOwned>(
    bytes: &[u8],
    target: &GoStruct,
) -> Result<T, GoTrueError> {
    let decoded = match normalized_object(bytes, target) {
        Some(Ok(object)) => serde_json::from_value(Value::Object(object)),
        // Go reports the first type mismatch even when a later duplicate
        // key would have overwritten the field with a valid value.
        Some(Err(message)) => return Err(bad_json(message)),
        None => serde_json::from_slice(bytes),
    };
    decoded.map_err(|error| bad_json(go_message(bytes, target, &error)))
}

/// Re-key the top-level object the way Go's decoder matches fields: an exact
/// name is kept, otherwise an ASCII case-insensitive match maps to the
/// canonical field name, and a later duplicate overwrites an earlier one. A
/// `null` for a string field is a no-op in Go, so it keeps an earlier value.
/// `Some(Err)` carries the first Go type-mismatch message; `None` when the
/// body is not an object (or too deeply nested for serde).
fn normalized_object(
    bytes: &[u8],
    target: &GoStruct,
) -> Option<Result<serde_json::Map<String, Value>, String>> {
    let Pairs(pairs) = serde_json::from_slice(bytes).ok()?;
    let mut object = serde_json::Map::new();
    for (key, value) in pairs {
        let field = target
            .fields
            .iter()
            .find(|(name, _)| *name == key)
            .or_else(|| {
                target
                    .fields
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case(&key))
            });
        let Some((name, kind)) = field else {
            object.insert(key, value);
            continue;
        };
        if let Some(message) = field_mismatch(target, name, *kind, &value) {
            return Some(Err(message));
        }
        if value.is_null() && matches!(kind, Kind::Str | Kind::Bool | Kind::Text) {
            continue;
        }
        object.insert((*name).to_string(), value);
    }
    Some(Ok(object))
}

fn go_message(bytes: &[u8], target: &GoStruct, serde_error: &serde_json::Error) -> String {
    if let Some(message) = syntax_error(bytes) {
        return message;
    }
    type_error(bytes, target).unwrap_or_else(|| serde_error.to_string())
}

/// Ordered key/value pairs of a JSON object (Go reports the first mismatch in
/// document order).
pub(crate) struct Pairs(pub(crate) Vec<(String, Value)>);

impl<'de> Deserialize<'de> for Pairs {
    fn deserialize<D: Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        struct PairsVisitor;
        impl<'de> Visitor<'de> for PairsVisitor {
            type Value = Pairs;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an object")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Pairs, A::Error> {
                let mut pairs = Vec::new();
                while let Some(pair) = map.next_entry::<String, Value>()? {
                    pairs.push(pair);
                }
                Ok(Pairs(pairs))
            }
        }
        de.deserialize_map(PairsVisitor)
    }
}

fn value_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Go `UnmarshalTypeError.Error()` for a syntactically valid body.
fn type_error(bytes: &[u8], target: &GoStruct) -> Option<String> {
    let Ok(Pairs(pairs)) = serde_json::from_slice::<Pairs>(bytes) else {
        let value = serde_json::from_slice::<Value>(bytes).ok()?;
        return Some(format!(
            "json: cannot unmarshal {} into Go value of type api.{}",
            value_kind(&value),
            target.name
        ));
    };
    for (key, value) in &pairs {
        let Some((name, kind)) = target
            .fields
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(key))
        else {
            continue;
        };
        if let Some(message) = field_mismatch(target, name, *kind, value) {
            return Some(message);
        }
    }
    None
}

/// Go `UnmarshalTypeError` text when `value` does not fit field `name`.
fn field_mismatch(target: &GoStruct, name: &str, kind: Kind, value: &Value) -> Option<String> {
    let (matches, go_type) = match kind {
        Kind::Str => (value.is_string(), "string"),
        Kind::Map => (value.is_object(), "map[string]interface {}"),
        Kind::Bool => (value.is_boolean(), "bool"),
        Kind::Text => (true, ""),
    };
    (!matches && !value.is_null()).then(|| {
        format!(
            "json: cannot unmarshal {} into Go struct field {}.{name} of type {go_type}",
            value_kind(value),
            target.name
        )
    })
}

#[derive(Clone, Copy)]
enum Frame {
    Object,
    Array,
}

/// Where the Go scanner is between tokens.
#[derive(Clone, Copy)]
enum State {
    /// A value must start here.
    Value,
    /// Just after `[`: a value or `]`.
    ArrayFirst,
    /// Just after `{`: a key or `}`.
    ObjectFirst,
    /// After `,` in an object: a key.
    ObjectKey,
    /// After a key: `:`.
    Colon,
    /// After a complete value.
    After,
}

const EOF: &str = "unexpected end of JSON input";

/// Go `quoteChar`.
fn quote_char(byte: u8) -> String {
    match byte {
        b'\'' => "'\\''".into(),
        b'"' => "'\"'".into(),
        b'\\' => "'\\\\'".into(),
        0x07 => "'\\a'".into(),
        0x08 => "'\\b'".into(),
        0x0c => "'\\f'".into(),
        b'\n' => "'\\n'".into(),
        b'\r' => "'\\r'".into(),
        b'\t' => "'\\t'".into(),
        0x0b => "'\\v'".into(),
        0x00..=0x1f | 0x7f => format!("'\\x{byte:02x}'"),
        0x80..=0xa0 | 0xad => format!("'\\u{byte:04x}'"),
        _ => format!("'{}'", char::from(byte)),
    }
}

fn invalid(byte: u8, context: &str) -> String {
    format!("invalid character {} {context}", quote_char(byte))
}

/// Go `checkValid`: the `SyntaxError` text, or `None` when the body is valid
/// JSON. Iterative so arbitrary nesting cannot overflow the stack.
pub(crate) fn syntax_error(bytes: &[u8]) -> Option<String> {
    let mut stack: Vec<Frame> = Vec::new();
    let mut state = State::Value;
    let mut i = 0;
    loop {
        // Skip whitespace between tokens.
        while matches!(bytes.get(i), Some(b' ' | b'\t' | b'\r' | b'\n')) {
            i += 1;
        }
        let Some(&byte) = bytes.get(i) else {
            return if matches!(state, State::After) && stack.is_empty() {
                None
            } else {
                Some(EOF.into())
            };
        };
        match state {
            State::ArrayFirst if byte == b']' => {
                i += 1;
                stack.pop();
                state = State::After;
            }
            State::Value | State::ArrayFirst => match byte {
                b'{' | b'[' => {
                    if stack.len() >= MAX_DEPTH {
                        return Some("exceeded max depth".into());
                    }
                    i += 1;
                    if byte == b'{' {
                        stack.push(Frame::Object);
                        state = State::ObjectFirst;
                    } else {
                        stack.push(Frame::Array);
                        state = State::ArrayFirst;
                    }
                }
                _ => {
                    if let Err(message) = scalar(bytes, &mut i) {
                        return Some(message);
                    }
                    state = State::After;
                }
            },
            State::ObjectFirst if byte == b'}' => {
                i += 1;
                stack.pop();
                state = State::After;
            }
            State::ObjectFirst | State::ObjectKey => {
                if byte != b'"' {
                    return Some(invalid(byte, "looking for beginning of object key string"));
                }
                if let Err(message) = string(bytes, &mut i) {
                    return Some(message);
                }
                state = State::Colon;
            }
            State::Colon => {
                if byte != b':' {
                    return Some(invalid(byte, "after object key"));
                }
                i += 1;
                state = State::Value;
            }
            State::After => match (stack.last(), byte) {
                (None, _) => return Some(invalid(byte, "after top-level value")),
                (Some(Frame::Object), b',') => {
                    i += 1;
                    state = State::ObjectKey;
                }
                (Some(Frame::Object), b'}') | (Some(Frame::Array), b']') => {
                    i += 1;
                    stack.pop();
                }
                (Some(Frame::Object), _) => {
                    return Some(invalid(byte, "after object key:value pair"));
                }
                (Some(Frame::Array), b',') => {
                    i += 1;
                    state = State::Value;
                }
                (Some(Frame::Array), _) => return Some(invalid(byte, "after array element")),
            },
        }
    }
}

/// Consume a string, number, or literal starting at `bytes[*i]`.
fn scalar(bytes: &[u8], i: &mut usize) -> Result<(), String> {
    let byte = bytes[*i];
    match byte {
        b'"' => string(bytes, i),
        b'-' | b'0'..=b'9' => number(bytes, i),
        b't' => literal(bytes, i, "true"),
        b'f' => literal(bytes, i, "false"),
        b'n' => literal(bytes, i, "null"),
        _ => Err(invalid(byte, "looking for beginning of value")),
    }
}

fn literal(bytes: &[u8], i: &mut usize, word: &str) -> Result<(), String> {
    for (offset, expected) in word.bytes().enumerate().skip(1) {
        match bytes.get(*i + offset) {
            None => return Err(EOF.into()),
            Some(&byte) if byte == expected => {}
            Some(&byte) => {
                return Err(invalid(
                    byte,
                    &format!("in literal {word} (expecting {})", quote_char(expected)),
                ))
            }
        }
    }
    *i += word.len();
    Ok(())
}

fn string(bytes: &[u8], i: &mut usize) -> Result<(), String> {
    *i += 1;
    loop {
        let Some(&byte) = bytes.get(*i) else {
            return Err(EOF.into());
        };
        *i += 1;
        match byte {
            b'"' => return Ok(()),
            b'\\' => {
                let Some(&escape) = bytes.get(*i) else {
                    return Err(EOF.into());
                };
                *i += 1;
                match escape {
                    b'b' | b'f' | b'n' | b'r' | b't' | b'\\' | b'/' | b'"' => {}
                    b'u' => {
                        for _ in 0..4 {
                            let Some(&hex) = bytes.get(*i) else {
                                return Err(EOF.into());
                            };
                            if !hex.is_ascii_hexdigit() {
                                return Err(invalid(hex, "in \\u hexadecimal character escape"));
                            }
                            *i += 1;
                        }
                    }
                    _ => return Err(invalid(escape, "in string escape code")),
                }
            }
            0x00..=0x1f => return Err(invalid(byte, "in string literal")),
            _ => {}
        }
    }
}

fn number(bytes: &[u8], i: &mut usize) -> Result<(), String> {
    let digits = |i: &mut usize| {
        while matches!(bytes.get(*i), Some(b'0'..=b'9')) {
            *i += 1;
        }
    };
    if bytes[*i] == b'-' {
        *i += 1;
        match bytes.get(*i) {
            None => return Err(EOF.into()),
            Some(b'0'..=b'9') => {}
            Some(&byte) => return Err(invalid(byte, "in numeric literal")),
        }
    }
    if bytes[*i] == b'0' {
        *i += 1;
    } else {
        digits(i);
    }
    if bytes.get(*i) == Some(&b'.') {
        *i += 1;
        match bytes.get(*i) {
            None => return Err(EOF.into()),
            Some(b'0'..=b'9') => digits(i),
            Some(&byte) => return Err(invalid(byte, "after decimal point in numeric literal")),
        }
    }
    if matches!(bytes.get(*i), Some(b'e' | b'E')) {
        *i += 1;
        if matches!(bytes.get(*i), Some(b'+' | b'-')) {
            *i += 1;
        }
        match bytes.get(*i) {
            None => return Err(EOF.into()),
            Some(b'0'..=b'9') => digits(i),
            Some(&byte) => return Err(invalid(byte, "in exponent of numeric literal")),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Deserialize)]
    struct Params {
        #[allow(dead_code)]
        email: Option<String>,
        #[allow(dead_code)]
        password: Option<String>,
        #[allow(dead_code)]
        data: Option<serde_json::Map<String, Value>>,
    }

    fn message(body: &str) -> String {
        let Err(error) = decode::<Params>(body.as_bytes(), &SIGNUP_PARAMS) else {
            panic!("{body} should fail");
        };
        error.message
    }

    const PREFIX: &str = "Could not parse request body as JSON: ";

    #[test]
    fn syntax_errors_use_go_wording() {
        assert_eq!(
            message("not-json"),
            format!("{PREFIX}invalid character 'o' in literal null (expecting 'u')")
        );
        assert_eq!(message(""), format!("{PREFIX}unexpected end of JSON input"));
        assert_eq!(
            message("{\"a\":1"),
            format!("{PREFIX}unexpected end of JSON input")
        );
        assert_eq!(
            message("x"),
            format!("{PREFIX}invalid character 'x' looking for beginning of value")
        );
        assert_eq!(
            message("{} 1"),
            format!("{PREFIX}invalid character '1' after top-level value")
        );
        assert_eq!(
            message("{1:2}"),
            format!("{PREFIX}invalid character '1' looking for beginning of object key string")
        );
        assert_eq!(
            message("{\"a\" 1}"),
            format!("{PREFIX}invalid character '1' after object key")
        );
        assert_eq!(
            message("{\"a\":1 2}"),
            format!("{PREFIX}invalid character '2' after object key:value pair")
        );
        assert_eq!(
            message("[1 2]"),
            format!("{PREFIX}invalid character '2' after array element")
        );
        assert_eq!(
            message("{\"a\":\"\\x\"}"),
            format!("{PREFIX}invalid character 'x' in string escape code")
        );
        assert_eq!(
            message("{\"a\":\"\\u12g4\"}"),
            format!("{PREFIX}invalid character 'g' in \\u hexadecimal character escape")
        );
        assert_eq!(
            message("{\"a\":\"\n\"}"),
            format!("{PREFIX}invalid character '\\n' in string literal")
        );
        assert_eq!(
            message("-x"),
            format!("{PREFIX}invalid character 'x' in numeric literal")
        );
        assert_eq!(
            message("1.x"),
            format!("{PREFIX}invalid character 'x' after decimal point in numeric literal")
        );
        assert_eq!(
            message("1ex"),
            format!("{PREFIX}invalid character 'x' in exponent of numeric literal")
        );
        assert_eq!(
            message("tru!"),
            format!("{PREFIX}invalid character '!' in literal true (expecting 'e')")
        );
    }

    #[test]
    fn deep_nesting_is_panic_free() {
        let body = "[".repeat(20_000);
        assert_eq!(
            syntax_error(body.as_bytes()).as_deref(),
            Some("exceeded max depth")
        );
        assert_eq!(syntax_error(b"[[[]]]"), None);
        assert_eq!(
            syntax_error(b" {\"a\": [1, -2.5e+3, true, null, \"\\u00e9\"]} "),
            None
        );
    }

    #[test]
    fn type_errors_name_the_go_struct_field() {
        assert_eq!(
            message(r#"{"email":["a@example.com"],"password":"x"}"#),
            format!(
                "{PREFIX}json: cannot unmarshal array into Go struct field SignupParams.email of type string"
            )
        );
        assert_eq!(
            message(r#"{"email":"a@example.com","password":1}"#),
            format!(
                "{PREFIX}json: cannot unmarshal number into Go struct field SignupParams.password of type string"
            )
        );
        assert_eq!(
            message(r#"{"data":"x"}"#),
            format!(
                "{PREFIX}json: cannot unmarshal string into Go struct field SignupParams.data of type map[string]interface {{}}"
            )
        );
        assert_eq!(
            message("[]"),
            format!("{PREFIX}json: cannot unmarshal array into Go value of type api.SignupParams")
        );
    }

    #[test]
    fn valid_body_decodes() {
        let params: Params = decode(br#"{"email":"a@example.com"}"#, &SIGNUP_PARAMS)
            .unwrap_or_else(|_| panic!("valid body"));
        assert_eq!(params.email.as_deref(), Some("a@example.com"));
    }

    #[test]
    fn keys_match_fields_case_insensitively_last_wins() {
        let decode_email = |body: &str| {
            decode::<Params>(body.as_bytes(), &SIGNUP_PARAMS)
                .unwrap_or_else(|_| panic!("{body} is valid"))
                .email
        };
        assert_eq!(
            decode_email(r#"{"EMAIL":"a@x.co"}"#).as_deref(),
            Some("a@x.co")
        );
        assert_eq!(
            decode_email(r#"{"email":"first","Email":"second"}"#).as_deref(),
            Some("second")
        );
        assert_eq!(
            decode_email(r#"{"Email":"first","email":"second"}"#).as_deref(),
            Some("second")
        );
    }

    #[test]
    fn null_duplicate_keeps_earlier_string() {
        let body = br#"{"email":"a@x.co","Email":null}"#;
        let params =
            decode::<Params>(body, &SIGNUP_PARAMS).unwrap_or_else(|_| panic!("body is valid"));
        assert_eq!(params.email.as_deref(), Some("a@x.co"));
    }

    #[test]
    fn earlier_type_error_survives_later_valid_duplicate() {
        for body in [
            r#"{"Email":1,"email":"a@x.co"}"#,
            r#"{"email":1,"EMAIL":"a@x.co"}"#,
        ] {
            assert_eq!(
                message(body),
                format!(
                    "{PREFIX}json: cannot unmarshal number into Go struct field SignupParams.email of type string"
                )
            );
        }
    }

    #[test]
    fn case_variant_key_reports_go_type_error() {
        assert_eq!(
            message(r#"{"Email":1}"#),
            format!(
                "{PREFIX}json: cannot unmarshal number into Go struct field SignupParams.email of type string"
            )
        );
    }
}
