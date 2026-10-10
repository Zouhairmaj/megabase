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

/// Parsed preferences. Kinds that are not served yet are in `unserved`.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Prefer {
    pub(crate) resolution: Option<Resolution>,
    pub(crate) count: Option<Count>,
    pub(crate) missing: Option<Missing>,
    pub(crate) handling: Option<Handling>,
    pub(crate) max_affected: Option<i64>,
    /// Unit id of the first `return=`, `tx=` or `timezone=` preference.
    pub(crate) unserved: Option<&'static str>,
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
                self.unserved = self.unserved.or(Some("rest:prefer:return=minimal"))
            }
            "return=representation" => {
                self.unserved = self.unserved.or(Some("rest:prefer:return=representation"));
            }
            "return=headers-only" => {
                self.unserved = self.unserved.or(Some("rest:prefer:return=headers-only"));
            }
            "tx=commit" => self.unserved = self.unserved.or(Some("rest:prefer:tx=commit")),
            "tx=rollback" => self.unserved = self.unserved.or(Some("rest:prefer:tx=rollback")),
            _ if pref.starts_with("timezone=") => {
                self.unserved = self.unserved.or(Some("rest:prefer:timezone=*"));
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
    fn unserved_kinds_are_reported() {
        assert_eq!(
            parse(&["return=minimal"]).unserved,
            Some("rest:prefer:return=minimal")
        );
        assert_eq!(
            parse(&["timezone=UTC"]).unserved,
            Some("rest:prefer:timezone=*")
        );
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
        };
        assert_eq!(
            prefer.applied(all).unwrap(),
            "resolution=merge-duplicates, missing=default, count=exact, handling=strict, max-affected=2"
        );
        assert_eq!(prefer.applied(Scope::default()).unwrap(), "handling=strict");
        assert!(parse(&["count=exact"]).applied(Scope::default()).is_none());
    }
}
