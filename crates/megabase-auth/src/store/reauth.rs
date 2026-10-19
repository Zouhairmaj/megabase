// Ported from supabase/auth internal/api/mail.go (sendConfirmation,
// sendEmailChange, sendReauthenticationOtp), internal/api/resend.go,
// internal/api/reauthenticate.go, internal/api/pkce.go (generateFlowState),
// and internal/models/one_time_token.go (MIT), pin v2.197.0.

//! Token writes for `GET /reauthenticate` and `POST /resend`.

use std::time::SystemTime;

use sqlx::Row;
use uuid::Uuid;

use super::pkce::insert_flow_state;
use super::{
    from_ts, insert_audit, remember_audit, timed, ts, Backend, BackendKind, FlowStateRecord,
    MemoryDb, OneTimeToken, StoreError, UserRecord, OTT_CONFIRMATION, OTT_EMAIL_CURRENT,
    OTT_EMAIL_NEW,
};

const REAUTHENTICATE: &str = "user_reauthenticate_requested";
const CONFIRMATION_REQUESTED: &str = "user_confirmation_requested";
const OTT_REAUTHENTICATION: &str = "reauthentication_token";

/// Columns `sendConfirmation`, `sendEmailChange`, and
/// `sendReauthenticationOtp` read before they send.
pub struct PendingMail {
    pub confirmation_sent_at: Option<SystemTime>,
    pub reauthentication_sent_at: Option<SystemTime>,
    pub email_change: String,
    pub email_change_sent_at: Option<SystemTime>,
    pub phone_change: String,
}

/// A resend of the signup confirmation email, after the mail hand-off.
pub struct SignupResend<'a> {
    pub user: &'a UserRecord,
    pub token_hash: &'a str,
    pub sent_at: SystemTime,
    /// PKCE flow written in the same transaction as the token.
    pub flow: Option<&'a FlowStateRecord>,
}

/// A resend of a pending email change, after the mail hand-off.
pub struct EmailChangeResend<'a> {
    pub user: &'a UserRecord,
    /// The address already stored in `email_change`.
    pub email_change: &'a str,
    /// Hash for the current address. `None` leaves that column alone
    /// (`SecureEmailChangeEnabled` is off, or the user has no email).
    pub token_current: Option<&'a str>,
    pub token_new: &'a str,
    pub sent_at: SystemTime,
    pub flow: Option<&'a FlowStateRecord>,
}

impl Backend {
    /// The mail columns for `user_id`. A missing row is empty pending mail.
    ///
    /// # Errors
    ///
    /// [`StoreError`] when the database is missing or the read fails.
    pub async fn pending_mail(&self, user_id: Uuid) -> Result<PendingMail, StoreError> {
        match &self.inner {
            BackendKind::None => Err(StoreError::Unavailable),
            BackendKind::Memory(db) => {
                let db = db.lock().await;
                Ok(pending_from_tokens(db.tokens.get(&user_id)))
            }
            BackendKind::Postgres(pg) => {
                timed(async {
                    let row = sqlx::query(
                        "SELECT confirmation_sent_at, reauthentication_sent_at, email_change,
                                email_change_sent_at, phone_change
                         FROM auth.users WHERE id = $1::uuid",
                    )
                    .bind(user_id)
                    .fetch_optional(&pg.pool)
                    .await?;
                    let Some(row) = row else {
                        return Ok(PendingMail::empty());
                    };
                    let time = |name: &str| -> Result<Option<SystemTime>, sqlx::Error> {
                        Ok(row
                            .try_get::<Option<chrono::DateTime<chrono::Utc>>, _>(name)?
                            .map(from_ts))
                    };
                    let text = |name: &str| -> Result<String, sqlx::Error> {
                        Ok(row.try_get::<Option<String>, _>(name)?.unwrap_or_default())
                    };
                    Ok(PendingMail {
                        confirmation_sent_at: time("confirmation_sent_at")?,
                        reauthentication_sent_at: time("reauthentication_sent_at")?,
                        email_change: text("email_change")?,
                        email_change_sent_at: time("email_change_sent_at")?,
                        phone_change: text("phone_change")?,
                    })
                })
                .await
            }
        }
    }

    /// `user_reauthenticate_requested`, then `reauthentication_token`,
    /// `reauthentication_sent_at`, and a replacement one-time token.
    ///
    /// # Errors
    ///
    /// [`StoreError`] when the database is missing or a write fails.
    pub async fn save_reauthentication(
        &self,
        user: &UserRecord,
        token_hash: &str,
        sent_at: SystemTime,
    ) -> Result<(), StoreError> {
        let saved = match &self.inner {
            BackendKind::None => Err(StoreError::Unavailable),
            BackendKind::Memory(db) => {
                let mut db = db.lock().await;
                let tokens = db.tokens.entry(user.id).or_default();
                tokens.reauthentication_token = token_hash.to_string();
                tokens.reauthentication_sent_at = Some(sent_at);
                replace_ott_memory(&mut db, user.id, OTT_REAUTHENTICATION, token_hash);
                if let Some(stored) = db.users.get_mut(&user.id) {
                    stored.updated_at = sent_at;
                }
                remember_audit(user, REAUTHENTICATE, "user", None);
                Ok(())
            }
            BackendKind::Postgres(pg) => {
                timed(async {
                    let mut tx = pg.pool.begin().await?;
                    insert_audit(&mut tx, user, REAUTHENTICATE, "user", None).await?;
                    sqlx::query(
                        "UPDATE auth.users
                         SET reauthentication_token = $2, reauthentication_sent_at = $3,
                             updated_at = $3
                         WHERE id = $1::uuid",
                    )
                    .bind(user.id)
                    .bind(token_hash)
                    .bind(ts(sent_at))
                    .execute(&mut *tx)
                    .await?;
                    replace_ott(
                        &mut tx,
                        user.id,
                        OTT_REAUTHENTICATION,
                        token_hash,
                        &user.email,
                        sent_at,
                    )
                    .await?;
                    tx.commit().await?;
                    Ok(())
                })
                .await
            }
        };
        if saved.is_ok() {
            self.clear_otp_attempts(user.id);
        }
        saved
    }

    /// `user_confirmation_requested`, the optional PKCE flow, then
    /// `confirmation_token` and its one-time token.
    ///
    /// # Errors
    ///
    /// [`StoreError`] when the database is missing or a write fails.
    pub async fn save_signup_resend(&self, cmd: SignupResend<'_>) -> Result<(), StoreError> {
        let saved = match &self.inner {
            BackendKind::None => Err(StoreError::Unavailable),
            BackendKind::Memory(db) => {
                let mut db = db.lock().await;
                let tokens = db.tokens.entry(cmd.user.id).or_default();
                tokens.confirmation_token = cmd.token_hash.to_string();
                tokens.confirmation_sent_at = Some(cmd.sent_at);
                replace_ott_memory(&mut db, cmd.user.id, OTT_CONFIRMATION, cmd.token_hash);
                store_flow_memory(&mut db, cmd.flow, cmd.user.id, cmd.sent_at);
                if let Some(stored) = db.users.get_mut(&cmd.user.id) {
                    stored.updated_at = cmd.sent_at;
                }
                remember_audit(cmd.user, CONFIRMATION_REQUESTED, "user", None);
                Ok(())
            }
            BackendKind::Postgres(pg) => {
                timed(async {
                    let mut tx = pg.pool.begin().await?;
                    insert_audit(&mut tx, cmd.user, CONFIRMATION_REQUESTED, "user", None).await?;
                    if let Some(flow) = cmd.flow {
                        let mut flow = flow.clone();
                        flow.user_id = Some(cmd.user.id);
                        insert_flow_state(&mut tx, Uuid::new_v4(), &flow, cmd.sent_at).await?;
                    }
                    sqlx::query(
                        "UPDATE auth.users
                         SET confirmation_token = $2, confirmation_sent_at = $3, updated_at = $3
                         WHERE id = $1::uuid",
                    )
                    .bind(cmd.user.id)
                    .bind(cmd.token_hash)
                    .bind(ts(cmd.sent_at))
                    .execute(&mut *tx)
                    .await?;
                    replace_ott(
                        &mut tx,
                        cmd.user.id,
                        OTT_CONFIRMATION,
                        cmd.token_hash,
                        &cmd.user.email,
                        cmd.sent_at,
                    )
                    .await?;
                    tx.commit().await?;
                    Ok(())
                })
                .await
            }
        };
        if saved.is_ok() {
            self.clear_otp_attempts(cmd.user.id);
        }
        saved
    }

    /// Optional PKCE flow, then the email-change token columns and one-time
    /// tokens. Resend writes no audit entry for this type.
    ///
    /// # Errors
    ///
    /// [`StoreError`] when the database is missing or a write fails.
    pub async fn save_email_change_resend(
        &self,
        cmd: EmailChangeResend<'_>,
    ) -> Result<(), StoreError> {
        let saved = match &self.inner {
            BackendKind::None => Err(StoreError::Unavailable),
            BackendKind::Memory(db) => {
                let mut db = db.lock().await;
                let tokens = db.tokens.entry(cmd.user.id).or_default();
                tokens.email_change = cmd.email_change.to_string();
                tokens.email_change_token_new = cmd.token_new.to_string();
                tokens.email_change_sent_at = Some(cmd.sent_at);
                tokens.email_change_confirm_status = 0;
                if let Some(current) = cmd.token_current {
                    tokens.email_change_token_current = current.to_string();
                }
                replace_ott_memory(&mut db, cmd.user.id, OTT_EMAIL_NEW, cmd.token_new);
                if let Some(current) = cmd.token_current {
                    replace_ott_memory(&mut db, cmd.user.id, OTT_EMAIL_CURRENT, current);
                }
                store_flow_memory(&mut db, cmd.flow, cmd.user.id, cmd.sent_at);
                if let Some(stored) = db.users.get_mut(&cmd.user.id) {
                    stored.updated_at = cmd.sent_at;
                }
                Ok(())
            }
            BackendKind::Postgres(pg) => {
                timed(async {
                    let mut tx = pg.pool.begin().await?;
                    if let Some(flow) = cmd.flow {
                        let mut flow = flow.clone();
                        flow.user_id = Some(cmd.user.id);
                        insert_flow_state(&mut tx, Uuid::new_v4(), &flow, cmd.sent_at).await?;
                    }
                    if let Some(current) = cmd.token_current {
                        sqlx::query(
                            "UPDATE auth.users SET email_change = $2, email_change_sent_at = $3,
                                    email_change_confirm_status = 0,
                                    email_change_token_current = $4, email_change_token_new = $5,
                                    updated_at = $3
                             WHERE id = $1::uuid",
                        )
                        .bind(cmd.user.id)
                        .bind(cmd.email_change)
                        .bind(ts(cmd.sent_at))
                        .bind(current)
                        .bind(cmd.token_new)
                        .execute(&mut *tx)
                        .await?;
                        replace_ott(
                            &mut tx,
                            cmd.user.id,
                            OTT_EMAIL_CURRENT,
                            current,
                            &cmd.user.email,
                            cmd.sent_at,
                        )
                        .await?;
                    } else {
                        sqlx::query(
                            "UPDATE auth.users SET email_change = $2, email_change_sent_at = $3,
                                    email_change_confirm_status = 0, email_change_token_new = $4,
                                    updated_at = $3
                             WHERE id = $1::uuid",
                        )
                        .bind(cmd.user.id)
                        .bind(cmd.email_change)
                        .bind(ts(cmd.sent_at))
                        .bind(cmd.token_new)
                        .execute(&mut *tx)
                        .await?;
                    }
                    replace_ott(
                        &mut tx,
                        cmd.user.id,
                        OTT_EMAIL_NEW,
                        cmd.token_new,
                        cmd.email_change,
                        cmd.sent_at,
                    )
                    .await?;
                    tx.commit().await?;
                    Ok(())
                })
                .await
            }
        };
        if saved.is_ok() {
            self.clear_otp_attempts(cmd.user.id);
        }
        saved
    }
}

impl PendingMail {
    fn empty() -> Self {
        Self {
            confirmation_sent_at: None,
            reauthentication_sent_at: None,
            email_change: String::new(),
            email_change_sent_at: None,
            phone_change: String::new(),
        }
    }
}

fn pending_from_tokens(tokens: Option<&super::UserTokens>) -> PendingMail {
    let Some(tokens) = tokens else {
        return PendingMail::empty();
    };
    PendingMail {
        confirmation_sent_at: tokens.confirmation_sent_at,
        reauthentication_sent_at: tokens.reauthentication_sent_at,
        email_change: tokens.email_change.clone(),
        email_change_sent_at: tokens.email_change_sent_at,
        phone_change: tokens.phone_change.clone(),
    }
}

fn replace_ott_memory(db: &mut MemoryDb, user_id: Uuid, token_type: &str, token_hash: &str) {
    db.one_time
        .retain(|ott| !(ott.user_id == user_id && ott.token_type == token_type));
    db.one_time.push(OneTimeToken {
        user_id,
        token_type: token_type.to_string(),
        token_hash: token_hash.to_string(),
    });
}

fn store_flow_memory(
    db: &mut MemoryDb,
    flow: Option<&FlowStateRecord>,
    user_id: Uuid,
    sent_at: SystemTime,
) {
    let Some(flow) = flow else {
        return;
    };
    let mut stored = flow.clone();
    stored.user_id = Some(user_id);
    stored.created_at = Some(sent_at);
    db.flow_states.insert(Uuid::new_v4(), stored);
}

async fn replace_ott(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: Uuid,
    token_type: &str,
    token_hash: &str,
    relates_to: &str,
    sent_at: SystemTime,
) -> Result<(), StoreError> {
    sqlx::query(
        "DELETE FROM auth.one_time_tokens
         WHERE user_id = $1::uuid AND token_type = $2::auth.one_time_token_type",
    )
    .bind(user_id)
    .bind(token_type)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "INSERT INTO auth.one_time_tokens
             (id, user_id, token_type, token_hash, relates_to, created_at, updated_at)
         VALUES ($1::uuid, $2::uuid, $3::auth.one_time_token_type, $4, $5, $6, $6)",
    )
    .bind(Uuid::new_v4())
    .bind(user_id)
    .bind(token_type)
    .bind(token_hash)
    .bind(relates_to.to_lowercase())
    .bind(ts(sent_at))
    .execute(&mut **tx)
    .await?;
    Ok(())
}

#[cfg(test)]
impl Backend {
    /// `reauthentication_token` of the memory user with `email`.
    pub async fn reauthentication_token_for_test(&self, email: &str) -> String {
        self.token_column_for_test(email, |tokens| tokens.reauthentication_token.clone())
            .await
    }

    /// `(email_change_token_current, email_change_token_new)` for `email`.
    pub async fn email_change_tokens_for_test(&self, email: &str) -> (String, String) {
        self.token_column_for_test(email, |tokens| {
            (
                tokens.email_change_token_current.clone(),
                tokens.email_change_token_new.clone(),
            )
        })
        .await
    }

    /// Set `email_change` on the memory user with `email`.
    pub async fn stage_email_change_for_test(&self, email: &str, new_email: &str) {
        let BackendKind::Memory(db) = &self.inner else {
            panic!("email-change fixture is memory-only");
        };
        let mut db = db.lock().await;
        let Some(user_id) = db
            .users
            .values()
            .find(|user| user.email.eq_ignore_ascii_case(email))
            .map(|user| user.id)
        else {
            panic!("no user {email}");
        };
        db.tokens.entry(user_id).or_default().email_change = new_email.to_string();
    }

    /// `(authentication_method, code_challenge)` of a flow for `user_id`.
    pub async fn flow_for_user_for_test(&self, user_id: Uuid) -> Option<(String, String)> {
        let BackendKind::Memory(db) = &self.inner else {
            panic!("flow fixture is memory-only");
        };
        let db = db.lock().await;
        db.flow_states.values().find_map(|flow| {
            (flow.user_id == Some(user_id)).then(|| {
                (
                    flow.authentication_method.clone(),
                    flow.code_challenge.clone().unwrap_or_default(),
                )
            })
        })
    }

    /// A memory user with the given contacts and confirmation flags.
    pub async fn plant_contact_for_test(
        &self,
        email: &str,
        phone: &str,
        email_confirmed: bool,
        phone_confirmed: bool,
    ) -> Uuid {
        let BackendKind::Memory(db) = &self.inner else {
            panic!("contact fixture is memory-only");
        };
        let id = Uuid::new_v4();
        let now = SystemTime::now();
        let user = UserRecord {
            id,
            aud: "authenticated".into(),
            role: "authenticated".into(),
            email: email.to_string(),
            phone: phone.to_string(),
            email_confirmed_at: email_confirmed.then_some(now),
            last_sign_in_at: None,
            app_metadata: serde_json::json!({}),
            user_metadata: serde_json::json!({}),
            identities: Vec::new(),
            created_at: now,
            updated_at: now,
            is_anonymous: email.is_empty() && phone.is_empty(),
            banned_until: None,
            is_sso_user: false,
            password_hash: String::new(),
            phone_confirmed_at: phone_confirmed.then_some(now),
            confirmed_at: None,
        };
        db.lock().await.users.insert(id, user);
        id
    }

    async fn token_column_for_test<T: Default>(
        &self,
        email: &str,
        pick: impl Fn(&super::UserTokens) -> T,
    ) -> T {
        let BackendKind::Memory(db) = &self.inner else {
            panic!("token fixture is memory-only");
        };
        let db = db.lock().await;
        db.users
            .values()
            .find(|user| user.email.eq_ignore_ascii_case(email))
            .and_then(|user| db.tokens.get(&user.id))
            .map(pick)
            .unwrap_or_default()
    }
}
