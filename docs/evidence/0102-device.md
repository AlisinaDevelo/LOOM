# LOOM 0102 device evidence

This artifact records the target-device verification for the canonical SQLite artifact and passage
model. The original implementation run is retained below for history; the merged-main reproduction
now appears at the end of this artifact. Issue [#11](https://github.com/AlisinaDevelo/LOOM/issues/11)
remains open until independent review and a protected-main policy are available.

## Device and toolchain

- Device: MacBook Pro 17,1, Apple M1, 8 GB
- OS: macOS 26.6.2 (25G83), `aarch64-apple-darwin` / arm64
- Native toolchain: `rustc 1.96.0` / Cargo 1.96.0, Node v26.7.0, npm 11.19.0
- Declared MSRV toolchain: `rustc 1.88.0` (`6b00bc3880198600130e1cf62b8f8a93494488cc`),
  Cargo 1.88.0
- Source under test: `609f61c4223e4453d66a6f6bb1e8108daf7cffa0`
  (`feature/issue-11-device-evidence`; merge SHA to be recorded after review)
- Fixture: `benchmarks/retrieval/v0/manifest.json`

## Acceptance-criterion evidence map

| Artifact ID | Acceptance criterion | Retained evidence |
| --- | --- | --- |
| `LOOM-0102-MIGRATION-PRAGMA` | Empty migration creates schema records and runtime guards | `store::tests::empty_database_migration_records_schema_and_runtime_guards`; schema version 2, WAL, foreign keys, 5,000 ms busy timeout, trusted schema off |
| `LOOM-0102-FIXTURE-CONTRACT` | Fixtures retain original bytes, hashes, extractor output, and anchors | `fixture_contract::every_rights_clean_fixture_round_trips_hash_extractor_and_anchor`; all 3 checked-in fixtures pass |
| `LOOM-0102-REINDEX-ID` | Deduplication, update, and stable IDs across reindex | `store::tests::indexes_searches_versions_and_verifies_original`; unchanged reindex keeps artifact/version IDs, changed bytes keep artifact ID and create a new version |
| `LOOM-0102-CASCADE-FTS` | Integrity constraints and cascade behavior | `store::tests::deleting_an_artifact_cascades_canonical_rows_and_fts_state`; locators, versions, passages, relationships, FTS rows, and foreign-key check are verified |
| `LOOM-0102-NEGATIVE-RECOVERY` | Failure, recovery, containment, and resource boundaries | Core tests for symlink escape, unsupported input, deletion, oversized input, stale source, and schema refusal; mixed-corpus CLI run below |

## Target-device reproduction

Commands run on the Mac:

```text
cargo +1.88.0 test -p loom-core --lib --tests -- --nocapture
cargo +1.88.0 check --workspace --all-targets --locked
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo run --locked -q -p loom-cli -- benchmark \
  --corpus benchmarks/retrieval/v0/corpus \
  --queries benchmarks/retrieval/v0/queries.jsonl
npm run check
npm run tauri build -- --debug --no-bundle
```

The repeatable local harness is [scripts/verify-device.sh](../../scripts/verify-device.sh). It
records one log per command, the exact toolchain and source SHA, and a SHA-256 manifest:

```text
bash scripts/verify-device.sh /tmp/loom-0102-device-rerun.ttqOOm
```

The retained pushed-tip run is `/tmp/loom-0102-device-rerun.ttqOOm` with `status=PASS`.

Observed results:

- Rust 1.88 core tests: 18 unit tests + 1 fixture-contract integration test passed.
- Native workspace tests: Tauri tests 0, CLI tests 5, core tests 18, fixture integration 1;
  no failures.
- Format, clippy, MSRV check, frontend lint/typecheck/Vitest/build, and Tauri debug build passed.
- Retrieval benchmark: 3/3 indexed, completeness 1.0, Recall@1 1.0, Recall@5 1.0, anchor
  precision 1.0, false-positive rate 0.0, median 0.191667 ms, p95 0.393667 ms.

The mixed-corpus run included a supported Markdown file, an unsupported binary, an 8,388,609-byte
Markdown file, and a symlink to a file outside the selected root. Initial indexing reported
`discovered=4`, `indexed=2`, `skipped=1`, and one bounded-size failure. The outside marker was not
searchable. Replacing the oversized file with valid text produced `indexed=1`, `unchanged=2`,
`skipped=1`, no failures, and recovered the new evidence on the next search. Final stats were
3 artifacts, 3 versions, 3 passages, and 250 indexed bytes.

## Log and digest record

The pushed-tip harness log manifest is `/tmp/loom-0102-device-rerun.ttqOOm/log-sha256.txt`:

```text
clippy.log             sha256:40d632405763050ffb47ebf1857f094617f02af036f60154d69b9a60e3d15158
fmt.log                sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
mixed-corpus.log       sha256:5eb8b3a8b2762f981b39e576b78ceed59ecb755893ef1a62c041a8214a45b9f1
npm-check.log          sha256:df2855af05008a3c099dbec95db1e994aae376196b55d260d8a64bb33be04394
npm-install.log        sha256:bbacb689fb0c829145033c5a0e39bff2b1fcb458b993473ecadaf839f978c9c0
retrieval-benchmark.log sha256:36eb6f2c7f7ad2c6dfb7a8bf536e53f223b8b5592a7c73426ce4bf480477f15c
rust-msrv-check.log    sha256:71be71ce1096fa54a430e2f093e56b9a0d465042fe1cd51cd830e4696359f23c
rust-msrv-tests.log    sha256:0bd4961cdc6dbd1a4b50199bf7b11709b3b2728f98784d335830e85a6fc52064
rust-workspace.log     sha256:013c60dbf9a82d27c27c4e78a7637ed8bedebc5de56cfa15ceec2a389961e275
tauri-build.log        sha256:612e6da938b44acd697e62b65be772eb0380897f2ef5dffc4c014b2811a79756
```

The focused Rust 1.88 log was retained locally as `/tmp/loom-0102-focused.log`:

```text
sha256:c4311de447b33e3a6578337879d9d59719d32169091e328ef20ba13fec1a7402  (1,968 bytes)
```

The MSRV workspace-check log was retained locally as `/tmp/loom-0102-check.log`:

```text
sha256:f561d56136ef6040f35bfb02e35cff7f39cc34633db9f0739990e5e490341fa8  (7,715 bytes)
```

## Limitations and closure gate

No user data, network service, live capture path, or unavailable hardware was substituted. This
evidence covers the current local text/Markdown slice only; PDF/OCR/browser connectors and passive
capture remain unsupported. The post-merge reproduction below satisfies the code and target-device
evidence portion; independent review and a protected-main policy remain required before closure.

## Merged-main reproduction

The same target-device harness was rerun against runtime-tested merged `main` commit
`eee1236710b98375e86b12187d545ed451ee2b7c` on the Mac specified above. The current main tip
`58b4934abf43f74f00a42be706aae1e83def711e` adds only documentation and roadmap metadata after that
runtime-tested commit; no canonical-store source changed.

- Verification directory: `/tmp/loom-0110-main-device.QLkKl1`
- Harness summary SHA-256: `45bc997dcb26b8bc6cbd63a09fa17aed6c5d4ae968ef349be473b0b034e94e70`
- Commands SHA-256: `d840925fb008af9101dc3121870b79960c1b6924451df17a7851f2f6132bb209`
- Log manifest: `/tmp/loom-0110-main-device.QLkKl1/log-sha256.txt`
- Log manifest SHA-256: `ed539c48e8c9b648f0ae341ddf37f02107d5aacff5d5e2a19ae0d28612c57d64`

The full local pipe passed: format, warnings-denied Clippy, workspace tests, Rust 1.88 MSRV
check/tests, `npm ci`, `npm run check`, retrieval benchmark, semantic contract, local security
scan, Tauri debug build, and mixed-corpus failure/recovery. The canonical-store fixture contract,
migration, stable-ID, cascade, and source-boundary tests passed within the workspace/MSRV runs.
The retrieval fixture indexed 3/3 sources with completeness 1.0, Recall@1/5 1.0, anchor precision
1.0, false-positive rate 0.0, median latency 0.185 ms, and p95 latency 0.518041 ms.

The mixed corpus again reported `discovered=4`, `indexed=2`, `skipped=1`, and one bounded-size
failure on the first index; an outside-root symlink remained unreachable. Replacing the oversized
file recovered it on the next index with `indexed=1`, `unchanged=2`, `skipped=1`, no failures, and
final stats of 3 artifacts, 3 versions, 3 passages, and 250 indexed bytes. No hosted CI or
unavailable hardware substituted for this target-device evidence. Any future desktop capture must
be cropped to the relevant evidence panel.
