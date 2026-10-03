# Durable maintenance queue: local verification

## Tested scope

Code head: `aaa4dfae3a7ea267b7ed8dc724fd2c608b44a783`, based on merged main
`7d67bf54715fd2c4cb0a7be74232568d6e029b4f`. Verified on 2026-10-03 on
MacBookPro17,1, Apple M1 arm64 / 8 GiB, macOS 26.6.2 build 25G83; Rust 1.96.0
and 1.88.0, Node 26.7.0, npm 11.19.0, Python 3.9.6. Rust used one build job,
no incremental compilation, and no debug info.

This is the first opt-in adapter for roadmap `0400` / GitHub #36: a durable queue that
actually runs FTS repair. Source indexing, OCR, semantic rebuild, and desktop scheduling
are not yet adapters. The operational contract and limitations are in
[BACKGROUND_JOBS.md](../BACKGROUND_JOBS.md). No issue is complete merely from this slice.

## Actual checks

- Formatting, diff hygiene, all-target workspace Clippy with warnings denied: PASS.
- Full workspace: 240 passed (184 core, 12 CLI, 28 browser protocol, 16 desktop).
  The 21 new core test entries include a subprocess helper; the parent actually kills
  that process and verifies kernel-lock release and recovery of its running job.
- Full workspace/all-target MSRV check and 196 core/CLI MSRV tests: PASS.
- Real staged CLI: enqueue, list, dedupe, conflicting input, real FTS completion/result,
  queued cancellation, explicit terminal forgetting/reuse, source bytes/search anchor: PASS.
- npm lint/typecheck/build, 39 UI / 12 extension / 6 tooling tests: PASS.
- Python roadmap / CI / browser protocol / accessibility contracts: 24 / 8 / 10 / 9 passed.
- v0 fixture: 3/3 exact sources; PDF fixture: 4 indexed / 3 expected unsupported,
  seven expected outcomes with no unexpected failure.
- Real native host: accepted capture, four deterministic refusals, three unpaired callers,
  and recovery: PASS. No browser profile or user document was used.
- Desktop development build: PASS (`--debug --no-bundle`); not a signed release.
- Local security script: secret scan, zero npm findings, locked Cargo metadata: PASS.
  This is not an independent security audit and does not resolve the separate Rust advisory #278.
- Offline roadmap: 154 active / 4 retired / 20 quarters / 141 parents / 314 prerequisites valid.
- Read-only live reconciliation: same counts, zero mutations and no warnings.

CLI was built/staged separately before the desktop binary replaced `target/debug/loom`.
Raw logs and binaries were recorded at `/tmp/loom-jobs-evidence.INpfja`.
That scratch directory was observed missing during later approved-file verification on
2026-10-03. The digests below record historical runs, not currently accessible raw logs.

## Race, failure, and negative fixtures

The core tests inspect actual persisted rows and derive a real repaired FTS projection:

- two SQLite connections admit the same key concurrently and obtain one stable ID;
- conflicting/invalid input cannot rewrite the prior request;
- pending/retained bounds reject new work without eviction, while explicit terminal
  forgetting frees capacity and deliberately forgets only that deduplication key;
- queued/running cancellation, immutable completion, retry due-times and exhaustion;
- priority bursts and oldest eligible dispatch across separately acquired workers;
- stale epochs/tokens cannot publish or complete after recovery;
- live ownership rejects a second worker; an actual killed process releases ownership;
- portable export excludes the queue; valid restore clears/fences it, invalid restore rolls back;
- worker acquisition does not implicitly rebuild a damaged derivative;
- a separate worker connection runs while the interactive connection's Rust mutex is held;
- symlink database aliases converge, hard-linked aliases and symlink lock files are refused;
- unsupported changed schema, malformed queue policy, and permanent-vs-transient SQL errors;
- diagnostics remain byte-bounded, and invalid operational policy does not hide canonical search.
- completion/failure-versus-forgetting returns the transaction's terminal snapshot;
- changed database file identity or runtime table shape is refused before worker execution;
- malformed JSON and oversized UTF-8 diagnostics/results cannot be persisted.
- a legacy queue upgrades without losing queued/running/completed rows, policy, epoch,
  sequence or burst accounting; valid restore then clears/fences it;
- invalid legacy result data rolls back runtime migration without dropping rows or
  preventing canonical evidence access; all actual CLI queue commands refuse hard links.

The newly added restart-fairness test failed against the first local queue implementation:
worker acquisition reset burst accounting, allowing a new high-priority job to bypass the older
low-priority job indefinitely across one-shot CLI runs. The counter now persists across
acquisitions; the same regression passes in the full stable/MSRV suites. The failed log is retained.
Initial compiler/Clippy errors and one Markdown line-length failure were also fixed before merge.

Final review found two additional blockers in the first local implementation: ordinary CLI
opening implicitly repaired FTS before worker ownership, and a post-settlement lookup raced
explicit terminal forgetting. The real CLI corruption regression fails against the initial
binary (`jobs-cli-ownership-before.log`), then passes with no-rebuild queue opening and ownership
acquired before SQLite access. Empty/cancelled/contending work and queue-only commands leave FTS
damaged; a claimed repair records `before.healthy = false`. Both completion and failure now
return their snapshot captured inside settlement, with deterministic forgetting-before-return
fixtures. Review also prompted stable canonical database identity and runtime-shape validation.
Two initial harness mistakes (external-content FTS row counting and error-name case) were corrected
before the genuine failing/passing comparison; they are not evidence of product failures.

Re-review required transactional runtime migration instead of only rejecting the older DDL.
The runtime-only version-2 marker and upgrade/rollback fixtures now cover that compatibility
boundary. Hard-link admission is consistently refused, not just worker execution.
Static re-review found no blocking issue in this deliberately FTS-only adapter. The remaining
public-API contract note was addressed: ordinary `Library::open` handles also reject queue
operations against incompatible runtime layouts, without preventing canonical evidence access.
Policy corruption still permits bounded diagnostic inspection; admission/execution refuses it.

The final stable capture suite took 91.20 seconds but passed without a restart. A retained
three-second process sample during the delay shows Vision/CoreRecognition/ANE waits, not a
queue SQLite lock. This observation does not establish a provider-time bound or diagnose the
underlying OS cause; it reinforces the need for bounded extraction before the full engine closes.

## Limits and next acceptance gates

FTS repair is a single maintenance transaction. Cancellation that commits before publication
prevents it; cancellation arriving after publication begins may observe completion. These tests
do not establish bounded rebuild execution time/memory or interactive-search p95. Dispatch
fairness is not a wall-clock guarantee while an unsliced maintenance operation runs.
Existing foreground writers use their existing SQLite/scope contracts, not queue ownership.

Before #36 can close, indexing/OCR must consume bounded slices and queue+scope+OCR fences,
semantic/FTS work needs bounded staging/publication or measured maintenance limits, and the
desktop must actually use durable admission/progress/cancellation/relaunch. Retain resource,
search-responsiveness, extraction/crash, and merged-main proof for those adapters. No new source
scope, ambient capture, networking, model download, external study, or signing credential was added.

## Selected log hashes

| Log | SHA-256 |
| --- | --- |
| `jobs-reviewed-workspace.log` | `b7ad52ca98ce21b422942da34900b44c25fec35738ef9b42ebf9008b53460317` |
| `jobs-reviewed-msrv-tests.log` | `36b882ab7038c0b4ab3a16faf528450f856c87c170efd0e43aa34565f3686578` |
| `jobs-reviewed-cli.log` | `cd15b03cd5860d5b2c9391e10c64df8cc3b8648cdaab69932f2f878b444ed5f8` |
| `jobs-fairness-before.log` | `eaefa669fec9a9d1104924795dfe7f99554502c7acf429510c04f09db5b92f69` |
| `jobs-cli-ownership-before.log` | `ad865d9d05568847934269c99994077ec72526f43fb020f12b1691270f05fd01` |
| `jobs-reviewed-frontend.log` | `6e75fff1af65dc389af8c00a713864d430951fb57bef7c0a2a60e51b93c3b0fe` |
| `jobs-ship-capture-sample.txt` | `e80cfe460f4731189e08ac0867c171cb68c2c13dafc7ae20bae3bd73df209a2b` |

## Merged-main reproduction

[PR #316](https://github.com/AlisinaDevelo/LOOM/pull/316) merged normally on
2026-10-03 at 02:57:00 UTC as `39b8d2f7ed4bed297b48b723eecd195254fc2d62`.
The tested PR head `aee4a804fe52f2e3d7dfd38a45f77c0a971c9eb0` and main both have tree
`f56eb5b72e75623ed213be408350ce5baeb5bb90`; their complete diff is empty.
No hosted-check wait, administrative merge, protection change, or required-check bypass was used.

On clean main, all 240 workspace tests passed again. A CLI rebuilt on main passed the actual
queue corruption/ownership/hard-link/cancellation/forgetting fixtures and 3/3 v0 recovery queries.
Native-host acceptance/refusal/recovery, frontend 39/12/6 tests plus lint/typecheck/build, and the
local security script passed again. The separate Rust advisory #278 remains open; no issue
was closed by this partial engine. GitHub has no open PRs and 122 open / 44 closed issues.
The read-only roadmap plan still has zero mutations and no warnings.

| Merged-main log | SHA-256 |
| --- | --- |
| `jobs-merged-workspace.log` | `00dc93b5517f1254d10911db931cd230bbab9a4571de37a1daa2445ae8fd277f` |
| `jobs-merged-cli.log` | `cd15b03cd5860d5b2c9391e10c64df8cc3b8648cdaab69932f2f878b444ed5f8` |
| `jobs-merged-v0.log` | `67a8eba8576c0d241509ebaeda3e02fbd439e98f81f7000e0786b9d4314ef1ea` |
| `jobs-merged-native-host.log` | `babf157dc05792fdd7a07e6332690413dfceacc492c315157fa903dab97e59e5` |
| `jobs-merged-frontend.log` | `33f9103ce7c0d97952d5c118225a63ee4b09176281d28993680a973bc56fa0b9` |
| `jobs-merged-security.log` | `5e13b8393973b8070e13ceb8230fd5952dd1efbe888b389e585df45350bf3eab` |
