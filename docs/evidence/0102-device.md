# LOOM 0102 device evidence

This artifact records the target-device verification for the canonical SQLite artifact and passage
model. Issue [#11](https://github.com/AlisinaDevelo/LOOM/issues/11) remains open until the same
reproduction is rerun against the merged `main` SHA and the resulting evidence is linked there.

## Device and toolchain

- Device: MacBook Pro 17,1, Apple M1, 8 GB
- OS: macOS 26.6.2 (25G83), `aarch64-apple-darwin` / arm64
- Native toolchain: `rustc 1.96.0` / Cargo 1.96.0, Node v26.7.0, npm 11.19.0
- Declared MSRV toolchain: `rustc 1.88.0` (`6b00bc3880198600130e1cf62b8f8a93494488cc`),
  Cargo 1.88.0
- Source under test: `babc49325df18d4424ec64b46909633f90d495a5`
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
bash scripts/verify-device.sh /tmp/loom-device-verify.UBvd0k
```

The retained pushed-tip run is `/tmp/loom-device-verify.UBvd0k` with `status=PASS`.

Observed results:

- Rust 1.88 core tests: 18 unit tests + 1 fixture-contract integration test passed.
- Native workspace tests: Tauri tests 0, CLI tests 5, core tests 18, fixture integration 1;
  no failures.
- Format, clippy, MSRV check, frontend lint/typecheck/Vitest/build, and Tauri debug build passed.
- Retrieval benchmark: 3/3 indexed, completeness 1.0, Recall@1 1.0, Recall@5 1.0, anchor
  precision 1.0, false-positive rate 0.0, median 0.224208 ms, p95 0.403500 ms.

The mixed-corpus run included a supported Markdown file, an unsupported binary, an 8,388,609-byte
Markdown file, and a symlink to a file outside the selected root. Initial indexing reported
`discovered=4`, `indexed=2`, `skipped=1`, and one bounded-size failure. The outside marker was not
searchable. Replacing the oversized file with valid text produced `indexed=1`, `unchanged=2`,
`skipped=1`, no failures, and recovered the new evidence on the next search. Final stats were
3 artifacts, 3 versions, 3 passages, and 250 indexed bytes.

## Log and digest record

The pushed-tip harness log manifest is `/tmp/loom-device-verify.UBvd0k/log-sha256.txt`:

```text
clippy.log             sha256:6867d59394fddf01cc4c5298662108a6d2c2b3662bf86f6d8416cec00acfbe1b
fmt.log                sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
mixed-corpus.log       sha256:64c7fd548af981d9db448f02b2e3456a7e4a1f16c454706ee313e726f3b69da5
npm-check.log          sha256:5b225bba9dbd9f21ee35615621060326e122ffa5d13b3b16a8c6d8e2a7a8e4c8
npm-install.log        sha256:bbacb689fb0c829145033c5a0e39bff2b1fcb458b993473ecadaf839f978c9c0
retrieval-benchmark.log sha256:da5ef124f1bdc24a6c2ddac5742217d139b3b841b8a4df6779a5fd4ce0ac8146
rust-msrv-check.log    sha256:e736d608af4933d92144949c87701ac8f4de67b152ddfb1a7507d4d66022e032
rust-msrv-tests.log    sha256:7e69c3b553c48e7976e34b32f50e85f95f204f62a68a02eabec350fe1d13d642
rust-workspace.log     sha256:bf9b501daea6f9e09604c58a5865f1d0a4568346d9440460824eb24f0d60ed04
tauri-build.log        sha256:d7909739b13c84913eb3eca3005c18b09e8796f76e8b6d0d042dcab7cac340f1
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
capture remain unsupported. The PR must be pushed, reviewed, and merged to protected `main`; the
same commands and mixed-corpus reproduction must then be rerun against that merged SHA. The final
merged SHA, post-merge log digests, and issue closure decision belong in the issue before closure.
