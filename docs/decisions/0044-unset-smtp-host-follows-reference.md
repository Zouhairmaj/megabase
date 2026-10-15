# Unset `GOTRUE_SMTP_HOST` keeps the reference mail host

- Status: Accepted
- Date: 2026-10-15

Issue [#402](https://github.com/Zouhairmaj/megabase/issues/402). Outside
the compose stack, `POST /auth/v1/recover` for a known email dials the
reference default `supabase-mail:2500`, waits up to GoTrue's 10-second dial
timeout, and returns 500 `Error sending recovery email`. The issue asked
for a no-op mailer in that case, or for documentation of the empty
`GOTRUE_SMTP_HOST` that selects one.

Options considered:

1. Unset `GOTRUE_SMTP_HOST` selects the no-op client. The judge starts
   Megabase without `GOTRUE_SMTP_HOST` (`judge/compose.override.yml`),
   while the reference GoTrue gets `supabase-mail` from `.env.example` and
   answers 500. A recover case for a known user would then differ.
2. Fall back to the no-op client when the host does not resolve or the
   dial fails. That replaces GoTrue's 500 with a 200 that the reference
   never sends, and hides a misconfigured SMTP host in production.
3. Keep the reference default. Log the mail client at startup with the
   setting that selects the no-op client, and document a minimal standalone
   environment that sets `GOTRUE_SMTP_HOST=` (empty).

**Decision.** Option 3, like the other unset `GOTRUE_*` defaults
([0024](0024-auth-http-defaults.md)). The standalone environment is in
`docs/configuration.md` ("Running outside the Supabase stack").
