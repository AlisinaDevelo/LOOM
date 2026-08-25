# Issue 0304 — merged-main privacy and storage-controls evidence

Issue: [#33](https://github.com/AlisinaDevelo/LOOM/issues/33)

Implementation PR: [#228](https://github.com/AlisinaDevelo/LOOM/pull/228)

Implementation commit: `0f24d4da32f9d9e8c72b1abdff0c70ed9edb8dfb`

Merged-main SHA: `73f3f3d03660d7b8f73153fefee1c247ac9daba5`

Roadmap ID: `0304`

Run date: 2026-08-25 (Europe/Rome)

This record closes the four acceptance criteria using the protected `main` reproduction. Hosted
GitHub Actions were not used as evidence; the device runner and focused commands below ran locally
on the target Mac. The first merged-main full-pipe attempt reached every stage but the 100,000-file
performance run exhausted the nearly-full development disk. After removing only generated LOOM
artifacts, the identical merged-main binary passed the 100,000-file performance gate; both outcomes
are retained below rather than conflated.

## Device and toolchain

| Field | Recorded value |
| --- | --- |
| OS | macOS 26.6.2, build 25G83 |
| Device | MacBookPro17,1, Apple M1, arm64, 8 GiB |
| Rust | rustc 1.96.0 / cargo 1.96.0 |
| MSRV | rustc 1.88.0 / cargo 1.88.0 |
| Node/npm | Node v26.7.0 / npm 11.19.0 |
| Python | 3.9.6 |
| Merged source | `73f3f3d03660d7b8f73153fefee1c247ac9daba5` |
| Build mode | one job, non-incremental, symbol-light debug artifacts; targets cleared between large phases |

## Acceptance mapping

| Artifact ID | Acceptance criterion | Retained result |
| --- | --- | --- |
| `LOOM-0304-INSPECT` | Inspect canonical, derived, cache, log, sidecar, and source bytes by source/approximate size | `Library::inspect_storage`, `storage-inspection` Tauri command, CLI `storage-inspect`, and `storage_inspection_accounts_for_sources_and_known_disposable_files` report source/version bytes, canonical/derived estimates, database/WAL/SHM/journal, all six fixed disposable categories, file counts, and existence without following symlinks. |
| `LOOM-0304-PURGE` | Delete by artifact/root/time and verify after restart | `purge_artifact`, `purge_root`, and `purge_before` use one transaction, cascade versions/passages/relationships/bookmarks, rebuild FTS5, checkpoint/vacuum SQLite, and `purge_artifact_removes_evidence_and_survives_restart` plus `root_and_time_deletion_are_explicit_and_retention_is_deterministic` verify zero results and healthy FTS after reopening. |
| `LOOM-0304-DERIVATIVES` | Inspect deletion residue beyond the main database | `disposable_cleanup_removes_only_known_local_derivatives` creates and removes WAL/journal, cache, model-cache, thumbnails, OCR scratch, temporary exports, and logs; it verifies every file is gone while source bytes remain. The merged-main CLI smoke removed eight files (two SQLite sidecars plus six disposable files) and retained the source root. |
| `LOOM-0304-EXCLUSIONS` | Preserve exclusions and private capture boundaries | Existing source-root/capture tests cover denied, revoked, moved, and symlink-replaced roots; the new cleanup test refuses to follow a disposable symlink. The desktop panel exposes inspection and deletion explicitly, while original user-owned files and managed captures are never removed by disposable cleanup. |

## Merged-main device verification

The complete command was:

```text
bash scripts/verify-device.sh /tmp/loom-0304-device-verify-merged-main
```

It recorded merged SHA `73f3f3d03660d7b8f73153fefee1c247ac9daba5`. The run passed formatting,
clippy, the 104-test workspace, MSRV check/tests, CLI build, both retrieval benchmarks, adversarial
PDF, hybrid ablation, semantic contract, performance unit checks, mixed-corpus recovery,
accessibility, npm check, gitleaks/npm audit, and the Tauri macOS debug build. The only failure was
the 100,000-file performance measurement at approximately 24,585 artifacts with SQLite reporting
`database or disk is full`; the 10,000-file run in the same pipe was complete and healthy.

After removing only LOOM's generated `target/`, `node_modules/`, and `dist/` (no active LOOM process),
the exact merged-main binary was rerun:

```text
python3 scripts/performance-budget.py \
  --evidence-dir /tmp/loom-0304-merged-performance-rerun \
  --loom /tmp/loom-0304-device-verify-merged-main/loom \
  --counts 100000 --runs 1
```

The 100,000-artifact run passed with 100,000/100,000 indexed, 1,000.41 artifacts/second, warm
query p95 0.438 ms, 110,804,992-byte maximum RSS, 10.145x database amplification, 0.6895 CPU
seconds per 1,000 artifacts, and a 9.275-second FTS rebuild. Canonical and derived digests were
identical before and after rebuild.

## Test matrix

All commands below were run on this device. The full merged-main log set is under
`/tmp/loom-0304-device-verify-merged-main`; the focused post-pipe logs are retained in `/tmp`.

- `cargo fmt --all --check` — pass.
- `cargo clippy --workspace --all-targets --locked -- -D warnings` — pass; final focused log
  `/tmp/loom-0304-clippy-final-v5.log`.
- `cargo test --workspace --locked` — 24 result sections, 104 passed, 0 failed; final focused
  privacy regression is 4 passed, 0 failed in `/tmp/loom-0304-privacy-controls-final-v3.log`.
- `cargo +1.88.0 check --workspace --all-targets --locked` — pass.
- `cargo +1.88.0 test -p loom-core --lib --tests -- --nocapture` — all reported suites passed.
- `npm ci` and `npm run check` — lint, Markdown lint, 23 UI tests, TypeScript, and Vite build pass.
- `bash scripts/security-check.sh` — gitleaks found no leaks and npm audit reported 0 vulnerabilities.
- `scripts/verify-device.sh` — retrieval v0/v1, adversarial PDF, hybrid ablation, semantic contract,
  mixed corpus, accessibility contract, and Tauri debug build pass.
- Merged-main performance rerun — 100,000/100,000 complete with all six pre-optimization budgets
  passing; report digest is recorded below.

## CLI and MVP demonstrations

The merged-main CLI smoke used a fresh temporary database and a rights-clean, intentionally selected
corpus. It indexed 5 artifacts / 21,816 bytes, recovered a text passage, a PDF page anchor, and a
cropped OCR image region, and reported 5 artifacts, 5 versions, and 7 passages. The OCR fixture is
`benchmarks/retrieval/v1/corpus/screenshot/ocr-cropped.png`, an explicitly cropped 878×191 image;
the OCR result retained a 876×68 region anchor. No full desktop screenshot was captured.

The merged-main privacy CLI smoke inspected all fixed disposable categories, deleted one artifact,
removed eight disposable/sidecar files, reopened with zero artifacts/passages, and verified the
user-owned source file still existed. SQLite WAL/SHM files reappeared after the follow-up CLI opened
the database, which is normal SQLite operation and is why the UI describes cleanup as application-level
deletion rather than secure erasure.

## Negative, privacy, recovery, and resource coverage

- Empty/control-character artifact IDs, invalid roots, invalid RFC3339 cutoffs, and retention values
  outside 1–36,500 days fail closed without mutation.
- Artifact/root/time deletion is transactional and followed by FTS rebuild plus checkpoint/vacuum;
  restart tests verify no stale search result or FTS corruption remains.
- Symlinks are not followed during inspection or disposable cleanup; unknown sibling directories and
  user-owned source/capture files are outside the deletion surface.
- SQLite WAL/journal/SHM, logs, model cache, thumbnails, OCR scratch, and temporary exports are
  explicitly created and checked by the device regression suite.
- The first merged performance attempt's `ENOSPC` was an environmental disk boundary, not treated as
  a functional pass; the cleaned-device merged-main rerun is the authoritative 100k resource result.

## Log and report digests

Logs remain in `/tmp` on the test device; SHA-256 digests identify the retained reproductions without
committing generated output.

| Artifact | SHA-256 |
| --- | --- |
| `/tmp/loom-0304-device-verify-merged-main/summary.txt` | `fbc6558d0e2832e8137107ae0237a7ab2bcf34cf5e88cb3fb4a36d59ecfdaf65` |
| `/tmp/loom-0304-device-verify-merged-main/log-sha256.txt` | `8b978f480d42a1f9706b2bb3f4ecb3c3a7523bd0fc5f95fe7499c3266cbc1ba8` |
| `/tmp/loom-0304-workspace-final-v2.log` | `7bfd83dad574594e464cbc1c8ff7f793e8539b40bcb0d171aaa4e9efea419d95` |
| `/tmp/loom-0304-privacy-controls-final-v3.log` | `663880372431c4486275cd1fc004d53a859f5b3d9f2aa6c49c3e11843c1a32c1` |
| `/tmp/loom-0304-clippy-final-v5.log` | `22e6e9f9db3eb7298442993397f44c7b345ca40d514e55f079b9f86298188054` |
| `/tmp/loom-0304-security-final-v3.log` | `bfa99a735a50254a78f8a352a0c1fb79b40b183c579c4e3b007c4c4b6ad36cc3` |
| `/tmp/loom-0304-demo-merged-main.log` | `d56c677d4ca63a15182bed39a3f993b6f13fa26a73fdf812f710d908fb1195a8` |
| `/tmp/loom-0304-privacy-merged-main.log` | `e64233378ab4f85712360e64f881ab178d9de9247ca1f0299b121795225813cf` |
| `/tmp/loom-0304-merged-performance-rerun/report.json` | `12bf66b087724daf27014c6177d18decd606cda3a347692f1201edb92e51c4f7` |
| `/tmp/loom-0304-merged-performance-rerun/run-100000-1.stdout.json` | `f3ec71414c271ab8ed58ebd36e4a13642f7da2ca396ff85966725bb69258c3eb` |
| `/tmp/loom-0304-merged-performance-rerun/run-100000-1.time.txt` | `35b68eb3581f6ae58dd7a731ada6c7c40da9605395b83ad6c514db9f460af5dc` |

Key merged-main runner log digests:

```text
clippy.log                 7bcdac440804dfef5142946f4096d26b216e90398f768e3848f7e126c4a8a9dc
rust-workspace.log         27145de192f525a88e7193371b8bc81d26234f1c9438f3ed56c3cb308d680e25
rust-msrv-check.log        b49979be02d10dc37c0499ee726739777582ea72cd0a5d82008bec03948e60f1
rust-msrv-tests.log        b64ff5035d1fb9045344b341707af1b09ce0c741c8c98f997c8cb2e7b879ca5b
retrieval-benchmark.log    58c27af5949e63b04c066adb631ae3f896353557fdd83df2387b68f8722ae253
retrieval-benchmark-v1.log 9455f603e7d57c5d3a1ab7f93194b94b468444e11b363078584cf6cc58e3c39d
performance-budget.log     5e2514e16346175a9495a534acdb631a28ab9b684f73313a548b9ba450a8e502
npm-check.log              dd9288f35feaff696795fd76eb46162b1be16d5d2f804523fbc63b66bfcb05bc
security-check.log         17d9029b9e79d5db47029a4b0089c320c59e504c90d24a61f88fe63282848dfb
tauri-build.log            9c45973ff4a1979b4c4e30365e4d6e3d2a85fc67f2fb9890065ed4dec76804af
```

## Closure decision

All four acceptance criteria map to source code, focused tests, CLI output, and the merged-main
device run. The roadmap entry can be promoted to `done`; the live issue may be closed only after
this evidence PR is merged and the final roadmap reconciliation reports zero delta.
