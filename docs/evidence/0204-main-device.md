# 0204 merged-main reproduction

This is the post-merge reproduction for roadmap 0204 / issue [#23](https://github.com/AlisinaDevelo/LOOM/issues/23).
The tested source is exactly `783f99cfc9d67b19b260c0eec2f03a23d3985fe9`, the merge commit for
PR #198, checked out on local `main` before the run.

## Target and run

- Device: MacBook Pro 17,1, Apple M1, 8 GB, `arm64`.
- OS: macOS 26.6.2 (build 25G83).
- Toolchains: Rust 1.96.0 / Cargo 1.96.0; MSRV Rust 1.88.0 / Cargo 1.88.0; Node v26.7.0;
  npm 11.19.0.
- Resource guard: `CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0`.
  This only removes debug symbols and incremental cache to fit the device; it does not alter
  retrieval behavior.
- Evidence directory: `/tmp/loom-0204-main-device.xQXose` (created 2026-08-24 18:52:52 +0200).
- Summary SHA-256: `9f8a41a2154418daffc3849cfccc0c19ad8dfbaf5b89e9fc7398e63f4677e41d`.
- Commands SHA-256: `ae6d3293aebc3aa4e5cd5531514d1a4bf7779a459dcc453a83920e48e7a8e3ed`.
- Final runner status: `PASS`.

## Gate and regression results

The full device pipe passed formatting, warnings-denied Clippy, locked workspace tests, Rust 1.88
checks/tests, v0 and v1 retrieval benchmarks, the hybrid ablation, semantic rebuild/drop/rebuild,
the mixed failure/recovery corpus, `npm run check`, local security/audit checks, and a debug Tauri
build. The canonical hybrid ablation log is
`/tmp/loom-0204-main-device.xQXose/hybrid-ablation.log` (SHA-256
`3e02ad6dbec8bd2740c2a0182699df9506670ae124fda37d8f716cd77d2ab284`).

| Mode | Recall@1 | Recall@5 | Anchor precision | False-positive rate | p95 ms |
| --- | ---: | ---: | ---: | ---: | ---: |
| Lexical | 1.0 | 1.0 | 1.0 | 0.0 | 9.212958 |
| Semantic | 1.0 | 1.0 | 0.0 | 0.6666666667 | 9.340084 |
| Hybrid | 1.0 | 1.0 | 1.0 | 0.0 | 9.515792 |

The gate is `eligible`: accuracy, anchor precision, false-positive threshold, completeness,
latency, lexical non-regression, and failure checks all pass. Semantic-only failures remain in the
report as a diagnostic baseline. The hybrid gate is still an explicit evaluation result; desktop
default promotion remains a separate product decision.

The v1 benchmark also remained stable: 8 fixtures indexed with completeness `1.0`, overall
Recall@1/5 `0.9`, MRR `0.9`, anchor precision `1.0`, and the same two explicit local-text failures
(paraphrase no-result and duplicate hard-negative false positive). Its log SHA-256 is
`8f4608828e97484eec01b311ecd0db6239b24ebe7aa320489a85f166d34edaab`.

Semantic rebuild/drop/rebuild remained repeatable, fail-closed, and evidence-bound; its summary
SHA-256 is `7f54be3ea04ecf5b5d5df1ca9c11095faed6721450ff277dcbba6406a9eb0cfd`. The mixed corpus
still rejects oversized input, does not follow the outside-root symlink, and recovers valid
replacement content; its log SHA-256 is
`f9823515873ec74c634f3b760a65579cbfb01b9f7c8c9c498fbd6207314aef03`.

## MVP demo

The merged-main demo at `/tmp/loom-mvp-demo-0204-main.5A6ct0/demo.log` indexed five selected
sources and recovered text, PDF-page, and cropped OCR image-region evidence. It reported OCR
enabled with two derived passages and five artifacts/seven passages. Demo SHA-256:
`2f8551d9e2d7ab4eca06b2cbf041d96a48bb62fccd86a3cc616ba6a2de500e51`.

This reproduction confirms the merged code path on the target device. It does not turn the small
synthetic corpus into a large-corpus, multilingual, learned-reranking, or user-study claim; those
remain later roadmap work.
