# Realtime: API ping (issue 349)

Status: complete
Unit ids (`coverage/units.json`): `realtime:route:GET /realtime/v1/api/ping`
Level: 1

Liveness check of the Realtime HTTP API that clients and monitors call
through the gateway.

## Upstream

| Behavior | File:line (pin) |
|---|---|
| `get("/ping", PingController, :ping)` in the `:tenant_api` scope | `vendor/realtime/lib/realtime_web/router.ex:100`–`:104` |
| `json(conn, %{message: "Success"})` | `vendor/realtime/lib/realtime_web/controllers/ping_controller.ex:4` |
| Gateway `key-auth` on `/realtime/v1/api` | `vendor/supabase/docker/volumes/api/kong.yml:289` |

Pins: realtime `v2.143.3`, supabase `v1.26.08` (`vendor.toml`).

## Inputs

`GET /realtime/v1/api/ping` with an `apikey`.

## Outputs

200, `Content-Type: application/json; charset=utf-8`, body
`{"message":"Success"}`.

## Errors

| When | Status | Body |
|---|---|---|
| No `apikey` | 401 | Kong `No API key found in request` |

## Edge cases

`AssignTenant` resolves the tenant from the `Host` header. The self-hosted
stack has one tenant, so the lookup always succeeds there and Megabase does
not model it.

## Out of scope

The tenant rate limiter and every other Realtime route (501).

## Judge cases

`judge/cases/realtime.toml`: `realtime.ping`.
