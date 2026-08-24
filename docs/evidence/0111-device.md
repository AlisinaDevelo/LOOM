# LOOM 0111 device evidence

This artifact records the schema compatibility and migration contract for issue [#64](https://github.com/AlisinaDevelo/LOOM/issues/64).
It covers the reviewed version-2 to version-3 migration, canonical-row preservation, derived-index
rebuild, and fail-closed malformed-version behavior.

## Device and toolchain

- Device: MacBook Pro 17,1, Apple M1, 8 GB
- OS: macOS 26.6.2 (25G83), `aarch64-apple-darwin` / arm64
- Native toolchain: `rustc 1.96.0` / Cargo 1.96.0, Node v26.7.0, npm 11.19.0
- Declared MSRV toolchain: `rustc 1.88.0` (`6b00bc3880198600130e1cf62b8f8a93494488cc`),
  Cargo 1.88.0
- Source under test: `b6283d847c1cfd9d2c61d1f93692a2c033e3cbbf`
  (`feature/issue-63-activation-gate`)

## Acceptance-criterion evidence map

|Artifact ID|Acceptance criterion|Retained evidence|
|---|---|---|
|`LOOM-0111-MATRIX`|A versioned matrix covers create, open, rebuild, unknown refusal, and the first reviewed migration|`docs/SCHEMA_COMPATIBILITY.md` defines new/create v3, v3 open, v2→v3 migration, derived FTS rebuild, v1 rejection, missing-marker refusal, malformed/unknown refusal, and support/rebuild paths. The compatibility integration suite covers populated migration, reopen, derived-index rebuild, and malformed v2. Existing unit coverage proves v1 and unknown version markers remain untouched.|
|`LOOM-0111-PRESERVE`|Migration fixtures preserve source hashes, extractor identity, anchors, and relationship rows|`tests/fixtures/schema-v2.sql` is a populated rights-clean v2 database. `populated_v2_migration_preserves_canonical_identity_and_evidence` verifies version 3, the BLAKE3 source hash, extractor ID/version, serialized and scalar anchors, relationship endpoints/evidence/confidence, searchable evidence, and all values after a second reopen.|
|`LOOM-0111-FAIL-CLOSED`|Malformed or unsupported databases fail with a named reason without silent rewriting|`validate_schema_shape` checks required canonical tables and columns before migration. `malformed_v2_marker_fails_closed_without_creating_new_tables` expects `schema version 2 is missing required table \`source_roots\`` and verifies the marker remains 2 and `index_jobs` is absent. Existing v1/unknown tests verify their markers are not overwritten.|
|`LOOM-0111-DERIVED-REBUILD`|A damaged derived search projection can be reconstructed without becoming canonical truth|Opening a library deterministically rebuilds FTS5 from canonical passages. `opening_a_library_rebuilds_a_missing_derived_fts_projection` deletes the FTS projection, reopens the database, and recovers the exact source-backed result; canonical rows are never read from FTS5.|

## Target-device reproduction

The checked-in harness is [`scripts/verify-device.sh`](../../scripts/verify-device.sh):

```text
bash scripts/verify-device.sh /tmp/loom-0111-device-final.eHpdDF
```

The run completed with `status=PASS` and exit code 0 at source commit
`b6283d847c1cfd9d2c61d1f93692a2c033e3cbbf`. Format, clippy, the full Rust workspace, MSRV check
and tests, `npm ci`, `npm run check`, retrieval benchmark, local security check, Tauri debug build,
and the mixed failure/recovery corpus all passed. The Rust workspace reported 39 passing tests
across the CLI, 23 core unit tests, durable observation integration, fixture/result contracts, and
the three new schema compatibility tests. The MSRV run passed the 31 loom-core tests. The frontend
check ran 2 files and 6 tests; markdown lint and the 19 Python contract tests also passed.

The rights-clean retrieval benchmark indexed 3/3 fixtures with completeness 1.0, exact-source
Recall@1 1.0, Recall@5 1.0, anchor precision 1.0, false-positive rate 0.0, median latency
0.456125 ms, and p95 latency 0.89025 ms.

The mixed corpus contained supported Markdown, an unsupported binary, an 8,388,609-byte Markdown
file, and an outside-root symlink. Initial indexing reported `discovered=4`, `indexed=2`,
`skipped=1`, and one bounded-size failure; the outside marker remained unreachable. Replacing the
oversized file recovered it on the next index with `indexed=1`, `unchanged=2`, `skipped=1`, and no
failures. Final stats were 3 artifacts, 3 versions, 3 passages, and 250 indexed bytes.

The local security check found no gitleaks secrets and `npm audit` reported zero vulnerabilities.
This is not a third-party audit or a complete dependency advisory statement; the repository still
has a GitHub Dependabot advisory for `glib`, and `cargo-audit` is not installed on this device.
No GitHub-hosted Actions result was used.

## Log and digest record

The retained run directory is `/tmp/loom-0111-device-final.eHpdDF`. Its `summary.txt`, command
record, individual logs, and `log-sha256.txt` are the source evidence for this entry:

```text
clippy.log              sha256:dccad65a474a0e64d7c677fc96c87f30265067841005a793f4d82605fea6fe77
fmt.log                 sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
mixed-corpus.log        sha256:1745ff6f3a453b68b05aa2adf5fd23d288f79cc8f11dbea3b84c16425417a503
npm-check.log           sha256:f64c0192ab3fdaf4cf4a72106986d9a9e734b57d268d5a8e19e98f6bedc00fc1
npm-install.log         sha256:85b4814e52ba71ae231930f0e73b027fadd1667b8aede7ff5c562e1a0a4e22fe
retrieval-benchmark.log sha256:f863946a19a11fddafe9468717ad359ab1f649406573630a286cbb3a18a807e7
rust-msrv-check.log     sha256:3d2669067df0e368d85c2b4b048789f5173e7075f65941f8ba392ab3680e0f4a
rust-msrv-tests.log     sha256:b8b69b94b753ec23e310af6b0da3123828168aafdb878d65c828368533b40d97
rust-workspace.log      sha256:9d2a9a2d88a871f40f802e76e979981a148705c0337e1339896314da154c0df8
security-check.log      sha256:c6e6d1f7232f0f6ce034d1d4a11a8abb2e360792cf7548988081df3967f8f2f9
tauri-build.log         sha256:c533c1a8c1c2c297f79f9b49b12ba8dd974cc3108c08fb138c9e467e22cdb16c
```

## Limitations and closure gate

This run proves the migration contract on the specified Mac; it does not claim another
OS/architecture, notarization, a third-party security audit, a `cargo-audit` result, or a
large-library startup/resource benchmark for deterministic FTS rebuild. Issue 64 remains open
until independent review, a protected-main merge, and the same reproduction against the merged
`main` SHA are available.
