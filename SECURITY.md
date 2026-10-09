# Security policy

Megabase is an unofficial experiment. It is not production software
today: unimplemented routes return HTTP 501
`{"code":"MEGABASE_NOT_IMPLEMENTED",...}`. We still want private reports
of vulnerabilities in this repository's code, CI, and release artifacts.

## Reporting a vulnerability

**Do not open a public GitHub issue** for a security vulnerability.

Report privately, in this order:

1. Email **agent@megabase.sh** (maintainers). Include the affected path
   or crate, a description of the issue, impact, and a proof of concept
   if you have one.
2. Or use [GitHub private vulnerability reporting](https://github.com/Zouhairmaj/megabase/security/advisories/new).

We acknowledge reports within **3 days**. For a confirmed vulnerability
we aim to ship a fix and coordinate disclosure within **90 days** of
the report (sooner if a patch is ready). Please do not publish the
issue until a fix is released or that 90-day window ends, whichever
comes first.

## What counts as a vulnerability

Report bugs that let an attacker:

- bypass authentication or authorization Megabase claims to implement
- execute unintended SQL or commands
- read or write data they should not
- compromise the Megabase process, host, or release artifacts
  (binary, container image, GitHub Release)

Do **not** report:

- HTTP 501 `MEGABASE_NOT_IMPLEMENTED` responses (expected)
- missing features that are not implemented yet
- issues that exist only in `vendor/` (report those upstream)
- theoretical issues in components that still return 501

After a GitHub Release publishes signed assets, verify binaries and
`ghcr.io/zouhairmaj/megabase` as in [`docs/install.md`](docs/install.md).
`v0.1.0` shipped without those assets.

## Secure design

How the Saltzer and Schroeder principles apply to this gateway, and which
OWASP Top 10 and CWE classes the Rust HTTP server mitigates, is recorded in
[`docs/SECURE_DESIGN.md`](docs/SECURE_DESIGN.md).

## Disclosure

After a fix lands we credit the reporter in the GitHub Security Advisory
and the changelog unless you ask us not to. Maintainers: `@Zouhairmaj`
and `megabase-agent`.
