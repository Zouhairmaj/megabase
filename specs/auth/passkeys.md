# Auth: passkeys (issue 50)

Status: disabled gate complete on every `/passkeys` path; with passkeys on,
list, rename, and delete are served. Registration and authentication run
GoTrue's checks up to the go-webauthn call and then answer 501.
Unit ids (`coverage/units.json`):
`auth:route:GET /auth/v1/passkeys`,
`auth:route:PATCH /auth/v1/passkeys/{passkey_id}`,
`auth:route:DELETE /auth/v1/passkeys/{passkey_id}`,
`auth:route:POST /auth/v1/passkeys/registration/options`,
`auth:route:POST /auth/v1/passkeys/registration/verify`,
`auth:route:POST /auth/v1/passkeys/authentication/options`,
`auth:route:POST /auth/v1/passkeys/authentication/verify`
Level: 2

First-factor WebAuthn sign-in (discoverable credentials). Separate from MFA
WebAuthn factors under `/factors` and from the admin routes
`/admin/users/{user_id}/passkeys` (`specs/auth/admin.md`).

## Upstream

| Behavior | File:line (pin) |
|---|---|
| Routes and middleware order | [`internal/api/api.go:324`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/api.go#L324)–`:344` |
| `requirePasskeyEnabled` | [`internal/api/middleware.go:420`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/middleware.go#L420) |
| `PasskeyList`, `PasskeyUpdate`, `PasskeyDelete`, `toPasskeyListItem` | [`internal/api/passkey_manage.go:34`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/passkey_manage.go#L34), [`:54`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/passkey_manage.go#L54), [`:111`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/passkey_manage.go#L111), [`:156`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/passkey_manage.go#L156) |
| `PasskeyRegistrationOptions`, `PasskeyRegistrationVerify` | [`internal/api/passkey_registration.go:42`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/passkey_registration.go#L42), [`:112`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/passkey_registration.go#L112) |
| `PasskeyAuthenticationOptions`, `PasskeyAuthenticationVerify` | [`internal/api/passkey_authentication.go:33`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/passkey_authentication.go#L33), [`:69`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/passkey_authentication.go#L69) |
| `requirePasskeyManagementAAL` | [`internal/api/passkey_webauthn.go:17`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/passkey_webauthn.go#L17) |
| `WebAuthnCredential` queries | [`internal/models/webauthn_credential.go:119`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/models/webauthn_credential.go#L119)–`:194` |
| Audit actions `passkey_updated`, `passkey_deleted` (log type `user`) | [`internal/models/audit_log_entry.go:47`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/models/audit_log_entry.go#L47) |
| `auth.webauthn_credentials`, `auth.webauthn_challenges` | `migrations/20260302000000_add_passkeys.up.sql` |

Pins: Auth `v2.197.0` (`4eee58f`, MIT).

## Configuration

- `GOTRUE_PASSKEY_ENABLED` (default false; unset on the reference stack).
- `GOTRUE_PASSKEY_MAX_PASSKEYS_PER_USER` (default 10).

## Passkeys disabled (reference stack)

All routes live in one chi sub-router whose first middleware is
`requirePasskeyEnabled`. It runs before chi matches a route or a method,
and before authentication, so **every method on every path** under
`/auth/v1/passkeys` answers:

```
404 {"code":404,"error_code":"passkey_disabled","msg":"Passkeys are disabled"}
```

## Passkeys enabled

Errors use GoTrue's default shape (`code`, `error_code`, `msg`).
`passkey_id` and `challenge_id` parse like gofrs `uuid.FromString`
(canonical, braced, `urn:uuid:`, or 32 hex digits). Request bodies decode
like Go's `json.Unmarshal`: `null` is the zero value, keys match ASCII
case-insensitively, and type errors are 400 `bad_json`.

### `GET /passkeys`

`requireAuthentication`. 200 with the user's passkeys, oldest
`created_at` first, each `{id, friendly_name?, created_at, last_used_at?}`
(`omitempty` on the optional two). No passkeys is `[]`.

### `PATCH /passkeys/{passkey_id}`

`requireAuthentication` (no AAL check), then in order:

1. bad `passkey_id` → 404 `validation_failed` `Passkey not found`
2. body `{friendly_name}`; decode error → 400 `bad_json`
3. empty → 400 `validation_failed` `friendly_name is required`
4. more than 120 **bytes** → 400 `validation_failed`
   `friendly_name must be 120 characters or less`
5. not the caller's passkey → 404 `validation_failed` `Passkey not found`
6. update `friendly_name`, `updated_at`, and write the `passkey_updated`
   audit entry (traits `{passkey_id}`) in one transaction
7. 200 with the list item

### `DELETE /passkeys/{passkey_id}`

`requireAuthentication`, then:

1. the AAL check below
2. bad id or not the caller's passkey → 404 `validation_failed`
   `Passkey not found`
3. delete and write `passkey_deleted` in one transaction
4. 204, empty body

### AAL check (`requirePasskeyManagementAAL`)

A user with any verified MFA factor needs an `aal2` session; a JWT
without a session fails closed:
403 `insufficient_aal`
`AAL2 session is required to manage passkeys when MFA is enabled`.

### `POST /passkeys/registration/options` and `/verify`

The `/registration` sub-router runs `requireAuthentication` then
`requireNotAnonymous` (403 `no_authorization`) before its method match.
Both handlers start with:

1. SSO user → 422 `validation_failed` `SSO users cannot register passkeys`
2. the AAL check

`options` then rejects `count >= GOTRUE_PASSKEY_MAX_PASSKEYS_PER_USER`
with 422 `too_many_passkeys` `Maximum number of passkeys reached`.
Building the go-webauthn creation options and storing the challenge are
**not ported: 501**.

`verify` then checks its body (see below). Consuming the challenge,
attestation verification, credential insert, and `passkey_created` are
**not ported: 501**.

### `POST /passkeys/authentication/options` and `/verify`

No authentication. `options` (rate limiter and captcha, then go-webauthn
`BeginDiscoverableLogin`) is **not ported: 501**.

`verify` checks its body, then consuming the challenge, assertion
verification, and session issuance are **not ported: 501**.

### Verify body

`{challenge_id: string, credential: json.RawMessage}`:

1. decode error → 400 `bad_json`
2. empty `challenge_id` → 400 `validation_failed` `challenge_id is required`
3. no `credential` key → 400 `validation_failed` `credential is required`.
   A JSON `null` value still counts as present (`RawMessage("null")`).
4. bad UUID → 400 `validation_failed` `challenge_id must be a valid UUID`

### Unrouted paths and methods

Past the gate, a method or path chi does not route answers 501.

## Not ported

WebAuthn option generation and attestation / assertion verification need
CBOR, COSE keys, and ECDSA / RSA / EdDSA verification crates that are not on
the `deny.toml` allow list. Porting them is a separate dependency proposal.
