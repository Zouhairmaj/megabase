// Ported from supabase/auth internal/api/signup.go (Signup, signupNewUser),
// internal/api/mail.go (sendConfirmation), internal/api/pkce.go
// (generateFlowState), internal/models/user.go
// (RemoveUnconfirmedIdentities), and internal/models/one_time_token.go (MIT),
// pin v2.197.0.

//! User, flow state, and token writes for an email signup with
//! `GOTRUE_MAILER_AUTOCONFIRM=false`.

use std::time::SystemTime;

use serde_json::{json, Map, Value};
use uuid::Uuid;

use super::invite::insert_unconfirmed_user;
use super::pkce::insert_flow_state;
use super::{
    app_metadata, find_email, find_user_by_id, insert_audit, insert_identity, load_user_tokens,
    new_identity, provider_traits, remember_audit, timed, ts, write_to_store, Backend, BackendKind,
    FlowStateRecord, MemoryDb, OneTimeToken, StoreError, UserRecord, EMAIL_PROVIDER,
    OTT_CONFIRMATION,
};

const CONFIRMATION_REQUESTED: &str = "user_confirmation_requested";
const REPEATED_SIGNUP: &str = "user_repeated_signup";

/// One signup after the mail hand-off has passed.
pub struct ConfirmationSignup<'a> {
    /// Lowercased, validated email.
    pub email: &'a str,
    pub aud: &'a str,
    /// Role a new user gets (`GOTRUE_JWT_DEFAULT_GROUP_NAME`).
    pub role: &'a str,
    /// Request `data`; seeds the identity of a new user.
    pub data: &'a Map<String, Value>,
    /// bcrypt hash of the password. Only a new user stores it; `None` when
    /// the lookup found a user.
    pub password_hash: Option<String>,
    /// SHA-224 of email + OTP, with a `pkce_` prefix in the PKCE flow;
    /// stored as `confirmation_token`.
    pub token_hash: &'a str,
    /// PKCE flow: the `email/signup` flow state to write. `user_id` is set
    /// here.
    pub flow: Option<&'a FlowStateRecord>,
    pub sent_at: SystemTime,
}

/// Result of [`Backend::save_signup_confirmation`].
pub enum ConfirmationOutcome {
    /// The confirmation email was recorded. `invited_at` is set when the user
    /// was invited (`HasBeenInvited`).
    Sent {
        user: Box<UserRecord>,
        invited_at: Option<SystemTime>,
    },
    /// A confirmed user owns the email: only the `user_repeated_signup`
    /// audit entry is written. The user is what `sanitizeUser` starts from.
    Repeated(Box<UserRecord>),
}

/// What [`Backend::signup_confirmation_lookup`] found.
pub struct ConfirmationLookup {
    pub confirmed: bool,
    pub confirmation_sent_at: Option<SystemTime>,
}

impl Backend {
    /// The signup lookup (`IsDuplicatedEmail`) and the user's
    /// `confirmation_sent_at`, for the frequency limit `sendConfirmation`
    /// checks before it sends.
    ///
    /// # Errors
    ///
    /// [`StoreError`] when the database is missing or the read fails.
    pub async fn signup_confirmation_lookup(
        &self,
        email: &str,
        aud: &str,
    ) -> Result<Option<ConfirmationLookup>, StoreError> {
        match &self.inner {
            BackendKind::None => Err(StoreError::Unavailable),
            BackendKind::Memory(db) => {
                let db = db.lock().await;
                Ok(db.find_email(email, aud).map(|user| ConfirmationLookup {
                    confirmed: user.email_confirmed_at.is_some(),
                    confirmation_sent_at: db
                        .tokens
                        .get(&user.id)
                        .and_then(|tokens| tokens.confirmation_sent_at),
                }))
            }
            BackendKind::Postgres(pg) => {
                timed(async {
                    let mut conn = pg.pool.acquire().await?;
                    let Some(user) = find_email(&mut conn, email, aud).await? else {
                        return Ok(None);
                    };
                    let tokens = load_user_tokens(&mut conn, user.id).await?;
                    Ok(Some(ConfirmationLookup {
                        confirmed: user.email_confirmed_at.is_some(),
                        confirmation_sent_at: tokens.confirmation_sent_at,
                    }))
                })
                .await
            }
        }
    }

    /// The `Signup` transaction for an unconfirmed email signup: create the
    /// user (or reuse an unconfirmed one), the `user_confirmation_requested`
    /// audit entry, the PKCE flow state, then `sendConfirmation`'s writes
    /// (`confirmation_token`, `confirmation_sent_at`, and a
    /// `confirmation_token` one-time token that replaces the previous one).
    ///
    /// # Errors
    ///
    /// [`StoreError`] when the database is missing, a write fails, or a new
    /// user has no password hash; nothing is kept then.
    pub async fn save_signup_confirmation(
        &self,
        cmd: ConfirmationSignup<'_>,
    ) -> Result<ConfirmationOutcome, StoreError> {
        match &self.inner {
            BackendKind::None => Err(StoreError::Unavailable),
            BackendKind::Memory(db) => save_memory(&mut *db.lock().await, &cmd),
            BackendKind::Postgres(pg) => {
                timed(async {
                    let mut tx = pg.pool.begin().await?;
                    let user = match find_email(&mut tx, cmd.email, cmd.aud).await? {
                        Some(user) if user.email_confirmed_at.is_some() => {
                            insert_audit(
                                &mut tx,
                                &user,
                                REPEATED_SIGNUP,
                                "user",
                                Some(provider_traits()),
                            )
                            .await?;
                            tx.commit().await?;
                            return Ok(ConfirmationOutcome::Repeated(Box::new(user)));
                        }
                        Some(user) => keep_email_identity_pg(&mut tx, user, &cmd).await?,
                        None => {
                            let user = new_user(&cmd)?;
                            insert_unconfirmed_user(&mut tx, &user).await?;
                            user
                        }
                    };
                    insert_audit(
                        &mut tx,
                        &user,
                        CONFIRMATION_REQUESTED,
                        "user",
                        Some(provider_traits()),
                    )
                    .await?;
                    if let Some(flow) = cmd.flow {
                        let mut flow = flow.clone();
                        flow.user_id = Some(user.id);
                        insert_flow_state(&mut tx, Uuid::new_v4(), &flow, cmd.sent_at).await?;
                    }
                    save_token_pg(&mut tx, &cmd, user.id).await?;
                    let mut user = find_user_by_id(&mut tx, user.id)
                        .await?
                        .ok_or(StoreError::Unavailable)?;
                    user.identities
                        .retain(|identity| identity.provider == EMAIL_PROVIDER);
                    let invited_at = load_user_tokens(&mut tx, user.id).await?.invited_at;
                    tx.commit().await?;
                    Ok(ConfirmationOutcome::Sent {
                        user: Box::new(user),
                        invited_at,
                    })
                })
                .await
            }
        }
    }
}

/// `ToUserModel` for a new email user, then `createNewIdentity` and
/// `RemoveUnconfirmedIdentities`: `user_metadata` becomes the identity data
/// (`sub`, `email`, both `*_verified` false, then the request `data`).
fn new_user(cmd: &ConfirmationSignup<'_>) -> Result<UserRecord, StoreError> {
    let password_hash = cmd.password_hash.clone().ok_or(StoreError::Unavailable)?;
    let id = Uuid::new_v4();
    let now = cmd.sent_at;
    let identity = new_identity(id, cmd.email, cmd.data, now);
    Ok(UserRecord {
        id,
        aud: cmd.aud.to_string(),
        role: cmd.role.to_string(),
        email: cmd.email.to_string(),
        phone: String::new(),
        email_confirmed_at: None,
        last_sign_in_at: None,
        app_metadata: app_metadata(),
        user_metadata: identity.identity_data.clone(),
        identities: vec![identity],
        created_at: now,
        updated_at: now,
        is_anonymous: false,
        banned_until: None,
        is_sso_user: false,
        password_hash,
        phone_confirmed_at: None,
        confirmed_at: None,
    })
}

/// An unconfirmed user keeps its password and data. Only a missing email
/// identity is created, and then `RemoveUnconfirmedIdentities` makes it the
/// user's only identity, its data the `user_metadata`, and `email` the only
/// provider.
fn ensure_email_identity(user: &mut UserRecord, cmd: &ConfirmationSignup<'_>) -> bool {
    if user
        .identities
        .iter()
        .any(|identity| identity.provider == EMAIL_PROVIDER)
    {
        return false;
    }
    let identity = new_identity(user.id, &user.email, cmd.data, cmd.sent_at);
    user.user_metadata = identity.identity_data.clone();
    user.identities = vec![identity];
    if let Value::Object(meta) = &mut user.app_metadata {
        meta.insert("providers".into(), json!([EMAIL_PROVIDER]));
    }
    true
}

async fn keep_email_identity_pg(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    mut user: UserRecord,
    cmd: &ConfirmationSignup<'_>,
) -> Result<UserRecord, StoreError> {
    if !ensure_email_identity(&mut user, cmd) {
        return Ok(user);
    }
    let identity = &user.identities[0];
    insert_identity(tx, identity)
        .await
        .map_err(write_to_store)?;
    sqlx::query("DELETE FROM auth.identities WHERE user_id = $1::uuid AND id <> $2::uuid")
        .bind(user.id)
        .bind(identity.id)
        .execute(&mut **tx)
        .await?;
    sqlx::query(
        "UPDATE auth.users
         SET raw_user_meta_data = $2::jsonb, raw_app_meta_data = $3::jsonb, updated_at = $4
         WHERE id = $1::uuid",
    )
    .bind(user.id)
    .bind(&user.user_metadata)
    .bind(&user.app_metadata)
    .bind(ts(cmd.sent_at))
    .execute(&mut **tx)
    .await?;
    Ok(user)
}

/// The writes at the end of `sendConfirmation`.
async fn save_token_pg(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    cmd: &ConfirmationSignup<'_>,
    user_id: Uuid,
) -> Result<(), StoreError> {
    let sent = ts(cmd.sent_at);
    sqlx::query(
        "UPDATE auth.users
         SET confirmation_token = $2, confirmation_sent_at = $3, updated_at = $3
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

fn save_memory(
    db: &mut MemoryDb,
    cmd: &ConfirmationSignup<'_>,
) -> Result<ConfirmationOutcome, StoreError> {
    let mut user = match db.find_email(cmd.email, cmd.aud) {
        Some(user) if user.email_confirmed_at.is_some() => {
            remember_audit(user, REPEATED_SIGNUP, "user", Some(provider_traits()));
            return Ok(ConfirmationOutcome::Repeated(Box::new(user.clone())));
        }
        Some(user) => {
            let mut user = user.clone();
            ensure_email_identity(&mut user, cmd);
            user
        }
        // `users_email_partial_key` is unique on email for every non-SSO
        // user, so another audience's row makes the insert fail.
        None if db.non_sso_email_elsewhere(cmd.email, cmd.aud) => {
            return Err(StoreError::Unavailable);
        }
        None => new_user(cmd)?,
    };
    let id = user.id;
    user.updated_at = cmd.sent_at;
    remember_audit(
        &user,
        CONFIRMATION_REQUESTED,
        "user",
        Some(provider_traits()),
    );
    if let Some(flow) = cmd.flow {
        let mut flow = flow.clone();
        flow.user_id = Some(id);
        flow.created_at = Some(cmd.sent_at);
        db.flow_states.insert(Uuid::new_v4(), flow);
    }
    db.users.insert(id, user.clone());
    let tokens = db.tokens.entry(id).or_default();
    tokens.confirmation_token = cmd.token_hash.to_string();
    tokens.confirmation_sent_at = Some(cmd.sent_at);
    let invited_at = tokens.invited_at;
    db.one_time
        .retain(|ott| !(ott.user_id == id && ott.token_type == OTT_CONFIRMATION));
    db.one_time.push(OneTimeToken {
        user_id: id,
        token_type: OTT_CONFIRMATION.into(),
        token_hash: cmd.token_hash.to_string(),
    });
    user.identities
        .retain(|identity| identity.provider == EMAIL_PROVIDER);
    Ok(ConfirmationOutcome::Sent {
        user: Box::new(user),
        invited_at,
    })
}

#[cfg(test)]
impl Backend {
    /// `confirmation_token` of the memory user with `email`, empty when
    /// unset.
    pub async fn confirmation_token_for_test(&self, email: &str) -> String {
        let BackendKind::Memory(db) = &self.inner else {
            panic!("confirmation fixture is memory-only");
        };
        let db = db.lock().await;
        db.users
            .values()
            .find(|user| user.email.eq_ignore_ascii_case(email))
            .and_then(|user| db.tokens.get(&user.id))
            .map(|tokens| tokens.confirmation_token.clone())
            .unwrap_or_default()
    }
}
