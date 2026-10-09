//! Normalization applied identically to both responses before comparison.
//! Every rule here is documented in `judge/NORMALIZATION.md`; keep them in sync.

use std::sync::OnceLock;

use regex::Regex;
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

fn rules() -> &'static [(Regex, &'static str)] {
    static RULES: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();
    RULES.get_or_init(|| {
        [
            (
                r"eyJ[A-Za-z0-9_-]*\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+",
                "<jwt>",
            ),
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
    let mut out = s.to_string();
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
    walk(value);
}

fn walk(value: &mut Value) {
    match value {
        Value::String(s) => *s = text(s),
        Value::Array(items) => items.iter_mut().for_each(walk),
        Value::Object(map) => {
            for (key, v) in map.iter_mut() {
                if VOLATILE_KEYS.contains(&key.as_str()) && !v.is_null() {
                    *v = Value::String(format!("<{key}>"));
                } else {
                    walk(v);
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
            "Bearer <jwt>"
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
}
