---
title: OpenSSF Best Practices
description: Met, Unmet, or N/A for every passing criterion of project 15348, with a repo URL.
section: project
order: 2
card: Passing-level answers for bestpractices.dev project 15348. Every MUST is Met or N/A.
---

# OpenSSF Best Practices

Passing-level answers for [project 15348](https://www.bestpractices.dev/en/projects/15348).
The criterion list is the English [passing set](https://www.bestpractices.dev/en/criteria/0)
(67 criteria, before `achieve_passing`). Silver and gold are not in this file.

Assessed 2026-10-09 from this repository. Copy the **Status** cell into the form.
A one-line justification is the text to paste when the form asks for one.
N/A is allowed only where the criterion says so, and it counts as met.

The passing badge needs every MUST met (or N/A where allowed), every SHOULD
met or unmet with a justification, and every SUGGESTED at least considered.
Every MUST criterion below is Met or N/A. Every SHOULD is Met or N/A.
Three SUGGESTED criteria stay Unmet: `test_most`, `dynamic_analysis`, and
`dynamic_analysis_enable_assertions`.

Counts: MUST 38 Met, 0 Unmet, 5 N/A. SHOULD 9 Met, 0 Unmet, 1 N/A.
SUGGESTED 10 Met, 3 Unmet, 1 N/A.

## Basics

### Basic project website content

| Criterion | Level | Status | Justification | Evidence |
|---|---|---|---|---|
| `description_good` | MUST | Met | The site and README say, in plain language, that this is a Supabase-compatible API in one Rust binary. | [README](https://github.com/Zouhairmaj/megabase/blob/main/README.md) |
| `interact` | MUST | Met | Install docs say how to obtain it, the FAQ says how a person can help, and GitHub Issues take bug reports. | [Install](https://github.com/Zouhairmaj/megabase/blob/main/docs/install.md), [FAQ source](https://github.com/Zouhairmaj/megabase/blob/main/site/src/pages.rs) |
| `contribution` | MUST | Met | Contributions are pull requests from agents, with the workflow in AGENTS.md; humans use issues. | [CONTRIBUTING.md](https://github.com/Zouhairmaj/megabase/blob/main/CONTRIBUTING.md) |
| `contribution_requirements` | SHOULD | Met | Acceptable changes need Rust, tests for new behavior, Conventional Commits, and the hard rules in AGENTS.md. | [AGENTS.md](https://github.com/Zouhairmaj/megabase/blob/main/AGENTS.md) |

### FLOSS license

| Criterion | Level | Status | Justification | Evidence |
|---|---|---|---|---|
| `floss_license` | MUST | Met | Results are Apache-2.0, an OSI-approved license. | [LICENSE](https://github.com/Zouhairmaj/megabase/blob/main/LICENSE) |
| `floss_license_osi` | SUGGESTED | Met | Apache-2.0 is OSI-approved. | [LICENSE](https://github.com/Zouhairmaj/megabase/blob/main/LICENSE) |
| `license_location` | MUST | Met | The license text is the repository root file `LICENSE`. | [LICENSE](https://github.com/Zouhairmaj/megabase/blob/main/LICENSE) |

### Documentation

| Criterion | Level | Status | Justification | Evidence |
|---|---|---|---|---|
| `documentation_basics` | MUST | Met | Install, quickstart, and configuration cover build, start, use, and what not to do with `JWT_SECRET` and database TLS. | [docs/install.md](https://github.com/Zouhairmaj/megabase/blob/main/docs/install.md), [docs/configuration.md](https://github.com/Zouhairmaj/megabase/blob/main/docs/configuration.md) |
| `documentation_interface` | MUST | Met | The HTTP interface that exists today is documented: env vars, `/_megabase/health`, gateway prefixes, and the 501 JSON body. | [docs/quickstart.md](https://github.com/Zouhairmaj/megabase/blob/main/docs/quickstart.md), [ADR 0002](https://github.com/Zouhairmaj/megabase/blob/main/docs/adr/0002-gateway-layout.md) |

### Other

| Criterion | Level | Status | Justification | Evidence |
|---|---|---|---|---|
| `sites_https` | MUST | Met | `https://megabase.sh`, the GitHub repository, and GitHub Release downloads all use HTTPS. | [README](https://github.com/Zouhairmaj/megabase/blob/main/README.md) |
| `discussion` | MUST | Met | GitHub Issues, pull requests, and Discussions are searchable, URL-addressable, and open to new people. | [Discussions](https://github.com/Zouhairmaj/megabase/discussions) |
| `english` | SHOULD | Met | Repository docs and issue text are in English. | [AGENTS.md](https://github.com/Zouhairmaj/megabase/blob/main/AGENTS.md) |
| `maintained` | MUST | Met | The project is actively developed (release v0.1.0 on 2026-10-09) and is pursuing this badge. | [Releases](https://github.com/Zouhairmaj/megabase/releases) |

## Change control

### Public version-controlled source repository

| Criterion | Level | Status | Justification | Evidence |
|---|---|---|---|---|
| `repo_public` | MUST | Met | The git repository is public at a stable GitHub URL. | [Zouhairmaj/megabase](https://github.com/Zouhairmaj/megabase) |
| `repo_track` | MUST | Met | Git history records the change, the author, and the time. | [commits](https://github.com/Zouhairmaj/megabase/commits/main) |
| `repo_interim` | MUST | Met | Work lands through pull requests, so the history is not release snapshots only. | [pull requests](https://github.com/Zouhairmaj/megabase/pulls) |
| `repo_distributed` | SUGGESTED | Met | The repository is git. | [Zouhairmaj/megabase](https://github.com/Zouhairmaj/megabase) |

### Unique version numbering

| Criterion | Level | Status | Justification | Evidence |
|---|---|---|---|---|
| `version_unique` | MUST | Met | Each release-please release has one version; the published release is v0.1.0. | [v0.1.0](https://github.com/Zouhairmaj/megabase/releases/tag/v0.1.0) |
| `version_semver` | SUGGESTED | Met | Versions follow semantic versioning, with level gates called out in the roadmap. | [docs/ROADMAP.md](https://github.com/Zouhairmaj/megabase/blob/main/docs/ROADMAP.md) |
| `version_tags` | SUGGESTED | Met | Releases are git tags; `v0.1.0` exists. | [v0.1.0](https://github.com/Zouhairmaj/megabase/releases/tag/v0.1.0) |

### Release notes

| Criterion | Level | Status | Justification | Evidence |
|---|---|---|---|---|
| `release_notes` | MUST | Met | `CHANGELOG.md` and the GitHub Release are a human summary from release-please, not `git log`. | [CHANGELOG.md](https://github.com/Zouhairmaj/megabase/blob/main/CHANGELOG.md) |
| `release_notes_vulns` | MUST | N/A | No CVE or similar public vulnerability has been assigned to Megabase, which is the criterion's N/A case. | [security advisories](https://github.com/Zouhairmaj/megabase/security/advisories) |

## Reporting

### Bug-reporting process

| Criterion | Level | Status | Justification | Evidence |
|---|---|---|---|---|
| `report_process` | MUST | Met | Bugs are filed as GitHub Issues. | [issues](https://github.com/Zouhairmaj/megabase/issues) |
| `report_tracker` | SHOULD | Met | GitHub Issues tracks one report per issue. | [issues](https://github.com/Zouhairmaj/megabase/issues) |
| `report_responses` | MUST | Met | The repository was created on 2026-10-09, so the 2–12 month window contains no bug reports. | [Zouhairmaj/megabase](https://github.com/Zouhairmaj/megabase) |
| `enhancement_responses` | SHOULD | Met | The same 2–12 month window contains no enhancement requests. | [issues](https://github.com/Zouhairmaj/megabase/issues) |
| `report_archive` | MUST | Met | Issues and their comments stay public on GitHub. | [issues](https://github.com/Zouhairmaj/megabase/issues) |

### Vulnerability report process

| Criterion | Level | Status | Justification | Evidence |
|---|---|---|---|---|
| `vulnerability_report_process` | MUST | Met | SECURITY.md tells reporters not to open a public issue and where to report instead. | [SECURITY.md](https://github.com/Zouhairmaj/megabase/blob/main/SECURITY.md) |
| `vulnerability_report_private` | MUST | Met | Private channels are email to agent@megabase.sh and GitHub private vulnerability reporting. | [SECURITY.md](https://github.com/Zouhairmaj/megabase/blob/main/SECURITY.md) |
| `vulnerability_report_response` | MUST | N/A | No vulnerability report was received in the last 6 months (zero published advisories on 2026-10-09). | [security advisories](https://github.com/Zouhairmaj/megabase/security/advisories) |

## Quality

### Working build system

| Criterion | Level | Status | Justification | Evidence |
|---|---|---|---|---|
| `build` | MUST | Met | `cargo build --release --locked -p megabase` rebuilds the binary from source. | [docs/install.md](https://github.com/Zouhairmaj/megabase/blob/main/docs/install.md) |
| `build_common_tools` | SUGGESTED | Met | The build uses Cargo and rustc, the standard Rust tools. | [Cargo.toml](https://github.com/Zouhairmaj/megabase/blob/main/Cargo.toml) |
| `build_floss_tools` | SHOULD | Met | The Rust toolchain and Cargo are FLOSS; CI builds on that toolchain. | [.github/workflows/ci.yml](https://github.com/Zouhairmaj/megabase/blob/main/.github/workflows/ci.yml) |

### Automated test suite

| Criterion | Level | Status | Justification | Evidence |
|---|---|---|---|---|
| `test` | MUST | Met | `cargo test --workspace --locked` is the public suite (`just test`). CI runs those tests with `cargo nextest` and doctests with `cargo test --doc`. | [.github/workflows/ci.yml](https://github.com/Zouhairmaj/megabase/blob/main/.github/workflows/ci.yml) |
| `test_invocation` | SHOULD | Met | `cargo test` is the standard Rust invocation. | [README](https://github.com/Zouhairmaj/megabase/blob/main/README.md) |
| `test_most` | SUGGESTED | Unmet | Codecov on main (2026-10-09) reports 51.32% line coverage and no branch coverage, which is not most branches. | [Codecov](https://codecov.io/gh/Zouhairmaj/megabase) |
| `test_continuous_integration` | SUGGESTED | Met | GitHub Actions runs the test suite on pull requests and on pushes to main. | [.github/workflows/ci.yml](https://github.com/Zouhairmaj/megabase/blob/main/.github/workflows/ci.yml) |

### New functionality testing

| Criterion | Level | Status | Justification | Evidence |
|---|---|---|---|---|
| `test_policy` | MUST | Met | The definition of done requires tests in the crate for new behavior and a failing test for a bug fix. | [AGENTS.md](https://github.com/Zouhairmaj/megabase/blob/main/AGENTS.md) |
| `tests_are_added` | MUST | Met | Recent major changes (JWT verification, Auth SQL install, the 501 gateway) ship with crate tests. | [crates/megabase-core/src/jwt.rs](https://github.com/Zouhairmaj/megabase/blob/main/crates/megabase-core/src/jwt.rs) |
| `tests_documented_added` | SUGGESTED | Met | The same test policy is in the contributor instructions. | [AGENTS.md](https://github.com/Zouhairmaj/megabase/blob/main/AGENTS.md) |

### Warning flags

| Criterion | Level | Status | Justification | Evidence |
|---|---|---|---|---|
| `warnings` | MUST | Met | CI runs `cargo clippy --workspace --all-targets --locked -- -D warnings` on the Rust sources. | [.github/workflows/ci.yml](https://github.com/Zouhairmaj/megabase/blob/main/.github/workflows/ci.yml) |
| `warnings_fixed` | MUST | Met | `-D warnings` fails the build, so a green CI run has no remaining default warnings. | [.github/workflows/ci.yml](https://github.com/Zouhairmaj/megabase/blob/main/.github/workflows/ci.yml) |
| `warnings_strict` | SUGGESTED | Met | That flag turns every default rustc and Clippy warning into an error; `clippy::pedantic` is not enabled. | [.github/workflows/ci.yml](https://github.com/Zouhairmaj/megabase/blob/main/.github/workflows/ci.yml) |

## Security

### Secure development knowledge

| Criterion | Level | Status | Justification | Evidence |
|---|---|---|---|---|
| `know_secure_design` | MUST | Met | `docs/SECURE_DESIGN.md` records the Saltzer and Schroeder principles as applied to this gateway, including the ones not fully implemented yet. | [docs/SECURE_DESIGN.md](https://github.com/Zouhairmaj/megabase/blob/main/docs/SECURE_DESIGN.md) |
| `know_common_errors` | MUST | Met | The same document lists the OWASP Top 10 and the CWE classes that apply to this Rust HTTP server, with the mitigation in the tree for each. | [docs/SECURE_DESIGN.md](https://github.com/Zouhairmaj/megabase/blob/main/docs/SECURE_DESIGN.md) |

### Use basic good cryptographic practices

| Criterion | Level | Status | Justification | Evidence |
|---|---|---|---|---|
| `crypto_published` | MUST | Met | The only default algorithm is HS256 (HMAC-SHA-256), which is published and widely reviewed. | [crates/megabase-core/src/jwt.rs](https://github.com/Zouhairmaj/megabase/blob/main/crates/megabase-core/src/jwt.rs) |
| `crypto_call` | SHOULD | Met | HMAC and SHA-256 come from the `hmac` and `sha2` crates; the project does not reimplement those primitives. | [Cargo.toml](https://github.com/Zouhairmaj/megabase/blob/main/Cargo.toml) |
| `crypto_floss` | MUST | Met | Those crates are FLOSS (RustCrypto, MIT or Apache-2.0). | [Cargo.toml](https://github.com/Zouhairmaj/megabase/blob/main/Cargo.toml) |
| `crypto_keylength` | MUST | Met | Startup rejects a `JWT_SECRET` shorter than 32 bytes (256 bits), above the NIST SP 800-131A 112-bit minimum through 2030. `Hs256::new` rejects the same short keys. | [crates/megabase-core/src/config.rs](https://github.com/Zouhairmaj/megabase/blob/main/crates/megabase-core/src/config.rs), [docs/configuration.md](https://github.com/Zouhairmaj/megabase/blob/main/docs/configuration.md) |
| `crypto_working` | MUST | Met | Verification rejects every `alg` other than HS256, including `none`; MD4, MD5, DES, and RC4 are not used. | [crates/megabase-core/src/jwt.rs](https://github.com/Zouhairmaj/megabase/blob/main/crates/megabase-core/src/jwt.rs) |
| `crypto_weaknesses` | SHOULD | Met | The default mechanism is HMAC-SHA-256, not SHA-1 and not SSH CBC. | [docs/configuration.md](https://github.com/Zouhairmaj/megabase/blob/main/docs/configuration.md) |
| `crypto_pfs` | SHOULD | N/A | The binary does not implement a key-agreement protocol: HTTP is plaintext and PostgreSQL uses `NoTls`. | [docs/configuration.md](https://github.com/Zouhairmaj/megabase/blob/main/docs/configuration.md) |
| `crypto_password_storage` | MUST | Met | Signup stores passwords as bcrypt at cost 10, the GoTrue `DefaultCost`. `POST /auth/v1/token` verifies that hash. Passwords and hashes are not logged. | [docs/SECURE_DESIGN.md](https://github.com/Zouhairmaj/megabase/blob/main/docs/SECURE_DESIGN.md) |
| `crypto_random` | MUST | N/A | Product code does not generate cryptographic keys or nonces; `JWT_SECRET` is supplied by the operator. | [docs/configuration.md](https://github.com/Zouhairmaj/megabase/blob/main/docs/configuration.md) |

### Secured delivery against man-in-the-middle (MITM) attacks

| Criterion | Level | Status | Justification | Evidence |
|---|---|---|---|---|
| `delivery_mitm` | MUST | Met | Source, the site, and GitHub Releases are HTTPS; later releases also keyless-sign assets, but v0.1.0 shipped without them. | [docs/install.md](https://github.com/Zouhairmaj/megabase/blob/main/docs/install.md) |
| `delivery_unsigned` | MUST | Met | `SHA256SUMS` is downloaded over HTTPS and `cosign verify-blob` runs before `sha256sum -c`. | [docs/install.md](https://github.com/Zouhairmaj/megabase/blob/main/docs/install.md) |

### Publicly known vulnerabilities fixed

| Criterion | Level | Status | Justification | Evidence |
|---|---|---|---|---|
| `vulnerabilities_fixed_60_days` | MUST | Met | There is no public medium-or-higher vulnerability in Megabase (zero GitHub advisories on 2026-10-09). | [security advisories](https://github.com/Zouhairmaj/megabase/security/advisories) |
| `vulnerabilities_critical_fixed` | SHOULD | Met | No critical vulnerability has been reported to fix. | [SECURITY.md](https://github.com/Zouhairmaj/megabase/blob/main/SECURITY.md) |

### Other security issues

| Criterion | Level | Status | Justification | Evidence |
|---|---|---|---|---|
| `no_leaked_credentials` | MUST | Met | The only embedded token material is the public Supabase demo secret from `.env.example`, which the criterion allows as a sample. | [crates/megabase-core/src/jwt.rs](https://github.com/Zouhairmaj/megabase/blob/main/crates/megabase-core/src/jwt.rs) |

## Analysis

### Static code analysis

| Criterion | Level | Status | Justification | Evidence |
|---|---|---|---|---|
| `static_analysis` | MUST | Met | Before release, CI runs Clippy (beyond rustc warnings), forbids `unsafe_code`, and runs `cargo-deny` and `cargo audit` on both owned lockfiles. | [.github/workflows/ci.yml](https://github.com/Zouhairmaj/megabase/blob/main/.github/workflows/ci.yml) |
| `static_analysis_common_vulnerabilities` | SUGGESTED | Met | GitHub code scanning default setup runs CodeQL on Rust for pull requests; that query set looks for common vulnerabilities. | [README](https://github.com/Zouhairmaj/megabase/blob/main/README.md) |
| `static_analysis_fixed` | MUST | Met | Clippy `-D warnings`, `unsafe_code = "forbid"`, cargo-deny, and cargo-audit fail CI, so a release from green main has no open finding from those tools. | [.github/workflows/ci.yml](https://github.com/Zouhairmaj/megabase/blob/main/.github/workflows/ci.yml) |
| `static_analysis_often` | SUGGESTED | Met | Those jobs run on every pull request and on every push to main. | [.github/workflows/ci.yml](https://github.com/Zouhairmaj/megabase/blob/main/.github/workflows/ci.yml) |

`scorecard.yml` uploads Scorecard SARIF through `codeql-action/upload-sarif`.
That upload is not the CodeQL analysis. The analysis is GitHub code scanning
default setup (`CODE_SCANNING_IS_STEADY_STATE_DEFAULT_SETUP`), which extracted
Rust and uploaded results on this change. There is no `codeql.yml` in the
repository.

### Dynamic code analysis

| Criterion | Level | Status | Justification | Evidence |
|---|---|---|---|---|
| `dynamic_analysis` | SUGGESTED | Unmet | There is no fuzzer or web scanner, and branch coverage is not the 80% alternative the criterion allows. | [issue 153](https://github.com/Zouhairmaj/megabase/issues/153) |
| `dynamic_analysis_unsafe` | SUGGESTED | N/A | The software produced is Rust. Workspace crates forbid `unsafe_code`, so there is no unsafe block for a fuzzer to exercise. | [Cargo.toml](https://github.com/Zouhairmaj/megabase/blob/main/Cargo.toml) |
| `dynamic_analysis_enable_assertions` | SUGGESTED | Unmet | `crates/` has no `debug_assert!` or other production assertions for a test or fuzz build to turn on. | [crates/](https://github.com/Zouhairmaj/megabase/tree/main/crates) |
| `dynamic_analysis_fixed` | MUST | N/A | No dynamic-analysis tool is run, so there is no confirmed finding from one. | [issue 153](https://github.com/Zouhairmaj/megabase/issues/153) |
