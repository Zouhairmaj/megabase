# Auth: database (issue 12)

Issue #12 ports ten Auth SQL tables. HTTP `/auth/v1` is unchanged (501).
These objects are the compatibility surface that RLS policies and clients
call inside PostgreSQL.

Namespace is always `auth` (GoTrue `{{ index .Options "Namespace" }}`).
Pin: `vendor/auth` `v2.197.0` (`4eee58f296d9698a1c2c0ae14d7a0b379c7622d3`),
MIT. Desired state is the **final** definition after later migrations, not
the first `CREATE`. Install is idempotent (`IF NOT EXISTS` / `ADD COLUMN IF
NOT EXISTS`) so it is safe against a database the official stack already
migrated, and against the `auth.users` / `auth.sessions` stubs from issues
#10 / #11.

Parent keys: `auth.sessions.oauth_client_id` references
`auth.oauth_clients(id)` and `auth.saml_relay_states.flow_state_id`
references `auth.flow_state(id)`. This issue creates those parent tables
with only the columns the foreign keys need. Full column lists are other
units; do not mark them implemented here. `auth.aal_level` is created
because `auth.sessions.aal` uses it.

---

## `auth:sql-table:auth.users`

Upstream: `migrations/00_init_auth_schema.up.sql:3`. Later:
phone columns (`20210710035447`); `confirmed_at` generated from
`LEAST(email_confirmed_at, phone_confirmed_at)` (`20210722035447`) after
renaming the original `confirmed_at` to `email_confirmed_at`;
`email_change_token` renamed to `email_change_token_new`
(`20210730183235`); `users_instance_id_email_idx` uses `lower(email)`
(`20220114185221`); `banned_until` (`20220114185340`); reauthentication
columns (`20220323170000`); partial unique token indexes
(`20220429102000`); `is_sso_user` and `users_email_partial_key` after
dropping `users_email_key` (`20221215195500`); `phone` / `phone_change`
become `text` (`20230116124310`); `deleted_at` (`20230116124412`);
`is_anonymous` (`20240214120130`). RLS/grant: `20240612123726` lines 5
and 22.

Installer must **not** `RENAME` `confirmed_at` or `email_change_token`
when the final names already exist.

| Column | Type | Notes |
|---|---|---|
| instance_id | uuid | NULL |
| id | uuid | NOT NULL, PK |
| aud | varchar(255) | NULL |
| role | varchar(255) | NULL |
| email | varchar(255) | NULL; unique only via `users_email_partial_key` where `is_sso_user = false` |
| encrypted_password | varchar(255) | NULL |
| email_confirmed_at | timestamptz | NULL (renamed from `confirmed_at`) |
| invited_at | timestamptz | NULL |
| confirmation_token | varchar(255) | NULL |
| confirmation_sent_at | timestamptz | NULL |
| recovery_token | varchar(255) | NULL |
| recovery_sent_at | timestamptz | NULL |
| email_change_token_new | varchar(255) | NULL (renamed from `email_change_token`) |
| email_change | varchar(255) | NULL |
| email_change_sent_at | timestamptz | NULL |
| last_sign_in_at | timestamptz | NULL |
| raw_app_meta_data | jsonb | NULL |
| raw_user_meta_data | jsonb | NULL |
| is_super_admin | bool | NULL |
| created_at | timestamptz | NULL |
| updated_at | timestamptz | NULL |
| phone | text | NULL, UNIQUE (`users_phone_key`) |
| phone_confirmed_at | timestamptz | NULL |
| phone_change | text | NULL DEFAULT `''` |
| phone_change_token | varchar(255) | NULL DEFAULT `''` |
| phone_change_sent_at | timestamptz | NULL |
| confirmed_at | timestamptz | `GENERATED ALWAYS AS (LEAST (email_confirmed_at, phone_confirmed_at)) STORED` |
| email_change_token_current | varchar(255) | NULL DEFAULT `''` |
| email_change_confirm_status | smallint | DEFAULT 0, CHECK 0–2 |
| banned_until | timestamptz | NULL |
| reauthentication_token | varchar(255) | NULL DEFAULT `''` |
| reauthentication_sent_at | timestamptz | NULL |
| deleted_at | timestamptz | NULL |
| is_sso_user | boolean | NOT NULL DEFAULT false |
| is_anonymous | boolean | NOT NULL DEFAULT false |

Indexes: `users_instance_id_idx`; `users_instance_id_email_idx` on
`(instance_id, lower(email))` (drop the pre-`20220114185221` definition in
schema `auth` only when `indexdef` lacks `lower(`); partial unique token
indexes matching `WHERE … !~ '^[0-9 ]*$'`; `users_email_partial_key`;
`users_is_anonymous_idx`. Comment: `Auth: Stores user login data within a
secure schema.` RLS on. Select grant to `postgres` when that role exists.

---

## `auth:sql-table:auth.sessions`

Upstream create: `migrations/20220811173540_add_sessions_table.up.sql:2`.
Later: `factor_id` / `aal` (`20221003041400`, type `auth.aal_level`);
`sessions_user_id_idx` (`20221020193600`); `user_id_created_at_idx`
(`20221011041400`); `not_after` (`20221114143122`);
`sessions_not_after_idx` (`20230508135423`); `refreshed_at` (timestamp
**without** time zone), `user_agent`, `ip` (`20231027141322`); `tag`
(`20231114161723`); `oauth_client_id` FK → `auth.oauth_clients(id)` ON
DELETE CASCADE (`20250904133000`); `refresh_token_hmac_key` /
`refresh_token_counter` (`20251007112900`); `scopes` with
`sessions_scopes_length` (`20251111201300`). RLS/grant: `20240612123726`
lines 10 and 27.

| Column | Type | Notes |
|---|---|---|
| id | uuid | NOT NULL, PK |
| user_id | uuid | NOT NULL, FK → `auth.users(id)` ON DELETE CASCADE |
| created_at | timestamptz | NULL |
| updated_at | timestamptz | NULL |
| factor_id | uuid | NULL (no FK in this pin) |
| aal | `auth.aal_level` | NULL |
| not_after | timestamptz | NULL |
| refreshed_at | timestamp | without time zone, NULL |
| user_agent | text | NULL |
| ip | inet | NULL |
| tag | text | NULL |
| oauth_client_id | uuid | NULL, FK → `auth.oauth_clients(id)` ON DELETE CASCADE |
| scopes | text | NULL, `char_length <= 4096` |
| refresh_token_hmac_key | text | NULL |
| refresh_token_counter | bigint | NULL |

Comment: `Auth: Stores session data associated to a user.` RLS on.

---

## `auth:sql-table:auth.schema_migrations`

Upstream: `migrations/00_init_auth_schema.up.sql:74`. RLS/grant:
`20240612123726` lines 3 and 20.

| Column | Type | Notes |
|---|---|---|
| version | varchar(255) | NOT NULL, PK |

Comment: `Auth: Manages updates to the auth system.` RLS on.

---

## `auth:sql-table:auth.sso_providers`

Upstream: `migrations/20221021082433_add_saml.up.sql:3`. `disabled`:
`20250717082212`. Pattern index on `resource_id text_pattern_ops` in the
same file. RLS/grant: `20240612123726` lines 11 and 28.

| Column | Type | Notes |
|---|---|---|
| id | uuid | NOT NULL, PK |
| resource_id | text | NULL, check empty-or-null (upstream uses `= null`) |
| created_at | timestamptz | NULL |
| updated_at | timestamptz | NULL |
| disabled | boolean | NULL |

Unique index `sso_providers_resource_id_idx` on `lower(resource_id)`.
Comment as in the add-SAML migration.

---

## `auth:sql-table:auth.sso_domains`

Upstream: `migrations/20221021082433_add_saml.up.sql:17`. RLS/grant:
`20240612123726` lines 12 and 29.

| Column | Type | Notes |
|---|---|---|
| id | uuid | NOT NULL, PK |
| sso_provider_id | uuid | NOT NULL, FK → `auth.sso_providers(id)` ON DELETE CASCADE |
| domain | text | NOT NULL, check non-empty |
| created_at | timestamptz | NULL |
| updated_at | timestamptz | NULL |

Unique index `sso_domains_domain_idx` on `lower(domain)`.

---

## `auth:sql-table:auth.saml_providers`

Upstream: `migrations/20221021082433_add_saml.up.sql:33`.
`name_id_format`: `20240314092811`. RLS/grant: `20240612123726` lines 15
and 32.

| Column | Type | Notes |
|---|---|---|
| id | uuid | NOT NULL, PK |
| sso_provider_id | uuid | NOT NULL, FK → `auth.sso_providers(id)` ON DELETE CASCADE |
| entity_id | text | NOT NULL, UNIQUE, check non-empty |
| metadata_xml | text | NOT NULL, check non-empty |
| metadata_url | text | NULL, check empty-or-null |
| attribute_mapping | jsonb | NULL |
| created_at | timestamptz | NULL |
| updated_at | timestamptz | NULL |
| name_id_format | text | NULL |

---

## `auth:sql-table:auth.saml_relay_states`

Upstream: `migrations/20221021082433_add_saml.up.sql:53`.
`flow_state_id` FK → `auth.flow_state(id)` ON DELETE CASCADE
(`20230818113222`). `from_ip_address` **dropped** (`20240115144230`).
Cleanup index (`20230508135423`). RLS/grant: `20240612123726` lines 7
and 24.

| Column | Type | Notes |
|---|---|---|
| id | uuid | NOT NULL, PK |
| sso_provider_id | uuid | NOT NULL, FK → `auth.sso_providers(id)` ON DELETE CASCADE |
| request_id | text | NOT NULL, check non-empty |
| for_email | text | NULL |
| redirect_to | text | NULL |
| created_at | timestamptz | NULL |
| updated_at | timestamptz | NULL |
| flow_state_id | uuid | NULL, FK → `auth.flow_state(id)` ON DELETE CASCADE |

Do not leave `from_ip_address` on an already-migrated table.

---

## `auth:sql-table:auth.sso_sessions`

Upstream create: `migrations/20221021082433_add_saml.up.sql:72`.
**Dropped** by `migrations/20221215195900_remove_sso_sessions.up.sql`
because the data lives on `auth.sessions`. Final shape after the pin: the
table does not exist. The installer creates then drops it so a fresh
database matches the reference stack.

---

## `auth:sql-table:auth.scim_users`

Upstream: `migrations/20260821000000_add_scim_users.up.sql:4`. No RLS in
this pin. Queryable columns are generated from `resource`.

| Column | Type | Notes |
|---|---|---|
| id | uuid | NOT NULL, PK |
| sso_provider_id | uuid | NOT NULL, FK → `auth.sso_providers(id)` ON DELETE CASCADE |
| user_id | uuid | NULL, FK → `auth.users(id)` ON DELETE SET NULL |
| resource | jsonb | NOT NULL |
| user_name | text | NOT NULL, `GENERATED ALWAYS AS (lower(resource->>'userName')) STORED` |
| external_id | text | `GENERATED ALWAYS AS (resource->>'externalId') STORED` |
| active | boolean | NOT NULL, `GENERATED ALWAYS AS (coalesce((resource->>'active')::boolean, true)) STORED` |
| created_at | timestamptz | NOT NULL DEFAULT `now()` |
| updated_at | timestamptz | NOT NULL DEFAULT `now()` |
| deleted_at | timestamptz | NULL |

Partial unique indexes on `(sso_provider_id, user_name)` and
`(sso_provider_id, external_id)` excluding soft-deleted rows, plus the
list/purge indexes in that migration (`user_name` list index uses
`COLLATE "C"`).

---

## `auth:sql-table:auth.scim_tokens`

Upstream: `migrations/20260821010000_add_scim_tokens.up.sql:4`. No RLS in
this pin. Only the SHA-256 digest is stored.

| Column | Type | Notes |
|---|---|---|
| id | uuid | NOT NULL, PK |
| sso_provider_id | uuid | NOT NULL, FK → `auth.sso_providers(id)` ON DELETE CASCADE |
| token_hash | text | NOT NULL, CHECK `^[0-9a-f]{64}$` |
| prefix | text | NOT NULL |
| created_at | timestamptz | NOT NULL DEFAULT `now()` |
| expires_at | timestamptz | NULL, CHECK null or `> created_at` |
| revoked_at | timestamptz | NULL, CHECK null or `>= created_at` |
| last_used_at | timestamptz | NULL |

Unique `scim_tokens_token_hash_key`. Indexes on `sso_provider_id`,
`expires_at`, `revoked_at`.

---

## Errors / install

Install runs at process start when `DATABASE_URL` is set. Failure aborts
startup (GOAL.md fail loudly). Missing `DATABASE_URL` skips install; HTTP
still serves. Objects are created in one transaction under a 30s deadline, after
`pg_advisory_xact_lock(hashtext('megabase.auth.install_schema'))` so
concurrent Megabase processes do not race on DDL.
After `ADD COLUMN IF NOT EXISTS`, required columns get `SET NOT NULL` so
an older table is not left nullable. `sslmode` is parsed by
`tokio-postgres` (the same parser as the connection). `sslmode=require`
aborts as TLS-required; `verify-*` is an invalid value for this client
and also aborts. The connection is always cleartext (`NoTls`);
`sslmode=disable` is not a safe remote option. Use a Unix socket or
loopback, or a separate encrypted transport, until TLS exists.

Judge cases that list these unit ids belong on a `review/*` branch
(`AGENTS.md`). The harness is HTTP-only, so schema cases cannot pass
until database side-effect steps exist.
