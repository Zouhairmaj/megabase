// Ported from supabase/auth internal/api/mfa.go, internal/api/recovery_codes.go,
// internal/models/factor.go, internal/models/challenge.go,
// internal/models/recovery_code.go, and internal/models/amr.go (MIT),
// pin v2.197.0.

//! Enrollment, challenge, verification, and recovery-code writes.

use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};
use sqlx::Row;
use uuid::Uuid;

use super::super::{
    find_user_by_id, insert_audit, remember_audit, secure_alphanumeric, timed, ts, MemoryDb,
};
use super::{FactorRow, MemoryChallenge, MemoryCode, MemoryMfa, MemoryRecovery};
use crate::config::MfaConfig;
use crate::recovery_code::{self, HashError};
use crate::store::{Backend, BackendKind, StoreError, UserRecord};
use crate::totp::{self, TotpError};

/// A factor write that is either a database failure or a GoTrue HTTP error.
#[derive(Debug)]
pub(crate) enum MfaError {
    /// The database could not answer.
    Store(StoreError),
    /// Status, `error_code`, and `msg` taken from GoTrue.
    Status {
        status: u16,
        code: &'static str,
        message: String,
    },
}

impl From<StoreError> for MfaError {
    fn from(error: StoreError) -> Self {
        Self::Store(error)
    }
}

impl From<sqlx::Error> for MfaError {
    fn from(error: sqlx::Error) -> Self {
        Self::Store(StoreError::from(error))
    }
}

/// `EnrollFactorResponse` fields the route JSON needs.
#[derive(Debug)]
pub(crate) struct EnrolledFactor {
    pub id: Uuid,
    pub friendly_name: String,
    pub factor_type: &'static str,
    pub secret: String,
    pub uri: String,
    pub qr_code: String,
    /// Original `phone` request field. Empty for TOTP and WebAuthn.
    pub phone: String,
}

/// `ChallengeFactorResponse` without WebAuthn options.
#[derive(Debug)]
pub(crate) struct CreatedChallenge {
    pub id: Uuid,
    pub factor_type: String,
    pub expires_at: i64,
}

/// Inputs for `verifyTOTPFactor` and `verifyPhoneFactor` after the route
/// has decoded the body.
pub(crate) struct FactorVerify<'a> {
    pub session_id: Uuid,
    pub challenge_id: Uuid,
    pub code: &'a str,
    pub ip: &'a str,
}

/// Access-token material after a successful verify.
#[derive(Debug)]
pub(crate) struct UpgradedSession {
    pub user: UserRecord,
    pub refresh_token: String,
    /// AMR entries, most recent first.
    pub amr: Vec<(String, SystemTime)>,
}

/// Recovery codes returned once, on generate or regenerate.
#[derive(Debug)]
pub(crate) struct RecoveryIssue {
    pub id: Uuid,
    pub friendly_name: String,
    pub codes: Vec<String>,
}

fn http(code_status: u16, code: &'static str, message: impl Into<String>) -> MfaError {
    MfaError::Status {
        status: code_status,
        code,
        message: message.into(),
    }
}

fn go_quote(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| format!("\"{value}\""))
}

fn unix(time: SystemTime) -> i64 {
    time.duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

fn expires_at(created: SystemTime, seconds: u64) -> i64 {
    let extra = std::time::Duration::from_secs(seconds);
    unix(created.checked_add(extra).unwrap_or(created))
}

fn aal2_method(method: &str) -> bool {
    matches!(
        method,
        "totp" | "mfa/phone" | "mfa/webauthn" | "mfa/recovery_code"
    )
}

fn fresh_factor(user_id: Uuid, factor_type: &str, friendly_name: &str, phone: &str) -> FactorRow {
    FactorRow {
        id: Uuid::new_v4(),
        user_id,
        factor_type: factor_type.to_string(),
        status: "unverified".into(),
        friendly_name: friendly_name.to_string(),
        secret: String::new(),
        phone: phone.to_string(),
        created_at: SystemTime::now(),
        last_challenged_at: None,
    }
}

impl Backend {
    /// `enrollTOTPFactor`.
    ///
    /// # Errors
    /// Returns [`MfaError`] for a validation failure or a database error.
    pub(crate) async fn enroll_totp(
        &self,
        user: &UserRecord,
        session_id: Uuid,
        friendly_name: &str,
        issuer: &str,
        config: &MfaConfig,
    ) -> Result<EnrolledFactor, MfaError> {
        self.prepare_enroll(user.id, session_id, friendly_name, config)
            .await?;
        let key = totp::generate(issuer, &user.email).map_err(|error| match error {
            TotpError::Account | TotpError::Qr => {
                http(500, "unexpected_failure", "Error generating QR Code")
            }
        })?;
        let mut row = fresh_factor(user.id, "totp", friendly_name, "");
        row.secret = key.secret.clone();
        let id = row.id;
        self.insert_factor(user, &row, json!({ "factor_id": id.to_string() }))
            .await?;
        Ok(EnrolledFactor {
            id,
            friendly_name: friendly_name.to_string(),
            factor_type: "totp",
            secret: key.secret,
            uri: key.uri,
            qr_code: key.qr_code,
            phone: String::new(),
        })
    }

    /// `enrollPhoneFactor`. `phone` is normalized; `shown` is the request value.
    ///
    /// # Errors
    /// Returns [`MfaError`] for a validation failure or a database error.
    pub(crate) async fn enroll_phone(
        &self,
        user: &UserRecord,
        session_id: Uuid,
        friendly_name: &str,
        phone: &str,
        shown: &str,
        config: &MfaConfig,
    ) -> Result<EnrolledFactor, MfaError> {
        self.reject_verified_phone(user.id, phone).await?;
        self.delete_unverified_phone(user.id, phone).await?;
        self.prepare_enroll(user.id, session_id, friendly_name, config)
            .await?;
        let row = fresh_factor(user.id, "phone", friendly_name, phone);
        let id = row.id;
        self.insert_factor(
            user,
            &row,
            json!({ "factor_id": id.to_string(), "factor_type": "phone" }),
        )
        .await?;
        Ok(EnrolledFactor {
            id,
            friendly_name: friendly_name.to_string(),
            factor_type: "phone",
            secret: String::new(),
            uri: String::new(),
            qr_code: String::new(),
            phone: shown.to_string(),
        })
    }

    /// `enrollWebAuthnFactor`. The credential is created at challenge time.
    ///
    /// # Errors
    /// Returns [`MfaError`] for a validation failure or a database error.
    pub(crate) async fn enroll_webauthn(
        &self,
        user: &UserRecord,
        session_id: Uuid,
        friendly_name: &str,
        config: &MfaConfig,
    ) -> Result<EnrolledFactor, MfaError> {
        self.prepare_enroll(user.id, session_id, friendly_name, config)
            .await?;
        let row = fresh_factor(user.id, "webauthn", friendly_name, "");
        let id = row.id;
        self.insert_factor(
            user,
            &row,
            json!({ "factor_id": id.to_string(), "factor_type": "webauthn" }),
        )
        .await?;
        Ok(EnrolledFactor {
            id,
            friendly_name: friendly_name.to_string(),
            factor_type: "webauthn",
            secret: String::new(),
            uri: String::new(),
            qr_code: String::new(),
            phone: String::new(),
        })
    }

    /// `challengeTOTPFactor`.
    ///
    /// # Errors
    /// Returns [`MfaError`] when the challenge cannot be stored.
    pub(crate) async fn challenge_totp(
        &self,
        user: &UserRecord,
        factor: &FactorRow,
        ip: &str,
        config: &MfaConfig,
    ) -> Result<CreatedChallenge, MfaError> {
        self.write_challenge(user, factor, ip, "", config).await
    }

    /// Phone frequency gate. The route sends SMS only after this succeeds.
    ///
    /// # Errors
    /// Returns the GoTrue SMS rate-limit error, or a database error.
    pub(crate) async fn phone_challenge_allowed(
        &self,
        factor: &FactorRow,
        config: &MfaConfig,
    ) -> Result<(), MfaError> {
        let Some(last) = factor.last_challenged_at else {
            return Ok(());
        };
        let Some(deadline) = last.checked_add(config.phone_max_frequency) else {
            return Ok(());
        };
        let now = SystemTime::now();
        if now <= deadline {
            let seconds = deadline
                .duration_since(now)
                .map(|left| left.as_secs())
                .unwrap_or(0);
            return Err(http(
                429,
                "over_sms_send_rate_limit",
                format!(
                    "For security purposes, you can only request this after {seconds} seconds."
                ),
            ));
        }
        Ok(())
    }

    /// `challengePhoneFactor` after SMS delivery.
    ///
    /// # Errors
    /// Returns [`MfaError`] when the challenge cannot be stored.
    pub(crate) async fn challenge_phone(
        &self,
        user: &UserRecord,
        factor: &FactorRow,
        ip: &str,
        otp: &str,
        config: &MfaConfig,
    ) -> Result<CreatedChallenge, MfaError> {
        self.phone_challenge_allowed(factor, config).await?;
        self.write_challenge(user, factor, ip, otp, config).await
    }

    /// `verifyTOTPFactor`.
    ///
    /// # Errors
    /// Returns [`MfaError`] for a bad challenge, a bad code, or a database error.
    pub(crate) async fn verify_totp(
        &self,
        user: &UserRecord,
        factor: &FactorRow,
        attempt: FactorVerify<'_>,
        config: &MfaConfig,
    ) -> Result<UpgradedSession, MfaError> {
        let _otp = self
            .consume_challenge(factor, attempt.challenge_id, attempt.ip, config)
            .await?;
        let now = unix(SystemTime::now()) as u64;
        if !totp::verify(attempt.code, &factor.secret, now) {
            return Err(http(
                422,
                "mfa_verification_failed",
                "Invalid TOTP code entered",
            ));
        }
        self.finish_verify(
            user,
            attempt.session_id,
            factor,
            attempt.challenge_id,
            "totp",
        )
        .await
    }

    /// `verifyPhoneFactor`.
    ///
    /// # Errors
    /// Returns [`MfaError`] for a bad challenge, a bad code, or a database error.
    pub(crate) async fn verify_phone(
        &self,
        user: &UserRecord,
        factor: &FactorRow,
        attempt: FactorVerify<'_>,
        config: &MfaConfig,
    ) -> Result<UpgradedSession, MfaError> {
        let otp = self
            .consume_challenge(factor, attempt.challenge_id, attempt.ip, config)
            .await?;
        if !ct_eq(otp.as_bytes(), attempt.code.as_bytes()) {
            return Err(http(
                422,
                "mfa_verification_failed",
                "Invalid MFA Phone code entered",
            ));
        }
        self.finish_verify(
            user,
            attempt.session_id,
            factor,
            attempt.challenge_id,
            "mfa/phone",
        )
        .await
    }

    /// `RecoveryCodesGenerate` after the route's AAL and "already enrolled" checks.
    ///
    /// # Errors
    /// Returns [`MfaError`] when another verified factor is missing, a limit
    /// is hit, or the database cannot store the set.
    pub(crate) async fn generate_recovery_codes(
        &self,
        user: &UserRecord,
        session_id: Uuid,
        friendly_name: &str,
        config: &MfaConfig,
    ) -> Result<RecoveryIssue, MfaError> {
        let friendly_name = if friendly_name.trim().is_empty() {
            "Recovery codes"
        } else {
            friendly_name
        };
        self.prepare_enroll(user.id, session_id, friendly_name, config)
            .await?;
        self.require_other_verified_factor(user.id).await?;
        let (codes, hashes) = build_codes(config).await?;
        let mut row = fresh_factor(user.id, "recovery_code", friendly_name, "");
        row.status = "verified".into();
        let id = row.id;
        let count = codes.len();
        self.insert_recovery_set(
            user,
            &row,
            &hashes,
            "recovery_codes_generated",
            json!({ "factor_id": id.to_string(), "count": count }),
        )
        .await?;
        Ok(RecoveryIssue {
            id,
            friendly_name: friendly_name.to_string(),
            codes,
        })
    }

    /// `RecoveryCodesRegenerate`.
    ///
    /// # Errors
    /// Returns [`MfaError`] when the set is missing or the database fails.
    pub(crate) async fn regenerate_recovery_codes(
        &self,
        user: &UserRecord,
        config: &MfaConfig,
    ) -> Result<RecoveryIssue, MfaError> {
        let (codes, hashes) = build_codes(config).await?;
        self.replace_recovery_codes(user, &hashes, &codes).await
    }

    /// `RecoveryCodesVerify`.
    ///
    /// # Errors
    /// Returns [`MfaError`] for a bad code, a lockout, or a database error.
    pub(crate) async fn verify_recovery_code(
        &self,
        user: &UserRecord,
        session_id: Uuid,
        code: &str,
        config: &MfaConfig,
    ) -> Result<UpgradedSession, MfaError> {
        let normalized = recovery_code::normalize(code);
        self.redeem_recovery_code(user, session_id, &normalized, config)
            .await
    }
}

/// `crypto.GenerateOtp` for a phone challenge. Digits are already clamped.
pub(crate) fn phone_otp(digits: usize) -> String {
    let mut buf = [0u8; 8];
    if std::fs::File::open("/dev/urandom")
        .and_then(|mut file| std::io::Read::read_exact(&mut file, &mut buf))
        .is_err()
    {
        buf.fill(0);
    }
    let mut value = 0u64;
    for byte in buf {
        value = value.wrapping_mul(256).wrapping_add(u64::from(byte));
    }
    let upper = 10u64.pow(digits as u32);
    format!("{:0width$}", value % upper, width = digits)
}

fn ct_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut diff = 0u8;
    for (a, b) in left.iter().zip(right.iter()) {
        diff |= a ^ b;
    }
    diff == 0
}

async fn build_codes(config: &MfaConfig) -> Result<(Vec<String>, Vec<String>), MfaError> {
    let count = config.recovery_code_count;
    let length = config.recovery_code_length;
    tokio::task::spawn_blocking(move || {
        let mut codes = Vec::with_capacity(count);
        let mut hashes = Vec::with_capacity(count);
        for _ in 0..count {
            let code = recovery_code::generate_code(length).map_err(hash_to_mfa)?;
            let hash = recovery_code::hash_code(&code).map_err(hash_to_mfa)?;
            codes.push(code);
            hashes.push(hash);
        }
        Ok((codes, hashes))
    })
    .await
    .map_err(|_| StoreError::Hash)?
}

fn hash_to_mfa(_: HashError) -> MfaError {
    http(500, "unexpected_failure", "Error generating recovery codes")
}

impl Backend {
    async fn prepare_enroll(
        &self,
        user_id: Uuid,
        session_id: Uuid,
        friendly_name: &str,
        config: &MfaConfig,
    ) -> Result<(), MfaError> {
        match &self.inner {
            BackendKind::None => Err(StoreError::Unavailable.into()),
            BackendKind::Memory(db) => {
                let mut db = db.lock().await;
                let aal2 = db.mfa.aal2_sessions.contains_key(&session_id);
                purge_expired_memory(&mut db.mfa, config);
                validate_list(&db.mfa.factors, user_id, friendly_name, aal2, config)
            }
            BackendKind::Postgres(pg) => {
                timed(async {
                    let mut tx = pg.pool.begin().await?;
                    sqlx::query(
                        "DELETE FROM auth.mfa_factors AS f
                         WHERE f.user_id = $1 AND f.status::text <> 'verified'
                           AND NOT EXISTS (
                               SELECT 1 FROM auth.mfa_challenges c WHERE c.factor_id = f.id
                           )
                           AND f.created_at + make_interval(secs => $2) < now()",
                    )
                    .bind(user_id)
                    .bind(config.factor_expiry.as_secs_f64())
                    .execute(&mut *tx)
                    .await?;
                    let rows = sqlx::query(
                        "SELECT id, user_id, factor_type::text AS factor_type,
                                status::text AS status, COALESCE(friendly_name, '') AS friendly_name,
                                COALESCE(secret, '') AS secret, COALESCE(phone, '') AS phone,
                                created_at, last_challenged_at
                         FROM auth.mfa_factors WHERE user_id = $1",
                    )
                    .bind(user_id)
                    .fetch_all(&mut *tx)
                    .await?;
                    let factors = rows
                        .iter()
                        .map(super::factor_row)
                        .collect::<Result<Vec<_>, _>>()?;
                    let aal = sqlx::query("SELECT aal::text AS aal FROM auth.sessions WHERE id = $1")
                        .bind(session_id)
                        .fetch_optional(&mut *tx)
                        .await?;
                    let aal2 = aal
                        .and_then(|row| row.try_get::<Option<String>, _>("aal").ok().flatten())
                        .as_deref()
                        == Some("aal2");
                    tx.commit().await?;
                    validate_list(&factors, user_id, friendly_name, aal2, config)
                })
                .await
            }
        }
    }

    async fn reject_verified_phone(&self, user_id: Uuid, phone: &str) -> Result<(), MfaError> {
        let hit = match &self.inner {
            BackendKind::None => return Err(StoreError::Unavailable.into()),
            BackendKind::Memory(db) => db.lock().await.mfa.factors.iter().any(|factor| {
                factor.user_id == user_id
                    && factor.factor_type == "phone"
                    && factor.phone == phone
                    && factor.is_verified()
            }),
            BackendKind::Postgres(pg) => {
                timed(async {
                    let row = sqlx::query(
                        "SELECT 1 FROM auth.mfa_factors
                         WHERE user_id = $1 AND factor_type = 'phone' AND phone = $2
                           AND status = 'verified' LIMIT 1",
                    )
                    .bind(user_id)
                    .bind(phone)
                    .fetch_optional(&pg.pool)
                    .await?;
                    Ok::<bool, MfaError>(row.is_some())
                })
                .await?
            }
        };
        if hit {
            Err(http(
                422,
                "mfa_verified_factor_exists",
                "A verified phone factor already exists, unenroll the existing factor to continue",
            ))
        } else {
            Ok(())
        }
    }

    async fn delete_unverified_phone(&self, user_id: Uuid, phone: &str) -> Result<(), MfaError> {
        match &self.inner {
            BackendKind::None => Err(StoreError::Unavailable.into()),
            BackendKind::Memory(db) => {
                db.lock().await.mfa.factors.retain(|factor| {
                    !(factor.user_id == user_id
                        && factor.factor_type == "phone"
                        && factor.phone == phone
                        && !factor.is_verified())
                });
                Ok(())
            }
            BackendKind::Postgres(pg) => {
                timed(async {
                    sqlx::query(
                        "DELETE FROM auth.mfa_factors
                         WHERE user_id = $1 AND factor_type = 'phone' AND phone = $2
                           AND status = 'unverified'",
                    )
                    .bind(user_id)
                    .bind(phone)
                    .execute(&pg.pool)
                    .await?;
                    Ok(())
                })
                .await
            }
        }
    }

    async fn require_other_verified_factor(&self, user_id: Uuid) -> Result<(), MfaError> {
        let ok = match &self.inner {
            BackendKind::None => return Err(StoreError::Unavailable.into()),
            BackendKind::Memory(db) => db.lock().await.mfa.factors.iter().any(|factor| {
                factor.user_id == user_id && factor.is_verified() && factor.factor_type != "recovery_code"
            }),
            BackendKind::Postgres(pg) => {
                timed(async {
                    let row = sqlx::query(
                        "SELECT 1 FROM auth.mfa_factors
                         WHERE user_id = $1 AND status = 'verified' AND factor_type <> 'recovery_code'
                         LIMIT 1",
                    )
                    .bind(user_id)
                    .fetch_optional(&pg.pool)
                    .await?;
                    Ok::<bool, MfaError>(row.is_some())
                })
                .await?
            }
        };
        if ok {
            Ok(())
        } else {
            Err(http(
                422,
                "mfa_recovery_codes_sole_factor",
                "At least one other verified factor is required to generate recovery codes",
            ))
        }
    }

    async fn insert_factor(
        &self,
        user: &UserRecord,
        factor: &FactorRow,
        traits: Value,
    ) -> Result<(), MfaError> {
        match &self.inner {
            BackendKind::None => Err(StoreError::Unavailable.into()),
            BackendKind::Memory(db) => {
                let mut db = db.lock().await;
                db.mfa.factors.push(factor.clone());
                remember_audit(user, "factor_in_progress", "factor", Some(traits));
                Ok(())
            }
            BackendKind::Postgres(pg) => {
                let factor = factor.clone();
                let user = user.clone();
                timed(async move {
                    let mut tx = pg.pool.begin().await?;
                    insert_factor_sql(&mut tx, &factor).await?;
                    insert_audit(&mut tx, &user, "factor_in_progress", "factor", Some(traits))
                        .await?;
                    tx.commit().await?;
                    Ok(())
                })
                .await
            }
        }
    }

    async fn insert_recovery_set(
        &self,
        user: &UserRecord,
        factor: &FactorRow,
        hashes: &[String],
        action: &str,
        traits: Value,
    ) -> Result<(), MfaError> {
        match &self.inner {
            BackendKind::None => Err(StoreError::Unavailable.into()),
            BackendKind::Memory(db) => {
                let mut db = db.lock().await;
                db.mfa.factors.push(factor.clone());
                store_memory_codes(&mut db.mfa, user.id, factor.id, hashes, true);
                remember_audit(user, action, "factor", Some(traits));
                Ok(())
            }
            BackendKind::Postgres(pg) => {
                let factor = factor.clone();
                let user = user.clone();
                let hashes = hashes.to_vec();
                let action = action.to_string();
                timed(async move {
                    let mut tx = pg.pool.begin().await?;
                    insert_factor_sql(&mut tx, &factor).await?;
                    let set_id = Uuid::new_v4();
                    sqlx::query(
                        "INSERT INTO auth.mfa_recovery_code_sets
                            (id, user_id, mfa_factor_id, failed_verification_count, created_at, updated_at)
                         VALUES ($1, $2, $3, 0, now(), now())",
                    )
                    .bind(set_id)
                    .bind(factor.user_id)
                    .bind(factor.id)
                    .execute(&mut *tx)
                    .await?;
                    insert_code_rows(&mut tx, set_id, &hashes).await?;
                    insert_audit(&mut tx, &user, &action, "factor", Some(traits)).await?;
                    tx.commit().await?;
                    Ok(())
                })
                .await
            }
        }
    }

    async fn write_challenge(
        &self,
        user: &UserRecord,
        factor: &FactorRow,
        ip: &str,
        otp: &str,
        config: &MfaConfig,
    ) -> Result<CreatedChallenge, MfaError> {
        let id = Uuid::new_v4();
        let created = SystemTime::now();
        match &self.inner {
            BackendKind::None => Err(StoreError::Unavailable.into()),
            BackendKind::Memory(db) => {
                let mut db = db.lock().await;
                if let Some(row) = db.mfa.factors.iter_mut().find(|row| row.id == factor.id) {
                    row.last_challenged_at = Some(created);
                }
                db.mfa.challenges.push(MemoryChallenge {
                    id,
                    factor_id: factor.id,
                    created_at: created,
                    verified_at: None,
                    ip: ip.to_string(),
                    otp: otp.to_string(),
                });
                remember_audit(
                    user,
                    "challenge_created",
                    "factor",
                    Some(json!({
                        "factor_id": factor.id.to_string(),
                        "factor_status": factor.status,
                    })),
                );
                Ok(CreatedChallenge {
                    id,
                    factor_type: factor.factor_type.clone(),
                    expires_at: expires_at(created, config.challenge_expiry_seconds),
                })
            }
            BackendKind::Postgres(pg) => {
                let factor_id = factor.id;
                let factor_type = factor.factor_type.clone();
                let status_text = factor.status.clone();
                let user = user.clone();
                let ip = ip.to_string();
                let otp = otp.to_string();
                let expiry = config.challenge_expiry_seconds;
                timed(async move {
                    let mut tx = pg.pool.begin().await?;
                    sqlx::query(
                        "INSERT INTO auth.mfa_challenges
                            (id, factor_id, created_at, ip_address, otp_code)
                         VALUES ($1, $2, $3, $4::inet, NULLIF($5, ''))",
                    )
                    .bind(id)
                    .bind(factor_id)
                    .bind(ts(created))
                    .bind(&ip)
                    .bind(&otp)
                    .execute(&mut *tx)
                    .await?;
                    sqlx::query(
                        "UPDATE auth.mfa_factors
                         SET last_challenged_at = $2, updated_at = $2 WHERE id = $1",
                    )
                    .bind(factor_id)
                    .bind(ts(created))
                    .execute(&mut *tx)
                    .await?;
                    insert_audit(
                        &mut tx,
                        &user,
                        "challenge_created",
                        "factor",
                        Some(json!({
                            "factor_id": factor_id.to_string(),
                            "factor_status": status_text,
                        })),
                    )
                    .await?;
                    tx.commit().await?;
                    Ok(CreatedChallenge {
                        id,
                        factor_type,
                        expires_at: expires_at(created, expiry),
                    })
                })
                .await
            }
        }
    }

    /// Load a challenge, reject a mismatch or expiry, and return its OTP.
    /// The row stays until [`Self::finish_verify`] marks it verified, matching
    /// GoTrue (a failed code does not consume the challenge).
    async fn consume_challenge(
        &self,
        factor: &FactorRow,
        challenge_id: Uuid,
        ip: &str,
        config: &MfaConfig,
    ) -> Result<String, MfaError> {
        match &self.inner {
            BackendKind::None => Err(StoreError::Unavailable.into()),
            BackendKind::Memory(db) => {
                let mut db = db.lock().await;
                let Some(index) = db
                    .mfa
                    .challenges
                    .iter()
                    .position(|row| row.id == challenge_id && row.factor_id == factor.id)
                else {
                    return Err(http(
                        422,
                        "mfa_factor_not_found",
                        "MFA factor with the provided challenge ID not found",
                    ));
                };
                let created = db.mfa.challenges[index].created_at;
                let verified = db.mfa.challenges[index].verified_at.is_some();
                let stored_ip = db.mfa.challenges[index].ip.clone();
                let otp = db.mfa.challenges[index].otp.clone();
                if verified || stored_ip != ip {
                    return Err(http(
                        422,
                        "mfa_ip_address_mismatch",
                        "Challenge and verify IP addresses mismatch.",
                    ));
                }
                let deadline = created
                    .checked_add(std::time::Duration::from_secs(
                        config.challenge_expiry_seconds,
                    ))
                    .unwrap_or(created);
                if SystemTime::now() > deadline {
                    db.mfa.challenges.remove(index);
                    return Err(http(
                        422,
                        "mfa_challenge_expired",
                        format!("MFA challenge {challenge_id} has expired, verify against another challenge or create a new challenge."),
                    ));
                }
                Ok(otp)
            }
            BackendKind::Postgres(pg) => {
                let factor_id = factor.id;
                let ip = ip.to_string();
                let expiry = config.challenge_expiry_seconds;
                timed(async move {
                    let row = sqlx::query(
                        "SELECT created_at, verified_at, host(ip_address) AS ip,
                                COALESCE(otp_code, '') AS otp
                         FROM auth.mfa_challenges
                         WHERE id = $1 AND factor_id = $2",
                    )
                    .bind(challenge_id)
                    .bind(factor_id)
                    .fetch_optional(&pg.pool)
                    .await?;
                    let Some(row) = row else {
                        return Err(http(
                            422,
                            "mfa_factor_not_found",
                            "MFA factor with the provided challenge ID not found",
                        ));
                    };
                    let created: chrono::DateTime<chrono::Utc> = row.try_get("created_at")?;
                    let verified: Option<chrono::DateTime<chrono::Utc>> = row.try_get("verified_at")?;
                    let stored_ip: String = row.try_get("ip")?;
                    let otp: String = row.try_get("otp")?;
                    if verified.is_some() || stored_ip != ip {
                        return Err(http(
                            422,
                            "mfa_ip_address_mismatch",
                            "Challenge and verify IP addresses mismatch.",
                        ));
                    }
                    let deadline = created + chrono::Duration::seconds(expiry as i64);
                    if chrono::Utc::now() > deadline {
                        sqlx::query("DELETE FROM auth.mfa_challenges WHERE id = $1")
                            .bind(challenge_id)
                            .execute(&pg.pool)
                            .await?;
                        return Err(http(
                            422,
                            "mfa_challenge_expired",
                            format!("MFA challenge {challenge_id} has expired, verify against another challenge or create a new challenge."),
                        ));
                    }
                    Ok(otp)
                })
                .await
            }
        }
    }

    async fn finish_verify(
        &self,
        user: &UserRecord,
        session_id: Uuid,
        factor: &FactorRow,
        challenge_id: Uuid,
        method: &str,
    ) -> Result<UpgradedSession, MfaError> {
        let traits = json!({
            "factor_id": factor.id.to_string(),
            "challenge_id": challenge_id.to_string(),
            "factor_type": factor.factor_type,
        });
        match &self.inner {
            BackendKind::None => Err(StoreError::Unavailable.into()),
            BackendKind::Memory(db) => {
                let mut db = db.lock().await;
                if let Some(challenge) = db
                    .mfa
                    .challenges
                    .iter_mut()
                    .find(|row| row.id == challenge_id)
                {
                    challenge.verified_at = Some(SystemTime::now());
                }
                if let Some(row) = db.mfa.factors.iter_mut().find(|row| row.id == factor.id) {
                    row.status = "verified".into();
                }
                let factor_type = factor.factor_type.clone();
                let factor_id = factor.id;
                db.mfa.factors.retain(|row| {
                    !(row.user_id == user.id
                        && row.factor_type == factor_type
                        && row.status != "verified")
                });
                let upgraded = upgrade_memory(&mut db, user, session_id, factor_id, method)?;
                remember_audit(user, "verification_attempted", "factor", Some(traits));
                Ok(upgraded)
            }
            BackendKind::Postgres(pg) => {
                let user = user.clone();
                let factor_id = factor.id;
                let factor_type = factor.factor_type.clone();
                let method = method.to_string();
                timed(async move {
                    let mut tx = pg.pool.begin().await?;
                    sqlx::query(
                        "UPDATE auth.mfa_challenges SET verified_at = now() WHERE id = $1",
                    )
                    .bind(challenge_id)
                    .execute(&mut *tx)
                    .await?;
                    sqlx::query(
                        "UPDATE auth.mfa_factors
                         SET status = 'verified', updated_at = now() WHERE id = $1",
                    )
                    .bind(factor_id)
                    .execute(&mut *tx)
                    .await?;
                    insert_audit(
                        &mut tx,
                        &user,
                        "verification_attempted",
                        "factor",
                        Some(traits),
                    )
                    .await?;
                    let upgraded =
                        upgrade_postgres(&mut tx, &user, session_id, factor_id, &method).await?;
                    sqlx::query(
                        "DELETE FROM auth.sessions
                         WHERE user_id = $1 AND aal < 'aal2'::auth.aal_level",
                    )
                    .bind(user.id)
                    .execute(&mut *tx)
                    .await?;
                    sqlx::query(
                        "DELETE FROM auth.mfa_factors
                         WHERE user_id = $1 AND status = 'unverified' AND factor_type = $2::auth.factor_type",
                    )
                    .bind(user.id)
                    .bind(&factor_type)
                    .execute(&mut *tx)
                    .await?;
                    tx.commit().await?;
                    Ok(upgraded)
                })
                .await
            }
        }
    }

    async fn replace_recovery_codes(
        &self,
        user: &UserRecord,
        hashes: &[String],
        codes: &[String],
    ) -> Result<RecoveryIssue, MfaError> {
        match &self.inner {
            BackendKind::None => Err(StoreError::Unavailable.into()),
            BackendKind::Memory(db) => {
                let mut db = db.lock().await;
                let Some(factor) = db
                    .mfa
                    .factors
                    .iter()
                    .find(|row| row.user_id == user.id && row.factor_type == "recovery_code")
                    .cloned()
                else {
                    return Err(http(
                        404,
                        "mfa_factor_not_found",
                        "The user has not enrolled recovery codes",
                    ));
                };
                store_memory_codes(&mut db.mfa, user.id, factor.id, hashes, true);
                let count = codes.len();
                remember_audit(
                    user,
                    "recovery_codes_regenerated",
                    "factor",
                    Some(json!({ "factor_id": factor.id.to_string(), "count": count })),
                );
                Ok(RecoveryIssue {
                    id: factor.id,
                    friendly_name: factor.friendly_name,
                    codes: codes.to_vec(),
                })
            }
            BackendKind::Postgres(pg) => {
                let user = user.clone();
                let hashes = hashes.to_vec();
                let codes = codes.to_vec();
                timed(async move {
                    let mut tx = pg.pool.begin().await?;
                    let set = sqlx::query(
                        "SELECT id, mfa_factor_id FROM auth.mfa_recovery_code_sets
                         WHERE user_id = $1 LIMIT 1 FOR UPDATE",
                    )
                    .bind(user.id)
                    .fetch_optional(&mut *tx)
                    .await?;
                    let Some(set) = set else {
                        return Err(http(
                            404,
                            "mfa_factor_not_found",
                            "The user has not enrolled recovery codes",
                        ));
                    };
                    let set_id: Uuid = set.try_get("id")?;
                    let factor_id: Uuid = set.try_get("mfa_factor_id")?;
                    let name: String = sqlx::query(
                        "SELECT COALESCE(friendly_name, '') AS friendly_name
                         FROM auth.mfa_factors WHERE id = $1",
                    )
                    .bind(factor_id)
                    .fetch_optional(&mut *tx)
                    .await?
                    .map(|row| row.try_get("friendly_name"))
                    .transpose()?
                    .unwrap_or_default();
                    sqlx::query(
                        "DELETE FROM auth.mfa_recovery_codes WHERE mfa_recovery_code_set_id = $1",
                    )
                    .bind(set_id)
                    .execute(&mut *tx)
                    .await?;
                    insert_code_rows(&mut tx, set_id, &hashes).await?;
                    sqlx::query(
                        "UPDATE auth.mfa_recovery_code_sets
                         SET failed_verification_count = 0, verification_locked_until = NULL, updated_at = now()
                         WHERE id = $1",
                    )
                    .bind(set_id)
                    .execute(&mut *tx)
                    .await?;
                    let count = codes.len();
                    insert_audit(
                        &mut tx,
                        &user,
                        "recovery_codes_regenerated",
                        "factor",
                        Some(json!({ "factor_id": factor_id.to_string(), "count": count })),
                    )
                    .await?;
                    tx.commit().await?;
                    Ok(RecoveryIssue {
                        id: factor_id,
                        friendly_name: name,
                        codes,
                    })
                })
                .await
            }
        }
    }

    async fn redeem_recovery_code(
        &self,
        user: &UserRecord,
        session_id: Uuid,
        code: &str,
        config: &MfaConfig,
    ) -> Result<UpgradedSession, MfaError> {
        match &self.inner {
            BackendKind::None => Err(StoreError::Unavailable.into()),
            BackendKind::Memory(db) => {
                let mut db = db.lock().await;
                let Some(factor) = db
                    .mfa
                    .factors
                    .iter()
                    .find(|row| row.user_id == user.id && row.factor_type == "recovery_code")
                    .cloned()
                else {
                    return Err(invalid_recovery());
                };
                let Some(set) = db.mfa.recovery_codes.get(&user.id) else {
                    return Err(invalid_recovery());
                };
                if set
                    .locked_until
                    .is_some_and(|until| SystemTime::now() < until)
                {
                    return Err(locked_recovery());
                }
                let matched = match_codes(&set.codes, code);
                apply_recovery_result(&mut db, user, session_id, &factor, matched, config)
            }
            BackendKind::Postgres(pg) => {
                let user = user.clone();
                let code = code.to_string();
                let max_attempts = config.recovery_max_attempts;
                let lockout = config.recovery_lockout;
                timed(async move {
                    let mut tx = pg.pool.begin().await?;
                    let set = sqlx::query(
                        "SELECT id, mfa_factor_id, failed_verification_count, verification_locked_until
                         FROM auth.mfa_recovery_code_sets WHERE user_id = $1 LIMIT 1 FOR UPDATE",
                    )
                    .bind(user.id)
                    .fetch_optional(&mut *tx)
                    .await?;
                    let Some(set) = set else {
                        return Err(invalid_recovery());
                    };
                    let set_id: Uuid = set.try_get("id")?;
                    let factor_id: Uuid = set.try_get("mfa_factor_id")?;
                    let mut failed: i32 = set.try_get("failed_verification_count")?;
                    let locked: Option<chrono::DateTime<chrono::Utc>> =
                        set.try_get("verification_locked_until")?;
                    if locked.is_some_and(|until| chrono::Utc::now() < until) {
                        return Err(locked_recovery());
                    }
                    if locked.is_some() {
                        failed = 0;
                        sqlx::query(
                            "UPDATE auth.mfa_recovery_code_sets
                             SET failed_verification_count = 0, verification_locked_until = NULL, updated_at = now()
                             WHERE id = $1",
                        )
                        .bind(set_id)
                        .execute(&mut *tx)
                        .await?;
                    }
                    let rows = sqlx::query(
                        "SELECT id, code_hash FROM auth.mfa_recovery_codes
                         WHERE mfa_recovery_code_set_id = $1 AND consumed_at IS NULL
                         ORDER BY created_at ASC, id ASC",
                    )
                    .bind(set_id)
                    .fetch_all(&mut *tx)
                    .await?;
                    let mut matched: Option<Uuid> = None;
                    for row in &rows {
                        let hash: String = row.try_get("code_hash")?;
                        match recovery_code::matches(&hash, &code) {
                            Ok(true) if matched.is_none() => matched = Some(row.try_get("id")?),
                            Ok(_) => {}
                            Err(_) => {}
                        }
                    }
                    let Some(code_id) = matched else {
                        let next = failed.saturating_add(1);
                        let lock_until = (next >= max_attempts).then(|| {
                            chrono::Utc::now()
                                + chrono::Duration::seconds(lockout.as_secs() as i64)
                        });
                        sqlx::query(
                            "UPDATE auth.mfa_recovery_code_sets
                             SET failed_verification_count = $2,
                                 verification_locked_until = $3,
                                 updated_at = now()
                             WHERE id = $1",
                        )
                        .bind(set_id)
                        .bind(next)
                        .bind(lock_until)
                        .execute(&mut *tx)
                        .await?;
                        tx.commit().await?;
                        return Err(invalid_recovery());
                    };
                    let updated = sqlx::query(
                        "UPDATE auth.mfa_recovery_codes
                         SET consumed_at = now() WHERE id = $1 AND consumed_at IS NULL",
                    )
                    .bind(code_id)
                    .execute(&mut *tx)
                    .await?;
                    if updated.rows_affected() == 0 {
                        tx.commit().await?;
                        return Err(invalid_recovery());
                    }
                    sqlx::query(
                        "UPDATE auth.mfa_recovery_code_sets
                         SET failed_verification_count = 0, verification_locked_until = NULL, updated_at = now()
                         WHERE id = $1",
                    )
                    .bind(set_id)
                    .execute(&mut *tx)
                    .await?;
                    let remaining: i64 = sqlx::query(
                        "SELECT COUNT(*) AS remaining FROM auth.mfa_recovery_codes
                         WHERE mfa_recovery_code_set_id = $1 AND consumed_at IS NULL",
                    )
                    .bind(set_id)
                    .fetch_one(&mut *tx)
                    .await?
                    .try_get("remaining")?;
                    insert_audit(
                        &mut tx,
                        &user,
                        "recovery_codes_verified",
                        "factor",
                        Some(json!({
                            "factor_id": factor_id.to_string(),
                            "remaining": remaining,
                        })),
                    )
                    .await?;
                    let upgraded =
                        upgrade_postgres(&mut tx, &user, session_id, factor_id, "mfa/recovery_code")
                            .await?;
                    sqlx::query(
                        "DELETE FROM auth.sessions WHERE user_id = $1 AND aal < 'aal2'::auth.aal_level",
                    )
                    .bind(user.id)
                    .execute(&mut *tx)
                    .await?;
                    tx.commit().await?;
                    Ok(upgraded)
                })
                .await
            }
        }
    }
}

fn invalid_recovery() -> MfaError {
    http(
        422,
        "mfa_verification_failed",
        "Invalid recovery code entered",
    )
}

fn locked_recovery() -> MfaError {
    http(
        429,
        "mfa_recovery_codes_locked",
        "Too many failed verification attempts, try again later",
    )
}

fn match_codes(codes: &[MemoryCode], code: &str) -> Option<Uuid> {
    let mut matched = None;
    for entry in codes {
        if entry.consumed_at.is_some() {
            continue;
        }
        match recovery_code::matches(&entry.hash, code) {
            Ok(true) if matched.is_none() => matched = Some(entry.id),
            Ok(_) => {}
            Err(_) => {}
        }
    }
    matched
}

fn apply_recovery_result(
    db: &mut MemoryDb,
    user: &UserRecord,
    session_id: Uuid,
    factor: &FactorRow,
    matched: Option<Uuid>,
    config: &MfaConfig,
) -> Result<UpgradedSession, MfaError> {
    let now = SystemTime::now();
    let set = db
        .mfa
        .recovery_codes
        .get_mut(&user.id)
        .ok_or_else(invalid_recovery)?;
    if let Some(until) = set.locked_until {
        if now >= until {
            set.failed = 0;
            set.locked_until = None;
        }
    }
    let consumed = matched.is_some_and(|id| {
        set.codes.iter_mut().any(|entry| {
            if entry.id == id && entry.consumed_at.is_none() {
                entry.consumed_at = Some(now);
                true
            } else {
                false
            }
        })
    });
    if !consumed {
        set.failed = set.failed.saturating_add(1);
        if set.failed >= config.recovery_max_attempts {
            set.locked_until = Some(now + config.recovery_lockout);
        }
        if let Some(status) = db.mfa.recovery.get_mut(&user.id) {
            // Counts stay; a failure does not consume a code.
            let _ = status;
        }
        return Err(invalid_recovery());
    }
    set.failed = 0;
    set.locked_until = None;
    if let Some(status) = db.mfa.recovery.get_mut(&user.id) {
        status.remaining = status.remaining.saturating_sub(1);
    }
    let remaining = db
        .mfa
        .recovery
        .get(&user.id)
        .map(|status| status.remaining)
        .unwrap_or(0);
    let factor_id = factor.id;
    let upgraded = upgrade_memory(db, user, session_id, factor_id, "mfa/recovery_code")?;
    remember_audit(
        user,
        "recovery_codes_verified",
        "factor",
        Some(json!({
            "factor_id": factor_id.to_string(),
            "remaining": remaining,
        })),
    );
    Ok(upgraded)
}

fn validate_list(
    factors: &[FactorRow],
    user_id: Uuid,
    friendly_name: &str,
    aal2: bool,
    config: &MfaConfig,
) -> Result<(), MfaError> {
    let owned: Vec<&FactorRow> = factors
        .iter()
        .filter(|factor| factor.user_id == user_id)
        .collect();
    if owned
        .iter()
        .any(|factor| factor.friendly_name == friendly_name)
    {
        return Err(http(
            422,
            "mfa_factor_name_conflict",
            format!(
                "A factor with the friendly name {} for this user already exists",
                go_quote(friendly_name)
            ),
        ));
    }
    let verified = owned.iter().filter(|factor| factor.is_verified()).count();
    if owned.len() >= config.max_enrolled_factors as usize
        || verified >= config.max_verified_factors.max(0) as usize
    {
        return Err(http(
            422,
            "too_many_enrolled_mfa_factors",
            "Maximum number of verified factors reached, unenroll to continue",
        ));
    }
    if verified > 0 && !aal2 {
        return Err(http(
            403,
            "insufficient_aal",
            "AAL2 required to enroll a new factor",
        ));
    }
    Ok(())
}

fn purge_expired_memory(mfa: &mut MemoryMfa, config: &MfaConfig) {
    let now = SystemTime::now();
    let challenged: Vec<Uuid> = mfa.challenges.iter().map(|row| row.factor_id).collect();
    mfa.factors.retain(|factor| {
        if factor.is_verified() || challenged.contains(&factor.id) {
            return true;
        }
        let Some(deadline) = factor.created_at.checked_add(config.factor_expiry) else {
            return true;
        };
        now <= deadline
    });
}

fn store_memory_codes(
    mfa: &mut MemoryMfa,
    user_id: Uuid,
    factor_id: Uuid,
    hashes: &[String],
    reset_status: bool,
) {
    let codes = hashes
        .iter()
        .map(|hash| MemoryCode {
            id: Uuid::new_v4(),
            hash: hash.clone(),
            consumed_at: None,
        })
        .collect::<Vec<_>>();
    let total = codes.len() as i64;
    mfa.recovery_codes.insert(
        user_id,
        MemoryRecovery {
            failed: 0,
            locked_until: None,
            codes,
        },
    );
    if reset_status {
        mfa.recovery.insert(
            user_id,
            super::RecoveryCodeStatus {
                factor_id,
                total,
                remaining: total,
            },
        );
    }
}

fn upgrade_memory(
    db: &mut MemoryDb,
    user: &UserRecord,
    session_id: Uuid,
    factor_id: Uuid,
    method: &str,
) -> Result<UpgradedSession, MfaError> {
    let now = SystemTime::now();
    let amr_at = db
        .refresh_tokens
        .values()
        .find(|row| row.session_id == session_id && !row.revoked)
        .map(|row| row.amr_at);
    let current = db
        .refresh_tokens
        .iter()
        .find(|(_, row)| row.session_id == session_id && !row.revoked)
        .map(|(token, row)| (token.clone(), row.amr_at));
    let Some((old_token, inherited)) = current else {
        return Err(StoreError::Unavailable.into());
    };
    if let Some(row) = db.refresh_tokens.get_mut(&old_token) {
        row.revoked = true;
    }
    let refresh_token = secure_alphanumeric(12);
    db.refresh_tokens.insert(
        refresh_token.clone(),
        super::super::LegacyRefresh {
            user_id: user.id,
            session_id,
            revoked: false,
            parent: old_token,
            amr_at: inherited,
        },
    );
    remember_audit(user, "token_revoked", "token", None);
    let claims = db.mfa.amr.entry(session_id).or_default();
    if claims.is_empty() {
        if let Some(at) = amr_at {
            claims.push(("password".into(), at));
        }
    }
    if let Some(existing) = claims.iter_mut().find(|(name, _)| name == method) {
        existing.1 = now;
    } else {
        claims.push((method.to_string(), now));
    }
    let mut amr = claims.clone();
    amr.sort_by_key(|entry| std::cmp::Reverse(entry.1));
    db.mfa.aal2_sessions.insert(session_id, factor_id);
    let drop_ids: Vec<Uuid> = db
        .sessions
        .iter()
        .filter(|(id, owner)| {
            **owner == user.id && **id != session_id && !db.mfa.aal2_sessions.contains_key(*id)
        })
        .map(|(id, _)| *id)
        .collect();
    for id in drop_ids {
        db.sessions.remove(&id);
        db.refresh_tokens.retain(|_, row| row.session_id != id);
        db.mfa.amr.remove(&id);
        db.mfa.aal2_sessions.remove(&id);
    }
    let _ = aal2_method(method);
    Ok(UpgradedSession {
        user: user.clone(),
        refresh_token,
        amr,
    })
}

async fn upgrade_postgres(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user: &UserRecord,
    session_id: Uuid,
    factor_id: Uuid,
    method: &str,
) -> Result<UpgradedSession, MfaError> {
    let now = chrono::Utc::now();
    sqlx::query(
        "INSERT INTO auth.mfa_amr_claims
            (id, session_id, created_at, updated_at, authentication_method)
         VALUES ($1, $2, $3, $3, $4)
         ON CONFLICT ON CONSTRAINT mfa_amr_claims_session_id_authentication_method_pkey
         DO UPDATE SET updated_at = EXCLUDED.updated_at",
    )
    .bind(Uuid::new_v4())
    .bind(session_id)
    .bind(now)
    .bind(method)
    .execute(&mut **tx)
    .await?;
    let old = sqlx::query(
        "SELECT token FROM auth.refresh_tokens
         WHERE instance_id = $1 AND session_id = $2 AND revoked = false
         ORDER BY created_at ASC LIMIT 1",
    )
    .bind(Uuid::nil())
    .bind(session_id)
    .fetch_optional(&mut **tx)
    .await?;
    let Some(old) = old else {
        return Err(StoreError::Unavailable.into());
    };
    let old_token: String = old.try_get("token")?;
    let refresh_token = secure_alphanumeric(12);
    insert_audit(tx, user, "token_revoked", "token", None).await?;
    sqlx::query(
        "UPDATE auth.refresh_tokens SET revoked = true, updated_at = now() WHERE token = $1",
    )
    .bind(&old_token)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "INSERT INTO auth.refresh_tokens
            (instance_id, token, user_id, revoked, created_at, updated_at, parent, session_id)
         VALUES ($1, $2, $3, false, now(), now(), $4, $5)",
    )
    .bind(Uuid::nil())
    .bind(&refresh_token)
    .bind(user.id.to_string())
    .bind(&old_token)
    .bind(session_id)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "UPDATE auth.sessions SET aal = 'aal2', factor_id = $2, updated_at = now() WHERE id = $1",
    )
    .bind(session_id)
    .bind(factor_id)
    .execute(&mut **tx)
    .await?;
    let claims = sqlx::query(
        "SELECT authentication_method, updated_at FROM auth.mfa_amr_claims WHERE session_id = $1",
    )
    .bind(session_id)
    .fetch_all(&mut **tx)
    .await?;
    let mut amr = Vec::new();
    for row in claims {
        let method: String = row.try_get("authentication_method")?;
        let updated: chrono::DateTime<chrono::Utc> = row.try_get("updated_at")?;
        amr.push((method, super::super::from_ts(updated)));
    }
    amr.sort_by_key(|entry| std::cmp::Reverse(entry.1));
    let reloaded = find_user_by_id(tx, user.id)
        .await?
        .unwrap_or_else(|| user.clone());
    let _ = aal2_method;
    Ok(UpgradedSession {
        user: reloaded,
        refresh_token,
        amr,
    })
}

async fn insert_factor_sql(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    factor: &FactorRow,
) -> Result<(), StoreError> {
    let phone = if factor.phone.is_empty() {
        None
    } else {
        Some(factor.phone.as_str())
    };
    let secret = if factor.secret.is_empty() {
        None
    } else {
        Some(factor.secret.as_str())
    };
    sqlx::query(
        "INSERT INTO auth.mfa_factors
            (id, user_id, friendly_name, factor_type, status, secret, phone, created_at, updated_at)
         VALUES ($1, $2, $3, $4::auth.factor_type, $5::auth.factor_status, $6, $7, $8, $8)",
    )
    .bind(factor.id)
    .bind(factor.user_id)
    .bind(&factor.friendly_name)
    .bind(&factor.factor_type)
    .bind(&factor.status)
    .bind(secret)
    .bind(phone)
    .bind(ts(factor.created_at))
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn insert_code_rows(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    set_id: Uuid,
    hashes: &[String],
) -> Result<(), StoreError> {
    for hash in hashes {
        sqlx::query(
            "INSERT INTO auth.mfa_recovery_codes
                (id, mfa_recovery_code_set_id, code_hash, created_at)
             VALUES ($1, $2, $3, now())",
        )
        .bind(Uuid::new_v4())
        .bind(set_id)
        .bind(hash)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}
