# Source consent fencing — prerequisite evidence

This is a prerequisite to [#36](https://github.com/AlisinaDevelo/LOOM/issues/36), not
completion of its durable background job engine. It also hardens existing source scopes,
bookmark recovery, and portable restore. No roadmap issue is closed by this change.

## Verified implementation

| Field | Value |
| --- | --- |
| Code commit | `4e6a1d359bf5ae3297516d144e2d50623134f693` |
| Base main | `14e4a3db140a7faac197b592f67d9de90d64fe92` |
| Device | MacBook Pro 17,1; Apple M1; 8 GB; arm64 |
| OS | macOS 26.6.2 (25G83) |
| Toolchains | Rust/Cargo 1.96.0; Rust/Cargo 1.88.0; Node 26.7.0; npm 11.19.0; Python 3.9.6 |
| Local run | 2026-10-03; build jobs 1, incremental off, dev debug info off |

The evidence-only commit following this code commit does not change runtime source. Retained
logs are named below; their digest identifies the local record without publishing user data.
Hosted CI was not used as a merge gate. The full `verify-device.sh` runner was not executed
for this prerequisite; the individual commands below were run directly.

## Contract and regression mapping

| Evidence ID | Outcome | Deterministic reproduction |
| --- | --- | --- |
| `LOOM-SCOPE-COMMIT` | Prepared extraction cannot reactivate revoked evidence or replace a later selection's version | Two independent `Library` connections prepare, revoke, re-select, and attempt a late commit; canonical tables remain unchanged after the rejected write |
| `LOOM-SCOPE-CHECKPOINT` | Stale cleanup, job admission, progress, and terminal writes cannot mutate a newer selection or another root's job | Root/generation checks occur in the write transaction; a mismatched or non-running job row rejects the entire transaction |
| `LOOM-SCOPE-OBSERVER` | Startup/event reconciliation and bookmark retry cannot silently re-select a scope | Pending observer/retry work retains its original authorization; revocation and explicit re-selection both invalidate it |
| `LOOM-SCOPE-READ` | Disabled roots remain unreadable even if an artifact row incorrectly says active | Lexical search, evidence/original resolution, inspection, semantic projection and index digest require enabled-root joins |
| `LOOM-SCOPE-RESTORE` | Restore cannot revive a pre-purge worker with identical canonical root IDs and generations | Import clears runtime checkpoints and rotates an operational incarnation in the same transaction; invalid imports roll back both |
| `LOOM-SCOPE-BOOKMARK` | Import/replay cannot adopt another scope's URL evidence; explicit re-selection restores unchanged entries | Five cross-scope collision variants roll back; changed-export recovery, legacy disabled-root retry, and inconsistent replay fixtures cover the previous failures |
| `LOOM-SCOPE-ARCHIVE` | Foreign-key-valid bookmark archives still must preserve source ownership | Wrong import root/locator, record ownership, item ownership, and failure-resolution ownership are rejected transactionally |
| `LOOM-SCOPE-SHAPE` | A file/directory replacement does not inherit the old capability | Both kind transitions reject prepared writes, observer work, and selection/import until explicit old-scope reset |
| `LOOM-SCOPE-DENIED` | Permission loss rejects already-prepared writes and checkpoint changes | A real Unix `chmod 000` selected file rejects commit, cleanup, checkpoint and terminal mutations; restoring access permits explicit re-indexing |
| `LOOM-SCOPE-MIGRATION` | Existing evidence survives schema 9 to 10; malformed schema 10 is not repaired destructively | A genuine populated v9 fixture lacks the new field/incarnation; migration preserves IDs, hashes, anchors and checkpoints; missing v10 consent column is refused before writes |

The revocation, re-selection, disabled-root read, restore, bookmark recovery, wrong-job,
archive ownership, and denied-file regression tests were run against their respective
pre-fix implementations and failed their assertions. Those red logs are retained separately
(`revocation-before`, `reselection-before`, `read-scope-before`, `restore-scope-before`,
`bookmark-review-before`, `job-owner-before`, `portable-scope-before`, and
`scope-denied-before`, each with `.log`). All pass in the verified suite.

## Local checks performed

| Command or check | Result |
| --- | --- |
| `cargo fmt --all --check`; `git diff --check` | Pass |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | Pass |
| `cargo test --workspace --locked` | 204 passed: 150 core, 11 CLI, 28 browser capture, 15 desktop; no failures |
| `cargo +1.88.0 check --workspace --all-targets --locked` | Pass |
| Rust 1.88.0 core library, bookmarks, source roots, schema compatibility, portable backup suites | 103 passed; no failures |
| `cargo +1.88.0 test --locked -p loom-cli` | 11 passed; no failures |
| `npm run check` | 35 UI tests and 12 extension tests passed; code/Markdown lint, typecheck and production build passed |
| Python contract suite / CI contract / browser protocol / accessibility contract | 24 / 8 / 10 / 9 tests passed |
| `python3 scripts/roadmap.py --validate-only` | 154 active issues, 4 retired, 20 quarters, 141 parent links, 314 dependencies; valid |
| `bash scripts/security-check.sh` | Secret scan clean; npm audit 0 vulnerabilities; locked Cargo metadata valid |
| Debug Tauri build, without bundle | Pass; this is not signing, notarization or packaged-distribution proof |
| Staged CLI retrieval v0 | 3/3 exact source recovery, anchor precision 1.0, no failures |
| Staged CLI retrieval v1 | Diagnostic fixture passed its configured thresholds; 9/10 positive queries recovered; one hard-negative false positive and one missed paraphrase remain |
| Staged CLI adversarial PDF corpus | 7 fixtures: 4 indexed, 3 expected unsupported, no unexpected failures |
| Native-host build and device contract | Pass: 1 accepted, 4 deterministic rejections, 3 unpaired callers refused, recovery safe |
| Read-only live roadmap reconciliation | Zero planned mutations; no unexpected closed issues or unmanaged relationships |

Native OCR/capture tests in the stable workspace suite exercised the macOS adapters. The
selected MSRV run is not claimed to be the entire native MSRV workspace suite. Synthetic
retrieval fixtures are not evidence of pilot success or real-world product quality.

### Log digests (SHA-256)

| Log | Digest |
| --- | --- |
| `scope-verified-workspace.log` | `1c30faec33efbeaab06ecc60d7d220daa3c3af07f0d700aec7640286cdb62566` |
| `scope-verified-clippy.log` | `f39caf455bfad0e0e434486e3e23241fe6fe63e97929450b35fd1829902ade50` |
| `scope-verified-msrv-focused.log` | `1872f38b9c99745ef798329f8123c4c7e2926f1f3fbbe4f7ff3c24bf8de80fc6` |
| `scope-verified-msrv-cli.log` | `d3d5c7db2a6c3cd35300ffcd7c7e1dbba86abaa12ec68db6f5cf086e2719fb47` |
| `scope-verified-frontend.log` | `c028e4a75930186062b1cf3e87d015e3917e698fea2f5bc67c9afa1194d4cf5f` |
| `scope-verified-tauri-build.log` | `7774c9512f0bcbc33419473b962e4505b9b93fad3bae42e37bdd57ca4d5fc480` |
| `scope-verified-security.log` | `67672286bd85e05bed2854541ec775c3ad3b9a100f72224dea5a36c9310de0a8` |
| `scope-verified-v0.log` | `893c0441ab9c19c261f17d28ed3815c828e7dc7598ec6caff2a10f49a85cfc97` |
| `scope-verified-v1.log` | `2a00d0bc456af9bc1f73abc63207d3cea4bb8769dd77827b93289da1a5e696ee` |
| `scope-verified-pdf.log` | `d96056dc309a6114c4b99d3bc3f9e51ce452f664445d36a1b47dc82eeea874cf` |

## Limits and next gates

- The existing index checkpoint is not a durable queue lease or worker epoch. Admission
  limits, six-state lifecycle, recovery ownership, fairness and real background adapters
  remain part of #36.
- Filesystem availability checks do not make a filesystem and SQLite transaction atomic.
  Stable bounded reads/hash verification remain necessary; this is not a forensic guarantee.
- Bookmark URL/folder identity is still global. Cross-scope collisions now fail closed;
  independent per-root duplicate URL identity remains future work.
- OCR-policy coherence between independent library connections needs its own durable fence;
  source consent generations do not substitute for that policy.
- No human TCC/browser-permission sessions, user study, encryption audit, signing or
  notarization were fabricated. The evidence viewer shows passage text and anchor geometry,
  not a rasterized original PDF/image.

## Merged-main reproduction

[PR #314](https://github.com/AlisinaDevelo/LOOM/pull/314) merged normally at
`4ee54e6ae0c678fad6bbf9467f5b92f7dce27eda` on 2026-10-03 at 00:57:12 UTC. No
required-check bypass or administrative merge was used. The tested PR tree and merged-main
tree both equal `f567fe68479b21e437d6f37b0753943ff36b219f`; their complete diff is empty.

On that clean main tree, `cargo test --workspace --locked` again passed all 204 tests;
the staged, tree-identical CLI again passed the 3-query v0 fixture, and the native-host
contract again passed its accepted/refused/recovery cases. No new issue was closed.

| Merged-main log | SHA-256 |
| --- | --- |
| `scope-merged-workspace.log` | `34a720cfc088520cc25e002793fe2444210428fda2b13daa602a8b83f49d8d96` |
| `scope-merged-v0.log` | `ee6a5f46f3035b0381999f64076a6c05f4c1b9413332249200bda05105d643bd` |
| `scope-merged-native-host.log` | `babf157dc05792fdd7a07e6332690413dfceacc492c315157fa903dab97e59e5` |
