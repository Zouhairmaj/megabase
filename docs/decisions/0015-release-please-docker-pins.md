# release-please and Docker pins

- Status: Accepted (agent coordinator)
- Date: 2026-10-09

Workspace versions use `version.workspace = true`, so release-please's
`rust` strategy errors (`value at path package.version is not tagged`).
Config uses `release-type: simple` and bumps `[workspace.package].version`
only. `bootstrap-sha` is the Phase 0 merge (`7aa41e8`, exclusive) so
changelog collection skips non-conventional `Day 0` / `[phase0]` / `[brand]`
commits. The first release is still Phase 0 at `0.1.0` (`release-as`). The
v0.1.0 release PR must delete `release-as` before it merges.

Container `FROM` lines are pinned by digest (root image and
`.cursor/Dockerfile`). Cloud Agent `rustup-init` and `cargo-binstall`
downloads in `.cursor/Dockerfile` are pinned by SHA-256 and verified before
exec (no `curl|sh`). That pin is the two bootstrap binaries only; the
`stable` toolchain they install and later `cargo binstall` tool fetches
remain unpinned.
