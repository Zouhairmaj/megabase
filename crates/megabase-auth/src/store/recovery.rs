// Ported from supabase/auth internal/api/mail.go (sendPasswordRecovery),
// internal/models/user.go, and internal/models/one_time_token.go (MIT),
// pin v2.197.0.

//! Password-recovery token reads and writes for `POST /recover`.

use std::time::SystemTime;

use sqlx::Row;
use uuid::Uuid;

use super::{
    insert_audit, remember_audit, timed, ts, Backend, BackendKind, OneTimeToken, StoreError,
    UserRecord, OTT_RECOVERY,
};

const RECOVERY_REQUESTED: &str = "user_recovery_requested";

impl Backend {
    /// `users.recovery_sent_at` for the frequency limit.
    ///
    /// # Errors
    ///
    /// [`StoreError`] when the database is missing or the read fails.
    pub async fn recovery_sent_at(&self, user_id: Uuid) -> Result<Option<SystemTime>, StoreError> {
        match &self.inner {
            BackendKind::None => Err(StoreError::Unavailable),
            BackendKind::Memory(db) => Ok(db
                .lock()
                .await
                .tokens
                .get(&user_id)
                .and_then(|tokens| tokens.recovery_sent_at)),
            BackendKind::Postgres(pg) => {
                timed(async {
                    let row = sqlx::query(
                        "SELECT recovery_sent_at FROM auth.users WHERE id = $1::uuid LIMIT 1",
                    )
                    .bind(user_id)
                    .fetch_optional(&pg.pool)
                    .await?;
                    let sent: Option<chrono::DateTime<chrono::Utc>> = match row {
                        Some(row) => row.try_get("recovery_sent_at")?,
                        None => None,
                    };
                    Ok(sent.map(SystemTime::from))
                })
                .await
            }
        }
    }

    /// Store a delivered recovery token: the `user_recovery_requested` audit
    /// entry, `recovery_token` and `recovery_sent_at` on the user, and a
    /// `recovery_token` one-time token that replaces the previous one. One
    /// transaction, as GoTrue's `Recover`.
    ///
    /// # Errors
    ///
    /// [`StoreError`] when the database is missing or a write fails; nothing
    /// is kept then.
    pub async fn save_recovery(
        &self,
        user: &UserRecord,
        token_hash: &str,
        sent_at: SystemTime,
    ) -> Result<(), StoreError> {
        match &self.inner {
            BackendKind::None => Err(StoreError::Unavailable),
            BackendKind::Memory(db) => {
                let mut db = db.lock().await;
                let tokens = db.tokens.entry(user.id).or_default();
                tokens.recovery_token = token_hash.to_string();
                tokens.recovery_sent_at = Some(sent_at);
                db.one_time
                    .retain(|ott| !(ott.user_id == user.id && ott.token_type == OTT_RECOVERY));
                db.one_time.push(OneTimeToken {
                    user_id: user.id,
                    token_type: OTT_RECOVERY.into(),
                    token_hash: token_hash.to_string(),
                });
                if let Some(stored) = db.users.get_mut(&user.id) {
                    stored.updated_at = sent_at;
                }
                remember_audit(user, RECOVERY_REQUESTED, "user", None);
                drop(db);
                self.clear_otp_attempts(user.id);
                Ok(())
            }
            BackendKind::Postgres(pg) => {
                timed(async {
                    let mut tx = pg.pool.begin().await?;
                    insert_audit(&mut tx, user, RECOVERY_REQUESTED, "user", None).await?;
                    sqlx::query(
                        "UPDATE auth.users
                         SET recovery_token = $2, recovery_sent_at = $3, updated_at = $3
                         WHERE id = $1::uuid",
                    )
                    .bind(user.id)
                    .bind(token_hash)
                    .bind(ts(sent_at))
                    .execute(&mut *tx)
                    .await?;
                    sqlx::query(
                        "DELETE FROM auth.one_time_tokens
                         WHERE user_id = $1::uuid AND token_type = $2::auth.one_time_token_type",
                    )
                    .bind(user.id)
                    .bind(OTT_RECOVERY)
                    .execute(&mut *tx)
                    .await?;
                    sqlx::query(
                        "INSERT INTO auth.one_time_tokens
                             (id, user_id, token_type, token_hash, relates_to, created_at, updated_at)
                         VALUES ($1::uuid, $2::uuid, $3::auth.one_time_token_type, $4, $5, $6, $6)",
                    )
                    .bind(Uuid::new_v4())
                    .bind(user.id)
                    .bind(OTT_RECOVERY)
                    .bind(token_hash)
                    .bind(user.email.to_lowercase())
                    .bind(ts(sent_at))
                    .execute(&mut *tx)
                    .await?;
                    tx.commit().await?;
                    self.clear_otp_attempts(user.id);
                    Ok(())
                })
                .await
            }
        }
    }

    /// `recovery_token` of the memory user with `email`, empty when unset.
    #[cfg(test)]
    pub async fn recovery_token_for_test(&self, email: &str) -> String {
        let BackendKind::Memory(db) = &self.inner else {
            panic!("recovery fixture is memory-only");
        };
        let db = db.lock().await;
        db.users
            .values()
            .find(|user| user.email.eq_ignore_ascii_case(email))
            .and_then(|user| db.tokens.get(&user.id))
            .map(|tokens| tokens.recovery_token.clone())
            .unwrap_or_default()
    }
}
