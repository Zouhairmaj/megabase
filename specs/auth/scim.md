# Auth: SCIM discovery (issue 51)

Status: complete for the three discovery endpoints
Unit ids (`coverage/units.json`):
`auth:route:GET /auth/v1/scim/v2/ServiceProviderConfig`,
`auth:route:GET /auth/v1/scim/v2/ResourceTypes`,
`auth:route:GET /auth/v1/scim/v2/Schemas`
Level: 2

SCIM 2.0 (RFC 7643 / 7644) discovery. GoTrue ships the SCIM server dark, so
the reference stack answers every path under `/scim/v2` with 404
`feature_disabled`.

## Upstream

Pin: Auth `v2.197.0` (MIT).

| Behavior | File:line |
|---|---|
| Routes, `r.Use(requireScimServerEnabled)`, `r.NotFound` | `internal/api/api.go:463-471` |
| `requireScimServerEnabled` | `internal/api/middleware.go:428` |
| `ScimEnabled` (`GOTRUE_EXPERIMENTAL_SCIM_ENABLED`, default false) | `internal/conf/configuration.go:408` |
| `ServiceProviderConfig`, `ResourceTypes`, `Schemas`, `NotFound`, `list` | `internal/api/scim/server.go` |
| Document shapes | `internal/api/scim/core/service_provider_config.go`, `meta.go` |
| Error and list envelopes, media type | `internal/api/scim/protocol/` |
| Fixtures | `internal/api/scim/testdata/*.json` |

## Behavior

Kong strips `/auth/v1`; the gateway applies `key-auth` first.

1. SCIM disabled (default): every method on every path at or under
   `/auth/v1/scim/v2` is 404 `{"code":404,"error_code":"feature_disabled","msg":"SCIM server is disabled"}`.
2. SCIM enabled, `GET` only (other methods on the three routes: 405 with
   `Allow: GET`). Responses are `application/scim+json`, compact JSON.
   - `ServiceProviderConfig`: 200, every feature unsupported, one primary
     `oauthbearertoken` scheme, `meta.location` =
     `API_EXTERNAL_URL` (trailing `/` trimmed) + `/scim/v2/ServiceProviderConfig`.
   - `ResourceTypes`, `Schemas`: 200 empty `ListResponse`
     (`totalResults` 0, `startIndex` 1, `itemsPerPage` 0, `Resources` `[]`).
     A `filter` query key (any value) is 403 SCIM error "Filtering is not
     supported on this endpoint".
   - Any other path: 404 SCIM error "Endpoint or resource does not exist".

## Not covered

Resource endpoints (`Users`, bearer-token authentication) are other units and
stay 501 behind the enabled gate's 404 fallback only for unknown paths.

## Tests

`crates/megabase-auth/src/scim.rs` unit tests cover the fixtures, the
disabled gate, the filter rejection, and method handling. The reference
stack has SCIM disabled, so judge cases assert the `feature_disabled` 404.
