# 0206 device evidence: multimodal retrieval benchmark

Status: implementation-ready review evidence. This record describes a target-device run of
source commit `df497f9ccd553b8a991794710e1014b1faee3634` on 2026-08-24. It does not claim
population-quality retrieval.

## Target and reproducibility

- Device: Apple Silicon Mac (`arm64`), macOS 26.6.2 (build 25G83).
- Toolchains: Rust 1.96.0 / Cargo 1.96.0; MSRV Rust 1.88.0 / Cargo 1.88.0; Node v26.7.0;
  npm 11.19.0.
- Full pipe directory: `/tmp/loom-0206-device.zxI49p`.
- Device summary SHA-256: `248bc9ed01963f3760f10d6ad52d63c9cc8840a9f636894d39c8dce0d7dd6c55`.
- Command list SHA-256: `6b503a2f05203823e08720439cc07962249c805e94b3f826ce778b9bb3a105d9`.

The first unmodified-pipe attempt stopped at `npm ci` with `ENOSPC` before its benchmark
steps could run. The runner was then changed to clear only its ignored LOOM `target/` between
the Rust/evidence phase and the JS/Tauri phase. The final run above completed with `status=PASS`;
the disk failure is retained as a test-harness limitation, not counted as a product pass.

## Acceptance evidence

### E-0206-A1 — rights-clean multimodal corpus

`benchmarks/retrieval/v1/manifest.json` validates 8 fixtures and 11 queries: local Markdown/plain
text (including an exact duplicate and date), a hard negative, a saved-web Markdown stand-in, a
two-page PDF, a cropped screenshot, and a cropped rasterized scanned-page stand-in. Every fixture
has a raw BLAKE3 content hash, extractor identity/version, passage hash, and exact text/page/region
anchor. The screenshot crop is the non-background `878x191` box at `(84,170)` from the synthetic
OCR fixture; the scanned-page crop is `575x143`. Both were visually checked on the target Mac.

Manifest SHA-256 is `3444f17ff9825aa92f52bd6d03463e8c568ce04b874205d989d45b5346a9a9de`.
The query-set SHA-256 is `a52350bd7e2067edf379a8141477071c983efcb27053a3c7a4c2aad0e5845664`.
The two cropped PNG SHA-256 values are `8683cca586d52a3f74338d05204a6dfdb5b6c1c73acdb3523b322488563164ce`
and `eda01189fa24678d326cfa591558ce347168f4214372ac4785661cec25424eaa`.

### E-0206-A2 — metrics and index cost

The final v1 report is `/tmp/loom-0206-device.zxI49p/retrieval-benchmark-v1.log` (SHA-256
`4440a74ac6afce850a574636bb695c709397b16fa1a6d92ef0ccb7f7d7923285`). It indexed all 8
fixtures with no index failures and completeness `1.0`.

| Metric | Measured value |
| --- | ---: |
| Exact-source Recall@1 / Recall@5 | 0.900 / 0.900 |
| MRR | 0.900 |
| Anchor precision | 1.000 |
| False-positive rate | 0.153846 |
| Reformulation success | 1.000 (1 query) |
| Negative no-result rate | 0.000 (1 query) |
| Median / p95 latency | 0.214584 / 0.472791 ms |
| Index elapsed time | 349.281542 ms |
| Source bytes read | 52,083 |
| Database bytes | 786,736 |
| Database/source-byte ratio | 15.1054279 |

The v0 compatibility report remains clean: 3/3 sources, Recall@1/5 `1.0`, anchor precision
`1.0`, false-positive rate `0.0`, and no failures. Its log SHA-256 is
`997f5e315fdc3287b0767f1f9214099e7d673df0409328a6aeb32fdb9a59b443`.

### E-0206-A3 — source-class breakdown

The report separates local text, PDF, saved web, screenshot, and scanned page. PDF, saved web,
screenshot, and scanned page each recovered every expected source and anchor at rank one. Local
text measured Recall@1/5 `0.75`, MRR `0.75`, anchor precision `1.0`, and false-positive rate
`0.285714`; those values are intentionally not hidden by the overall score.

### E-0206-A4 — known failures and recovery work

The v1 report retains two local-text failures:

- `q008` (`no_results`): the primary paraphrase `"retry problem"` misses the engineering note;
  its declared reformulation `"retry anomalies"` succeeds.
- `q009` (`false_positive`): the hard-negative query returns the exact duplicate engineering
  notes. This is a known duplicate/disambiguation failure, not a threshold change.

The mixed-corpus run also proves the failure path: an 8 MiB-plus file is rejected without being
indexed, an outside-root symlink is not followed, and the file is recovered after replacement
with valid content. The mixed log SHA-256 is
`151de3fa2419670440bf3fbc27a9303868878bf6e0b7507144ca079edbcafde4`.

## Full local pipe

All of these completed with exit code 0 in the final run: `cargo fmt --all --check`, workspace
Clippy with `-D warnings`, locked workspace tests, Rust 1.88 workspace check and core tests,
v0 benchmark, v1 benchmark, semantic rebuild/drop/rebuild repeatability, the mixed failure and
recovery corpus, `npm ci`, `npm run check`, `scripts/security-check.sh`, and
`npm run tauri build -- --debug --no-bundle`. The semantic summary proves repeatable rebuild,
fail-closed drop behavior, and evidence-bound search; its SHA-256 is
`c523aa307f1a5395726bf29cb794c97e7f4dccc3c37ca97894e3a8369e59fed3`.

Top-level log digests are retained in `/tmp/loom-0206-device.zxI49p/log-sha256.txt`; the
benchmark and mixed-corpus digests above are the acceptance-critical subset.

## MVP demo

`scripts/demo-mvp.sh` was rerun on the same device/source family at 18:22:20 +0200. It indexed
5 selected sources and visibly recovered a text passage, a PDF page anchor, and a cropped OCR
image-region anchor; OCR status reported two derived passages. Demo log:
`/tmp/loom-mvp-demo-0206.yKMm9I/demo.log` (SHA-256
`f72b557217841b011eff53aa20672dbfe0f3010aa5d92bf85b5c6e7ec64eb314`). The retained viewer path
is printed by the script for a local Tauri run; no user data or uncropped private screenshot was
used.

## Limitations

The corpus is synthetic and small. The saved-web item is a Markdown stand-in, the PDF is a
synthetic text PDF, and the scanned-page case is a rasterized OCR stand-in; this evidence does
not establish HTML archival completeness, scanned-PDF OCR parity, multilingual quality, or
real-world user success. The next ranking work is the explicit q008 paraphrase and q009 duplicate
failure; the benchmark must remain unchanged while those failures are addressed.
