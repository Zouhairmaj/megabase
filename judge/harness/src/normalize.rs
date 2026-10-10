//! Normalization applied identically to both responses before comparison.
//! Every rule here is documented in `judge/NORMALIZATION.md`; keep them in sync.

use std::sync::OnceLock;

use base64::Engine;
use regex::{Captures, Regex};
use serde_json::Value;

/// Response headers compared on every step. All others (date, server, via,
/// x-kong-*, content-length, ...) depend on the gateway, not the API.
pub const DEFAULT_HEADERS: &[&str] = &[
    "content-type",
    "content-range",
    "location",
    "preference-applied",
];

/// JSON keys whose values are generated per request and carry no contract
/// beyond their presence and type.
const VOLATILE_KEYS: &[&str] = &["expires_at", "iat", "exp", "refresh_token"];

/// `auth.users` columns whose *contents* are secrets or per-insert hashes.
/// Empty strings stay empty so an autoconfirmed sign-up that cleared a
/// token still differs from one that left a random value.
const AUTH_USER_SECRET_KEYS: &[&str] = &[
    "encrypted_password",
    "confirmation_token",
    "recovery_token",
    "email_change_token",
    "email_change_token_new",
    "email_change_token_current",
    "reauthentication_token",
    "phone_change_token",
];

/// Compact JWT: header, payload (captured), signature.
const JWT_PATTERN: &str = r"eyJ[A-Za-z0-9_-]*\.([A-Za-z0-9_-]+)\.[A-Za-z0-9_-]+";

/// Claims generated per token that carry no contract.
const VOLATILE_CLAIMS: &[&str] = &["iat", "exp", "jti"];

fn jwt_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(JWT_PATTERN).expect("valid jwt regex"))
}

/// Renders a JWT as its decoded claims minus `iat`, `exp` and `jti`, so two
/// tokens compare equal exactly when every other claim matches. The
/// signature is not checked. A payload that is not a JSON object becomes
/// `<jwt>`.
fn jwt_claims(token: &Captures<'_>) -> String {
    let claims = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(&token[1])
        .ok()
        .and_then(|raw| serde_json::from_slice::<Value>(&raw).ok());
    match claims {
        Some(Value::Object(mut map)) => {
            for key in VOLATILE_CLAIMS {
                map.remove(*key);
            }
            // Each `amr` entry carries the session's creation time; keep its
            // method and structure.
            if let Some(Value::Array(entries)) = map.get_mut("amr") {
                for entry in entries {
                    if let Value::Object(entry) = entry {
                        entry.remove("timestamp");
                    }
                }
            }
            format!("<jwt:{}>", Value::Object(map))
        }
        _ => "<jwt>".to_string(),
    }
}

fn rules() -> &'static [(Regex, &'static str)] {
    static RULES: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();
    RULES.get_or_init(|| {
        [
            (r"(?i)\$2[abxy]?\$\d{2}\$[A-Za-z0-9./]{53}", "<bcrypt>"),
            (
                r"\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}:\d{2}(\.\d+)?(Z|[+-]\d{2}(:?\d{2})?)?",
                "<timestamp>",
            ),
            (
                r"(?i)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}",
                "<uuid>",
            ),
        ]
        .into_iter()
        .map(|(re, to)| (Regex::new(re).expect("valid normalization regex"), to))
        .collect()
    })
}

pub fn text(s: &str) -> String {
    let mut out = jwt_regex().replace_all(s, jwt_claims).into_owned();
    for (re, to) in rules() {
        out = re.replace_all(&out, *to).into_owned();
    }
    out
}

pub fn header(name: &str, value: &str) -> String {
    let value = text(value.trim());
    if name.eq_ignore_ascii_case("content-type") {
        value.to_ascii_lowercase().replace(' ', "")
    } else {
        value
    }
}

pub fn json(value: &mut Value, ignore: &[String]) {
    for pointer in ignore {
        remove_pointer(value, pointer);
    }
    walk(value, false);
}

/// Same as [`json`], plus `auth.users` secret columns and a stable row
/// order. Used for database snapshots, not HTTP bodies.
pub fn json_rows(value: &mut Value) {
    walk(value, true);
    if let Value::Array(items) = value {
        items.sort_by_cached_key(Value::to_string);
    }
}

fn walk(value: &mut Value, secrets: bool) {
    match value {
        Value::String(s) => *s = text(s),
        Value::Array(items) => items.iter_mut().for_each(|v| walk(v, secrets)),
        Value::Object(map) => {
            for (key, v) in map.iter_mut() {
                let replace_volatile = VOLATILE_KEYS.contains(&key.as_str()) && !v.is_null();
                let replace_secret = secrets
                    && AUTH_USER_SECRET_KEYS.contains(&key.as_str())
                    && matches!(v, Value::String(s) if !s.is_empty());
                if replace_volatile || replace_secret {
                    *v = Value::String(format!("<{key}>"));
                } else {
                    walk(v, secrets);
                }
            }
        }
        _ => {}
    }
}

fn remove_pointer(value: &mut Value, pointer: &str) {
    let Some((parent, last)) = pointer.rsplit_once('/') else {
        return;
    };
    let last = last.replace("~1", "/").replace("~0", "~");
    match value.pointer_mut(parent) {
        Some(Value::Object(map)) => {
            map.remove(&last);
        }
        Some(Value::Array(items)) => {
            if let Ok(i) = last.parse::<usize>() {
                if i < items.len() {
                    items.remove(i);
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn replaces_generated_values() {
        assert_eq!(
            text("at 2026-10-09T09:44:01.123456Z id 0b6a1f8e-3f43-4b5e-9c1d-2a0e5f6b7c8d"),
            "at <timestamp> id <uuid>"
        );
        assert_eq!(
            text("Bearer eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.abc-_x"),
            r#"Bearer <jwt:{"sub":"1"}>"#
        );
        assert_eq!(
            text("$2a$10$N9qo8uLOickgx2ZMRZoMyeIjZAgcfl7p92ldGxad68LJZdL17lhWy"),
            "<bcrypt>"
        );
    }

    fn token(claims: &Value) -> String {
        let enc = |v: &[u8]| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(v);
        format!(
            "{}.{}.sig",
            enc(br#"{"alg":"HS256","typ":"JWT"}"#),
            enc(claims.to_string().as_bytes())
        )
    }

    #[test]
    fn jwt_claim_differences_fail() {
        let base = json!({
            "role": "anon", "aud": "authenticated", "sub": "u1",
            "aal": "aal1", "amr": ["password"]
        });
        let reference = text(&token(&base));
        for (claim, other) in [
            ("role", json!("service_role")),
            ("aud", json!("other")),
            ("sub", json!("u2")),
            ("aal", json!("aal2")),
            ("amr", json!(["otp"])),
            ("email", json!("a@example.com")),
        ] {
            let mut changed = base.clone();
            changed[claim] = other;
            assert_ne!(text(&token(&changed)), reference, "{claim}");
        }
    }

    #[test]
    fn jwt_volatile_claims_do_not_matter() {
        let a = json!({"role": "anon", "sub": "u1", "iat": 1, "exp": 2, "jti": "a"});
        let b = json!({"role": "anon", "sub": "u1", "iat": 10, "exp": 20, "jti": "b"});
        assert_eq!(text(&token(&a)), text(&token(&b)));
        let bare = json!({"role": "anon", "sub": "u1"});
        assert_eq!(text(&token(&a)), text(&token(&bare)));
    }

    #[test]
    fn jwt_amr_timestamps_do_not_matter_but_methods_do() {
        let amr = |method: &str, ts: i64| {
            json!({"role": "anon", "amr": [{"method": method, "timestamp": ts}]})
        };
        assert_eq!(
            text(&token(&amr("password", 100))),
            text(&token(&amr("password", 101)))
        );
        assert_ne!(
            text(&token(&amr("password", 100))),
            text(&token(&amr("otp", 100)))
        );
    }

    #[test]
    fn volatile_keys_and_pointers() {
        let mut v =
            json!({"expires_at": 17, "user": {"id": "x", "keep": 1}, "refresh_token": null});
        json(&mut v, &["/user/id".to_string()]);
        assert_eq!(
            v,
            json!({"expires_at": "<expires_at>", "user": {"keep": 1}, "refresh_token": null})
        );
    }

    #[test]
    fn content_type_is_case_and_space_insensitive() {
        assert_eq!(
            header("Content-Type", "application/json; charset=utf-8"),
            "application/json;charset=utf-8"
        );
        assert_eq!(header("location", " /x "), "/x");
    }

    #[test]
    fn json_walks_arrays_and_volatile_keys() {
        let mut v = json!({
            "iat": 1,
            "exp": 2,
            "rows": [{"id": "0b6a1f8e-3f43-4b5e-9c1d-2a0e5f6b7c8d"}, 3],
            "keep": null
        });
        json(
            &mut v,
            &[
                "/rows/1".to_string(),
                "/missing".to_string(),
                "no-slash".to_string(),
            ],
        );
        assert_eq!(v["iat"], "<iat>");
        assert_eq!(v["exp"], "<exp>");
        assert_eq!(v["rows"][0]["id"], "<uuid>");
        assert_eq!(v["rows"].as_array().unwrap().len(), 1);
        assert!(v["keep"].is_null());
    }

    #[test]
    fn database_rows_normalize_secrets_and_order() {
        let mut a = json!([
            {
                "instance_id": "00000000-0000-0000-0000-000000000000",
                "email": "b@example.com",
                "encrypted_password": "$2a$10$N9qo8uLOickgx2ZMRZoMyeIjZAgcfl7p92ldGxad68LJZdL17lhWy",
                "confirmation_token": ""
            },
            {
                "instance_id": "00000000-0000-0000-0000-000000000000",
                "email": "a@example.com",
                "encrypted_password": "$2b$10$N9qo8uLOickgx2ZMRZoMyeIjZAgcfl7p92ldGxad68LJZdL17lhWz",
                "confirmation_token": "abc123"
            }
        ]);
        let mut b = json!([
            {
                "instance_id": "11111111-1111-1111-1111-111111111111",
                "email": "a@example.com",
                "encrypted_password": "$2a$10$otherhashotherhashotherhashotherhashotherhashe",
                "confirmation_token": "zzzzzz"
            },
            {
                "instance_id": "11111111-1111-1111-1111-111111111111",
                "email": "b@example.com",
                "encrypted_password": "$2y$12$N9qo8uLOickgx2ZMRZoMyeIjZAgcfl7p92ldGxad68LJZdL17lhWy",
                "confirmation_token": ""
            }
        ]);
        json_rows(&mut a);
        json_rows(&mut b);
        assert_eq!(a, b);
        let empty_token = a
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["email"] == "b@example.com")
            .unwrap();
        let leftover_token = a
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["email"] == "a@example.com")
            .unwrap();
        assert_eq!(empty_token["confirmation_token"], "");
        assert_eq!(leftover_token["confirmation_token"], "<confirmation_token>");
        assert_eq!(empty_token["encrypted_password"], "<encrypted_password>");
    }
}
