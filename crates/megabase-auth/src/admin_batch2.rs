// Ported from supabase/auth (MIT), pin v2.197.0:
//   internal/api/admin.go (adminUsers, adminUserGet, adminUserGetFactors)
//   internal/api/pagination.go
//   internal/api/sorting.go
//   internal/api/ssoadmin.go
//   internal/api/passkey_admin.go
//   internal/api/mail.go (adminGenerateLink)
//   internal/api/custom_oauth_admin.go
//   internal/api/oauthserver/handlers.go
//   internal/api/oauthserver/service.go
//   internal/utilities/url_validator.go
//   internal/utilities/request.go (GetReferrer, IsRedirectURLValid)
//   internal/crypto/crypto.go

//! Issue #7 admin reads and creates.

use std::collections::BTreeMap;
use std::time::{Duration, SystemTime};

use axum::{
    body::Bytes,
    extract::{OriginalUri, Path, State},
    http::{HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::{DateTime, Utc};
use megabase_core::MegabaseNotImplemented;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha224, Sha256};
use uuid::Uuid;

use crate::admin::{
    custom_provider_json, is_uuid, nil_instance, oauth_client_json, parse_uint, pool,
    require_admin, require_custom_oauth, require_oauth_server, system_time_rfc3339, ts_json,
    verified_admin_role, CustomProviderRow, OAuthClientRow,
};
use crate::error::{go_quote, MAX_BODY_BYTES};
use crate::http::{json_ok, json_status, AuthError};
use crate::routes::request_aud;
use crate::state::AuthState;

const AUTH_PREFIX: &str = "/auth/v1";
const USER_NOT_FOUND: &str = "User not found";
const DUPLICATE_EMAIL: &str = "A user with this email address has already been registered";
/// Bound for `tokio::net::lookup_host` so a slow resolver cannot stall a worker.
const OAUTH_DNS_TIMEOUT: Duration = Duration::from_secs(5);

// megabase:unit auth:route:GET /auth/v1/admin/users
pub(crate) async fn list_users(
    State(state): State<AuthState>,
    headers: HeaderMap,
    uri: axum::http::Uri,
) -> Result<Response, AuthError> {
    require_admin(&state, &headers)?;
    let query = query_map(uri.query());
    let aud = request_aud(&headers, &state.config);
    let sort = sort_directions(&query)?;
    if use_cursor(&state, &query) {
        return list_users_keyset(&state, &uri, &query, &aud, &sort).await;
    }
    let page = parse_uint(first(&query, "page"), 1)?;
    let per_page = parse_uint(first(&query, "per_page"), 50)?;
    let db = pool(&state)?;
    let filter = first(&query, "filter").filter(|value| !value.is_empty());
    let order = sort
        .iter()
        .map(|dir| format!("created_at {dir}"))
        .collect::<Vec<_>>()
        .join(", ");
    let rows = fetch_users(&db, &aud, filter, &order, page, per_page).await?;
    let total = count_users(&db, &aud, filter).await?;
    let users = rows
        .iter()
        .map(|row| user_json(row, None, &[]))
        .collect::<Vec<_>>();
    let mut response = json_ok(&json!({ "users": users, "aud": aud }));
    write_offset_page(&mut response, &uri, page, per_page, total);
    Ok(response)
}

// megabase:unit auth:route:GET /auth/v1/admin/users/{user_id}
pub(crate) async fn get_user(
    State(state): State<AuthState>,
    headers: HeaderMap,
    Path(user_id): Path<String>,
) -> Result<Response, AuthError> {
    require_admin(&state, &headers)?;
    let id = parse_user_id(&user_id)?;
    let db = pool(&state)?;
    let mut conn = db
        .acquire()
        .await
        .map_err(|_| AuthError::internal("Database error loading user"))?;
    let user = load_user_json(&mut conn, id).await?;
    Ok(json_ok(&user))
}

// megabase:unit auth:route:GET /auth/v1/admin/users/{user_id}/factors
pub(crate) async fn get_user_factors(
    State(state): State<AuthState>,
    headers: HeaderMap,
    Path(user_id): Path<String>,
) -> Result<Response, AuthError> {
    require_admin(&state, &headers)?;
    let id = parse_user_id(&user_id)?;
    let db = pool(&state)?;
    let mut conn = db
        .acquire()
        .await
        .map_err(|_| AuthError::internal("Database error loading user"))?;
    let _user = load_user_row(&mut conn, id).await?;
    let factors = load_factors(&mut conn, id).await?;
    Ok(json_ok(&Value::Array(factors)))
}

// megabase:unit auth:route:GET /auth/v1/admin/users/{user_id}/passkeys
pub(crate) async fn get_user_passkeys(
    State(state): State<AuthState>,
    headers: HeaderMap,
    Path(user_id): Path<String>,
) -> Result<Response, AuthError> {
    require_admin(&state, &headers)?;
    let id = parse_user_id(&user_id)?;
    let db = pool(&state)?;
    let mut conn = db
        .acquire()
        .await
        .map_err(|_| AuthError::internal("Database error loading user"))?;
    let _user = load_user_row(&mut conn, id).await?;
    let rows = sqlx::query_as::<_, PasskeyRow>(
        "SELECT id, friendly_name, created_at, last_used_at
         FROM auth.webauthn_credentials WHERE user_id = $1 ORDER BY created_at ASC",
    )
    .bind(id)
    .fetch_all(&mut *conn)
    .await
    .map_err(|_| AuthError::internal("Database error loading passkeys"))?;
    let items = rows
        .iter()
        .map(|row| {
            let mut item = Map::new();
            item.insert("id".into(), json!(row.id.to_string()));
            if !row.friendly_name.is_empty() {
                item.insert("friendly_name".into(), json!(row.friendly_name));
            }
            item.insert("created_at".into(), ts_json(Some(row.created_at)));
            if let Some(used) = row.last_used_at {
                item.insert("last_used_at".into(), ts_json(Some(used)));
            }
            Value::Object(item)
        })
        .collect::<Vec<_>>();
    Ok(json_ok(&Value::Array(items)))
}

// megabase:unit auth:route:GET /auth/v1/admin/sso/providers
pub(crate) async fn list_sso_providers(
    State(state): State<AuthState>,
    headers: HeaderMap,
    uri: axum::http::Uri,
) -> Result<Response, AuthError> {
    require_admin(&state, &headers)?;
    let query = query_map(uri.query());
    let db = pool(&state)?;
    let rows = load_sso_rows(
        &db,
        first(&query, "resource_id"),
        first(&query, "resource_id_prefix"),
    )
    .await?;
    let mut items = Vec::with_capacity(rows.len());
    let mut conn = db
        .acquire()
        .await
        .map_err(|_| AuthError::internal("error loading all SAML SSO providers"))?;
    for row in rows {
        items.push(sso_json(&mut conn, &row, false).await?);
    }
    Ok(json_ok(&json!({ "items": items })))
}

// megabase:unit auth:route:GET /auth/v1/admin/sso/providers/{idp_id}
pub(crate) async fn get_sso_provider(
    State(state): State<AuthState>,
    headers: HeaderMap,
    Path(idp_id): Path<String>,
) -> Result<Response, AuthError> {
    require_admin(&state, &headers)?;
    if !idp_id.starts_with("resource_") && !is_uuid(&idp_id) {
        return Err(AuthError::not_found(
            "sso_provider_not_found",
            "SSO Identity Provider not found",
        ));
    }
    let db = pool(&state)?;
    let mut conn = db
        .acquire()
        .await
        .map_err(|_| AuthError::internal("Database error finding SSO Identity Provider"))?;
    let row = load_one_sso(&mut conn, &idp_id).await?;
    let body = sso_json(&mut conn, &row, true).await?;
    Ok(json_ok(&body))
}

// megabase:unit auth:route:GET /auth/v1/admin/oauth/clients/{client_id}
pub(crate) async fn get_oauth_client(
    State(state): State<AuthState>,
    headers: HeaderMap,
    Path(client_id): Path<String>,
) -> Result<Response, AuthError> {
    require_admin(&state, &headers)?;
    require_oauth_server(&state)?;
    if client_id.is_empty() {
        return Err(AuthError::validation(400, "client_id is required"));
    }
    if !is_uuid(&client_id) {
        return Err(AuthError::validation(400, "invalid client_id format"));
    }
    let id = Uuid::parse_str(&client_id)
        .map_err(|_| AuthError::validation(400, "invalid client_id format"))?;
    let db = pool(&state)?;
    let row = sqlx::query_as::<_, OAuthClientRow>(
        "SELECT id, client_type::text AS client_type, redirect_uris, token_endpoint_auth_method,
                grant_types, client_name, client_uri, logo_uri, registration_type::text AS registration_type,
                created_at, updated_at
         FROM auth.oauth_clients WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(id)
    .fetch_optional(&db)
    .await
    .map_err(|_| AuthError::internal("Error loading OAuth client"))?
    .ok_or_else(|| AuthError::not_found("oauth_client_not_found", "OAuth client not found"))?;
    Ok(json_ok(&oauth_client_json(&row)))
}

// megabase:unit auth:route:POST /auth/v1/admin/oauth/clients
pub(crate) async fn post_oauth_client(
    State(state): State<AuthState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, AuthError> {
    require_admin(&state, &headers)?;
    require_oauth_server(&state)?;
    limit_body(&body)?;
    let params: OAuthRegister = serde_json::from_slice(&body)
        .map_err(|_| AuthError::new(400, "bad_json", "Invalid JSON body"))?;
    match register_oauth_client(&state, &params).await {
        Ok(body) => Ok(json_status(StatusCode::CREATED, &body)),
        Err(message) => Err(AuthError::validation(400, message)),
    }
}

// megabase:unit auth:route:POST /auth/v1/admin/custom-providers
pub(crate) async fn post_custom_provider(
    State(state): State<AuthState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, AuthError> {
    require_admin(&state, &headers)?;
    require_custom_oauth(&state)?;
    limit_body(&body)?;
    let params: CustomParams = serde_json::from_slice(&body).map_err(|err| {
        AuthError::new(
            400,
            "bad_json",
            format!(
                "Could not parse request body as JSON: {}",
                go_json(&err, &body)
            ),
        )
    })?;
    let created = create_custom_provider(&state, &params).await?;
    Ok(json_status(StatusCode::CREATED, &created))
}

// megabase:unit auth:route:POST /auth/v1/admin/generate_link
pub(crate) async fn generate_link(
    State(state): State<AuthState>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
    body: Bytes,
) -> Result<Response, GenerateLinkError> {
    let role = verified_admin_role(&state, &headers)?;
    limit_body(&body)?;
    let mut params: LinkParams = serde_json::from_slice(&body).map_err(|err| {
        AuthError::new(
            400,
            "bad_json",
            format!(
                "Could not parse request body as JSON: {}",
                go_json(&err, &body)
            ),
        )
    })?;
    params.email = validate_admin_email(&params.email)?;
    let query = query_map(uri.query());
    let query_redirect = first(&query, "redirect_to").unwrap_or("");
    let redirect = link_referrer(&state, &headers, query_redirect, &params.redirect_to)?;
    let aud = request_aud(&headers, &state.config);
    let db = pool(&state)?;
    let mut conn = db
        .acquire()
        .await
        .map_err(|_| AuthError::internal("Database error finding user"))?;
    let existing = find_user_by_email(&mut conn, &params.email, &aud).await?;
    drop(conn);
    let mut link_type = params.link_type.clone();
    let user_id = existing.as_ref().map(|row| row.id);
    if existing.is_none() {
        match link_type.as_str() {
            "magiclink" => {
                link_type = "signup".into();
                params.password = generate_password()?;
            }
            "recovery" | "email_change_current" | "email_change_new" => {
                return Err(AuthError::not_found(
                    "user_not_found",
                    "User with this email not found",
                )
                .into());
            }
            _ => {}
        }
    }
    if link_type == "signup" && user_id.is_none() {
        check_signup_password(&state, &params.password)?;
    }
    let otp = generate_otp(state.otp_length)?;
    let hashed = token_hash(&params.email, &otp);
    let mut tx = db
        .begin()
        .await
        .map_err(|_| AuthError::internal("Database error finding user"))?;
    let verification = apply_link(
        &mut tx,
        LinkApply {
            state: &state,
            admin_role: &role,
            params: &params,
            link_type: &link_type,
            aud: &aud,
            user_id,
            otp: &otp,
            hashed: &hashed,
        },
    )
    .await?;
    tx.commit()
        .await
        .map_err(|_| AuthError::internal("Database error finding user"))?;
    let mut conn = db
        .acquire()
        .await
        .map_err(|_| AuthError::internal("Database error loading user"))?;
    let mut user = load_user_json(&mut conn, verification.user_id).await?;
    let action = action_link(
        &state,
        &verification.token_type,
        &verification.token,
        &redirect,
    );
    if let Value::Object(map) = &mut user {
        map.insert("action_link".into(), json!(action));
        map.insert("email_otp".into(), json!(otp));
        map.insert("hashed_token".into(), json!(verification.hashed_token));
        map.insert(
            "verification_type".into(),
            json!(verification.verification_type),
        );
        map.insert("redirect_to".into(), json!(redirect));
    }
    Ok(json_ok(&user))
}

#[derive(Debug)]
struct LinkResult {
    user_id: Uuid,
    token: String,
    token_type: String,
    hashed_token: String,
    verification_type: String,
}

struct LinkApply<'a> {
    state: &'a AuthState,
    admin_role: &'a str,
    params: &'a LinkParams,
    link_type: &'a str,
    aud: &'a str,
    user_id: Option<Uuid>,
    otp: &'a str,
    hashed: &'a str,
}

async fn apply_link(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    apply: LinkApply<'_>,
) -> Result<LinkResult, AuthError> {
    let LinkApply {
        state,
        admin_role,
        params,
        link_type,
        aud,
        user_id,
        otp,
        hashed,
    } = apply;
    let now = Utc::now();
    match link_type {
        "magiclink" | "recovery" => {
            let id = user_id.ok_or_else(|| {
                AuthError::not_found("user_not_found", "User with this email not found")
            })?;
            let row = load_user_row(tx, id).await?;
            write_audit(
                tx,
                &row.id.to_string(),
                false,
                row.email.as_deref().unwrap_or(""),
                "user_recovery_requested",
                "user",
                None,
            )
            .await?;
            sqlx::query(
                "UPDATE auth.users SET recovery_token = $2, recovery_sent_at = $3, updated_at = $3 WHERE id = $1",
            )
            .bind(id)
            .bind(hashed)
            .bind(now)
            .execute(&mut **tx)
            .await
            .map_err(|_| AuthError::internal("Database error updating user for recovery"))?;
            insert_ott(tx, id, &params.email, hashed, "recovery_token").await?;
            Ok(LinkResult {
                user_id: id,
                token: hashed.to_string(),
                token_type: link_type.to_string(),
                hashed_token: hashed.to_string(),
                verification_type: link_type.to_string(),
            })
        }
        "invite" => {
            let id = match user_id {
                Some(id) => {
                    let row = load_user_row(tx, id).await?;
                    if row.email_confirmed_at.is_some() {
                        return Err(AuthError::unprocessable("email_exists", DUPLICATE_EMAIL));
                    }
                    id
                }
                None => create_unconfirmed_user(tx, aud, &params.email, "", &params.data).await?,
            };
            write_audit(
                tx,
                "00000000-0000-0000-0000-000000000000",
                false,
                admin_role,
                "user_invited",
                "team",
                Some(json!({
                    "user_id": id.to_string(),
                    "user_email": params.email,
                })),
            )
            .await?;
            sqlx::query(
                "UPDATE auth.users SET confirmation_token = $2, confirmation_sent_at = $3, invited_at = $3, updated_at = $3 WHERE id = $1",
            )
            .bind(id)
            .bind(hashed)
            .bind(now)
            .execute(&mut **tx)
            .await
            .map_err(|_| AuthError::internal("Database error updating user for invite"))?;
            insert_ott(tx, id, &params.email, hashed, "confirmation_token").await?;
            Ok(LinkResult {
                user_id: id,
                token: hashed.to_string(),
                token_type: "invite".into(),
                hashed_token: hashed.to_string(),
                verification_type: "invite".into(),
            })
        }
        "signup" => {
            let id = match user_id {
                Some(id) => {
                    let row = load_user_row(tx, id).await?;
                    if row.email_confirmed_at.is_some() {
                        return Err(AuthError::unprocessable("email_exists", DUPLICATE_EMAIL));
                    }
                    if let Some(data) = &params.data {
                        let merged = merge_meta(row.raw_user_meta_data.as_ref(), data);
                        sqlx::query(
                            "UPDATE auth.users SET raw_user_meta_data = $2, updated_at = $3 WHERE id = $1",
                        )
                        .bind(id)
                        .bind(merged)
                        .bind(now)
                        .execute(&mut **tx)
                        .await
                        .map_err(|_| AuthError::internal("Database error updating user"))?;
                    }
                    id
                }
                None => {
                    let hash = hash_password(&params.password).await?;
                    create_unconfirmed_user(tx, aud, &params.email, &hash, &params.data).await?
                }
            };
            sqlx::query(
                "UPDATE auth.users SET confirmation_token = $2, confirmation_sent_at = $3, updated_at = $3 WHERE id = $1",
            )
            .bind(id)
            .bind(hashed)
            .bind(now)
            .execute(&mut **tx)
            .await
            .map_err(|_| AuthError::internal("Database error updating user for confirmation"))?;
            insert_ott(tx, id, &params.email, hashed, "confirmation_token").await?;
            Ok(LinkResult {
                user_id: id,
                token: hashed.to_string(),
                token_type: "signup".into(),
                hashed_token: hashed.to_string(),
                verification_type: "signup".into(),
            })
        }
        "email_change_current" | "email_change_new" => {
            if !state.secure_email_change && link_type == "email_change_current" {
                return Err(AuthError::validation(
                    400,
                    "Enable secure email change to generate link for current email",
                ));
            }
            let id = user_id.ok_or_else(|| {
                AuthError::not_found("user_not_found", "User with this email not found")
            })?;
            let new_email = validate_admin_email(&params.new_email)?;
            if email_taken(tx, &new_email, aud, id).await? {
                return Err(AuthError::unprocessable("email_exists", DUPLICATE_EMAIL));
            }
            let current_hash = if link_type == "email_change_current" {
                hashed.to_string()
            } else {
                String::new()
            };
            let new_hash = if link_type == "email_change_new" {
                token_hash(&new_email, otp)
            } else {
                String::new()
            };
            sqlx::query(
                "UPDATE auth.users SET email_change = $2, email_change_sent_at = $3,
                        email_change_confirm_status = 0, email_change_token_current = $4,
                        email_change_token_new = $5, updated_at = $3
                 WHERE id = $1",
            )
            .bind(id)
            .bind(&new_email)
            .bind(now)
            .bind(&current_hash)
            .bind(&new_hash)
            .execute(&mut **tx)
            .await
            .map_err(|_| AuthError::internal("Database error updating user for email change"))?;
            if !current_hash.is_empty() {
                insert_ott(
                    tx,
                    id,
                    &params.email,
                    &current_hash,
                    "email_change_token_current",
                )
                .await?;
            }
            if !new_hash.is_empty() {
                insert_ott(tx, id, &new_email, &new_hash, "email_change_token_new").await?;
            }
            let token = if link_type == "email_change_new" {
                new_hash
            } else {
                current_hash
            };
            Ok(LinkResult {
                user_id: id,
                token,
                token_type: link_type.to_string(),
                hashed_token: hashed.to_string(),
                verification_type: link_type.to_string(),
            })
        }
        other => Err(AuthError::validation(
            400,
            format!("Invalid email action link type requested: {other}"),
        )),
    }
}

fn action_link(state: &AuthState, link_type: &str, token: &str, redirect: &str) -> String {
    let path = match link_type {
        "invite" => state.mailer_invite_path.as_str(),
        "signup" => state.mailer_confirmation_path.as_str(),
        "email_change_current" | "email_change_new" => state.mailer_email_change_path.as_str(),
        _ => state.mailer_recovery_path.as_str(),
    };
    let wire_type = match link_type {
        "email_change_current" | "email_change_new" => "email_change",
        other => other,
    };
    let query = format!(
        "token={}&type={}&redirect_to={}",
        query_escape(token),
        query_escape(wire_type),
        encode_redirect(redirect)
    );
    let base = state.api_external_url.trim_end_matches('/');
    let path = if path.starts_with('/') {
        // Absolute path replaces the base path (`url.URL.ResolveReference`).
        let origin = base
            .split("//")
            .nth(1)
            .and_then(|rest| rest.split('/').next())
            .unwrap_or("localhost:8000");
        let scheme = base.split("://").next().unwrap_or("http");
        format!("{scheme}://{origin}{path}?{query}")
    } else {
        format!("{base}/{path}?{query}")
    };
    path
}

async fn create_unconfirmed_user(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    aud: &str,
    email: &str,
    password_hash: &str,
    data: &Option<Value>,
) -> Result<Uuid, AuthError> {
    let id = Uuid::new_v4();
    let now = Utc::now();
    let meta = match data {
        Some(Value::Object(map)) => Value::Object(map.clone()),
        _ => json!({}),
    };
    let app = json!({ "provider": "email", "providers": ["email"] });
    let role = "authenticated";
    sqlx::query(
        "INSERT INTO auth.users (
            instance_id, id, aud, role, email, encrypted_password,
            confirmation_token, recovery_token, email_change_token_new, email_change,
            email_change_token_current, email_change_confirm_status,
            phone_change, phone_change_token, reauthentication_token,
            raw_app_meta_data, raw_user_meta_data,
            is_sso_user, is_anonymous, created_at, updated_at
        ) VALUES (
            $1, $2, $3, $4, $5, $6,
            '', '', '', '',
            '', 0,
            '', '', '',
            $7, $8,
            false, false, $9, $9
        )",
    )
    .bind(nil_instance())
    .bind(id)
    .bind(aud)
    .bind(role)
    .bind(email)
    .bind(password_hash)
    .bind(&app)
    .bind(&meta)
    .bind(now)
    .execute(&mut **tx)
    .await
    .map_err(|_| AuthError::internal("Database error saving new user"))?;
    let mut claims = Map::new();
    claims.insert("sub".into(), json!(id.to_string()));
    claims.insert("email".into(), json!(email));
    claims.insert("email_verified".into(), json!(false));
    claims.insert("phone_verified".into(), json!(false));
    if let Some(Value::Object(extra)) = data {
        for (key, value) in extra {
            claims.entry(key.clone()).or_insert_with(|| value.clone());
        }
    }
    sqlx::query(
        "INSERT INTO auth.identities (id, provider_id, user_id, identity_data, provider, created_at, updated_at)
         VALUES ($1, $2, $3, $4, 'email', $5, $5)",
    )
    .bind(Uuid::new_v4())
    .bind(id.to_string())
    .bind(id)
    .bind(Value::Object(claims))
    .bind(now)
    .execute(&mut **tx)
    .await
    .map_err(|_| AuthError::internal("Database error saving new user"))?;
    Ok(id)
}

async fn insert_ott(
    conn: &mut sqlx::PgConnection,
    user_id: Uuid,
    relates_to: &str,
    token_hash: &str,
    token_type: &str,
) -> Result<(), AuthError> {
    sqlx::query("DELETE FROM auth.one_time_tokens WHERE user_id = $1 AND token_type = $2::auth.one_time_token_type")
        .bind(user_id)
        .bind(token_type)
        .execute(&mut *conn)
        .await
        .map_err(|_| AuthError::internal("Database error creating token in admin"))?;
    sqlx::query(
        "INSERT INTO auth.one_time_tokens (id, user_id, token_type, token_hash, relates_to)
         VALUES ($1, $2, $3::auth.one_time_token_type, $4, $5)",
    )
    .bind(Uuid::new_v4())
    .bind(user_id)
    .bind(token_type)
    .bind(token_hash)
    .bind(relates_to.to_lowercase())
    .execute(&mut *conn)
    .await
    .map_err(|_| AuthError::internal("Database error creating token in admin"))?;
    Ok(())
}

async fn write_audit(
    conn: &mut sqlx::PgConnection,
    actor_id: &str,
    via_sso: bool,
    username: &str,
    action: &str,
    log_type: &str,
    traits: Option<Value>,
) -> Result<(), AuthError> {
    let mut payload = Map::new();
    payload.insert("actor_id".into(), json!(actor_id));
    payload.insert("actor_via_sso".into(), json!(via_sso));
    payload.insert("actor_username".into(), json!(username));
    payload.insert("action".into(), json!(action));
    payload.insert("log_type".into(), json!(log_type));
    if let Some(traits) = traits {
        payload.insert("traits".into(), traits);
    }
    sqlx::query(
        "INSERT INTO auth.audit_log_entries (instance_id, id, payload, created_at, ip_address)
         VALUES ($1, $2, $3, NOW(), '')",
    )
    .bind(nil_instance())
    .bind(Uuid::new_v4())
    .bind(Value::Object(payload))
    .execute(&mut *conn)
    .await
    .map_err(|_| AuthError::internal("Error recording audit log entry"))?;
    Ok(())
}

async fn email_taken(
    conn: &mut sqlx::PgConnection,
    email: &str,
    aud: &str,
    user_id: Uuid,
) -> Result<bool, AuthError> {
    let found = sqlx::query_as::<_, (Uuid,)>(
        "SELECT id FROM auth.users
         WHERE instance_id = $1 AND LOWER(email) = $2 AND aud = $3 AND is_sso_user = false AND id <> $4
         LIMIT 1",
    )
    .bind(nil_instance())
    .bind(email)
    .bind(aud)
    .bind(user_id)
    .fetch_optional(&mut *conn)
    .await
    .map_err(|_| AuthError::internal("Database error checking email"))?;
    Ok(found.is_some())
}

fn check_signup_password(state: &AuthState, password: &str) -> Result<(), AuthError> {
    if password.is_empty() {
        return Err(AuthError::validation(
            400,
            "Signup requires a valid password",
        ));
    }
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

async fn hash_password(password: &str) -> Result<String, AuthError> {
    let password = password.to_string();
    tokio::task::spawn_blocking(move || bcrypt::hash(password, crate::config::BCRYPT_COST))
        .await
        .map_err(|_| AuthError::internal("Database error creating user"))?
        .map_err(|_| AuthError::internal("Database error creating user"))
}

fn generate_password() -> Result<String, AuthError> {
    const LOWER: &[u8] = b"abcdefghijklmnopqrstuvwxyz";
    const UPPER: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    const DIGITS: &[u8] = b"0123456789";
    const SYMBOLS: &[u8] = b"~!@#$%^&*()_+`-={}|[]\\:\"<>?,./";
    let bytes = random_bytes(64)?;
    let mut chars = Vec::with_capacity(64);
    for (i, byte) in bytes.iter().enumerate() {
        let alphabet = if i < 10 {
            DIGITS
        } else if i == 10 {
            SYMBOLS
        } else if i % 2 == 0 {
            UPPER
        } else {
            LOWER
        };
        chars.push(alphabet[(*byte as usize) % alphabet.len()] as char);
    }
    Ok(chars.into_iter().collect())
}

fn generate_otp(digits: usize) -> Result<String, AuthError> {
    let digits = digits.clamp(6, 10);
    let bytes = random_bytes(8)?;
    let mut value = 0u64;
    for byte in bytes {
        value = value.wrapping_mul(256).wrapping_add(u64::from(byte));
    }
    let upper = 10u64.pow(digits as u32);
    Ok(format!("{:0width$}", value % upper, width = digits))
}

pub(crate) fn token_hash(email: &str, otp: &str) -> String {
    let mut hasher = Sha224::new();
    hasher.update(email.as_bytes());
    hasher.update(otp.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn random_bytes(len: usize) -> Result<Vec<u8>, AuthError> {
    let mut buf = vec![0u8; len];
    let mut file = std::fs::File::open("/dev/urandom")
        .map_err(|_| AuthError::internal("failed to generate random bytes"))?;
    std::io::Read::read_exact(&mut file, &mut buf)
        .map_err(|_| AuthError::internal("failed to generate random bytes"))?;
    Ok(buf)
}

fn validate_admin_email(email: &str) -> Result<String, AuthError> {
    if email.is_empty() {
        return Err(AuthError::validation(400, "An email address is required"));
    }
    if email.len() > 255 {
        return Err(AuthError::validation(400, "An email address is too long"));
    }
    if !email_ok(email) {
        return Err(AuthError::validation(
            400,
            "Unable to validate email address: invalid format",
        ));
    }
    Ok(email.to_lowercase())
}

fn email_ok(email: &str) -> bool {
    let Some((local, domain)) = email.split_once('@') else {
        return false;
    };
    if local.is_empty() || domain.is_empty() || domain.starts_with('.') || domain.ends_with('.') {
        return false;
    }
    local
        .chars()
        .all(|ch| matches!(ch, 'a'..='z' | 'A'..='Z' | '0'..='9' | '.' | '!' | '#' | '$' | '%' | '&' | '\'' | '*' | '+' | '/' | '=' | '?' | '^' | '_' | '`' | '{' | '|' | '}' | '~' | '-'))
        && domain
            .chars()
            .all(|ch| matches!(ch, 'a'..='z' | 'A'..='Z' | '0'..='9' | '.' | '-'))
        && domain.contains('.')
}

/// `generate_link` fails with this when an allow-list pattern uses syntax this
/// build does not evaluate (`[`, `]`, `{`, `}`, or a dangling `\`).
#[derive(Debug)]
pub(crate) struct RedirectAllowListUnsupported;

pub(crate) enum GenerateLinkError {
    Auth(AuthError),
    NotImplemented(MegabaseNotImplemented),
}

impl From<AuthError> for GenerateLinkError {
    fn from(value: AuthError) -> Self {
        Self::Auth(value)
    }
}

impl From<RedirectAllowListUnsupported> for GenerateLinkError {
    fn from(_value: RedirectAllowListUnsupported) -> Self {
        Self::NotImplemented(MegabaseNotImplemented::new(
            crate::COMPONENT,
            "GOTRUE_URI_ALLOW_LIST",
        ))
    }
}

impl IntoResponse for GenerateLinkError {
    fn into_response(self) -> Response {
        match self {
            Self::Auth(err) => err.into_response(),
            Self::NotImplemented(err) => err.into_response(),
        }
    }
}

/// GoTrue `GetReferrer`, then `adminGenerateLink` overwrites with a valid body
/// `redirect_to`. A non-empty `redirect_to` header hides the query value even
/// when the header itself is not a valid redirect.
fn link_referrer(
    state: &AuthState,
    headers: &HeaderMap,
    query_redirect: &str,
    body_redirect: &str,
) -> Result<String, RedirectAllowListUnsupported> {
    let header_redirect = headers
        .get("redirect_to")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    let primary = if header_redirect.is_empty() {
        query_redirect
    } else {
        header_redirect
    };
    let referer = headers
        .get(axum::http::header::REFERER)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    let allow = state.uri_allow_list.as_slice();
    let mut referrer = if redirect_ok(&state.site_url, primary, allow)? {
        primary.to_string()
    } else if redirect_ok(&state.site_url, referer, allow)? {
        referer.to_string()
    } else {
        state.site_url.clone()
    };
    if redirect_ok(&state.site_url, body_redirect, allow)? {
        referrer = body_redirect.to_string();
    }
    Ok(referrer)
}

/// GoTrue `IsRedirectURLValid` (`internal/utilities/request.go`).
pub(crate) fn redirect_ok(
    site_url: &str,
    redirect: &str,
    allow: &[String],
) -> Result<bool, RedirectAllowListUnsupported> {
    if redirect.is_empty() {
        return Ok(false);
    }
    let Ok(base) = url_parts(site_url) else {
        return Ok(false);
    };
    let Ok(target) = url_parts(redirect) else {
        return Ok(false);
    };
    if base.host == target.host
        && base.scheme == target.scheme
        && (base.port == target.port || go_is_localhost(&target.host))
    {
        return Ok(true);
    }
    let host = target.host.as_str();
    if host.chars().all(|ch| ch.is_ascii_digit()) {
        return Ok(false);
    }
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        return Ok(ip.is_loopback());
    }
    if (target.scheme == "http" || target.scheme == "https") && !regular_hostname(host) {
        return Ok(false);
    }
    allow_list_matches(allow, redirect)
}

/// `url_validator.go` `isLocalhost`. Used only to skip the port check.
fn go_is_localhost(host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    matches!(
        host.as_str(),
        "localhost" | "127.0.0.1" | "::1" | "0.0.0.0" | "::"
    ) || host.ends_with(".localhost")
}

/// `regularHostname` in `request.go`: `^[a-zA-Z0-9]([a-zA-Z0-9.-]*[a-zA-Z0-9])?$`.
fn regular_hostname(host: &str) -> bool {
    let bytes = host.as_bytes();
    let Some((&first, rest)) = bytes.split_first() else {
        return false;
    };
    if !first.is_ascii_alphanumeric() {
        return false;
    }
    let Some((&last, middle)) = rest.split_last() else {
        return true;
    };
    last.is_ascii_alphanumeric()
        && middle
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'.' || *byte == b'-')
}

fn allow_list_matches(
    allow: &[String],
    redirect: &str,
) -> Result<bool, RedirectAllowListUnsupported> {
    let target = redirect.split_once('#').map_or(redirect, |(head, _)| head);
    let mut unsupported = false;
    for pattern in allow {
        match glob_match(pattern, target) {
            Ok(true) => return Ok(true),
            Ok(false) => {}
            Err(()) => unsupported = true,
        }
    }
    if unsupported {
        Err(RedirectAllowListUnsupported)
    } else {
        Ok(false)
    }
}

/// gobwas/glob with separators `.` and `/`, the separators GoTrue passes to
/// `glob.MustCompile`. `*` and `?` do not cross a separator. `**` does.
/// Character classes and `{` alternatives are not implemented.
fn glob_match(pattern: &str, text: &str) -> Result<bool, ()> {
    let pattern = pattern.as_bytes();
    let text = text.as_bytes();
    if pattern.len().saturating_mul(text.len().saturating_add(1)) > 250_000 {
        return Err(());
    }
    let width = text.len() + 1;
    let mut memo = vec![0u8; (pattern.len() + 1) * width];
    glob_at(pattern, 0, text, 0, &mut memo)
}

fn glob_at(pattern: &[u8], pi: usize, text: &[u8], ti: usize, memo: &mut [u8]) -> Result<bool, ()> {
    let width = text.len() + 1;
    let index = pi * width + ti;
    match memo[index] {
        1 => return Ok(true),
        2 => return Ok(false),
        3 => return Err(()),
        _ => {}
    }
    let result = glob_step(pattern, pi, text, ti, memo);
    memo[index] = match &result {
        Ok(true) => 1,
        Ok(false) => 2,
        Err(()) => 3,
    };
    result
}

fn glob_step(
    pattern: &[u8],
    mut pi: usize,
    text: &[u8],
    mut ti: usize,
    memo: &mut [u8],
) -> Result<bool, ()> {
    loop {
        if pi == pattern.len() {
            return Ok(ti == text.len());
        }
        match pattern[pi] {
            b'\\' => {
                if pi + 1 >= pattern.len() {
                    return Err(());
                }
                if ti >= text.len() || text[ti] != pattern[pi + 1] {
                    return Ok(false);
                }
                pi += 2;
                ti += 1;
            }
            b'[' | b']' | b'{' | b'}' => return Err(()),
            b'?' => {
                if ti >= text.len() || is_glob_sep(text[ti]) {
                    return Ok(false);
                }
                pi += 1;
                ti += 1;
            }
            b'*' if pi + 1 < pattern.len() && pattern[pi + 1] == b'*' => {
                return glob_star(pattern, pi + 2, text, ti, true, memo);
            }
            b'*' => return glob_star(pattern, pi + 1, text, ti, false, memo),
            literal => {
                if ti >= text.len() || text[ti] != literal {
                    return Ok(false);
                }
                pi += 1;
                ti += 1;
            }
        }
    }
}

fn glob_star(
    pattern: &[u8],
    rest: usize,
    text: &[u8],
    ti: usize,
    crosses_separator: bool,
    memo: &mut [u8],
) -> Result<bool, ()> {
    let mut end = ti;
    loop {
        if glob_at(pattern, rest, text, end, memo)? {
            return Ok(true);
        }
        if end == text.len() || (!crosses_separator && is_glob_sep(text[end])) {
            return Ok(false);
        }
        end += 1;
    }
}

fn is_glob_sep(byte: u8) -> bool {
    byte == b'.' || byte == b'/'
}

struct UrlParts {
    scheme: String,
    host: String,
    port: String,
}

// Go `net/url` `Hostname` and `Port`: the host is the authority after the last
// `@`. Only an all-digit port is split off. `\` and space in that host are
// rejected so they cannot hide the real authority.
fn url_parts(raw: &str) -> Result<UrlParts, ()> {
    let (scheme, rest) = raw.split_once("://").ok_or(())?;
    if scheme.is_empty() || rest.is_empty() {
        return Err(());
    }
    let hostport = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    let hostport = hostport.rsplit_once('@').map_or(hostport, |(_, host)| host);
    if hostport.is_empty() || hostport.chars().any(|ch| ch == '\\' || ch == ' ') {
        return Err(());
    }
    let (host, port) = split_host_port(hostport)?;
    if host.is_empty() {
        return Err(());
    }
    Ok(UrlParts {
        scheme: scheme.to_ascii_lowercase(),
        host,
        port,
    })
}

fn split_host_port(hostport: &str) -> Result<(String, String), ()> {
    if let Some(rest) = hostport.strip_prefix('[') {
        let (host, after) = rest.split_once(']').ok_or(())?;
        if host.is_empty() {
            return Err(());
        }
        let port = if let Some(port) = after.strip_prefix(':') {
            if port.is_empty() || !port.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(());
            }
            port
        } else if after.is_empty() {
            ""
        } else {
            return Err(());
        };
        return Ok((host.to_string(), port.to_string()));
    }
    if let Some((host, port)) = hostport.rsplit_once(':') {
        if !host.is_empty()
            && !host.contains(':')
            && !port.is_empty()
            && port.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Ok((host.to_string(), port.to_string()));
        }
    }
    Ok((hostport.to_string(), String::new()))
}

fn encode_redirect(redirect: &str) -> String {
    if redirect.chars().any(|ch| matches!(ch, '&' | '=' | '#')) {
        query_escape(redirect)
    } else {
        redirect.to_string()
    }
}

#[derive(Debug, Deserialize)]
struct LinkParams {
    #[serde(rename = "type", default)]
    link_type: String,
    #[serde(default)]
    email: String,
    #[serde(default)]
    new_email: String,
    #[serde(default)]
    password: String,
    #[serde(default)]
    data: Option<Value>,
    #[serde(default)]
    redirect_to: String,
}

fn merge_meta(existing: Option<&Value>, data: &Value) -> Value {
    let mut map = match existing {
        Some(Value::Object(map)) => map.clone(),
        _ => Map::new(),
    };
    if let Value::Object(extra) = data {
        for (key, value) in extra {
            map.insert(key.clone(), value.clone());
        }
    }
    Value::Object(map)
}

async fn find_user_by_email(
    conn: &mut sqlx::PgConnection,
    email: &str,
    aud: &str,
) -> Result<Option<AdminUserRow>, AuthError> {
    sqlx::query_as::<_, AdminUserRow>(&format!(
        "SELECT {USER_COLUMNS} FROM auth.users
         WHERE instance_id = $1 AND LOWER(email) = $2 AND aud = $3 AND is_sso_user = false"
    ))
    .bind(nil_instance())
    .bind(email)
    .bind(aud)
    .fetch_optional(&mut *conn)
    .await
    .map_err(|_| AuthError::internal("Database error finding user"))
}

async fn fetch_users(
    db: &sqlx::PgPool,
    aud: &str,
    filter: Option<&str>,
    order: &str,
    page: u64,
    per_page: u64,
) -> Result<Vec<AdminUserRow>, AuthError> {
    let offset = page.saturating_sub(1).saturating_mul(per_page) as i64;
    let limit = per_page as i64;
    let sql = match filter {
        Some(_) => format!(
            "SELECT {USER_COLUMNS} FROM auth.users
             WHERE instance_id = $1 AND aud = $2
               AND (email LIKE $3 OR raw_user_meta_data->>'full_name' ILIKE $3)
             ORDER BY {order} LIMIT $4 OFFSET $5"
        ),
        None => format!(
            "SELECT {USER_COLUMNS} FROM auth.users
             WHERE instance_id = $1 AND aud = $2
             ORDER BY {order} LIMIT $3 OFFSET $4"
        ),
    };
    let mut query = sqlx::query_as::<_, AdminUserRow>(&sql)
        .bind(nil_instance())
        .bind(aud);
    if let Some(filter) = filter {
        query = query.bind(format!("%{filter}%"));
    }
    query
        .bind(limit)
        .bind(offset)
        .fetch_all(db)
        .await
        .map_err(|_| AuthError::internal("Database error finding users"))
}

async fn count_users(db: &sqlx::PgPool, aud: &str, filter: Option<&str>) -> Result<i64, AuthError> {
    let sql = match filter {
        Some(_) => {
            "SELECT COUNT(*)::bigint AS count FROM auth.users
                    WHERE instance_id = $1 AND aud = $2
                      AND (email LIKE $3 OR raw_user_meta_data->>'full_name' ILIKE $3)"
        }
        None => {
            "SELECT COUNT(*)::bigint AS count FROM auth.users WHERE instance_id = $1 AND aud = $2"
        }
    };
    let mut query = sqlx::query_as::<_, CountRow>(sql)
        .bind(nil_instance())
        .bind(aud);
    if let Some(filter) = filter {
        query = query.bind(format!("%{filter}%"));
    }
    let row = query
        .fetch_one(db)
        .await
        .map_err(|_| AuthError::internal("Database error finding users"))?;
    Ok(row.count)
}

async fn list_users_keyset(
    state: &AuthState,
    uri: &axum::http::Uri,
    query: &BTreeMap<String, Vec<String>>,
    aud: &str,
    sort: &[&str],
) -> Result<Response, AuthError> {
    let keyset = parse_keyset(query)?;
    let dir = sort.first().copied().unwrap_or("DESC");
    let db = pool(state)?;
    let filter = first(query, "filter").filter(|value| !value.is_empty());
    let limit = i64::try_from(keyset.limit.saturating_add(1)).unwrap_or(i64::MAX);
    let sql = keyset_sql(dir, filter.is_some(), keyset.cursor.is_some());
    let mut q = sqlx::query_as::<_, AdminUserRow>(&sql)
        .bind(nil_instance())
        .bind(aud);
    if let Some(filter) = filter {
        q = q.bind(format!("%{filter}%"));
    }
    if let Some(cursor) = &keyset.cursor {
        q = q.bind(cursor.created_at).bind(cursor.id);
    }
    let mut rows = q
        .bind(limit)
        .fetch_all(&db)
        .await
        .map_err(|_| AuthError::internal("Database error finding users"))?;
    let has_more = rows.len() > keyset.limit as usize;
    if has_more {
        rows.truncate(keyset.limit as usize);
    }
    let next = if has_more {
        rows.last().map(|row| encode_cursor(row.created_at, row.id))
    } else {
        None
    };
    let users = rows
        .iter()
        .map(|row| user_json(row, None, &[]))
        .collect::<Vec<_>>();
    let mut pagination = Map::new();
    pagination.insert("has_more".into(), json!(has_more));
    if let Some(next) = &next {
        pagination.insert("next_cursor".into(), json!(next));
    }
    let mut response = json_ok(&json!({
        "users": users,
        "aud": aud,
        "pagination": pagination,
    }));
    if let Some(next) = next {
        let mut pairs = query.clone();
        pairs.remove("page");
        pairs.insert("cursor".into(), vec![next]);
        let path = uri.path().strip_prefix(AUTH_PREFIX).unwrap_or(uri.path());
        let header = format!("<{path}?{}>; rel=\"next\"", go_encode(&pairs));
        if let Ok(value) = HeaderValue::from_str(&header) {
            response.headers_mut().insert("link", value);
        }
    }
    Ok(response)
}

fn keyset_sql(dir: &str, filter: bool, cursor: bool) -> String {
    let cmp = if dir == "ASC" { ">" } else { "<" };
    let mut sql =
        format!("SELECT {USER_COLUMNS} FROM auth.users WHERE instance_id = $1 AND aud = $2");
    let mut slot = 3;
    if filter {
        sql.push_str(&format!(
            " AND (email LIKE ${slot} OR raw_user_meta_data->>'full_name' ILIKE ${slot})"
        ));
        slot += 1;
    }
    if cursor {
        sql.push_str(&format!(
            " AND (created_at, id) {cmp} (${slot}, ${next})",
            next = slot + 1
        ));
        slot += 2;
    }
    sql.push_str(&format!(
        " ORDER BY created_at {dir}, id {dir} LIMIT ${slot}"
    ));
    sql
}

struct Keyset {
    limit: u64,
    cursor: Option<DecodedCursor>,
}

struct DecodedCursor {
    created_at: DateTime<Utc>,
    id: Uuid,
}

fn parse_keyset(query: &BTreeMap<String, Vec<String>>) -> Result<Keyset, AuthError> {
    let mut limit = 50u64;
    if let Some(raw) = first(query, "limit").filter(|value| !value.is_empty()) {
        limit = raw.parse::<u64>().map_err(|_| {
            AuthError::validation(
                400,
                format!("Bad Pagination Parameters: strconv.ParseUint: parsing \"{raw}\": invalid syntax"),
            )
        })?;
        if limit == 0 {
            return Err(AuthError::validation(
                400,
                "Bad Pagination Parameters: limit must be greater than 0",
            ));
        }
    }
    if limit > 1000 {
        limit = 1000;
    }
    let cursor = match first(query, "cursor").filter(|value| !value.is_empty()) {
        None => None,
        Some(raw) => Some(decode_cursor(raw).map_err(|err| {
            AuthError::validation(400, format!("Bad Pagination Parameters: {err}"))
        })?),
    };
    Ok(Keyset { limit, cursor })
}

fn decode_cursor(raw: &str) -> Result<DecodedCursor, String> {
    let bytes = URL_SAFE_NO_PAD
        .decode(raw)
        .map_err(|_| "invalid cursor encoding".to_string())?;
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| "invalid cursor".to_string())?;
    let id = value
        .get("id")
        .and_then(Value::as_str)
        .and_then(|id| Uuid::parse_str(id).ok())
        .filter(|id| !id.is_nil())
        .ok_or_else(|| "invalid cursor: missing id".to_string())?;
    let created_at = value
        .get("created_at")
        .and_then(Value::as_str)
        .and_then(|raw| DateTime::parse_from_rfc3339(raw).ok())
        .map(|time| time.with_timezone(&Utc))
        .ok_or_else(|| "invalid cursor: missing created_at".to_string())?;
    Ok(DecodedCursor { created_at, id })
}

fn encode_cursor(created_at: Option<DateTime<Utc>>, id: Uuid) -> String {
    let stamp = created_at
        .map(|time| system_time_rfc3339(SystemTime::from(time)))
        .unwrap_or_else(|| "0001-01-01T00:00:00Z".into());
    let raw = serde_json::to_vec(&json!({ "created_at": stamp, "id": id.to_string() }))
        .unwrap_or_default();
    URL_SAFE_NO_PAD.encode(raw)
}

fn use_cursor(state: &AuthState, query: &BTreeMap<String, Vec<String>>) -> bool {
    state.cursor_pagination
        && first(query, "page").unwrap_or("").is_empty()
        && first(query, "per_page").unwrap_or("").is_empty()
}

fn write_offset_page(
    response: &mut Response,
    uri: &axum::http::Uri,
    page: u64,
    per_page: u64,
    total: i64,
) {
    let total = u64::try_from(total).unwrap_or(0);
    let total_pages = match per_page {
        0 => 0,
        _ => total / per_page + u64::from(total % per_page > 0),
    };
    let mut pairs = query_map(uri.query());
    let path = uri.path().strip_prefix(AUTH_PREFIX).unwrap_or(uri.path());
    let mut header = String::new();
    if total_pages > page {
        pairs.insert("page".into(), vec![(page + 1).to_string()]);
        header.push_str(&format!("<{path}?{}>; rel=\"next\", ", go_encode(&pairs)));
    }
    pairs.insert("page".into(), vec![total_pages.to_string()]);
    header.push_str(&format!("<{path}?{}>; rel=\"last\"", go_encode(&pairs)));
    if let Ok(value) = HeaderValue::from_str(&header) {
        response.headers_mut().insert("link", value);
    }
    if let Ok(value) = HeaderValue::from_str(&total.to_string()) {
        response.headers_mut().insert("x-total-count", value);
    }
}

const USER_COLUMNS: &str =
    "id, aud, role, email, phone, phone_confirmed_at, email_confirmed_at, invited_at, \
confirmation_sent_at, confirmed_at, recovery_sent_at, email_change, email_change_sent_at, \
phone_change, phone_change_sent_at, reauthentication_sent_at, last_sign_in_at, \
raw_app_meta_data, raw_user_meta_data, created_at, updated_at, banned_until, deleted_at, \
is_anonymous";

#[derive(Debug, sqlx::FromRow)]
struct AdminUserRow {
    id: Uuid,
    aud: Option<String>,
    role: Option<String>,
    email: Option<String>,
    phone: Option<String>,
    phone_confirmed_at: Option<DateTime<Utc>>,
    email_confirmed_at: Option<DateTime<Utc>>,
    invited_at: Option<DateTime<Utc>>,
    confirmation_sent_at: Option<DateTime<Utc>>,
    confirmed_at: Option<DateTime<Utc>>,
    recovery_sent_at: Option<DateTime<Utc>>,
    email_change: Option<String>,
    email_change_sent_at: Option<DateTime<Utc>>,
    phone_change: Option<String>,
    phone_change_sent_at: Option<DateTime<Utc>>,
    reauthentication_sent_at: Option<DateTime<Utc>>,
    last_sign_in_at: Option<DateTime<Utc>>,
    raw_app_meta_data: Option<Value>,
    raw_user_meta_data: Option<Value>,
    created_at: Option<DateTime<Utc>>,
    updated_at: Option<DateTime<Utc>>,
    banned_until: Option<DateTime<Utc>>,
    deleted_at: Option<DateTime<Utc>>,
    is_anonymous: bool,
}

#[derive(Debug, sqlx::FromRow)]
struct CountRow {
    count: i64,
}

#[derive(Debug, sqlx::FromRow)]
struct IdentityRow {
    id: Uuid,
    provider_id: String,
    user_id: Uuid,
    identity_data: Value,
    provider: String,
    last_sign_in_at: Option<DateTime<Utc>>,
    created_at: Option<DateTime<Utc>>,
    updated_at: Option<DateTime<Utc>>,
    email: Option<String>,
}

#[derive(Debug, sqlx::FromRow)]
struct FactorListRow {
    id: Uuid,
    friendly_name: Option<String>,
    factor_type: String,
    status: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    phone: Option<String>,
    last_challenged_at: Option<DateTime<Utc>>,
    web_authn_aaguid: Option<Uuid>,
    last_webauthn_challenge_data: Option<Value>,
}

#[derive(Debug, sqlx::FromRow)]
struct PasskeyRow {
    id: Uuid,
    friendly_name: String,
    created_at: DateTime<Utc>,
    last_used_at: Option<DateTime<Utc>>,
}

#[derive(Debug, sqlx::FromRow)]
struct SsoRow {
    id: Uuid,
    resource_id: Option<String>,
    disabled: Option<bool>,
    created_at: Option<DateTime<Utc>>,
    updated_at: Option<DateTime<Utc>>,
}

fn user_json(row: &AdminUserRow, identities: Option<&[Value]>, factors: &[Value]) -> Value {
    let mut object = Map::new();
    object.insert("id".into(), json!(row.id.to_string()));
    object.insert("aud".into(), json!(row.aud.clone().unwrap_or_default()));
    object.insert("role".into(), json!(row.role.clone().unwrap_or_default()));
    object.insert("email".into(), json!(row.email.clone().unwrap_or_default()));
    insert_opt_time(&mut object, "email_confirmed_at", row.email_confirmed_at);
    insert_opt_time(&mut object, "invited_at", row.invited_at);
    object.insert("phone".into(), json!(row.phone.clone().unwrap_or_default()));
    insert_opt_time(&mut object, "phone_confirmed_at", row.phone_confirmed_at);
    insert_opt_time(
        &mut object,
        "confirmation_sent_at",
        row.confirmation_sent_at,
    );
    insert_opt_time(&mut object, "confirmed_at", row.confirmed_at);
    insert_opt_time(&mut object, "recovery_sent_at", row.recovery_sent_at);
    if let Some(email) = row.email_change.clone().filter(|value| !value.is_empty()) {
        object.insert("new_email".into(), json!(email));
    }
    insert_opt_time(
        &mut object,
        "email_change_sent_at",
        row.email_change_sent_at,
    );
    if let Some(phone) = row.phone_change.clone().filter(|value| !value.is_empty()) {
        object.insert("new_phone".into(), json!(phone));
    }
    insert_opt_time(
        &mut object,
        "phone_change_sent_at",
        row.phone_change_sent_at,
    );
    insert_opt_time(
        &mut object,
        "reauthentication_sent_at",
        row.reauthentication_sent_at,
    );
    insert_opt_time(&mut object, "last_sign_in_at", row.last_sign_in_at);
    object.insert(
        "app_metadata".into(),
        meta_or_empty(row.raw_app_meta_data.as_ref()),
    );
    object.insert(
        "user_metadata".into(),
        meta_or_empty(row.raw_user_meta_data.as_ref()),
    );
    if !factors.is_empty() {
        object.insert("factors".into(), json!(factors));
    }
    object.insert(
        "identities".into(),
        match identities {
            Some(items) => Value::Array(items.to_vec()),
            None => Value::Null,
        },
    );
    object.insert("created_at".into(), required_time(row.created_at));
    object.insert("updated_at".into(), required_time(row.updated_at));
    insert_opt_time(&mut object, "banned_until", row.banned_until);
    insert_opt_time(&mut object, "deleted_at", row.deleted_at);
    object.insert("is_anonymous".into(), json!(row.is_anonymous));
    Value::Object(object)
}

fn insert_opt_time(object: &mut Map<String, Value>, key: &str, time: Option<DateTime<Utc>>) {
    if let Some(time) = time {
        object.insert(key.into(), ts_json(Some(time)));
    }
}

fn required_time(time: Option<DateTime<Utc>>) -> Value {
    match time {
        Some(time) => ts_json(Some(time)),
        None => Value::String("0001-01-01T00:00:00Z".into()),
    }
}

fn meta_or_empty(value: Option<&Value>) -> Value {
    match value {
        Some(Value::Null) | None => json!({}),
        Some(other) => other.clone(),
    }
}

async fn load_user_row(conn: &mut sqlx::PgConnection, id: Uuid) -> Result<AdminUserRow, AuthError> {
    sqlx::query_as::<_, AdminUserRow>(&format!(
        "SELECT {USER_COLUMNS} FROM auth.users WHERE instance_id = $1 AND id = $2"
    ))
    .bind(nil_instance())
    .bind(id)
    .fetch_optional(&mut *conn)
    .await
    .map_err(|_| AuthError::internal("Database error loading user"))?
    .ok_or_else(|| AuthError::not_found("user_not_found", USER_NOT_FOUND))
}

async fn load_user_json(conn: &mut sqlx::PgConnection, id: Uuid) -> Result<Value, AuthError> {
    let row = load_user_row(conn, id).await?;
    let identities = load_identities(conn, id).await?;
    let factors = load_factors(conn, id).await?;
    Ok(user_json(&row, Some(&identities), &factors))
}

async fn load_identities(
    conn: &mut sqlx::PgConnection,
    user_id: Uuid,
) -> Result<Vec<Value>, AuthError> {
    let rows = sqlx::query_as::<_, IdentityRow>(
        "SELECT id, provider_id, user_id, identity_data, provider, last_sign_in_at, created_at, updated_at, email
         FROM auth.identities WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_all(&mut *conn)
    .await
    .map_err(|_| AuthError::internal("Database error loading user"))?;
    Ok(rows
        .iter()
        .map(|row| {
            let mut item = Map::new();
            item.insert("identity_id".into(), json!(row.id.to_string()));
            item.insert("id".into(), json!(row.provider_id));
            item.insert("user_id".into(), json!(row.user_id.to_string()));
            if !row.identity_data.is_null()
                && row
                    .identity_data
                    .as_object()
                    .is_none_or(|map| !map.is_empty())
            {
                item.insert("identity_data".into(), row.identity_data.clone());
            }
            item.insert("provider".into(), json!(row.provider));
            if let Some(time) = row.last_sign_in_at {
                item.insert("last_sign_in_at".into(), ts_json(Some(time)));
            }
            item.insert("created_at".into(), required_time(row.created_at));
            item.insert("updated_at".into(), required_time(row.updated_at));
            if let Some(email) = row.email.clone().filter(|email| !email.is_empty()) {
                item.insert("email".into(), json!(email));
            }
            Value::Object(item)
        })
        .collect())
}

async fn load_factors(
    conn: &mut sqlx::PgConnection,
    user_id: Uuid,
) -> Result<Vec<Value>, AuthError> {
    let rows = sqlx::query_as::<_, FactorListRow>(
        "SELECT id, friendly_name, factor_type::text AS factor_type, status::text AS status,
                created_at, updated_at, phone, last_challenged_at, web_authn_aaguid,
                last_webauthn_challenge_data
         FROM auth.mfa_factors WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_all(&mut *conn)
    .await
    .map_err(|_| AuthError::internal("Database error loading factor"))?;
    Ok(rows
        .iter()
        .map(|row| {
            let mut factor = Map::new();
            factor.insert("id".into(), json!(row.id.to_string()));
            factor.insert("created_at".into(), ts_json(Some(row.created_at)));
            factor.insert("updated_at".into(), ts_json(Some(row.updated_at)));
            factor.insert("status".into(), json!(row.status));
            if let Some(name) = row.friendly_name.clone().filter(|name| !name.is_empty()) {
                factor.insert("friendly_name".into(), json!(name));
            }
            factor.insert("factor_type".into(), json!(row.factor_type));
            factor.insert("phone".into(), json!(row.phone.clone().unwrap_or_default()));
            factor.insert("last_challenged_at".into(), ts_json(row.last_challenged_at));
            if let Some(aaguid) = row.web_authn_aaguid {
                factor.insert("web_authn_aaguid".into(), json!(aaguid.to_string()));
            }
            if let Some(data) = row.last_webauthn_challenge_data.clone() {
                if !data.is_null() {
                    factor.insert("last_webauthn_challenge_data".into(), data);
                }
            }
            Value::Object(factor)
        })
        .collect())
}

fn parse_user_id(user_id: &str) -> Result<Uuid, AuthError> {
    if !is_uuid(user_id) {
        return Err(AuthError::not_found(
            "validation_failed",
            "user_id must be an UUID",
        ));
    }
    Uuid::parse_str(user_id)
        .map_err(|_| AuthError::not_found("validation_failed", "user_id must be an UUID"))
}

async fn load_sso_rows(
    db: &sqlx::PgPool,
    resource_id: Option<&str>,
    prefix: Option<&str>,
) -> Result<Vec<SsoRow>, AuthError> {
    let (sql, arg): (&str, Option<&str>) = if let Some(id) = resource_id.filter(|id| !id.is_empty())
    {
        (
            "SELECT id, resource_id, disabled, created_at, updated_at FROM auth.sso_providers WHERE resource_id = $1",
            Some(id),
        )
    } else if let Some(prefix) = prefix.filter(|prefix| !prefix.is_empty()) {
        (
            "SELECT id, resource_id, disabled, created_at, updated_at FROM auth.sso_providers WHERE resource_id LIKE $1",
            Some(prefix),
        )
    } else {
        (
            "SELECT id, resource_id, disabled, created_at, updated_at FROM auth.sso_providers",
            None,
        )
    };
    let mut query = sqlx::query_as::<_, SsoRow>(sql);
    if let Some(arg) = arg {
        let pattern = if sql.contains("LIKE") {
            format!("{arg}%")
        } else {
            arg.to_string()
        };
        query = query.bind(pattern);
    }
    query
        .fetch_all(db)
        .await
        .map_err(|_| AuthError::internal("error loading all SAML SSO providers"))
}

async fn load_one_sso(conn: &mut sqlx::PgConnection, idp_id: &str) -> Result<SsoRow, AuthError> {
    let missing =
        || AuthError::not_found("sso_provider_not_found", "SSO Identity Provider not found");
    if let Some(resource_id) = idp_id.strip_prefix("resource_") {
        return sqlx::query_as::<_, SsoRow>(
            "SELECT id, resource_id, disabled, created_at, updated_at FROM auth.sso_providers WHERE resource_id = $1",
        )
        .bind(resource_id)
        .fetch_optional(&mut *conn)
        .await
        .map_err(|_| AuthError::internal("Database error finding SSO Identity Provider"))?
        .ok_or_else(missing);
    }
    if !is_uuid(idp_id) {
        return Err(missing());
    }
    let id = Uuid::parse_str(idp_id).map_err(|_| missing())?;
    sqlx::query_as::<_, SsoRow>(
        "SELECT id, resource_id, disabled, created_at, updated_at FROM auth.sso_providers WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&mut *conn)
    .await
    .map_err(|_| AuthError::internal("Database error finding SSO Identity Provider"))?
    .ok_or_else(missing)
}

async fn sso_json(
    conn: &mut sqlx::PgConnection,
    row: &SsoRow,
    include_xml: bool,
) -> Result<Value, AuthError> {
    let saml = sqlx::query_as::<_, SamlRow>(
        "SELECT entity_id, metadata_xml, metadata_url, attribute_mapping, name_id_format
         FROM auth.saml_providers WHERE sso_provider_id = $1",
    )
    .bind(row.id)
    .fetch_optional(&mut *conn)
    .await
    .map_err(|_| AuthError::internal("Database error finding SSO Identity Provider"))?;
    let domains = sqlx::query_as::<_, DomainRow>(
        "SELECT domain FROM auth.sso_domains WHERE sso_provider_id = $1",
    )
    .bind(row.id)
    .fetch_all(&mut *conn)
    .await
    .map_err(|_| AuthError::internal("Database error finding SSO Identity Provider"))?;
    let mut provider = Map::new();
    provider.insert("id".into(), json!(row.id.to_string()));
    if let Some(resource_id) = row.resource_id.clone().filter(|id| !id.is_empty()) {
        provider.insert("resource_id".into(), json!(resource_id));
    }
    provider.insert("disabled".into(), json!(row.disabled));
    if let Some(saml) = saml {
        let mut saml_obj = Map::new();
        saml_obj.insert("entity_id".into(), json!(saml.entity_id));
        if include_xml && !saml.metadata_xml.is_empty() {
            saml_obj.insert("metadata_xml".into(), json!(saml.metadata_xml));
        }
        if let Some(url) = saml.metadata_url.filter(|url| !url.is_empty()) {
            saml_obj.insert("metadata_url".into(), json!(url));
        }
        if let Some(mapping) = saml.attribute_mapping {
            if mapping.as_object().is_some_and(|map| !map.is_empty()) {
                saml_obj.insert("attribute_mapping".into(), mapping);
            }
        }
        if let Some(format) = saml.name_id_format.filter(|format| !format.is_empty()) {
            saml_obj.insert("name_id_format".into(), json!(format));
        }
        provider.insert("saml".into(), Value::Object(saml_obj));
    }
    provider.insert(
        "domains".into(),
        json!(domains
            .into_iter()
            .map(|domain| json!({ "domain": domain.domain }))
            .collect::<Vec<_>>()),
    );
    provider.insert("created_at".into(), required_time(row.created_at));
    provider.insert("updated_at".into(), required_time(row.updated_at));
    Ok(Value::Object(provider))
}

#[derive(Debug, sqlx::FromRow)]
struct SamlRow {
    entity_id: String,
    metadata_xml: String,
    metadata_url: Option<String>,
    attribute_mapping: Option<Value>,
    name_id_format: Option<String>,
}

#[derive(Debug, sqlx::FromRow)]
struct DomainRow {
    domain: String,
}

#[derive(Debug, Deserialize)]
struct OAuthRegister {
    #[serde(default)]
    redirect_uris: Vec<String>,
    #[serde(default)]
    client_type: String,
    #[serde(default)]
    token_endpoint_auth_method: String,
    #[serde(default)]
    grant_types: Vec<String>,
    #[serde(default)]
    client_name: String,
    #[serde(default)]
    client_uri: String,
    #[serde(default)]
    logo_uri: String,
}

async fn register_oauth_client(state: &AuthState, params: &OAuthRegister) -> Result<Value, String> {
    validate_oauth_register(params).map_err(|message| format!("400: {message}"))?;
    let grant_types = if params.grant_types.is_empty() {
        vec!["authorization_code".into(), "refresh_token".into()]
    } else {
        params.grant_types.clone()
    };
    let client_type = if !params.client_type.is_empty() {
        params.client_type.clone()
    } else if params.token_endpoint_auth_method == "none" {
        "public".into()
    } else {
        "confidential".into()
    };
    let auth_method = if !params.token_endpoint_auth_method.is_empty() {
        params.token_endpoint_auth_method.clone()
    } else if client_type == "public" {
        "none".into()
    } else {
        "client_secret_basic".into()
    };
    let id = Uuid::new_v4();
    let secret = if client_type == "confidential" {
        let raw = random_bytes(32).map_err(|err| err.message)?;
        Some(URL_SAFE_NO_PAD.encode(raw))
    } else {
        None
    };
    let hash = secret.as_deref().map(|secret| {
        let mut hasher = Sha256::new();
        hasher.update(secret.as_bytes());
        URL_SAFE_NO_PAD.encode(hasher.finalize())
    });
    let db =
        pool(state).map_err(|err| format!("failed to create OAuth client: {}", err.message))?;
    let row = sqlx::query_as::<_, OAuthClientRow>(
        "INSERT INTO auth.oauth_clients (
            id, client_secret_hash, registration_type, redirect_uris, grant_types,
            client_name, client_uri, logo_uri, client_type, token_endpoint_auth_method
         ) VALUES (
            $1, $2, 'manual'::auth.oauth_registration_type, $3, $4,
            $5, $6, $7, $8::auth.oauth_client_type, $9
         )
         RETURNING id, client_type::text AS client_type, redirect_uris, token_endpoint_auth_method,
                   grant_types, client_name, client_uri, logo_uri,
                   registration_type::text AS registration_type, created_at, updated_at",
    )
    .bind(id)
    .bind(hash)
    .bind(params.redirect_uris.join(","))
    .bind(grant_types.join(","))
    .bind(empty_as_none(&params.client_name))
    .bind(empty_as_none(&params.client_uri))
    .bind(empty_as_none(&params.logo_uri))
    .bind(&client_type)
    .bind(&auth_method)
    .fetch_one(&db)
    .await
    .map_err(|err| format!("failed to create OAuth client: {err}"))?;
    let mut body = oauth_client_json(&row);
    if let (Value::Object(map), Some(secret)) = (&mut body, secret) {
        map.insert("client_secret".into(), json!(secret));
    }
    Ok(body)
}

fn empty_as_none(value: &str) -> Option<&str> {
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

fn validate_oauth_register(params: &OAuthRegister) -> Result<(), String> {
    if params.redirect_uris.is_empty() {
        return Err("redirect_uris is required".into());
    }
    if params.redirect_uris.len() > 10 {
        return Err("redirect_uris cannot exceed 10 items".into());
    }
    for uri in &params.redirect_uris {
        if let Err(reason) = validate_redirect_uri(uri) {
            return Err(format!("invalid redirect_uri '{uri}': {reason}"));
        }
    }
    if !params.grant_types.is_empty()
        && params
            .grant_types
            .iter()
            .any(|grant| grant != "authorization_code" && grant != "refresh_token")
    {
        return Err(
            "grant_types must only contain 'authorization_code' and/or 'refresh_token'".into(),
        );
    }
    if params.client_name.len() > 1024 {
        return Err("client_name cannot exceed 1024 characters".into());
    }
    if !params.client_uri.is_empty() {
        if params.client_uri.len() > 2048 {
            return Err("client_uri cannot exceed 2048 characters".into());
        }
        if url_parts(&params.client_uri).is_err() {
            return Err("client_uri must be a valid URL".into());
        }
    }
    if !params.logo_uri.is_empty() {
        if params.logo_uri.len() > 2048 {
            return Err("logo_uri cannot exceed 2048 characters".into());
        }
        if url_parts(&params.logo_uri).is_err() {
            return Err("logo_uri must be a valid URL".into());
        }
    }
    if !params.client_type.is_empty()
        && params.client_type != "public"
        && params.client_type != "confidential"
    {
        return Err("client_type must be 'public' or 'confidential'".into());
    }
    if !params.token_endpoint_auth_method.is_empty()
        && !matches!(
            params.token_endpoint_auth_method.as_str(),
            "none" | "client_secret_basic" | "client_secret_post"
        )
    {
        return Err(
            "token_endpoint_auth_method must be one of: [none client_secret_basic client_secret_post]"
                .into(),
        );
    }
    if !params.client_type.is_empty() && !params.token_endpoint_auth_method.is_empty() {
        let expected = if params.token_endpoint_auth_method == "none" {
            "public"
        } else {
            "confidential"
        };
        if params.client_type != expected {
            return Err(format!(
                "client_type '{}' is inconsistent with token_endpoint_auth_method '{}' (expected client_type '{}')",
                params.client_type, params.token_endpoint_auth_method, expected
            ));
        }
    }
    Ok(())
}

fn validate_redirect_uri(uri: &str) -> Result<(), String> {
    if uri.is_empty() {
        return Err("redirect URI cannot be empty".into());
    }
    let parts = url_parts(uri).map_err(|_| "must have scheme and host".to_string())?;
    for scheme in ["javascript", "data", "file", "vbscript", "about", "blob"] {
        if parts.scheme.eq_ignore_ascii_case(scheme) {
            return Err(format!(
                "scheme '{scheme}' is not allowed for security reasons"
            ));
        }
    }
    if parts.scheme == "http" && !matches!(parts.host.as_str(), "localhost" | "127.0.0.1" | "::1") {
        return Err("HTTP scheme only allowed for localhost".into());
    }
    if uri
        .split_once('#')
        .is_some_and(|(_, fragment)| !fragment.is_empty())
    {
        return Err("fragment not allowed in redirect URI".into());
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
struct CustomParams {
    #[serde(default)]
    provider_type: String,
    #[serde(default)]
    identifier: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    client_id: String,
    #[serde(default)]
    client_secret: String,
    #[serde(default)]
    acceptable_client_ids: Option<Vec<String>>,
    #[serde(default)]
    scopes: Option<Vec<String>>,
    pkce_enabled: Option<bool>,
    #[serde(default)]
    attribute_mapping: Option<Map<String, Value>>,
    #[serde(default)]
    custom_claims_allowlist: Option<Vec<String>>,
    #[serde(default)]
    authorization_params: Option<Map<String, Value>>,
    enabled: Option<bool>,
    email_optional: Option<bool>,
    #[serde(default)]
    issuer: String,
    discovery_url: Option<String>,
    skip_nonce_check: Option<bool>,
    #[serde(default)]
    authorization_url: String,
    #[serde(default)]
    token_url: String,
    #[serde(default)]
    userinfo_url: String,
    jwks_uri: Option<String>,
}

async fn create_custom_provider(
    state: &AuthState,
    params: &CustomParams,
) -> Result<Value, AuthError> {
    if params.provider_type != "oauth2" && params.provider_type != "oidc" {
        return Err(AuthError::validation(
            400,
            "provider_type must be either 'oauth2' or 'oidc'",
        ));
    }
    validate_custom_params(params)?;
    validate_auth_params(params.authorization_params.as_ref())?;
    validate_attribute_mapping(params.attribute_mapping.as_ref())?;
    validate_claims(params.custom_claims_allowlist.as_deref())?;
    validate_custom_urls(params).await?;
    if params.provider_type == "oidc" {
        let url = discovery_url(params);
        // `skip_nonce_check` is stored only after a successful OIDC discovery fetch.
        let _ = params.skip_nonce_check;
        return Err(AuthError::validation(
            400,
            format!(
                "OIDC discovery from {} failed: OIDC discovery fetch is not available",
                go_quote(&url)
            ),
        ));
    }
    let db = pool(state)?;
    let existing = sqlx::query_as::<_, (Uuid,)>(
        "SELECT id FROM auth.custom_oauth_providers WHERE identifier = $1",
    )
    .bind(&params.identifier)
    .fetch_optional(&db)
    .await
    .map_err(|_| AuthError::internal("Error checking for existing provider"))?;
    if existing.is_some() {
        // v2.197.0 `adminCustomOAuthProviderCreate` uses `NewBadRequestError`
        // (HTTP 400, error code `conflict`), not `NewConflictError` (HTTP 409).
        return Err(AuthError::new(
            400,
            "conflict",
            "A custom OAuth provider with this identifier already exists",
        ));
    }
    let id = Uuid::new_v4();
    let scopes = params.scopes.clone();
    let acceptable = params.acceptable_client_ids.clone();
    let claims = params.custom_claims_allowlist.clone();
    let mapping = params
        .attribute_mapping
        .clone()
        .map(Value::Object)
        .unwrap_or_else(|| json!({}));
    let auth_params = params
        .authorization_params
        .clone()
        .map(Value::Object)
        .unwrap_or_else(|| json!({}));
    let row = sqlx::query_as::<_, CustomProviderRow>(
        "INSERT INTO auth.custom_oauth_providers (
            id, provider_type, identifier, name, client_id, client_secret,
            acceptable_client_ids, scopes, pkce_enabled, attribute_mapping,
            custom_claims_allowlist, authorization_params, enabled, email_optional,
            authorization_url, token_url, userinfo_url, jwks_uri
         ) VALUES (
            $1, $2, $3, $4, $5, $6,
            $7, $8, $9, $10,
            $11, $12, $13, $14,
            $15, $16, $17, $18
         )
         RETURNING id, provider_type, identifier, name, client_id,
                   acceptable_client_ids, scopes, pkce_enabled, attribute_mapping,
                   custom_claims_allowlist, authorization_params, enabled, email_optional,
                   issuer, discovery_url, skip_nonce_check, cached_discovery,
                   authorization_url, token_url, userinfo_url, jwks_uri,
                   created_at, updated_at",
    )
    .bind(id)
    .bind(&params.provider_type)
    .bind(&params.identifier)
    .bind(&params.name)
    .bind(&params.client_id)
    .bind(&params.client_secret)
    .bind(acceptable.unwrap_or_default())
    .bind(scopes.unwrap_or_default())
    .bind(params.pkce_enabled.unwrap_or(true))
    .bind(&mapping)
    .bind(claims.unwrap_or_default())
    .bind(&auth_params)
    .bind(params.enabled.unwrap_or(true))
    .bind(params.email_optional.unwrap_or(false))
    .bind(empty_as_none(&params.authorization_url))
    .bind(empty_as_none(&params.token_url))
    .bind(empty_as_none(&params.userinfo_url))
    .bind(params.jwks_uri.as_deref().filter(|uri| !uri.is_empty()))
    .fetch_one(&db)
    .await
    .map_err(|_| AuthError::internal("Error creating custom OAuth provider"))?;
    Ok(custom_provider_json(&row))
}

fn validate_custom_params(params: &CustomParams) -> Result<(), AuthError> {
    if params.identifier.is_empty() {
        return Err(AuthError::validation(400, "identifier is required"));
    }
    if !params.identifier.starts_with("custom:") {
        return Err(AuthError::validation(
            400,
            format!(
                "identifier must start with 'custom:' prefix, e.g. 'custom:{}'",
                params.identifier
            ),
        ));
    }
    if params.name.is_empty() {
        return Err(AuthError::validation(400, "name is required"));
    }
    if params.client_id.is_empty() {
        return Err(AuthError::validation(400, "client_id is required"));
    }
    if params.client_secret.is_empty() {
        return Err(AuthError::validation(400, "client_secret is required"));
    }
    if params.provider_type == "oidc" && params.issuer.is_empty() {
        return Err(AuthError::validation(
            400,
            "issuer is required for OIDC providers",
        ));
    }
    if params.provider_type == "oauth2" {
        if params.authorization_url.is_empty() {
            return Err(AuthError::validation(
                400,
                "authorization_url is required for OAuth2 providers",
            ));
        }
        if params.token_url.is_empty() {
            return Err(AuthError::validation(
                400,
                "token_url is required for OAuth2 providers",
            ));
        }
        if params.userinfo_url.is_empty() {
            return Err(AuthError::validation(
                400,
                "userinfo_url is required for OAuth2 providers",
            ));
        }
    }
    Ok(())
}

fn validate_auth_params(params: Option<&Map<String, Value>>) -> Result<(), AuthError> {
    let Some(params) = params else {
        return Ok(());
    };
    const RESERVED: &[&str] = &[
        "client_id",
        "client_secret",
        "redirect_uri",
        "response_type",
        "state",
        "code_challenge",
        "code_challenge_method",
        "code_verifier",
        "nonce",
    ];
    for (key, value) in params {
        if RESERVED.contains(&key.as_str()) {
            return Err(AuthError::validation(
                400,
                format!("Cannot override reserved OAuth parameter: {key}"),
            ));
        }
        if !value.is_string() {
            return Err(AuthError::validation(
                400,
                format!("Authorization parameter {} must be a string", go_quote(key)),
            ));
        }
    }
    Ok(())
}

fn validate_attribute_mapping(mapping: Option<&Map<String, Value>>) -> Result<(), AuthError> {
    let Some(mapping) = mapping else {
        return Ok(());
    };
    const BLOCKED: &[&str] = &[
        "id",
        "aud",
        "role",
        "app_metadata",
        "created_at",
        "updated_at",
        "confirmed_at",
        "email_confirmed_at",
        "phone_confirmed_at",
        "email_verified",
        "phone_verified",
        "banned_until",
        "is_super_admin",
    ];
    for key in mapping.keys() {
        if BLOCKED.contains(&key.as_str()) {
            return Err(AuthError::validation(
                400,
                format!("Cannot map to protected system field: {key}"),
            ));
        }
    }
    Ok(())
}

fn validate_claims(allowlist: Option<&[String]>) -> Result<(), AuthError> {
    if let Some(allowlist) = allowlist {
        for key in allowlist {
            if key.trim().is_empty() {
                return Err(AuthError::validation(
                    400,
                    "custom_claims_allowlist entries must be non-empty strings",
                ));
            }
        }
    }
    Ok(())
}

async fn validate_custom_urls(params: &CustomParams) -> Result<(), AuthError> {
    let mut urls = Vec::new();
    if params.provider_type == "oidc" {
        urls.push(params.issuer.as_str());
        if let Some(url) = params
            .discovery_url
            .as_deref()
            .filter(|url| !url.is_empty())
        {
            urls.push(url);
        }
    } else {
        urls.push(params.authorization_url.as_str());
        urls.push(params.token_url.as_str());
        urls.push(params.userinfo_url.as_str());
        if let Some(url) = params.jwks_uri.as_deref().filter(|url| !url.is_empty()) {
            urls.push(url);
        }
    }
    for url in urls {
        if !url.is_empty() {
            validate_oauth_url(url).await?;
        }
    }
    Ok(())
}

fn discovery_url(params: &CustomParams) -> String {
    if let Some(url) = params
        .discovery_url
        .as_deref()
        .filter(|url| !url.is_empty())
    {
        return url.to_string();
    }
    format!(
        "{}/.well-known/openid-configuration",
        params.issuer.trim_end_matches('/')
    )
}

pub(crate) async fn validate_oauth_url(url: &str) -> Result<(), AuthError> {
    let parts = url_parts(url).map_err(|_| AuthError::validation(400, "Invalid URL format"))?;
    if parts.scheme != "https" {
        return Err(AuthError::validation(400, "URL must use HTTPS"));
    }
    if parts.host.is_empty() {
        return Err(AuthError::validation(400, "URL must have a valid hostname"));
    }
    let host = parts.host.to_ascii_lowercase();
    if matches!(
        host.as_str(),
        "localhost" | "127.0.0.1" | "::1" | "0.0.0.0" | "::"
    ) || host.ends_with(".localhost")
    {
        return Err(AuthError::validation(
            400,
            "URL cannot point to localhost or loopback addresses",
        ));
    }
    // `lookup_host` is the non-blocking equivalent of `net.LookupIP`. Port 443
    // is only the required service argument; the check uses the resolved IPs.
    let addrs = tokio::time::timeout(
        OAUTH_DNS_TIMEOUT,
        tokio::net::lookup_host((host.as_str(), 443u16)),
    )
    .await
    .map_err(|_| AuthError::validation(400, "Unable to resolve hostname"))?
    .map_err(|_| AuthError::validation(400, "Unable to resolve hostname"))?;
    for addr in addrs {
        validate_ip(addr.ip())?;
    }
    Ok(())
}

fn validate_ip(ip: std::net::IpAddr) -> Result<(), AuthError> {
    // Go `net.IP.To4` unwraps IPv4-mapped IPv6 (`::ffff:a.b.c.d`) before
    // `IsLoopback`, `IsPrivate`, and `IsLinkLocalUnicast`. Rust's `IpAddr`
    // checks leave those mapped addresses on the IPv6 path.
    let ip = match ip {
        std::net::IpAddr::V6(addr) => addr
            .to_ipv4_mapped()
            .map_or(std::net::IpAddr::V6(addr), std::net::IpAddr::V4),
        addr => addr,
    };
    if ip.is_loopback() {
        return Err(AuthError::validation(
            400,
            "URL cannot resolve to loopback addresses",
        ));
    }
    let private = match ip {
        std::net::IpAddr::V4(ip) => ip.is_private(),
        std::net::IpAddr::V6(ip) => ip.is_unique_local(),
    };
    if private {
        return Err(AuthError::validation(
            400,
            "URL cannot resolve to private network addresses",
        ));
    }
    let link_local = match ip {
        std::net::IpAddr::V4(ip) => ip.is_link_local(),
        std::net::IpAddr::V6(ip) => ip.is_unicast_link_local(),
    };
    if link_local {
        return Err(AuthError::validation(
            400,
            "URL cannot resolve to link-local addresses",
        ));
    }
    if ip.is_multicast() {
        return Err(AuthError::validation(
            400,
            "URL cannot resolve to multicast addresses",
        ));
    }
    if ip.is_unspecified() {
        return Err(AuthError::validation(
            400,
            "URL cannot resolve to unspecified addresses",
        ));
    }
    Ok(())
}

fn limit_body(body: &Bytes) -> Result<(), AuthError> {
    if body.len() > MAX_BODY_BYTES {
        return Err(AuthError::new(
            413,
            "request_entity_too_large",
            format!("Request body too large (max {MAX_BODY_BYTES} bytes)"),
        ));
    }
    Ok(())
}

fn go_json(err: &serde_json::Error, body: &[u8]) -> String {
    if body.is_empty() || err.is_eof() {
        "unexpected end of JSON input".into()
    } else {
        err.to_string()
    }
}

fn sort_directions(query: &BTreeMap<String, Vec<String>>) -> Result<Vec<&'static str>, AuthError> {
    let Some(values) = query.get("sort").filter(|values| !values.is_empty()) else {
        return Ok(vec!["DESC"]);
    };
    let mut dirs = Vec::new();
    for value in values {
        let mut parts = value.splitn(2, ' ');
        let field = parts.next().unwrap_or("");
        if field != "created_at" {
            return Err(AuthError::validation(
                400,
                format!("Bad Sort Parameters: bad field for sort '{field}'"),
            ));
        }
        let dir = match parts.next() {
            None => "DESC",
            Some(raw) if raw.eq_ignore_ascii_case("asc") => "ASC",
            Some(raw) if raw.eq_ignore_ascii_case("desc") => "DESC",
            Some(raw) => {
                return Err(AuthError::validation(
                    400,
                    format!(
                        "Bad Sort Parameters: bad direction for sort '{raw}', only 'asc' and 'desc' allowed"
                    ),
                ));
            }
        };
        dirs.push(dir);
    }
    Ok(dirs)
}

fn query_map(raw: Option<&str>) -> BTreeMap<String, Vec<String>> {
    let mut map = BTreeMap::new();
    let Some(raw) = raw.filter(|value| !value.is_empty()) else {
        return map;
    };
    for pair in raw.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        map.entry(percent_decode(key))
            .or_default()
            .push(percent_decode(value));
    }
    map
}

fn first<'a>(query: &'a BTreeMap<String, Vec<String>>, key: &str) -> Option<&'a str> {
    query
        .get(key)
        .and_then(|values| values.first().map(String::as_str))
}

fn go_encode(map: &BTreeMap<String, Vec<String>>) -> String {
    let mut parts = Vec::new();
    for (key, values) in map {
        for value in values {
            parts.push(format!("{}={}", query_escape(key), query_escape(value)));
        }
    }
    parts.join("&")
}

fn query_escape(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                let hex = &value[i + 1..i + 3];
                if let Ok(byte) = u8::from_str_radix(hex, 16) {
                    out.push(byte);
                    i += 3;
                } else {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
            byte => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_hash_is_sha224() {
        assert_eq!(
            token_hash("user@example.com", "123456"),
            "b507a4ef007e1379d644aa6fe395cf98f1054ac96b63a052dca9a26a"
        );
    }

    #[test]
    fn sort_rejects_unknown_field() {
        let mut query = BTreeMap::new();
        query.insert("sort".into(), vec!["email desc".into()]);
        let err = sort_directions(&query).unwrap_err();
        assert_eq!(
            err.message,
            "Bad Sort Parameters: bad field for sort 'email'"
        );
    }

    #[test]
    fn oauth_register_wraps_missing_redirects() {
        let err = validate_oauth_register(&OAuthRegister {
            redirect_uris: vec![],
            client_type: String::new(),
            token_endpoint_auth_method: String::new(),
            grant_types: vec![],
            client_name: String::new(),
            client_uri: String::new(),
            logo_uri: String::new(),
        })
        .unwrap_err();
        assert_eq!(err, "redirect_uris is required");
    }

    #[tokio::test]
    async fn https_is_required_for_custom_urls() {
        let err = validate_oauth_url("http://example.com/auth")
            .await
            .unwrap_err();
        assert_eq!(err.message, "URL must use HTTPS");
    }

    #[test]
    fn url_parts_use_the_host_after_userinfo() {
        let loopback = url_parts("https://a.com:443@127.0.0.1/").unwrap();
        assert_eq!(loopback.scheme, "https");
        assert_eq!(loopback.host, "127.0.0.1");
        assert_eq!(loopback.port, "");

        let evil = url_parts("http://localhost:80@evil.com").unwrap();
        assert_eq!(evil.host, "evil.com");
        assert_eq!(evil.port, "");

        let evil_port = url_parts("http://localhost:1@evil.com/cb").unwrap();
        assert_eq!(evil_port.host, "evil.com");
        assert_eq!(evil_port.port, "");

        let v6 = url_parts("http://user@[::1]:8080/cb").unwrap();
        assert_eq!(v6.host, "::1");
        assert_eq!(v6.port, "8080");

        let disguised = url_parts("https://evil.com\\@127.0.0.1/").unwrap();
        assert_eq!(disguised.host, "127.0.0.1");
        assert!(url_parts(r"https://127.0.0.1\evil.com/").is_err());
        assert!(url_parts("https://exa mple.com/").is_err());
    }

    #[test]
    fn redirect_checks_ignore_userinfo() {
        let allow = Vec::<String>::new();
        assert!(!redirect_ok(
            "http://localhost:3000",
            "http://localhost:80@evil.com",
            &allow
        )
        .unwrap());
        assert!(!redirect_ok(
            "http://localhost:3000",
            "http://localhost:1@evil.com",
            &allow
        )
        .unwrap());
        assert!(redirect_ok("https://127.0.0.1", "https://a.com:443@127.0.0.1/", &allow).unwrap());
        assert!(!redirect_ok(
            "http://localhost:3000",
            r"http://localhost\evil.com/cb",
            &allow
        )
        .unwrap());
    }

    fn gotrue_allow_list() -> Vec<String> {
        [
            "http://localhost:8000/*",
            "http://*.localhost:8000/*",
            "http://*:12345/*",
            "http://**:12345/*",
        ]
        .into_iter()
        .map(str::to_string)
        .collect()
    }

    fn referrer_state(allow: Vec<String>) -> AuthState {
        let mut state = AuthState::from_lookup(|_| None);
        state.site_url = "https://example.com".into();
        state.uri_allow_list = allow;
        state
    }

    #[test]
    fn query_redirect_follows_gotrue_allow_list() {
        let state = referrer_state(gotrue_allow_list());
        let cases = [
            ("http://localhost:8000/path", "http://localhost:8000/path"),
            ("http://localhost:3000", "https://example.com"),
            ("http://localhost:8000", "https://example.com"),
            ("http://localhost:8000/path/to/page", "https://example.com"),
            (
                "http://localhost:8000/path?param=1",
                "http://localhost:8000/path?param=1",
            ),
            ("http://123?.localhost:8000/path", "https://example.com"),
            (
                "http://123.123.123.123?localhost:8000/path",
                "https://example.com",
            ),
            (
                "http://[65e7:9410:d8b6:e227:58cd:e55b:8fc0:206d]?localhost:8000/path",
                "https://example.com",
            ),
            (
                "http://65e7:9410:d8b6:e227:58cd:e55b:8fc0:206d?localhost:8000/path",
                "https://example.com",
            ),
            ("http://127.0.0.1:12345/path", "http://127.0.0.1:12345/path"),
            (
                "http://[0:0:0:0:0:0:0:1]:12345/path",
                "http://[0:0:0:0:0:0:0:1]:12345/path",
            ),
            (
                "https://example.com/dashboard",
                "https://example.com/dashboard",
            ),
            ("http://example.com/dashboard", "https://example.com"),
            ("https://example.com:8443/dashboard", "https://example.com"),
            (
                "http://foo.localhost:8000/path",
                "http://foo.localhost:8000/path",
            ),
            ("http://a.b.localhost:8000/path", "https://example.com"),
            ("http://a.b:12345/path", "http://a.b:12345/path"),
            ("http://foo:12345/path", "http://foo:12345/path"),
            (
                "http://localhost:8000/path#section",
                "http://localhost:8000/path#section",
            ),
        ];
        for (query_redirect, expected) in cases {
            let got = link_referrer(&state, &HeaderMap::new(), query_redirect, "").unwrap();
            assert_eq!(got, expected, "{query_redirect}");
        }
    }

    #[test]
    fn redirect_header_hides_query_and_body_can_win() {
        let state = referrer_state(gotrue_allow_list());
        let mut headers = HeaderMap::new();
        headers.insert(
            "redirect_to",
            HeaderValue::from_static("http://evil.example/nope"),
        );
        let hidden = link_referrer(&state, &headers, "http://localhost:8000/path", "").unwrap();
        assert_eq!(hidden, "https://example.com");

        headers.insert(
            axum::http::header::REFERER,
            HeaderValue::from_static("http://localhost:8000/ok"),
        );
        let from_referer =
            link_referrer(&state, &headers, "http://localhost:8000/path", "").unwrap();
        assert_eq!(from_referer, "http://localhost:8000/ok");

        let from_body = link_referrer(
            &state,
            &HeaderMap::new(),
            "http://localhost:8000/path",
            "https://example.com/from-body",
        )
        .unwrap();
        assert_eq!(from_body, "https://example.com/from-body");
    }

    #[test]
    fn redirect_localhost_port_and_ip_rules() {
        let allow = Vec::<String>::new();
        assert!(redirect_ok(
            "http://localhost:3000",
            "http://localhost:8080/callback",
            &allow
        )
        .unwrap());
        assert!(redirect_ok(
            "http://127.0.0.1:3000",
            "http://127.0.0.1:8080/callback",
            &allow
        )
        .unwrap());
        assert!(!redirect_ok(
            "https://localhost:3000",
            "http://localhost:8080/callback",
            &allow
        )
        .unwrap());
        assert!(redirect_ok("https://example.com", "http://127.0.0.2:9/cb", &allow).unwrap());
        assert!(redirect_ok("http://0.0.0.0:3000", "http://0.0.0.0:9/cb", &allow).unwrap());
        assert!(!redirect_ok("https://example.com", "http://0.0.0.0/cb", &allow).unwrap());
        assert!(!redirect_ok("http://localhost:3000", "http://LocalHost:9/cb", &allow).unwrap());
        assert!(redirect_ok(
            "http://app.localhost:3000",
            "http://app.localhost:9/cb",
            &allow
        )
        .unwrap());
        assert!(redirect_ok("https://example.com", "http://[::1]:12345/path", &allow).unwrap());
        let ip_glob = vec!["http://8.8.8.8/*".to_string()];
        assert!(!redirect_ok("https://example.com", "http://8.8.8.8/path", &ip_glob).unwrap());
        let digit = vec!["http://2130706433/*".to_string()];
        assert!(!redirect_ok("https://example.com", "http://2130706433/path", &digit).unwrap());
    }

    #[test]
    fn unsupported_allow_list_pattern_is_not_implemented() {
        let patterns = vec!["https://app.example/[id]/**".to_string()];
        let err =
            redirect_ok("https://example.com", "https://app.example/1", &patterns).unwrap_err();
        let response = GenerateLinkError::from(err).into_response();
        assert_eq!(response.status(), StatusCode::NOT_IMPLEMENTED);

        assert!(redirect_ok("https://example.com", "https://example.com/dash", &patterns).unwrap());

        let mixed = vec![
            "https://app.example/[id]/**".to_string(),
            "https://app.example/**".to_string(),
        ];
        assert!(redirect_ok("https://example.com", "https://app.example/ok", &mixed).unwrap());

        let custom = vec!["myapp://**".to_string()];
        assert!(redirect_ok("https://example.com", "myapp://callback", &custom).unwrap());
    }

    #[test]
    fn query_redirect_is_percent_decoded() {
        let state = referrer_state(gotrue_allow_list());
        let query = query_map(Some("redirect_to=http%3A%2F%2Flocalhost%3A8000%2Fpath+two"));
        let query_redirect = first(&query, "redirect_to").unwrap();
        let got = link_referrer(&state, &HeaderMap::new(), query_redirect, "").unwrap();
        assert_eq!(got, "http://localhost:8000/path two");
    }

    #[test]
    fn redirect_uri_uses_the_host_after_userinfo() {
        let evil = validate_redirect_uri("http://localhost:80@evil.com").unwrap_err();
        assert_eq!(evil, "HTTP scheme only allowed for localhost");
        let other = validate_redirect_uri("http://localhost:1@evil.com").unwrap_err();
        assert_eq!(other, "HTTP scheme only allowed for localhost");
        validate_redirect_uri("https://a.com:443@127.0.0.1/").unwrap();
        validate_redirect_uri("http://a.com:443@127.0.0.1/").unwrap();
        validate_redirect_uri("http://user@[::1]/cb").unwrap();
    }

    #[tokio::test]
    async fn oauth_url_rejects_userinfo_loopback_without_dns() {
        let err = validate_oauth_url("https://a.com:443@127.0.0.1/")
            .await
            .unwrap_err();
        assert_eq!(
            err.message,
            "URL cannot point to localhost or loopback addresses"
        );
        assert_eq!(err.status, 400);
        let spaced = validate_oauth_url("https://exa mple.com/")
            .await
            .unwrap_err();
        assert_eq!(spaced.message, "Invalid URL format");
        let link_local = validate_oauth_url("https://169.254.169.254/latest")
            .await
            .unwrap_err();
        assert_eq!(
            link_local.message,
            "URL cannot resolve to link-local addresses"
        );
        let mapped = validate_oauth_url("https://[::ffff:127.0.0.1]/")
            .await
            .unwrap_err();
        assert_eq!(mapped.message, "URL cannot resolve to loopback addresses");
        let mapped_meta = validate_oauth_url("https://[::ffff:169.254.169.254]/")
            .await
            .unwrap_err();
        assert_eq!(
            mapped_meta.message,
            "URL cannot resolve to link-local addresses"
        );
    }

    #[test]
    fn generated_otp_and_password_come_from_urandom() {
        let password = generate_password().unwrap();
        assert_eq!(password.len(), 64);
        let otp = generate_otp(6).unwrap();
        assert_eq!(otp.len(), 6);
        assert!(otp.chars().all(|ch| ch.is_ascii_digit()));
    }

    #[test]
    fn signup_password_rules() {
        let state = AuthState::from_lookup(|_| None);
        let empty = String::new();
        let missing = check_signup_password(&state, &empty).unwrap_err();
        assert_eq!(missing.message, "Signup requires a valid password");
        let short_value = "x".repeat(state.config.password_min_length - 1);
        let short = check_signup_password(&state, &short_value).unwrap_err();
        assert_eq!(short.error_code, "weak_password");
        assert_eq!(
            short.weak_password.as_deref(),
            Some(["length".to_string()].as_slice())
        );
        let long = "x".repeat(73);
        let too_long = check_signup_password(&state, &long).unwrap_err();
        assert_eq!(
            too_long.message,
            "Password cannot be longer than 72 characters"
        );
    }

    #[test]
    fn link_header_sorts_query_keys() {
        let mut pairs = BTreeMap::new();
        pairs.insert("per_page".into(), vec!["1".into()]);
        pairs.insert("page".into(), vec!["0".into()]);
        assert_eq!(go_encode(&pairs), "page=0&per_page=1");
    }
}
