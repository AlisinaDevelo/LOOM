# 0211 merged-main device evidence

Roadmap task 0211 is evaluated against merged `main` at
`b99a78d5e79ee8f809448f70c23a7b07a6f89154` (PR [#211](https://github.com/AlisinaDevelo/LOOM/pull/211),
merged 2026-08-24 20:28:32 UTC). The reproduction ran on the target Mac:

- macOS 26.6.2 (build 25G83), arm64;
- Rust/cargo 1.96.0, Node.js v26.7.0, npm 11.19.0; and
- evidence directory `/tmp/loom-0211-merged.bhKoJO`.

The OCR fixture is the checked-in, rights-clean crop `tests/fixtures/ocr-golden.png`; no
full-screen screenshot was captured or retained.

## Acceptance mapping

| Criterion | Retained evidence | Result |
| --- | --- | --- |
| Store OCR engine/version, language, confidence, bounding region, image hash, and an explicit low-confidence state | `rust-core.log`; `cli-contract-summary.json`; `crates/loom-core/src/domain.rs`, `ocr.rs`, `store.rs`; `tests/image_ocr.rs` | Native Vision records provider/model identity, `language: auto`, image BLAKE3 hash equal to the canonical version hash, fixed-point confidence/region geometry, threshold `800`, aggregate state, and per-result `confirmed`/`low_confidence` state. |
| Verify coordinate normalization across image sizes, rotation, and Retina scale | `rust-core.log` geometry tests; `tests/fixtures/ocr-golden.png` (1200×600); `benchmarks/retrieval/v1` cropped OCR fixtures (1200×600 and 575×143) | The unit contract covers normal and EXIF-rotated dimensions plus 2× fixed-point scaling; native integration checks clamped regions, orientation, scale, and bounds on the target Mac. |
| Distinguish confirmed text, low-confidence text, and no-readable-text in UI and CLI | `npm-check.log`; `cli-contract-summary.json`; `blank-index.json` under `cli-contract/`; `src/App.test.tsx` | 20 UI tests pass, including visible low-confidence labeling and no-readable notice. CLI inspect/search emit the confidence state; a blank PNG fails with the machine-readable `no-readable-text` reason. |

## Reproduction

The exact command list is `commands.txt`. The merged-main run passed:

```text
cargo fmt --all --check
cargo test -p loom-core --lib --tests --locked --offline -- --nocapture
cargo test -p loom-cli --locked --offline -- --nocapture
npm ci && npm run check
direct CLI OCR metadata/search/no-readable contract
```

The Rust run reports 36 core unit/integration tests, including five native OCR integration tests;
the CLI run reports 9 passed tests. The frontend run reports lint, Markdown lint, 20 Vitest tests,
TypeScript, and the Vite production build all passed.

The direct CLI contract indexed the cropped OCR fixture, confirmed that search and inspect expose
the same confidence state and image hash, and rejected a generated blank PNG with
`no-readable-text`. The native blank-image integration test independently exercises the same
failure and the recovery/purge tests keep derived OCR state transactional.

## Evidence digests

```text
cli-contract-summary.json  188c2896942294728c20d61f2a78a57825c52f21320b187d68a81d11dbcb83c5
rust-core.log              eaffb3aa437c82996eed4b467305f47bcb8f858a27fe3b69c9200cf8e1372325
rust-cli.log               97a2f0e3109b369d7ca85dc8fb16289066b8037c0a9e45131cf3a650220f5f60
npm-check.log              6982f8d059982bb37b03a7b8ef08d65afe9550b363d438c044daecaaf026b555
environment.txt            66989ee8fb92ba8a736de17d6ea5697d6192eaf857460d8d538166f801202d28
```

Clippy was attempted on this device but stopped when the volume reached `ENOSPC` while writing a
dependency artifact; the retained `clippy-enospc.log` digest is
`bf7ab41ddf28cd787e98b7e6851753c249237e7485139ca845f4a54658ac6602`. The focused Rust, native
OCR, CLI, frontend, and direct payload gates above completed successfully. This evidence does not
claim a packaged Tauri artifact or a separate Screen Recording permission session.
