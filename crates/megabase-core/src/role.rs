//! Which JWT `role` values may be applied with `set_config('role', …, true)`.
//!
//! PostgREST connects as `authenticator` and applies the claim with
//! `set_config` (`Query/PreQuery.hs`). PostgreSQL then rejects a missing
//! role and a role that session user cannot become (`check_role`).
//! `Error.hs` `mapSQLtoHTTP` turns those failures into HTTP 401 and 403.
//! Megabase decides the same rejection before `set_config` runs.

// Ported from postgrest src/library/PostgREST/Query/PreQuery.hs (MIT), pin v16.4,
// and postgrest src/library/PostgREST/Error.hs (MIT), pin v16.4.

use axum::http::StatusCode;

/// `anon`.
pub const ANON: &str = "anon";
/// `authenticated`.
pub const AUTHENTICATED: &str = "authenticated";
/// `service_role`.
pub const SERVICE_ROLE: &str = "service_role";

/// `SELECT EXISTS(SELECT 1 FROM pg_roles WHERE rolname = $1)`.
///
/// Bound lookup used when [`db_role`] rejects a name, so the error matches
/// a missing role or a role the API user cannot become.
pub const ROLE_EXISTS_SQL: &str = "SELECT EXISTS(SELECT 1 FROM pg_roles WHERE rolname = $1)";

/// Roles that may be passed to `set_config('role', …, true)`.
///
/// `anon`, `authenticated`, and `service_role` are always included. `extra`
/// is the operator list (`MEGABASE_DB_EXTRA_ROLES`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ApiRoles {
    extra: Vec<String>,
}

impl ApiRoles {
    /// Roles in `extra`, ignoring empty names. The three API roles are
    /// always allowed and are not stored here.
    #[must_use]
    pub fn new(extra: impl IntoIterator<Item = impl AsRef<str>>) -> Self {
        let mut extra: Vec<String> = extra
            .into_iter()
            .map(|role| role.as_ref().trim().to_string())
            .filter(|role| !role.is_empty())
            .collect();
        extra.sort();
        extra.dedup();
        Self { extra }
    }

    /// Whether `role` may be applied. Comparison is exact, as `pg_roles.rolname` is.
    ///
    /// The setting value `none` is never applied: PostgreSQL's `check_role`
    /// treats that string as clearing the current role.
    #[must_use]
    pub fn allows(&self, role: &str) -> bool {
        role != "none"
            && (matches!(role, ANON | AUTHENTICATED | SERVICE_ROLE)
                || self.extra.iter().any(|extra| extra == role))
    }
}

/// `role` is not one of [`ApiRoles`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnconfiguredRole {
    role: String,
}

impl UnconfiguredRole {
    /// The role name that was rejected.
    #[must_use]
    pub fn role(&self) -> &str {
        &self.role
    }

    /// The PostgreSQL error for this name.
    ///
    /// `exists` is the result of [`ROLE_EXISTS_SQL`]. A missing role is
    /// SQLSTATE `22023` (`role "…" does not exist`). A role that exists is
    /// SQLSTATE `42501` (`permission denied to set role "…"`), which is what
    /// `check_role` raises when the session user is not a member.
    #[must_use]
    pub fn failure(&self, exists: bool) -> RoleFailure {
        RoleFailure::from_catalog(&self.role, exists)
    }
}

/// PostgreSQL's `set_config('role', …)` failure for an API session user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleFailure {
    /// SQLSTATE `22023` or `42501`.
    pub sqlstate: &'static str,
    /// Primary PostgreSQL message, without a severity prefix.
    pub message: String,
}

impl RoleFailure {
    /// Builds the failure `check_role` reports for `role`.
    #[must_use]
    pub fn from_catalog(role: &str, exists: bool) -> Self {
        if exists {
            Self {
                sqlstate: "42501",
                message: format!("permission denied to set role \"{role}\""),
            }
        } else {
            Self {
                sqlstate: "22023",
                message: format!("role \"{role}\" does not exist"),
            }
        }
    }

    /// HTTP status PostgREST uses for this failure.
    ///
    /// `22023` whose message is `role "…" does not exist` is 401. `42501` is
    /// 403 when the requested role is not the anonymous role. A rejected
    /// role is never `anon`, because `anon` is always configured.
    #[must_use]
    pub fn postgrest_status(&self) -> StatusCode {
        if self.sqlstate == "42501" {
            StatusCode::FORBIDDEN
        } else {
            StatusCode::UNAUTHORIZED
        }
    }
}

/// Returns `role` when [`ApiRoles`] allows it.
///
/// # Errors
///
/// Returns [`UnconfiguredRole`] when `role` is not configured. Callers still
/// have to ask PostgreSQL whether that name exists before choosing
/// [`RoleFailure`].
pub fn db_role<'a>(role: &'a str, roles: &ApiRoles) -> Result<&'a str, UnconfiguredRole> {
    if roles.allows(role) {
        Ok(role)
    } else {
        Err(UnconfiguredRole {
            role: role.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_roles_are_allowed_without_extra_configuration() {
        let roles = ApiRoles::default();
        for role in [ANON, AUTHENTICATED, SERVICE_ROLE] {
            assert_eq!(db_role(role, &roles).unwrap(), role);
        }
    }

    #[test]
    fn extra_roles_are_allowed_and_other_names_are_not() {
        let roles = ApiRoles::new([" reporter ", "", "reporter", "analytics"]);
        assert!(roles.allows("reporter"));
        assert!(roles.allows("analytics"));
        assert!(roles.allows(ANON));
        assert!(db_role("postgres", &roles).is_err());
        assert!(db_role("Anon", &roles).is_err());
        assert!(db_role("not existing", &roles).is_err());
        assert!(!ApiRoles::new(["none"]).allows("none"));
        assert!(db_role("none", &ApiRoles::new(["none"])).is_err());
    }

    #[test]
    fn missing_role_matches_postgrest() {
        let rejected = db_role("not existing", &ApiRoles::default()).unwrap_err();
        let failure = rejected.failure(false);
        assert_eq!(failure.sqlstate, "22023");
        assert_eq!(failure.message, "role \"not existing\" does not exist");
        assert_eq!(failure.postgrest_status(), StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn existing_role_matches_postgrest() {
        let rejected = db_role("postgres", &ApiRoles::default()).unwrap_err();
        let failure = rejected.failure(true);
        assert_eq!(failure.sqlstate, "42501");
        assert_eq!(
            failure.message,
            "permission denied to set role \"postgres\""
        );
        assert_eq!(failure.postgrest_status(), StatusCode::FORBIDDEN);
    }

    #[test]
    fn quote_characters_in_the_name_are_copied_into_the_message() {
        let failure = RoleFailure::from_catalog("a\"b", false);
        assert_eq!(failure.message, "role \"a\"b\" does not exist");
    }
}
