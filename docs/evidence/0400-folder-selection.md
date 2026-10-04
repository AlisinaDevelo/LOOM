# 0400: atomic first-folder queue admission

Recorded 2026-10-04 on an Apple Silicon Mac. This is local implementation/process evidence,
not interactive desktop acceptance, a signed release, or a real-world retrieval study.

[PR #343](https://github.com/AlisinaDevelo/LOOM/pull/343) merged as
`9c97f66ab0d36afc2ddf92d229bd84437cf9fbda` at 21:20:38 UTC. Its complete tree
`b08ccce506beaad4142033cacf8b4cb0029f1291` exactly matches tested candidate
`2d1db6cfe486fa77385d9bb4bc1867ffc909b535`.

[Issue #36](https://github.com/AlisinaDevelo/LOOM/issues/36) remains open. This adds the core/CLI
admission prerequisite, not a desktop worker or background-indexing controls.

## Outcome and fences

`Library::select_and_enqueue_directory` and CLI `select-and-enqueue-directory PATH KEY` make
first-folder selection possible without first extracting/indexing its contents. Finite metadata
discovery runs outside SQLite. One IMMEDIATE transaction verifies the observed consent and
filesystem identity, grants/reselects the exact root, admits the job and inserts its manifest.
Any admission refusal rolls back root state, job/sequence changes and semantic invalidation together.

Same-key/folder/priority replay returns the original job without refreshing consent or last-seen,
even after revocation or changed membership. Concurrent identical admission is replayed inside
the writer transaction before reauthorization. A new explicit selection needs a fresh key.
The existing `enqueue-index-directory` remains incapable of granting consent.

Both admission paths pin device/inode/birth/change before discovery. Retained nested directory
stamps are rechecked under the writer before and after manifest insertion. This is finite observed
stability, not an atomic filesystem/SQLite snapshot. Runtime v6, canonical schema 10 and the
portable archive format are unchanged. See [the full contract](../BACKGROUND_JOBS.md).

## Regression controls

The initial seven-test integration target failed on the unimplemented selection API. A separate
identity control then demonstrated replacement between selection and discovery. Review controls
reproduced refresh identity loss and nested namespace drift before admission; both failed before
the fences were added and passed afterwards. Compilation-error logs were kept separate from
these behavioral red controls.

The final fixtures cover:

- First selection with invalid UTF-8 content admits metadata only: zero artifacts, passages and
  legacy foreground checkpoints before a worker runs.
- Replay after completion, revocation or changed membership; key/path/priority conflict; concurrent
  identical admission without consent or manifest mutation.
- Discovery depth and queue capacity refusal for new, enabled and revoked roots; injected manifest
  SQL failure rolling back root state, canonical export and sequence.
- Revoke/reselect, absent-root purge, restore, exact-child ownership and root replacement between
  discovery and admission; nested namespace changes while waiting for the writer.
- Exact-file/nested-root ownership, inactive locators, tombstones and file/directory kind conflict.
- Legacy, published-v6 and staged-v6 semantic rows survive replay/conflict/failed admission;
  successful re-selection invalidates them through the existing source-generation fences.

New file contents are synthetic and created during tests. Process checks use temporary libraries
and synthetic/CC0 repository fixtures; no real library, browser profile or screen content was used.

## Local candidate checks

Environment: Rust/Cargo 1.96.0, declared minimum Rust/Cargo 1.88.0, Node 26.7.0 and npm 11.19.0.
Rust builds disabled incremental compilation/debug information and used two build jobs.

| Check | Result |
| --- | --- |
| Workspace Rust tests | 410 passed |
| Workspace/all-targets Clippy with `-D warnings`; format/diff | Passed |
| Rust 1.88 workspace/all-targets check | Passed |
| Rust 1.88 core/extraction library/integration tests | 345 passed |
| Full frontend check | 43 UI, 12 extension and 6 Markdown-tooling tests; lint/typecheck/build passed |
| Python suite / CI / accessibility / browser protocol | 33 / 14 / 9 / 10 passed |
| Canonical roadmap validation | 154 active, 4 retired, 20 milestones, 141 parent and 314 dependency edges |
| Real CLI selection, directory, file, maintenance, semantic and discovery contracts | Passed; native Vision OCR included |
| Native-host contract; PDF adversarial fixtures; retrieval v0/v1 smoke | Passed |
| `npm run tauri build -- --debug --no-bundle` | Built; app not launched for interactive acceptance |
| Secret scan / npm audit | No leaks / zero vulnerabilities |
| Strict Cargo audit | Failed visibly on eight tracked advisories; no suppression |

The Cargo findings remain tracked in [#329](https://github.com/AlisinaDevelo/LOOM/issues/329) and
[#278](https://github.com/AlisinaDevelo/LOOM/issues/278). GitHub's open moderate `glib` alert is the
same tracked VariantStrIter unsoundness, not a new finding introduced by this change. No dependency
or MSRV decision was made. Hosted CI was not used as proof or awaited for the normal merge.

## Merged-main reproduction

On `9c97f66`, the nine selection integration tests and 25 directory-related unit tests passed
again on Rust 1.88. A fresh stable CLI/helper build on that merged SHA was byte-identical to the
tested candidate pair:

| Executable | SHA-256 |
| --- | --- |
| CLI | `b2c52a392a08bce3e78981d61e82023a2ec8dc7065a11c6078b31677bdd14446` |
| Extractor | `52c9e5f99fc181ed51df8ec030b9d6bd783c784571e9918024ed122eca683766` |

The rebuilt pair passed selection, approved-directory, approved-file/native-OCR and semantic
CLI contracts again. Every selection-contract worker invocation is a fresh process. It verified
cold admission with zero indexed artifacts, durable cancellation after one quantum, fresh-key
resume with an unchanged version rather than duplication, terminal replay, and purge of roots,
jobs and manifests while preserving the synthetic original files. Separate old-version CLI
interoperability binaries were not supplied for this slice; the released-v5 migration unit
fixtures passed, and the prior actual migration evidence remains separate.

CLI and GUI share the output filename `loom`; their executables were staged separately and CLI
fixtures used the verified CLI copy. Raw logs, red controls and matching executables were retained
locally, outside public source commits.

## Remaining gates

Desktop admission/worker lifecycle, progress/pause/cancellation/relaunch controls and interactive
mixed-resource verification remain. Manifest admission and final reconciliation are finite but
unsliced writer phases; sharing the interactive connection mutex would still block search there.
A desktop path needs independently fenced connection ownership and measured reader responsiveness.
The larger resource-budget corpus was not rerun for this slice. This evidence closes neither #36
nor [fair per-root scheduling #78](https://github.com/AlisinaDevelo/LOOM/issues/78).
