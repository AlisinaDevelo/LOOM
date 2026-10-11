# Issue #348: interrupted foreground resume revalidation

Recorded 2026-10-11. This receipt records focused local regression and recovery evidence for
[issue #348](https://github.com/AlisinaDevelo/LOOM/issues/348). Candidate identity and integrated
verification are recorded below; hosted gates and merged-main reproduction remain pending.

## Provenance and device

| Field | Recorded value |
| --- | --- |
| Original behavioral-red and focused-green source base | `7d396ecb4df64db69a427293a0892a08575bfc5d` |
| Integration base, after PR #349 | `f656636f3c88105536578587d80ea6a8af264b8b` |
| Candidate code SHA | `eb9f73b1d0f586005c476a9c1a14aba28bf2abe4` |
| Merged-main SHA | Pending merge and reproduction |
| Device / architecture | Apple Silicon Mac, model `Mac17,3`; `arm64` |
| OS | macOS 27.0.1, build `26A434` |
| Rust | `rustc 1.99.0 (b940084d7 2026-09-28)`; host `aarch64-apple-darwin`; LLVM 23.1.1 |
| Cargo | `cargo 1.99.0 (5f94df478 2026-08-27)` |
| Build isolation | Dedicated worktree target; `CARGO_INCREMENTAL=0`, `CARGO_PROFILE_DEV_DEBUG=0`; offline, locked dependencies; final suite used a separate temporary Cargo registry cache |

The retained results below use an isolated target. Earlier shared-target results are superseded
because concurrent worktree builds could reuse another worktree's outputs. The original red used
the original production `store.rs` at the recorded source base with the regression fixtures added.
Focused greens tested the corrected working-tree implementation before integration with PR #349.

## Behavior and boundaries

Before reusing a completed supported prefix, foreground resume and approved-root reconciliation
read and hash current bytes through bounded, no-follow stable reads. Reuse requires the active
canonical artifact/version, content hash, media type, extractor contract and current byte/page
limits to match. Hashing holds no connection mutex or database transaction and invokes no parser
or OCR provider. A changed, unavailable, failed or missing canonical unit rewinds to the first
affected ordinal so normal ingestion can retry it. Earlier verified units remain skipped.

Writer transactions revalidate source/OCR consent, canonical projection and the captured job
state, cursor, discovery fingerprint and update revision before reuse or rewind. A final snapshot
check also fences healthy and deliberate skip-only continuations. Unsupported files and images
skipped under the same disabled OCR policy remain deliberate skips. No schema migration or new
source grant is introduced; approved-root reconciliation preserves the selected scopes.

`bytes_read` includes successful prefix-validation reads. Unit counters such as `attempted` count
the subsequently processed units. Cancellation is checked before and between prefix reads and
after a read. A pre-cancelled resume reads zero source bytes and preserves its checkpoint and
canonical evidence. Previously committed units survive cancellation.

These checks observe current bytes and a checkpoint snapshot. They do not provide an atomic
filesystem snapshot or serialize concurrent whole-scan writers after the helper returns. See
[the data-model contract](../DATA_MODEL.md#durable-indexing-jobs) and
[directory discovery](../DIRECTORY_DISCOVERY.md).

## Issue acceptance mapping

| Issue #348 criterion | Focused artifact and evidence | Status |
| --- | --- | --- |
| The first completed resume/reconciliation reflects edits in the completed prefix. | `LOOM-348-PREFIX`: cancellation fixtures validate an edited prefix with unchanged byte length and modification time, a recovered prior failure, the pending suffix and preservation of an earlier healthy prefix. The restart fixture closes/reopens the library, reconciles the approved root, exposes the new marker and removes the old marker on that first completion. | Passed locally; merged-main rerun pending |
| Unchanged interrupted scans retain correct idempotence, consent fencing and cancellation behavior. | `LOOM-348-FENCES`: all 16 public integration tests plus three private snapshot/consent tests pass. Controls cover unchanged resume without duplicate versions, pre-cancelled checkpoint preservation, deliberate unsupported/OCR-disabled skips, stale rewind and healthy continuation, terminal/ABA job revisions, and superseded source consent. | Integrated candidate passed; merged-main rerun pending |
| Retain regression, restart/recovery and merged-SHA evidence with device/toolchain; do not silently broaden selected scopes. | `LOOM-348-RECOVERY`: both behavioral reds and the focused greens are retained by SHA-256 below with source base and device/toolchain. Startup reconciliation leaves the complete `source_roots` table unchanged and never indexes an unapproved sibling. Existing selected-locator, denial, revocation and symlink controls pass. | Candidate code identity, local provenance and scope evidence recorded; merged-main identity and final gates pending |

## Public regression coverage

The focused command ran all tests in `cancellation`, `durable_index` and `source_roots`:

| Target | Passing tests |
| --- | --- |
| `cancellation` — 6 | `cancellation_commits_complete_units_and_resumes_to_uninterrupted_rows`; `resumed_scan_revalidates_an_edited_completed_prefix`; `resumed_scan_retries_a_recovered_completed_failure`; `resumed_scan_keeps_unsupported_and_ocr_disabled_prefix_units_skipped`; `pre_cancelled_resume_keeps_the_completed_prefix_and_checkpoint`; `pre_cancelled_token_reports_all_remaining_units_without_writing` |
| `durable_index` — 4 | `interrupted_job_commits_units_and_resumes_idempotently`; `startup_reconciliation_revalidates_the_interrupted_completed_prefix`; `approved_root_observation_reconciles_rename_delete_overflow_and_restart`; `event_reconciliation_rejects_unapproved_roots` |
| `source_roots` — 6 | `denied_root_is_visible_and_reconcile_fails_without_fallback`; `symlink_replacement_is_unsafe_and_never_followed_on_reconcile`; `moved_root_reports_missing_and_requires_explicit_reselection`; `revocation_hides_existing_evidence_until_explicit_reselection`; `reselection_after_revocation_does_not_resume_an_old_partial_scan`; `persisted_root_reopens_and_reconciles_only_the_selected_locator` |

The three private unit controls are:

- `resume_prefix_revalidation_cannot_rewind_a_newer_checkpoint_revision`: an obsolete snapshot
  cannot rewind newer progress, including a completion/restart with the same job ID, cursor and
  fingerprint but a newer update revision.
- `resume_prefix_revalidation_rejects_stale_healthy_or_skipped_prefixes`: unchanged text,
  unsupported and OCR-disabled prefixes reject terminal and ABA checkpoint states before
  continuation.
- `resume_prefix_revalidation_refuses_superseded_source_consent`: revocation, revoke/reselect,
  purge/replacement and restore/incarnation rotation reject stale authorization without changing
  canonical tables or the checkpoint.

Fixtures use synthetic temporary sources and the repository OCR golden image for the disabled-OCR
skip control. These focused tests invoke no native OCR provider and use no real library, browser
profile or screen content.

## Behavioral reds and focused checks

The original implementation failed the edited-prefix, recovered-failure and startup-reconciliation
regressions: each new-marker assertion returned zero hits instead of one. Seven controls passed.
This is a behavioral red, not a compilation failure.

A separate continuation control failed against the earlier candidate helper before its healthy
continuation fence: `text/terminal=false accepted an obsolete continuation`. That fixture passes
with the final snapshot checks and also exercises deliberate skip-only prefixes.

| Check | Recorded result |
| --- | --- |
| Original production implementation, `cancellation` and `durable_index` | 7 passed; 3 expected behavioral failures |
| Earlier candidate helper, stale healthy/skipped continuation control | 1 expected behavioral failure |
| Corrected implementation, three public targets | 16 passed; 0 failed |
| Corrected implementation, `--lib resume_prefix_revalidation` | 3 passed; 0 failed; 177 other unit tests filtered out |
| `cargo clippy --offline --locked -p loom-core --lib -- -D warnings` | Passed |
| Rustfmt on the three changed Rust files; `git diff --check` | Passed |

The focused Cargo commands, with the isolation settings above, were:

```sh
cargo test --offline --locked -p loom-core --test cancellation --test durable_index --no-fail-fast -- --nocapture
cargo test --offline --locked -p loom-core --lib resume_prefix_revalidation_rejects_stale_healthy_or_skipped_prefixes -- --nocapture
cargo test --offline --locked -p loom-core --test cancellation --test durable_index --test source_roots --no-fail-fast
cargo test --offline --locked -p loom-core --lib resume_prefix_revalidation -- --nocapture
```

Raw logs are retained locally outside public source commits. The basenames identify compact
receipts; their digests bind the retained bytes, not a candidate commit identity.

| Retained artifact | SHA-256 |
| --- | --- |
| `loom-checkpoint-red.log` — original behavioral red | `fdd6643974cb939a8a6356a2b635bae28f150972740a012fdf4a26a0c2141e67` |
| `loom-checkpoint-review-red.log` — stale continuation red | `f178bcabac4e88c80bdd5d479babad40feb7ff0a8b3d7798ae74167b81338b41` |
| `loom-checkpoint-green.log` — 16 public tests | `168e5bd50cdd81e4c202935b427489da02e79166ecf5cf20dcb00d004541f9e3` |
| `loom-checkpoint-fences.log` — three private guards | `f25261efd1ae14d4d7f86a8430ec95422dc743d1892c215e13254d0c8a25e284` |
| `loom-checkpoint-clippy.log` — core library Clippy | `d95abf8dfc8c35a1484e292dc96f9401bb1d63e6c7b88c98c4d0c42d88572de6` |

## Integrated candidate checks

The candidate code commit above includes PR #349. All 16 public tests and three private guards
passed again in this integrated tree. Workspace Clippy with all targets and warnings denied,
workspace Rustfmt, `git diff --check`, `npm ci`, `npm run check`, and `make roadmap-check` passed.
The frontend check includes 59 Vitest tests, 15 extension tests, six markdown-tooling tests, lint,
typecheck and production build. No frontend behavior changed in this candidate.

| Integrated artifact | SHA-256 |
| --- | --- |
| `loom-checkpoint-integrated.log` — 16 public tests | `27346231f0817ba896ee7891332ba2801555d9609ca66d6e330f4709a315f92e` |
| `loom-checkpoint-integrated-fences.log` — three private guards | `721d576fe944b8f1369bde09906041844a41d4413fa91ca99c6e70ca946167b8` |
| `loom-checkpoint-integrated-clippy.log` — workspace/all-target Clippy | `6626fd295d6cc04ea892ee11bcafd9a5164ba76b50c642f887449ab3932c2bd5` |
| `loom-checkpoint-npm.log` — frontend check | `4f30c958fde35a7bf902c18b71e03b6969649c1e3a8200fc49de5486aee213ef` |
| `loom-checkpoint-roadmap.log` — roadmap and protocol contracts | `220bfc4e64a55f20d5c04a29860972a3479dbddfe37011554f52d86c8814cbf6` |
| `loom-checkpoint-workspace.log` — full workspace, including native failure | `686da2d3c402a353158996ad05ad35a99ca3ab617ec49a1a956b98c085f3be69` |
| `loom-checkpoint-benchmark.json` — synthetic retrieval smoke | `ab53cbd9330e617ea268cb3480be0c38a9eaeb5830452d982a2326c074cc64d3` |

The production extraction helper built successfully. The full locked, offline workspace suite
completed with **419 passed and one failed** across 38 targets. The only failure was
`jobs::tests::persistent_retrieval_during_native_queued_ocr`, which reported provider refusal.
Its core unit-test cohort finished in 38.43 seconds. Untouched main and the preceding isolated
PR #349 candidate reproduce this native Vision problem; see
[issue #338](https://github.com/AlisinaDevelo/LOOM/issues/338) and
[the earlier device receipt](2026-10-11-recovery-fixes.md). The local native queued-OCR result is
an explicit **no-go**; this change makes no OCR mitigation claim and retains the failure.

The CC0 v0 retrieval smoke passed: three sources indexed, three positive queries, exact-source
recall at 1 and 5, mean reciprocal rank and anchor precision all 1.0, and no reported failures.
It recorded 533 source bytes read, 679,616 database bytes, 8.517 ms indexing and 0.614125 ms p95
query latency. This tiny fixture contains no negative or reformulation queries and establishes
no real-world retrieval quality or native OCR/resource gate.

The README now links the observed verification limits instead of asserting an unconditional
local workspace pass. Independent source review found a stale healthy-prefix continuation gap;
the correction and its behavioral red/green control are recorded above. Final documentation
review corrected two stale pending-status cells; Markdown lint and whitespace checks passed.

## Pending hosted gates and merged-main evidence

Record actual hosted gates, the merged-main SHA, and the focused regression/restart/fence rerun
on that merged SHA before closing issue #348. This receipt claims no full-workspace success,
hosted success or merged-main reproduction.
