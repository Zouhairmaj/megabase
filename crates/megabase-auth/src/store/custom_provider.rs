// Ported from supabase/auth internal/models/custom_oauth_provider.go (MIT),
// pin v2.197.0.

//! The `auth.custom_oauth_providers` read the `id_token` grant makes.

use super::{timed, Backend, BackendKind, StoreError};

/// The columns `getProvider` checks on a `custom:` provider.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CustomProviderRow {
    pub enabled: bool,
    /// `oauth2` or `oidc`.
    pub provider_type: String,
    pub issuer: Option<String>,
}

impl Backend {
    /// `FindCustomOAuthProviderByIdentifier`. The in-memory backend has no
    /// custom providers, so every identifier is missing there.
    ///
    /// # Errors
    ///
    /// [`StoreError`] when the database is missing or the read fails.
    pub async fn find_custom_provider(
        &self,
        identifier: &str,
    ) -> Result<Option<CustomProviderRow>, StoreError> {
        match &self.inner {
            BackendKind::None => Err(StoreError::Unavailable),
            BackendKind::Memory(_) => Ok(None),
            BackendKind::Postgres(pg) => {
                timed(async {
                    let row = sqlx::query_as::<_, (bool, String, Option<String>)>(
                        "SELECT enabled, provider_type, issuer FROM auth.custom_oauth_providers
                         WHERE identifier = $1 LIMIT 1",
                    )
                    .bind(identifier)
                    .fetch_optional(&pg.pool)
                    .await?;
                    Ok(
                        row.map(|(enabled, provider_type, issuer)| CustomProviderRow {
                            enabled,
                            provider_type,
                            issuer,
                        }),
                    )
                })
                .await
            }
        }
    }
}
