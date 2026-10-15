// Ported from supabase/auth internal/models/factor.go,
// internal/models/recovery_code.go, internal/models/sessions.go, and
// internal/models/user.go (MIT), pin v2.197.0.

//! MFA factor and recovery-code reads, and factor removal.
//!
//! Enrollment, challenges, and verification are not ported, so nothing here
//! creates a factor, a challenge, or a recovery code.

use std::collections::HashMap;

use serde_json::Value;
use sqlx::Row;
use uuid::Uuid;

use super::{insert_audit, remember_audit, timed, Backend, BackendKind, StoreError, UserRecord};

/// One `auth.mfa_factors` row, as far as the factor routes read it.
#[derive(Clone, Debug)]
pub struct FactorRow {
    pub id: Uuid,
    pub user_id: Uuid,
    /// `totp`, `phone`, `webauthn`, or `recovery_code`.
    pub factor_type: String,
    /// `unverified` or `verified`.
    pub status: String,
    pub friendly_name: String,
}

impl FactorRow {
    /// `Factor.IsVerified`.
    pub fn is_verified(&self) -> bool {
        self.status == "verified"
    }

    /// `amrMethodForFactorType`. `None` for a type GoTrue does not map.
    fn amr_method(&self) -> Option<&'static str> {
        match self.factor_type.as_str() {
            "totp" => Some("totp"),
            "phone" => Some("mfa/phone"),
            "webauthn" => Some("mfa/webauthn"),
            "recovery_code" => Some("mfa/recovery_code"),
            _ => None,
        }
    }
}

/// `GET /factors/recovery-codes` counts for the user's recovery-code set.
#[derive(Clone, Copy, Debug)]
pub struct RecoveryCodeStatus {
    /// The set's `mfa_factor_id`; GoTrue reports the factor id, not the set id.
    pub factor_id: Uuid,
    pub total: i64,
    pub remaining: i64,
}

/// In-memory stand-in for the MFA tables.
#[derive(Default)]
pub(super) struct MemoryMfa {
    factors: Vec<FactorRow>,
    /// Recovery-code set per user.
    recovery: HashMap<Uuid, RecoveryCodeStatus>,
    /// AAL2 sessions and the factor (`sessions.factor_id`) that raised them.
    aal2_sessions: HashMap<Uuid, Uuid>,
}

impl Backend {
    /// `Session.IsAAL2` for the JWT's session row.
    ///
    /// # Errors
    /// Returns [`StoreError`] when the database cannot answer.
    pub async fn session_is_aal2(&self, session_id: Uuid) -> Result<bool, StoreError> {
        match &self.inner {
            BackendKind::None => Err(StoreError::Unavailable),
            BackendKind::Memory(db) => {
                Ok(db.lock().await.mfa.aal2_sessions.contains_key(&session_id))
            }
            BackendKind::Postgres(pg) => {
                timed(async {
                    let row =
                        sqlx::query("SELECT aal::text AS aal FROM auth.sessions WHERE id = $1")
                            .bind(session_id)
                            .fetch_optional(&pg.pool)
                            .await?;
                    let aal = match row {
                        Some(row) => row.try_get::<Option<String>, _>("aal")?,
                        None => None,
                    };
                    Ok(aal.as_deref() == Some("aal2"))
                })
                .await
            }
        }
    }

    /// `User.FindOwnedFactorByID`.
    ///
    /// # Errors
    /// Returns [`StoreError`] when the database cannot answer.
    pub async fn find_owned_factor(
        &self,
        user_id: Uuid,
        factor_id: Uuid,
    ) -> Result<Option<FactorRow>, StoreError> {
        match &self.inner {
            BackendKind::None => Err(StoreError::Unavailable),
            BackendKind::Memory(db) => Ok(db
                .lock()
                .await
                .mfa
                .factors
                .iter()
                .find(|factor| factor.user_id == user_id && factor.id == factor_id)
                .cloned()),
            BackendKind::Postgres(pg) => {
                timed(async {
                    let row = sqlx::query(
                        "SELECT id, user_id, factor_type::text AS factor_type,
                                status::text AS status, COALESCE(friendly_name, '') AS friendly_name
                         FROM auth.mfa_factors WHERE user_id = $1 AND id = $2 LIMIT 1",
                    )
                    .bind(user_id)
                    .bind(factor_id)
                    .fetch_optional(&pg.pool)
                    .await?;
                    row.map(|row| factor_row(&row)).transpose()
                })
                .await
            }
        }
    }

    /// `FindRecoveryCodeSetByUser` followed by `CountRecoveryCodes`.
    ///
    /// # Errors
    /// Returns [`StoreError`] when the database cannot answer.
    pub async fn recovery_code_status(
        &self,
        user_id: Uuid,
    ) -> Result<Option<RecoveryCodeStatus>, StoreError> {
        match &self.inner {
            BackendKind::None => Err(StoreError::Unavailable),
            BackendKind::Memory(db) => Ok(db.lock().await.mfa.recovery.get(&user_id).copied()),
            BackendKind::Postgres(pg) => {
                timed(async {
                    let Some(set) = sqlx::query(
                        "SELECT id, mfa_factor_id FROM auth.mfa_recovery_code_sets
                         WHERE user_id = $1 LIMIT 1",
                    )
                    .bind(user_id)
                    .fetch_optional(&pg.pool)
                    .await?
                    else {
                        return Ok(None);
                    };
                    let set_id: Uuid = set.try_get("id")?;
                    let factor_id: Uuid = set.try_get("mfa_factor_id")?;
                    let counts = sqlx::query(
                        "SELECT COUNT(*) AS total,
                                COUNT(*) FILTER (WHERE consumed_at IS NULL) AS remaining
                         FROM auth.mfa_recovery_codes WHERE mfa_recovery_code_set_id = $1",
                    )
                    .bind(set_id)
                    .fetch_one(&pg.pool)
                    .await?;
                    Ok(Some(RecoveryCodeStatus {
                        factor_id,
                        total: counts.try_get("total")?,
                        remaining: counts.try_get("remaining")?,
                    }))
                })
                .await
            }
        }
    }

    /// `FindRecoveryCodeFactorByUser`.
    ///
    /// # Errors
    /// Returns [`StoreError`] when the database cannot answer.
    pub async fn recovery_code_factor(
        &self,
        user_id: Uuid,
    ) -> Result<Option<FactorRow>, StoreError> {
        match &self.inner {
            BackendKind::None => Err(StoreError::Unavailable),
            BackendKind::Memory(db) => Ok(db
                .lock()
                .await
                .mfa
                .factors
                .iter()
                .find(|factor| factor.user_id == user_id && factor.factor_type == "recovery_code")
                .cloned()),
            BackendKind::Postgres(pg) => {
                timed(async {
                    let row = sqlx::query(
                        "SELECT id, user_id, factor_type::text AS factor_type,
                                status::text AS status, COALESCE(friendly_name, '') AS friendly_name
                         FROM auth.mfa_factors
                         WHERE user_id = $1 AND factor_type = 'recovery_code' LIMIT 1",
                    )
                    .bind(user_id)
                    .fetch_optional(&pg.pool)
                    .await?;
                    row.map(|row| factor_row(&row)).transpose()
                })
                .await
            }
        }
    }

    /// Delete `factor`, write the audit entry, and run
    /// `Factor.DowngradeSessionsToAAL1`, in one transaction.
    ///
    /// `action` is the audit action (`factor_unenrolled` or
    /// `recovery_codes_deleted`) and `traits` its traits object.
    ///
    /// # Errors
    /// Returns [`StoreError`] when the database cannot answer; the
    /// transaction is rolled back.
    pub async fn unenroll_factor(
        &self,
        user: &UserRecord,
        factor: &FactorRow,
        action: &str,
        traits: Value,
    ) -> Result<(), StoreError> {
        match &self.inner {
            BackendKind::None => Err(StoreError::Unavailable),
            BackendKind::Memory(db) => {
                let mut db = db.lock().await;
                let mfa = &mut db.mfa;
                mfa.factors.retain(|row| row.id != factor.id);
                mfa.recovery.retain(|_, set| set.factor_id != factor.id);
                mfa.aal2_sessions
                    .retain(|_, factor_id| *factor_id != factor.id);
                remember_audit(user, action, "factor", Some(traits));
                Ok(())
            }
            BackendKind::Postgres(pg) => {
                timed(async {
                    let mut tx = pg.pool.begin().await?;
                    sqlx::query("DELETE FROM auth.mfa_factors WHERE id = $1")
                        .bind(factor.id)
                        .execute(&mut *tx)
                        .await?;
                    insert_audit(&mut tx, user, action, "factor", Some(traits)).await?;
                    if let Some(method) = factor.amr_method() {
                        sqlx::query(
                            "DELETE FROM auth.mfa_amr_claims
                             WHERE authentication_method = $2 AND session_id IN
                                 (SELECT id FROM auth.sessions WHERE factor_id = $1)",
                        )
                        .bind(factor.id)
                        .bind(method)
                        .execute(&mut *tx)
                        .await?;
                    }
                    sqlx::query(
                        "UPDATE auth.sessions SET aal = 'aal1', factor_id = NULL
                         WHERE user_id = $1 AND factor_id = $2",
                    )
                    .bind(factor.user_id)
                    .bind(factor.id)
                    .execute(&mut *tx)
                    .await?;
                    tx.commit().await?;
                    Ok(())
                })
                .await
            }
        }
    }
}

fn factor_row(row: &sqlx::postgres::PgRow) -> Result<FactorRow, StoreError> {
    Ok(FactorRow {
        id: row.try_get("id")?,
        user_id: row.try_get("user_id")?,
        factor_type: row.try_get("factor_type")?,
        status: row.try_get("status")?,
        friendly_name: row.try_get("friendly_name")?,
    })
}

#[cfg(test)]
impl Backend {
    /// Add a factor for the memory user with `email`; returns its id.
    pub async fn add_factor_for_test(&self, email: &str, factor_type: &str, status: &str) -> Uuid {
        let BackendKind::Memory(db) = &self.inner else {
            panic!("factor fixture is memory-only");
        };
        let mut db = db.lock().await;
        let user_id = db
            .users
            .values()
            .find(|user| user.email.eq_ignore_ascii_case(email))
            .expect("factor fixture user")
            .id;
        let id = Uuid::new_v4();
        db.mfa.factors.push(FactorRow {
            id,
            user_id,
            factor_type: factor_type.into(),
            status: status.into(),
            friendly_name: String::new(),
        });
        id
    }

    /// Give the memory user with `email` a recovery-code set backed by `factor_id`.
    pub async fn add_recovery_set_for_test(
        &self,
        email: &str,
        factor_id: Uuid,
        total: i64,
        remaining: i64,
    ) {
        let BackendKind::Memory(db) = &self.inner else {
            panic!("recovery set fixture is memory-only");
        };
        let mut db = db.lock().await;
        let user_id = db
            .users
            .values()
            .find(|user| user.email.eq_ignore_ascii_case(email))
            .expect("recovery set fixture user")
            .id;
        db.mfa.recovery.insert(
            user_id,
            RecoveryCodeStatus {
                factor_id,
                total,
                remaining,
            },
        );
    }

    /// Mark `session_id` AAL2, raised by `factor_id`.
    pub async fn set_aal2_for_test(&self, session_id: Uuid, factor_id: Uuid) {
        let BackendKind::Memory(db) = &self.inner else {
            panic!("aal fixture is memory-only");
        };
        db.lock()
            .await
            .mfa
            .aal2_sessions
            .insert(session_id, factor_id);
    }

    /// Whether the memory store still holds `factor_id`.
    pub async fn has_factor_for_test(&self, factor_id: Uuid) -> bool {
        let BackendKind::Memory(db) = &self.inner else {
            panic!("factor fixture is memory-only");
        };
        db.lock()
            .await
            .mfa
            .factors
            .iter()
            .any(|factor| factor.id == factor_id)
    }
}
