# Auth: sso (issue 52)

Status: disabled gate complete on every `/sso` path; with SAML on,
`POST /sso` runs GoTrue's checks up to signing the SAML `AuthnRequest` and
then answers 501. The SP metadata and the ACS answer 501 past the gate.
Unit ids (`coverage/units.json`):
`auth:route:POST /auth/v1/sso`,
`auth:route:GET /auth/v1/sso/saml/metadata`,
`auth:route:POST /auth/v1/sso/saml/acs`
Level: 2

SAML 2.0 single sign-on for end users. The admin provider routes under
`/admin/sso/providers` are in `specs/auth/admin.md`.

## Upstream

| Behavior | File:line (pin) |
|---|---|
| Routes and middleware order | [`internal/api/api.go:346`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/api.go#L346)–`:357` |
| `requireSAMLEnabled` | [`internal/api/middleware.go:388`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/middleware.go#L388) |
| `SingleSignOnParams`, `validate`, `SingleSignOn` | [`internal/api/sso.go:13`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/sso.go#L13), [`:26`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/sso.go#L26), [`:40`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/sso.go#L40) |
| `newSAMLServiceProvider`, `SAMLMetadata` | [`internal/api/saml.go:20`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/saml.go#L20), [`:49`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/saml.go#L49) |
| `SamlAcs`, `handleSamlAcs` | [`internal/api/samlacs.go:48`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/samlacs.go#L48), [`:63`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/samlacs.go#L63) |
| `SSO` and `SAMLAssertion` limiters | [`internal/api/apilimiter/apilimiter.go:217`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/apilimiter/apilimiter.go#L217)–`:225` |
| `IsEnabled`, `FindSSOProviderByID`, `FindSSOProviderByDomain` | [`internal/models/sso.go:30`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/models/sso.go#L30), [`:198`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/models/sso.go#L198), [`:232`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/models/sso.go#L232) |
| `validatePKCEParams` | [`internal/api/pkce.go:67`](https://github.com/supabase/auth/blob/4eee58f296d9698a1c2c0ae14d7a0b379c7622d3/internal/api/pkce.go#L67) |

Pins: Auth `v2.197.0` (`4eee58f`, MIT).

## Configuration

- `GOTRUE_SAML_ENABLED` (default false; commented out in the reference
  stack's `docker-compose.yml`).
- `GOTRUE_RATE_LIMIT_SSO` (default 30 per five minutes, burst 30).
- `GOTRUE_SAML_RATE_LIMIT_ASSERTION` (default 15 per five minutes, burst 30).
- Both limiters key on `GOTRUE_RATE_LIMIT_HEADER`, like the token limiter.

## SAML disabled (reference stack)

All routes live in one chi sub-router whose first middleware is
`requireSAMLEnabled`. It runs before chi matches a route or a method, and
before the rate limiters and captcha, so **every method on every path**
under `/auth/v1/sso` (including `/auth/v1/sso/`) answers:

```
404 {"code":404,"error_code":"saml_provider_disabled","msg":"SAML 2.0 is disabled"}
```

## SAML enabled

Errors use GoTrue's default shape (`code`, `error_code`, `msg`). A path or
method under `/sso` with no GoTrue handler is 501.

### `POST /sso` (also `POST /sso/`)

1. `SSO` rate limit: 429 `over_request_rate_limit`, `Request rate limit
   reached`.
2. `verifyCaptcha`: not configurable in Megabase; captcha is off.
3. Body decodes like Go's `json.Unmarshal` into `SingleSignOnParams`:
   `provider_id` (`uuid.UUID`), `domain`, `redirect_to`,
   `skip_http_redirect` (`*bool`), `code_challenge`,
   `code_challenge_method`. Syntax and type errors are 400 `bad_json`;
   `null` is the zero value. A `provider_id` that is neither `null` nor a
   UUID gofrs `UnmarshalText` accepts is 501: gofrs' error wording, and
   the precedence of a `uuid.UUID` type error over other field errors, are
   not ported. The nil UUID counts as no provider id.
4. `validate`, 400 `validation_failed`:
   both set → `Only one of provider_id or domain supported`;
   neither → `A provider_id or domain needs to be provided`.
5. Lookup by id (`auth.sso_providers.id`) or by exact `auth.sso_domains.domain`:
   missing → 404 `sso_provider_not_found` with `No such SSO provider` (id)
   or `No SSO provider assigned for this domain` (domain). A database
   error is 500 `unexpected_failure` `Unable to find SSO provider by ID` /
   `… by domain`.
6. `disabled = true` → 404 `sso_provider_disabled`,
   `SSO Provider is currently disabled`.
7. `validatePKCEParams`: 400 `validation_failed` (shared with signup).
8. **Not ported (501):** creating the flow state and the
   `auth.saml_relay_states` row, building the crewjam/saml
   `AuthnRequest` against the IdP metadata, and signing the HTTP-Redirect
   binding URL with `GOTRUE_SAML_PRIVATE_KEY` (RSA-SHA256). GoTrue answers
   303 to that URL, or 200 `{"url": …}` when `skip_http_redirect` is true.

### `GET /sso/saml/metadata`

**Not ported (501).** GoTrue serves `application/xml` SP metadata with the
signing (and, with `GOTRUE_SAML_ALLOW_ENCRYPTED_ASSERTIONS`, encryption)
certificate of `GOTRUE_SAML_PRIVATE_KEY`, `Cache-Control: public,
max-age=600` (`max-age=60` and a one-hour `cacheDuration` while
`GOTRUE_SAML_PRIVATE_KEY_NEXT` is set), and `download=true` adds a five-year
`validUntil` and `Content-Disposition: attachment; filename="metadata.xml"`.

### `POST /sso/saml/acs`

1. `SAMLAssertion` rate limit: 429 `over_request_rate_limit`.
2. **Not ported (501):** relay state lookup, IdP metadata refresh, XML-DSig
   verification of `SAMLResponse`, and user sign-in. GoTrue turns every
   handler error into a 303 to `GOTRUE_SITE_URL` with the error query
   string.

## Not ported, and why

The enabled paths need RSA keys, X.509 certificates, XML canonicalization,
and XML-DSig. No crate on the `deny.toml` allow list provides them; adding
them is a dependency proposal (see the PR for issue 52).
