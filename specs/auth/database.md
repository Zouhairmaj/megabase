# Auth: database (issues 10 and 11)

Issues #10 and #11 port Auth SQL objects. HTTP `/auth/v1` is unchanged
(501). These objects are the compatibility surface that RLS policies and
clients call inside PostgreSQL.

Namespace is always `auth` (GoTrue `{{ index .Options "Namespace" }}`).
Pin: `vendor/auth` `v2.197.0` (`4eee58f296d9698a1c2c0ae14d7a0b379c7622d3`),
MIT. Desired state is the **final** definition after later migrations, not
the first `CREATE`. Install is idempotent (`IF NOT EXISTS` / `CREATE OR
REPLACE`) so it is safe against a database the official stack already
migrated.

Parent keys: several tables reference `auth.users(id)` and
`auth.sessions(id)`. The installer creates those parent tables with only
the columns the foreign keys need. Full `auth.users` / `auth.sessions`
column lists are later units; do not mark them implemented here.

---

## `auth:sql-function:auth.uid()`

Upstream: `migrations/00_init_auth_schema.up.sql:81`, replaced by
`migrations/20211124214934_update_auth_functions.up.sql`,
`migrations/20211202183645_update_auth_uid.up.sql`, final body
`migrations/20220224000811_update_auth_functions.up.sql:3`. Comment
`migrations/20220531120530_add_auth_jwt_function.up.sql:3`.

**Inputs.** None. Reads GUC `request.jwt.claim.sub` then JSON
`request.jwt.claims` → `sub`. `current_setting(..., true)` so a missing
setting is `''`, not an error.

**Output.** `uuid`, `STABLE`, language `SQL`. `NULL` when both sources are
missing or empty. Comment: `Deprecated. Use auth.jwt() -> 'sub' instead.`

**Errors.** `invalid input syntax for type uuid` if `sub` is non-empty and
not a UUID. Invalid JSON in `request.jwt.claims` raises a JSON error
(empty string is `nullif`'d first, so it does not).

**Edge cases.** Prefer the legacy per-claim GUC when it is non-empty, even
if `request.jwt.claims` differs.

---

## `auth:sql-function:auth.role()`

Upstream: `migrations/00_init_auth_schema.up.sql:86`, final body
`migrations/20220224000811_update_auth_functions.up.sql:14`. Comment
`migrations/20220531120530_add_auth_jwt_function.up.sql:4`.

**Inputs.** None. `request.jwt.claim.role`, then `request.jwt.claims` →
`role`.

**Output.** `text`, `STABLE`, `SQL`. `NULL` when both sources are missing
or empty. Comment: `Deprecated. Use auth.jwt() -> 'role' instead.`

**Errors.** Invalid JSON in `request.jwt.claims` (after `nullif`).

---

## `auth:sql-function:auth.email()`

Upstream first create: `migrations/20211124214934_update_auth_functions.up.sql:25`.
Final body: `migrations/20220224000811_update_auth_functions.up.sql:25`.
Comment: `migrations/20220531120530_add_auth_jwt_function.up.sql:5`.

**Inputs.** None. `request.jwt.claim.email`, then `request.jwt.claims` →
`email`.

**Output.** `text`, `STABLE`, `SQL`. `NULL` when both sources are missing
or empty. Comment: `Deprecated. Use auth.jwt() -> 'email' instead.`

**Errors.** Same JSON error as `auth.role()`.

---

## `auth:sql-function:auth.jwt()`

Upstream: `migrations/20220531120530_add_auth_jwt_function.up.sql:7`.

**Inputs.** None. `coalesce` of `request.jwt.claim` then
`request.jwt.claims` (both `nullif`'d empty strings) cast to `jsonb`.

**Output.** `jsonb`, `STABLE`, `SQL`. `NULL` when both settings are missing
or empty. No comment on the function itself.

**Errors.** Non-empty invalid JSON raises. There is no `.sub` suffix on the
first GUC: it is the whole claim document, unlike `auth.uid()`.

---

## `auth:sql-table:auth.instances`

Upstream: `migrations/00_init_auth_schema.up.sql:50`. RLS/grant:
`migrations/20240612123726_enable_rls_update_grants.up.sql:4,21`.

| Column | Type | Null |
|---|---|---|
| id | uuid | NOT NULL, PK |
| uuid | uuid | NULL |
| raw_base_config | text | NULL |
| created_at | timestamptz | NULL |
| updated_at | timestamptz | NULL |

Table comment: `Auth: Manages users across multiple sites.` RLS on. `GRANT
SELECT` to `postgres` WITH GRANT OPTION when that role exists.

---

## `auth:sql-table:auth.audit_log_entries`

Upstream: `migrations/00_init_auth_schema.up.sql:62`. IP:
`migrations/20220614074223_add_ip_address_to_audit_log.postgres.up.sql:2`.
RLS/grant: `20240612123726` lines 6 and 23.

| Column | Type | Null |
|---|---|---|
| instance_id | uuid | NULL |
| id | uuid | NOT NULL, PK |
| payload | json | NULL |
| created_at | timestamptz | NULL |
| ip_address | varchar(64) | NOT NULL DEFAULT `''` |

Index `audit_logs_instance_id_idx` on `instance_id`. Comment: `Auth: Audit
trail for user actions.` RLS on. Select grant to `postgres` as above.

---

## `auth:sql-table:auth.identities`

Upstream create: `migrations/20210909172000_create_identities_table.up.sql:3`.
Final shape: rename `id` → `provider_id` and UUID PK
(`20231117164230_add_id_pkey_identities.up.sql`); generated `email`
(`20221215195800_add_identities_email_column.up.sql`); `user_id` index
(`20211122151130` / `20221027105023`). RLS/grant: `20240612123726` lines
17 and 34.

| Column | Type | Notes |
|---|---|---|
| id | uuid | PK, `DEFAULT gen_random_uuid()` |
| provider_id | text | NOT NULL |
| user_id | uuid | NOT NULL, FK → `auth.users(id)` ON DELETE CASCADE |
| identity_data | jsonb | NOT NULL |
| provider | text | NOT NULL |
| last_sign_in_at | timestamptz | NULL |
| created_at | timestamptz | NULL |
| updated_at | timestamptz | NULL |
| email | text | `GENERATED ALWAYS AS (lower(identity_data->>'email')) STORED` |

Unique `(provider_id, provider)` as `identities_provider_id_provider_unique`.
Indexes: `identities_user_id_idx`, `identities_email_idx` (`text_pattern_ops`).
Comments as in those migrations. Installer must **not** `RENAME` `id`: that
breaks a database that already has the UUID primary key.

---

## `auth:sql-table:auth.flow_state`

Upstream create: `migrations/20230322519590_add_flow_state_table.up.sql:7`.
Later: `authentication_method` (`20230402418590`), `auth_code_issued_at`
(`20240306115329`), OAuth columns and nullable PKCE
(`20260115000000_add_flow_state_oauth_context.up.sql`), cleanup index
(`20230508135423`). Enum `auth.code_challenge_method` (`s256`, `plain`).
RLS/grant: `20240612123726` lines 16 and 33.

Final comment: `Stores metadata for all OAuth/SSO login flows`.

| Column | Type | Notes |
|---|---|---|
| id | uuid | PK |
| user_id | uuid | NULL |
| auth_code | text | NULL (NOT NULL dropped) |
| code_challenge_method | `auth.code_challenge_method` | NULL (NOT NULL dropped) |
| code_challenge | text | NULL (NOT NULL dropped) |
| provider_type | text | NOT NULL |
| provider_access_token | text | NULL |
| provider_refresh_token | text | NULL |
| created_at | timestamptz | NULL |
| updated_at | timestamptz | NULL |
| authentication_method | text | NOT NULL |
| auth_code_issued_at | timestamptz | NULL |
| invite_token | text | NULL |
| referrer | text | NULL |
| oauth_client_state_id | uuid | NULL |
| linking_target_id | uuid | NULL |
| email_optional | boolean | NOT NULL DEFAULT false |

Indexes: `idx_auth_code`, `idx_user_id_auth_method`,
`flow_state_created_at_idx` on `created_at DESC`.

---

## `auth:sql-table:auth.mfa_amr_claims`

Upstream create: `migrations/20221003041349_add_mfa_schema.up.sql:52`.
Primary key `id`: `migrations/20221011041400_add_mfa_indexes.up.sql:1`.
RLS/grant: `20240612123726` lines 14 and 31.

| Column | Type | Notes |
|---|---|---|
| session_id | uuid | NOT NULL, FK → `auth.sessions(id)` ON DELETE CASCADE |
| created_at | timestamptz | NOT NULL |
| updated_at | timestamptz | NOT NULL |
| authentication_method | text | NOT NULL |
| id | uuid | NOT NULL, PK `amr_id_pk` |

Unique `(session_id, authentication_method)` named
`mfa_amr_claims_session_id_authentication_method_pkey`. Comment:
`auth: stores authenticator method reference claims for multi factor authentication`.

---

## `auth:sql-table:auth.custom_oauth_providers`

Upstream: `migrations/20260219120000_add_custom_oauth_providers.up.sql:5`.
Column `custom_claims_allowlist`:
`migrations/20260625000000_add_custom_claims_allowlist.up.sql:5`. No RLS
in this pin.

Checks, indexes, and nullability match those two files (OIDC requires
`issuer`; OAuth2 requires authorization/token/userinfo HTTPS URLs;
identifier `^[a-z0-9][a-z0-9:-]{0,48}[a-z0-9]$`).
`custom_claims_allowlist text[] NOT NULL DEFAULT '{}'`.

---

## `auth:sql-table:auth.mfa_factors`

Upstream create: `migrations/20221003041349_add_mfa_schema.up.sql:21`.
Enums `factor_type` (`totp`, `webauthn`), `factor_status`, `aal_level` in
the same file. Later labels: `phone`
(`20240729123726_add_mfa_phone_config.up.sql`), `recovery_code`
(`20260824000000_add_recovery_codes_factor_type.up.sql`). Indexes:
`20221011041400`, `20230914180801`. Phone column then drop
`mfa_factors_phone_key` and rename `unique_verified_phone_factor` →
`unique_phone_factor_per_user` (`20240806073726`). `last_challenged_at`
(`20240802193726`). WebAuthn columns (`20241009103726`).
`last_webauthn_challenge_data` (`20250925093508`). RLS/grant:
`20240612123726` lines 9 and 26.

| Column | Type | Notes |
|---|---|---|
| id | uuid | NOT NULL, PK |
| user_id | uuid | NOT NULL, FK → `auth.users(id)` ON DELETE CASCADE |
| friendly_name | text | NULL |
| factor_type | `auth.factor_type` | NOT NULL (`totp`, `webauthn`, `phone`, `recovery_code`) |
| status | `auth.factor_status` | NOT NULL |
| created_at | timestamptz | NOT NULL |
| updated_at | timestamptz | NOT NULL |
| secret | text | NULL |
| phone | text | NULL; no column UNIQUE |
| last_challenged_at | timestamptz | NULL, UNIQUE |
| web_authn_credential | jsonb | NULL |
| web_authn_aaguid | uuid | NULL |
| last_webauthn_challenge_data | jsonb | NULL; comment from `20250925093508` |

Indexes: `mfa_factors_user_friendly_name_unique` on
`(friendly_name, user_id) WHERE trim(friendly_name) <> ''`;
`factor_id_created_at_idx` on `(user_id, created_at)`;
`mfa_factors_user_id_idx`; `unique_phone_factor_per_user` on
`(user_id, phone)`. Table comment: `auth: stores metadata about factors`.

---

## `auth:sql-table:auth.mfa_challenges`

Upstream create: `migrations/20221003041349_add_mfa_schema.up.sql:38`.
Cleanup index `mfa_challenge_created_at_idx` (`20230523124323`).
`otp_code` (`20240729123726`). `web_authn_session_data`
(`20241009103726`). RLS/grant: `20240612123726` lines 13 and 30.

| Column | Type | Notes |
|---|---|---|
| id | uuid | NOT NULL, PK |
| factor_id | uuid | NOT NULL, FK → `auth.mfa_factors(id)` ON DELETE CASCADE (`mfa_challenges_auth_factor_id_fkey`) |
| created_at | timestamptz | NOT NULL |
| verified_at | timestamptz | NULL |
| ip_address | inet | NOT NULL |
| otp_code | text | NULL |
| web_authn_session_data | jsonb | NULL |

Comment: `auth: stores metadata about challenge requests made`.

---

## `auth:sql-table:auth.mfa_recovery_code_sets`

Upstream: `migrations/20260824000001_add_recovery_codes_tables.up.sql:2`.
No RLS in this pin.

| Column | Type | Notes |
|---|---|---|
| id | uuid | PK |
| user_id | uuid | NOT NULL, UNIQUE, FK → `auth.users(id)` ON DELETE CASCADE |
| mfa_factor_id | uuid | NOT NULL, UNIQUE, FK → `auth.mfa_factors(id)` ON DELETE CASCADE |
| failed_verification_count | integer | NOT NULL DEFAULT 0, `>= 0` |
| verification_locked_until | timestamptz | NULL |
| created_at | timestamptz | NOT NULL DEFAULT `now()` |
| updated_at | timestamptz | NOT NULL DEFAULT `now()` |

---

## `auth:sql-table:auth.mfa_recovery_codes`

Upstream: `migrations/20260824000001_add_recovery_codes_tables.up.sql:13`.
Index `mfa_recovery_codes_set_id_idx`. No RLS in this pin.

| Column | Type | Notes |
|---|---|---|
| id | uuid | PK |
| mfa_recovery_code_set_id | uuid | NOT NULL, FK → `auth.mfa_recovery_code_sets(id)` ON DELETE CASCADE |
| code_hash | text | NOT NULL |
| consumed_at | timestamptz | NULL |
| created_at | timestamptz | NOT NULL DEFAULT `now()` |

---

## `auth:sql-table:auth.oauth_clients`

Upstream create: `migrations/20250731150234_add_oauth_clients_table.up.sql:9`.
`client_secret_hash` nullable and `client_type` (`public`/`confidential`)
in `20250901200500`. Drop `client_id` column, unique, and index in
`20250903112500` (`id` is the public client identifier).
`token_endpoint_auth_method` text NOT NULL with check
`client_secret_basic` / `client_secret_post` / `none`
(`20260121000000`). No RLS in this pin.

| Column | Type | Notes |
|---|---|---|
| id | uuid | NOT NULL, PK |
| client_secret_hash | text | NULL |
| registration_type | `auth.oauth_registration_type` | NOT NULL (`dynamic`, `manual`) |
| redirect_uris | text | NOT NULL |
| grant_types | text | NOT NULL |
| client_name | text | NULL, length ≤ 1024 |
| client_uri | text | NULL, length ≤ 2048 |
| logo_uri | text | NULL, length ≤ 2048 |
| created_at | timestamptz | NOT NULL DEFAULT `now()` |
| updated_at | timestamptz | NOT NULL DEFAULT `now()` |
| deleted_at | timestamptz | NULL; index `oauth_clients_deleted_at_idx` |
| client_type | `auth.oauth_client_type` | NOT NULL DEFAULT `confidential` |
| token_endpoint_auth_method | text | NOT NULL, check as above |

Installer must **not** keep `client_id`: drop the column if an older table
still has it.

---

## `auth:sql-table:auth.oauth_client_states`

Upstream: `migrations/20251201000000_add_oauth_client_states_table.up.sql:2`.
No RLS in this pin.

| Column | Type | Notes |
|---|---|---|
| id | uuid | PK |
| provider_type | text | NOT NULL |
| code_verifier | text | NULL |
| created_at | timestamptz | NOT NULL |

Index `idx_oauth_client_states_created_at`. Comment: `Stores OAuth states
for third-party provider authentication flows where Supabase acts as the
OAuth client.`

---

## `auth:sql-table:auth.oauth_authorizations`

Upstream create:
`migrations/20250804100000_add_oauth_authorizations_consents.up.sql:17`.
`nonce` + length check: `20251104100000`. Uses
`auth.code_challenge_method` from the flow_state unit. No RLS in this pin.

| Column | Type | Notes |
|---|---|---|
| id | uuid | NOT NULL, PK |
| authorization_id | text | NOT NULL, UNIQUE |
| client_id | uuid | NOT NULL, FK → `auth.oauth_clients(id)` ON DELETE CASCADE |
| user_id | uuid | NULL, FK → `auth.users(id)` ON DELETE CASCADE |
| redirect_uri | text | NOT NULL, length ≤ 2048 |
| scope | text | NOT NULL, length ≤ 4096 |
| state | text | NULL, length ≤ 4096 |
| resource | text | NULL, length ≤ 2048 |
| code_challenge | text | NULL, length ≤ 128 |
| code_challenge_method | `auth.code_challenge_method` | NULL |
| response_type | `auth.oauth_response_type` | NOT NULL DEFAULT `code` |
| status | `auth.oauth_authorization_status` | NOT NULL DEFAULT `pending` |
| authorization_code | text | NULL, UNIQUE, length ≤ 255 |
| created_at | timestamptz | NOT NULL DEFAULT `now()` |
| expires_at | timestamptz | NOT NULL DEFAULT `now() + 3 minutes`; `expires_at > created_at` |
| approved_at | timestamptz | NULL |
| nonce | text | NULL, length ≤ 255 |

Partial index `oauth_auth_pending_exp_idx` on `expires_at` where
`status = 'pending'`.

---

## `auth:sql-table:auth.oauth_consents`

Upstream:
`migrations/20250804100000_add_oauth_authorizations_consents.up.sql:60`.
No RLS in this pin.

| Column | Type | Notes |
|---|---|---|
| id | uuid | NOT NULL, PK |
| user_id | uuid | NOT NULL, FK → `auth.users(id)` ON DELETE CASCADE |
| client_id | uuid | NOT NULL, FK → `auth.oauth_clients(id)` ON DELETE CASCADE |
| scopes | text | NOT NULL, length 1–2048 after trim |
| granted_at | timestamptz | NOT NULL DEFAULT `now()` |
| revoked_at | timestamptz | NULL; `revoked_at IS NULL OR revoked_at >= granted_at` |

Unique `(user_id, client_id)` as `oauth_consents_user_client_unique`.
Indexes: `oauth_consents_active_user_client_idx` (non-revoked),
`oauth_consents_user_order_idx` on `(user_id, granted_at DESC)`,
`oauth_consents_active_client_idx` (non-revoked).

---

## `auth:sql-table:auth.one_time_tokens`

Upstream create: `migrations/20240427152123_add_one_time_tokens_table.up.sql:16`.
`expires_at timestamptz` (`20260831180000`). RLS/grant: `20240612123726`
lines 18 and 35. Enum `auth.one_time_token_type`. Hash indexes on
`token_hash` and `relates_to`, btree fallback if hash cannot be created.
Unique `(user_id, token_type)`.

| Column | Type | Notes |
|---|---|---|
| id | uuid | PK |
| user_id | uuid | NOT NULL, FK → `auth.users` ON DELETE CASCADE |
| token_type | `auth.one_time_token_type` | NOT NULL |
| token_hash | text | NOT NULL, `char_length > 0` |
| relates_to | text | NOT NULL |
| created_at | timestamp without time zone | NOT NULL DEFAULT `now()` |
| updated_at | timestamp without time zone | NOT NULL DEFAULT `now()` |
| expires_at | timestamptz | NULL |

---

## `auth:sql-table:auth.refresh_tokens`

Upstream create: `migrations/00_init_auth_schema.up.sql:33`. `parent`
column (`20210927181326`); parent FK **dropped**
(`20221114143410_remove_parent_foreign_key_refresh_tokens.up.sql`).
`session_id` FK → `auth.sessions(id)` ON DELETE CASCADE
(`20220811173540`). Index `refresh_tokens_token_idx` **dropped**
(`20230411005111`). Cleanup index `refresh_tokens_updated_at_idx`
(`20230508135423`). `refresh_tokens_session_id_revoked_idx`
(`20221021073300`). RLS/grant: `20240612123726` lines 8 and 25.

| Column | Type | Notes |
|---|---|---|
| instance_id | uuid | NULL |
| id | bigserial | NOT NULL, PK |
| token | varchar(255) | NULL, UNIQUE `refresh_tokens_token_unique` |
| user_id | varchar(255) | NULL (not uuid) |
| revoked | bool | NULL |
| created_at | timestamptz | NULL |
| updated_at | timestamptz | NULL |
| parent | varchar(255) | NULL; no FK |
| session_id | uuid | NULL, FK → `auth.sessions(id)` ON DELETE CASCADE |

Indexes: `refresh_tokens_instance_id_idx`,
`refresh_tokens_instance_id_user_id_idx`, `refresh_tokens_parent_idx`,
`refresh_tokens_session_id_revoked_idx`, `refresh_tokens_updated_at_idx`
on `updated_at DESC`. Comment: `Auth: Store of tokens used to refresh JWT
tokens once they expire.`

---

## Errors / install

Install runs at process start when `DATABASE_URL` is set. Failure aborts
startup (GOAL.md fail loudly). Missing `DATABASE_URL` skips install; HTTP
still serves. Objects are created in one transaction. After `ADD COLUMN IF
NOT EXISTS`, required columns get `SET NOT NULL` so an older table is not
left nullable. Indexes that need those columns are created after the
repairs. Hash indexes on `one_time_tokens` fall back to btree only for
`undefined_object` / `feature_not_supported`; other errors abort. Connect
plus SQL are bounded by 30 seconds; a stall returns a startup error
instead of hanging. `sslmode` is parsed by `tokio-postgres` (the same
parser as the connection). `sslmode=require` aborts as TLS-required;
`verify-*` is an invalid value for this client and also aborts. The
connection is always cleartext (`NoTls`); `sslmode=disable` is not a safe
remote option. Use a Unix socket or loopback, or a separate encrypted
transport, until TLS exists.
