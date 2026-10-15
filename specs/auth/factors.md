# Auth: factors (issue 47)

Status: partial
Unit ids (`coverage/units.json`): `auth:route:POST /auth/v1/factors`,
`auth:route:GET /auth/v1/factors/recovery-codes`,
`auth:route:POST /auth/v1/factors/recovery-codes`,
`auth:route:POST /auth/v1/factors/recovery-codes/verify`,
`auth:route:POST /auth/v1/factors/recovery-codes/regenerate`,
`auth:route:DELETE /auth/v1/factors/recovery-codes`,
`auth:route:POST /auth/v1/factors/{factor_id}/verify`,
`auth:route:POST /auth/v1/factors/{factor_id}/challenge`,
`auth:route:DELETE /auth/v1/factors/{factor_id}`
Level: 2

The MFA routes let a signed-in user enroll, challenge, verify, and remove
second factors, and manage one set of recovery codes. Megabase serves the
authentication, configuration, AAL, and validation checks of every route,
factor unenroll, recovery-code status, and recovery-code deletion. TOTP,
phone, and WebAuthn enrollment, challenges, factor verification, and
recovery-code generation, rotation, and verification return 501.

## Upstream

| Behavior | File:line (pin) |
|---|---|
| Route table under `/factors` | [`vendor/auth/internal/api/api.go:299`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/api.go#L299) |
| Bearer check, user, session, ban | [`vendor/auth/internal/api/auth.go:20`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/auth.go#L20) |
| Anonymous users refused | [`vendor/auth/internal/api/auth.go:44`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/auth.go#L44) |
| `loadFactor` | [`vendor/auth/internal/api/admin.go:75`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/admin.go#L75) |
| Enroll | [`vendor/auth/internal/api/mfa.go:306`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/mfa.go#L306) |
| Challenge | [`vendor/auth/internal/api/mfa.go:566`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/mfa.go#L566) |
| Verify | [`vendor/auth/internal/api/mfa.go:989`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/mfa.go#L989) |
| Unenroll | [`vendor/auth/internal/api/mfa.go:1025`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/mfa.go#L1025) |
| Recovery codes: status, generate, regenerate, verify, delete | [`vendor/auth/internal/api/recovery_codes.go:64`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/recovery_codes.go#L64) |
| Recovery code normalization | [`vendor/auth/internal/crypto/recovery_codes.go:53`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/crypto/recovery_codes.go#L53) |
| Session downgrade on factor removal | [`vendor/auth/internal/models/factor.go:449`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/models/factor.go#L449) |
| MFA switches and defaults | [`vendor/auth/internal/conf/configuration.go:160`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/conf/configuration.go#L160) |

Pins: Auth `v2.197.0` (`4eee58f296d9698a1c2c0ae14d7a0b379c7622d3`, MIT).

## Configuration

| Variable | Default |
|---|---|
| `GOTRUE_MFA_TOTP_ENROLL_ENABLED`, `GOTRUE_MFA_TOTP_VERIFY_ENABLED` | true |
| `GOTRUE_MFA_PHONE_ENROLL_ENABLED`, `GOTRUE_MFA_PHONE_VERIFY_ENABLED` | false |
| `GOTRUE_MFA_WEB_AUTHN_ENROLL_ENABLED`, `GOTRUE_MFA_WEB_AUTHN_VERIFY_ENABLED` | false |
| `GOTRUE_MFA_RECOVERY_CODES_ENROLL_ENABLED`, `GOTRUE_MFA_RECOVERY_CODES_VERIFY_ENABLED` | false |

The reference stack sets none of them.

## Common checks

Every route, in order:

1. No `Authorization: Bearer` header: 401 `no_authorization`,
   `This endpoint requires a valid Bearer token`.
2. JWT, `sub`, `session_id`, user, session, and ban checks: the same
   responses as `GET /user`.
3. `is_anonymous` claim true: 403 `no_authorization`,
   `Anonymous user not allowed to perform these actions`.

The `{factor_id}` routes then load the factor:

4. Not a UUID in a form gofrs `uuid.FromString` accepts (canonical,
   braced, `urn:uuid:`, or 32 hex digits): 404 `validation_failed`,
   `factor_id must be an UUID`.
5. No factor with that id owned by the caller: 404 `mfa_factor_not_found`,
   `Factor not found`.

A missing `session_id` claim leaves GoTrue's session nil. Routes that need
it answer 500 `unexpected_failure` with the message in the table below.
Request bodies decode like Go `encoding/json`: field names match without
case, `null` is the zero value, and a decode error is 400 `bad_json`.

## Routes

| Route | Behavior |
|---|---|
| `POST /factors` | No session: 500 `A valid session and a registered user are required to enroll a factor`. Body `{factor_type, friendly_name, issuer, phone}`. `factor_type` other than `totp`, `phone`, `webauthn`: 400 `validation_failed`, `factor_type needs to be totp, phone, or webauthn`. Disabled type: 422 `mfa_<type>_enroll_not_enabled`, `MFA enroll is disabled for TOTP` / `Phone` / `WebAuthn`. Enabled type: **501** (secret, QR code, and factor creation are not ported). |
| `POST /factors/{id}/challenge` | Factor type gate: `phone` and `totp` verify off: 422 `mfa_<type>_verify_not_enabled`, `MFA verification is disabled for <label>`; `webauthn` off: 422 `mfa_webauthn_verify_not_enabled`; other types (a recovery-code factor): 400 `factor_type needs to be totp, phone, or webauthn`. Enabled: **501**. |
| `POST /factors/{id}/verify` | Body `{challenge_id, code, webauthn}`. Empty `code` on a non-WebAuthn factor: 400 `validation_failed`, `Code needs to be non-empty`. Then the challenge gate, except a disabled WebAuthn verify reports `mfa_webauthn_enroll_not_enabled` (upstream uses the enroll code here). Enabled: **501**. A `challenge_id` that is not a UUID string, or a `webauthn` value that is not an object, returns **501**: Go's decode error text for those types is not reproduced. |
| `DELETE /factors/{id}` | No session: 500 `A valid session and factor are required to unenroll a factor`. Verified factor and the session is not `aal2`: 422 `insufficient_aal`, `AAL2 required to unenroll verified factor`. Otherwise, in one transaction: delete the factor, write audit `factor_unenrolled` (`log_type` `factor`, traits `factor_id`, `factor_status`, `session_id`), delete the factor type's AMR claim from sessions raised by the factor, and set those sessions to `aal1` with no `factor_id`. 200 `{"id": factor_id}`. |
| `GET /factors/recovery-codes` | No recovery-code set: 404 `mfa_factor_not_found`, `The user has not enrolled recovery codes`. Otherwise 200 `{"id": mfa_factor_id, "type": "recovery_code", "total", "remaining"}`, where `remaining` counts codes with `consumed_at` NULL. Never returns codes. |
| `POST /factors/recovery-codes` | No session: 500 `... to generate recovery codes`. Enroll off: 422 `mfa_recovery_codes_enroll_not_enabled`, `MFA enroll is disabled for recovery codes`. A non-empty body is decoded (`friendly_name`). Session not `aal2`: 403 `insufficient_aal`, `AAL2 required to generate recovery codes`. Set exists: 422 `mfa_verified_factor_exists`, `Recovery codes are already enrolled for this user, use regenerate to rotate them`. Otherwise **501** (argon2id code generation is not ported). |
| `POST /factors/recovery-codes/regenerate` | No session: 500 `... to regenerate recovery codes`. Enroll off: 422 as above. Not `aal2`: 403 `AAL2 required to regenerate recovery codes`. No set: 404 `The user has not enrolled recovery codes`. Otherwise **501**. |
| `POST /factors/recovery-codes/verify` | No session: 500 `... to verify recovery codes`. Verify off: 422 `mfa_recovery_codes_verify_not_enabled`, `MFA verification is disabled for recovery codes`. Body `{code}`; the code with hyphens and white space removed is empty: 400 `Code needs to be non-empty`. Otherwise **501**. |
| `DELETE /factors/recovery-codes` | No session: 500 `... to delete recovery codes`. Not `aal2`: 403 `AAL2 required to delete recovery codes`. No recovery-code factor: 404 `The user has not enrolled recovery codes`. Otherwise the unenroll transaction above with audit `recovery_codes_deleted` (traits `factor_id`); the set and its codes cascade. 200 `{"id": factor_id}`. |

## Not ported

- Rate limits on `/verify`, `/challenge`, and `/recovery-codes/verify`
  (`GOTRUE_MFA_RATE_LIMIT_CHALLENGE_AND_VERIFY`).
- `validateFactors` (expired-factor cleanup, friendly-name conflict, factor
  limits, AAL2 for a second factor). It runs only after an enabled type is
  chosen, which is 501.
- Enrolled and unenrolled notification emails (off by default).
- The audit entry's `ip_address` is empty, as in the other Auth routes.

## Megabase

`crates/megabase-auth/src/factors.rs` serves the routes;
`crates/megabase-auth/src/store/mfa.rs` reads `auth.mfa_factors`,
`auth.mfa_recovery_code_sets`, `auth.mfa_recovery_codes`, and
`auth.sessions.aal`. Markers are on `DELETE /factors/{factor_id}` and the
recovery-code status and delete routes, whose behavior is fully served.
`POST /factors`, the `{factor_id}` challenge and verify routes, and the
recovery-code generate, regenerate and verify routes carry no marker: their
main path (TOTP, or argon2id when the switches are on) is 501.
