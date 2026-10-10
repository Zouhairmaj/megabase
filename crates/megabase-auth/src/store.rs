// Ported from supabase/auth internal/api/signup.go, internal/api/token.go,
// internal/api/token_refresh.go, internal/api/user.go, internal/api/identity.go,
// internal/api/oauthserver/handlers.go, internal/models/user.go, sessions.go,
// refresh_token.go, amr.go, oauth_consent.go, oauth_client.go,
// internal/tokens/service.go, and internal/crypto/crypto.go (MIT), pin v2.197.0.

//! Auth users, identities, sessions, and legacy refresh tokens.
//!
//! Email autoconfirm signup writes `auth.users` the way GoTrue leaves the row
//! after a successful autoconfirm: nil `instance_id`, bcrypt cost 10, empty
//! confirmation tokens (not NULL), phone NULL, and `email_confirmed_at` set.
//! Refresh tokens are the legacy 12-character form (algorithm version 0).

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use serde_json::{json, Map, Value};
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::config::BCRYPT_COST;
use crate::schema::SchemaError;

const EMAIL_PROVIDER: &str = "email";

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database unavailable")]
    Unavailable,
    #[error("password hash failed")]
    Hash,
    #[error("database connection failed: {0}")]
    Connect(#[source] SchemaError),
    #[error("database operation timed out")]
    TimedOut,
    #[error(transparent)]
    Postgres(#[from] sqlx::Error),
}

/// Same bound the admin routes use for a connect. A stalled query fails
/// instead of holding the Auth client forever.
const QUERY_DEADLINE: Duration = Duration::from_secs(30);

enum WriteError {
    Conflict,
    Db(StoreError),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogoutScope {
    Global,
    Local,
    Others,
}

pub struct SignupCommand {
    pub email: String,
    pub password: String,
    pub aud: String,
    pub role: String,
    pub data: Map<String, Value>,
}

impl std::fmt::Debug for SignupCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SignupCommand")
            .field("email", &self.email)
            .field("password", &"<redacted>")
            .field("aud", &self.aud)
            .field("role", &self.role)
            .finish()
    }
}

#[derive(Clone, Debug)]
pub struct UserRecord {
    pub id: Uuid,
    pub aud: String,
    pub role: String,
    pub email: String,
    pub phone: String,
    pub email_confirmed_at: Option<SystemTime>,
    pub last_sign_in_at: Option<SystemTime>,
    pub app_metadata: Value,
    pub user_metadata: Value,
    pub identities: Vec<IdentityRecord>,
    pub created_at: SystemTime,
    pub updated_at: SystemTime,
    pub is_anonymous: bool,
    pub banned_until: Option<SystemTime>,
    pub is_sso_user: bool,
    pub password_hash: String,
    pub phone_confirmed_at: Option<SystemTime>,
    /// Generated `LEAST(email_confirmed_at, phone_confirmed_at)`. Populated
    /// when the row is loaded. Signup leaves it unset. Login and refresh
    /// fill it, including the in-memory store.
    pub confirmed_at: Option<SystemTime>,
}

#[derive(Clone, Debug)]
pub struct IdentityRecord {
    pub id: Uuid,
    pub provider_id: String,
    pub user_id: Uuid,
    pub identity_data: Value,
    pub provider: String,
    pub email: String,
    pub last_sign_in_at: Option<SystemTime>,
    pub created_at: SystemTime,
    pub updated_at: SystemTime,
}

#[derive(Clone, Debug)]
pub struct IssuedSession {
    pub user: UserRecord,
    pub refresh_token: String,
    pub session_id: Uuid,
    pub amr_at: SystemTime,
}

pub enum SignupResult {
    Created(Box<IssuedSession>),
    AlreadyExists,
}

/// Password-grant lookup. Phone numbers are stored without a leading `+`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoginChannel {
    Email,
    Phone,
}

/// Outcome of rotating one refresh token. HTTP status mapping lives in `token`.
///
/// `rotated` is false when the presented token is the parent of the active
/// child (the client missed the previous response). That retry returns the
/// child and does not mint another token.
#[derive(Debug)]
pub enum RefreshStatus {
    Issued {
        session: Box<IssuedSession>,
        rotated: bool,
    },
    NotFound,
    NoSession,
    Banned,
    AlreadyUsed,
}

#[derive(Clone, Debug)]
pub struct Subject {
    pub banned_until: Option<SystemTime>,
}

/// Writes from `PUT /user` after the HTTP checks have passed.
///
/// `password_hash` is already bcrypt. `phone` is E.164 without a leading `+`.
pub struct UserUpdate {
    pub user_id: Uuid,
    pub session_id: Option<Uuid>,
    pub data: Option<Map<String, Value>>,
    pub app_data: Option<Map<String, Value>>,
    pub password_hash: Option<String>,
    pub phone: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum UserUpdateError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("user missing")]
    Missing,
}

#[derive(Debug, thiserror::Error)]
pub enum UnlinkError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("user missing")]
    Missing,
    #[error("identity missing")]
    NotFound,
    #[error("email conflict")]
    EmailConflict,
    #[error("single identity")]
    Single,
}

/// One active OAuth consent joined to its client.
#[derive(Clone, Debug)]
pub struct OAuthGrantView {
    pub client_id: Uuid,
    pub name: String,
    pub uri: String,
    pub logo_uri: String,
    pub scopes: Vec<String>,
    pub granted_at: SystemTime,
}

pub enum RevokeGrant {
    Revoked,
    Missing,
}

/// Auth persistence. The variant is private so tests and HTTP share one type
/// without exposing the in-memory fixture layout.
#[derive(Clone)]
pub struct Backend {
    inner: BackendKind,
}

#[derive(Clone)]
enum BackendKind {
    None,
    Memory(Arc<Mutex<MemoryDb>>),
    Postgres(Pg),
}

/// Auth connection pool. Closed on process shutdown.
#[derive(Clone)]
struct Pg {
    pool: sqlx::PgPool,
}

#[derive(Clone)]
struct LegacyRefresh {
    user_id: Uuid,
    session_id: Uuid,
    revoked: bool,
    parent: String,
    /// AMR timestamp for the session. Refresh keeps this; it does not add a claim.
    amr_at: SystemTime,
}

struct MemoryDb {
    users: HashMap<Uuid, UserRecord>,
    sessions: HashMap<Uuid, Uuid>,
    refresh_tokens: HashMap<String, LegacyRefresh>,
    oauth_clients: HashMap<Uuid, MemoryClient>,
    oauth_consents: Vec<MemoryConsent>,
}

struct MemoryClient {
    name: String,
    uri: String,
    logo_uri: String,
    deleted: bool,
}

struct MemoryConsent {
    user_id: Uuid,
    client_id: Uuid,
    scopes: String,
    granted_at: SystemTime,
    revoked: bool,
}

impl Backend {
    pub fn none() -> Self {
        Self {
            inner: BackendKind::None,
        }
    }

    pub fn memory() -> Self {
        Self {
            inner: BackendKind::Memory(Arc::new(Mutex::new(MemoryDb {
                users: HashMap::new(),
                sessions: HashMap::new(),
                refresh_tokens: HashMap::new(),
                oauth_clients: HashMap::new(),
                oauth_consents: Vec::new(),
            }))),
        }
    }

    pub async fn connect(database_url: &str) -> Result<Self, SchemaError> {
        Ok(Self {
            inner: BackendKind::Postgres(Pg::connect(database_url).await?),
        })
    }

    /// Release the Auth database pool after the HTTP listener has stopped.
    pub async fn close(&self) {
        if let BackendKind::Postgres(pg) = &self.inner {
            pg.close().await;
        }
    }

    pub fn pg_pool(&self) -> Option<sqlx::PgPool> {
        match &self.inner {
            BackendKind::Postgres(pg) => Some(pg.pool.clone()),
            _ => None,
        }
    }

    pub async fn signup_email(&self, cmd: SignupCommand) -> Result<SignupResult, StoreError> {
        match &self.inner {
            BackendKind::None => Err(StoreError::Unavailable),
            BackendKind::Memory(db) => memory_signup(db, cmd).await,
            BackendKind::Postgres(pg) => timed(postgres_signup(pg, cmd)).await,
        }
    }

    pub async fn load_subject(&self, user_id: Uuid) -> Result<Option<Subject>, StoreError> {
        match &self.inner {
            BackendKind::None => Err(StoreError::Unavailable),
            BackendKind::Memory(db) => {
                let db = db.lock().await;
                Ok(db.users.get(&user_id).map(|user| Subject {
                    banned_until: user.banned_until,
                }))
            }
            BackendKind::Postgres(pg) => {
                timed(async {
                    let instance = Uuid::nil();
                    let row = sqlx::query!(
                        "SELECT banned_until FROM auth.users
                         WHERE instance_id = $1::uuid AND id = $2::uuid",
                        instance,
                        user_id,
                    )
                    .fetch_optional(&pg.pool)
                    .await?;
                    Ok(row.map(|row| Subject {
                        banned_until: row.banned_until.map(from_ts),
                    }))
                })
                .await
            }
        }
    }

    pub async fn session_exists(&self, session_id: Uuid) -> Result<bool, StoreError> {
        match &self.inner {
            BackendKind::None => Err(StoreError::Unavailable),
            BackendKind::Memory(db) => Ok(db.lock().await.sessions.contains_key(&session_id)),
            BackendKind::Postgres(pg) => {
                timed(async {
                    let row = sqlx::query!(
                        "SELECT 1 AS present FROM auth.sessions WHERE id = $1::uuid",
                        session_id,
                    )
                    .fetch_optional(&pg.pool)
                    .await?;
                    Ok(row.is_some())
                })
                .await
            }
        }
    }

    pub async fn logout(
        &self,
        user_id: Uuid,
        session_id: Option<Uuid>,
        scope: LogoutScope,
    ) -> Result<(), StoreError> {
        match &self.inner {
            BackendKind::None => Err(StoreError::Unavailable),
            BackendKind::Memory(db) => {
                let mut db = db.lock().await;
                if let Some(user) = db.users.get(&user_id).cloned() {
                    remember_audit(&user, "logout", "account", None);
                }
                apply_logout(&mut db, user_id, session_id, scope);
                Ok(())
            }
            BackendKind::Postgres(pg) => {
                timed(async {
                    let mut tx = pg.pool.begin().await?;
                    if let Some(user) = find_user_by_id(&mut tx, user_id).await? {
                        insert_audit(&mut tx, &user, "logout", "account", None).await?;
                    }
                    exec_logout(&mut tx, user_id, session_id, scope).await?;
                    tx.commit().await?;
                    Ok(())
                })
                .await
            }
        }
    }

    /// Email or phone lookup for the password grant. `identifier` is already
    /// normalized (lowercased email, or a phone with `+` and spaces removed).
    pub async fn find_login_user(
        &self,
        channel: LoginChannel,
        identifier: &str,
        aud: &str,
    ) -> Result<Option<UserRecord>, StoreError> {
        match &self.inner {
            BackendKind::None => Err(StoreError::Unavailable),
            BackendKind::Memory(db) => {
                let db = db.lock().await;
                let user = match channel {
                    LoginChannel::Email => db.find_email(identifier, aud).cloned(),
                    LoginChannel::Phone => db.find_phone(identifier, aud).cloned(),
                };
                Ok(user)
            }
            BackendKind::Postgres(pg) => {
                let identifier = identifier.to_string();
                let aud = aud.to_string();
                timed(async move {
                    let mut conn = pg.pool.acquire().await?;
                    match channel {
                        LoginChannel::Email => {
                            find_user_by_email_and_audience(&mut conn, &identifier, &aud).await
                        }
                        LoginChannel::Phone => find_phone(&mut conn, &identifier, &aud).await,
                    }
                })
                .await
            }
        }
    }

    /// New legacy session after a successful password check.
    ///
    /// Updates `last_sign_in_at` only. `replacement_hash` is set when GoTrue
    /// would re-encrypt a bcrypt cost other than the default.
    pub async fn issue_login_session(
        &self,
        user_id: Uuid,
        provider: &str,
        replacement_hash: Option<String>,
    ) -> Result<Option<IssuedSession>, StoreError> {
        match &self.inner {
            BackendKind::None => Err(StoreError::Unavailable),
            BackendKind::Memory(db) => {
                let mut db = db.lock().await;
                let Some(user) = db.users.get_mut(&user_id) else {
                    return Ok(None);
                };
                let now = SystemTime::now();
                user.last_sign_in_at = Some(now);
                if let Some(hash) = replacement_hash {
                    user.password_hash = hash;
                }
                let mut issued = grant_session(user, now);
                reload_confirmed_at(&mut issued.user);
                let traits = json!({ "provider": provider });
                remember_audit(&issued.user, "login", "account", Some(traits));
                db.track(&issued);
                Ok(Some(issued))
            }
            BackendKind::Postgres(pg) => {
                let provider = provider.to_string();
                timed(async move {
                    let mut tx = pg.pool.begin().await?;
                    let Some(mut user) = find_user_by_id(&mut tx, user_id).await? else {
                        tx.commit().await?;
                        return Ok(None);
                    };
                    let now = SystemTime::now();
                    user.last_sign_in_at = Some(now);
                    if let Some(hash) = &replacement_hash {
                        user.password_hash.clone_from(hash);
                    }
                    let instance = Uuid::nil();
                    let signed_in = ts(now);
                    sqlx::query!(
                        "UPDATE auth.users SET last_sign_in_at = $1::timestamptz
                         WHERE instance_id = $2::uuid AND id = $3::uuid",
                        signed_in,
                        instance,
                        user_id,
                    )
                    .execute(&mut *tx)
                    .await?;
                    if let Some(hash) = &replacement_hash {
                        sqlx::query!(
                            "UPDATE auth.users SET encrypted_password = $1
                             WHERE instance_id = $2::uuid AND id = $3::uuid",
                            hash,
                            instance,
                            user_id,
                        )
                        .execute(&mut *tx)
                        .await?;
                    }
                    let issued = grant_session(&mut user, now);
                    insert_session_rows(&mut tx, &issued)
                        .await
                        .map_err(write_to_store)?;
                    insert_audit(
                        &mut tx,
                        &issued.user,
                        "login",
                        "account",
                        Some(json!({ "provider": provider })),
                    )
                    .await?;
                    tx.commit().await?;
                    Ok(Some(issued))
                })
                .await
            }
        }
    }

    /// Rotate a legacy refresh token (algorithm version 0).
    pub async fn refresh_login(&self, token: &str) -> Result<RefreshStatus, StoreError> {
        match &self.inner {
            BackendKind::None => Err(StoreError::Unavailable),
            BackendKind::Memory(db) => {
                let mut db = db.lock().await;
                Ok(refresh_memory(&mut db, token))
            }
            BackendKind::Postgres(pg) => {
                let token = token.to_string();
                timed(refresh_postgres(pg, &token)).await
            }
        }
    }

    /// Full user row, including the generated `confirmed_at` column.
    pub async fn load_user(&self, user_id: Uuid) -> Result<Option<UserRecord>, StoreError> {
        match &self.inner {
            BackendKind::None => Err(StoreError::Unavailable),
            BackendKind::Memory(db) => {
                let db = db.lock().await;
                Ok(db.users.get(&user_id).cloned().map(|mut user| {
                    reload_confirmed_at(&mut user);
                    user
                }))
            }
            BackendKind::Postgres(pg) => {
                timed(async {
                    let mut conn = pg.pool.acquire().await?;
                    find_user_by_id(&mut conn, user_id).await
                })
                .await
            }
        }
    }

    /// Verified MFA factor with a session that is not AAL2.
    ///
    /// GoTrue refuses email, phone, and password changes in that case.
    pub async fn mfa_blocks_sensitive_update(
        &self,
        user_id: Uuid,
        session_id: Option<Uuid>,
    ) -> Result<bool, StoreError> {
        match &self.inner {
            BackendKind::None => Err(StoreError::Unavailable),
            BackendKind::Memory(_) => Ok(false),
            BackendKind::Postgres(pg) => {
                timed(async {
                    let mut conn = pg.pool.acquire().await?;
                    let verified = sqlx::query!(
                        "SELECT id FROM auth.mfa_factors
                         WHERE user_id = $1 AND status::text = 'verified'
                         LIMIT 1",
                        user_id,
                    )
                    .fetch_optional(&mut *conn)
                    .await?
                    .is_some();
                    if !verified {
                        return Ok(false);
                    }
                    let Some(session_id) = session_id else {
                        return Ok(true);
                    };
                    let row = sqlx::query!(
                        "SELECT COALESCE(aal::text, 'aal1') AS \"aal!\"
                         FROM auth.sessions WHERE id = $1",
                        session_id,
                    )
                    .fetch_optional(&mut *conn)
                    .await?;
                    let aal = row.map(|row| row.aal).unwrap_or_else(|| "aal1".to_string());
                    Ok(aal != "aal2")
                })
                .await
            }
        }
    }

    /// Another non-SSO user in `aud` already owns `email` (identity or `users.email`).
    pub async fn email_owned_by_other(
        &self,
        email: &str,
        aud: &str,
        user_id: Uuid,
    ) -> Result<bool, StoreError> {
        match &self.inner {
            BackendKind::None => Err(StoreError::Unavailable),
            BackendKind::Memory(db) => {
                let db = db.lock().await;
                Ok(email_taken(&db, email, aud, user_id))
            }
            BackendKind::Postgres(pg) => {
                let email = email.to_string();
                let aud = aud.to_string();
                timed(async move {
                    let mut conn = pg.pool.acquire().await?;
                    let instance = Uuid::nil();
                    let row = sqlx::query!(
                        "SELECT (
                            EXISTS (
                                SELECT 1 FROM auth.identities i
                                JOIN auth.users u ON u.id = i.user_id
                                WHERE lower(COALESCE(i.email, '')) = $1
                                  AND u.aud = $2
                                  AND u.id <> $3
                                  AND u.is_sso_user = false
                                  AND u.instance_id = $4
                                  AND i.provider NOT LIKE 'sso:%'
                            )
                            OR EXISTS (
                                SELECT 1 FROM auth.users
                                WHERE instance_id = $4
                                  AND lower(COALESCE(email, '')) = $1
                                  AND aud = $2
                                  AND id <> $3
                                  AND is_sso_user = false
                            )
                         ) AS \"taken!\"",
                        email,
                        aud,
                        user_id,
                        instance,
                    )
                    .fetch_one(&mut *conn)
                    .await?;
                    Ok(row.taken)
                })
                .await
            }
        }
    }

    /// Another non-SSO user in `aud` already has this phone.
    pub async fn phone_taken(&self, phone: &str, aud: &str) -> Result<bool, StoreError> {
        match &self.inner {
            BackendKind::None => Err(StoreError::Unavailable),
            BackendKind::Memory(db) => {
                let db = db.lock().await;
                Ok(db.find_phone(phone, aud).is_some())
            }
            BackendKind::Postgres(pg) => {
                let phone = phone.to_string();
                let aud = aud.to_string();
                timed(async move {
                    let mut conn = pg.pool.acquire().await?;
                    Ok(find_phone(&mut conn, &phone, &aud).await?.is_some())
                })
                .await
            }
        }
    }

    /// Metadata, password, and autoconfirmed phone changes from `PUT /user`.
    pub async fn update_user(&self, update: UserUpdate) -> Result<UserRecord, UserUpdateError> {
        match &self.inner {
            BackendKind::None => Err(StoreError::Unavailable.into()),
            BackendKind::Memory(db) => memory_update_user(db, update).await,
            BackendKind::Postgres(pg) => postgres_update_user(pg, update).await,
        }
    }

    /// `DELETE /user/identities/{identity_id}` after manual linking is enabled.
    pub async fn unlink_identity(
        &self,
        user_id: Uuid,
        identity_id: Uuid,
        autoconfirm: bool,
    ) -> Result<IdentityRecord, UnlinkError> {
        match &self.inner {
            BackendKind::None => Err(StoreError::Unavailable.into()),
            BackendKind::Memory(db) => {
                let mut db = db.lock().await;
                let Some(user) = db.users.get(&user_id).cloned() else {
                    return Err(UnlinkError::Missing);
                };
                let aud = user.aud.clone();
                let plan = plan_unlink(&user, identity_id, autoconfirm, &|email| {
                    user_has_email(&db, email, &aud)
                })?;
                remember_audit(
                    &user,
                    "identity_unlinked",
                    "user",
                    Some(unlink_traits(&plan.removed)),
                );
                let removed = plan.removed;
                db.users.insert(user_id, plan.user);
                Ok(removed)
            }
            BackendKind::Postgres(pg) => {
                postgres_unlink(pg, user_id, identity_id, autoconfirm).await
            }
        }
    }

    /// Active OAuth consents for `GET /user/oauth/grants`, newest first.
    pub async fn list_oauth_grants(
        &self,
        user_id: Uuid,
    ) -> Result<Vec<OAuthGrantView>, StoreError> {
        match &self.inner {
            BackendKind::None => Err(StoreError::Unavailable),
            BackendKind::Memory(db) => {
                let db = db.lock().await;
                Ok(memory_grants(&db, user_id))
            }
            BackendKind::Postgres(pg) => {
                timed(async {
                    let mut conn = pg.pool.acquire().await?;
                    let rows = sqlx::query!(
                        "SELECT c.id,
                                COALESCE(c.client_name, '') AS \"name!\",
                                COALESCE(c.client_uri, '') AS \"uri!\",
                                COALESCE(c.logo_uri, '') AS \"logo_uri!\",
                                s.scopes,
                                s.granted_at
                         FROM auth.oauth_consents s
                         JOIN auth.oauth_clients c
                           ON c.id = s.client_id AND c.deleted_at IS NULL
                         WHERE s.user_id = $1 AND s.revoked_at IS NULL
                         ORDER BY s.granted_at DESC",
                        user_id,
                    )
                    .fetch_all(&mut *conn)
                    .await?;
                    Ok(rows
                        .into_iter()
                        .map(|row| OAuthGrantView {
                            client_id: row.id,
                            name: row.name,
                            uri: row.uri,
                            logo_uri: row.logo_uri,
                            scopes: parse_scopes(&row.scopes),
                            granted_at: from_ts(row.granted_at),
                        })
                        .collect())
                })
                .await
            }
        }
    }

    /// `DELETE /user/oauth/grants?client_id=`.
    pub async fn revoke_oauth_grant(
        &self,
        user_id: Uuid,
        client_id: Uuid,
    ) -> Result<RevokeGrant, StoreError> {
        match &self.inner {
            BackendKind::None => Err(StoreError::Unavailable),
            BackendKind::Memory(db) => {
                let mut db = db.lock().await;
                let Some(consent) = db.oauth_consents.iter_mut().find(|consent| {
                    consent.user_id == user_id && consent.client_id == client_id && !consent.revoked
                }) else {
                    return Ok(RevokeGrant::Missing);
                };
                consent.revoked = true;
                if let Some(user) = db.users.get(&user_id).cloned() {
                    remember_audit(
                        &user,
                        "token_revoked",
                        "token",
                        Some(revoke_traits(client_id)),
                    );
                }
                Ok(RevokeGrant::Revoked)
            }
            BackendKind::Postgres(pg) => {
                timed(async {
                    let mut tx = pg.pool.begin().await?;
                    let now = ts(SystemTime::now());
                    let updated = sqlx::query!(
                        "UPDATE auth.oauth_consents SET revoked_at = $3
                         WHERE user_id = $1 AND client_id = $2
                           AND revoked_at IS NULL",
                        user_id,
                        client_id,
                        now,
                    )
                    .execute(&mut *tx)
                    .await?
                    .rows_affected();
                    if updated == 0 {
                        tx.commit().await?;
                        return Ok(RevokeGrant::Missing);
                    }
                    sqlx::query!(
                        "DELETE FROM auth.sessions
                         WHERE user_id = $1 AND oauth_client_id = $2",
                        user_id,
                        client_id,
                    )
                    .execute(&mut *tx)
                    .await?;
                    if let Some(user) = find_user_by_id(&mut tx, user_id).await? {
                        insert_audit(
                            &mut tx,
                            &user,
                            "token_revoked",
                            "token",
                            Some(revoke_traits(client_id)),
                        )
                        .await?;
                    }
                    tx.commit().await?;
                    Ok(RevokeGrant::Revoked)
                })
                .await
            }
        }
    }
}

#[cfg(test)]
impl Backend {
    pub async fn insert_unconfirmed_for_test(
        &self,
        email: &str,
        aud: &str,
        role: &str,
        password_hash: &str,
    ) {
        let BackendKind::Memory(db) = &self.inner else {
            panic!("unconfirmed fixture is memory-only");
        };
        let now = SystemTime::now();
        let id = Uuid::new_v4();
        let user = UserRecord {
            id,
            aud: aud.into(),
            role: role.into(),
            email: email.to_lowercase(),
            phone: String::new(),
            email_confirmed_at: None,
            last_sign_in_at: None,
            app_metadata: app_metadata(),
            user_metadata: json!({ "email_verified": false }),
            identities: vec![new_identity(id, &email.to_lowercase(), &Map::new(), now)],
            created_at: now,
            updated_at: now,
            is_anonymous: false,
            banned_until: None,
            is_sso_user: false,
            password_hash: password_hash.into(),
            phone_confirmed_at: None,
            confirmed_at: None,
        };
        db.lock().await.users.insert(id, user);
    }

    pub async fn password_hash_for_test(&self, email: &str) -> Option<String> {
        let BackendKind::Memory(db) = &self.inner else {
            return None;
        };
        let db = db.lock().await;
        db.find_email(&email.to_lowercase(), "authenticated")
            .map(|user| user.password_hash.clone())
    }

    pub async fn ban_for_test(&self, email: &str) {
        let BackendKind::Memory(db) = &self.inner else {
            panic!("ban fixture is memory-only");
        };
        let until = SystemTime::now() + std::time::Duration::from_secs(3600);
        let mut db = db.lock().await;
        let user = db
            .users
            .values_mut()
            .find(|user| user.email.eq_ignore_ascii_case(email) && user.aud == "authenticated")
            .expect("ban fixture user");
        user.banned_until = Some(until);
    }

    pub async fn set_phone_for_test(&self, email: &str, phone: &str, confirmed: bool) {
        let BackendKind::Memory(db) = &self.inner else {
            panic!("phone fixture is memory-only");
        };
        let mut db = db.lock().await;
        let user = db
            .users
            .values_mut()
            .find(|user| user.email.eq_ignore_ascii_case(email) && user.aud == "authenticated")
            .expect("phone fixture user");
        user.phone = phone.to_string();
        user.phone_confirmed_at = confirmed.then(SystemTime::now);
    }

    pub async fn add_identity_for_test(
        &self,
        email: &str,
        provider: &str,
        identity_email: &str,
        verified: bool,
    ) -> Uuid {
        let BackendKind::Memory(db) = &self.inner else {
            panic!("identity fixture is memory-only");
        };
        let mut db = db.lock().await;
        let user = db
            .users
            .values_mut()
            .find(|user| user.email.eq_ignore_ascii_case(email) && user.aud == "authenticated")
            .expect("identity fixture user");
        let now = SystemTime::now();
        let id = Uuid::new_v4();
        let mut data = Map::new();
        data.insert("sub".into(), json!(Uuid::new_v4().to_string()));
        data.insert("email".into(), json!(identity_email.to_lowercase()));
        data.insert("email_verified".into(), json!(verified));
        data.insert("phone_verified".into(), json!(false));
        user.identities.push(IdentityRecord {
            id,
            provider_id: Uuid::new_v4().to_string(),
            user_id: user.id,
            identity_data: Value::Object(data),
            provider: provider.into(),
            email: identity_email.to_lowercase(),
            last_sign_in_at: Some(now),
            created_at: now,
            updated_at: now,
        });
        id
    }

    pub async fn insert_oauth_grant_for_test(
        &self,
        email: &str,
        name: &str,
        scopes: &str,
        deleted: bool,
    ) -> Uuid {
        let BackendKind::Memory(db) = &self.inner else {
            panic!("oauth grant fixture is memory-only");
        };
        let mut db = db.lock().await;
        let user_id = db
            .users
            .values()
            .find(|user| user.email.eq_ignore_ascii_case(email) && user.aud == "authenticated")
            .expect("oauth grant fixture user")
            .id;
        let client_id = Uuid::new_v4();
        db.oauth_clients.insert(
            client_id,
            MemoryClient {
                name: name.into(),
                uri: "https://client.example".into(),
                logo_uri: String::new(),
                deleted,
            },
        );
        db.oauth_consents.push(MemoryConsent {
            user_id,
            client_id,
            scopes: scopes.into(),
            granted_at: SystemTime::now(),
            revoked: false,
        });
        client_id
    }

    pub async fn set_sso_for_test(&self, email: &str) {
        let BackendKind::Memory(db) = &self.inner else {
            panic!("sso fixture is memory-only");
        };
        let mut db = db.lock().await;
        let user = db
            .users
            .values_mut()
            .find(|user| user.email.eq_ignore_ascii_case(email) && user.aud == "authenticated")
            .expect("sso fixture user");
        user.is_sso_user = true;
    }

    pub async fn set_anonymous_for_test(&self, email: &str) {
        let BackendKind::Memory(db) = &self.inner else {
            panic!("anonymous fixture is memory-only");
        };
        let mut db = db.lock().await;
        let user = db
            .users
            .values_mut()
            .find(|user| user.email.eq_ignore_ascii_case(email) && user.aud == "authenticated")
            .expect("anonymous fixture user");
        user.is_anonymous = true;
    }
}

async fn memory_signup(
    db: &Mutex<MemoryDb>,
    cmd: SignupCommand,
) -> Result<SignupResult, StoreError> {
    let preexisting = {
        let guard = db.lock().await;
        guard
            .find_email(&cmd.email, &cmd.aud)
            .map(|user| user.email_confirmed_at.is_some())
    };
    if preexisting == Some(true) {
        let guard = db.lock().await;
        if let Some(user) = guard.find_email(&cmd.email, &cmd.aud).cloned() {
            if user.email_confirmed_at.is_some() {
                remember_audit(
                    &user,
                    "user_repeated_signup",
                    "user",
                    Some(provider_traits()),
                );
                return Ok(SignupResult::AlreadyExists);
            }
        }
    }
    if preexisting == Some(false) {
        let mut guard = db.lock().await;
        return Ok(SignupResult::Created(Box::new(
            guard.confirm_existing(&cmd)?,
        )));
    }
    let hash = hash_password(cmd.password.clone()).await?;
    let mut guard = db.lock().await;
    if let Some(user) = guard.find_email(&cmd.email, &cmd.aud).cloned() {
        if user.email_confirmed_at.is_some() {
            remember_audit(
                &user,
                "user_repeated_signup",
                "user",
                Some(provider_traits()),
            );
            return Ok(SignupResult::AlreadyExists);
        }
        return Ok(SignupResult::Created(Box::new(
            guard.confirm_existing(&cmd)?,
        )));
    }
    // `users_email_partial_key` is unique on email for every non-SSO user,
    // not per audience. A different `aud` still collides in PostgreSQL.
    if guard.non_sso_email_elsewhere(&cmd.email, &cmd.aud) {
        return Ok(SignupResult::AlreadyExists);
    }
    Ok(SignupResult::Created(Box::new(
        guard.insert_new(&cmd, hash),
    )))
}

impl MemoryDb {
    fn find_email(&self, email: &str, aud: &str) -> Option<&UserRecord> {
        self.users.values().find(|user| {
            !user.is_sso_user && user.aud == aud && user.email.eq_ignore_ascii_case(email)
        })
    }

    /// True when a non-SSO user already owns this email under another audience.
    fn non_sso_email_elsewhere(&self, email: &str, aud: &str) -> bool {
        self.users.values().any(|user| {
            !user.is_sso_user && user.aud != aud && user.email.eq_ignore_ascii_case(email)
        })
    }

    fn find_phone(&self, phone: &str, aud: &str) -> Option<&UserRecord> {
        self.users
            .values()
            .find(|user| !user.is_sso_user && user.aud == aud && user.phone == phone)
    }

    fn track(&mut self, issued: &IssuedSession) {
        self.sessions.insert(issued.session_id, issued.user.id);
        self.refresh_tokens.insert(
            issued.refresh_token.clone(),
            LegacyRefresh {
                user_id: issued.user.id,
                session_id: issued.session_id,
                revoked: false,
                parent: String::new(),
                amr_at: issued.amr_at,
            },
        );
    }

    fn confirm_existing(&mut self, cmd: &SignupCommand) -> Result<IssuedSession, StoreError> {
        let now = SystemTime::now();
        let user_id = self
            .find_email(&cmd.email, &cmd.aud)
            .map(|user| user.id)
            .ok_or(StoreError::Unavailable)?;
        let user = self
            .users
            .get_mut(&user_id)
            .ok_or(StoreError::Unavailable)?;
        user.email_confirmed_at = Some(now);
        user.last_sign_in_at = Some(now);
        user.updated_at = now;
        if !user
            .identities
            .iter()
            .any(|identity| identity.provider == EMAIL_PROVIDER)
        {
            let stored = new_identity(user.id, &user.email, &cmd.data, now);
            let (shown, meta) = shown_signup_identity(&stored);
            user.user_metadata = meta;
            user.identities.push(shown);
            let issued = grant_session(user, now);
            if let Some(identity) = user
                .identities
                .iter_mut()
                .find(|identity| identity.id == stored.id)
            {
                *identity = stored;
            }
            self.track(&issued);
            remember_audit(
                &issued.user,
                "user_signedup",
                "team",
                Some(provider_traits()),
            );
            remember_audit(&issued.user, "login", "account", Some(provider_traits()));
            return Ok(issued);
        } else {
            set_email_verified(&mut user.user_metadata);
        }
        let issued = grant_session(user, now);
        self.track(&issued);
        remember_audit(
            &issued.user,
            "user_signedup",
            "team",
            Some(provider_traits()),
        );
        remember_audit(&issued.user, "login", "account", Some(provider_traits()));
        Ok(issued)
    }

    fn insert_new(&mut self, cmd: &SignupCommand, password_hash: String) -> IssuedSession {
        let now = SystemTime::now();
        let id = Uuid::new_v4();
        let stored = new_identity(id, &cmd.email, &cmd.data, now);
        let (shown, meta) = shown_signup_identity(&stored);
        let mut user = UserRecord {
            id,
            aud: cmd.aud.clone(),
            role: cmd.role.clone(),
            email: cmd.email.clone(),
            phone: String::new(),
            email_confirmed_at: Some(now),
            last_sign_in_at: Some(now),
            app_metadata: app_metadata(),
            user_metadata: meta,
            identities: vec![shown],
            created_at: now,
            updated_at: now,
            is_anonymous: false,
            banned_until: None,
            is_sso_user: false,
            password_hash,
            phone_confirmed_at: None,
            confirmed_at: None,
        };
        let issued = grant_session(&mut user, now);
        self.track(&issued);
        remember_audit(&user, "user_signedup", "team", Some(provider_traits()));
        remember_audit(&user, "login", "account", Some(provider_traits()));
        // The signup response shows the post-confirm identity. The row keeps
        // `email_verified: false`, which `GET /user` reloads.
        user.identities = vec![stored];
        self.users.insert(id, user);
        issued
    }
}

/// PostgreSQL `LEAST` skips nulls, so one confirmed timestamp is enough.
fn reload_confirmed_at(user: &mut UserRecord) {
    user.confirmed_at = match (user.email_confirmed_at, user.phone_confirmed_at) {
        (Some(email), Some(phone)) => Some(email.min(phone)),
        (email, phone) => email.or(phone),
    };
}

fn grant_session(user: &mut UserRecord, now: SystemTime) -> IssuedSession {
    let session_id = Uuid::new_v4();
    let refresh_token = secure_alphanumeric(12);
    IssuedSession {
        user: user.clone(),
        refresh_token,
        session_id,
        amr_at: now,
    }
}

fn apply_logout(db: &mut MemoryDb, user_id: Uuid, session_id: Option<Uuid>, scope: LogoutScope) {
    match (session_id, scope) {
        (None, _) | (_, LogoutScope::Global) => {
            db.sessions.retain(|_, owner| *owner != user_id);
        }
        (Some(session_id), LogoutScope::Local) => {
            db.sessions.remove(&session_id);
        }
        (Some(session_id), LogoutScope::Others) => {
            db.sessions
                .retain(|id, owner| *id == session_id || *owner != user_id);
        }
    }
    db.refresh_tokens
        .retain(|_, row| db.sessions.contains_key(&row.session_id));
}

fn login_banned(until: Option<SystemTime>) -> bool {
    until.is_some_and(|until| SystemTime::now() < until)
}

fn refresh_memory(db: &mut MemoryDb, token: &str) -> RefreshStatus {
    let Some(row) = db.refresh_tokens.get(token).cloned() else {
        return RefreshStatus::NotFound;
    };
    let Some(mut user) = db.users.get(&row.user_id).cloned() else {
        return RefreshStatus::NotFound;
    };
    reload_confirmed_at(&mut user);
    if login_banned(user.banned_until) {
        return RefreshStatus::Banned;
    }
    if !db.sessions.contains_key(&row.session_id) {
        db.refresh_tokens.remove(token);
        return RefreshStatus::NoSession;
    }
    if row.revoked {
        let child = db.refresh_tokens.iter().find(|(_, other)| {
            !other.revoked && other.session_id == row.session_id && other.parent == token
        });
        if let Some((child_token, child)) = child {
            let child_token = child_token.clone();
            let session_id = child.session_id;
            return RefreshStatus::Issued {
                session: Box::new(IssuedSession {
                    user,
                    refresh_token: child_token,
                    session_id,
                    amr_at: row.amr_at,
                }),
                rotated: false,
            };
        }
        for other in db.refresh_tokens.values_mut() {
            if other.session_id == row.session_id {
                other.revoked = true;
            }
        }
        return RefreshStatus::AlreadyUsed;
    }
    if let Some(stored) = db.refresh_tokens.get_mut(token) {
        stored.revoked = true;
    }
    let refresh_token = secure_alphanumeric(12);
    db.refresh_tokens.insert(
        refresh_token.clone(),
        LegacyRefresh {
            user_id: row.user_id,
            session_id: row.session_id,
            revoked: false,
            parent: token.to_string(),
            amr_at: row.amr_at,
        },
    );
    RefreshStatus::Issued {
        session: Box::new(IssuedSession {
            user,
            refresh_token,
            session_id: row.session_id,
            amr_at: row.amr_at,
        }),
        rotated: true,
    }
}

/// Legacy refresh rotation against `auth.refresh_tokens`.
///
/// Algorithm version 0 (the reference stack leaves
/// `GOTRUE_SECURITY_REFRESH_TOKEN_ALGORITHM_VERSION` unset). A longer token is
/// not stored in `token`, and sessions from this crate have no HMAC key, so
/// the lookup misses. GoTrue maps that miss to refresh-token-not-found.
async fn refresh_postgres(pg: &Pg, token: &str) -> Result<RefreshStatus, StoreError> {
    let mut tx = pg.pool.begin().await?;
    let status = refresh_postgres_tx(&mut tx, token).await?;
    tx.commit().await?;
    Ok(status)
}

async fn refresh_postgres_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    token: &str,
) -> Result<RefreshStatus, StoreError> {
    let row = sqlx::query!(
        "SELECT user_id,
                COALESCE(revoked, false) AS revoked,
                session_id::text AS session_id,
                COALESCE(parent, '') AS parent
         FROM auth.refresh_tokens
         WHERE token = $1
         LIMIT 1
         FOR UPDATE",
        token,
    )
    .fetch_optional(&mut **tx)
    .await?;
    let Some(row) = row else {
        return Ok(RefreshStatus::NotFound);
    };
    let Some(user_id_text) = row.user_id.filter(|value| !value.is_empty()) else {
        return Ok(RefreshStatus::NotFound);
    };
    let user_id = parse_uuid(&user_id_text)?;
    let revoked = row.revoked.unwrap_or(false);
    let session_text = row.session_id;
    let Some(user) = find_user_by_id(&mut *tx, user_id).await? else {
        return Ok(RefreshStatus::NotFound);
    };
    if login_banned(user.banned_until) {
        return Ok(RefreshStatus::Banned);
    }
    let Some(session_text) = session_text.filter(|value| !value.is_empty()) else {
        sqlx::query!("DELETE FROM auth.refresh_tokens WHERE token = $1", token)
            .execute(&mut **tx)
            .await?;
        return Ok(RefreshStatus::NoSession);
    };
    let session_id = parse_uuid(&session_text)?;
    let session = sqlx::query!(
        "SELECT id FROM auth.sessions WHERE id = $1::uuid FOR UPDATE",
        session_id,
    )
    .fetch_optional(&mut **tx)
    .await?;
    if session.is_none() {
        sqlx::query!("DELETE FROM auth.refresh_tokens WHERE token = $1", token)
            .execute(&mut **tx)
            .await?;
        return Ok(RefreshStatus::NoSession);
    }
    let amr_at = session_amr_at(&mut *tx, session_id).await?;
    let mut issued_token = String::new();
    let mut rotated = false;
    if revoked {
        let active = sqlx::query!(
            "SELECT token, COALESCE(parent, '') AS parent
             FROM auth.refresh_tokens
             WHERE session_id = $1::uuid AND revoked IS FALSE
             ORDER BY id DESC
             LIMIT 1",
            session_id,
        )
        .fetch_optional(&mut **tx)
        .await?;
        if let Some(active) = active {
            if active.parent.as_deref() == Some(token) {
                if let Some(active_token) = active.token {
                    issued_token = active_token;
                }
            }
        }
        if issued_token.is_empty() {
            // Reuse interval is 0 and rotation is on: revoke the family, then fail.
            sqlx::query!(
                "UPDATE auth.refresh_tokens
                 SET revoked = true, updated_at = now()
                 WHERE session_id = $1::uuid AND revoked = false",
                session_id,
            )
            .execute(&mut **tx)
            .await?;
            return Ok(RefreshStatus::AlreadyUsed);
        }
    }
    insert_audit(&mut *tx, &user, "token_refreshed", "token", None).await?;
    if issued_token.is_empty() {
        let now = ts(SystemTime::now());
        let user_id_text = user.id.to_string();
        let instance = Uuid::nil();
        issued_token = secure_alphanumeric(12);
        rotated = true;
        insert_audit(&mut *tx, &user, "token_revoked", "token", None).await?;
        sqlx::query!(
            "UPDATE auth.refresh_tokens SET revoked = true, updated_at = $2::timestamptz
             WHERE token = $1",
            token,
            now,
        )
        .execute(&mut **tx)
        .await?;
        sqlx::query!(
            "INSERT INTO auth.refresh_tokens (
                instance_id, token, user_id, revoked, created_at, updated_at, parent, session_id
             ) VALUES (
                $1::uuid, $2, $3, false, $4::timestamptz, $4::timestamptz, $5, $6::uuid
             )",
            instance,
            issued_token,
            user_id_text,
            now,
            token,
            session_id,
        )
        .execute(&mut **tx)
        .await?;
    }
    sqlx::query!(
        "UPDATE auth.sessions
         SET refreshed_at = (now() AT TIME ZONE 'utc')
         WHERE id = $1::uuid",
        session_id,
    )
    .execute(&mut **tx)
    .await?;
    Ok(RefreshStatus::Issued {
        session: Box::new(IssuedSession {
            user,
            refresh_token: issued_token,
            session_id,
            amr_at,
        }),
        rotated,
    })
}

async fn session_amr_at(
    conn: &mut sqlx::PgConnection,
    session_id: Uuid,
) -> Result<SystemTime, StoreError> {
    let row = sqlx::query!(
        "SELECT created_at FROM auth.mfa_amr_claims
         WHERE session_id = $1::uuid
         ORDER BY created_at ASC
         LIMIT 1",
        session_id,
    )
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row
        .map(|row| from_ts(row.created_at))
        .unwrap_or_else(SystemTime::now))
}

impl Pg {
    async fn connect(url: &str) -> Result<Self, SchemaError> {
        // Reject `sslmode=require` before sqlx opens a socket. Schema install
        // uses the same check.
        crate::schema::reject_tls(url)?;
        let connect = sqlx::postgres::PgPoolOptions::new()
            .max_connections(10)
            .acquire_timeout(QUERY_DEADLINE)
            .connect(url);
        let pool = match tokio::time::timeout(QUERY_DEADLINE, connect).await {
            Ok(Ok(pool)) => pool,
            Ok(Err(error)) => return Err(SchemaError::Sqlx(error)),
            Err(_) => {
                return Err(SchemaError::TimedOut {
                    timeout: QUERY_DEADLINE,
                })
            }
        };
        Ok(Self { pool })
    }

    async fn close(&self) {
        self.pool.close().await;
    }
}

async fn timed<T, E>(fut: impl Future<Output = Result<T, E>>) -> Result<T, E>
where
    E: From<StoreError>,
{
    match tokio::time::timeout(QUERY_DEADLINE, fut).await {
        Ok(result) => result,
        Err(_) => {
            tracing::error!("auth database operation timed out");
            Err(StoreError::TimedOut.into())
        }
    }
}

fn ts(time: SystemTime) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::<chrono::Utc>::from(time)
}

fn from_ts(time: chrono::DateTime<chrono::Utc>) -> SystemTime {
    SystemTime::from(time)
}

async fn postgres_signup(pg: &Pg, cmd: SignupCommand) -> Result<SignupResult, StoreError> {
    let preexisting = {
        let mut conn = pg.pool.acquire().await?;
        find_email(&mut conn, &cmd.email, &cmd.aud)
            .await?
            .map(|user| user.email_confirmed_at.is_some())
    };
    let hash = if preexisting.is_some() {
        None
    } else {
        Some(hash_password(cmd.password.clone()).await?)
    };
    let mut tx = pg.pool.begin().await?;
    if let Some(user) = find_email(&mut tx, &cmd.email, &cmd.aud).await? {
        if user.email_confirmed_at.is_some() {
            insert_audit(
                &mut tx,
                &user,
                "user_repeated_signup",
                "user",
                Some(provider_traits()),
            )
            .await?;
            tx.commit().await?;
            return Ok(SignupResult::AlreadyExists);
        }
        let issued = confirm_existing_tx(&mut tx, user, &cmd).await?;
        tx.commit().await?;
        return Ok(SignupResult::Created(Box::new(issued)));
    }
    let Some(hash) = hash else {
        return Err(StoreError::Unavailable);
    };
    match insert_new_tx(&mut tx, &cmd, hash).await {
        Ok(issued) => {
            tx.commit().await?;
            Ok(SignupResult::Created(Box::new(issued)))
        }
        Err(WriteError::Conflict) => {
            drop(tx);
            audit_conflict(pg, &cmd).await
        }
        Err(WriteError::Db(error)) => Err(error),
    }
}

async fn audit_conflict(pg: &Pg, cmd: &SignupCommand) -> Result<SignupResult, StoreError> {
    let mut tx = pg.pool.begin().await?;
    if let Some(user) = find_email(&mut tx, &cmd.email, &cmd.aud).await? {
        insert_audit(
            &mut tx,
            &user,
            "user_repeated_signup",
            "user",
            Some(provider_traits()),
        )
        .await?;
    }
    tx.commit().await?;
    Ok(SignupResult::AlreadyExists)
}

async fn confirm_existing_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    mut user: UserRecord,
    cmd: &SignupCommand,
) -> Result<IssuedSession, StoreError> {
    let now = SystemTime::now();
    user.email_confirmed_at = Some(now);
    user.last_sign_in_at = Some(now);
    user.updated_at = now;
    if !user
        .identities
        .iter()
        .any(|identity| identity.provider == EMAIL_PROVIDER)
    {
        let stored = new_identity(user.id, &user.email, &cmd.data, now);
        let (shown, meta) = shown_signup_identity(&stored);
        insert_identity(tx, &stored).await.map_err(write_to_store)?;
        user.user_metadata = meta;
        user.identities.push(shown);
    } else {
        set_email_verified(&mut user.user_metadata);
    }
    let now_ts = ts(now);
    sqlx::query!(
        "UPDATE auth.users SET
            email_confirmed_at = $2::timestamptz,
            confirmation_token = '',
            last_sign_in_at = $2::timestamptz,
            raw_user_meta_data = $3::jsonb,
            updated_at = $2::timestamptz
         WHERE id = $1::uuid",
        user.id,
        now_ts,
        user.user_metadata,
    )
    .execute(&mut **tx)
    .await?;
    let issued = grant_session(&mut user, now);
    insert_session_rows(tx, &issued)
        .await
        .map_err(write_to_store)?;
    insert_audit(
        tx,
        &issued.user,
        "user_signedup",
        "team",
        Some(provider_traits()),
    )
    .await?;
    insert_audit(
        tx,
        &issued.user,
        "login",
        "account",
        Some(provider_traits()),
    )
    .await?;
    Ok(issued)
}

async fn insert_new_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    cmd: &SignupCommand,
    password_hash: String,
) -> Result<IssuedSession, WriteError> {
    let now = SystemTime::now();
    let id = Uuid::new_v4();
    let stored = new_identity(id, &cmd.email, &cmd.data, now);
    let (shown, meta) = shown_signup_identity(&stored);
    let mut user = UserRecord {
        id,
        aud: cmd.aud.clone(),
        role: cmd.role.clone(),
        email: cmd.email.clone(),
        phone: String::new(),
        email_confirmed_at: Some(now),
        last_sign_in_at: Some(now),
        app_metadata: app_metadata(),
        user_metadata: meta,
        identities: vec![shown],
        created_at: now,
        updated_at: now,
        is_anonymous: false,
        banned_until: None,
        is_sso_user: false,
        password_hash,
        phone_confirmed_at: None,
        confirmed_at: None,
    };
    insert_user(tx, &user).await?;
    insert_identity(tx, &stored).await?;
    let issued = grant_session(&mut user, now);
    insert_session_rows(tx, &issued).await?;
    insert_audit(tx, &user, "user_signedup", "team", Some(provider_traits()))
        .await
        .map_err(WriteError::Db)?;
    insert_audit(tx, &user, "login", "account", Some(provider_traits()))
        .await
        .map_err(WriteError::Db)?;
    Ok(issued)
}

async fn insert_user(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user: &UserRecord,
) -> Result<(), WriteError> {
    let confirmed = ts(user.email_confirmed_at.unwrap_or(user.created_at));
    let signed_in = ts(user.last_sign_in_at.unwrap_or(user.created_at));
    let created = ts(user.created_at);
    let instance = Uuid::nil();
    let result = sqlx::query!(
        "INSERT INTO auth.users (
            instance_id, id, aud, role, email, encrypted_password,
            email_confirmed_at, confirmation_token, recovery_token,
            email_change_token_new, email_change, email_change_token_current,
            email_change_confirm_status,
            phone, phone_change, phone_change_token, reauthentication_token,
            last_sign_in_at, raw_app_meta_data, raw_user_meta_data,
            is_sso_user, is_anonymous, created_at, updated_at
        ) VALUES (
            $1::uuid, $2::uuid, $3, $4, $5, $6,
            $7::timestamptz, '', '',
            '', '', '',
            0,
            NULL, '', '', '',
            $8::timestamptz, $9::jsonb, $10::jsonb,
            false, false, $11::timestamptz, $11::timestamptz
        )",
        instance,
        user.id,
        user.aud,
        user.role,
        user.email,
        user.password_hash,
        confirmed,
        signed_in,
        user.app_metadata,
        user.user_metadata,
        created,
    )
    .execute(&mut **tx)
    .await
    .map(|done| done.rows_affected());
    map_write(result)
}

async fn insert_identity(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    identity: &IdentityRecord,
) -> Result<(), WriteError> {
    let created = ts(identity.created_at);
    let result = sqlx::query!(
        "INSERT INTO auth.identities (
            id, provider_id, user_id, identity_data, provider,
            last_sign_in_at, created_at, updated_at
        ) VALUES (
            $1::uuid, $2, $3::uuid, $4::jsonb, $5,
            $6::timestamptz, $6::timestamptz, $6::timestamptz
        )",
        identity.id,
        identity.provider_id,
        identity.user_id,
        identity.identity_data,
        identity.provider,
        created,
    )
    .execute(&mut **tx)
    .await
    .map(|done| done.rows_affected());
    map_write(result)
}

async fn insert_session_rows(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    issued: &IssuedSession,
) -> Result<(), WriteError> {
    let instance = Uuid::nil();
    // `auth.refresh_tokens.user_id` is varchar, not uuid.
    let user_id = issued.user.id.to_string();
    let amr_id = Uuid::new_v4();
    let amr_at = ts(issued.amr_at);
    map_write(
        sqlx::query!(
            "INSERT INTO auth.sessions (id, user_id, created_at, updated_at, aal)
             VALUES ($1::uuid, $2::uuid, $3::timestamptz, $3::timestamptz, 'aal1')",
            issued.session_id,
            issued.user.id,
            amr_at,
        )
        .execute(&mut **tx)
        .await
        .map(|done| done.rows_affected()),
    )?;
    map_write(
        sqlx::query!(
            "INSERT INTO auth.refresh_tokens (
                instance_id, token, user_id, revoked, created_at, updated_at, session_id
             ) VALUES (
                $1::uuid, $2, $3, false, $4::timestamptz, $4::timestamptz, $5::uuid
             )",
            instance,
            issued.refresh_token,
            user_id,
            amr_at,
            issued.session_id,
        )
        .execute(&mut **tx)
        .await
        .map(|done| done.rows_affected()),
    )?;
    map_write(
        sqlx::query!(
            "INSERT INTO auth.mfa_amr_claims (
                id, session_id, created_at, updated_at, authentication_method
             ) VALUES (
                $1::uuid, $2::uuid, $3::timestamptz, $3::timestamptz, 'password'
             )",
            amr_id,
            issued.session_id,
            amr_at,
        )
        .execute(&mut **tx)
        .await
        .map(|done| done.rows_affected()),
    )?;
    Ok(())
}

async fn insert_audit(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user: &UserRecord,
    action: &str,
    log_type: &str,
    traits: Option<Value>,
) -> Result<(), StoreError> {
    let instance = Uuid::nil();
    let id = Uuid::new_v4();
    let now = ts(SystemTime::now());
    let payload = audit_payload(user, action, log_type, traits);
    sqlx::query!(
        "INSERT INTO auth.audit_log_entries (instance_id, id, payload, created_at, ip_address)
         VALUES ($1::uuid, $2::uuid, $3::json, $4::timestamptz, '')",
        instance,
        id,
        payload,
        now,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn exec_logout(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: Uuid,
    session_id: Option<Uuid>,
    scope: LogoutScope,
) -> Result<(), StoreError> {
    match (session_id, scope) {
        (None, _) | (_, LogoutScope::Global) => {
            sqlx::query!(
                "DELETE FROM auth.sessions WHERE user_id = $1::uuid",
                user_id,
            )
            .execute(&mut **tx)
            .await?;
        }
        (Some(session_id), LogoutScope::Local) => {
            sqlx::query!("DELETE FROM auth.sessions WHERE id = $1::uuid", session_id,)
                .execute(&mut **tx)
                .await?;
        }
        (Some(session_id), LogoutScope::Others) => {
            sqlx::query!(
                "DELETE FROM auth.sessions WHERE user_id = $1::uuid AND id <> $2::uuid",
                user_id,
                session_id,
            )
            .execute(&mut **tx)
            .await?;
        }
    }
    Ok(())
}

async fn find_user_by_email_and_audience(
    conn: &mut sqlx::PgConnection,
    email: &str,
    aud: &str,
) -> Result<Option<UserRecord>, StoreError> {
    let instance = Uuid::nil();
    let row = sqlx::query!(
        "SELECT id
         FROM auth.users
         WHERE instance_id = $1::uuid
           AND lower(email) = $2
           AND aud = $3
           AND is_sso_user = false
         LIMIT 1",
        instance,
        email,
        aud,
    )
    .fetch_optional(&mut *conn)
    .await?;
    match row {
        Some(row) => find_user_by_id(conn, row.id).await,
        None => Ok(None),
    }
}

async fn find_phone(
    conn: &mut sqlx::PgConnection,
    phone: &str,
    aud: &str,
) -> Result<Option<UserRecord>, StoreError> {
    let instance = Uuid::nil();
    let row = sqlx::query!(
        "SELECT id
         FROM auth.users
         WHERE instance_id = $1::uuid
           AND phone = $2
           AND aud = $3
           AND is_sso_user = false
         LIMIT 1",
        instance,
        phone,
        aud,
    )
    .fetch_optional(&mut *conn)
    .await?;
    match row {
        Some(row) => find_user_by_id(conn, row.id).await,
        None => Ok(None),
    }
}

async fn find_email(
    conn: &mut sqlx::PgConnection,
    email: &str,
    aud: &str,
) -> Result<Option<UserRecord>, StoreError> {
    let instance = Uuid::nil();
    let by_identity = sqlx::query!(
        "SELECT u.id::text AS id
         FROM auth.identities i
         JOIN auth.users u ON u.id = i.user_id
         WHERE i.email = $1
           AND u.aud = $2
           AND u.instance_id = $3::uuid
           AND u.is_sso_user = false
         LIMIT 1",
        email,
        aud,
        instance,
    )
    .fetch_optional(&mut *conn)
    .await?;
    if let Some(row) = by_identity {
        let id = parse_uuid(row.id.as_deref().unwrap_or(""))?;
        return find_user_by_id(conn, id).await;
    }
    let by_user = sqlx::query!(
        "SELECT id::text AS id FROM auth.users
         WHERE instance_id = $1::uuid
           AND lower(email) = $2
           AND aud = $3
           AND is_sso_user = false
         LIMIT 1",
        instance,
        email,
        aud,
    )
    .fetch_optional(&mut *conn)
    .await?;
    match by_user {
        Some(row) => find_user_by_id(conn, parse_uuid(row.id.as_deref().unwrap_or(""))?).await,
        None => Ok(None),
    }
}

async fn find_user_by_id(
    conn: &mut sqlx::PgConnection,
    id: Uuid,
) -> Result<Option<UserRecord>, StoreError> {
    let instance = Uuid::nil();
    let row = sqlx::query!(
        "SELECT id, aud, role, email, phone,
                email_confirmed_at, last_sign_in_at, created_at, updated_at,
                raw_app_meta_data, raw_user_meta_data,
                is_anonymous, banned_until, is_sso_user, encrypted_password,
                phone_confirmed_at, confirmed_at
         FROM auth.users
         WHERE instance_id = $1 AND id = $2",
        instance,
        id,
    )
    .fetch_optional(&mut *conn)
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let identities = load_identities(conn, row.id).await?;
    let now = SystemTime::now();
    Ok(Some(UserRecord {
        id: row.id,
        aud: row.aud.unwrap_or_default(),
        role: row.role.unwrap_or_default(),
        email: row.email.unwrap_or_default(),
        phone: row.phone.unwrap_or_default(),
        email_confirmed_at: row.email_confirmed_at.map(from_ts),
        last_sign_in_at: row.last_sign_in_at.map(from_ts),
        app_metadata: row.raw_app_meta_data.unwrap_or(Value::Null),
        user_metadata: row.raw_user_meta_data.unwrap_or(Value::Null),
        identities,
        created_at: row.created_at.map(from_ts).unwrap_or(now),
        updated_at: row.updated_at.map(from_ts).unwrap_or(now),
        is_anonymous: row.is_anonymous,
        banned_until: row.banned_until.map(from_ts),
        is_sso_user: row.is_sso_user,
        password_hash: row.encrypted_password.unwrap_or_default(),
        phone_confirmed_at: row.phone_confirmed_at.map(from_ts),
        confirmed_at: row.confirmed_at.map(from_ts),
    }))
}

async fn load_identities(
    conn: &mut sqlx::PgConnection,
    user_id: Uuid,
) -> Result<Vec<IdentityRecord>, StoreError> {
    let rows = sqlx::query!(
        "SELECT id, provider_id, user_id, identity_data, provider, email,
                last_sign_in_at, created_at, updated_at
         FROM auth.identities WHERE user_id = $1",
        user_id,
    )
    .fetch_all(&mut *conn)
    .await?;
    let mut identities = Vec::with_capacity(rows.len());
    for row in rows {
        let now = SystemTime::now();
        identities.push(IdentityRecord {
            id: row.id,
            provider_id: row.provider_id,
            user_id: row.user_id,
            identity_data: row.identity_data,
            provider: row.provider,
            email: row.email.unwrap_or_default(),
            last_sign_in_at: row.last_sign_in_at.map(from_ts),
            created_at: row.created_at.map(from_ts).unwrap_or(now),
            updated_at: row.updated_at.map(from_ts).unwrap_or(now),
        });
    }
    Ok(identities)
}

fn write_to_store(error: WriteError) -> StoreError {
    match error {
        WriteError::Conflict => StoreError::Unavailable,
        WriteError::Db(error) => error,
    }
}

fn map_write(result: Result<u64, sqlx::Error>) -> Result<(), WriteError> {
    match result {
        Ok(_) => Ok(()),
        Err(error) if unique_violation(&error) => Err(WriteError::Conflict),
        Err(error) => Err(WriteError::Db(error.into())),
    }
}

fn unique_violation(error: &sqlx::Error) -> bool {
    error
        .as_database_error()
        .and_then(|db| db.code())
        .is_some_and(|code| code == "23505")
}

fn parse_uuid(value: &str) -> Result<Uuid, StoreError> {
    Uuid::parse_str(value).map_err(|_| StoreError::Unavailable)
}

fn app_metadata() -> Value {
    json!({ "provider": EMAIL_PROVIDER, "providers": [EMAIL_PROVIDER] })
}

fn set_email_verified(metadata: &mut Value) {
    if let Value::Object(map) = metadata {
        map.insert("email_verified".into(), json!(true));
    } else {
        *metadata = json!({ "email_verified": true });
    }
}

/// `structs.Map` of `provider.Claims` for an email signup.
///
/// `email_verified` and `phone_verified` have no `omitempty` tag, so both
/// bools are stored. Request `data` fills only keys that are not already set
/// (`signup.go`).
fn identity_claims(user_id: Uuid, email: &str, data: &Map<String, Value>) -> Map<String, Value> {
    let mut claims = Map::new();
    claims.insert("sub".into(), json!(user_id.to_string()));
    claims.insert("email".into(), json!(email));
    claims.insert("email_verified".into(), json!(false));
    claims.insert("phone_verified".into(), json!(false));
    for (key, value) in data {
        claims.entry(key.clone()).or_insert_with(|| value.clone());
    }
    claims
}

/// Signup `Confirm` sets `email_verified` on the map
/// `RemoveUnconfirmedIdentities` assigned to both `UserMetaData` and
/// `IdentityData`. The identity row was inserted before that write, so it
/// keeps `email_verified: false`.
fn shown_signup_identity(stored: &IdentityRecord) -> (IdentityRecord, Value) {
    let mut shown = stored.clone();
    let meta = match &stored.identity_data {
        Value::Object(claims) => {
            let mut meta = claims.clone();
            meta.insert("email_verified".into(), json!(true));
            Value::Object(meta)
        }
        other => other.clone(),
    };
    shown.identity_data = meta.clone();
    (shown, meta)
}

fn new_identity(
    user_id: Uuid,
    email: &str,
    data: &Map<String, Value>,
    now: SystemTime,
) -> IdentityRecord {
    let identity_data = identity_claims(user_id, email, data);
    IdentityRecord {
        id: Uuid::new_v4(),
        provider_id: user_id.to_string(),
        user_id,
        identity_data: Value::Object(identity_data),
        provider: EMAIL_PROVIDER.into(),
        email: email.to_string(),
        last_sign_in_at: Some(now),
        created_at: now,
        updated_at: now,
    }
}

fn provider_traits() -> Value {
    json!({ "provider": EMAIL_PROVIDER })
}

fn audit_payload(user: &UserRecord, action: &str, log_type: &str, traits: Option<Value>) -> Value {
    let mut payload = Map::new();
    payload.insert("action".into(), json!(action));
    payload.insert("actor_id".into(), json!(user.id.to_string()));
    let username = if user.phone.is_empty() {
        user.email.as_str()
    } else {
        user.phone.as_str()
    };
    payload.insert("actor_username".into(), json!(username));
    payload.insert("actor_via_sso".into(), json!(user.is_sso_user));
    payload.insert("log_type".into(), json!(log_type));
    if let Some(traits) = traits {
        payload.insert("traits".into(), traits);
    }
    Value::Object(payload)
}

/// Memory-store stand-in for `insert_audit`.
///
/// The database row keeps the actor id and traits. The process log names the
/// action only, so user ids, identity ids, emails, and phones are not written
/// here.
fn remember_audit(_user: &UserRecord, action: &str, log_type: &str, traits: Option<Value>) {
    drop(traits);
    tracing::debug!(action, log_type, "auth audit");
}

pub(crate) async fn password_matches(password: &str, hash: &str) -> Result<bool, StoreError> {
    if hash.is_empty() {
        return Ok(false);
    }
    let password = password.to_string();
    let hash = hash.to_string();
    tokio::task::spawn_blocking(move || bcrypt::verify(password, &hash))
        .await
        .map_err(|_| StoreError::Hash)?
        .map_err(|_| StoreError::Hash)
}

pub(crate) async fn hash_new_password(password: String) -> Result<String, StoreError> {
    hash_password(password).await
}

async fn hash_password(password: String) -> Result<String, StoreError> {
    tokio::task::spawn_blocking(move || bcrypt::hash(password, BCRYPT_COST))
        .await
        .map_err(|_| StoreError::Hash)?
        .map_err(|_| StoreError::Hash)
}

fn secure_alphanumeric(length: usize) -> String {
    let length = length.max(8);
    let nbytes = (length * 5).div_ceil(8);
    let mut raw = Vec::with_capacity(nbytes);
    while raw.len() < nbytes {
        // UUIDv4 fixes the version nibble in byte 6 and the variant bits in
        // byte 8. GoTrue reads `crypto/rand` (`crypto.go` SecureAlphanumeric).
        // Those two bytes stay out of the token.
        let bytes = *Uuid::new_v4().as_bytes();
        raw.extend_from_slice(&bytes[0..6]);
        raw.push(bytes[7]);
        raw.extend_from_slice(&bytes[9..16]);
    }
    raw.truncate(nbytes);
    base32_lower(&raw).chars().take(length).collect()
}

fn base32_lower(data: &[u8]) -> String {
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz234567";
    let mut out = String::new();
    let mut buffer = 0u32;
    let mut bits = 0u32;
    for byte in data {
        buffer = (buffer << 8) | u32::from(*byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            let index = ((buffer >> bits) & 31) as usize;
            out.push(ALPHABET[index] as char);
        }
    }
    if bits > 0 {
        let index = ((buffer << (5 - bits)) & 31) as usize;
        out.push(ALPHABET[index] as char);
    }
    out
}

fn email_taken(db: &MemoryDb, email: &str, aud: &str, user_id: Uuid) -> bool {
    if email.is_empty() {
        return false;
    }
    db.users.values().any(|user| {
        if user.id == user_id || user.is_sso_user || user.aud != aud {
            return false;
        }
        user.email.eq_ignore_ascii_case(email)
            || user.identities.iter().any(|identity| {
                !identity.provider.starts_with("sso:") && identity.email.eq_ignore_ascii_case(email)
            })
    })
}

/// `users.email` already belongs to some non-SSO user in `aud`, including self.
fn user_has_email(db: &MemoryDb, email: &str, aud: &str) -> bool {
    if email.is_empty() {
        return false;
    }
    db.users
        .values()
        .any(|user| !user.is_sso_user && user.aud == aud && user.email.eq_ignore_ascii_case(email))
}

fn merge_meta(target: &mut Value, updates: &Map<String, Value>) {
    let Some(map) = target.as_object_mut() else {
        *target = Value::Object(updates.clone());
        return;
    };
    for (key, value) in updates {
        if value.is_null() {
            map.remove(key);
        } else {
            map.insert(key.clone(), value.clone());
        }
    }
}

fn apply_user_update(user: &mut UserRecord, update: &UserUpdate) {
    let now = SystemTime::now();
    let mut wrote = false;
    if let Some(hash) = &update.password_hash {
        user.password_hash.clone_from(hash);
        wrote = true;
    }
    if let Some(data) = &update.data {
        merge_meta(&mut user.user_metadata, data);
        wrote = true;
    }
    if let Some(data) = &update.app_data {
        merge_meta(&mut user.app_metadata, data);
        wrote = true;
    }
    if let Some(phone) = &update.phone {
        apply_phone(user, phone, now);
        wrote = true;
    }
    if wrote {
        user.updated_at = now;
    }
}

fn apply_phone(user: &mut UserRecord, phone: &str, now: SystemTime) {
    user.phone = phone.to_string();
    user.phone_confirmed_at = Some(now);
    user.is_anonymous = false;
    let provider_id = user.id.to_string();
    if let Some(identity) = user
        .identities
        .iter_mut()
        .find(|identity| identity.provider == "phone" && identity.provider_id == provider_id)
    {
        let mut patch = Map::new();
        patch.insert("phone".into(), json!(phone));
        patch.insert("phone_verified".into(), json!(true));
        merge_meta(&mut identity.identity_data, &patch);
        identity.updated_at = now;
    } else {
        user.identities.push(phone_identity(user.id, phone, now));
    }
}

fn phone_identity(user_id: Uuid, phone: &str, now: SystemTime) -> IdentityRecord {
    let mut claims = Map::new();
    claims.insert("sub".into(), json!(user_id.to_string()));
    claims.insert("email_verified".into(), json!(false));
    claims.insert("phone".into(), json!(phone));
    claims.insert("phone_verified".into(), json!(true));
    IdentityRecord {
        id: Uuid::new_v4(),
        provider_id: user_id.to_string(),
        user_id,
        identity_data: Value::Object(claims),
        provider: "phone".into(),
        email: String::new(),
        last_sign_in_at: Some(now),
        created_at: now,
        updated_at: now,
    }
}

struct UnlinkPlan {
    user: UserRecord,
    removed: IdentityRecord,
    clear_tokens: bool,
}

fn plan_unlink(
    user: &UserRecord,
    identity_id: Uuid,
    autoconfirm: bool,
    email_exists: &dyn Fn(&str) -> bool,
) -> Result<UnlinkPlan, UnlinkError> {
    if user.identities.len() <= 1 {
        return Err(UnlinkError::Single);
    }
    let mut next = user.clone();
    let index = next
        .identities
        .iter()
        .position(|identity| identity.id == identity_id)
        .ok_or(UnlinkError::NotFound)?;
    let removed = next.identities.remove(index);
    let mut clear_tokens = false;
    if removed.provider == "phone" {
        next.phone.clear();
        next.phone_confirmed_at = None;
    } else if !next
        .identities
        .iter()
        .any(|identity| identity.email.eq_ignore_ascii_case(&next.email))
    {
        let mut ranked = next.identities.clone();
        ranked.sort_by(|left, right| {
            email_rank(left)
                .cmp(&email_rank(right))
                .then_with(|| left.created_at.cmp(&right.created_at))
                .then_with(|| left.id.cmp(&right.id))
        });
        let Some(primary) = ranked
            .into_iter()
            .find(|identity| !email_exists(&identity.email))
        else {
            return Err(UnlinkError::EmailConflict);
        };
        let new_email = primary.email.to_lowercase();
        let verified = identity_email_verified(&primary);
        next.email = new_email.clone();
        clear_tokens = true;
        if new_email.is_empty() || (!verified && !autoconfirm) {
            next.email_confirmed_at = None;
            let mut patch = Map::new();
            patch.insert("email_verified".into(), json!(false));
            merge_meta(&mut next.user_metadata, &patch);
        }
    }
    refresh_providers(&mut next);
    next.updated_at = SystemTime::now();
    Ok(UnlinkPlan {
        user: next,
        removed,
        clear_tokens,
    })
}

fn email_rank(identity: &IdentityRecord) -> u8 {
    if identity.email.is_empty() {
        2
    } else if identity_email_verified(identity) {
        0
    } else {
        1
    }
}

fn identity_email_verified(identity: &IdentityRecord) -> bool {
    identity
        .identity_data
        .get("email_verified")
        .and_then(Value::as_bool)
        == Some(true)
}

fn refresh_providers(user: &mut UserRecord) {
    let mut ordered: Vec<&IdentityRecord> = user.identities.iter().collect();
    ordered.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.id.cmp(&right.id))
    });
    let mut providers = Vec::new();
    for identity in ordered {
        if !providers
            .iter()
            .any(|provider| provider == &identity.provider)
        {
            providers.push(identity.provider.clone());
        }
    }
    let mut patch = Map::new();
    patch.insert("providers".into(), json!(providers));
    if let Some(provider) = providers.first() {
        patch.insert("provider".into(), json!(provider));
    }
    merge_meta(&mut user.app_metadata, &patch);
}

fn unlink_traits(identity: &IdentityRecord) -> Value {
    json!({
        "identity_id": identity.id.to_string(),
        "provider": identity.provider,
        "provider_id": identity.provider_id,
    })
}

fn revoke_traits(client_id: Uuid) -> Value {
    json!({
        "oauth_client_id": client_id.to_string(),
        "action": "revoke_oauth_grant",
    })
}

fn parse_scopes(scopes: &str) -> Vec<String> {
    scopes
        .split(' ')
        .map(str::trim)
        .filter(|scope| !scope.is_empty())
        .map(str::to_string)
        .collect()
}

fn memory_grants(db: &MemoryDb, user_id: Uuid) -> Vec<OAuthGrantView> {
    let mut rows: Vec<&MemoryConsent> = db
        .oauth_consents
        .iter()
        .filter(|consent| consent.user_id == user_id && !consent.revoked)
        .collect();
    rows.sort_by_key(|consent| std::cmp::Reverse(consent.granted_at));
    rows.into_iter()
        .filter_map(|consent| {
            let client = db.oauth_clients.get(&consent.client_id)?;
            if client.deleted {
                return None;
            }
            Some(OAuthGrantView {
                client_id: consent.client_id,
                name: client.name.clone(),
                uri: client.uri.clone(),
                logo_uri: client.logo_uri.clone(),
                scopes: parse_scopes(&consent.scopes),
                granted_at: consent.granted_at,
            })
        })
        .collect()
}

async fn memory_update_user(
    db: &Mutex<MemoryDb>,
    update: UserUpdate,
) -> Result<UserRecord, UserUpdateError> {
    let mut db = db.lock().await;
    let Some(mut user) = db.users.get(&update.user_id).cloned() else {
        return Err(UserUpdateError::Missing);
    };
    finish_user_update(&mut db, &mut user, &update);
    reload_confirmed_at(&mut user);
    db.users.insert(user.id, user.clone());
    Ok(user)
}

fn finish_user_update(db: &mut MemoryDb, user: &mut UserRecord, update: &UserUpdate) {
    let before_phone = update.phone.is_some().then(|| user.clone());
    let password = update.password_hash.is_some();
    apply_user_update(user, update);
    if password {
        let scope = if update.session_id.is_some() {
            LogoutScope::Others
        } else {
            LogoutScope::Global
        };
        apply_logout(db, user.id, update.session_id, scope);
        let actor = before_phone.as_ref().unwrap_or(user);
        remember_audit(actor, "user_updated_password", "user", None);
    }
    if let Some(actor) = &before_phone {
        remember_audit(actor, "user_modified", "user", None);
    }
    remember_audit(user, "user_modified", "user", None);
}

async fn postgres_update_user(pg: &Pg, update: UserUpdate) -> Result<UserRecord, UserUpdateError> {
    timed(async move {
        let mut tx = pg.pool.begin().await.map_err(StoreError::from)?;
        let Some(mut user) = find_user_by_id(&mut tx, update.user_id).await? else {
            return Err(UserUpdateError::Missing);
        };
        let before_phone = update.phone.is_some().then(|| user.clone());
        let before: Vec<(Uuid, Value)> = user
            .identities
            .iter()
            .map(|identity| (identity.id, identity.identity_data.clone()))
            .collect();
        apply_user_update(&mut user, &update);
        if let Some(hash) = &update.password_hash {
            clear_password_row(&mut tx, &user, hash).await?;
            let scope = if update.session_id.is_some() {
                LogoutScope::Others
            } else {
                LogoutScope::Global
            };
            exec_logout(&mut tx, user.id, update.session_id, scope).await?;
            let actor = before_phone.as_ref().unwrap_or(&user);
            insert_audit(&mut tx, actor, "user_updated_password", "user", None).await?;
        }
        if update.data.is_some() || update.app_data.is_some() || update.phone.is_some() {
            write_profile(&mut tx, &user).await?;
        }
        for identity in &user.identities {
            if let Some((_, previous)) = before.iter().find(|(id, _)| *id == identity.id) {
                if previous != &identity.identity_data {
                    let updated_at = ts(identity.updated_at);
                    sqlx::query!(
                        "UPDATE auth.identities
                         SET identity_data = $2::jsonb, updated_at = $3::timestamptz
                         WHERE id = $1::uuid",
                        identity.id,
                        identity.identity_data,
                        updated_at,
                    )
                    .execute(&mut *tx)
                    .await
                    .map_err(StoreError::from)?;
                }
            } else {
                insert_identity(&mut tx, identity)
                    .await
                    .map_err(write_to_store)?;
            }
        }
        if let Some(actor) = &before_phone {
            insert_audit(&mut tx, actor, "user_modified", "user", None).await?;
        }
        insert_audit(&mut tx, &user, "user_modified", "user", None).await?;
        let user_id = user.id;
        tx.commit().await.map_err(StoreError::from)?;
        let mut conn = pg.pool.acquire().await.map_err(StoreError::from)?;
        find_user_by_id(&mut conn, user_id)
            .await?
            .ok_or(UserUpdateError::Missing)
    })
    .await
}

async fn clear_password_row(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user: &UserRecord,
    hash: &str,
) -> Result<(), StoreError> {
    let updated_at = ts(user.updated_at);
    sqlx::query!(
        "UPDATE auth.users SET
            encrypted_password = $3,
            confirmation_token = '',
            confirmation_sent_at = NULL,
            recovery_token = '',
            recovery_sent_at = NULL,
            email_change_token_current = '',
            email_change_token_new = '',
            email_change_sent_at = NULL,
            phone_change_token = '',
            phone_change_sent_at = NULL,
            reauthentication_token = '',
            reauthentication_sent_at = NULL,
            updated_at = $4::timestamptz
         WHERE instance_id = $1::uuid AND id = $2::uuid",
        Uuid::nil(),
        user.id,
        hash,
        updated_at,
    )
    .execute(&mut **tx)
    .await?;
    sqlx::query!(
        "DELETE FROM auth.one_time_tokens WHERE user_id = $1",
        user.id,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn write_profile(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user: &UserRecord,
) -> Result<(), StoreError> {
    let updated_at = ts(user.updated_at);
    let phone_confirmed_at = user.phone_confirmed_at.map(ts);
    sqlx::query!(
        "UPDATE auth.users SET
            raw_app_meta_data = $3::jsonb,
            raw_user_meta_data = $4::jsonb,
            phone = NULLIF($5, ''),
            phone_confirmed_at = $6::timestamptz,
            is_anonymous = $7,
            updated_at = $8::timestamptz
         WHERE instance_id = $1::uuid AND id = $2::uuid",
        Uuid::nil(),
        user.id,
        user.app_metadata,
        user.user_metadata,
        user.phone,
        phone_confirmed_at,
        user.is_anonymous,
        updated_at,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn postgres_unlink(
    pg: &Pg,
    user_id: Uuid,
    identity_id: Uuid,
    autoconfirm: bool,
) -> Result<IdentityRecord, UnlinkError> {
    timed(async move {
        let mut tx = pg.pool.begin().await.map_err(StoreError::from)?;
        let Some(user) = find_user_by_id(&mut tx, user_id).await? else {
            return Err(UnlinkError::Missing);
        };
        let mut taken = HashSet::new();
        for identity in &user.identities {
            if email_row_exists(&mut tx, &identity.email, &user.aud).await? {
                taken.insert(identity.email.to_lowercase());
            }
        }
        let plan = plan_unlink(&user, identity_id, autoconfirm, &|email| {
            taken.contains(&email.to_lowercase())
        })?;
        insert_audit(
            &mut tx,
            &user,
            "identity_unlinked",
            "user",
            Some(unlink_traits(&plan.removed)),
        )
        .await?;
        sqlx::query!(
            "DELETE FROM auth.identities WHERE id = $1::uuid AND user_id = $2::uuid",
            plan.removed.id,
            user_id,
        )
        .execute(&mut *tx)
        .await
        .map_err(StoreError::from)?;
        write_unlinked_user(&mut tx, &plan).await?;
        let removed = plan.removed;
        tx.commit().await.map_err(StoreError::from)?;
        Ok(removed)
    })
    .await
}

async fn email_row_exists(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    email: &str,
    aud: &str,
) -> Result<bool, StoreError> {
    if email.is_empty() {
        return Ok(false);
    }
    let email = email.to_lowercase();
    let row = sqlx::query!(
        r#"SELECT EXISTS (
            SELECT 1 FROM auth.users
            WHERE instance_id = $1::uuid
              AND lower(email) = $2
              AND aud = $3
              AND is_sso_user = false
        ) AS "taken!""#,
        Uuid::nil(),
        email,
        aud,
    )
    .fetch_one(&mut **tx)
    .await?;
    Ok(row.taken)
}

async fn write_unlinked_user(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    plan: &UnlinkPlan,
) -> Result<(), StoreError> {
    let user = &plan.user;
    let updated_at = ts(user.updated_at);
    let email_confirmed_at = user.email_confirmed_at.map(ts);
    let phone_confirmed_at = user.phone_confirmed_at.map(ts);
    if plan.clear_tokens {
        sqlx::query!(
            "UPDATE auth.users SET
                email = NULLIF($3, ''),
                phone = NULLIF($4, ''),
                email_confirmed_at = $5::timestamptz,
                phone_confirmed_at = $6::timestamptz,
                raw_app_meta_data = $7::jsonb,
                raw_user_meta_data = $8::jsonb,
                updated_at = $9::timestamptz,
                confirmation_token = '',
                confirmation_sent_at = NULL,
                recovery_token = '',
                recovery_sent_at = NULL,
                email_change = '',
                email_change_token_current = '',
                email_change_token_new = '',
                email_change_sent_at = NULL,
                email_change_confirm_status = 0,
                phone_change = '',
                phone_change_token = '',
                phone_change_sent_at = NULL,
                reauthentication_token = '',
                reauthentication_sent_at = NULL
             WHERE instance_id = $1::uuid AND id = $2::uuid",
            Uuid::nil(),
            user.id,
            user.email,
            user.phone,
            email_confirmed_at,
            phone_confirmed_at,
            user.app_metadata,
            user.user_metadata,
            updated_at,
        )
        .execute(&mut **tx)
        .await?;
        sqlx::query!(
            "DELETE FROM auth.one_time_tokens WHERE user_id = $1",
            user.id,
        )
        .execute(&mut **tx)
        .await?;
    } else {
        sqlx::query!(
            "UPDATE auth.users SET
                email = NULLIF($3, ''),
                phone = NULLIF($4, ''),
                email_confirmed_at = $5::timestamptz,
                phone_confirmed_at = $6::timestamptz,
                raw_app_meta_data = $7::jsonb,
                raw_user_meta_data = $8::jsonb,
                updated_at = $9::timestamptz
             WHERE instance_id = $1::uuid AND id = $2::uuid",
            Uuid::nil(),
            user.id,
            user.email,
            user.phone,
            email_confirmed_at,
            phone_confirmed_at,
            user.app_metadata,
            user.user_metadata,
            updated_at,
        )
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refresh_token_is_legacy_base32() {
        let token = secure_alphanumeric(12);
        assert_eq!(token.len(), 12);
        assert!(token.chars().all(|ch| matches!(ch, 'a'..='z' | '2'..='7')));
    }

    #[test]
    fn base32_matches_rfc4648_lowercase() {
        assert_eq!(base32_lower(b"f"), "my");
    }

    #[test]
    fn native_params_match_postgres_types() {
        use tokio_postgres::types::{ToSql, Type};
        assert!(<Uuid as ToSql>::accepts(&Type::UUID));
        assert!(<SystemTime as ToSql>::accepts(&Type::TIMESTAMPTZ));
        assert!(<Value as ToSql>::accepts(&Type::JSONB));
        assert!(<Value as ToSql>::accepts(&Type::JSON));
    }
}
