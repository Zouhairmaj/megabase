// Ported from supabase/auth internal/api/signup.go, internal/api/token.go,
// internal/api/token_refresh.go, internal/models/user.go, sessions.go,
// refresh_token.go, amr.go, internal/tokens/service.go, and
// internal/crypto/crypto.go (MIT), pin v2.197.0.

//! Auth users, identities, sessions, and legacy refresh tokens.
//!
//! Email autoconfirm signup writes `auth.users` the way GoTrue leaves the row
//! after a successful autoconfirm: nil `instance_id`, bcrypt cost 10, empty
//! confirmation tokens (not NULL), phone NULL, and `email_confirmed_at` set.
//! Refresh tokens are the legacy 12-character form (algorithm version 0).

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use serde_json::{json, Map, Value};
use tokio::sync::{Mutex, MutexGuard};
use tokio_postgres::GenericClient;
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
    Postgres(#[from] tokio_postgres::Error),
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

/// One Auth connection, replaced when PostgreSQL closes it.
#[derive(Clone)]
struct Pg {
    url: String,
    client: Arc<Mutex<tokio_postgres::Client>>,
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
            }))),
        }
    }

    pub async fn connect(database_url: &str) -> Result<Self, SchemaError> {
        Ok(Self {
            inner: BackendKind::Postgres(Pg::connect(database_url).await?),
        })
    }

    pub async fn signup_email(&self, cmd: SignupCommand) -> Result<SignupResult, StoreError> {
        match &self.inner {
            BackendKind::None => Err(StoreError::Unavailable),
            BackendKind::Memory(db) => memory_signup(db, cmd).await,
            BackendKind::Postgres(pg) => timed(pg, postgres_signup(pg, cmd)).await,
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
                timed(pg, async {
                    let instance = Uuid::nil();
                    let client = pg.lock().await?;
                    let row = client
                        .query_opt(
                            "SELECT banned_until FROM auth.users
                         WHERE instance_id = $1::uuid AND id = $2::uuid",
                            &[&instance, &user_id],
                        )
                        .await?;
                    Ok(row.map(|row| Subject {
                        banned_until: row.get(0),
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
                timed(pg, async {
                    let client = pg.lock().await?;
                    let row = client
                        .query_opt(
                            "SELECT 1 FROM auth.sessions WHERE id = $1::uuid",
                            &[&session_id],
                        )
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
                timed(pg, async {
                    let mut client = pg.lock().await?;
                    let tx = client.transaction().await?;
                    if let Some(user) = find_user_by_id(&tx, user_id).await? {
                        insert_audit(&tx, &user, "logout", "account", None).await?;
                    }
                    exec_logout(&tx, user_id, session_id, scope).await?;
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
                timed(pg, async move {
                    let client = pg.lock().await?;
                    match channel {
                        LoginChannel::Email => {
                            find_user_by_email_and_audience(&*client, &identifier, &aud).await
                        }
                        LoginChannel::Phone => find_phone(&*client, &identifier, &aud).await,
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
                timed(pg, async move {
                    let mut client = pg.lock().await?;
                    let tx = client.transaction().await?;
                    let Some(mut user) = find_user_by_id(&tx, user_id).await? else {
                        tx.commit().await?;
                        return Ok(None);
                    };
                    let now = SystemTime::now();
                    user.last_sign_in_at = Some(now);
                    if let Some(hash) = &replacement_hash {
                        user.password_hash.clone_from(hash);
                    }
                    tx.execute(
                        "UPDATE auth.users SET last_sign_in_at = $1::timestamptz
                         WHERE instance_id = $2::uuid AND id = $3::uuid",
                        &[&now, &Uuid::nil(), &user_id],
                    )
                    .await?;
                    if let Some(hash) = &replacement_hash {
                        tx.execute(
                            "UPDATE auth.users SET encrypted_password = $1
                             WHERE instance_id = $2::uuid AND id = $3::uuid",
                            &[hash, &Uuid::nil(), &user_id],
                        )
                        .await?;
                    }
                    let issued = grant_session(&mut user, now);
                    insert_session_rows(&tx, &issued)
                        .await
                        .map_err(write_to_store)?;
                    insert_audit(
                        &tx,
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
                timed(pg, async move { refresh_postgres(pg, &token).await }).await
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
            let (shown, meta) =
                shown_signup_identity(&new_identity(user.id, &user.email, &cmd.data, now));
            user.user_metadata = meta;
            user.identities.push(shown);
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
        let (shown, meta) = shown_signup_identity(&new_identity(id, &cmd.email, &cmd.data, now));
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
    let mut client = pg.lock().await?;
    let tx = client.transaction().await?;
    let status = refresh_postgres_tx(&tx, token).await?;
    tx.commit().await?;
    Ok(status)
}

async fn refresh_postgres_tx(
    tx: &tokio_postgres::Transaction<'_>,
    token: &str,
) -> Result<RefreshStatus, StoreError> {
    let row = tx
        .query_opt(
            "SELECT user_id, COALESCE(revoked, false), session_id::text, COALESCE(parent, '')
             FROM auth.refresh_tokens
             WHERE token = $1
             LIMIT 1
             FOR UPDATE",
            &[&token],
        )
        .await?;
    let Some(row) = row else {
        return Ok(RefreshStatus::NotFound);
    };
    let user_id = parse_uuid(&row.get::<_, String>(0))?;
    let revoked: bool = row.get(1);
    let session_text: Option<String> = row.get(2);
    let Some(user) = find_user_by_id(tx, user_id).await? else {
        return Ok(RefreshStatus::NotFound);
    };
    if login_banned(user.banned_until) {
        return Ok(RefreshStatus::Banned);
    }
    let Some(session_text) = session_text.filter(|value| !value.is_empty()) else {
        tx.execute(
            "DELETE FROM auth.refresh_tokens WHERE token = $1",
            &[&token],
        )
        .await?;
        return Ok(RefreshStatus::NoSession);
    };
    let session_id = parse_uuid(&session_text)?;
    let session = tx
        .query_opt(
            "SELECT id::text FROM auth.sessions WHERE id = $1::uuid FOR UPDATE",
            &[&session_id],
        )
        .await?;
    if session.is_none() {
        tx.execute(
            "DELETE FROM auth.refresh_tokens WHERE token = $1",
            &[&token],
        )
        .await?;
        return Ok(RefreshStatus::NoSession);
    }
    let amr_at = session_amr_at(tx, session_id).await?;
    let mut issued_token = String::new();
    let mut rotated = false;
    if revoked {
        let active = tx
            .query_opt(
                "SELECT token, COALESCE(parent, '')
                 FROM auth.refresh_tokens
                 WHERE session_id = $1::uuid AND revoked IS FALSE
                 ORDER BY id DESC
                 LIMIT 1",
                &[&session_id],
            )
            .await?;
        if let Some(active) = active {
            let active_token: String = active.get(0);
            let active_parent: String = active.get(1);
            if active_parent == token {
                issued_token = active_token;
            }
        }
        if issued_token.is_empty() {
            // Reuse interval is 0 and rotation is on: revoke the family, then fail.
            tx.execute(
                "UPDATE auth.refresh_tokens
                 SET revoked = true, updated_at = now()
                 WHERE session_id = $1::uuid AND revoked = false",
                &[&session_id],
            )
            .await?;
            return Ok(RefreshStatus::AlreadyUsed);
        }
    }
    insert_audit(tx, &user, "token_refreshed", "token", None).await?;
    if issued_token.is_empty() {
        let now = SystemTime::now();
        let user_id_text = user.id.to_string();
        issued_token = secure_alphanumeric(12);
        rotated = true;
        insert_audit(tx, &user, "token_revoked", "token", None).await?;
        tx.execute(
            "UPDATE auth.refresh_tokens SET revoked = true, updated_at = $2::timestamptz
             WHERE token = $1",
            &[&token, &now],
        )
        .await?;
        tx.execute(
            "INSERT INTO auth.refresh_tokens (
                instance_id, token, user_id, revoked, created_at, updated_at, parent, session_id
             ) VALUES (
                $1::uuid, $2, $3, false, $4::timestamptz, $4::timestamptz, $5, $6::uuid
             )",
            &[
                &Uuid::nil(),
                &issued_token,
                &user_id_text,
                &now,
                &token,
                &session_id,
            ],
        )
        .await?;
    }
    tx.execute(
        "UPDATE auth.sessions
         SET refreshed_at = (now() AT TIME ZONE 'utc')
         WHERE id = $1::uuid",
        &[&session_id],
    )
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
    tx: &tokio_postgres::Transaction<'_>,
    session_id: Uuid,
) -> Result<SystemTime, StoreError> {
    let row = tx
        .query_opt(
            "SELECT created_at FROM auth.mfa_amr_claims
             WHERE session_id = $1::uuid
             ORDER BY created_at ASC
             LIMIT 1",
            &[&session_id],
        )
        .await?;
    Ok(row.map(|row| row.get(0)).unwrap_or_else(SystemTime::now))
}

impl Pg {
    async fn connect(url: &str) -> Result<Self, SchemaError> {
        let client = connect_deadline(url).await?;
        Ok(Self {
            url: url.to_string(),
            client: Arc::new(Mutex::new(client)),
        })
    }

    async fn lock(&self) -> Result<MutexGuard<'_, tokio_postgres::Client>, StoreError> {
        let mut guard = self.client.lock().await;
        if guard.is_closed() {
            tracing::warn!("auth postgres connection closed; reconnecting");
            *guard = connect_deadline(&self.url)
                .await
                .map_err(StoreError::Connect)?;
        }
        Ok(guard)
    }

    async fn reconnect(&self) -> Result<(), StoreError> {
        let client = connect_deadline(&self.url)
            .await
            .map_err(StoreError::Connect)?;
        *self.client.lock().await = client;
        Ok(())
    }
}

async fn connect_deadline(url: &str) -> Result<tokio_postgres::Client, SchemaError> {
    match tokio::time::timeout(QUERY_DEADLINE, crate::schema::connect(url)).await {
        Ok(result) => result,
        Err(_) => Err(SchemaError::TimedOut {
            timeout: QUERY_DEADLINE,
        }),
    }
}

async fn timed<T>(
    pg: &Pg,
    fut: impl Future<Output = Result<T, StoreError>>,
) -> Result<T, StoreError> {
    match tokio::time::timeout(QUERY_DEADLINE, fut).await {
        Ok(result) => result,
        Err(_) => {
            tracing::error!("auth database operation timed out");
            let _ = pg.reconnect().await;
            Err(StoreError::TimedOut)
        }
    }
}

async fn postgres_signup(pg: &Pg, cmd: SignupCommand) -> Result<SignupResult, StoreError> {
    let preexisting = {
        let client = pg.lock().await?;
        find_email(&*client, &cmd.email, &cmd.aud)
            .await?
            .map(|user| user.email_confirmed_at.is_some())
    };
    let hash = if preexisting.is_some() {
        None
    } else {
        Some(hash_password(cmd.password.clone()).await?)
    };
    let mut guard = pg.lock().await?;
    let tx = guard.transaction().await?;
    if let Some(user) = find_email(&tx, &cmd.email, &cmd.aud).await? {
        if user.email_confirmed_at.is_some() {
            insert_audit(
                &tx,
                &user,
                "user_repeated_signup",
                "user",
                Some(provider_traits()),
            )
            .await?;
            tx.commit().await?;
            return Ok(SignupResult::AlreadyExists);
        }
        let issued = confirm_existing_tx(&tx, user, &cmd).await?;
        tx.commit().await?;
        return Ok(SignupResult::Created(Box::new(issued)));
    }
    let Some(hash) = hash else {
        return Err(StoreError::Unavailable);
    };
    match insert_new_tx(&tx, &cmd, hash).await {
        Ok(issued) => {
            tx.commit().await?;
            Ok(SignupResult::Created(Box::new(issued)))
        }
        Err(WriteError::Conflict) => {
            drop(tx);
            drop(guard);
            audit_conflict(pg, &cmd).await
        }
        Err(WriteError::Db(error)) => Err(error),
    }
}

async fn audit_conflict(pg: &Pg, cmd: &SignupCommand) -> Result<SignupResult, StoreError> {
    let mut client = pg.lock().await?;
    let tx = client.transaction().await?;
    if let Some(user) = find_email(&tx, &cmd.email, &cmd.aud).await? {
        insert_audit(
            &tx,
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
    tx: &tokio_postgres::Transaction<'_>,
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
    tx.execute(
        "UPDATE auth.users SET
            email_confirmed_at = $2::timestamptz,
            confirmation_token = '',
            last_sign_in_at = $2::timestamptz,
            raw_user_meta_data = $3::jsonb,
            updated_at = $2::timestamptz
         WHERE id = $1::uuid",
        &[&user.id, &now, &user.user_metadata],
    )
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
    tx: &tokio_postgres::Transaction<'_>,
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
    tx: &tokio_postgres::Transaction<'_>,
    user: &UserRecord,
) -> Result<(), WriteError> {
    let confirmed = user.email_confirmed_at.unwrap_or(user.created_at);
    let signed_in = user.last_sign_in_at.unwrap_or(user.created_at);
    let instance = Uuid::nil();
    let result = tx
        .execute(
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
            &[
                &instance,
                &user.id,
                &user.aud,
                &user.role,
                &user.email,
                &user.password_hash,
                &confirmed,
                &signed_in,
                &user.app_metadata,
                &user.user_metadata,
                &user.created_at,
            ],
        )
        .await;
    map_write(result)
}

async fn insert_identity(
    tx: &tokio_postgres::Transaction<'_>,
    identity: &IdentityRecord,
) -> Result<(), WriteError> {
    let result = tx
        .execute(
            "INSERT INTO auth.identities (
                id, provider_id, user_id, identity_data, provider,
                last_sign_in_at, created_at, updated_at
            ) VALUES (
                $1::uuid, $2, $3::uuid, $4::jsonb, $5,
                $6::timestamptz, $6::timestamptz, $6::timestamptz
            )",
            &[
                &identity.id,
                &identity.provider_id,
                &identity.user_id,
                &identity.identity_data,
                &identity.provider,
                &identity.created_at,
            ],
        )
        .await;
    map_write(result)
}

async fn insert_session_rows(
    tx: &tokio_postgres::Transaction<'_>,
    issued: &IssuedSession,
) -> Result<(), WriteError> {
    let instance = Uuid::nil();
    // `auth.refresh_tokens.user_id` is varchar, not uuid.
    let user_id = issued.user.id.to_string();
    let amr_id = Uuid::new_v4();
    map_write(
        tx.execute(
            "INSERT INTO auth.sessions (id, user_id, created_at, updated_at, aal)
             VALUES ($1::uuid, $2::uuid, $3::timestamptz, $3::timestamptz, 'aal1')",
            &[&issued.session_id, &issued.user.id, &issued.amr_at],
        )
        .await,
    )?;
    map_write(
        tx.execute(
            "INSERT INTO auth.refresh_tokens (
                instance_id, token, user_id, revoked, created_at, updated_at, session_id
             ) VALUES (
                $1::uuid, $2, $3, false, $4::timestamptz, $4::timestamptz, $5::uuid
             )",
            &[
                &instance,
                &issued.refresh_token,
                &user_id,
                &issued.amr_at,
                &issued.session_id,
            ],
        )
        .await,
    )?;
    map_write(
        tx.execute(
            "INSERT INTO auth.mfa_amr_claims (
                id, session_id, created_at, updated_at, authentication_method
             ) VALUES (
                $1::uuid, $2::uuid, $3::timestamptz, $3::timestamptz, 'password'
             )",
            &[&amr_id, &issued.session_id, &issued.amr_at],
        )
        .await,
    )?;
    Ok(())
}

async fn insert_audit(
    tx: &tokio_postgres::Transaction<'_>,
    user: &UserRecord,
    action: &str,
    log_type: &str,
    traits: Option<Value>,
) -> Result<(), StoreError> {
    let instance = Uuid::nil();
    let id = Uuid::new_v4();
    let now = SystemTime::now();
    let payload = audit_payload(user, action, log_type, traits);
    tx.execute(
        "INSERT INTO auth.audit_log_entries (instance_id, id, payload, created_at, ip_address)
         VALUES ($1::uuid, $2::uuid, $3::json, $4::timestamptz, '')",
        &[&instance, &id, &payload, &now],
    )
    .await?;
    Ok(())
}

async fn exec_logout(
    tx: &tokio_postgres::Transaction<'_>,
    user_id: Uuid,
    session_id: Option<Uuid>,
    scope: LogoutScope,
) -> Result<(), StoreError> {
    match (session_id, scope) {
        (None, _) | (_, LogoutScope::Global) => {
            tx.execute(
                "DELETE FROM auth.sessions WHERE user_id = $1::uuid",
                &[&user_id],
            )
            .await?;
        }
        (Some(session_id), LogoutScope::Local) => {
            tx.execute(
                "DELETE FROM auth.sessions WHERE id = $1::uuid",
                &[&session_id],
            )
            .await?;
        }
        (Some(session_id), LogoutScope::Others) => {
            tx.execute(
                "DELETE FROM auth.sessions WHERE user_id = $1::uuid AND id <> $2::uuid",
                &[&user_id, &session_id],
            )
            .await?;
        }
    }
    Ok(())
}

async fn find_user_by_email_and_audience<C: GenericClient + Sync>(
    client: &C,
    email: &str,
    aud: &str,
) -> Result<Option<UserRecord>, StoreError> {
    let instance = Uuid::nil();
    let row = client
        .query_opt(
            "SELECT id::text FROM auth.users
             WHERE instance_id = $1::uuid
               AND lower(email) = $2
               AND aud = $3
               AND is_sso_user = false
             LIMIT 1",
            &[&instance, &email, &aud],
        )
        .await?;
    match row {
        Some(row) => find_user_by_id(client, parse_uuid(&row.get::<_, String>(0))?).await,
        None => Ok(None),
    }
}

async fn find_phone<C: GenericClient + Sync>(
    client: &C,
    phone: &str,
    aud: &str,
) -> Result<Option<UserRecord>, StoreError> {
    let instance = Uuid::nil();
    let row = client
        .query_opt(
            "SELECT id::text FROM auth.users
             WHERE instance_id = $1::uuid
               AND phone = $2
               AND aud = $3
               AND is_sso_user = false
             LIMIT 1",
            &[&instance, &phone, &aud],
        )
        .await?;
    match row {
        Some(row) => find_user_by_id(client, parse_uuid(&row.get::<_, String>(0))?).await,
        None => Ok(None),
    }
}

async fn find_email<C: GenericClient + Sync>(
    client: &C,
    email: &str,
    aud: &str,
) -> Result<Option<UserRecord>, StoreError> {
    let instance = Uuid::nil();
    let by_identity = client
        .query_opt(
            "SELECT u.id::text
             FROM auth.identities i
             JOIN auth.users u ON u.id = i.user_id
             WHERE i.email = $1
               AND u.aud = $2
               AND u.instance_id = $3::uuid
               AND u.is_sso_user = false
             LIMIT 1",
            &[&email, &aud, &instance],
        )
        .await?;
    if let Some(row) = by_identity {
        let id = parse_uuid(&row.get::<_, String>(0))?;
        return find_user_by_id(client, id).await;
    }
    let by_user = client
        .query_opt(
            "SELECT id::text FROM auth.users
             WHERE instance_id = $1::uuid
               AND lower(email) = $2
               AND aud = $3
               AND is_sso_user = false
             LIMIT 1",
            &[&instance, &email, &aud],
        )
        .await?;
    match by_user {
        Some(row) => find_user_by_id(client, parse_uuid(&row.get::<_, String>(0))?).await,
        None => Ok(None),
    }
}

async fn find_user_by_id<C: GenericClient + Sync>(
    client: &C,
    id: Uuid,
) -> Result<Option<UserRecord>, StoreError> {
    let row = client
        .query_opt(
            "SELECT id::text, aud, COALESCE(role, ''), COALESCE(email, ''),
                    COALESCE(phone, ''), email_confirmed_at, last_sign_in_at,
                    created_at, updated_at,
                    COALESCE(raw_app_meta_data::text, 'null'),
                    COALESCE(raw_user_meta_data::text, 'null'),
                    is_anonymous, banned_until, is_sso_user,
                    COALESCE(encrypted_password, ''), phone_confirmed_at,
                    confirmed_at
             FROM auth.users
             WHERE instance_id = $1::uuid AND id = $2::uuid",
            &[&Uuid::nil(), &id],
        )
        .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let id = parse_uuid(&row.get::<_, String>(0))?;
    let identities = load_identities(client, id).await?;
    let created_at: Option<SystemTime> = row.get(7);
    let updated_at: Option<SystemTime> = row.get(8);
    let now = SystemTime::now();
    Ok(Some(UserRecord {
        id,
        aud: row.get(1),
        role: row.get(2),
        email: row.get(3),
        phone: row.get(4),
        email_confirmed_at: row.get(5),
        last_sign_in_at: row.get(6),
        app_metadata: parse_json_text(&row.get::<_, String>(9)),
        user_metadata: parse_json_text(&row.get::<_, String>(10)),
        identities,
        created_at: created_at.unwrap_or(now),
        updated_at: updated_at.unwrap_or(now),
        is_anonymous: row.get(11),
        banned_until: row.get(12),
        is_sso_user: row.get(13),
        password_hash: row.get(14),
        phone_confirmed_at: row.get(15),
        confirmed_at: row.get(16),
    }))
}

async fn load_identities<C: GenericClient + Sync>(
    client: &C,
    user_id: Uuid,
) -> Result<Vec<IdentityRecord>, StoreError> {
    let rows = client
        .query(
            "SELECT id::text, provider_id, user_id::text, identity_data::text, provider,
                    COALESCE(email, ''), last_sign_in_at, created_at, updated_at
             FROM auth.identities WHERE user_id = $1::uuid",
            &[&user_id],
        )
        .await?;
    let mut identities = Vec::with_capacity(rows.len());
    for row in rows {
        let created_at: Option<SystemTime> = row.get(7);
        let updated_at: Option<SystemTime> = row.get(8);
        let now = SystemTime::now();
        identities.push(IdentityRecord {
            id: parse_uuid(&row.get::<_, String>(0))?,
            provider_id: row.get(1),
            user_id: parse_uuid(&row.get::<_, String>(2))?,
            identity_data: parse_json_text(&row.get::<_, String>(3)),
            provider: row.get(4),
            email: row.get(5),
            last_sign_in_at: row.get(6),
            created_at: created_at.unwrap_or(now),
            updated_at: updated_at.unwrap_or(now),
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

fn map_write(result: Result<u64, tokio_postgres::Error>) -> Result<(), WriteError> {
    match result {
        Ok(_) => Ok(()),
        Err(error) if unique_violation(&error) => Err(WriteError::Conflict),
        Err(error) => Err(WriteError::Db(error.into())),
    }
}

fn unique_violation(error: &tokio_postgres::Error) -> bool {
    error.code().is_some_and(|code| code.code() == "23505")
}

fn parse_uuid(value: &str) -> Result<Uuid, StoreError> {
    Uuid::parse_str(value).map_err(|_| StoreError::Unavailable)
}

fn parse_json_text(value: &str) -> Value {
    serde_json::from_str(value).unwrap_or(Value::Null)
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
    payload.insert("actor_username".into(), json!(user.email));
    payload.insert("actor_via_sso".into(), json!(user.is_sso_user));
    payload.insert("log_type".into(), json!(log_type));
    if let Some(traits) = traits {
        payload.insert("traits".into(), traits);
    }
    Value::Object(payload)
}

fn remember_audit(user: &UserRecord, action: &str, log_type: &str, traits: Option<Value>) {
    let traits = traits.unwrap_or(Value::Null);
    tracing::debug!(
        user_id = %user.id,
        action,
        log_type,
        %traits,
        "auth audit"
    );
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
