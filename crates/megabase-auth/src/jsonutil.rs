// Ported from supabase/auth internal/models/user.go, identity.go, and
// internal/tokens/service.go AccessTokenResponse (MIT), pin v2.197.0.

//! JSON shapes for a session and the embedded user.

use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, SecondsFormat, Utc};
use serde::Serialize;
use serde_json::{json, Map, Value};
use uuid::Uuid;

use crate::store::{IdentityRecord, IssuedSession, UserRecord};

pub fn format_ts(time: SystemTime) -> String {
    DateTime::<Utc>::from(time).to_rfc3339_opts(SecondsFormat::AutoSi, true)
}

pub fn unix_secs(time: SystemTime) -> i64 {
    time.duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

pub fn session_json(
    issued: &IssuedSession,
    access_token: &str,
    expires_in: i64,
    now: i64,
) -> Value {
    json!({
        "access_token": access_token,
        "token_type": "bearer",
        "expires_in": expires_in,
        "expires_at": now + expires_in,
        "refresh_token": issued.refresh_token,
        "user": user_json(&issued.user),
    })
}

pub fn user_json(user: &UserRecord) -> Value {
    let email_confirmed_at = user.email_confirmed_at.map(format_ts);
    let body = UserJson {
        id: user.id.to_string(),
        aud: &user.aud,
        role: &user.role,
        email: &user.email,
        phone: &user.phone,
        email_confirmed_at,
        // `confirmed_at` is generated. Signup does not reload the row after
        // `Confirm`, so that response omits it. A later load includes it.
        confirmed_at: user.confirmed_at.map(format_ts),
        last_sign_in_at: user.last_sign_in_at.map(format_ts),
        app_metadata: &user.app_metadata,
        user_metadata: &user.user_metadata,
        identities: user.identities.iter().map(identity_json).collect(),
        created_at: format_ts(user.created_at),
        updated_at: format_ts(user.updated_at),
        is_anonymous: user.is_anonymous,
        banned_until: user.banned_until.map(format_ts),
    };
    serde_json::to_value(body).unwrap_or(Value::Null)
}

fn identity_json(identity: &IdentityRecord) -> IdentityJson<'_> {
    IdentityJson {
        identity_id: identity.id.to_string(),
        id: &identity.provider_id,
        user_id: identity.user_id.to_string(),
        identity_data: &identity.identity_data,
        provider: &identity.provider,
        last_sign_in_at: identity.last_sign_in_at.map(format_ts),
        created_at: format_ts(identity.created_at),
        updated_at: format_ts(identity.updated_at),
        email: if identity.email.is_empty() {
            None
        } else {
            Some(&identity.email)
        },
    }
}

#[derive(Serialize)]
struct UserJson<'a> {
    id: String,
    aud: &'a str,
    role: &'a str,
    email: &'a str,
    phone: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    email_confirmed_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    confirmed_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_sign_in_at: Option<String>,
    app_metadata: &'a Value,
    user_metadata: &'a Value,
    identities: Vec<IdentityJson<'a>>,
    created_at: String,
    updated_at: String,
    is_anonymous: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    banned_until: Option<String>,
}

#[derive(Serialize)]
struct IdentityJson<'a> {
    identity_id: String,
    id: &'a str,
    user_id: String,
    identity_data: &'a Value,
    provider: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_sign_in_at: Option<String>,
    created_at: String,
    updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    email: Option<&'a str>,
}

pub fn access_claims(
    user: &UserRecord,
    session_id: Uuid,
    amr_at: SystemTime,
    amr_method: &str,
    issuer: &str,
    now: i64,
    expires_in: i64,
) -> Value {
    let mut payload = Map::new();
    if !issuer.is_empty() {
        payload.insert("iss".into(), json!(issuer));
    }
    payload.insert("sub".into(), json!(user.id.to_string()));
    payload.insert("aud".into(), json!(user.aud));
    payload.insert("exp".into(), json!(now.saturating_add(expires_in)));
    payload.insert("iat".into(), json!(now));
    payload.insert("email".into(), json!(user.email));
    payload.insert("phone".into(), json!(user.phone));
    payload.insert("app_metadata".into(), user.app_metadata.clone());
    payload.insert("user_metadata".into(), user.user_metadata.clone());
    payload.insert("role".into(), json!(user.role));
    payload.insert("aal".into(), json!("aal1"));
    payload.insert(
        "amr".into(),
        json!([{ "method": amr_method, "timestamp": unix_secs(amr_at) }]),
    );
    payload.insert("session_id".into(), json!(session_id.to_string()));
    payload.insert("is_anonymous".into(), json!(user.is_anonymous));
    Value::Object(payload)
}
