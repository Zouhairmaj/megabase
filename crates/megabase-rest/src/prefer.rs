// Ported from postgrest src/library/PostgREST/ApiRequest/Preferences.hs (MIT), pin v16.4.

//! The `Prefer` request header and the `Preference-Applied` response header.
//!
//! Served: `return=*`, `timezone=*`, and `tx=*`. `tx=*` is only honoured when
//! `PGRST_DB_TX_END` allows an override. The pinned stack leaves it at
//! `commit`, so PostgREST parses the token, ignores it, and does not echo it.
//! Every other preference answers `501`.

use axum::http::{HeaderMap, HeaderName, HeaderValue};
use axum::response::Response;

/// `return=` preference (`PreferRepresentation`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Representation {
    /// `return=representation`.
    Full,
    /// `return=headers-only`.
    HeadersOnly,
    /// `return=minimal`.
    Minimal,
}

impl Representation {
    fn token(self) -> &'static str {
        match self {
            Self::Full => "return=representation",
            Self::HeadersOnly => "return=headers-only",
            Self::Minimal => "return=minimal",
        }
    }
}

/// Preferences of one request. The first occurrence of each wins.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Preferences {
    pub(crate) representation: Option<Representation>,
    pub(crate) timezone: Option<String>,
}

impl Preferences {
    /// Parse every `Prefer` header. Tokens are comma-separated and trimmed.
    /// Matching is case-sensitive, like `Preferences.fromHeaders`.
    pub(crate) fn from_headers(headers: &HeaderMap) -> Self {
        // megabase:unit rest:prefer:timezone=*
        // megabase:unit rest:prefer:tx=commit
        // megabase:unit rest:prefer:tx=rollback
        // `tx=` is parsed only when `db-tx-end` allows an override, which the
        // pinned stack does not. No field, so it is never applied or echoed.
        let mut out = Self::default();
        for token in tokens(headers) {
            if out.representation.is_none() {
                out.representation = match token {
                    "return=representation" => Some(Representation::Full),
                    "return=headers-only" => Some(Representation::HeadersOnly),
                    "return=minimal" => Some(Representation::Minimal),
                    _ => None,
                };
            }
            if out.timezone.is_none() {
                out.timezone = token.strip_prefix("timezone=").map(str::to_string);
            }
        }
        out
    }

    /// `Preference-Applied` value. `return=` only applies to mutations.
    pub(crate) fn applied(&self, mutation: bool) -> Option<String> {
        let mut parts = Vec::new();
        if mutation {
            if let Some(representation) = self.representation {
                parts.push(representation.token().to_string());
            }
        }
        if let Some(timezone) = &self.timezone {
            parts.push(format!("timezone={timezone}"));
        }
        if parts.is_empty() {
            None
        } else {
            Some(parts.join(", "))
        }
    }

    /// Add `Preference-Applied` to a successful response.
    pub(crate) fn stamp(&self, response: &mut Response, mutation: bool) {
        if !response.status().is_success() {
            return;
        }
        let Some(value) = self.applied(mutation) else {
            return;
        };
        if let Ok(value) = HeaderValue::from_str(&value) {
            response
                .headers_mut()
                .insert(HeaderName::from_static("preference-applied"), value);
        }
    }
}

fn tokens(headers: &HeaderMap) -> impl Iterator<Item = &str> {
    headers
        .get_all(HeaderName::from_static("prefer"))
        .into_iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(str::trim)
        .filter(|token| !token.is_empty())
}

/// Unit id of the first preference that is not served yet, if any.
pub(crate) fn unserved(headers: &HeaderMap) -> Option<&'static str> {
    for token in tokens(headers) {
        let lower = token.to_ascii_lowercase();
        let unit = match lower.as_str() {
            "count=exact" => "rest:prefer:count=exact",
            "count=estimated" => "rest:prefer:count=estimated",
            "count=planned" => "rest:prefer:count=planned",
            "handling=lenient" => "rest:prefer:handling=lenient",
            "handling=strict" => "rest:prefer:handling=strict",
            "missing=default" => "rest:prefer:missing=default",
            "missing=null" => "rest:prefer:missing=null",
            "resolution=ignore-duplicates" => "rest:prefer:resolution=ignore-duplicates",
            "resolution=merge-duplicates" => "rest:prefer:resolution=merge-duplicates",
            _ if lower.starts_with("max-affected=") => "rest:prefer:max-affected=*",
            _ => continue,
        };
        return Some(unit);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(values: &[&str]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for value in values {
            headers.append("prefer", HeaderValue::from_str(value).expect("header"));
        }
        headers
    }

    #[test]
    fn first_return_and_timezone_win_across_headers() {
        let prefs = Preferences::from_headers(&map(&[
            "tx=commit,  return=headers-only , timezone=America/Los_Angeles",
            "return=representation, timezone=UTC",
        ]));
        assert_eq!(prefs.representation, Some(Representation::HeadersOnly));
        assert_eq!(prefs.timezone.as_deref(), Some("America/Los_Angeles"));
        assert_eq!(
            prefs.applied(true).as_deref(),
            Some("return=headers-only, timezone=America/Los_Angeles")
        );
        assert_eq!(
            prefs.applied(false).as_deref(),
            Some("timezone=America/Los_Angeles")
        );
    }

    #[test]
    fn tx_and_unknown_tokens_are_ignored_and_not_echoed() {
        let prefs = Preferences::from_headers(&map(&["tx=rollback, tx=commit, anything"]));
        assert_eq!(prefs, Preferences::default());
        assert_eq!(prefs.applied(true), None);
        assert_eq!(unserved(&map(&["tx=rollback, anything"])), None);
    }

    #[test]
    fn matching_is_case_sensitive_for_served_tokens() {
        let prefs = Preferences::from_headers(&map(&["Return=minimal"]));
        assert_eq!(prefs.representation, None);
    }

    #[test]
    fn unserved_scans_every_token() {
        assert_eq!(
            unserved(&map(&["return=minimal, count=exact"])),
            Some("rest:prefer:count=exact")
        );
        assert_eq!(
            unserved(&map(&["timezone=UTC", "max-affected=5"])),
            Some("rest:prefer:max-affected=*")
        );
        assert_eq!(unserved(&map(&["return=minimal"])), None);
    }
}
