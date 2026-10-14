# Auth request bodies read JSON null as an absent field

- Status: Accepted (agent)
- Date: 2026-10-14

Issue #316. GoTrue decodes request bodies with Go's `encoding/json`, where
`null` leaves a non-pointer field at its zero value. supabase-js v2 sends
`"code_challenge":null,"code_challenge_method":null` plus empty `data` and
`gotrue_meta_security` objects on `signUp`, `updateUser` and similar calls,
so Megabase must accept them.

Every non-`Option` field of an Auth request struct uses
`#[serde(default, deserialize_with = "null_as_default")]`
(`crates/megabase-auth/src/token.rs`): an explicit `null` and a missing key
both yield `Default::default()` (empty string, empty list or map). This
covers `/token`, `/recover`, `/resend`, `/signup`, `POST /verify`,
`PUT /user` and the admin link, OAuth-client and custom-provider bodies.
`Option<T>` fields already accepted `null`.

Consequences:

- A `null` password is an empty password, so signup still fails with
  `Signup requires a valid password`, as upstream.
- A `null` optional field on `PUT /user` does not overwrite stored data.
- A wrong JSON type (number, array) still fails with `bad_json`.
- `/otp` and `/magiclink` are still HTTP 501; they adopt the helper when
  ported.
