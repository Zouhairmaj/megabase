# Auth: database (issue 10)

Issue #10 ports the first ten Auth SQL objects. HTTP `/auth/v1` is unchanged
(501). These objects are the compatibility surface that RLS policies and
clients call inside PostgreSQL.

Namespace is always `auth` (GoTrue `{{ index .Options "Namespace" }}`).
Pin: `vendor/auth` `v2.197.0` (`4eee58f296d9698a1c2c0ae14d7a0b379c7622d3`),
MIT. Desired state is the **final** definition after later migrations, not
the first `CREATE`. Install is idempotent (`IF NOT EXISTS` / `CREATE OR
REPLACE`) so it is safe against a database the official stack already
migrated.

Parent keys: `auth.identities.user_id` references `auth.users(id)` and
`auth.mfa_amr_claims.session_id` references `auth.sessions(id)`. This issue
creates those parent tables with only the columns the foreign keys need.
Full `auth.users` / `auth.sessions` column lists are later units; do not
mark them implemented here.

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

## Errors / install

Install runs at process start when `DATABASE_URL` is set. Failure aborts
startup (GOAL.md fail loudly). Missing `DATABASE_URL` skips install; HTTP
still serves. Objects are created in one transaction. After `ADD COLUMN IF
NOT EXISTS`, required columns get `SET NOT NULL` so an older table is not
left nullable. `sslmode=require` / `verify-*` abort; the connection is
cleartext.
