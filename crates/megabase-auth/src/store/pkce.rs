// Ported from supabase/auth internal/api/token.go (PKCE),
// internal/models/flow_state.go, and internal/tokens/service.go (MIT),
// pin v2.197.0.

//! `auth.flow_state` reads and the session write for `grant_type=pkce`.

use std::time::SystemTime;

use serde_json::json;
use uuid::Uuid;

use super::{
    customize_memory, customize_pg, find_user_by_id, flow_state_from_row, grant_session,
    insert_audit, insert_session_rows, reload_confirmed_at, remember_audit, timed, ts,
    write_to_store, Backend, BackendKind, FlowStateRecord, IssuedSession, StoreError,
    FLOW_STATE_COLUMNS,
};
use crate::hooks::TokenHookCall;

/// What [`Backend::redeem_pkce_flow`] needs from the flow state row.
pub struct PkceRedemption<'a> {
    pub flow_id: Uuid,
    pub user_id: Uuid,
    /// `AuthenticationMethod.String()` of the flow; the session's AMR claim.
    pub amr_method: &'static str,
    /// `provider_type` audit trait of the `login` entry.
    pub provider_type: &'a str,
}

impl Backend {
    /// `FindFlowStateByAuthCode`: the first `auth.flow_state` row whose
    /// `auth_code` matches, with its id.
    ///
    /// # Errors
    ///
    /// [`StoreError`] when the database is missing or the read fails.
    pub async fn find_flow_state_by_auth_code(
        &self,
        auth_code: &str,
    ) -> Result<Option<(Uuid, FlowStateRecord)>, StoreError> {
        match &self.inner {
            BackendKind::None => Err(StoreError::Unavailable),
            BackendKind::Memory(db) => Ok(db
                .lock()
                .await
                .flow_states
                .iter()
                .find(|(_, flow)| flow.auth_code.as_deref() == Some(auth_code))
                .map(|(id, flow)| (*id, flow.clone()))),
            BackendKind::Postgres(pg) => {
                timed(async {
                    use sqlx::Row;
                    let row = sqlx::query(&format!(
                        "SELECT {FLOW_STATE_COLUMNS} FROM auth.flow_state
                         WHERE auth_code = $1 LIMIT 1"
                    ))
                    .bind(auth_code)
                    .fetch_optional(&pg.pool)
                    .await?;
                    let Some(row) = row else {
                        return Ok(None);
                    };
                    let id: Uuid = row.try_get("id")?;
                    Ok(Some((id, flow_state_from_row(&row)?)))
                })
                .await
            }
        }
    }

    /// Issue the PKCE session in one transaction: `last_sign_in_at`, a new
    /// `aal1` session with a legacy refresh token and the flow's AMR claim,
    /// a `login` audit entry with `provider_type`, the custom access token
    /// hook, and the flow state delete.
    ///
    /// The flow state row is deleted first. `Ok(None)` means another request
    /// already redeemed it (or the user is gone), and nothing is written.
    ///
    /// # Errors
    ///
    /// [`StoreError`] when the database is missing, a write fails, or the
    /// hook rejects the session; the transaction is rolled back then.
    pub async fn redeem_pkce_flow(
        &self,
        redemption: &PkceRedemption<'_>,
        hook: Option<&TokenHookCall<'_>>,
    ) -> Result<Option<IssuedSession>, StoreError> {
        let traits = json!({ "provider_type": redemption.provider_type });
        match &self.inner {
            BackendKind::None => Err(StoreError::Unavailable),
            BackendKind::Memory(db_handle) => {
                let mut db = db_handle.lock().await;
                if !db.users.contains_key(&redemption.user_id)
                    || db.flow_states.remove(&redemption.flow_id).is_none()
                {
                    return Ok(None);
                }
                let Some(user) = db.users.get_mut(&redemption.user_id) else {
                    return Ok(None);
                };
                let now = SystemTime::now();
                user.last_sign_in_at = Some(now);
                let mut issued = grant_session(user, now);
                issued.amr_method = redemption.amr_method.into();
                reload_confirmed_at(&mut issued.user);
                remember_audit(&issued.user, "login", "account", Some(traits));
                db.track(&issued);
                drop(db);
                customize_memory(db_handle, hook, &mut issued).await?;
                Ok(Some(issued))
            }
            BackendKind::Postgres(pg) => {
                timed(async move {
                    let mut tx = pg.pool.begin().await?;
                    let deleted = sqlx::query("DELETE FROM auth.flow_state WHERE id = $1::uuid")
                        .bind(redemption.flow_id)
                        .execute(&mut *tx)
                        .await?
                        .rows_affected();
                    if deleted == 0 {
                        return Ok(None);
                    }
                    let Some(mut user) = find_user_by_id(&mut tx, redemption.user_id).await? else {
                        return Ok(None);
                    };
                    let now = SystemTime::now();
                    user.last_sign_in_at = Some(now);
                    sqlx::query(
                        "UPDATE auth.users SET last_sign_in_at = $1::timestamptz
                         WHERE instance_id = $2::uuid AND id = $3::uuid",
                    )
                    .bind(ts(now))
                    .bind(Uuid::nil())
                    .bind(redemption.user_id)
                    .execute(&mut *tx)
                    .await?;
                    insert_audit(&mut tx, &user, "login", "account", Some(traits)).await?;
                    let mut issued = grant_session(&mut user, now);
                    issued.amr_method = redemption.amr_method.into();
                    insert_session_rows(&mut tx, &issued)
                        .await
                        .map_err(write_to_store)?;
                    customize_pg(&mut tx, hook, &mut issued).await?;
                    tx.commit().await?;
                    Ok(Some(issued))
                })
                .await
            }
        }
    }
}
