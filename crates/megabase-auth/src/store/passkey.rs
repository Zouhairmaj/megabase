// Ported from supabase/auth internal/models/webauthn_credential.go and
// internal/api/passkey_manage.go (MIT), pin v2.197.0.

//! `auth.webauthn_credentials` reads, renames, and deletes for the passkey
//! management routes.
//!
//! Registration and authentication need WebAuthn attestation and assertion
//! verification, which is not ported, so nothing here creates a credential.

use std::time::SystemTime;

use serde_json::json;
use sqlx::Row;
use uuid::Uuid;

use super::{insert_audit, remember_audit, timed, Backend, BackendKind, StoreError, UserRecord};

/// `PasskeyListItem` columns of one `auth.webauthn_credentials` row.
#[derive(Clone, Debug)]
pub struct PasskeyRow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub friendly_name: String,
    pub created_at: SystemTime,
    pub last_used_at: Option<SystemTime>,
}

const SELECT: &str = "SELECT id, user_id, COALESCE(friendly_name, '') AS friendly_name,
                             created_at, last_used_at
                      FROM auth.webauthn_credentials";

impl Backend {
    /// `FindWebAuthnCredentialsByUserID`: oldest first.
    ///
    /// # Errors
    /// Returns [`StoreError`] when the database cannot answer.
    pub async fn list_passkeys(&self, user_id: Uuid) -> Result<Vec<PasskeyRow>, StoreError> {
        match &self.inner {
            BackendKind::None => Err(StoreError::Unavailable),
            BackendKind::Memory(db) => {
                let db = db.lock().await;
                let mut rows: Vec<PasskeyRow> = db
                    .passkeys
                    .iter()
                    .filter(|row| row.user_id == user_id)
                    .cloned()
                    .collect();
                rows.sort_by_key(|row| row.created_at);
                Ok(rows)
            }
            BackendKind::Postgres(pg) => {
                timed(async {
                    let rows = sqlx::query(&format!(
                        "{SELECT} WHERE user_id = $1 ORDER BY created_at ASC"
                    ))
                    .bind(user_id)
                    .fetch_all(&pg.pool)
                    .await?;
                    rows.iter().map(passkey_row).collect()
                })
                .await
            }
        }
    }

    /// `FindWebAuthnCredentialByIDAndUserID`.
    ///
    /// # Errors
    /// Returns [`StoreError`] when the database cannot answer.
    pub async fn find_owned_passkey(
        &self,
        user_id: Uuid,
        passkey_id: Uuid,
    ) -> Result<Option<PasskeyRow>, StoreError> {
        match &self.inner {
            BackendKind::None => Err(StoreError::Unavailable),
            BackendKind::Memory(db) => Ok(db
                .lock()
                .await
                .passkeys
                .iter()
                .find(|row| row.user_id == user_id && row.id == passkey_id)
                .cloned()),
            BackendKind::Postgres(pg) => {
                timed(async {
                    let row = sqlx::query(&format!("{SELECT} WHERE id = $1 AND user_id = $2"))
                        .bind(passkey_id)
                        .bind(user_id)
                        .fetch_optional(&pg.pool)
                        .await?;
                    row.as_ref().map(passkey_row).transpose()
                })
                .await
            }
        }
    }

    /// `UpdateFriendlyName` and the `passkey_updated` audit entry, in one
    /// transaction.
    ///
    /// # Errors
    /// Returns [`StoreError`] when the database cannot answer; the
    /// transaction is rolled back.
    pub async fn rename_passkey(
        &self,
        user: &UserRecord,
        passkey_id: Uuid,
        friendly_name: &str,
    ) -> Result<(), StoreError> {
        let traits = json!({ "passkey_id": passkey_id.to_string() });
        match &self.inner {
            BackendKind::None => Err(StoreError::Unavailable),
            BackendKind::Memory(db) => {
                let mut db = db.lock().await;
                if let Some(row) = db.passkeys.iter_mut().find(|row| row.id == passkey_id) {
                    row.friendly_name = friendly_name.to_string();
                }
                remember_audit(user, "passkey_updated", "user", Some(traits));
                Ok(())
            }
            BackendKind::Postgres(pg) => {
                timed(async {
                    let mut tx = pg.pool.begin().await?;
                    sqlx::query(
                        "UPDATE auth.webauthn_credentials
                         SET friendly_name = $2, updated_at = now() WHERE id = $1",
                    )
                    .bind(passkey_id)
                    .bind(friendly_name)
                    .execute(&mut *tx)
                    .await?;
                    insert_audit(&mut tx, user, "passkey_updated", "user", Some(traits)).await?;
                    tx.commit().await?;
                    Ok(())
                })
                .await
            }
        }
    }

    /// `WebAuthnCredential.Delete` and the `passkey_deleted` audit entry, in
    /// one transaction.
    ///
    /// # Errors
    /// Returns [`StoreError`] when the database cannot answer; the
    /// transaction is rolled back.
    pub async fn delete_passkey(
        &self,
        user: &UserRecord,
        passkey_id: Uuid,
    ) -> Result<(), StoreError> {
        let traits = json!({ "passkey_id": passkey_id.to_string() });
        match &self.inner {
            BackendKind::None => Err(StoreError::Unavailable),
            BackendKind::Memory(db) => {
                db.lock().await.passkeys.retain(|row| row.id != passkey_id);
                remember_audit(user, "passkey_deleted", "user", Some(traits));
                Ok(())
            }
            BackendKind::Postgres(pg) => {
                timed(async {
                    let mut tx = pg.pool.begin().await?;
                    sqlx::query("DELETE FROM auth.webauthn_credentials WHERE id = $1")
                        .bind(passkey_id)
                        .execute(&mut *tx)
                        .await?;
                    insert_audit(&mut tx, user, "passkey_deleted", "user", Some(traits)).await?;
                    tx.commit().await?;
                    Ok(())
                })
                .await
            }
        }
    }
}

fn passkey_row(row: &sqlx::postgres::PgRow) -> Result<PasskeyRow, StoreError> {
    let created_at: chrono::DateTime<chrono::Utc> = row.try_get("created_at")?;
    let last_used_at: Option<chrono::DateTime<chrono::Utc>> = row.try_get("last_used_at")?;
    Ok(PasskeyRow {
        id: row.try_get("id")?,
        user_id: row.try_get("user_id")?,
        friendly_name: row.try_get("friendly_name")?,
        created_at: created_at.into(),
        last_used_at: last_used_at.map(Into::into),
    })
}

#[cfg(test)]
impl Backend {
    /// Add a passkey for the memory user with `email`; returns its id.
    pub async fn add_passkey_for_test(
        &self,
        email: &str,
        friendly_name: &str,
        created_at: SystemTime,
        last_used_at: Option<SystemTime>,
    ) -> Uuid {
        let BackendKind::Memory(db) = &self.inner else {
            panic!("passkey fixture is memory-only");
        };
        let mut db = db.lock().await;
        let user_id = db
            .users
            .values()
            .find(|user| user.email.eq_ignore_ascii_case(email))
            .expect("passkey fixture user")
            .id;
        let id = Uuid::new_v4();
        db.passkeys.push(PasskeyRow {
            id,
            user_id,
            friendly_name: friendly_name.into(),
            created_at,
            last_used_at,
        });
        id
    }
}
