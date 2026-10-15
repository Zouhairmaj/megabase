// Ported from supabase/auth internal/api/invite.go (Invite),
// internal/api/mail.go (sendInvite), internal/api/signup.go (ToUserModel,
// signupNewUser), internal/models/user.go (NewUser), and
// internal/models/one_time_token.go (MIT), pin v2.197.0.

//! User and token writes for `POST /invite`.

use std::time::SystemTime;

use serde_json::{json, Map, Value};
use uuid::Uuid;

use super::{
    app_metadata, find_user_by_email_and_audience, find_user_by_id, insert_identity, map_write,
    new_identity, timed, ts, write_to_store, Backend, BackendKind, MemoryDb, OneTimeToken,
    StoreError, UserRecord, OTT_CONFIRMATION,
};

const USER_INVITED: &str = "user_invited";

/// One invite after the HTTP checks and the mail hand-off have passed.
pub struct InviteCommand<'a> {
    /// Lowercased, validated email.
    pub email: &'a str,
    pub aud: &'a str,
    /// Role a new user gets (`GOTRUE_JWT_DEFAULT_GROUP_NAME`).
    pub role: &'a str,
    /// Request `data`; becomes `user_metadata` of a new user only.
    pub data: Map<String, Value>,
    /// SHA-224 of email + OTP, stored as `confirmation_token`.
    pub token_hash: &'a str,
    /// Role claim of the admin JWT, the audit `actor_username`.
    pub admin_role: &'a str,
    pub sent_at: SystemTime,
}

/// Result of [`Backend::invite_user`].
pub enum InviteOutcome {
    /// New or re-invited user, as GoTrue returns it. `invited_at` and
    /// `confirmation_sent_at` are both `sent_at`.
    Invited(Box<UserRecord>),
    /// A confirmed user owns the email in this audience (`email_exists`).
    EmailExists,
}

impl Backend {
    /// Create the invited user (or reuse an unconfirmed one), record the
    /// `user_invited` audit entry, and store the confirmation token. One
    /// transaction, as GoTrue's `Invite`.
    ///
    /// # Errors
    ///
    /// [`StoreError`] when the database is missing or a write fails; nothing
    /// is kept then.
    pub async fn invite_user(&self, cmd: InviteCommand<'_>) -> Result<InviteOutcome, StoreError> {
        match &self.inner {
            BackendKind::None => Err(StoreError::Unavailable),
            BackendKind::Memory(db) => Ok(invite_memory(&mut *db.lock().await, &cmd)),
            BackendKind::Postgres(pg) => {
                timed(async {
                    let mut tx = pg.pool.begin().await?;
                    let existing =
                        find_user_by_email_and_audience(&mut tx, cmd.email, cmd.aud).await?;
                    let id = match existing {
                        Some(user) if user.email_confirmed_at.is_some() => {
                            return Ok(InviteOutcome::EmailExists);
                        }
                        Some(user) => user.id,
                        None => {
                            let user = new_invited_user(&cmd);
                            insert_invited_user(&mut tx, &user).await?;
                            user.id
                        }
                    };
                    insert_admin_audit(&mut tx, &cmd, id).await?;
                    save_invite_token(&mut tx, &cmd, id).await?;
                    let user = find_user_by_id(&mut tx, id)
                        .await?
                        .ok_or(StoreError::Unavailable)?;
                    tx.commit().await?;
                    Ok(InviteOutcome::Invited(Box::new(user)))
                })
                .await
            }
        }
    }
}

/// `SignupParams.ToUserModel` for the email provider with no password:
/// empty `encrypted_password`, `data` as `user_metadata`, and one email
/// identity whose data is `structs.Map(provider.Claims{Subject, Email})`.
fn new_invited_user(cmd: &InviteCommand<'_>) -> UserRecord {
    let id = Uuid::new_v4();
    let now = cmd.sent_at;
    UserRecord {
        id,
        aud: cmd.aud.to_string(),
        role: cmd.role.to_string(),
        email: cmd.email.to_string(),
        phone: String::new(),
        email_confirmed_at: None,
        last_sign_in_at: None,
        app_metadata: app_metadata(),
        user_metadata: Value::Object(cmd.data.clone()),
        identities: vec![new_identity(id, cmd.email, &Map::new(), now)],
        created_at: now,
        updated_at: now,
        is_anonymous: false,
        banned_until: None,
        is_sso_user: false,
        password_hash: String::new(),
        phone_confirmed_at: None,
        confirmed_at: None,
    }
}

fn invite_memory(db: &mut MemoryDb, cmd: &InviteCommand<'_>) -> InviteOutcome {
    let mut user = match db.find_email(cmd.email, cmd.aud) {
        Some(user) if user.email_confirmed_at.is_some() => return InviteOutcome::EmailExists,
        Some(user) => user.clone(),
        None => new_invited_user(cmd),
    };
    user.updated_at = cmd.sent_at;
    let id = user.id;
    db.users.insert(id, user.clone());
    let tokens = db.tokens.entry(id).or_default();
    tokens.confirmation_token = cmd.token_hash.to_string();
    tokens.confirmation_sent_at = Some(cmd.sent_at);
    tokens.invited_at = Some(cmd.sent_at);
    db.one_time
        .retain(|ott| !(ott.user_id == id && ott.token_type == OTT_CONFIRMATION));
    db.one_time.push(OneTimeToken {
        user_id: id,
        token_type: OTT_CONFIRMATION.into(),
        token_hash: cmd.token_hash.to_string(),
    });
    tracing::debug!(action = USER_INVITED, log_type = "team", "auth audit");
    InviteOutcome::Invited(Box::new(user))
}

async fn insert_invited_user(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user: &UserRecord,
) -> Result<(), StoreError> {
    let created = ts(user.created_at);
    let inserted = sqlx::query(
        "INSERT INTO auth.users (
            instance_id, id, aud, role, email, encrypted_password,
            confirmation_token, recovery_token, email_change_token_new, email_change,
            email_change_token_current, email_change_confirm_status,
            phone_change, phone_change_token, reauthentication_token,
            raw_app_meta_data, raw_user_meta_data,
            is_sso_user, is_anonymous, created_at, updated_at
        ) VALUES (
            $1::uuid, $2::uuid, $3, $4, $5, '',
            '', '', '', '',
            '', 0,
            '', '', '',
            $6::jsonb, $7::jsonb,
            false, false, $8::timestamptz, $8::timestamptz
        )",
    )
    .bind(Uuid::nil())
    .bind(user.id)
    .bind(&user.aud)
    .bind(&user.role)
    .bind(&user.email)
    .bind(&user.app_metadata)
    .bind(&user.user_metadata)
    .bind(created)
    .execute(&mut **tx)
    .await
    .map(|done| done.rows_affected());
    // A unique violation (an SSO or other-audience row owns the email) is a
    // database error here, as in GoTrue's `signupNewUser`.
    map_write(inserted).map_err(write_to_store)?;
    for identity in &user.identities {
        insert_identity(tx, identity)
            .await
            .map_err(write_to_store)?;
    }
    Ok(())
}

/// `models.NewAuditLogEntry` with the synthetic admin user `requireAdmin`
/// builds: nil id, the role claim as username.
async fn insert_admin_audit(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    cmd: &InviteCommand<'_>,
    user_id: Uuid,
) -> Result<(), StoreError> {
    let payload = json!({
        "action": USER_INVITED,
        "actor_id": Uuid::nil().to_string(),
        "actor_username": cmd.admin_role,
        "actor_via_sso": false,
        "log_type": "team",
        "traits": { "user_id": user_id.to_string(), "user_email": cmd.email },
    });
    sqlx::query(
        "INSERT INTO auth.audit_log_entries (instance_id, id, payload, created_at, ip_address)
         VALUES ($1::uuid, $2::uuid, $3::json, $4::timestamptz, '')",
    )
    .bind(Uuid::nil())
    .bind(Uuid::new_v4())
    .bind(payload)
    .bind(ts(cmd.sent_at))
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// The writes at the end of `sendInvite`.
async fn save_invite_token(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    cmd: &InviteCommand<'_>,
    user_id: Uuid,
) -> Result<(), StoreError> {
    let sent = ts(cmd.sent_at);
    sqlx::query(
        "UPDATE auth.users
         SET confirmation_token = $2, confirmation_sent_at = $3, invited_at = $3, updated_at = $3
         WHERE id = $1::uuid",
    )
    .bind(user_id)
    .bind(cmd.token_hash)
    .bind(sent)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "DELETE FROM auth.one_time_tokens
         WHERE user_id = $1::uuid AND token_type = $2::auth.one_time_token_type",
    )
    .bind(user_id)
    .bind(OTT_CONFIRMATION)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "INSERT INTO auth.one_time_tokens
             (id, user_id, token_type, token_hash, relates_to, created_at, updated_at)
         VALUES ($1::uuid, $2::uuid, $3::auth.one_time_token_type, $4, $5, $6, $6)",
    )
    .bind(Uuid::new_v4())
    .bind(user_id)
    .bind(OTT_CONFIRMATION)
    .bind(cmd.token_hash)
    .bind(cmd.email.to_lowercase())
    .bind(sent)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

#[cfg(test)]
impl Backend {
    /// `confirmation_token` and `invited_at` of the memory user with `email`.
    pub async fn invite_token_for_test(&self, email: &str) -> (String, Option<SystemTime>) {
        let BackendKind::Memory(db) = &self.inner else {
            panic!("invite fixture is memory-only");
        };
        let db = db.lock().await;
        db.users
            .values()
            .find(|user| user.email.eq_ignore_ascii_case(email))
            .and_then(|user| db.tokens.get(&user.id))
            .map(|tokens| (tokens.confirmation_token.clone(), tokens.invited_at))
            .unwrap_or_default()
    }
}
