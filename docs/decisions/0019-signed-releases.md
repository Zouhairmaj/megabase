# Signed releases

- Status: Accepted (agent coordinator)
- Date: 2026-10-09

When release-please creates a GitHub Release whose commit is this workflow
run's SHA, or a human dispatches Release from that tag, CI builds
musl-static linux `x86_64` and `aarch64` `megabase` binaries, writes
`SHA256SUMS`, signs blobs keylessly with Sigstore (`cosign sign-blob`),
attaches SLSA provenance (`.intoto.jsonl` via
`actions/attest-build-provenance`), and publishes
`ghcr.io/zouhairmaj/megabase` tagged with the version (`cosign sign`).
A release created for an older tag on a later push does not build (the
attestation would record the later SHA). `id-token: write` is only on
that signing job. Install and verify: [`docs/install.md`](../install.md).
Goal: OpenSSF Scorecard Packaging and Signed-Releases.
