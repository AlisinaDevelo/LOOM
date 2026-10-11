# Foundation verification, 2026-10-11

This record supports the retained-source caption fix in [#352](https://github.com/AlisinaDevelo/LOOM/issues/352)
and current text/recovery checks for [#3](https://github.com/AlisinaDevelo/LOOM/issues/3).
The foundation epic remains open. Closed child issues are not a substitute for their acceptance
evidence; the active watcher gap has reopened [#18](https://github.com/AlisinaDevelo/LOOM/issues/18).

## Revision and device

- Baseline main: `e4c57a4ad2c51cdcb24092c06c5ffd1132441db5`.
- Tested implementation/harness: `9466345585c660336332a52df9a467e1b4d1fd85`.
- Mac17,3, arm64, 32 GiB; macOS 27.0.1, build 26A434.
- Rust/Cargo 1.99.0; Node 26.11.0, npm 11.20.0; Python 3.14.
- Raw records are retained locally under `.loom/verification/2026-10-11-foundation/` and excluded
  from commits. Public fixtures are synthetic CC0; no private documents were used.

The CLI was built with `cargo build --locked -p loom-cli -p loom-extraction` in a separate fresh
target directory. Its SHA-256 is
`74be131edacaa78b99a0dfdedf80dfc59fa2d33d1628f04e5b4d6d9947dba65b`.
Separate CLI and desktop output directories avoid the two executables named `loom` overwriting
each other. The tracked harness refuses staged or tracked source changes and a mismatched HEAD.
The initial native build used `77853c37b2bb4211ea7a7b1fd6b8c9d85c5e1dac`; its app/core source is
byte-identical to the tested revision. The intervening changes concern the harness, CI and research
date note. Source equivalence is recorded separately and does not supply native interaction proof.

## Current artifact map

| Artifact ID | Behavior/criterion | Result and retained record |
| --- | --- | --- |
| `LOOM-352-CAPTION` | Count retained sources without promising that revoked evidence is retrievable | Fail-first regression, minimal copy change, retained count 3 after revoke, cleared evidence and no-match response. `src/App.test.tsx`; full frontend checks pass: 60 UI, 15 extension, 6 tooling tests, lint, typecheck and build. Native repetition is pending. |
| `LOOM-352-BROWSER-CUA` | Rendered caption, evidence invalidation and explicit recovery | Actual compiled frontend in the in-app browser at 1280×720: quoted search, evidence viewer, revoke, no-match, page reload and explicit re-selection pass; count remains 3, no horizontal overflow or console warnings/errors. `browser-cua-9466345.json`, inline screenshots and fixture manifest. IPC and persistence are synthetic; this is not native restart, resolver or external viewer proof. |
| `LOOM-FOUNDATION-CLI` | Fresh local text index/search with exact source evidence | 59 CLI checks pass, including an expected refusal; 3 CC0 sources, 533 bytes, exact artifact/version/passage joins, declared BLAKE3 hashes, character/line anchors and structured excerpts. Separate-process reopen and unchanged indexing preserve identity. `cli-9466345/report.json`. |
| `LOOM-FOUNDATION-BOUNDARIES` | Explicit scope, failure and recovery | Unselected sibling and nested/explicit symlink remain excluded; oversize, invalid UTF-8 and unsupported inputs report bounded outcomes. Edited source gets a new version; returning bytes reuse the original version/passages. Explicit purge preserves originals. Same report and numbered raw step logs. |
| `LOOM-0108-SIGKILL` | Actual forced process termination and durable resume | Owned CLI killed with SIGKILL after committed cursor 1/96. SQLite integrity/FKs and complete ingestion units pass; restart resumes the same run for the remaining 95 units. Active canonical content and counts equal an uninterrupted reference; later indexing is idempotent and originals are unchanged. Same report, `kill.stdout`, `kill.stderr`. |
| `LOOM-0114-CURRENT-REPAIR` | Diagnose and repair a damaged derivative | Corruption injected only into a synthetic FTS projection. The real queued worker reports unhealthy-before/healthy-after; canonical rows, ranked results and all three digest fields remain identical. Same report and `fts-*` numbered step logs. |
| `LOOM-FOUNDATION-NETWORK-DENIED` | CLI text loop works with network operations denied | Loopback control connects without the policy; the same sandboxed probe executes and fails with EPERM. All CLI calls and the killed writer run under `(deny network*)`. `network-calibration.json` and `network-denied.sb`. |
| `LOOM-FOUNDATION-V0-SMOKE` | Current rights-clean retrieval smoke | 3 positive queries: source Recall@1/5, anchor precision and completeness 1.0; median 0.162500 ms, p95 0.874209 ms. No negative or reformulation queries in this benchmark. Separate no-match/boundary assertions are in the harness. Same report and `official-v0-smoke` stdout. |
| `LOOM-FOUNDATION-GOVERNANCE` | Current repository/protection snapshot | Public/default-main/merge settings and active ruleset 21332347 retained in `governance-*.json`. Ruleset requires PRs and prevents deletion/force pushes; it requires zero approving reviews. GitHub secret-scanning state remains unverified. |
| `LOOM-FOUNDATION-NATIVE-BUILD` | Build an isolated bundled desktop from current UI source | Debug app builds with identifier `dev.alisinakarimi.loom.foundation.20261011`; production data is untouched. `native-build-77853c3.log`, SHA-256 `ae875bdedffe9632bf93e0004a19f5b61d043f73f68ed1b9bf931189a989cc59`. Native QA is blocked by the locked Mac. A build is not an interaction test. |

CLI report SHA-256:
`3e391457abac7fc891616ec9853988134e893f3411fffe61eb67a813d7496c50`.
The first harness attempt is retained as a failure: it incorrectly required unchanged BM25 scores
after adding a historical passage. The corrected assertion still compares every source identity,
hash, excerpt, anchor, rank and confidence field; FTS repair compares the complete result vector
against the current pre-corruption baseline, including scores.

Measured CLI command wall times sum to 7,560.094 ms; maximum reported resident memory is
20,447,232 bytes. The killed process has a wall-time record but no RSS measurement. These are small
fixture measurements, not whole-library latency, energy, power-loss or disk-full evidence.
Network denial proves this CLI slice can function under that policy; it does not measure attempted
connections or cover the native app, framework helpers or the external viewer. CI runs the same
harness in explicitly unconstrained mode and makes no network-denial claim.

`cargo test --workspace --locked --no-fail-fast` reports **419 passed, 1 failed across 38 targets**.
The failure is the existing `jobs::tests::persistent_retrieval_during_native_queued_ocr`:
native extraction refuses source bytes, so queued jobs fail instead of completing. The unchanged
OCR no-go remains [#338](https://github.com/AlisinaDevelo/LOOM/issues/338). It is not replaced by a
warm retry or reported as a passing workspace.
Workspace clippy with all targets and `-D warnings`, Rust formatting, roadmap integrity and
Markdown lint pass. These checks do not override the native OCR failure.

## Foundation child audit and remaining gate

| Child | Available evidence | Remaining qualification |
| --- | --- | --- |
| #9 / 0100 | `RESEARCH.md`, `PRODUCT.md`, dependent activation definition | Obsolete Rewind-domain citation replaced by the official Limitless announcement; vendor positioning is not independent product quality. |
| #10 / 0101 | Governance files, current repository/ruleset snapshot | Secret-scanning setting lacks a verified settings receipt. |
| #11 / 0102 | Historical schema/model receipt; current canonical identity and repair checks | Old raw logs were not reverified; this run does not recreate every migration fixture. |
| #12 / 0103 | Current explicit scope/hash/limits/recovery fixture | Same-locator version reuse is not global equal-byte file deduplication. |
| #13 / 0104 | Current search tuple/hash/excerpt/anchor checks | CLI inspection is stored metadata. Native verified opening and matching external viewer remain pending. |
| #14 / 0105 | Current v0 smoke, schema and threshold receipt | Three synthetic queries do not establish user-corpus recovery quality. |
| #15 / 0106 | Current frontend regression/checks and isolated native build | Native interactions pending; actual screen-reader operation remains unverified. |
| #16 / 0107 | Current CI, dependency configuration and diff checks | Retained later scans cannot establish the literal before-initial-public-push scan ordering. |
| #17 / 0108 | Current actual SIGKILL/restart fixture | No power-loss/disk-full or GUI worker-lifecycle claim. |
| #18 / 0109 | Persisted scopes, supplied event hints, coalescing and startup reconciliation | Active watcher/event adapter absent; issue reopened. |
| #63 / 0110 | Tracked thresholds, worksheet and accepted definition record | Defines the study; participant study remains not run. |
| #64 / 0111 | Current schema policy/tests and historical compatibility receipt | Historical version counts remain historical. |
| #65 / 0112 | Prior native scope/relaunch/revoke receipt; current CLI scope checks | Fresh native revoke/reselect repetition pending. Explicit re-selection is the implemented permission recovery. |
| #66 / 0113 | Prior merged #348 resume-prefix regressions and current same-run SIGKILL recovery | This run does not replace native progress/cancel interaction tests. |
| #67 / 0114 | Current unhealthy-before queued repair, canonical/rank/digest preservation | Ordinary library open auto-repairs; the fixture uses the real job-opening path to retain a meaningful unhealthy-before result. |

Sampled historical `/tmp` paths cited by the older evidence documents are absent on this Mac.
Their tracked summaries/digests remain; absence does not establish that the old runs never happened,
but prevents independent inspection of those raw records. Several historical merges also explicitly
record owner review, unprotected main or source-equivalence instead of successful exact merged-SHA
reruns. The current protected-main delivery must be verified separately.

For #3 and #352, native original-source handoff, matching TextEdit path/content, restart,
revocation/re-selection, current claims and exact merged-SHA repetition remain outstanding.
The computer-use surface reports the Mac locked; the user has been asked to unlock it while CLI
work continues. The browser settings session was logged out, so it established no secret-scanning
state. The rendered browser fixture supplies frontend interaction evidence while native checks
remain pending. No unavailable hardware/UI check is replaced by a green CI status.

## Claim boundary

README now distinguishes the shared Rust core/schema from separate CLI and desktop default
database paths. Current tests support source-backed lexical results and conservative evidence
invalidation after revoke/purge. The native external viewer reads independently after verified
path handoff; verification is not an immutable snapshot of what another application later reads.
Current runtime network evidence is limited to the denied CLI slice above; no universal native
egress, real-world retrieval-quality, notarization or completed activation-study claim is made.

## Reproduction

Build the CLI in a separate target, preserve its binary and build log, then run from a clean
checkout with a new output directory:

```sh
python3 -B scripts/verify-foundation-cli.py \
  --loom /path/to/fresh-cli/loom --repo . --expected-commit "$(git rev-parse HEAD)" \
  --out-dir /path/to/new-receipt --network-mode macos-deny
```

The output directory's parent must exist. The harness retains raw command output, fixture hashes,
network calibration, report and an artifact digest manifest. `unconstrained` is available for
portable CI and must be reported as such.
