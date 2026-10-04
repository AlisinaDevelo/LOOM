# 0400: durable local semantic rebuild

Roadmap 0400 / [#36](https://github.com/AlisinaDevelo/LOOM/issues/36) remains open. This is
the canonical-passage semantic adapter, not completion of desktop scheduling, full-library
resource gates, neural model selection or sliced FTS/final maintenance.

## Reproduction

```sh
cargo build --locked -p loom-cli -p loom-extraction --bins
python3 scripts/test-semantic-jobs.py --loom target/debug/loom
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo +1.88.0 test --locked -p loom-core -p loom-extraction --lib --tests
npm ci
npm run check
```

On macOS, `--report REPORT.json` retains per-process wall/RSS receipts over a larger
synthetic corpus. `--previous-loom OLD_CLI` verifies the actual released v5 upgrade,
preserved directory units, FK integrity and old canonical read/new queue refusal. Keep
each CLI's matching `loom-extractor` sibling. These are bounded fixture checks, not a
large-library or real-world retrieval-quality result.

## Contract coverage

- One passage per claim, separate final publication, deterministic vector/manifest equality
  with foreground rebuild, independent processes, unchanged old index and fair interleaving.
- Compute→commit cancellation, explicit drop including an empty legacy derivative,
  revoke/reselect ABA, OCR revision changes, foreground/legacy rebuild, purge and restore.
- Actual killed worker before and after atomic unit commit; owned recovery does not duplicate
  committed work. Successful quanta preserve the retry budget.
- Injected final completion failure rolls back pointer publication and build detachment.
- Corrupt unit/target/cursor/vector/checksum/nonfinite/missing-row cases cannot publish;
  oversized canonical text is refused before reading its content.
- Legacy consent/policy/purge ABA cannot revive old vectors. Legacy nonfinite vectors,
  extra nullable published units and a still-job-owned active pointer fail closed.
- A writer changing sources between hybrid branches cannot mix lexical/semantic evidence
  or modification times from different SQLite snapshots.
- Released v5 queue migration preserves directory payload, policy and states; unknown/missing
  definitions roll back without marker/epoch/queue changes. Legacy/v2/v3/v4 fixtures still run.
- Unfenced old-runtime semantic reads/rebuild require the explicit upgrade, which discards
  unverifiable old vectors. Foreground rebuild and legacy status independently rehash text.
- Storage inspection includes pending metadata and staged/published vector payload bytes.
- Published generations survive terminal-job forgetting; portable export excludes all runtime
  state, and root purge/restore remove staging and the active generation.

No screenshots or interactive/device viewer acceptance are claimed by these programmatic tests.
The strict dependency-audit warnings remain tracked separately in
[#329](https://github.com/AlisinaDevelo/LOOM/issues/329), with no suppressed advisory or MSRV change.

## Recorded run

Local run: **2026-10-04**, macOS **26.6.2 (25G83)**, **arm64**. Base commit:
`40a5ce3096388ef4dbe993d97c8c299c86782e00`. Toolchains: Rust **1.96.0** and MSRV
**1.88.0**, Node **26.7.0**, npm **11.19.0**. Build jobs: two; incremental compilation and
debug symbols disabled. Local evidence ID: **semantic-queue.W0sg5b**. Receipts and matching
CLI/extractor binaries are retained outside disposable build output.

- Full Rust workspace: **396 tests passed**; strict workspace/all-targets Clippy and formatting
  passed. Rust 1.88: **331 core/extraction tests passed**, plus the full workspace/all-targets
  compile check.
- `npm run check`: **43 UI, 12 browser-extension and 6 Markdown-tooling tests passed**, with
  lint, TypeScript and production frontend build. Python contracts: **33 passed**; CI contract:
  **14 passed**; accessibility contract: **9 passed**. These are programmatic checks, not a
  VoiceOver or human usability session.
- Roadmap validation: **154 active / 4 retired issues, 20 milestones, 141 parent links and
  314 dependency links**. No public roadmap mutation or completion claim for #36.
- Tauri `--debug --no-bundle` build passed. This is not a signing/notarization or interactive
  application acceptance result.
- Fresh CLI: actual released-v5 upgrade, preserved directory units and FK integrity,
  fair semantic quanta, publication/drop, FTS maintenance, file text/PDF/native Vision OCR,
  directory recovery, finite discovery and native-host protocol contracts all passed.
  Optional retained v2/v3 binaries were not supplied; the Rust schema fixtures cover those
  recognized layouts instead.
- v0 retrieval recovered **3/3** sources. v1 Recall@1/5 was **0.9**, anchor precision **1.0**,
  index completeness **1.0**, false-positive rate **0.153846**. The existing missed local-text
  source and negative-query false positive remain; successful execution is not a quality sign-off.
  PDF adversarial, hybrid ablation and semantic rebuild/drop contracts passed. The hybrid gate
  is only the three-source v0 fixture, not evidence of neural semantic quality.

The semantic CLI resource fixture measured **135 one-passage claims** across the small and
131-passage libraries. Maximum process wall time was **69.102 ms** and peak resident size
**16,449,536 bytes**. Its two final publications had maximum process wall time **22.326 ms**
and peak resident size **16,744,448 bytes**. These include subprocess startup; they are not a
20,000-passage finalizer, desktop latency or whole-library resource gate.

The live-extraction restore fixture also exposed contention between a settling worker and the
existing deferred import transaction. Its test-only retry accepts only a Busy/Locked rollback
with unchanged canonical export digest; production restore was not changed. Deferred import
availability remains a portability follow-up rather than a repaired production guarantee.

Strict dependency audit remains **failed on eight denied warnings**, tracked in #329. No
advisory suppression, target filter, MSRV increase or Tauri 2.12 upgrade was introduced.

## Artifact digests

| Artifact | SHA-256 |
| --- | --- |
| Fresh CLI | `c3e8c41694b6539407af880c0a41eb8857b12e9b4a6817ed21dcc395d846b756` |
| Matching extractor | `52c9e5f99fc181ed51df8ec030b9d6bd783c784571e9918024ed122eca683766` |
| MSRV test receipt | `148ebed01676a493d7e261e3547ca88c9212794ebca3f6bafb7493244c5bc00e` |
| Workspace test receipt | `ff11e59643f8470dc18ad8ba5489eecef01e333defacdebf14e1c62df7b4bd64` |
| Semantic CLI measurements | `f619aaa5d831f0798f80e1c0dcac82d340dba81ef996b22c8ed6e264e62380e7` |
