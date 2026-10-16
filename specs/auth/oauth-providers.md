# Auth: OAuth providers (issue 49)

Status: partial. The authorize redirect is served for azure, bitbucket,
discord, facebook, figma, fly, github and gitlab. The provider code exchange
on `/callback` stays 501 for every provider.
Unit ids (`coverage/units.json`): `auth:provider:<name>` for the 25 built-in
providers and `auth:provider:custom:{identifier}`.
Level: 2

A provider unit covers `API.Provider` for that name: the client settings
check (`ValidateOAuth`), the provider's `oauth2.Config` (authorize URL,
default scopes, `scopes` handling) and the `GET /authorize` redirect built
from it. The code exchange (`GetOAuthToken`) and the profile read
(`GetUserData`) call the provider over HTTPS. Megabase has no outbound TLS
client on the `deny.toml` allow list, so these stay 501 until that
dependency is approved.

## Upstream

| Behavior | File:line (pin `v2.197.0`, MIT) |
|---|---|
| `GetExternalProviderRedirectURL` | `vendor/auth/internal/api/external.go:37-139` |
| `API.Provider` switch | `vendor/auth/internal/api/external.go:604-700` |
| `ValidateOAuth` | `vendor/auth/internal/conf/configuration.go:1369-1383` |
| `OAuthProviderConfiguration`, `ProviderConfiguration` | `vendor/auth/internal/conf/configuration.go:59-70`, `:553-592` |
| `chooseHost` | `vendor/auth/internal/api/provider/provider.go:151-162` |
| `reservedOAuthParams` | `vendor/auth/internal/api/custom_oauth_admin.go:654-664` |
| `validatePKCEParams` | `vendor/auth/internal/api/pkce.go:67-80` |
| `NewFlowState`, `ParseCodeChallengeMethod` | `vendor/auth/internal/models/flow_state.go:105-140`, `:69-77` |
| `GetReferrer`, `getRedirectTo` | `vendor/auth/internal/utilities/request.go:75-89`, `:142-153` |
| Constructors | `vendor/auth/internal/api/provider/<name>.go` `New<Name>Provider` |

The authorize URL is the output of golang.org/x/oauth2 v0.36.0
`Config.AuthCodeURL` (the version in `vendor/auth/go.mod`).

## Configuration

Per provider, `GOTRUE_EXTERNAL_<NAME>_` + `ENABLED`, `CLIENT_ID`, `SECRET`,
`REDIRECT_URI`, `URL`, `EMAIL_OPTIONAL`. `<NAME>` is the provider name in
upper case (`LINKEDIN_OIDC`, `SLACK_OIDC`, `VERCEL_MARKETPLACE`, `X`).
`CLIENT_ID` is a comma-separated list. A blank value is an empty list;
parts are not trimmed. Only the first client id goes into the authorize URL.
No redirect URI default exists. The reference stack enables no provider.

## `ValidateOAuth`

The first failure, as the `Unsupported provider: <reason>` detail:

| Check | Reason |
|---|---|
| unknown name (after lower-casing) | `Provider <name> could not be found` |
| `ENABLED` false | `provider is not enabled` |
| no client id | `missing OAuth client ID` |
| empty secret | `missing OAuth secret` |
| empty redirect URI | `missing redirect URI` |

`/authorize` and `/user/identities/authorize` answer 400
`validation_failed`. `/callback` redirects with 400
`oauth_provider_not_supported` (see `endpoints.md`).

## Ported constructors

`chooseHost(url, default)` is `https://<default>` when `URL` is empty, else
`URL` without one trailing `/`. `scopes` is split on `,` and appended to the
defaults; empty parts stay and become extra spaces in `scope`.

| Provider | Authorize URL | Default scopes | Takes `scopes` |
|---|---|---|---|
| azure | `chooseHost(url, login.microsoftonline.com/common)/oauth2/v2.0/authorize` | `openid` | yes |
| bitbucket | `chooseHost(url, bitbucket.org)/site/oauth2/authorize` | `account email` | no |
| discord | `chooseHost(url, discord.com)/api/oauth2/authorize` | `email identify` | yes |
| facebook | `chooseHost(url, www.facebook.com)/dialog/oauth` | `email` | yes |
| figma | `chooseHost(url, www.figma.com)/oauth` | `current_user:read` | yes |
| fly | `chooseHost(url, api.fly.io)/oauth/authorize` | `read` | yes |
| github | `chooseHost(url, github.com)/login/oauth/authorize` | `user:email` | yes |
| gitlab | `chooseHost(url, gitlab.com)/oauth/authorize` | `read_user` | yes |

None of them requires PKCE towards the provider, so no
`auth.oauth_client_states` row is written.

Not ported (501 after `ValidateOAuth` passes): apple, google, linkedin_oidc
and vercel_marketplace (their constructor fetches an OIDC discovery
document), twitter (OAuth 1.0 request token), every other built-in
provider (follow-up PRs of issue 49), and `custom:` providers.

## `GET /authorize`

Query parsing follows `url.ParseQuery`. In order:

1. `API.Provider(provider, scopes)`; failure: 400 as above.
2. `invite_token` set: 501 (`FindUserByOneTimeToken` is not ported).
3. Referrer: `redirect_to` header, else `redirect_to` query, else `Referer`,
   each only when `IsRedirectURLValid`; else `SITE_URL`. An allow-list
   pattern the glob port cannot evaluate: 501.
4. `validatePKCEParams`: 400 `validation_failed` (same messages as
   `/signup`).
5. Parameters for the provider: every query key except `scopes`,
   `provider` and a `reservedOAuthParams` name (compared in lower case;
   `nonce` is let through), first value per key. `workos_provider` is sent
   as `provider`. A parameter may override `scope`.
6. `auth.flow_state` row: `provider_type` as sent (not lower-cased),
   `authentication_method` `oauth`, `referrer`, `email_optional` from the
   provider. With `code_challenge`: a new UUID `auth_code`, the challenge,
   and `code_challenge_method` `s256` or `plain` (case-insensitive; other
   values 400 `validation_failed` / `Invalid code_challenge_method`). Insert
   failure: 500 `unexpected_failure` / `Error creating flow state`.
7. 302 with `Location` = authorize URL + `?` (or `&` when it already has a
   query) + `url.Values.Encode` of `response_type=code`, `client_id`,
   `redirect_uri`, `scope` (defaults joined with a space), `state` (the
   flow state id), then the step 5 parameters. Keys are sorted. Body is
   Go's `<a href="…">Found</a>.\n\n`, `Content-Type: text/html;
   charset=utf-8`. A scheme-less `URL` would make Go rewrite the location
   against the request path; that case is 501.

## Tests

`crates/megabase-auth/src/provider.rs` (validation order, URL encoding,
`chooseHost`, Bitbucket ignoring `scopes`) and
`crates/megabase-auth/src/external.rs` (flow state row, PKCE, reserved
parameters, 501 paths). The judge stack enables no provider, so its cases
can only check the rejection paths, which `auth.toml` already covers.
