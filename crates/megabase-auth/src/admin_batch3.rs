// Ported from supabase/auth (MIT), pin v2.197.0:
//   internal/api/admin.go (adminUserCreate, adminUserUpdate, adminUserUpdateFactor)
//   internal/api/ssoadmin.go (adminSSOProvidersCreate, adminSSOProvidersUpdate)
//   internal/api/custom_oauth_admin.go (adminCustomOAuthProviderUpdate)
//   internal/api/oauthserver/handlers.go (OAuthServerClientUpdate, OAuthServerClientRegenerateSecret)
//   internal/api/oauthserver/service.go
//   internal/api/oauthserver/client_auth.go
//   internal/api/phone.go (validatePhone)
//   internal/api/password.go (checkPasswordStrength)
//   internal/models/user.go, internal/models/sso.go

//! Issue #8 admin writes: user create/update, factor update, SSO create/update,
//! OAuth client update and secret regeneration, custom-provider update.

use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::Response,
};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::{DateTime, Utc};
use megabase_core::MegabaseNotImplemented;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::{
    empty_as_none, generate_password, go_json, hash_password, limit_body, load_factors,
    load_one_sso, load_user_json, parse_user_id, random_bytes, sso_json, validate_admin_email,
    validate_attribute_mapping, validate_auth_params, validate_claims, validate_redirect_uri,
    write_audit, GenerateLinkError,
};
use crate::admin::{
    is_uuid, nil_instance, oauth_client_json, pool, require_admin, require_custom_oauth,
    require_oauth_server, verified_admin_role, CustomProviderRow, OAuthClientRow,
};
use crate::error::go_quote;
use crate::http::{json_ok, json_status, AuthError};
use crate::routes::request_aud;
use crate::state::AuthState;

type Reply = Result<Response, GenerateLinkError>;

fn not_impl(unit: &'static str) -> GenerateLinkError {
    GenerateLinkError::NotImplemented(MegabaseNotImplemented::new(crate::COMPONENT, unit))
}

fn valid_request_uri(raw: &str) -> bool {
    super::url_parts(raw).is_ok()
}

fn bad_json(err: &serde_json::Error, body: &[u8]) -> AuthError {
    AuthError::new(
        400,
        "bad_json",
        format!(
            "Could not parse request body as JSON: {}",
            go_json(err, body)
        ),
    )
}

/// `retrieveRequestParams`: size limit, then a lenient JSON decode.
fn params<T: DeserializeOwned>(body: &Bytes) -> Result<T, AuthError> {
    limit_body(body)?;
    serde_json::from_slice(body).map_err(|err| bad_json(&err, body))
}

fn db_err<E>(message: &'static str) -> impl Fn(E) -> AuthError + Copy {
    move |_| AuthError::internal(message)
}

// ---------------------------------------------------------------------------
// Shared validation helpers
// ---------------------------------------------------------------------------

/// `validatePhone`: drop one leading `+` and all spaces, then E.164 digits.
fn validate_phone(phone: &str) -> Result<String, AuthError> {
    let phone = phone.strip_prefix('+').unwrap_or(phone).replace(' ', "");
    let bytes = phone.as_bytes();
    let ok =
        (2..=15).contains(&bytes.len()) && bytes[0] != b'0' && bytes.iter().all(u8::is_ascii_digit);
    if ok {
        Ok(phone)
    } else {
        Err(AuthError::validation(
            400,
            "Invalid phone number format (E.164 required)",
        ))
    }
}

/// `checkPasswordStrength` with the HIBP check off and no required characters.
fn check_password_strength(state: &AuthState, password: &str) -> Result<(), AuthError> {
    if password.len() > 72 {
        return Err(AuthError::validation(
            400,
            "Password cannot be longer than 72 characters",
        ));
    }
    if password.len() < state.config.password_min_length {
        return Err(AuthError::weak_password(
            format!(
                "Password should be at least {} characters.",
                state.config.password_min_length
            ),
            vec!["length".into()],
        ));
    }
    Ok(())
}

/// Go `time.ParseDuration`, in nanoseconds.
fn parse_go_duration(input: &str) -> Result<i128, String> {
    let invalid = || format!("time: invalid duration {}", go_quote(input));
    let mut rest = input;
    let mut negative = false;
    if let Some(stripped) = rest.strip_prefix(['-', '+']) {
        negative = rest.starts_with('-');
        rest = stripped;
    }
    if rest == "0" {
        return Ok(0);
    }
    if rest.is_empty() {
        return Err(invalid());
    }
    let mut total: f64 = 0.0;
    while !rest.is_empty() {
        let int_len = rest.bytes().take_while(u8::is_ascii_digit).count();
        let (int_part, after_int) = rest.split_at(int_len);
        rest = after_int;
        let mut frac_part = "";
        if let Some(after_dot) = rest.strip_prefix('.') {
            let frac_len = after_dot.bytes().take_while(u8::is_ascii_digit).count();
            frac_part = &after_dot[..frac_len];
            rest = &after_dot[frac_len..];
            if int_part.is_empty() && frac_part.is_empty() {
                return Err(invalid());
            }
        } else if int_part.is_empty() {
            return Err(invalid());
        }
        let unit_len = rest
            .bytes()
            .take_while(|byte| *byte != b'.' && !byte.is_ascii_digit())
            .count();
        if unit_len == 0 {
            return Err(format!(
                "time: missing unit in duration {}",
                go_quote(input)
            ));
        }
        let (unit, after_unit) = rest.split_at(unit_len);
        rest = after_unit;
        let scale: f64 = match unit {
            "ns" => 1.0,
            "us" | "\u{b5}s" | "\u{3bc}s" => 1e3,
            "ms" => 1e6,
            "s" => 1e9,
            "m" => 60e9,
            "h" => 3600e9,
            _ => {
                return Err(format!(
                    "time: unknown unit {} in duration {}",
                    go_quote(unit),
                    go_quote(input)
                ))
            }
        };
        let number: f64 = format!("{}.{}", if int_part.is_empty() { "0" } else { int_part }, {
            if frac_part.is_empty() {
                "0"
            } else {
                frac_part
            }
        })
        .parse()
        .map_err(|_| invalid())?;
        total += number * scale;
        if total > 9.223_372_036_854_775e18 {
            return Err(invalid());
        }
    }
    let nanos = total as i128;
    Ok(if negative { -nanos } else { nanos })
}

/// `ban_duration`: `none` clears the ban, anything else is a Go duration.
fn parse_ban(raw: Option<&str>) -> Result<Option<i128>, AuthError> {
    match raw {
        None | Some("") => Ok(None),
        Some("none") => Ok(Some(0)),
        Some(value) => parse_go_duration(value).map(Some).map_err(|err| {
            AuthError::validation(400, format!("invalid format for ban duration: {err}"))
        }),
    }
}

async fn apply_ban(
    tx: &mut sqlx::PgConnection,
    user_id: Uuid,
    nanos: i128,
) -> Result<(), sqlx::Error> {
    let until: Option<DateTime<Utc>> = if nanos == 0 {
        None
    } else {
        let micros = (nanos / 1000).clamp(i128::from(i64::MIN / 2), i128::from(i64::MAX / 2));
        let delta = chrono::Duration::microseconds(micros as i64);
        Utc::now().checked_add_signed(delta)
    };
    sqlx::query("UPDATE auth.users SET banned_until = $2, updated_at = NOW() WHERE id = $1")
        .bind(user_id)
        .bind(until)
        .execute(&mut *tx)
        .await
        .map(|_| ())
}

/// `UpdateUserMetaData` / `UpdateAppMetaData`: merge, with `null` deleting a key.
fn merge_meta(current: Option<Value>, updates: &Map<String, Value>) -> Value {
    let mut map = match current {
        Some(Value::Object(map)) => map,
        _ => Map::new(),
    };
    for (key, value) in updates {
        if value.is_null() {
            map.remove(key);
        } else {
            map.insert(key.clone(), value.clone());
        }
    }
    Value::Object(map)
}

async fn clear_one_time_tokens(
    tx: &mut sqlx::PgConnection,
    user_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM auth.one_time_tokens WHERE user_id = $1")
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .map(|_| ())
}

async fn confirm_email(tx: &mut sqlx::PgConnection, user_id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE auth.users SET confirmation_token = '', email_confirmed_at = NOW(),
                raw_user_meta_data = COALESCE(raw_user_meta_data, '{}'::jsonb) || '{\"email_verified\": true}'::jsonb,
                updated_at = NOW()
         WHERE id = $1",
    )
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    clear_one_time_tokens(tx, user_id).await
}

async fn confirm_phone(tx: &mut sqlx::PgConnection, user_id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE auth.users SET confirmation_token = '', phone_confirmed_at = NOW(), updated_at = NOW()
         WHERE id = $1",
    )
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    clear_one_time_tokens(tx, user_id).await
}

fn identity_claims(
    user_id: Uuid,
    email: Option<&str>,
    phone: Option<&str>,
    email_verified: bool,
    phone_verified: bool,
) -> Value {
    let mut claims = Map::new();
    claims.insert("sub".into(), json!(user_id.to_string()));
    if let Some(email) = email {
        claims.insert("email".into(), json!(email));
    }
    claims.insert("email_verified".into(), json!(email_verified));
    if let Some(phone) = phone {
        claims.insert("phone".into(), json!(phone));
    }
    claims.insert("phone_verified".into(), json!(phone_verified));
    Value::Object(claims)
}

async fn insert_identity(
    tx: &mut sqlx::PgConnection,
    user_id: Uuid,
    provider: &str,
    claims: &Value,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO auth.identities (id, provider_id, user_id, identity_data, provider, last_sign_in_at, created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, NOW(), NOW(), NOW())",
    )
    .bind(Uuid::new_v4())
    .bind(user_id.to_string())
    .bind(user_id)
    .bind(claims)
    .bind(provider)
    .execute(&mut *tx)
    .await
    .map(|_| ())
}

// ---------------------------------------------------------------------------
// POST /admin/users
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Deserialize)]
struct AdminUserParams {
    id: Option<String>,
    aud: Option<String>,
    role: Option<String>,
    email: Option<String>,
    phone: Option<String>,
    password: Option<String>,
    password_hash: Option<String>,
    email_confirm: Option<bool>,
    phone_confirm: Option<bool>,
    user_metadata: Option<Map<String, Value>>,
    app_metadata: Option<Map<String, Value>>,
    ban_duration: Option<String>,
}

/// `bcrypt.Cost` acceptance: version, two-digit cost, 53 hash characters.
fn bcrypt_hash_ok(hash: &str) -> bool {
    let bytes = hash.as_bytes();
    if !hash.is_ascii() || bytes.len() != 60 || bytes[0] != b'$' || bytes[1] != b'2' {
        return false;
    }
    let rest = &hash[2..];
    let rest = rest.strip_prefix(['a', 'b', 'x', 'y']).unwrap_or(rest);
    let Some(rest) = rest.strip_prefix('$') else {
        return false;
    };
    let (cost, tail) = rest.split_at(2.min(rest.len()));
    cost.bytes().all(|b| b.is_ascii_digit())
        && cost
            .parse::<u32>()
            .is_ok_and(|cost| (4..=31).contains(&cost))
        && tail.starts_with('$')
}

// megabase:unit auth:route:POST /auth/v1/admin/users
pub(crate) async fn create_user(
    State(state): State<AuthState>,
    headers: HeaderMap,
    body: Bytes,
) -> Reply {
    let role = verified_admin_role(&state, &headers)?;
    let mut params: AdminUserParams = params(&body)?;
    let mut aud = request_aud(&headers, &state.config);
    if let Some(custom) = params.aud.as_deref().filter(|aud| !aud.is_empty()) {
        aud = custom.to_string();
    }
    let email_in = params.email.take().unwrap_or_default();
    let phone_in = params.phone.take().unwrap_or_default();
    if email_in.is_empty() && phone_in.is_empty() {
        return Err(AuthError::validation(
            400,
            "Cannot create a user without either an email or phone",
        )
        .into());
    }
    let db = pool(&state)?;
    let mut providers: Vec<&str> = Vec::new();
    let mut email = String::new();
    let mut phone = String::new();
    if !email_in.is_empty() {
        email = validate_admin_email(&email_in)?;
        if email_exists(&db, &email, &aud, None).await? {
            return Err(AuthError::unprocessable(
                "email_exists",
                "A user with this email address has already been registered",
            )
            .into());
        }
        providers.push("email");
    }
    if !phone_in.is_empty() {
        phone = validate_phone(&phone_in)?;
        if phone_exists(&db, &phone, &aud, None).await? {
            return Err(AuthError::unprocessable(
                "phone_exists",
                "Phone number already registered by another user",
            )
            .into());
        }
        providers.push("phone");
    }
    let hash_given = params
        .password_hash
        .as_deref()
        .is_some_and(|h| !h.is_empty());
    if params.password.is_some() && hash_given {
        return Err(AuthError::validation(
            400,
            "Only a password or a password hash should be provided",
        )
        .into());
    }
    let password = params.password.clone().unwrap_or_default();
    if !password.is_empty() {
        check_password_strength(&state, &password)?;
    }
    let encrypted = if hash_given {
        let hash = params.password_hash.clone().unwrap_or_default();
        if hash.starts_with("$argon2") {
            return Err(not_impl("admin:password_hash:argon2"));
        }
        if hash.starts_with("$fbscrypt$") {
            return Err(not_impl("admin:password_hash:scrypt"));
        }
        if !bcrypt_hash_ok(&hash) {
            return Err(AuthError::internal("Error creating user").into());
        }
        hash
    } else if password.is_empty() {
        hash_password(&generate_password()?).await?
    } else {
        hash_password(&password).await?
    };
    let user_id = match params.id.as_deref().filter(|id| !id.is_empty()) {
        Some(raw) => {
            let id = Uuid::parse_str(raw)
                .map_err(|_| AuthError::validation(400, "ID must conform to the uuid v4 format"))?;
            if id.is_nil() {
                return Err(AuthError::validation(400, "ID cannot be a nil uuid").into());
            }
            id
        }
        None => Uuid::new_v4(),
    };
    let ban = parse_ban(params.ban_duration.as_deref())?;
    let user_role = params
        .role
        .as_deref()
        .filter(|role| !role.is_empty())
        .map(str::trim)
        .unwrap_or(state.config.jwt_default_group.as_str())
        .to_string();
    let meta = Value::Object(params.user_metadata.clone().unwrap_or_default());
    let app = merge_meta(
        Some(json!({ "provider": providers[0], "providers": providers })),
        &params.app_metadata.clone().unwrap_or_default(),
    );
    let fail = db_err("Database error creating new user");
    let mut tx = db.begin().await.map_err(fail)?;
    let fail = db_err("Database error creating new user");
    sqlx::query(
        "INSERT INTO auth.users (
            instance_id, id, aud, role, email, phone, encrypted_password,
            confirmation_token, recovery_token, email_change_token_new, email_change,
            email_change_token_current, email_change_confirm_status,
            phone_change, phone_change_token, reauthentication_token,
            raw_app_meta_data, raw_user_meta_data,
            is_sso_user, is_anonymous, created_at, updated_at
        ) VALUES (
            $1, $2, $3, $4, $5, $6, $7,
            '', '', '', '',
            '', 0,
            '', '', '',
            $8, $9,
            false, false, NOW(), NOW()
        )",
    )
    .bind(nil_instance())
    .bind(user_id)
    .bind(&aud)
    .bind(&user_role)
    .bind(empty_as_none(&email))
    .bind(empty_as_none(&phone))
    .bind(&encrypted)
    .bind(&app)
    .bind(&meta)
    .execute(&mut *tx)
    .await
    .map_err(|_| AuthError::internal("Database error creating new user"))?;
    if !email.is_empty() {
        insert_identity(
            &mut tx,
            user_id,
            "email",
            &identity_claims(user_id, Some(&email), None, false, false),
        )
        .await
        .map_err(|_| AuthError::internal("Database error creating new user"))?;
    }
    if !phone.is_empty() {
        insert_identity(
            &mut tx,
            user_id,
            "phone",
            &identity_claims(user_id, None, Some(&phone), false, false),
        )
        .await
        .map_err(|_| AuthError::internal("Database error creating new user"))?;
    }
    write_audit(
        &mut tx,
        "00000000-0000-0000-0000-000000000000",
        false,
        &role,
        "user_signedup",
        "team",
        Some(json!({
            "user_id": user_id.to_string(),
            "user_email": email,
            "user_phone": phone,
            "provider": providers[0],
        })),
    )
    .await
    .map_err(|_| AuthError::internal("Database error creating new user"))?;
    if params.email_confirm.unwrap_or(false) {
        confirm_email(&mut tx, user_id).await.map_err(fail)?;
    }
    if params.phone_confirm.unwrap_or(false) {
        confirm_phone(&mut tx, user_id).await.map_err(fail)?;
    }
    if let Some(nanos) = ban {
        apply_ban(&mut tx, user_id, nanos).await.map_err(fail)?;
    }
    let user = load_user_json(&mut tx, user_id).await?;
    tx.commit().await.map_err(fail)?;
    Ok(json_ok(&user))
}

/// Another user in `aud` holds `phone`; `exclude` is the user being updated.
async fn phone_exists(
    db: &sqlx::PgPool,
    phone: &str,
    aud: &str,
    exclude: Option<Uuid>,
) -> Result<bool, AuthError> {
    let found = sqlx::query_as::<_, (Uuid,)>(
        "SELECT id FROM auth.users
         WHERE instance_id = $1 AND phone = $2 AND aud = $3 AND is_sso_user = false
           AND ($4::uuid IS NULL OR id <> $4) LIMIT 1",
    )
    .bind(nil_instance())
    .bind(phone)
    .bind(aud)
    .bind(exclude)
    .fetch_optional(db)
    .await
    .map_err(db_err("Database error checking phone"))?;
    Ok(found.is_some())
}

/// Another user in `aud` holds `email`; `exclude` is the user being updated.
async fn email_exists(
    db: &sqlx::PgPool,
    email: &str,
    aud: &str,
    exclude: Option<Uuid>,
) -> Result<bool, AuthError> {
    let fail = db_err("Database error checking email");
    let via_identity = sqlx::query_as::<_, (Uuid,)>(
        "SELECT u.id FROM auth.identities i JOIN auth.users u ON u.id = i.user_id
         WHERE i.email = $1 AND u.aud = $2 AND ($3::uuid IS NULL OR u.id <> $3) LIMIT 1",
    )
    .bind(email)
    .bind(aud)
    .bind(exclude)
    .fetch_optional(db)
    .await
    .map_err(fail)?;
    if via_identity.is_some() {
        return Ok(true);
    }
    let fail = db_err("Database error checking email");
    let found = sqlx::query_as::<_, (Uuid,)>(
        "SELECT id FROM auth.users
         WHERE instance_id = $1 AND LOWER(email) = $2 AND aud = $3 AND is_sso_user = false
           AND ($4::uuid IS NULL OR id <> $4) LIMIT 1",
    )
    .bind(nil_instance())
    .bind(email)
    .bind(aud)
    .bind(exclude)
    .fetch_optional(db)
    .await
    .map_err(fail)?;
    Ok(found.is_some())
}

// ---------------------------------------------------------------------------
// PUT /admin/users/{user_id}
// ---------------------------------------------------------------------------

#[derive(Debug, sqlx::FromRow)]
struct UserState {
    aud: String,
    is_anonymous: bool,
    raw_app_meta_data: Option<Value>,
    raw_user_meta_data: Option<Value>,
}

// megabase:unit auth:route:PUT /auth/v1/admin/users/{user_id}
pub(crate) async fn update_user(
    State(state): State<AuthState>,
    headers: HeaderMap,
    Path(user_id): Path<String>,
    body: Bytes,
) -> Reply {
    let role = verified_admin_role(&state, &headers)?;
    let id = parse_user_id(&user_id)?;
    let db = pool(&state)?;
    let current = sqlx::query_as::<_, UserState>(
        "SELECT aud, is_anonymous, raw_app_meta_data, raw_user_meta_data
         FROM auth.users WHERE instance_id = $1 AND id = $2",
    )
    .bind(nil_instance())
    .bind(id)
    .fetch_optional(&db)
    .await
    .map_err(db_err("Database error loading user"))?
    .ok_or_else(|| AuthError::not_found("user_not_found", "User not found"))?;
    let params: AdminUserParams = params(&body)?;
    let email = match params.email.as_deref().filter(|e| !e.is_empty()) {
        Some(raw) => validate_admin_email(raw)?,
        None => String::new(),
    };
    if !email.is_empty() && email_exists(&db, &email, &current.aud, Some(id)).await? {
        return Err(AuthError::unprocessable(
            "email_exists",
            "A user with this email address has already been registered",
        )
        .into());
    }
    let phone = match params.phone.as_deref().filter(|p| !p.is_empty()) {
        Some(raw) => validate_phone(raw)?,
        None => String::new(),
    };
    if !phone.is_empty() && phone_exists(&db, &phone, &current.aud, Some(id)).await? {
        return Err(AuthError::unprocessable(
            "phone_exists",
            "Phone number already registered by another user",
        )
        .into());
    }
    let ban = parse_ban(params.ban_duration.as_deref())?;
    let mut password_hash: Option<Option<String>> = None;
    if let Some(password) = params.password.as_deref() {
        check_password_strength(&state, password)?;
        password_hash = Some(if password.is_empty() {
            None
        } else {
            Some(hash_password(password).await?)
        });
    }
    let email_confirm = params.email_confirm.unwrap_or(false);
    let phone_confirm = params.phone_confirm.unwrap_or(false);
    let fail = || AuthError::internal("Error updating user");
    let mut tx = db.begin().await.map_err(|_| fail())?;
    let result: Result<(), sqlx::Error> = async {
        if let Some(role) = params.role.as_deref().filter(|r| !r.is_empty()) {
            sqlx::query("UPDATE auth.users SET role = $2, updated_at = NOW() WHERE id = $1")
                .bind(id)
                .bind(role.trim())
                .execute(&mut *tx)
                .await?;
        }
        if email_confirm {
            confirm_email(&mut tx, id).await?;
        }
        if phone_confirm {
            confirm_phone(&mut tx, id).await?;
        }
        if let Some(hash) = &password_hash {
            // `UpdatePassword`: reset tokens, drop one-time tokens and sessions.
            sqlx::query(
                "UPDATE auth.users SET encrypted_password = $2,
                    confirmation_token = '', confirmation_sent_at = NULL,
                    recovery_token = '', recovery_sent_at = NULL,
                    email_change_token_current = '', email_change_token_new = '',
                    email_change_sent_at = NULL, phone_change_token = '',
                    phone_change_sent_at = NULL, reauthentication_token = '',
                    reauthentication_sent_at = NULL, updated_at = NOW()
                 WHERE id = $1",
            )
            .bind(id)
            .bind(hash.as_deref())
            .execute(&mut *tx)
            .await?;
            clear_one_time_tokens(&mut tx, id).await?;
            sqlx::query("DELETE FROM auth.sessions WHERE user_id = $1")
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }
        if !email.is_empty() {
            upsert_contact_identity(&mut tx, id, "email", &email, email_confirm).await?;
            if current.is_anonymous && email_confirm {
                sqlx::query("UPDATE auth.users SET is_anonymous = false WHERE id = $1")
                    .bind(id)
                    .execute(&mut *tx)
                    .await?;
            }
            sqlx::query("UPDATE auth.users SET email = $2, updated_at = NOW() WHERE id = $1")
                .bind(id)
                .bind(&email)
                .execute(&mut *tx)
                .await?;
        }
        if !phone.is_empty() {
            upsert_contact_identity(&mut tx, id, "phone", &phone, phone_confirm).await?;
            if current.is_anonymous && phone_confirm {
                sqlx::query("UPDATE auth.users SET is_anonymous = false WHERE id = $1")
                    .bind(id)
                    .execute(&mut *tx)
                    .await?;
            }
            sqlx::query("UPDATE auth.users SET phone = $2, updated_at = NOW() WHERE id = $1")
                .bind(id)
                .bind(&phone)
                .execute(&mut *tx)
                .await?;
        }
        if !email.is_empty() || !phone.is_empty() {
            // `ClearAllPendingTokens`
            sqlx::query(
                "UPDATE auth.users SET confirmation_token = '', confirmation_sent_at = NULL,
                    recovery_token = '', recovery_sent_at = NULL, email_change = '',
                    email_change_token_current = '', email_change_token_new = '',
                    email_change_sent_at = NULL, email_change_confirm_status = 0,
                    phone_change = '', phone_change_token = '', phone_change_sent_at = NULL,
                    reauthentication_token = '', reauthentication_sent_at = NULL,
                    updated_at = NOW()
                 WHERE id = $1",
            )
            .bind(id)
            .execute(&mut *tx)
            .await?;
            clear_one_time_tokens(&mut tx, id).await?;
        }
        if let Some(updates) = &params.app_metadata {
            let (stored,): (Option<Value>,) =
                sqlx::query_as("SELECT raw_app_meta_data FROM auth.users WHERE id = $1")
                    .bind(id)
                    .fetch_one(&mut *tx)
                    .await?;
            let merged = merge_meta(stored.or(current.raw_app_meta_data.clone()), updates);
            sqlx::query(
                "UPDATE auth.users SET raw_app_meta_data = $2, updated_at = NOW() WHERE id = $1",
            )
            .bind(id)
            .bind(&merged)
            .execute(&mut *tx)
            .await?;
        }
        if let Some(updates) = &params.user_metadata {
            let (stored,): (Option<Value>,) =
                sqlx::query_as("SELECT raw_user_meta_data FROM auth.users WHERE id = $1")
                    .bind(id)
                    .fetch_one(&mut *tx)
                    .await?;
            let merged = merge_meta(stored.or(current.raw_user_meta_data.clone()), updates);
            sqlx::query(
                "UPDATE auth.users SET raw_user_meta_data = $2, updated_at = NOW() WHERE id = $1",
            )
            .bind(id)
            .bind(&merged)
            .execute(&mut *tx)
            .await?;
        }
        if let Some(nanos) = ban {
            apply_ban(&mut tx, id, nanos).await?;
        }
        let (user_email, user_phone): (Option<String>, Option<String>) =
            sqlx::query_as("SELECT email, phone FROM auth.users WHERE id = $1")
                .bind(id)
                .fetch_one(&mut *tx)
                .await?;
        write_audit(
            &mut tx,
            "00000000-0000-0000-0000-000000000000",
            false,
            &role,
            "user_modified",
            "user",
            Some(json!({
                "user_id": id.to_string(),
                "user_email": user_email.unwrap_or_default(),
                "user_phone": user_phone.unwrap_or_default(),
            })),
        )
        .await
        .map_err(|_| sqlx::Error::Protocol("audit".into()))?;
        Ok(())
    }
    .await;
    if result.is_err() {
        return Err(fail().into());
    }
    let user = load_user_json(&mut tx, id).await?;
    tx.commit().await.map_err(|_| fail())?;
    Ok(json_ok(&user))
}

/// Update the user's `email` or `phone` identity, creating it when missing.
async fn upsert_contact_identity(
    tx: &mut sqlx::PgConnection,
    user_id: Uuid,
    provider: &str,
    value: &str,
    verified: bool,
) -> Result<(), sqlx::Error> {
    let existing = sqlx::query_as::<_, (Uuid,)>(
        "SELECT id FROM auth.identities WHERE provider_id = $1 AND provider = $2",
    )
    .bind(user_id.to_string())
    .bind(provider)
    .fetch_optional(&mut *tx)
    .await?;
    let (value_key, verified_key) = if provider == "email" {
        ("email", "email_verified")
    } else {
        ("phone", "phone_verified")
    };
    match existing {
        None => {
            let claims = if provider == "email" {
                identity_claims(user_id, Some(value), None, verified, false)
            } else {
                identity_claims(user_id, None, Some(value), false, verified)
            };
            insert_identity(tx, user_id, provider, &claims).await
        }
        Some((identity_id,)) => {
            let mut updates = Map::new();
            updates.insert(value_key.into(), json!(value));
            updates.insert(verified_key.into(), json!(verified));
            sqlx::query(
                "UPDATE auth.identities SET identity_data = COALESCE(identity_data, '{}'::jsonb) || $2
                 WHERE id = $1",
            )
            .bind(identity_id)
            .bind(Value::Object(updates))
            .execute(&mut *tx)
            .await
            .map(|_| ())
        }
    }
}

// ---------------------------------------------------------------------------
// PUT /admin/users/{user_id}/factors/{factor_id}
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Deserialize)]
struct FactorParams {
    friendly_name: Option<String>,
    phone: Option<String>,
}

// megabase:unit auth:route:PUT /auth/v1/admin/users/{user_id}/factors/{factor_id}
pub(crate) async fn update_factor(
    State(state): State<AuthState>,
    headers: HeaderMap,
    Path(path): Path<std::collections::HashMap<String, String>>,
    body: Bytes,
) -> Reply {
    let role = verified_admin_role(&state, &headers)?;
    let user_id = parse_user_id(path.get("user_id").map_or("", String::as_str))?;
    let db = pool(&state)?;
    let user = sqlx::query_as::<_, (Uuid,)>(
        "SELECT id FROM auth.users WHERE instance_id = $1 AND id = $2",
    )
    .bind(nil_instance())
    .bind(user_id)
    .fetch_optional(&db)
    .await
    .map_err(db_err("Database error loading user"))?;
    if user.is_none() {
        return Err(AuthError::not_found("user_not_found", "User not found").into());
    }
    let raw_factor = path.get("factor_id").map_or("", String::as_str);
    if !is_uuid(raw_factor) {
        return Err(AuthError::not_found("validation_failed", "factor_id must be an UUID").into());
    }
    let factor_id = Uuid::parse_str(raw_factor)
        .map_err(|_| AuthError::not_found("validation_failed", "factor_id must be an UUID"))?;
    let factor = sqlx::query_as::<_, (String,)>(
        "SELECT factor_type::text FROM auth.mfa_factors WHERE user_id = $1 AND id = $2",
    )
    .bind(user_id)
    .bind(factor_id)
    .fetch_optional(&db)
    .await
    .map_err(db_err("Database error loading factor"))?
    .ok_or_else(|| AuthError::not_found("mfa_factor_not_found", "Factor not found"))?;
    let params: FactorParams = params(&body)?;
    let mut tx = db
        .begin()
        .await
        .map_err(db_err("Database error updating factor"))?;
    let fail = db_err("Database error updating factor");
    if let Some(name) = params.friendly_name.as_deref().filter(|n| !n.is_empty()) {
        sqlx::query(
            "UPDATE auth.mfa_factors SET friendly_name = $2, updated_at = NOW() WHERE id = $1",
        )
        .bind(factor_id)
        .bind(name)
        .execute(&mut *tx)
        .await
        .map_err(fail)?;
    }
    if let Some(phone) = params.phone.as_deref().filter(|p| !p.is_empty()) {
        if factor.0 == "phone" {
            let phone = validate_phone(phone)?;
            sqlx::query("UPDATE auth.mfa_factors SET phone = $2, updated_at = NOW() WHERE id = $1")
                .bind(factor_id)
                .bind(&phone)
                .execute(&mut *tx)
                .await
                .map_err(fail)?;
        }
    }
    write_audit(
        &mut tx,
        "00000000-0000-0000-0000-000000000000",
        false,
        &role,
        "factor_updated",
        "factor",
        Some(json!({
            "user_id": user_id.to_string(),
            "factor_id": factor_id.to_string(),
            "factor_type": factor.0,
        })),
    )
    .await?;
    let factors = load_factors(&mut tx, user_id).await?;
    let wanted = factor_id.to_string();
    let body = factors
        .into_iter()
        .find(|item| item.get("id").and_then(Value::as_str) == Some(wanted.as_str()))
        .ok_or_else(|| AuthError::not_found("mfa_factor_not_found", "Factor not found"))?;
    tx.commit().await.map_err(fail)?;
    Ok(json_ok(&body))
}

// ---------------------------------------------------------------------------
// SSO providers
// ---------------------------------------------------------------------------

const NAME_ID_FORMATS: [&str; 4] = [
    "urn:oasis:names:tc:SAML:2.0:nameid-format:persistent",
    "urn:oasis:names:tc:SAML:1.1:nameid-format:emailAddress",
    "urn:oasis:names:tc:SAML:2.0:nameid-format:transient",
    "urn:oasis:names:tc:SAML:1.1:nameid-format:unspecified",
];

#[derive(Debug, Default, Deserialize)]
struct SsoParams {
    #[serde(rename = "type")]
    kind: Option<String>,
    metadata_url: Option<String>,
    metadata_xml: Option<String>,
    domains: Option<Vec<String>>,
    attribute_mapping: Option<Map<String, Value>>,
    name_id_format: Option<String>,
    resource_id: Option<String>,
    disabled: Option<bool>,
}

impl SsoParams {
    fn url(&self) -> &str {
        self.metadata_url.as_deref().unwrap_or("")
    }

    fn xml(&self) -> &str {
        self.metadata_xml.as_deref().unwrap_or("")
    }

    fn name_id(&self) -> &str {
        self.name_id_format.as_deref().unwrap_or("")
    }

    /// `CreateSSOProviderParams.validate`.
    fn validate(&self, for_update: bool) -> Result<(), AuthError> {
        let url = self.url();
        if !for_update && self.kind.as_deref() != Some("saml") {
            return Err(AuthError::validation(
                400,
                "Only 'saml' supported for SSO provider type",
            ));
        } else if !url.is_empty() && !self.xml().is_empty() {
            return Err(AuthError::validation(
                400,
                "Only one of metadata_xml or metadata_url needs to be set",
            ));
        } else if !for_update && url.is_empty() && self.xml().is_empty() {
            return Err(AuthError::validation(
                400,
                "Either metadata_xml or metadata_url must be set",
            ));
        } else if !url.is_empty() {
            if !url.starts_with('/') && !valid_request_uri(url) {
                return Err(AuthError::validation(
                    400,
                    "metadata_url is not a valid URL",
                ));
            }
            if !url.to_ascii_lowercase().starts_with("https://") {
                return Err(AuthError::validation(
                    400,
                    "metadata_url is not a HTTPS URL",
                ));
            }
        }
        let format = self.name_id();
        if !format.is_empty() && !NAME_ID_FORMATS.contains(&format) {
            return Err(AuthError::validation(
                400,
                format!(
                    "name_id_format must be unspecified or one of {}",
                    NAME_ID_FORMATS.join(", ")
                ),
            ));
        }
        Ok(())
    }

    /// Metadata from the request: the XML body, or a fetch this build lacks.
    fn metadata(&self) -> Result<Option<(String, String)>, GenerateLinkError> {
        if !self.xml().is_empty() {
            let entity = parse_saml_metadata(self.xml())?;
            return Ok(Some((self.xml().to_string(), entity)));
        }
        if !self.url().is_empty() {
            return Err(not_impl("sso:metadata_url_fetch"));
        }
        Ok(None)
    }

    /// Stored `attribute_mapping`: `{"keys": {...}}`, or `{}` when empty.
    fn mapping_keys(&self) -> Option<Map<String, Value>> {
        self.attribute_mapping
            .as_ref()
            .and_then(|map| map.get("keys"))
            .and_then(Value::as_object)
            .cloned()
    }
}

fn mapping_value(keys: &Map<String, Value>) -> Value {
    if keys.is_empty() {
        json!({})
    } else {
        json!({ "keys": keys })
    }
}

/// The `entityID` and descriptor checks of `parseSAMLMetadata`. XML this
/// scanner cannot place is left to a full SAML parser, so it answers 501.
fn parse_saml_metadata(xml: &str) -> Result<String, GenerateLinkError> {
    let unsupported = || not_impl("sso:saml_metadata_parse");
    // Constructs the scanner cannot read faithfully: answer 501, never guess.
    if xml.contains("<!--")
        || xml.contains("<![CDATA[")
        || xml.contains("<!DOCTYPE")
        || xml.contains("<!ENTITY")
        || xml.contains("&#")
    {
        return Err(unsupported());
    }
    let root_count = regex::Regex::new(r"<(?:[A-Za-z0-9_.-]+:)?EntityDescriptor\b")
        .map_err(|_| unsupported())?
        .find_iter(xml)
        .count();
    let closing = regex::Regex::new(r"</(?:[A-Za-z0-9_.-]+:)?EntityDescriptor\s*>\s*$")
        .map_err(|_| unsupported())?;
    if root_count != 1 || !closing.is_match(xml) {
        return Err(unsupported());
    }
    let root = regex::Regex::new(r"<(?:[A-Za-z0-9_.-]+:)?EntityDescriptor\b([^>]*)>")
        .map_err(|_| unsupported())?;
    let caps = root.captures(xml).ok_or_else(unsupported)?;
    let attrs = caps.get(1).map_or("", |m| m.as_str());
    let entity = regex::Regex::new(r#"\bentityID\s*=\s*(?:"([^"]*)"|'([^']*)')"#)
        .map_err(|_| unsupported())?
        .captures(attrs)
        .and_then(|c| c.get(1).or_else(|| c.get(2)))
        .map(|m| {
            m.as_str()
                .replace("&lt;", "<")
                .replace("&gt;", ">")
                .replace("&quot;", "\"")
                .replace("&apos;", "'")
                .replace("&amp;", "&")
        })
        .unwrap_or_default();
    if entity.is_empty() {
        return Err(
            AuthError::validation(400, "SAML Metadata does not contain an EntityID").into(),
        );
    }
    let idp = regex::Regex::new(r"<(?:[A-Za-z0-9_.-]+:)?IDPSSODescriptor\b")
        .map_err(|_| unsupported())?
        .find_iter(xml)
        .count();
    if idp < 1 {
        return Err(AuthError::validation(
            400,
            "SAML Metadata does not contain any IDPSSODescriptor",
        )
        .into());
    }
    if idp > 1 {
        return Err(AuthError::validation(
            400,
            "SAML Metadata contains multiple IDPSSODescriptors",
        )
        .into());
    }
    Ok(entity)
}

async fn domain_owner(db: &sqlx::PgPool, domain: &str) -> Result<Option<Uuid>, AuthError> {
    sqlx::query_as::<_, (Uuid,)>(
        "SELECT sso_provider_id FROM auth.sso_domains WHERE domain = $1 LIMIT 1",
    )
    .bind(domain)
    .fetch_optional(db)
    .await
    .map(|row| row.map(|(id,)| id))
    .map_err(db_err("Database error finding SSO Identity Provider"))
}

// megabase:unit auth:route:POST /auth/v1/admin/sso/providers
pub(crate) async fn create_sso_provider(
    State(state): State<AuthState>,
    headers: HeaderMap,
    body: Bytes,
) -> Reply {
    require_admin(&state, &headers)?;
    let params: SsoParams = params(&body)?;
    params.validate(false)?;
    let Some((raw_xml, entity_id)) = params.metadata()? else {
        return Err(AuthError::internal("Error creating SSO provider").into());
    };
    let db = pool(&state)?;
    let fail = db_err("Database error finding SSO Identity Provider");
    let existing = sqlx::query_as::<_, (Uuid,)>(
        "SELECT sso_provider_id FROM auth.saml_providers WHERE entity_id = $1 LIMIT 1",
    )
    .bind(&entity_id)
    .fetch_optional(&db)
    .await
    .map_err(fail)?;
    if existing.is_some() {
        return Err(AuthError::unprocessable(
            "saml_idp_already_exists",
            format!("SAML Identity Provider with this EntityID ({entity_id}) already exists"),
        )
        .into());
    }
    let domains = params.domains.clone().unwrap_or_default();
    for domain in &domains {
        if let Some(owner) = domain_owner(&db, domain).await? {
            return Err(AuthError::new(
                400,
                "sso_domain_already_exists",
                format!(
                    "SSO Domain '{domain}' is already assigned to an SSO identity provider ({owner})"
                ),
            )
            .into());
        }
    }
    let provider_id = Uuid::new_v4();
    let mapping = mapping_value(&params.mapping_keys().unwrap_or_default());
    let fail = || AuthError::internal("Database error creating SSO provider");
    let mut tx = db.begin().await.map_err(|_| fail())?;
    sqlx::query(
        "INSERT INTO auth.sso_providers (id, resource_id, disabled, created_at, updated_at)
         VALUES ($1, $2, $3, NOW(), NOW())",
    )
    .bind(provider_id)
    .bind(params.resource_id.as_deref().and_then(empty_as_none))
    .bind(params.disabled)
    .execute(&mut *tx)
    .await
    .map_err(|_| fail())?;
    sqlx::query(
        "INSERT INTO auth.saml_providers (id, sso_provider_id, entity_id, metadata_xml, metadata_url,
                                          attribute_mapping, name_id_format, created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, NOW(), NOW())",
    )
    .bind(Uuid::new_v4())
    .bind(provider_id)
    .bind(&entity_id)
    .bind(&raw_xml)
    .bind(empty_as_none(params.url()))
    .bind(&mapping)
    .bind(empty_as_none(params.name_id()))
    .execute(&mut *tx)
    .await
    .map_err(|_| fail())?;
    for domain in &domains {
        sqlx::query(
            "INSERT INTO auth.sso_domains (id, sso_provider_id, domain, created_at, updated_at)
             VALUES ($1, $2, $3, NOW(), NOW())",
        )
        .bind(Uuid::new_v4())
        .bind(provider_id)
        .bind(domain)
        .execute(&mut *tx)
        .await
        .map_err(|_| fail())?;
    }
    let row = load_one_sso(&mut tx, &provider_id.to_string()).await?;
    let body = sso_json(&mut tx, &row, true).await?;
    tx.commit().await.map_err(|_| fail())?;
    Ok(json_status(StatusCode::CREATED, &body))
}

#[derive(Debug, sqlx::FromRow)]
struct SamlState {
    entity_id: String,
    attribute_mapping: Option<Value>,
    name_id_format: Option<String>,
}

// megabase:unit auth:route:PUT /auth/v1/admin/sso/providers/{idp_id}
pub(crate) async fn update_sso_provider(
    State(state): State<AuthState>,
    headers: HeaderMap,
    Path(idp_id): Path<String>,
    body: Bytes,
) -> Reply {
    require_admin(&state, &headers)?;
    let db = pool(&state)?;
    let mut conn = db
        .acquire()
        .await
        .map_err(db_err("Database error finding SSO Identity Provider"))?;
    let row = load_one_sso(&mut conn, &idp_id).await?;
    drop(conn);
    let params: SsoParams = params(&body)?;
    params.validate(true)?;
    let provider_id = row.id;
    let fail = db_err("Database error finding SSO Identity Provider");
    let saml = sqlx::query_as::<_, SamlState>(
        "SELECT entity_id, attribute_mapping, name_id_format FROM auth.saml_providers WHERE sso_provider_id = $1",
    )
    .bind(provider_id)
    .fetch_optional(&db)
    .await
    .map_err(fail)?;
    let Some(saml) = saml else {
        return Err(AuthError::not_found(
            "sso_provider_not_found",
            "SSO Identity Provider not found",
        )
        .into());
    };
    let mut modified = false;
    let mut new_xml: Option<(String, Option<String>)> = None;
    if !params.xml().is_empty() || !params.url().is_empty() {
        if let Some((raw_xml, entity)) = params.metadata()? {
            if entity != saml.entity_id {
                return Err(AuthError::new(
                    400,
                    "saml_entity_id_mismatch",
                    format!(
                        "SAML Metadata can be updated only if the EntityID matches for the provider; expected '{}' but got '{}'",
                        saml.entity_id, entity
                    ),
                )
                .into());
            }
            let url = empty_as_none(params.url()).map(str::to_string);
            new_xml = Some((raw_xml, url));
            modified = true;
        }
    }
    let current_domains: Vec<String> = sqlx::query_as::<_, (String,)>(
        "SELECT domain FROM auth.sso_domains WHERE sso_provider_id = $1",
    )
    .bind(provider_id)
    .fetch_all(&db)
    .await
    .map_err(db_err("Database error finding SSO Identity Provider"))?
    .into_iter()
    .map(|(domain,)| domain)
    .collect();
    let update_domains = params.domains.is_some();
    let mut create_domains: Vec<String> = Vec::new();
    let mut keep: Vec<&str> = Vec::new();
    for domain in params.domains.iter().flatten() {
        match domain_owner(&db, domain).await? {
            Some(owner) if owner == provider_id => keep.push(domain),
            Some(owner) => {
                return Err(AuthError::new(
                    400,
                    "sso_domain_already_exists",
                    format!("SSO domain '{domain}' already assigned to another provider ({owner})"),
                )
                .into())
            }
            None => {
                modified = true;
                create_domains.push(domain.clone());
            }
        }
    }
    let mut delete_domains: Vec<String> = Vec::new();
    if update_domains {
        for domain in &current_domains {
            if !keep.contains(&domain.as_str()) {
                modified = true;
                delete_domains.push(domain.clone());
            }
        }
    }
    let mut new_mapping: Option<Value> = None;
    if let Some(keys) = params.mapping_keys() {
        let stored = saml
            .attribute_mapping
            .as_ref()
            .and_then(|value| value.get("keys"))
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        if stored != keys {
            modified = true;
            new_mapping = Some(mapping_value(&keys));
        }
    }
    let stored_format = saml.name_id_format.clone().unwrap_or_default();
    let mut format_change = false;
    if params.name_id() != stored_format {
        modified = true;
        format_change = true;
    }
    let mut new_resource: Option<Option<String>> = None;
    if let Some(resource) = params.resource_id.as_deref() {
        let current = row.resource_id.clone();
        if resource.is_empty() && current.is_some() {
            new_resource = Some(None);
            modified = true;
        } else if !resource.is_empty() && current.as_deref() != Some(resource) {
            new_resource = Some(Some(resource.to_string()));
            modified = true;
        }
    }
    let mut new_disabled: Option<bool> = None;
    if let Some(disabled) = params.disabled {
        if row.disabled != Some(disabled) {
            new_disabled = Some(disabled);
            modified = true;
        }
    }
    if modified {
        let conflict = || {
            AuthError::unprocessable(
                "conflict",
                "Updating SSO provider failed, likely due to a conflict. Try again?",
            )
        };
        let mut tx = db.begin().await.map_err(|_| conflict())?;
        let result: Result<(), sqlx::Error> = async {
            sqlx::query("UPDATE auth.sso_providers SET updated_at = NOW() WHERE id = $1")
                .bind(provider_id)
                .execute(&mut *tx)
                .await?;
            if let Some(resource) = &new_resource {
                sqlx::query("UPDATE auth.sso_providers SET resource_id = $2 WHERE id = $1")
                    .bind(provider_id)
                    .bind(resource.as_deref())
                    .execute(&mut *tx)
                    .await?;
            }
            if let Some(disabled) = new_disabled {
                sqlx::query("UPDATE auth.sso_providers SET disabled = $2 WHERE id = $1")
                    .bind(provider_id)
                    .bind(disabled)
                    .execute(&mut *tx)
                    .await?;
            }
            if let Some((xml, url)) = &new_xml {
                sqlx::query(
                    "UPDATE auth.saml_providers SET metadata_xml = $2,
                        metadata_url = COALESCE($3, metadata_url), updated_at = NOW()
                     WHERE sso_provider_id = $1",
                )
                .bind(provider_id)
                .bind(xml)
                .bind(url.as_deref())
                .execute(&mut *tx)
                .await?;
            }
            if let Some(mapping) = &new_mapping {
                sqlx::query("UPDATE auth.saml_providers SET attribute_mapping = $2, updated_at = NOW() WHERE sso_provider_id = $1")
                    .bind(provider_id)
                    .bind(mapping)
                    .execute(&mut *tx)
                    .await?;
            }
            if format_change {
                sqlx::query("UPDATE auth.saml_providers SET name_id_format = $2, updated_at = NOW() WHERE sso_provider_id = $1")
                    .bind(provider_id)
                    .bind(empty_as_none(params.name_id()))
                    .execute(&mut *tx)
                    .await?;
            }
            if update_domains {
                for domain in &delete_domains {
                    sqlx::query("DELETE FROM auth.sso_domains WHERE sso_provider_id = $1 AND domain = $2")
                        .bind(provider_id)
                        .bind(domain)
                        .execute(&mut *tx)
                        .await?;
                }
                for domain in &create_domains {
                    sqlx::query(
                        "INSERT INTO auth.sso_domains (id, sso_provider_id, domain, created_at, updated_at)
                         VALUES ($1, $2, $3, NOW(), NOW())",
                    )
                    .bind(Uuid::new_v4())
                    .bind(provider_id)
                    .bind(domain)
                    .execute(&mut *tx)
                    .await?;
                }
            }
            Ok(())
        }
        .await;
        if result.is_err() {
            return Err(conflict().into());
        }
        tx.commit().await.map_err(|_| conflict())?;
    }
    let mut conn = db
        .acquire()
        .await
        .map_err(db_err("Database error finding SSO Identity Provider"))?;
    let row = load_one_sso(&mut conn, &provider_id.to_string()).await?;
    let body = sso_json(&mut conn, &row, true).await?;
    Ok(json_ok(&body))
}

// ---------------------------------------------------------------------------
// OAuth clients
// ---------------------------------------------------------------------------

const AUTH_METHODS: [&str; 3] = ["none", "client_secret_basic", "client_secret_post"];

#[derive(Debug, Default, Deserialize)]
struct OAuthUpdate {
    redirect_uris: Option<Vec<String>>,
    grant_types: Option<Vec<String>>,
    client_name: Option<String>,
    client_uri: Option<String>,
    logo_uri: Option<String>,
    token_endpoint_auth_method: Option<String>,
}

const OAUTH_COLUMNS: &str = "id, client_type::text AS client_type, redirect_uris, token_endpoint_auth_method,
                grant_types, client_name, client_uri, logo_uri, registration_type::text AS registration_type,
                created_at, updated_at";

/// `LoadOAuthServerClient`: the id check, then the lookup.
async fn load_client(
    state: &AuthState,
    client_id: &str,
) -> Result<(sqlx::PgPool, Uuid, String), AuthError> {
    if client_id.is_empty() {
        return Err(AuthError::validation(400, "client_id is required"));
    }
    if !is_uuid(client_id) {
        return Err(AuthError::validation(400, "invalid client_id format"));
    }
    let id = Uuid::parse_str(client_id)
        .map_err(|_| AuthError::validation(400, "invalid client_id format"))?;
    let db = pool(state)?;
    let found = sqlx::query_as::<_, (String,)>(
        "SELECT client_type::text FROM auth.oauth_clients WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(id)
    .fetch_optional(&db)
    .await
    .map_err(db_err("Error loading OAuth client"))?
    .ok_or_else(|| AuthError::not_found("oauth_client_not_found", "OAuth client not found"))?;
    Ok((db, id, found.0))
}

async fn client_row(db: &sqlx::PgPool, id: Uuid) -> Result<OAuthClientRow, AuthError> {
    sqlx::query_as::<_, OAuthClientRow>(&format!(
        "SELECT {OAUTH_COLUMNS} FROM auth.oauth_clients WHERE id = $1 AND deleted_at IS NULL"
    ))
    .bind(id)
    .fetch_one(db)
    .await
    .map_err(db_err("Error updating OAuth client"))
}

fn validate_oauth_update(params: &OAuthUpdate) -> Result<(), AuthError> {
    let bad = |message: String| AuthError::validation(400, message);
    if let Some(uris) = &params.redirect_uris {
        if uris.is_empty() {
            return Err(bad("redirect_uris cannot be empty".into()));
        }
        if uris.len() > 10 {
            return Err(bad("redirect_uris cannot exceed 10 items".into()));
        }
        for uri in uris {
            if let Err(reason) = validate_redirect_uri(uri) {
                return Err(bad(format!("invalid redirect_uri '{uri}': {reason}")));
            }
        }
    }
    if let Some(grants) = &params.grant_types {
        if grants.is_empty() {
            return Err(bad("grant_types cannot be empty".into()));
        }
        if grants
            .iter()
            .any(|grant| grant != "authorization_code" && grant != "refresh_token")
        {
            return Err(bad(
                "grant_types must only contain 'authorization_code' and/or 'refresh_token'".into(),
            ));
        }
    }
    if params
        .client_name
        .as_deref()
        .is_some_and(|name| name.len() > 1024)
    {
        return Err(bad("client_name cannot exceed 1024 characters".into()));
    }
    for (field, value) in [
        ("client_uri", &params.client_uri),
        ("logo_uri", &params.logo_uri),
    ] {
        let Some(value) = value.as_deref().filter(|value| !value.is_empty()) else {
            continue;
        };
        if value.len() > 2048 {
            return Err(bad(format!("{field} cannot exceed 2048 characters")));
        }
        if !valid_request_uri(value) && !value.starts_with('/') {
            return Err(bad(format!("{field} must be a valid URL")));
        }
    }
    if let Some(method) = &params.token_endpoint_auth_method {
        if !AUTH_METHODS.contains(&method.as_str()) {
            return Err(bad(
                "invalid token_endpoint_auth_method: must be one of [none client_secret_basic client_secret_post]"
                    .into(),
            ));
        }
    }
    Ok(())
}

// megabase:unit auth:route:PUT /auth/v1/admin/oauth/clients/{client_id}
pub(crate) async fn update_oauth_client(
    State(state): State<AuthState>,
    headers: HeaderMap,
    Path(client_id): Path<String>,
    body: Bytes,
) -> Result<Response, AuthError> {
    require_admin(&state, &headers)?;
    require_oauth_server(&state)?;
    let (db, id, client_type) = load_client(&state, &client_id).await?;
    limit_body(&body)?;
    let update: OAuthUpdate = serde_json::from_slice(&body)
        .map_err(|_| AuthError::new(400, "bad_json", "Invalid JSON body"))?;
    if update.redirect_uris.is_none()
        && update.grant_types.is_none()
        && update.client_name.is_none()
        && update.client_uri.is_none()
        && update.logo_uri.is_none()
        && update.token_endpoint_auth_method.is_none()
    {
        return Err(AuthError::validation(400, "No fields provided for update"));
    }
    validate_oauth_update(&update)?;
    if let Some(method) = update.token_endpoint_auth_method.as_deref() {
        let valid: &[&str] = match client_type.as_str() {
            "public" => &["none"],
            "confidential" => &["client_secret_basic", "client_secret_post"],
            _ => &[],
        };
        if !valid.contains(&method) {
            return Err(AuthError::validation(
                400,
                format!(
                    "token_endpoint_auth_method '{method}' is not valid for client_type '{client_type}'; valid methods: [{}]",
                    valid.join(" ")
                ),
            ));
        }
    }
    let fail = db_err("Error updating OAuth client");
    sqlx::query(
        "UPDATE auth.oauth_clients SET
            redirect_uris = COALESCE($2, redirect_uris),
            grant_types = COALESCE($3, grant_types),
            client_name = COALESCE($4, client_name),
            client_uri = COALESCE($5, client_uri),
            logo_uri = COALESCE($6, logo_uri),
            token_endpoint_auth_method = COALESCE($7, token_endpoint_auth_method),
            updated_at = NOW()
         WHERE id = $1",
    )
    .bind(id)
    .bind(update.redirect_uris.as_ref().map(|uris| uris.join(",")))
    .bind(update.grant_types.as_ref().map(|grants| grants.join(",")))
    .bind(update.client_name.as_deref())
    .bind(update.client_uri.as_deref())
    .bind(update.logo_uri.as_deref())
    .bind(update.token_endpoint_auth_method.as_deref())
    .execute(&db)
    .await
    .map_err(fail)?;
    let row = client_row(&db, id).await?;
    Ok(json_ok(&oauth_client_json(&row)))
}

// megabase:unit auth:route:POST /auth/v1/admin/oauth/clients/{client_id}/regenerate_secret
pub(crate) async fn regenerate_oauth_secret(
    State(state): State<AuthState>,
    headers: HeaderMap,
    Path(client_id): Path<String>,
) -> Result<Response, AuthError> {
    require_admin(&state, &headers)?;
    require_oauth_server(&state)?;
    let (db, id, client_type) = load_client(&state, &client_id).await?;
    if client_type != "confidential" {
        return Err(AuthError::validation(
            400,
            "Cannot regenerate secret for public client",
        ));
    }
    let secret = URL_SAFE_NO_PAD.encode(random_bytes(32)?);
    let mut hasher = Sha256::new();
    hasher.update(secret.as_bytes());
    let hash = URL_SAFE_NO_PAD.encode(hasher.finalize());
    sqlx::query(
        "UPDATE auth.oauth_clients SET client_secret_hash = $2, updated_at = NOW() WHERE id = $1",
    )
    .bind(id)
    .bind(&hash)
    .execute(&db)
    .await
    .map_err(db_err("Error regenerating OAuth client secret"))?;
    let row = client_row(&db, id).await?;
    let mut body = oauth_client_json(&row);
    if let Value::Object(map) = &mut body {
        map.insert("client_secret".into(), json!(secret));
    }
    Ok(json_ok(&body))
}

// ---------------------------------------------------------------------------
// Custom providers
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Deserialize)]
struct CustomUpdate {
    name: Option<String>,
    client_id: Option<String>,
    client_secret: Option<String>,
    acceptable_client_ids: Option<Vec<String>>,
    scopes: Option<Vec<String>>,
    pkce_enabled: Option<bool>,
    attribute_mapping: Option<Map<String, Value>>,
    custom_claims_allowlist: Option<Vec<String>>,
    authorization_params: Option<Map<String, Value>>,
    enabled: Option<bool>,
    email_optional: Option<bool>,
    issuer: Option<String>,
    discovery_url: Option<String>,
    skip_nonce_check: Option<bool>,
    authorization_url: Option<String>,
    token_url: Option<String>,
    userinfo_url: Option<String>,
    jwks_uri: Option<String>,
}

#[derive(Debug, sqlx::FromRow)]
struct CustomState {
    provider_type: String,
    name: String,
    client_id: String,
    acceptable_client_ids: Vec<String>,
    scopes: Vec<String>,
    pkce_enabled: bool,
    attribute_mapping: Value,
    custom_claims_allowlist: Vec<String>,
    authorization_params: Value,
    enabled: bool,
    email_optional: bool,
    issuer: Option<String>,
    discovery_url: Option<String>,
    skip_nonce_check: bool,
    authorization_url: Option<String>,
    token_url: Option<String>,
    userinfo_url: Option<String>,
    jwks_uri: Option<String>,
}

fn nonempty(value: &Option<String>) -> Option<&str> {
    value.as_deref().filter(|value| !value.is_empty())
}

// megabase:unit auth:route:PUT /auth/v1/admin/custom-providers/{identifier}
pub(crate) async fn update_custom_provider(
    State(state): State<AuthState>,
    headers: HeaderMap,
    Path(identifier): Path<String>,
    body: Bytes,
) -> Reply {
    require_admin(&state, &headers)?;
    require_custom_oauth(&state)?;
    if identifier.is_empty() {
        return Err(AuthError::validation(400, "identifier is required").into());
    }
    if !identifier.starts_with("custom:") {
        return Err(AuthError::validation(
            400,
            format!("identifier must start with 'custom:' prefix, e.g. 'custom:{identifier}'"),
        )
        .into());
    }
    let update: CustomUpdate = params(&body)?;
    validate_auth_params(update.authorization_params.as_ref())?;
    validate_attribute_mapping(update.attribute_mapping.as_ref())?;
    validate_claims(update.custom_claims_allowlist.as_deref())?;
    let db = pool(&state)?;
    let mut row = sqlx::query_as::<_, CustomState>(
        "SELECT provider_type, name, client_id, acceptable_client_ids, scopes, pkce_enabled,
                attribute_mapping, custom_claims_allowlist, authorization_params, enabled,
                email_optional, issuer, discovery_url, skip_nonce_check, authorization_url,
                token_url, userinfo_url, jwks_uri
         FROM auth.custom_oauth_providers WHERE identifier = $1",
    )
    .bind(&identifier)
    .fetch_optional(&db)
    .await
    .map_err(db_err("Error retrieving custom OAuth provider"))?
    .ok_or_else(|| {
        AuthError::not_found(
            "custom_provider_not_found",
            "Custom OAuth provider not found",
        )
    })?;
    if let Some(name) = nonempty(&update.name) {
        row.name = name.to_string();
    }
    if let Some(client_id) = nonempty(&update.client_id) {
        row.client_id = client_id.to_string();
    }
    if let Some(ids) = &update.acceptable_client_ids {
        row.acceptable_client_ids.clone_from(ids);
    }
    if let Some(scopes) = &update.scopes {
        row.scopes.clone_from(scopes);
        if row.provider_type == "oidc" && !row.scopes.iter().any(|scope| scope == "openid") {
            row.scopes.insert(0, "openid".into());
        }
    }
    if let Some(pkce) = update.pkce_enabled {
        row.pkce_enabled = pkce;
    }
    if let Some(mapping) = &update.attribute_mapping {
        row.attribute_mapping = Value::Object(mapping.clone());
    }
    if let Some(claims) = &update.custom_claims_allowlist {
        row.custom_claims_allowlist.clone_from(claims);
    }
    if let Some(params) = &update.authorization_params {
        row.authorization_params = Value::Object(params.clone());
    }
    if let Some(enabled) = update.enabled {
        row.enabled = enabled;
    }
    if let Some(optional) = update.email_optional {
        row.email_optional = optional;
    }
    let oidc = row.provider_type == "oidc";
    if oidc {
        if let Some(issuer) = nonempty(&update.issuer) {
            crate::admin_batch2::validate_oauth_url(issuer).await?;
            row.issuer = Some(issuer.to_string());
        }
        if let Some(url) = nonempty(&update.discovery_url) {
            crate::admin_batch2::validate_oauth_url(url).await?;
            row.discovery_url = Some(url.to_string());
        }
        if let Some(skip) = update.skip_nonce_check {
            row.skip_nonce_check = skip;
        }
    } else if row.provider_type == "oauth2" {
        for (value, slot) in [
            (&update.authorization_url, &mut row.authorization_url),
            (&update.token_url, &mut row.token_url),
            (&update.userinfo_url, &mut row.userinfo_url),
            (&update.jwks_uri, &mut row.jwks_uri),
        ] {
            if let Some(url) = nonempty(value) {
                crate::admin_batch2::validate_oauth_url(url).await?;
                *slot = Some(url.to_string());
            }
        }
    }
    if oidc && (nonempty(&update.issuer).is_some() || nonempty(&update.discovery_url).is_some()) {
        return Err(not_impl("custom-provider:oidc_discovery"));
    }
    let secret = nonempty(&update.client_secret);
    sqlx::query(
        "UPDATE auth.custom_oauth_providers SET
            name = $2, client_id = $3, acceptable_client_ids = $4, scopes = $5,
            pkce_enabled = $6, attribute_mapping = $7, custom_claims_allowlist = $8,
            authorization_params = $9, enabled = $10, email_optional = $11,
            issuer = $12, discovery_url = $13, skip_nonce_check = $14,
            authorization_url = $15, token_url = $16, userinfo_url = $17, jwks_uri = $18,
            client_secret = COALESCE($19, client_secret), updated_at = NOW()
         WHERE identifier = $1",
    )
    .bind(&identifier)
    .bind(&row.name)
    .bind(&row.client_id)
    .bind(&row.acceptable_client_ids)
    .bind(&row.scopes)
    .bind(row.pkce_enabled)
    .bind(&row.attribute_mapping)
    .bind(&row.custom_claims_allowlist)
    .bind(&row.authorization_params)
    .bind(row.enabled)
    .bind(row.email_optional)
    .bind(row.issuer.as_deref())
    .bind(row.discovery_url.as_deref())
    .bind(row.skip_nonce_check)
    .bind(row.authorization_url.as_deref())
    .bind(row.token_url.as_deref())
    .bind(row.userinfo_url.as_deref())
    .bind(row.jwks_uri.as_deref())
    .bind(secret)
    .execute(&db)
    .await
    .map_err(db_err("Error updating custom OAuth provider"))?;
    let updated = sqlx::query_as::<_, CustomProviderRow>(
        "SELECT id, provider_type, identifier, name, client_id,
                acceptable_client_ids, scopes, pkce_enabled, attribute_mapping,
                custom_claims_allowlist, authorization_params, enabled, email_optional,
                issuer, discovery_url, skip_nonce_check, cached_discovery,
                authorization_url, token_url, userinfo_url, jwks_uri,
                created_at, updated_at
         FROM auth.custom_oauth_providers WHERE identifier = $1",
    )
    .bind(&identifier)
    .fetch_one(&db)
    .await
    .map_err(db_err("Error updating custom OAuth provider"))?;
    Ok(json_ok(&crate::admin::custom_provider_json(&updated)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = r#"<md:EntityDescriptor xmlns:md="urn:oasis:names:tc:SAML:2.0:metadata" entityID="https://idp.example/x"><md:IDPSSODescriptor/></md:EntityDescriptor>"#;

    fn is_501(result: Result<String, GenerateLinkError>) -> bool {
        matches!(result, Err(GenerateLinkError::NotImplemented(_)))
    }

    #[test]
    fn saml_metadata_simple_document_parses() {
        assert!(matches!(parse_saml_metadata(GOOD), Ok(id) if id == "https://idp.example/x"));
    }

    #[test]
    fn saml_metadata_unplaceable_xml_is_501() {
        let truncated = r#"<EntityDescriptor entityID="x"><IDPSSODescriptor>"#;
        assert!(is_501(parse_saml_metadata(truncated)));
        let char_ref = GOOD.replace("idp.example/x", "idp.example/&#65;");
        assert!(is_501(parse_saml_metadata(&char_ref)));
        let comment = GOOD.replace("<md:IDPSSODescriptor/>", "<!-- <md:IDPSSODescriptor/> -->");
        assert!(is_501(parse_saml_metadata(&comment)));
        let cdata = GOOD.replace(
            "<md:IDPSSODescriptor/>",
            "<![CDATA[<md:IDPSSODescriptor/>]]>",
        );
        assert!(is_501(parse_saml_metadata(&cdata)));
        let nested = format!("<outer>{GOOD}{GOOD}</outer>");
        assert!(is_501(parse_saml_metadata(&nested)));
    }
}
