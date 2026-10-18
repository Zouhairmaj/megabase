//! `postgres_changes`: polls the tenant's write-ahead log through a temporary
//! wal2json slot and pushes each authorized change to the channels that
//! subscribed to it.

// Ported from supabase/realtime lib/extensions/postgres_cdc_rls/replication_poller.ex,
// lib/extensions/postgres_cdc_rls/replications.ex,
// lib/extensions/postgres_cdc_rls/message_dispatcher.ex,
// lib/extensions/postgres_cdc_rls/db_settings.ex and lib/realtime/adapters/changes.ex
// (Apache-2.0), pin v2.143.3.

use std::collections::HashSet;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use std::borrow::Cow;

use megabase_core::{db_role, ApiRoles, ROLE_EXISTS_SQL};
use serde_json::Value;
use sqlx::postgres::PgConnection;
use sqlx::Row;
use uuid::Uuid;

use crate::hub::{ConnId, Outbox};
use crate::protocol::{Frame, Serializer, Wire};
use crate::subscriptions;

/// `DbSettings.default/0` `publication`.
pub(crate) const PUBLICATION: &str = "supabase_realtime";
/// `DbSettings.default/0` `slot_name`.
const SLOT_NAME: &str = "supabase_realtime_replication_slot";
/// `DbSettings.default/0` `poll_max_changes`.
const MAX_CHANGES: i32 = 100;
/// The self-hosted seed's `poll_max_record_bytes`.
const MAX_RECORD_BYTES: i32 = 1_048_576;
/// The self-hosted seed's `poll_interval_ms`.
const POLL_INTERVAL_MS: u64 = 100;
/// `@idle_multiplier`.
const IDLE_MULTIPLIER: u64 = 5;
/// `@max_retries`.
const MAX_RETRIES: u32 = 6;
/// `@check_oids_interval`.
const CHECK_OIDS_INTERVAL: Duration = Duration::from_secs(60);

/// One channel's fastlane subscription to `realtime:postgres:<tenant>`
/// (`{:subscriber_fastlane, transport_pid, serializer, ids, topic, is_new_api}`).
#[derive(Debug)]
pub(crate) struct Subscriber {
    pub conn: ConnId,
    /// The channel's join topic (`realtime:<name>`).
    pub topic: String,
    pub serializer: Serializer,
    pub outbox: Outbox,
    pub is_new_api: bool,
    /// `{subscription_id, id}` per `postgres_changes` entry, in join order.
    pub ids: Vec<(Uuid, u32)>,
    pub auth: Auth,
}

/// The claims a channel's subscription rows must carry, and whether the rows
/// carry them yet. `apply_rls` runs under the stored claims, so the
/// dispatcher only pushes to a channel whose rows hold its latest claims: a
/// refreshed token suspends delivery until its claims are stored.
#[derive(Debug)]
pub(crate) struct Auth {
    /// The verified claims of the channel's current access token.
    claims: Value,
    /// Bumped by every token refresh.
    generation: u64,
    /// The generation whose claims the rows hold.
    stored: Option<u64>,
    /// Serializes the channel's row writes, so a slower write with older
    /// claims never lands after a newer one.
    writes: Arc<tokio::sync::Mutex<()>>,
}

impl Auth {
    /// `claims`, not stored yet.
    pub(crate) fn new(claims: Value) -> Self {
        Self {
            claims,
            generation: 0,
            stored: None,
            writes: Arc::default(),
        }
    }

    /// Whether the rows hold the latest claims.
    fn current(&self) -> bool {
        self.stored == Some(self.generation)
    }
}

/// A joined channel's `pg_change_params` waiting for `:postgres_subscribe`.
#[derive(Debug, Clone)]
pub(crate) struct Pending {
    pub conn: ConnId,
    pub topic: String,
    pub join_ref: Value,
    pub serializer: Serializer,
    pub outbox: Outbox,
    /// Same order as `params`.
    pub ids: Vec<(Uuid, u32)>,
    pub params: Vec<Value>,
}

impl Pending {
    /// `push_system_message("postgres_changes", socket, status, message, channel_name)`.
    fn system(&self, status: &str, message: &str) {
        let channel = self.topic.strip_prefix("realtime:").unwrap_or(&self.topic);
        let frame = Frame::push(
            self.join_ref.clone(),
            &self.topic,
            "system",
            &serde_json::json!({
                "extension": "postgres_changes",
                "status": status,
                "message": message,
                "channel": channel,
            }),
        );
        self.outbox.push(Wire::Text(self.serializer.encode(frame)));
    }
}

/// One decoded change (`generate_record/1`): its type, its JSON encoding and
/// the subscriptions allowed to see it.
#[derive(Debug, PartialEq)]
pub(crate) struct Change {
    pub kind: String,
    pub payload: String,
    pub subscription_ids: HashSet<Uuid>,
}

/// The CDC extension of the self-hosted tenant: its database and the
/// channels subscribed to its changes.
#[derive(Debug)]
pub(crate) struct Cdc {
    pool: sqlx::PgPool,
    slot_name: String,
    /// `SLOT_NAME_SUFFIX` is set: the cluster may host another server.
    slot_suffixed: bool,
    subscribers: Mutex<Vec<Subscriber>>,
    roles: ApiRoles,
}

/// `claims ->> 'role'`: JSON null and a missing key are SQL NULL. A string
/// is that text. Any other JSON value is its text form, which is not NULL.
fn claim_role_text(claims: &Value) -> Option<Cow<'_, str>> {
    match claims.get("role") {
        None | Some(Value::Null) => None,
        Some(Value::String(role)) => Some(Cow::Borrowed(role.as_str())),
        Some(other) => Some(Cow::Owned(other.to_string())),
    }
}

fn fatal_insert(error: &subscriptions::CreateError) -> (String, bool) {
    (error.to_string(), error.fatal())
}

impl Cdc {
    /// The extension for `pool`; `slot_name_suffix` is `SLOT_NAME_SUFFIX`.
    pub(crate) fn new(pool: sqlx::PgPool, slot_name_suffix: Option<&str>, roles: ApiRoles) -> Self {
        let slot_name = match slot_name_suffix {
            Some(suffix) => format!("{SLOT_NAME}_{suffix}"),
            None => SLOT_NAME.to_owned(),
        };
        Self {
            pool,
            slot_name,
            slot_suffixed: slot_name_suffix.is_some(),
            subscribers: Mutex::new(Vec::new()),
            roles,
        }
    }

    fn subscribers(&self) -> MutexGuard<'_, Vec<Subscriber>> {
        self.subscribers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// `Endpoint.subscribe("realtime:postgres:" <> tenant, metadata)`.
    pub(crate) fn subscribe(&self, subscriber: Subscriber) {
        let mut subscribers = self.subscribers();
        subscribers.retain(|s| !(s.conn == subscriber.conn && s.topic == subscriber.topic));
        subscribers.push(subscriber);
    }

    /// The channel of `conn` on `topic` stopped: its fastlane subscription
    /// ends. Returns its subscription ids, which `SubscriptionManager`
    /// deletes once the channel process is down.
    pub(crate) fn unsubscribe(&self, conn: ConnId, topic: &str) -> Vec<Uuid> {
        let mut subscribers = self.subscribers();
        let mut ids = Vec::new();
        subscribers.retain(|s| {
            let gone = s.conn == conn && s.topic == topic;
            if gone {
                ids.extend(s.ids.iter().map(|(id, _)| *id));
            }
            !gone
        });
        ids
    }

    /// Whether the channel of `conn` on `topic` that holds subscription `id`
    /// is still subscribed.
    fn holds(&self, conn: ConnId, topic: &str, id: Uuid) -> bool {
        self.subscribers()
            .iter()
            .any(|s| s.conn == conn && s.topic == topic && s.ids.iter().any(|(i, _)| *i == id))
    }

    /// `SubscriptionManager` deleting a stopped channel's subscriptions.
    pub(crate) fn forget(self: &Arc<Self>, ids: Vec<Uuid>) {
        if ids.is_empty() {
            return;
        }
        let cdc = Arc::clone(self);
        tokio::spawn(async move {
            // A failed delete leaves rows `SubscriptionManager` would also
            // leave behind until its next cleanup; nothing to report.
            let _deleted = subscriptions::delete(&cdc.pool, &ids).await;
        });
    }

    /// `handle_info(:postgres_subscribe, …)`: inserts the channel's
    /// subscriptions, then pushes `system` "Subscribed to PostgreSQL", or the
    /// error and, unless it is fatal, tries again 5 to 10 s later. Stops once
    /// the channel is gone.
    pub(crate) async fn postgres_subscribe(self: Arc<Self>, pending: Pending) {
        let Some(&(first, _)) = pending.ids.first() else {
            return;
        };
        loop {
            if !self.holds(pending.conn, &pending.topic, first) {
                return;
            }
            match self.store(&pending).await {
                Ok(true) => {
                    pending.system("ok", "Subscribed to PostgreSQL");
                    return;
                }
                Ok(false) => {
                    // The channel stopped while the rows were inserted.
                    self.forget(pending.ids.iter().map(|(id, _)| *id).collect());
                    return;
                }
                Err((message, fatal)) => {
                    pending.system("error", &message);
                    if fatal {
                        return;
                    }
                }
            }
            tokio::time::sleep(resubscribe_wait()).await;
        }
    }

    /// A token refresh replaced the claims of the channel of `conn` on
    /// `topic` (`None`: the new token yields no claims). Delivery to the
    /// channel stops at once and resumes only once
    /// [`Cdc::refresh_claims`] stores the new claims.
    pub(crate) fn set_claims(&self, conn: ConnId, topic: &str, claims: Option<Value>) {
        for subscriber in self.subscribers().iter_mut() {
            if subscriber.conn == conn && subscriber.topic == topic {
                subscriber.auth.generation += 1;
                if let Some(claims) = claims {
                    subscriber.auth.claims = claims;
                }
                return;
            }
        }
    }

    /// Stores the channel's subscriptions again with the claims of its
    /// refreshed token (the insert's conflict path updates `claims`). Fails
    /// closed: until a write succeeds the channel gets no changes, and a
    /// failed write is logged and tried again 5 to 10 s later. A fatal error
    /// keeps the channel suspended. Stops once the channel is gone.
    pub(crate) async fn refresh_claims(self: Arc<Self>, pending: Pending) {
        let Some(&(first, _)) = pending.ids.first() else {
            return;
        };
        loop {
            if !self.holds(pending.conn, &pending.topic, first) {
                return;
            }
            match self.store(&pending).await {
                Ok(true) => return,
                Ok(false) => {
                    // The channel stopped while the rows were stored.
                    self.forget(pending.ids.iter().map(|(id, _)| *id).collect());
                    return;
                }
                Err((message, fatal)) => {
                    tracing::warn!(
                        %message,
                        fatal,
                        "could not refresh the claims of realtime subscriptions; changes are held back"
                    );
                    if fatal {
                        return;
                    }
                }
            }
            tokio::time::sleep(resubscribe_wait()).await;
        }
    }

    /// Runs `f` on the channel of `pending` that still holds its first
    /// subscription id.
    fn with_subscriber<T>(
        &self,
        pending: &Pending,
        f: impl FnOnce(&mut Subscriber) -> T,
    ) -> Option<T> {
        let &(first, _) = pending.ids.first()?;
        self.subscribers()
            .iter_mut()
            .find(|s| {
                s.conn == pending.conn
                    && s.topic == pending.topic
                    && s.ids.iter().any(|(i, _)| *i == first)
            })
            .map(f)
    }

    /// Writes the channel's rows with its latest claims and records which
    /// claims they hold. `Ok(false)`: the channel is gone.
    async fn store(&self, pending: &Pending) -> Result<bool, (String, bool)> {
        let Some(writes) = self.with_subscriber(pending, |s| Arc::clone(&s.auth.writes)) else {
            return Ok(false);
        };
        let _write = writes.lock().await;
        // Read under the write lock: a refresh that lands after this read
        // bumps the generation, so this write cannot resume delivery.
        let Some((generation, claims)) =
            self.with_subscriber(pending, |s| (s.auth.generation, s.auth.claims.clone()))
        else {
            return Ok(false);
        };
        self.subscribe_attempt(pending, &claims).await?;
        Ok(self
            .with_subscriber(pending, |s| s.auth.stored = Some(generation))
            .is_some())
    }

    /// Rejects a claims `role` that [`db_role`] does not allow, before the
    /// subscription rows are inserted. `apply_rls` reads `claims_role` from
    /// those rows, so a rejected role never reaches `set_config`.
    ///
    /// The reported error is the one the insert's generated column would
    /// raise: `realtime.to_regrole((claims ->> 'role'))` (`role_name::regrole`)
    /// is `42704` for an unknown name, and the `NOT NULL` `claims_role`
    /// column is `23502` when `->>` yields NULL.
    async fn ensure_claims_role(
        &self,
        claims: &Value,
        params: &subscriptions::SubscriptionParams,
    ) -> Result<(), (String, bool)> {
        let Some(role) = claim_role_text(claims) else {
            return Err(fatal_insert(&subscriptions::null_claims_role(params)));
        };
        if db_role(role.as_ref(), &self.roles).is_ok() {
            return Ok(());
        }
        let exists = match sqlx::query_scalar::<_, bool>(ROLE_EXISTS_SQL)
            .bind(role.as_ref())
            .fetch_one(&self.pool)
            .await
        {
            Ok(exists) => exists,
            Err(error) => {
                let error = subscriptions::CreateError::Database(error);
                let fatal = error.fatal();
                return Err((error.to_string(), fatal));
            }
        };
        let error = if exists {
            subscriptions::unconfigured_existing_role(role.as_ref(), params)
        } else {
            subscriptions::undefined_claims_role(role.as_ref(), params)
        };
        Err(fatal_insert(&error))
    }

    /// `postgres_subscribe_attempt/2`: the error message and whether it is
    /// one of `@postgres_subscribe_fatal_reasons`.
    async fn subscribe_attempt(
        &self,
        pending: &Pending,
        claims: &Value,
    ) -> Result<(), (String, bool)> {
        let mut list = Vec::with_capacity(pending.params.len());
        for ((id, _), params) in pending.ids.iter().zip(&pending.params) {
            let params = subscriptions::parse_subscription_params(params)
                .map_err(|error| (error.to_string(), true))?;
            list.push(subscriptions::NewSubscription {
                id: *id,
                claims: claims.clone(),
                params,
            });
        }
        // `subscription_list/1` prepends, so rows go in reverse join order.
        list.reverse();
        if let Some(params) = list.first().map(|sub| &sub.params) {
            self.ensure_claims_role(claims, params).await?;
        }
        subscriptions::create(&self.pool, PUBLICATION, &list)
            .await
            .map_err(|error| (error.to_string(), error.fatal()))
    }

    /// `MessageDispatcher.dispatch/3`: a push to every channel with at least
    /// one id in `change.subscription_ids`. The new API pushes
    /// `postgres_changes` with `{"data", "ids"}`; the old API pushes the
    /// change itself with the change type as the event.
    // megabase:unit realtime:server-event:postgres_changes
    pub(crate) fn dispatch(&self, change: &Change) {
        for subscriber in self.subscribers().iter() {
            // The rows' claims are not the channel's latest: hold back.
            if !subscriber.auth.current() {
                continue;
            }
            // `for … reduce: [] … [id | acc]` keeps the matches in reverse order.
            let valid: Vec<u32> = subscriber
                .ids
                .iter()
                .rev()
                .filter(|(bin_id, _)| change.subscription_ids.contains(bin_id))
                .map(|(_, id)| *id)
                .collect();
            if valid.is_empty() {
                continue;
            }
            let (event, payload) = if subscriber.is_new_api {
                let ids = Value::from(valid).to_string();
                (
                    "postgres_changes",
                    format!(r#"{{"data":{},"ids":{ids}}}"#, change.payload),
                )
            } else {
                (change.kind.as_str(), change.payload.clone())
            };
            let wire = fastlane(subscriber.serializer, &subscriber.topic, event, &payload);
            // A full outbox evicts the socket, as a full topic push does.
            subscriber.outbox.push(wire);
        }
    }

    /// Runs the `ReplicationPoller`. Upstream's supervisor restarts it after
    /// it gives up, so it never returns: after `@max_retries` consecutive
    /// failures it keeps retrying at the longest backoff and logs each one.
    pub(crate) async fn run(self: Arc<Self>) {
        // Rows of a previous run would be checked by `apply_rls` for every
        // change. With a slot suffix another server may own rows here.
        if self.slot_suffixed {
            tracing::debug!("keeping realtime.subscription rows: SLOT_NAME_SUFFIX is set");
        } else if let Err(error) = sqlx::query("delete from realtime.subscription")
            .execute(&self.pool)
            .await
        {
            tracing::warn!(%error, "could not clear stale realtime subscriptions");
        }
        let mut retries = 0;
        loop {
            let Err(error) = self.poll_slot(&mut retries).await else {
                return;
            };
            retries = retries.saturating_add(1);
            if retries >= MAX_RETRIES {
                tracing::warn!(%error, retries, "realtime replication poller keeps failing; retrying");
            }
            tokio::time::sleep(backoff(retries)).await;
        }
    }

    /// `prepare_replication/1` and the `:poll` loop on one connection: the
    /// slot is temporary, so it lives as long as that connection.
    async fn poll_slot(&self, retries: &mut u32) -> Result<(), sqlx::Error> {
        let mut conn = self.pool.acquire().await?.detach();
        loop {
            // An empty publication creates no slot; `:check_oids` waits.
            while !publication_has_tables(&mut conn).await? {
                tokio::time::sleep(CHECK_OIDS_INTERVAL).await;
            }
            prepare_replication(&mut conn, &self.slot_name).await?;
            *retries = 0;
            let mut checked = tokio::time::Instant::now();
            loop {
                let (processed, slot_changes) = self.poll(&mut conn).await?;
                let wait = if processed > 0 {
                    Duration::ZERO
                } else if slot_changes > 0 {
                    Duration::from_millis(POLL_INTERVAL_MS + jitter())
                } else {
                    Duration::from_millis(POLL_INTERVAL_MS * IDLE_MULTIPLIER)
                };
                tokio::time::sleep(wait).await;
                if checked.elapsed() >= CHECK_OIDS_INTERVAL {
                    checked = tokio::time::Instant::now();
                    if !publication_has_tables(&mut conn).await? {
                        // Tables vanished: drop the slot so it retains no WAL.
                        drop_replication_slot(&mut conn, &self.slot_name).await?;
                        break;
                    }
                }
            }
        }
    }

    /// One `:poll`: drains the slot, dispatches each change and returns
    /// `{processed_count, slot_changes_count}`.
    async fn poll(&self, conn: &mut PgConnection) -> Result<(usize, i64), sqlx::Error> {
        let rows = sqlx::query(LIST_CHANGES)
            .bind(PUBLICATION)
            .bind(&self.slot_name)
            .bind(MAX_CHANGES)
            .bind(MAX_RECORD_BYTES)
            .fetch_all(&mut *conn)
            .await?;
        let slot_changes = rows
            .first()
            .and_then(|row| row.try_get::<Option<i64>, _>(9).ok().flatten())
            .unwrap_or(0);
        let mut processed = 0;
        for row in &rows {
            let Some(row) = ChangeRow::from_row(row)? else {
                // The sentinel row: no change for any subscriber.
                continue;
            };
            processed += 1;
            if let Some(change) = row.generate_record() {
                self.dispatch(&change);
            }
        }
        Ok((processed, slot_changes))
    }
}

/// `Replications.list_changes/2` (no synchronous standby).
const LIST_CHANGES: &str = "SELECT wal->>'type' as type,
       wal->>'schema' as schema,
       wal->>'table' as table,
       COALESCE(wal->>'columns', '[]') as columns,
       COALESCE(wal->>'record', '{}') as record,
       COALESCE(wal->>'old_record', '{}') as old_record,
       wal->>'commit_timestamp' as commit_timestamp,
       subscription_ids,
       errors,
       slot_changes_count
FROM realtime.list_changes($1, $2, $3, $4)";

/// `Subscriptions.fetch_publication_tables/2`, reduced to whether the
/// publication has a table.
async fn publication_has_tables(conn: &mut PgConnection) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar("select exists (select 1 from pg_publication_tables where pubname = $1)")
        .bind(PUBLICATION)
        .fetch_one(conn)
        .await
}

/// `Replications.prepare_replication/2`.
async fn prepare_replication(conn: &mut PgConnection, slot_name: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        "select
        case when not exists (
          select 1
          from pg_replication_slots
          where slot_name = $1
        )
        then (
          select 1 from pg_create_logical_replication_slot(slot_name => $1, plugin => 'wal2json', temporary => true)
        )
        else 1
        end;",
    )
    .bind(slot_name)
    .execute(conn)
    .await?;
    Ok(())
}

/// `Replications.drop_replication_slot/2`; a missing slot is not an error.
async fn drop_replication_slot(
    conn: &mut PgConnection,
    slot_name: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "select pg_drop_replication_slot(slot_name) from pg_replication_slots where slot_name = $1",
    )
    .bind(slot_name)
    .execute(conn)
    .await?;
    Ok(())
}

/// `DBConnection.Backoff` `:rand_exp` between 100 ms and 5 s, simplified to
/// a doubling delay.
fn backoff(retries: u32) -> Duration {
    Duration::from_millis((100u64 << retries.min(6)).min(5_000))
}

/// `Enum.random(50..100)`.
fn jitter() -> u64 {
    50 + u64::from(Uuid::new_v4().as_bytes()[0]) % 51
}

/// `postgres_subscribe(5, 10)`: 5 to 10 s.
fn resubscribe_wait() -> Duration {
    let bytes = Uuid::new_v4();
    let random = u16::from_be_bytes([bytes.as_bytes()[0], bytes.as_bytes()[1]]);
    Duration::from_millis(5_000 + u64::from(random) % 5_001)
}

/// One `realtime.list_changes` row.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct ChangeRow {
    pub kind: String,
    pub schema: Option<String>,
    pub table: Option<String>,
    pub columns: String,
    pub record: String,
    pub old_record: String,
    pub commit_timestamp: Option<String>,
    pub subscription_ids: Vec<Uuid>,
    pub errors: Option<Vec<String>>,
}

impl ChangeRow {
    /// The row, or `None` for the sentinel row (`wal` is null).
    fn from_row(row: &sqlx::postgres::PgRow) -> Result<Option<Self>, sqlx::Error> {
        let Some(kind) = row.try_get::<Option<String>, _>(0)? else {
            return Ok(None);
        };
        Ok(Some(Self {
            kind,
            schema: row.try_get(1)?,
            table: row.try_get(2)?,
            columns: row.try_get::<Option<String>, _>(3)?.unwrap_or_default(),
            record: row.try_get::<Option<String>, _>(4)?.unwrap_or_default(),
            old_record: row.try_get::<Option<String>, _>(5)?.unwrap_or_default(),
            commit_timestamp: row.try_get(6)?,
            subscription_ids: row.try_get::<Option<Vec<Uuid>>, _>(7)?.unwrap_or_default(),
            errors: row.try_get(8)?,
        }))
    }

    /// `generate_record/1` and `Jason.encode!/1`: `NewRecord`,
    /// `UpdatedRecord` or `DeletedRecord`, keys sorted as Jason encodes a
    /// derived struct, `columns` and the records spliced in as fragments.
    /// Any other type is dropped.
    pub(crate) fn generate_record(&self) -> Option<Change> {
        let (record, old_record) = match self.kind.as_str() {
            "INSERT" => (Some(&self.record), None),
            "UPDATE" => (Some(&self.record), Some(&self.old_record)),
            "DELETE" => (None, Some(&self.old_record)),
            _ => return None,
        };
        let string = |value: Option<&String>| {
            value.map_or_else(
                || "null".to_owned(),
                |s| Value::from(s.as_str()).to_string(),
            )
        };
        // `convert_errors/1`: a non-empty list, else `nil`.
        let errors = match &self.errors {
            Some(errors) if !errors.is_empty() => Value::from(errors.clone()).to_string(),
            _ => "null".to_owned(),
        };
        let mut payload = format!(
            r#"{{"columns":{},"commit_timestamp":{},"errors":{errors}"#,
            self.columns,
            string(self.commit_timestamp.as_ref()),
        );
        if let Some(old_record) = old_record {
            payload.push_str(&format!(r#","old_record":{old_record}"#));
        }
        if let Some(record) = record {
            payload.push_str(&format!(r#","record":{record}"#));
        }
        payload.push_str(&format!(
            r#","schema":{},"table":{},"type":{}}}"#,
            string(self.schema.as_ref()),
            string(self.table.as_ref()),
            string(Some(&self.kind)),
        ));
        Some(Change {
            kind: self.kind.clone(),
            payload,
            subscription_ids: self.subscription_ids.iter().copied().collect(),
        })
    }
}

/// `serializer.fastlane!/1` for `%Broadcast{topic, event, payload}` whose
/// payload is the pre-encoded JSON `payload`.
fn fastlane(serializer: Serializer, topic: &str, event: &str, payload: &str) -> Wire {
    let topic = Value::from(topic).to_string();
    let event = Value::from(event).to_string();
    Wire::Text(match serializer {
        Serializer::V2 => format!("[null,null,{topic},{event},{payload}]"),
        Serializer::V1 => {
            format!(r#"{{"event":{event},"payload":{payload},"ref":null,"topic":{topic}}}"#)
        }
    })
}

/// The `id` the join reply and every push carry for one `postgres_changes`
/// entry. Upstream uses `:erlang.phash2(params)`, a value in `0..2^27`;
/// Megabase hashes the entry's JSON encoding (FNV-1a) into the same range.
pub(crate) fn params_id(params: &Value) -> u32 {
    let mut hash: u32 = 0x811c_9dc5;
    for byte in params.to_string().bytes() {
        hash ^= u32::from(byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash & ((1 << 27) - 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hub::OUTBOX_CAPACITY;
    use serde_json::json;

    fn lazy_pool() -> sqlx::PgPool {
        sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://megabase@127.0.0.1:1/megabase")
            .unwrap()
    }

    fn row(kind: &str) -> ChangeRow {
        ChangeRow {
            kind: kind.to_owned(),
            schema: Some("public".to_owned()),
            table: Some("todos".to_owned()),
            columns: r#"[{"name": "id", "type": "int8"}]"#.to_owned(),
            record: r#"{"id": 2}"#.to_owned(),
            old_record: r#"{"id": 1}"#.to_owned(),
            commit_timestamp: Some("2026-10-16T17:00:00.000Z".to_owned()),
            subscription_ids: vec![Uuid::nil()],
            errors: Some(Vec::new()),
        }
    }

    #[test]
    fn records_encode_like_jason() {
        assert_eq!(
            row("INSERT").generate_record().unwrap().payload,
            r#"{"columns":[{"name": "id", "type": "int8"}],"commit_timestamp":"2026-10-16T17:00:00.000Z","errors":null,"record":{"id": 2},"schema":"public","table":"todos","type":"INSERT"}"#
        );
        assert_eq!(
            row("UPDATE").generate_record().unwrap().payload,
            r#"{"columns":[{"name": "id", "type": "int8"}],"commit_timestamp":"2026-10-16T17:00:00.000Z","errors":null,"old_record":{"id": 1},"record":{"id": 2},"schema":"public","table":"todos","type":"UPDATE"}"#
        );
        let mut delete = row("DELETE");
        delete.errors = Some(vec!["Error 413: Payload Too Large".to_owned()]);
        assert_eq!(
            delete.generate_record().unwrap().payload,
            r#"{"columns":[{"name": "id", "type": "int8"}],"commit_timestamp":"2026-10-16T17:00:00.000Z","errors":["Error 413: Payload Too Large"],"old_record":{"id": 1},"schema":"public","table":"todos","type":"DELETE"}"#
        );
        assert_eq!(row("TRUNCATE").generate_record(), None);
    }

    fn drain(rx: &mut tokio::sync::mpsc::Receiver<Wire>) -> Vec<String> {
        let mut out = Vec::new();
        while let Ok(Wire::Text(text)) = rx.try_recv() {
            out.push(text);
        }
        out
    }

    #[tokio::test]
    async fn dispatch_pushes_matching_ids_per_api() {
        let cdc = Cdc::new(lazy_pool(), None, megabase_core::ApiRoles::default());
        let (a, b, c) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        let (new_box, mut new_rx) = Outbox::new(OUTBOX_CAPACITY);
        let (old_box, mut old_rx) = Outbox::new(OUTBOX_CAPACITY);
        let (other_box, mut other_rx) = Outbox::new(OUTBOX_CAPACITY);
        cdc.subscribe(Subscriber {
            conn: 1,
            topic: "realtime:room".to_owned(),
            serializer: Serializer::V2,
            outbox: new_box,
            is_new_api: true,
            ids: vec![(a, 11), (b, 22)],
            auth: stored_auth(),
        });
        cdc.subscribe(Subscriber {
            conn: 2,
            topic: "realtime:public:todos".to_owned(),
            serializer: Serializer::V1,
            outbox: old_box,
            is_new_api: false,
            ids: vec![(a, 33)],
            auth: stored_auth(),
        });
        cdc.subscribe(Subscriber {
            conn: 3,
            topic: "realtime:room".to_owned(),
            serializer: Serializer::V2,
            outbox: other_box,
            is_new_api: true,
            ids: vec![(c, 44)],
            auth: stored_auth(),
        });
        let change = Change {
            kind: "INSERT".to_owned(),
            payload: r#"{"type":"INSERT"}"#.to_owned(),
            subscription_ids: HashSet::from([a, b]),
        };
        cdc.dispatch(&change);
        assert_eq!(
            drain(&mut new_rx),
            [
                r#"[null,null,"realtime:room","postgres_changes",{"data":{"type":"INSERT"},"ids":[22,11]}]"#
            ]
        );
        assert_eq!(
            drain(&mut old_rx),
            [
                r#"{"event":"INSERT","payload":{"type":"INSERT"},"ref":null,"topic":"realtime:public:todos"}"#
            ]
        );
        assert!(drain(&mut other_rx).is_empty());

        assert_eq!(cdc.unsubscribe(1, "realtime:room"), [a, b]);
        cdc.dispatch(&change);
        assert!(drain(&mut new_rx).is_empty());
        assert_eq!(drain(&mut old_rx).len(), 1);
    }

    /// Claims whose rows are already stored.
    fn stored_auth() -> Auth {
        let mut auth = Auth::new(json!({"role": "authenticated"}));
        auth.stored = Some(0);
        auth
    }

    #[tokio::test]
    async fn a_failed_claims_refresh_holds_changes_back() {
        // Nothing listens on port 1: every write fails, as during an outage.
        let pool = sqlx::postgres::PgPoolOptions::new()
            .acquire_timeout(Duration::from_millis(200))
            .connect_lazy("postgres://megabase@127.0.0.1:1/megabase")
            .unwrap();
        let cdc = Cdc::new(pool, None, megabase_core::ApiRoles::default());
        let id = Uuid::new_v4();
        let (outbox, mut rx) = Outbox::new(OUTBOX_CAPACITY);
        cdc.subscribe(Subscriber {
            conn: 1,
            topic: "realtime:room".to_owned(),
            serializer: Serializer::V2,
            outbox: outbox.clone(),
            is_new_api: true,
            ids: vec![(id, 11)],
            auth: stored_auth(),
        });
        let change = Change {
            kind: "INSERT".to_owned(),
            payload: r#"{"type":"INSERT"}"#.to_owned(),
            subscription_ids: HashSet::from([id]),
        };
        cdc.dispatch(&change);
        assert_eq!(drain(&mut rx).len(), 1);

        // The refreshed token suspends delivery before anything is written.
        cdc.set_claims(1, "realtime:room", Some(json!({"role": "anon"})));
        cdc.dispatch(&change);
        assert!(drain(&mut rx).is_empty());

        let pending = Pending {
            conn: 1,
            topic: "realtime:room".to_owned(),
            join_ref: Value::Null,
            serializer: Serializer::V2,
            outbox,
            ids: vec![(id, 11)],
            params: vec![json!({"event": "*", "schema": "public"})],
        };
        let Err((_, fatal)) = cdc.store(&pending).await else {
            panic!("the write must fail without a database");
        };
        assert!(!fatal, "an unreachable database is retried");
        // The rows still hold the old claims, so the channel stays suspended.
        cdc.dispatch(&change);
        assert!(drain(&mut rx).is_empty());

        // A token without claims keeps it suspended too.
        cdc.set_claims(1, "realtime:room", None);
        cdc.dispatch(&change);
        assert!(drain(&mut rx).is_empty());
    }

    #[tokio::test]
    async fn a_stored_refresh_resumes_only_at_the_latest_generation() {
        let cdc = Cdc::new(lazy_pool(), None, megabase_core::ApiRoles::default());
        let id = Uuid::new_v4();
        let (outbox, mut rx) = Outbox::new(OUTBOX_CAPACITY);
        cdc.subscribe(Subscriber {
            conn: 1,
            topic: "realtime:room".to_owned(),
            serializer: Serializer::V2,
            outbox,
            is_new_api: true,
            ids: vec![(id, 11)],
            auth: stored_auth(),
        });
        let change = Change {
            kind: "INSERT".to_owned(),
            payload: r#"{"type":"INSERT"}"#.to_owned(),
            subscription_ids: HashSet::from([id]),
        };
        cdc.set_claims(1, "realtime:room", Some(json!({"role": "anon"})));
        cdc.set_claims(1, "realtime:room", Some(json!({"role": "authenticated"})));
        // A write that read generation 1 lands after the second refresh.
        cdc.subscribers()[0].auth.stored = Some(1);
        cdc.dispatch(&change);
        assert!(drain(&mut rx).is_empty());
        cdc.subscribers()[0].auth.stored = Some(2);
        cdc.dispatch(&change);
        assert_eq!(drain(&mut rx).len(), 1);
    }

    #[test]
    fn params_ids_are_stable_and_in_phash2_range() {
        let params = json!({"event": "*", "schema": "public"});
        assert_eq!(params_id(&params), params_id(&params.clone()));
        assert!(params_id(&params) < 1 << 27);
        assert_ne!(params_id(&params), params_id(&json!({"event": "INSERT"})));
    }

    #[test]
    fn slot_name_takes_the_suffix() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let _guard = rt.enter();
        assert_eq!(
            Cdc::new(lazy_pool(), None, megabase_core::ApiRoles::default()).slot_name,
            SLOT_NAME
        );
        assert_eq!(
            Cdc::new(lazy_pool(), Some("dev"), megabase_core::ApiRoles::default()).slot_name,
            "supabase_realtime_replication_slot_dev"
        );
        assert_eq!(backoff(1), Duration::from_millis(200));
        assert_eq!(backoff(6), Duration::from_millis(5_000));
        assert!((50..=100).contains(&jitter()));
    }
}
