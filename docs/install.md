---
title: Install
description: Download a signed GitHub Release binary, pull the GHCR image, or build from source.
section: get-started
order: 1
card: Signed linux binaries from a GitHub Release, the image on GHCR, or build from source.
---

# Install

Megabase is one Rust binary next to PostgreSQL. Unimplemented routes return
HTTP 501 `{"code":"MEGABASE_NOT_IMPLEMENTED",...}`. Nothing here is
production software.

The Release workflow attaches musl-static linux `x86_64` and `aarch64`
binaries, `SHA256SUMS`, Sigstore signatures, and SLSA provenance, and
publishes `ghcr.io/zouhairmaj/megabase` tagged with that version. `v0.1.0`
shipped without those assets; use a later tag, or a `v0.1.0` backfill
dispatched from that tag, that lists `megabase-*-unknown-linux-musl` on
[Releases](https://github.com/Zouhairmaj/megabase/releases). Commands for
download and verification live only on this page.

## Release binary

Install [cosign](https://docs.sigstore.dev/cosign/system_config/installation/).
Set `TAG` to a release that includes the musl assets, then authenticate
`SHA256SUMS` before trusting the checksum or running the binary.

```shell
TAG=vX.Y.Z
case "$(uname -m)" in
  x86_64) ARCH=x86_64-unknown-linux-musl ;;
  aarch64|arm64) ARCH=aarch64-unknown-linux-musl ;;
  *) echo "Unsupported architecture: $(uname -m)" >&2; exit 1 ;;
esac
BASE=https://github.com/Zouhairmaj/megabase/releases/download/${TAG}
ASSET=megabase-${TAG}-${ARCH}
curl -fsSL -O "${BASE}/${ASSET}"
curl -fsSL -O "${BASE}/SHA256SUMS"
curl -fsSL -O "${BASE}/SHA256SUMS.sig"
curl -fsSL -O "${BASE}/SHA256SUMS.pem"
cosign verify-blob \
  --certificate SHA256SUMS.pem \
  --signature SHA256SUMS.sig \
  --certificate-identity-regexp '^https://github.com/Zouhairmaj/megabase/\.github/workflows/release\.yml@refs/' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com \
  SHA256SUMS
sha256sum -c SHA256SUMS --ignore-missing
chmod +x "${ASSET}"
./"${ASSET}"
```

The binary listens on `0.0.0.0:8000` (`MEGABASE_HOST` / `MEGABASE_PORT`).
See [Configuration](configuration.md).

## Verify signatures

Release blobs are signed keylessly with Sigstore (`cosign sign-blob` in
`.github/workflows/release.yml`). The signing identity is this
repository's Release workflow; the OIDC issuer is GitHub Actions. The
install sequence above already authenticates `SHA256SUMS` before the
checksum check.

To verify a binary instead of the checksum file, use that asset's `.pem`
and `.sig` (same identity flags). SLSA provenance is
`megabase-${TAG}.intoto.jsonl` on the Release and in GitHub attestations:

```shell
gh attestation verify "${ASSET}" --repo Zouhairmaj/megabase
```

## Container image

After the Release workflow publishes an image, it is
`ghcr.io/zouhairmaj/megabase:<tag>` (also tagged without the leading `v`)
and signed with `cosign sign` in the same job. `v0.1.0` has no image until
that job is dispatched for the tag.

```shell
docker pull ghcr.io/zouhairmaj/megabase:vX.Y.Z
cosign verify \
  --certificate-identity-regexp '^https://github.com/Zouhairmaj/megabase/\.github/workflows/release\.yml@refs/' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com \
  ghcr.io/zouhairmaj/megabase:vX.Y.Z
docker run --rm -p 8000:8000 ghcr.io/zouhairmaj/megabase:vX.Y.Z
```

Pass `-e DATABASE_URL=...` if the process should install Auth SQL objects.
The image includes `megabase-healthcheck`, which probes
`/_megabase/health`.

## Clone

```shell
git clone --recurse-submodules https://github.com/Zouhairmaj/megabase.git
cd megabase
```

Vendor pins live in `vendor/` (`vendor.toml`). Coverage and the judge need the submodules.

## Build from source

MSRV is **1.89**. From the repository root:

```shell
cargo build --release --locked -p megabase
./target/release/megabase
```

Environment variables are in [Configuration](configuration.md). `just` lists
every recipe that works today.

When `DATABASE_URL` is set, startup creates the Auth schema objects Megabase
currently implements. The install is idempotent. Without `DATABASE_URL` the
process still serves HTTP.

The public site generator is separate:

```shell
cargo run --manifest-path site/Cargo.toml -- --repo-root . --out _site
```

## Build the image locally

The root `Dockerfile` builds the release image. Both `FROM` lines are
pinned by digest so Scorecard Pinned-Dependencies does not flag a
floating tag (alerts #6 and #7 on `main`):

| Stage | Image |
| --- | --- |
| Builder | `rust:1.89-slim-bookworm@sha256:d7fc7de78bb8c1469933aeecbf801314d30d7d6e9f0578bba4cfa285bfa37fe6` |
| Runtime | `debian:bookworm-slim@sha256:7c7b2c966bc9ee8cedfeef67e0e279108992c77681fa595db4a9d65c06ccc587` |

The Cloud Agent image in `.cursor/Dockerfile` is pinned the same way:
`ubuntu:24.04@sha256:534baea6a22c03a63003dbc8dbe78fe34bc0d7e595d9a9dc9834884ff530eb55`
(index digest for that tag, verified 2026-10-09).

That Dockerfile does not pipe installers into a shell. `rustup-init` 1.29.1
and `cargo-binstall` v1.25.2 are downloaded, checked with a hard-coded
SHA-256, then executed (Scorecard Pinned-Dependencies `downloadThenRun`,
alerts #17 and #18 on `main`). The pin is those two bootstrap binaries
only: `rustup-init` then installs the floating `stable` toolchain (the
checkout's `rust-toolchain.toml` selects the channel after the repo is
present), and `cargo binstall` installs latest `just`, `cargo-deny`,
`cargo-audit`, and `cargo-llvm-cov`. Those later fetches are outside
alerts #17 and #18. The Cloud Agent `install` script in
`.cursor/environment.json` uses the same hashes when those tools are
missing. Hashes were re-verified on 2026-10-09 against
`static.rust-lang.org/rustup/archive/1.29.1` and the GitHub release
asset digests. Renovate does not bump these archive hashes; bump the
version comment, URL, and digest together.

Renovate's `docker` datasource has `pinDigests: true`, so a tag bump and
its digest move in the same PR. Do not un-pin these images to a bare tag.

```shell
docker build -t megabase .
docker run --rm -p 8000:8000 megabase
```

## Requirements

| Tool | Why |
| --- | --- |
| Rust 1.89+ | Workspace MSRV; CI also checks 1.89 |
| PostgreSQL 15+ | External database (not required just to start the 501 gateway) |
| Docker | Release image; Compose for the judge |
| cosign (optional) | Verify Sigstore signatures on Release assets and GHCR |
