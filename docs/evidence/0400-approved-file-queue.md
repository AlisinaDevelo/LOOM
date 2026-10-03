# Approved-file queue: local verification

## Scope and device

Code verified at `e2e3ec324c1f0d648d7c3135f87c7802f0777029`, based on main
`002847ea4dc729b15c4810be7138a57d093aa706`. Later proof/retention edits do not change
the tested Rust, CLI, workflow or fixture code. Verification date: 2026-10-03.
Device: MacBookPro17,1, Apple M1 arm64, 8 GiB RAM, macOS 26.6.2 build 25G83.
Toolchains: Rust 1.96.0 and 1.88.0, Node 26.7.0, npm 11.19.0, Python 3.9.6.
Rust used one build job, no incremental compilation and no debug information.

This adds real text/PDF/Vision OCR refresh to the durable queue for one already-approved
regular file. It does not grant a source, scan a directory, launch a daemon or change desktop
indexing to use this queue. [#36](https://github.com/AlisinaDevelo/LOOM/issues/36) remains open.
See [the operational contract](../BACKGROUND_JOBS.md) for migration and compatibility limits.

## Actual local pipeline

`bash scripts/verify-device.sh EVIDENCE_DIRECTORY` completed all steps. Its overall exit was
**1**, from the Rust advisory gate; it is not a globally green pipeline or release approval.

- Full workspace: **262 passed** (206 core, 12 CLI, 28 browser protocol, 16 desktop).
- Full-workspace/all-target Clippy with warnings denied, formatting and diff checks: PASS.
- Full-workspace/all-target MSRV check and 206 MSRV core tests: PASS. A separate
  `cargo +1.88.0 test --locked -p loom-cli` passed all 12 CLI tests: **218 total**.
- Actual staged CLI: text/PDF refresh, stable artifact/new version, exact anchors, no extra
  `index_jobs` checkpoint, duplicate/conflict, scope rejection, cancellation and purge: PASS.
- Native Vision OCR used the existing cropped synthetic image; region anchors, policy
  reassertion/cancellation, purge and unchanged original bytes: PASS.
- Separate v2 binary from main `002847e`: explicit locked migration, live-owner refusal,
  repeated upgrade, old queue refusal, canonical read compatibility and old-binary purge
  without queued resurrection: PASS.
- Real FTS queue corruption/ownership/repair/cancellation/forgetting fixtures: PASS.
- npm lint/typecheck/build; 39 UI, 12 extension and 6 Markdown-tooling tests: PASS.
- Python contracts: 33 repository, 8 CI, 10 browser protocol, 9 accessibility and 3
  performance-runner tests: PASS. These are not manual VoiceOver or permission receipts.
- v0 recovery: 3/3 exact sources. Adversarial PDFs: four indexed and three expected refusals,
  all seven expected outcomes. Semantic rebuild/drop/rebuild and hybrid ablation: PASS.
- Native messaging host: accepted capture, four deterministic refusals, three unpaired callers
  refused and recovery: PASS. No real browser profile or user document was used.
- Tauri `--debug --no-bundle` build: PASS; not signed/notarized distribution.
- Five-year graph: 154 managed active / four retired / 20 quarters / 141 parents /
  314 prerequisites. Read-only live reconciliation had zero mutations and no warnings.

CLI was built and staged before the desktop binary replaced `target/debug/loom`.
The independent read-only review signed off on this adapter; it did not execute tests.

## Meaning of report results

The runner's `PASS` label means the command returned zero. Reports were inspected separately;
successful report emission is not proof that every quality or release threshold passed.

The v1 corpus reports positive exact-source Recall@1/5 of 0.9 and anchor precision of 1.0.
Its local-text slice is 0.75 recall, and the negative query does not return zero results.
`q008` misses its primary result and `q009` is a false positive. A freshly rebuilt v2 main
binary produced identical quality metrics and failure records. This change does not resolve
those existing retrieval weaknesses, and the fixture is not real-user recovery evidence.

The foreground performance report used two independent runs each at 10k/100k synthetic
Markdown/text artifacts and 31 warm queries per run. At 100k, warm p95 was **31.756833 ms**
in one run and **0.565459 ms** in the other. The first exceeds the 25 ms planning budget;
the report's release gate is **conditional**, with one explicit remediation disposition.
Maximum observed RSS was 112,607,232 bytes. The other five recorded budgets passed.
OS page cache was not dropped; shared-machine load and debug-build conditions are retained.
This is neither a background-search responsiveness test nor a provider peak-memory guarantee.

The security script passed secret scanning, zero npm findings and locked Cargo metadata.
Cargo audit 0.22.2, fresh RustSec revision `f8dee89e1b2f2f1eaf548312df7655fe5202a302`,
denied **eight warnings**: seven unmaintained dependencies and glib's unsound advisory.
No advisory was ignored, severity filtered or target filtered.
[#278](https://github.com/AlisinaDevelo/LOOM/issues/278) remains open; the queue change
does not change the lockfile or resolve the upstream GTK dependency.

## Regression and failure evidence

- The new actual CLI fixture rejects v2's missing `enqueue-index-file` command before this
  feature. The new staged binary completes its text/PDF/OCR and compatibility assertions.
- A purge regression failed before the fix: `IS NULL` matched an unrelated malformed target
  and erased its diagnostics. Explicit enabled selectors now preserve that record through
  artifact/OCR purge; dispatch permanently fails it without creating evidence.
- A missing-identity/delete-first regression completed work before its fix. The required
  deserializer now distinguishes explicit `null` from omission; omitted admission identity
  fails before extraction/publication, including after canonical-only deletion.
- The fixtures exercise source hash changes while waiting for the SQLite writer, foreground
  CAS updates, atomic canonical/parent rollback, scope re-selection, OCR revision races,
  artifact/root/OCR purge, restore, and actual killed-worker recovery/new claim rejection.
- Admission rejects unknown scopes, directories, symlinks, oversized files, unsupported
  formats and OCR-off images. Pending/retained capacity is shared with FTS work.
- v2/unversioned migration preserves bounded records, policy, sequence and fairness under
  kernel ownership; invalid old data/unknown runtime layouts roll back rather than losing jobs.

An initial frontend run also discovered the temporary compatibility checkout and executed
39 duplicated UI tests. The checkout was removed and the normal 39-test run passed again.
One attempted JSON comparison read an unfinished baseline report; the completed comparison
was then rerun successfully. These are harness mistakes, not product failures.

Earlier temporary evidence/tools disappeared during verification. Historical proof documents
now state that retention limit. Fresh evidence ID **approved-file-jT0uqm** is retained locally
outside temporary scratch directories, including raw failed/passed logs, binaries and reports.
Only task-owned inactive build caches were cleaned; originals, source, evidence and shared
Cargo caches were not removed by these cleanups.

## Selected digests

| Artifact | SHA-256 |
| --- | --- |
| Tested CLI | `962196bbd0edcd0602666a831b17fb582351d8d6d429da9a4517871b2f0e311a` |
| Rebuilt v2 CLI | `4965b2bb84cc9738638644cbb372b06b2e01756f49f6a9e40663037a124cb669` |
| Workspace log | `a1846846ffbf1ccb5b29900934c3fb6a6143453825760ba332f71a61170bff94` |
| MSRV check log | `fd74f94442311c2c7991d48c00368f2a2b35048f83df245b28bd713fd0955837` |
| MSRV core log | `a5168c36d0d434ae8471c3a8c181bffde295629db33a88886b89bf067540e66c` |
| Clippy log | `32c83aa614df86ebd1a65e252cf7f2af523ab29d3544adf472e658803b92c654` |
| Native CLI/v2 compatibility log | `0220b3f7797b8b1dd9c4659b530534eb41220b82f61ea5f65bdbf553e2030a84` |
| Purge regression, before | `27afbd16286120561b120f32620b57c62c28ab0bc4b720121ababad995055151` |
| Missing identity, before | `971231c503ba22c2dfc867649289b6a2434d57bd1989cad325f7a208c0217c98` |
| Performance report | `40731e828958f7ede3bff48eaa3c232320bba3e0446a2ce4018704c6fd9915f6` |
| Security log | `d763487c117d1c76e6f3d0dadcdc8f0eedca5c4653910702f10b2221302aee9a` |
| Tauri build log | `dc99d564ac4bd55e1a97fcc1df0cbf6c6c4f8a9262e712796461053e6f29bf4e` |

## Remaining #36 work

Directory quanta, staged semantic/FTS work, desktop durable admission/progress/cancellation/
relaunch, isolated native providers and measured search responsiveness remain unimplemented
or unproved. Current output limits are post-provider limits, not execution/memory isolation.
The in-process adapter also treats OCR-provider unavailability as terminal cancellation;
separating environment failures from consent cancellation belongs in that follow-up.
No issue is closed from this partial adapter or from a report command's zero exit status.
