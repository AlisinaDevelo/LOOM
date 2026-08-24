# LOOM 0209 intentional-capture device evidence

This record documents the implementation for
[issue #28](https://github.com/AlisinaDevelo/LOOM/issues/28), reviewed and merged
through [PR #207](https://github.com/AlisinaDevelo/LOOM/pull/207). The issue
remains open: the permission/onboarding criterion still needs a consented
interactive macOS capture session, and the final packaging step was blocked by
the target device's disk capacity. No screenshot was taken for this record; the
existing OCR fixture is already cropped to the relevant evidence region.

## Target and source

| Field | Value |
| --- | --- |
| Implementation commit | `907cf128996cc6bab63b03b4ee561bc7f19d7fdc` |
| Merged-main commit | `78ed38911c9c4414b383ce3231a25cf3bf47bff0` |
| Device | Apple Silicon Mac, `arm64` |
| OS | macOS 26.6.2 (`25G83`) |
| Rust | `rustc 1.96.0`; MSRV `rustc 1.88.0` |
| Node/npm | Node `v26.7.0`; npm `11.19.0` |
| Pre-merge pipe | `CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_PROFILE_DEV_STRIP=symbols CARGO_PROFILE_TEST_STRIP=symbols CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 ./scripts/verify-device.sh /tmp/loom-0209-pre-device` |
| Pre-merge result | All stages passed through security; Tauri compiled the crates but failed archiving `libloom_lib.a` with `ENOSPC`. |

The pre-merge evidence directory is `/tmp/loom-0209-pre-device`. Its
`commands.txt` SHA-256 is
`80ccc347628ca299f050d6666e2a081a3c972e92d29f242a04915debd31bf0d8`,
`summary.txt` SHA-256 is
`be656b908869077c668eb2cd86e3bedbb8b43ec4cf446f03c633d2efb90fa2a2`,
the performance report SHA-256 is
`3ee6b1df1de8e177a07f17adaa56d369a9bfd3b5c1bf5b91f8852576a00f68f3`, and the
log manifest SHA-256 is
`291e7376c2fc2338a63a0c4d0864556c6980a232b339022e14299cc1d5eb4ec6`.
The failed Tauri log SHA-256 is
`e72f80f7f8fcc8cc2c850c3618be0c655badafbf01b9d90244d8eb4e8ab85264`.

## Acceptance mapping

| Acceptance criterion | Evidence and boundary |
| --- | --- |
| macOS permission onboarding requests only the chosen capability and explains denial/recovery | **Not complete.** `capture_intentional` invokes `/usr/sbin/screencapture` only after an explicit button press, requests no background timer, and returns a clear Screen Recording denial/cancel recovery message. Tauri capability entries are command-scoped, and the static Tauri contract test passed. A real consented interactive permission grant/deny/retry session was not run, so this criterion stays open. |
| Original pixels, hash, capture time, display scale, bounds, and available context are stored before OCR | `crates/loom-core/tests/capture.rs::intentional_capture_metadata_is_recorded_before_ocr_and_duplicates_are_stable` passed on the target Mac. It uses the rights-clean, deliberately cropped `tests/fixtures/ocr-golden.png`, asserts capture metadata is present in extraction metadata before OCR, verifies BLAKE3-addressed duplicate stability, and verifies exact purge removes artifacts, versions, passages, and search visibility. The native picker itself was not exercised. |
| Exclusions, pause, duplicates, purge, and no background capture are regression-tested | `src/App.test.tsx` passed 18 tests, including explicit capture, exclusion, pause, disabled capture controls, and purge messaging. The Rust Tauri contract test passed for screen/window/region argument shapes and normalized exclusions; the core fixture passed duplicate/purge behavior. There is no periodic/background capture path in the command implementation. |

## Target-device verification

The pre-merge pipe passed rustfmt, warnings-denied workspace Clippy, locked Rust
workspace tests, Rust 1.88 check/tests, v0/v1 retrieval, hybrid ablation,
semantic contract, mixed-corpus failure/recovery, 10k/100k performance budgets,
accessibility contract, `npm ci`, the full frontend check, and the local
security scan. Hosted GitHub Actions were pending and were not used as evidence.

The 100k release gate passed all six budgets across two runs: indexing
throughput `1006.76–1157.68` artifacts/s, warm p95 `0.444208–0.517166` ms,
maximum RSS `85,606,400–118,734,848` bytes, database amplification
`10.0674–10.1320×`, CPU `0.6563–0.6688` seconds per 1,000 artifacts, and FTS
rebuild `6.3880–8.4931` seconds. The v0 fixture remained exact (Recall@1/5
`1.0`, anchor precision `1.0`, false-positive rate `0.0`, completeness `1.0`).
The v1 fixture retained its documented q008 no-result and q009 duplicate
hard-negative; hybrid remained eligible with exact source and anchor metrics.

On merged main (`78ed38911c9c4414b383ce3231a25cf3bf47bff0`), the focused capture
test passed (`1` test), the frontend production build passed, and the local
security scan reported no leaks and `0` vulnerabilities. A merged-main Tauri
unit/build attempt reached compilation but could not write Rust artifacts after
the volume reached 100%; the failure was `ENOSPC`, not a compiler or test
assertion. The full frontend check likewise completed lint and all `18` tests
before its build substep hit the same disk limit; a clean standalone build then
passed. This limitation must be cleared before claiming a packaged-device
release.

## Reproduction limits

This record does not claim an interactive Screen Recording permission test,
native picker behavior, signing/notarization, or a packaged Tauri artifact. It
does not claim background capture; the implementation deliberately excludes it.
No user files or live screen contents were read or uploaded. The live issue stays
open until the permission path and packaging are exercised on a device with
enough free space, with any captured evidence cropped to the relevant region.
