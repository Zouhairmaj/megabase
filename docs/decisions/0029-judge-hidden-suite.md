# Held-out judge suite

- Status: Accepted (agent coordinator)
- Date: 2026-10-10

Issue #114. The repository is public, so a second suite that lives in
git would be readable by every implementing agent. Visible cases in
`judge/cases/` stay the regression suite. The held-out suite is a
weekly differential run whose concrete requests are not in the tree
and whose published record is aggregate counts only.

## Context

Proposal 0001 called for a private suite the agents cannot read, run
weekly, with only pass/fail counts published. Two shapes fit a public
repository:

- Encrypt cases with a CI secret and keep the ciphertext out of git.
- Expand a secret seed into cases with a public grammar, and compare
  both stacks (the reference is the oracle).

A public grammar cannot hide the *classes* of request it knows how to
build. It does hide the instances: paths, literals, emails, passwords,
and forged tokens are seed-derived and are not logged. Cases a grammar
does not express belong in a human-held ciphertext, not in a file.

`workflow_dispatch` runs the workflow file on the selected ref. A
repository secret would be available to that ref. Environment secrets
restricted to `main` are not.

## Decision

`megabase-judge hidden` builds the suite in memory.

1. `MEGABASE_JUDGE_HIDDEN_SEED` (at least 32 bytes) is the HKDF-SHA256
   input. One expansion is a ChaCha20 keystream for the Level 1 REST
   and Auth grammar (21 cases). Separate expansions are the ChaCha20
   key and the HMAC-SHA256 key used to seal optional cases.
2. `MEGABASE_JUDGE_HIDDEN_CASES`, when set, is standard base64 of
   `MBJH`, a version byte, a 12-byte nonce, ChaCha20 ciphertext, and
   an HMAC-SHA256 tag over that prefix (encrypt-then-MAC). ChaCha20
   alone does not authenticate ciphertext. The plaintext is a normal
   case file. It is never committed.
3. The seed is an argument of the environment, never of the process
   command line. `hidden-open` writes plaintext only to `--out` and
   only when `MEGABASE_JUDGE_HIDDEN_ALLOW_OPEN=1`. CI sets neither
   that variable nor a call to `hidden-open` or `hidden-seal`.
4. The published record is `schema`, `suite`, `total`, `passed`,
   `failed`, `generated`, and `stored`. Failures of individual cases
   exit non-zero. Reference-stack and database errors abort with a
   fixed message that does not include ids, paths, or bodies.
5. `.github/workflows/judge-hidden.yml` runs that command on Mondays
   06:00 UTC and on `workflow_dispatch`, only for `refs/heads/main`,
   in the `judge-hidden` environment. The seed and the sealed blob
   are environment variables of the steps that read them. The product
   binary and third-party actions do not inherit them. The only
   artifact is that JSON. Server logs are discarded. Results are not
   merged into `coverage/judge-results.json` and do not move the
   conformance badge.
6. Read cases in the grammar constrain `public.todos` to fixture ids
   `1`, `2`, and `3`, so inserts left by an earlier run cannot make
   the two databases disagree on a later read.

The grammar is public on purpose. Hiding it would only move the same
Rust into a secret, which implementing agents could not review and
which this repository could not vendor. Unseen cases are the sealed
blob.

## Consequences

A human creates the `judge-hidden` environment, limits it to `main`,
and stores the seed there before the weekly run can pass. Until then
the workflow fails closed. That setup is a pending human-only action.
Do not add **Hidden judge** to required pull-request checks.

Rotating the seed changes every generated instance and invalidates a
previously sealed blob. Seal again with the new seed. Changing the
grammar in a later review PR also changes instances for the same seed;
the published counts are not a case-level baseline.
