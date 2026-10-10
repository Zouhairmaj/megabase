//! Newtypes for identifiers and secrets that would otherwise be bare strings.

use std::fmt;

/// JWT `sub` for a user. Auth access tokens set this to the user's id.
///
/// The demo anon and service-role keys omit `sub`. This type does not require
/// a UUID: verification accepts any JSON string, and Auth rejects a non-UUID
/// subject when a route needs one.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct UserId(String);

impl UserId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for UserId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("UserId").field(&self.0).finish()
    }
}

impl fmt::Display for UserId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for UserId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl std::ops::Deref for UserId {
    type Target = str;

    fn deref(&self) -> &str {
        &self.0
    }
}

impl From<String> for UserId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl From<&str> for UserId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

/// Compact serialized JWT (`header.payload.signature`).
///
/// Debug output redacts the token. [`Self::as_str`] is the wire form.
#[derive(Clone, PartialEq, Eq)]
pub struct CompactJwt(String);

impl CompactJwt {
    pub(crate) fn new(token: String) -> Self {
        Self(token)
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for CompactJwt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CompactJwt(<redacted>)")
    }
}

impl fmt::Display for CompactJwt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for CompactJwt {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl std::ops::Deref for CompactJwt {
    type Target = str;

    fn deref(&self) -> &str {
        &self.0
    }
}

/// `JWT_SECRET` bytes. Debug output is redacted.
#[derive(Clone, PartialEq, Eq)]
pub struct JwtSecret(String);

impl JwtSecret {
    pub fn new(secret: impl Into<String>) -> Self {
        Self(secret.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }
}

impl fmt::Debug for JwtSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("JwtSecret(<redacted>)")
    }
}
