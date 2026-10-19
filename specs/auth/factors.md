# Auth: factors (issues 47, 495)

Status: partial (WebAuthn ceremonies still 501)
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
second factors, and manage one set of recovery codes. Megabase serves TOTP
and phone enrollment, challenge, and verification, WebAuthn enrollment, and
recovery-code generation, rotation, and redemption, including the AAL2
session upgrade. WebAuthn challenge and verification return 501 when those
switches are on.

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
| `POST /factors` | No session: 500 `A valid session and a registered user are required to enroll a factor`. Body `{factor_type, friendly_name, issuer, phone}`. `factor_type` other than `totp`, `phone`, `webauthn`: 400 `validation_failed`, `factor_type needs to be totp, phone, or webauthn`. Disabled type: 422 `mfa_<type>_enroll_not_enabled`, `MFA enroll is disabled for TOTP` / `Phone` / `WebAuthn`. Then `validateFactors`: expired unverified factors with no challenge are deleted, a duplicate `friendly_name` is 422 `mfa_factor_name_conflict`, 10 enrolled or 10 verified factors is 422 `too_many_enrolled_mfa_factors`, and a second factor at AAL1 is 403 `insufficient_aal`, `AAL2 required to enroll a new factor`. TOTP: issuer defaults to the `SITE_URL` host; empty email or a bad site URL is 500 `Error generating QR Code` / `site url is improperly formatted`. 200 `{id, type, friendly_name, totp: {qr_code, secret, uri}}`. The secret is 20 random bytes, base32, SHA1, 6 digits, period 30. Phone: empty phone 400 `Phone number required to enroll Phone factor`; a bad number 400 `Invalid phone number format (E.164 required)`; a verified factor for that number 422 `mfa_verified_factor_exists`. Unverified factors for the same number are deleted. 200 echoes the request `phone`. WebAuthn: 200 `{id, type, friendly_name}` and a verified-later row. Audit `factor_in_progress`. |
| `POST /factors/{id}/challenge` | Factor type gate: `phone` and `totp` verify off: 422 `mfa_<type>_verify_not_enabled`, `MFA verification is disabled for <label>`; `webauthn` off: 422 `mfa_webauthn_verify_not_enabled`; other types (a recovery-code factor): 400 `factor_type needs to be totp, phone, or webauthn`. TOTP ignores the body and returns 200 `{id, type, expires_at}` (unix seconds, created plus the challenge expiry, at least 300). Phone reads `{channel}` (default `sms`); a bad channel is 400 `Invalid channel, supported values are 'sms' or 'whatsapp'. ...`. A challenge inside `GOTRUE_MFA_PHONE_MAX_FREQUENCY` (default 1 minute) is 429 `over_sms_send_rate_limit`. The SMS template is rendered. With no SMS provider the response is 500 `Failed to get SMS provider`. A configured provider is **501** because sending is not ported. Either failure writes no challenge row. WebAuthn verify on: **501**. Audit `challenge_created` when a row is written. |
| `POST /factors/{id}/verify` | Body `{challenge_id, code, webauthn}`. Empty `code` on a non-WebAuthn factor: 400 `validation_failed`, `Code needs to be non-empty`. Then the challenge gate, except a disabled WebAuthn verify reports `mfa_webauthn_enroll_not_enabled` (upstream uses the enroll code here). A `challenge_id` that is not a UUID string, or a `webauthn` value that is not an object, returns **501**: Go's decode error text for those types is not reproduced. Missing or null `challenge_id` is the nil UUID. No matching challenge: 422 `mfa_factor_not_found`, `MFA factor with the provided challenge ID not found`. Already verified, or the stored IP differs: 422 `mfa_ip_address_mismatch`, `Challenge and verify IP addresses mismatch.` Expired: the challenge row is deleted and 422 `mfa_challenge_expired`. TOTP uses SHA1, 6 digits, period 30, skew 1; a bad code is 422 `mfa_verification_failed`, `Invalid TOTP code entered`. Phone compares the stored OTP; a bad code is 422 `Invalid MFA Phone code entered`. Success marks the factor verified, writes audit `verification_attempted`, adds the AMR method (`totp` or `mfa/phone`), swaps the legacy refresh token (the previous one is revoked), sets the session to `aal2` with `factor_id`, deletes other `aal1` sessions, and deletes unverified factors of the same type. 200 is an access-token body whose `aal` is `aal2` and whose `amr` lists the new method first. When the custom access token hook is enabled, that token is the hook's claims (`authentication_method` `totp` or `mfa/phone`); a hook failure rolls the Postgres upgrade back. WebAuthn verify on: **501**. |
| `DELETE /factors/{id}` | No session: 500 `A valid session and factor are required to unenroll a factor`. Verified factor and the session is not `aal2`: 422 `insufficient_aal`, `AAL2 required to unenroll verified factor`. Otherwise, in one transaction: delete the factor, write audit `factor_unenrolled` (`log_type` `factor`, traits `factor_id`, `factor_status`, `session_id`), delete the factor type's AMR claim from sessions raised by the factor, and set those sessions to `aal1` with no `factor_id`. 200 `{"id": factor_id}`. |
| `GET /factors/recovery-codes` | No recovery-code set: 404 `mfa_factor_not_found`, `The user has not enrolled recovery codes`. Otherwise 200 `{"id": mfa_factor_id, "type": "recovery_code", "total", "remaining"}`, where `remaining` counts codes with `consumed_at` NULL. Never returns codes. |
| `POST /factors/recovery-codes` | No session: 500 `... to generate recovery codes`. Enroll off: 422 `mfa_recovery_codes_enroll_not_enabled`, `MFA enroll is disabled for recovery codes`. A non-empty body is decoded (`friendly_name`). Session not `aal2`: 403 `insufficient_aal`, `AAL2 required to generate recovery codes`. Set exists: 422 `mfa_verified_factor_exists`, `Recovery codes are already enrolled for this user, use regenerate to rotate them`. A blank name becomes `Recovery codes`. `validateFactors` runs, then 422 `mfa_recovery_codes_sole_factor` when no other verified factor exists. Otherwise 10 codes of 16 lowercase base32 characters, hashed argon2id (`m=19456,t=2,p=1`), stored on a verified `recovery_code` factor. 200 `{id, type, friendly_name, total, codes}`. Audit `recovery_codes_generated`. |
| `POST /factors/recovery-codes/regenerate` | No session: 500 `... to regenerate recovery codes`. Enroll off: 422 as above. Not `aal2`: 403 `AAL2 required to regenerate recovery codes`. No set: 404 `The user has not enrolled recovery codes`. Otherwise the codes are replaced, the lockout is cleared, and 200 returns the new plaintexts once. Audit `recovery_codes_regenerated`. |
| `POST /factors/recovery-codes/verify` | No session: 500 `... to verify recovery codes`. Verify off: 422 `mfa_recovery_codes_verify_not_enabled`, `MFA verification is disabled for recovery codes`. Body `{code}`; the code with hyphens and white space removed is empty: 400 `Code needs to be non-empty`. No set, a bad code, or a malformed hash: 422 `mfa_verification_failed`, `Invalid recovery code entered`. Five failures lock the set for 15 minutes: 429 `mfa_recovery_codes_locked`. A match consumes the code, clears the lock, audits `recovery_codes_verified`, and returns the same AAL2 access-token body as factor verify with AMR method `mfa/recovery_code`. The custom access token hook, when enabled, runs the same way (`authentication_method` `mfa/recovery_code`). |
| `DELETE /factors/recovery-codes` | No session: 500 `... to delete recovery codes`. Not `aal2`: 403 `AAL2 required to delete recovery codes`. No recovery-code factor: 404 `The user has not enrolled recovery codes`. Otherwise the unenroll transaction above with audit `recovery_codes_deleted` (traits `factor_id`); the set and its codes cascade. 200 `{"id": factor_id}`. |

## Not ported

- Rate limits on `/verify`, `/challenge`, and `/recovery-codes/verify`
  (`GOTRUE_MFA_RATE_LIMIT_CHALLENGE_AND_VERIFY`).
- WebAuthn challenge and verification (credential creation and assertion).
  Enrollment writes the factor row. An enabled WebAuthn verify returns 501.
- SMS delivery. Phone challenge with no SMS provider returns 500
  `Failed to get SMS provider`. A configured provider returns 501 because
  sending is not ported. The phone frequency gate and OTP compare are
  served.
- Database encryption of TOTP secrets and phone OTPs. The default
  `GOTRUE_SECURITY_DB_ENCRYPTION_ENCRYPT` is false, so values are stored
  in the clear, which is what GoTrue does then.
- The MFA verification-attempt hook and enrolled/unenrolled notification
  email. The custom access token hook runs on factor verify and recovery-code
  verify, the same as other token issuance.
- The audit entry's `ip_address` is empty, as in the other Auth routes.
  Challenge IP is `X-Forwarded-For` when that header parses, otherwise
  `0.0.0.0` (handlers do not see the socket address).

## Megabase

`crates/megabase-auth/src/factors.rs` serves the routes.
`crates/megabase-auth/src/totp.rs` builds the secret, otpauth URI, and QR
SVG. `crates/megabase-auth/src/recovery_code.rs` hashes codes with
argon2id. `crates/megabase-auth/src/store/mfa.rs` and `mfa_flow.rs` read
and write `auth.mfa_factors`, `auth.mfa_challenges`,
`auth.mfa_recovery_code_sets`, `auth.mfa_recovery_codes`,
`auth.mfa_amr_claims`, and `auth.sessions.aal`. Markers are on every
factors route. WebAuthn challenge and verify stay 501 inside the marked
handlers.
