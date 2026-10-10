---
title: Secure design
description: Saltzer and Schroeder as applied in Megabase, and the vulnerability classes this Rust HTTP server mitigates.
section: project
order: 3
card: Design principles and the OWASP and CWE classes that apply to this gateway, with the mitigation in the tree.
---

# Secure design

This is the project's record for OpenSSF Best Practices `know_secure_design`
and `know_common_errors`. It states how the Saltzer and Schroeder principles
apply to the Megabase gateway, and which OWASP Top 10 (2021) and CWE classes
matter for this Rust HTTP and PostgreSQL process, with the mitigation that
is actually in the tree.

Megabase is one Rust binary. Unimplemented Supabase routes return HTTP 501
`MEGABASE_NOT_IMPLEMENTED`. That 501 is the current control for many classes
below: a route that does not exist yet does not accept the request.

## Saltzer and Schroeder

Jerome H. Saltzer and Michael D. Schroeder, "The Protection of Information
in Computer Systems" (1975). Each row is what this repository does today.

| Principle | Applied here |
|---|---|
| Economy of mechanism | One binary. JWT verification lives once in `megabase-core` (`Hs256`), shared by Auth and REST when those routes exist. Unimplemented behavior returns 501 instead of a guessed response. |
| Fail-safe defaults | No default `JWT_SECRET`. `alg` other than `HS256`, including `none`, is rejected before HMAC. A present `JWT_SECRET` shorter than 32 bytes aborts startup. `sslmode=require` aborts schema install instead of sending the database password in the clear. |
| Complete mediation | Token checks go through `Hs256::verify`. Served admin routes and logout call that check before they act. `POST /auth/v1/token` checks the bcrypt password or the stored refresh token before it issues an HS256 access token. `GET`/`POST /auth/v1/verify` checks a stored one-time token before it issues a session. The user routes verify the bearer token, reload the user, and check the session before they read or change that user. Signup, verify, and the password and refresh-token grants are public. A served REST filter verifies a bearer token before it reads, or uses the `anon` role when `Authorization` is absent. REST routes and Auth routes that are not served return 501. |
| Open design | Algorithms, error kinds, and this file are public (`specs/core/jwt.md`, `docs/configuration.md`). The demo secret in tests is the public sample from `vendor/supabase/docker/.env.example`. |
| Separation of privilege | Verified tokens expose `role` (`anon`, `authenticated`, `service_role`) and `sub`. Served admin routes accept only roles listed in `GOTRUE_JWT_ADMIN_ROLES` (default `service_role`, `supabase_admin`). Logout and the user routes require a user access token. `PUT /auth/v1/user` writes `app_metadata` only when the user's `role` equals `GOTRUE_JWT_ADMIN_GROUP_NAME` and the audience matches. Verify and the password and refresh-token grants are public, matching GoTrue. A served REST filter `SET`s the verified role (or `anon`) for the read so row security applies. REST routes and Auth routes that are not served return 501. |
| Least privilege | CI workflows default to `contents: read`. Jobs that publish a release, a page, or a check run declare a narrower extra permission on that job. Product code does not shell out and does not fetch URLs from request input. Admin custom-provider URL checks take the host after any userinfo, resolve it, and reject loopback, private, link-local, multicast, and unspecified addresses, including IPv4-mapped IPv6 checked as IPv4. Random password and OTP generation fail the request when the system random source cannot be read. |
| Least common mechanism | There is no built-in shared secret. Operators supply `JWT_SECRET`. The process refuses a key under 32 bytes rather than sharing a short default. |
| Psychological acceptability | Startup and verification errors name the failure (`JWT_SECRET is N bytes…`, `Server lacks JWT secret`, `MEGABASE_NOT_IMPLEMENTED`). Configuration is the environment variables in `docs/configuration.md`. |
| Limited attack surface | Megabase-only paths live under `/_megabase/` (`/_megabase/health`). Supabase prefixes that are not ported answer 501. The binary does not start a shell and does not fetch URLs from callers. `GET /auth/v1/verify` returns GoTrue's fixed 303 HTML redirect; every other response is JSON. Request bodies are capped (default 50 MiB, 413) and requests time out (default 150s, 504). |
| Allowlist input validation | `MEGABASE_PORT` must parse as a `u16` or startup fails. `MEGABASE_HTTP_TIMEOUT_MS` must be a non-zero integer. `MEGABASE_REQUEST_BODY_LIMIT_BYTES` must be an integer. JWT `alg` must be exactly `HS256`. `Authorization: Bearer` must match the GoTrue bearer grammar (one non-whitespace token). JWT payloads must be a JSON object. |

Gaps that are still true, and that later ports have to keep closed:

- HTTP listens in the clear. PostgreSQL uses `NoTls`. `sslmode=disable` and `prefer` still send the database password in the clear; use a Unix socket or loopback until the client speaks TLS.
- Omitting `JWT_SECRET` still starts the process, so health checks and the CI image smoke test work. Verification then fails. Setting a short value does not start the process.
- Served admin routes enforce the configured admin roles. Logout and the user routes require a verified user token, and `app_metadata` updates require the admin group. Served REST filters run as the verified role or as `anon`. REST routes and Auth routes that are not served, including `POST /auth/v1/admin/users`, still return 501.

## Cryptographic key length

`crypto_keylength` (MUST). NIST SP 800-131A (2012) requires at least 112 bits
for symmetric keys through 2030. Megabase requires 32 bytes (256 bits) for
`JWT_SECRET`, measured as raw UTF-8 bytes.

`Config::from_env` rejects a present secret shorter than 32 bytes, including
the empty string, before the process listens:

```text
configuration error: JWT_SECRET is N bytes; HMAC-SHA-256 keys shorter than 32 bytes are disabled
```

`Hs256::new` rejects the same short keys, so a hand-built `Config` cannot
verify with one. HMAC-SHA-256 comes from the `hmac` and `sha2` crates.
The length check is not an entropy check: a 32-byte secret of low entropy
still passes, and the operator supplies the value (`crypto_random` is N/A
until the product generates keys).

The held-out judge uses the same HMAC-SHA256 (`hmac`, `sha2`) plus
HKDF-SHA256 and ChaCha20. `MEGABASE_JUDGE_HIDDEN_SEED` is an Actions
environment secret of at least 32 bytes. It is not in the tree. Optional
human-authored cases are sealed with ChaCha20 and encrypt-then-MAC
(HMAC-SHA256), because ChaCha20 alone does not authenticate ciphertext.
The weekly job publishes pass/fail counts and discards server logs.
MD5, SHA-1, DES, and RC4 are not used there either.

## Vulnerability classes

OWASP Top 10 (2021) and the CWE entries that apply to this kind of server.
The mitigation column is what the code and CI do now.

| Class | Where it would land | Mitigation in this tree |
|---|---|---|
| A01 Broken access control. CWE-862 missing authorization, CWE-285 improper authorization | REST and Auth routes that should honor `role` | Served admin routes require a verified bearer whose `role` is in the configured admin list. Logout and the user routes require a verified bearer. `PUT /user` refuses `app_metadata` unless the caller is in the admin group. Signup, verify, and the password and refresh-token grants are public. Verify still requires a stored one-time token. A served REST filter verifies a bearer or uses the `anon` role, then sets that role for the read. REST routes and Auth routes that are not served, including `POST /auth/v1/admin/users`, return 501. `role` is available only after `Hs256::verify` succeeds. |
| A02 Cryptographic failures. CWE-327 broken crypto, CWE-326 inadequate encryption strength | JWT, database password, HTTP | Default algorithm is HMAC-SHA-256. `alg=none` and every other `alg` fail closed. Keys under 32 bytes are disabled. `Hs256` and `Config` debug output redacts `JWT_SECRET`; `Config` also redacts `DATABASE_URL`. MD5, SHA-1, DES, and RC4 are not used. HTTP and PostgreSQL are still plaintext; see the gaps above. |
| A03 Injection. CWE-89 SQL injection, CWE-78 OS command injection | Schema install, Auth DML, REST filters | Schema install is static DDL. Signup, login, refresh, logout, verify, user update, identity unlink, OAuth grants, and the issue #6 admin statements are `sqlx::query!` / `query_as!` with bound parameters, checked against the committed `.sqlx` cache. Issue #7 admin reads and creates use runtime `sqlx::query` with bound parameters. User-list `ORDER BY` interpolates only the whitelist `ASC` or `DESC`. Audit search uses one static statement per scope (`author`, `action`, `type`); the request does not choose the SQL text. REST filter values are bound parameters. An `in` list is one bound array literal. `IS` keywords, `NOT`, and `= ANY('{}')` are fixed SQL tokens. The relation name is read from `pg_catalog` with a bound lookup and then quoted. Column names are quoted. A catalog type is spliced into a cast only when every character is in a fixed allow-list. `crates/` does not call a shell. |
| A04 Insecure design | New routes and parsers | 501 instead of a plausible answer. Short keys and `sslmode=require` fail closed. This file is the design record new auth, crypto, SQL, and request code follows (`AGENTS.md`). |
| A05 Security misconfiguration. CWE-16 configuration | Env, CI, defaults | Variables are listed in `docs/configuration.md`. No default JWT secret. A zero HTTP timeout is rejected. CI is `cargo clippy -- -D warnings`, `cargo deny`, and `cargo audit`, and those jobs fail the build. Workflows default to `contents: read`. |
| A06 Vulnerable and outdated components. CWE-1104 outdated component | Cargo dependencies | `cargo audit` on `Cargo.lock` and `site/Cargo.lock` in CI. `.cargo/audit.toml` ignores RUSTSEC-2023-0071 (`rsa` via `sqlx-mysql` only; the postgres build does not link it, and no fixed release exists). `cargo deny` for bans and advisories. `cargo vet --locked` requires an imported audit (Mozilla, Google, Bytecode Alliance) or an exemption in `supply-chain/`. Renovate opens dependency updates (`renovate.json`). GitHub code scanning default setup runs CodeQL on Rust. |
| A07 Identification and authentication failures. CWE-287 improper authentication, CWE-306 missing authentication | Bearer tokens, passwords, one-time tokens | Compact JWTs are verified (signature, then `exp` with PostgREST's 30-second skew) before claims are read. Empty tokens fail. Served admin routes, logout, and the user routes require that check. Email signup stores a bcrypt hash (cost 10) and does not log the password. `POST /auth/v1/token` verifies that hash, or the stored refresh token, before it issues an HS256 access token. `GET`/`POST /auth/v1/verify` accepts only a stored one-time token (the SHA-224 of the address and OTP, or the token hash itself) that is unexpired and belongs to a user who is not banned, then issues a session whose AMR method is `otp`. A password change on `PUT /user` rejects the stored hash, stores a new cost-10 hash, and revokes the other sessions. Routes that are not served return 501. Passwords and hashes are not written to logs. |
| A08 Software and data integrity failures. CWE-502 deserialization of untrusted data | JWT JSON, release artifacts | Header and payload are decoded as JSON and checked (object, `alg` allowlist, numeric `exp`) after the HMAC check for the payload. Releases after v0.1.0 keyless-sign with cosign; v0.1.0 shipped without those assets (`docs/install.md`). |
| A09 Security logging and monitoring failures. CWE-532 sensitive info in logs | Process logs | `tracing` logs at info. The HTTP span records method, path, and `x-request-id`. `Authorization`, `apikey`, and `Cookie` are sensitive headers, so their values are not in that span. Debug formatting of `Config`, `Hs256`, `JwtSecret`, and compact JWTs redacts secrets. Auth audit debug lines name the action and log type only; user ids, identity ids, and traits stay in `auth.audit_log_entries`. A caught panic returns a fixed JSON body and does not log the panic message. The held-out judge summary is pass/fail counts; that workflow does not upload server logs or case bodies. There is no security monitor or alert pipeline; a log line is not a detection system. |
| A10 Server-side request forgery. CWE-918 SSRF | Outbound HTTP, DNS | Product code does not make an outbound HTTP request from caller input, and it does not take a caller-controlled file path. `POST /auth/v1/admin/custom-providers` resolves admin-supplied hostnames (5 second timeout) and rejects loopback, private, link-local, multicast, and unspecified results, including IPv4-mapped IPv6 checked as IPv4. That check runs at create time only. It is not pinned to a later connection, so a future port that fetches stored provider URLs must check the connected address again (DNS rebinding). Storage and functions routes return 501. |
| CWE-79 cross-site scripting | HTML responses | `GET /auth/v1/verify` returns GoTrue's 303 body: one anchor whose `href` is HTML-escaped (`&`, quotes, angle brackets). The redirect destination is chosen first by the shared same-site, loopback, and `GOTRUE_URI_ALLOW_LIST` check (`redirect_ok`, GoTrue `IsRedirectURLValid`); HTML escaping does not choose that destination. Every other response is JSON. The static site generator escapes markdown links that are not `http`, `https`, or relative. |
| CWE-119 / CWE-787 buffer overflow | Parsers | The product is Rust. `[workspace.lints.rust]` forbids `unsafe_code` in every workspace crate, and `site/` repeats that lint. Bounds checks stay with the standard library and `serde_json`. |
| CWE-22 path traversal | Static files, storage | The server does not map a request path onto the filesystem. Storage returns 501. |
| CWE-352 cross-site request forgery | Browser session | There is no cookie session. Callers that authenticate will present `Authorization: Bearer`. Browser form routes are not implemented. |
| CWE-798 hardcoded credentials | Source tree | Tests use the public Supabase demo secret from `.env.example`. The process has no other embedded key. The held-out judge seed is an environment secret, not a value in the tree. |
| CWE-434 unrestricted upload | Storage | The storage prefix returns 501. |

Dynamic analysis (fuzzing) is not in CI. That is OpenSSF `dynamic_analysis`,
which stays Unmet, tracked in [issue 153](https://github.com/Zouhairmaj/megabase/issues/153).

## Reporting

Vulnerabilities in this repository's code, CI, and release artifacts are
reported in private. The process, the 3-day acknowledgement, and the 90-day
disclosure window are in [`SECURITY.md`](../SECURITY.md).
