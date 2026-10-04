# Approved-directory queue: local verification

## Scope and device

Code verified at `eb946e9e1b615e4464a9786541f82065a808b4e3`, based on main
`6ef8993cca5ccd1a089b928c64821ef630530b4f`. Verification date: 2026-10-04.
Device: MacBookPro17,1, Apple M1 arm64, 8 GiB RAM, macOS 26.6.2 build 25G83.
Toolchains: Rust 1.96.0 and 1.88.0, Node 26.7.0, npm 11.19.0, Python 3.9.6.
Rust used one build job, no incremental compilation and no debug information.

The CLI now discovers an already-approved exact directory, admits one durable bounded
manifest, and extracts one file per claim. Publication, counters, cursor and attempt-neutral
yield are atomic. Final namespace validation/reconciliation is a separate claim. Admission
does not grant consent, read file bytes, or create thousands of child queue jobs.
See [the operational contract](../BACKGROUND_JOBS.md) for bounds and explicit runtime-v5 upgrades.
[#36](https://github.com/AlisinaDevelo/LOOM/issues/36) remains open.

## Actual local checks

This was a local command-by-command pipeline, not a successful hosted CI run or release approval.
The security step failed; the overall verification is not globally green.

- Full Rust workspace: **373 tests passed**, repeated against the committed candidate.
- Rust 1.88 full-workspace/all-target check and **308 core/extraction tests**: PASS.
- Full-workspace/all-target Clippy with warnings denied, format and diff checks: PASS.
- Frontend lint/typecheck/build, **43 UI, 12 extension and 6 Markdown-tooling tests**: PASS.
- Five-year manifest/graph and **33 Python repository tests**: PASS.
- CI contract (**14 tests**) and accessibility contract (**9 tests**): PASS.
- Real staged CLI: directory text/PDF quanta, independent worker processes, fairness,
  explicit unsupported/OCR-off skips, exact PDF page anchor, final missing-state cleanup,
  duplicate delivery and purge: PASS.
- Existing maintenance/file CLI and finite directory-discovery fixtures: PASS. The file
  fixture also ran native macOS Vision OCR. No retained v2/v3 binary was supplied to this run;
  those optional old-binary interoperability checks were not performed.
- Recognized legacy/v2/v3/v4 runtime fixtures preserve jobs and canonical data during the
  explicit owned upgrade; unsupported definitions/extra directory objects fail closed: PASS.
- Tauri `--debug --no-bundle` build: PASS. No signing/notarization or new interactive session.
- Secret scan: no findings. npm audit: zero findings. Locked Cargo metadata: PASS.
  Strict Cargo audit: **eight denied warnings**, unchanged dependencies. No advisory
  suppression, target filter, MSRV increase or Tauri 2.12 upgrade was introduced.
  [#329](https://github.com/AlisinaDevelo/LOOM/issues/329) remains open.

## Failure and privacy coverage

Fixtures verify admission rollback, no consent reactivation, exact-file/nested-root ownership,
unsupported paths, tombstone refusal, capacity rollback, canonical/content/unit CAS,
atomic publication/cursor rollback, namespace changes before final cleanup, and actual
killed-worker recovery both before and after a committed quantum. Live blocked extraction
is interrupted/reaped by cancellation, revocation, purge, restore, OCR revision, and
foreground canonical changes without publishing stale evidence.
Normal foreground exact-file reparenting invalidates already-processed admission-absent units.

Source reads validate the actual opened root and child before allocating/reading bytes.
A deterministic preparation-to-open root replacement with the same hard-linked child inode
is refused before extraction. Birth-time and root-change fixtures simulate device/inode ABA;
these are not claims of forensic authenticity or hostile filesystem protection.
Discovery carries descriptor-bound child birth time into admission; a simulated recycled inode
between discovery and manifest insertion cannot be silently recaptured with a new identity.
Missing/symlink/wrong-kind namespaces return typed source-change/refusal outcomes.

Root/artifact/OCR purge invalidates matching work and removes manifest paths. A malformed
empty target refuses the entire purge without canonical deletion; explicit cancellation and
terminal diagnostic forgetting recover deletion. Canonical-only locator delete/update fences
prevent absent/create/purge ABA on managed foreign-key-enabled connections. Portable and
encrypted backup restore retain evidence but exclude/clear operational manifests and claims.
Unknown v5 private runtime objects refuse purge and restore. Job-bound unit digests and
canonical relations prevent malformed valid-shape units from evading artifact purge.
Bulk deletion validates once in its writer transaction; a permit cannot apply to another connection.
These are application-level deletion checks, not secure-erasure proofs for SQLite/WAL or storage.

## Small-fixture measurements and limitations

The five-file CLI fixture contains two texts, one PDF, an unsupported file and an OCR-off image.
Measured helper quanta use a 25 ms sample interval and an installed address-space guard:

| Unit | Helper wall ms | CPU ms | Peak resident bytes |
| --- | ---: | ---: | ---: |
| First text | 139 | 4 | 9,273,344 |
| Second text | 67 | 4 | 9,273,344 |
| PDF | 62 | 6 | 10,633,216 |

These are three functional-fixture measurements, not a large-folder latency/memory gate.
Admission/final discovery and capped reconciliation remain synchronous bounded phases.
FTS repair remains unsliced. Semantic rebuild, desktop queue admission/progress/cancellation,
and aggregate resource/search responsiveness evidence remain required before #36 can close.
Non-Unix systems and filesystems without birth-time identity refuse this adapter. Namespace
validation establishes observed stability, not an atomic filesystem/SQLite snapshot.

The existing retrieval smoke benchmark recovered 3/3 v0 sources. v1 positive-source Recall@1/5
remains 0.9 with anchor precision 1.0; its existing missed source and negative-query false
positive remain unresolved. Successful benchmark execution is not a retrieval-quality sign-off.
No private documents, browser profiles, permission settings or uncropped screenshots were used.

## Retained artifacts

Local evidence ID: **approved-directory.yApxtW**. Receipts and staged CLI/helper are retained
in the ignored evidence directory, outside temporary build caches. Public output contains
only synthetic measurements and digests, not raw paths, logs or source documents.

| Artifact | SHA-256 |
| --- | --- |
| Staged CLI | `7de0a5d574a5f70175a875d04ebd6382da2559d08c6f941d448edb4a30681c8d` |
| Staged helper | `52c9e5f99fc181ed51df8ec030b9d6bd783c784571e9918024ed122eca683766` |
| MSRV tests | `f7e2afa21f77bead3b9b38666b585f6e6d44d97264fbc3b5f8452c4036ee09d1` |
| CLI measurements | `3a7983a2e715e3b5985ba33dfb68066204c11092a5dc9c875e902550906e2ec8` |

Merged-main reproduction will be recorded separately after merge; this document does not
claim the candidate has already merged or that the remaining engine acceptance gates passed.
