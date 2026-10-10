// Ported from postgrest src/library/PostgREST/ApiRequest/Preferences.hs (MIT), pin v16.4,
// and postgrest src/library/PostgREST/MainTx.hs (MIT), pin v16.4.

//! The `Prefer` request header and the `Preference-Applied` response header.
//!
//! See `specs/rest/prefer.md`. Matching is exact and case-sensitive, like
//! upstream's lookup table. The first recognised value of a kind wins, across
//! all `Prefer` headers.

use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use axum::response::Response;

use crate::read::pgrst;

pub(crate) const PREFERENCE_APPLIED: HeaderName = HeaderName::from_static("preference-applied");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Resolution {
    Merge,
    Ignore,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Count {
    Exact,
    Planned,
    Estimated,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Missing {
    Default,
    Null,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Handling {
    Strict,
    Lenient,
}

/// `return=` preference (`PreferRepresentation`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Representation {
    /// `return=representation`.
    Full,
    /// `return=headers-only`.
    HeadersOnly,
    /// `return=minimal`.
    Minimal,
}

/// Parsed preferences.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Prefer {
    pub(crate) resolution: Option<Resolution>,
    pub(crate) count: Option<Count>,
    pub(crate) missing: Option<Missing>,
    pub(crate) handling: Option<Handling>,
    pub(crate) max_affected: Option<i64>,
    pub(crate) representation: Option<Representation>,
    pub(crate) timezone: Option<String>,
    invalid: Vec<String>,
}

impl Prefer {
    /// Parse every `Prefer` header (`Preferences.fromHeaders`).
    pub(crate) fn from_headers(headers: &HeaderMap) -> Self {
        let mut out = Self::default();
        for value in headers.get_all("prefer") {
            let Ok(text) = value.to_str() else { continue };
            for part in text.split(',') {
                out.take(part.trim());
            }
        }
        out
    }

    fn take(&mut self, pref: &str) {
        match pref {
            "resolution=merge-duplicates" => {
                self.resolution.get_or_insert(Resolution::Merge);
            }
            "resolution=ignore-duplicates" => {
                self.resolution.get_or_insert(Resolution::Ignore);
            }
            "count=exact" => {
                self.count.get_or_insert(Count::Exact);
            }
            "count=planned" => {
                self.count.get_or_insert(Count::Planned);
            }
            "count=estimated" => {
                self.count.get_or_insert(Count::Estimated);
            }
            "missing=default" => {
                self.missing.get_or_insert(Missing::Default);
            }
            "missing=null" => {
                self.missing.get_or_insert(Missing::Null);
            }
            "handling=strict" => {
                self.handling.get_or_insert(Handling::Strict);
            }
            "handling=lenient" => {
                self.handling.get_or_insert(Handling::Lenient);
            }
            "return=minimal" => {
                self.representation.get_or_insert(Representation::Minimal);
            }
            "return=representation" => {
                self.representation.get_or_insert(Representation::Full);
            }
            "return=headers-only" => {
                self.representation
                    .get_or_insert(Representation::HeadersOnly);
            }
            // megabase:unit rest:prefer:tx=commit
            // megabase:unit rest:prefer:tx=rollback
            // PostgREST parses `tx=` only when `db-tx-end` allows an override,
            // which the pinned stack does not. Ignored and never echoed.
            "tx=commit" | "tx=rollback" => {}
            // megabase:unit rest:prefer:timezone=*
            _ if pref.starts_with("timezone=") => {
                if self.timezone.is_none() {
                    self.timezone = Some(pref["timezone=".len()..].to_string());
                }
            }
            _ if pref.starts_with("max-affected=") => {
                if self.max_affected.is_none() {
                    self.max_affected = pref["max-affected=".len()..].parse().ok();
                }
            }
            "" => {}
            other => self.invalid.push(other.to_string()),
        }
    }

    // megabase:unit rest:prefer:handling=strict
    // megabase:unit rest:prefer:handling=lenient
    /// `PGRST122` when `handling=strict` meets an unrecognised preference.
    pub(crate) fn check_strict(&self) -> Result<(), Box<Response>> {
        if self.invalid.is_empty() || self.handling != Some(Handling::Strict) {
            return Ok(());
        }
        Err(Box::new(pgrst(
            StatusCode::BAD_REQUEST,
            "PGRST122",
            "Invalid preferences given with handling=strict",
            Some(&format!("Invalid preferences: {}", self.invalid.join(", "))),
            None,
        )))
    }

    /// Unit id of the first preference that RPC routes do not serve yet.
    pub(crate) fn rpc_unserved(&self) -> Option<&'static str> {
        match self {
            Self {
                resolution: Some(Resolution::Merge),
                ..
            } => Some("rest:prefer:resolution=merge-duplicates"),
            Self {
                resolution: Some(Resolution::Ignore),
                ..
            } => Some("rest:prefer:resolution=ignore-duplicates"),
            Self {
                count: Some(Count::Exact),
                ..
            } => Some("rest:prefer:count=exact"),
            Self {
                count: Some(Count::Planned),
                ..
            } => Some("rest:prefer:count=planned"),
            Self {
                count: Some(Count::Estimated),
                ..
            } => Some("rest:prefer:count=estimated"),
            Self {
                missing: Some(Missing::Default),
                ..
            } => Some("rest:prefer:missing=default"),
            Self {
                missing: Some(Missing::Null),
                ..
            } => Some("rest:prefer:missing=null"),
            Self {
                handling: Some(Handling::Strict),
                ..
            } => Some("rest:prefer:handling=strict"),
            Self {
                handling: Some(Handling::Lenient),
                ..
            } => Some("rest:prefer:handling=lenient"),
            Self {
                max_affected: Some(_),
                ..
            } => Some("rest:prefer:max-affected=*"),
            _ => None,
        }
    }

    /// `max-affected` only binds under `handling=strict`.
    pub(crate) fn strict_max_affected(&self) -> Option<i64> {
        if self.handling == Some(Handling::Strict) {
            self.max_affected
        } else {
            None
        }
    }

    /// `Preference-Applied` value (`prefAppliedHeader`). The caller says which
    /// kinds the route applies.
    pub(crate) fn applied(&self, scope: Scope) -> Option<HeaderValue> {
        let mut out: Vec<String> = Vec::new();
        if let (Some(resolution), true) = (self.resolution, scope.resolution) {
            out.push(
                match resolution {
                    Resolution::Merge => "resolution=merge-duplicates",
                    Resolution::Ignore => "resolution=ignore-duplicates",
                }
                .to_string(),
            );
        }
        if let (Some(missing), true) = (self.missing, scope.missing) {
            out.push(
                match missing {
                    Missing::Default => "missing=default",
                    Missing::Null => "missing=null",
                }
                .to_string(),
            );
        }
        if let (Some(representation), true) = (self.representation, scope.representation) {
            out.push(
                match representation {
                    Representation::Full => "return=representation",
                    Representation::HeadersOnly => "return=headers-only",
                    Representation::Minimal => "return=minimal",
                }
                .to_string(),
            );
        }
        if let (Some(count), true) = (self.count, scope.count) {
            out.push(
                match count {
                    Count::Exact => "count=exact",
                    Count::Planned => "count=planned",
                    Count::Estimated => "count=estimated",
                }
                .to_string(),
            );
        }
        if let Some(handling) = self.handling {
            out.push(
                match handling {
                    Handling::Strict => "handling=strict",
                    Handling::Lenient => "handling=lenient",
                }
                .to_string(),
            );
        }
        if let Some(timezone) = &self.timezone {
            out.push(format!("timezone={timezone}"));
        }
        if let (Some(n), true, Some(Handling::Strict)) =
            (self.max_affected, scope.max_affected, self.handling)
        {
            out.push(format!("max-affected={n}"));
        }
        if out.is_empty() {
            return None;
        }
        HeaderValue::from_str(&out.join(", ")).ok()
    }
}

/// Which preference kinds a response applied.
#[derive(Clone, Copy, Default)]
pub(crate) struct Scope {
    pub(crate) resolution: bool,
    pub(crate) missing: bool,
    pub(crate) count: bool,
    pub(crate) max_affected: bool,
    /// `return=` only applies to mutations.
    pub(crate) representation: bool,
}

/// Add `Preference-Applied` when `value` is set.
pub(crate) fn set_applied(response: &mut Response, value: Option<HeaderValue>) {
    if let Some(value) = value {
        response.headers_mut().insert(PREFERENCE_APPLIED, value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(values: &[&str]) -> Prefer {
        let mut headers = HeaderMap::new();
        for value in values {
            headers.append("prefer", HeaderValue::from_str(value).unwrap());
        }
        Prefer::from_headers(&headers)
    }

    #[test]
    fn first_value_of_a_kind_wins_across_headers() {
        let prefer = parse(&[
            "resolution=ignore-duplicates",
            "count=planned, resolution=merge-duplicates, count=exact",
        ]);
        assert_eq!(prefer.resolution, Some(Resolution::Ignore));
        assert_eq!(prefer.count, Some(Count::Planned));
    }

    #[test]
    fn spaces_and_unknowns() {
        let prefer = parse(&["count=exact,    missing=default   , handling=strict, anything"]);
        assert_eq!(prefer.count, Some(Count::Exact));
        assert_eq!(prefer.missing, Some(Missing::Default));
        assert_eq!(prefer.handling, Some(Handling::Strict));
        assert!(prefer.check_strict().is_err());
    }

    #[test]
    fn lenient_and_default_ignore_unknowns() {
        assert!(parse(&["anything, handling=lenient"])
            .check_strict()
            .is_ok());
        assert!(parse(&["anything"]).check_strict().is_ok());
    }

    #[test]
    fn matching_is_case_sensitive() {
        assert_eq!(parse(&["COUNT=EXACT"]).count, None);
    }

    #[test]
    fn max_affected_needs_strict_and_a_number() {
        assert_eq!(parse(&["max-affected=5"]).strict_max_affected(), None);
        assert_eq!(
            parse(&["handling=strict, max-affected=5"]).strict_max_affected(),
            Some(5)
        );
        assert_eq!(
            parse(&["handling=strict, max-affected=x"]).strict_max_affected(),
            None
        );
    }

    #[test]
    fn return_and_timezone_first_wins_and_tx_is_ignored() {
        let prefer = parse(&[
            "tx=rollback,  return=headers-only , timezone=America/Los_Angeles",
            "return=representation, timezone=UTC",
        ]);
        assert_eq!(prefer.representation, Some(Representation::HeadersOnly));
        assert_eq!(prefer.timezone.as_deref(), Some("America/Los_Angeles"));
        assert_eq!(
            prefer.applied(Scope::default()).unwrap(),
            "timezone=America/Los_Angeles"
        );
        let all = Scope {
            representation: true,
            ..Scope::default()
        };
        assert_eq!(
            prefer.applied(all).unwrap(),
            "return=headers-only, timezone=America/Los_Angeles"
        );
        assert!(parse(&["tx=commit"]).applied(all).is_none());
    }

    #[test]
    fn applied_header_order_and_scope() {
        let prefer = parse(&[
            "count=exact, handling=strict, max-affected=2, missing=default, resolution=merge-duplicates",
        ]);
        let all = Scope {
            resolution: true,
            missing: true,
            count: true,
            max_affected: true,
            representation: true,
        };
        assert_eq!(
            prefer.applied(all).unwrap(),
            "resolution=merge-duplicates, missing=default, count=exact, handling=strict, max-affected=2"
        );
        assert_eq!(prefer.applied(Scope::default()).unwrap(), "handling=strict");
        assert!(parse(&["count=exact"]).applied(Scope::default()).is_none());
    }
}
