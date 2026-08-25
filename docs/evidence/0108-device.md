# LOOM 0108 device evidence

This artifact records durable indexing checkpoints, atomic unit commits, migration, restart
recovery, and idempotent retry for issue [#17](https://github.com/AlisinaDevelo/LOOM/issues/17).
The durable-index implementation is already present on current `main`; this evidence closes the
roadmap item after the evidence/status merge. Hosted Actions are not used as evidence here.

## Device and toolchain

- Device: MacBook Pro 17,1, Apple M1, 8 GB
- OS: macOS 26.6.2 (25G83), `aarch64-apple-darwin` / arm64
- Native toolchain: `rustc 1.96.0` / Cargo 1.96.0; declared MSRV `rustc 1.88.0`
- Runtime-tested implementation baseline: `421bc6d469ba87a144495d0bf470d16ce44ec40f`
- Current main at evidence preparation: `924ccb3950dbc3f3c077452f1189d097ab5d92fb`

## Acceptance-criterion evidence map

| Artifact ID | Acceptance criterion | Retained evidence |
| --- | --- | --- |
| `LOOM-0108-ATOMIC-UNIT` | Each ingestion unit commits canonical content and progress atomically | `Library::index_document_with_extractor_and_checkpoint` updates `index_jobs.next_unit` on the same SQLite transaction as the artifact version and passage writes. Unsupported or unreadable units use a reconciliation transaction that advances the checkpoint with the explicit failure state. The fault test observes one complete artifact after interruption, never a partial passage projection. |
| `LOOM-0108-RECOVERY` | Job progress is recoverable after an interruption | [`crates/loom-core/tests/durable_index.rs`](../../crates/loom-core/tests/durable_index.rs) calls the explicit fault hook after one unit, verifies `state=interrupted`, `next_unit=1`, and `total_units=2`, then restarts normal indexing and verifies `state=completed`, `next_unit=2`. The checkpoint is bound to a discovery fingerprint and unit count; a changed selection resets rather than skips work. |
| `LOOM-0108-IDEMPOTENCE` | Retries are idempotent for the same source version | The same integration test searches both recovered markers, then runs a third scan and asserts `indexed=0`, `unchanged=2`, no failures, and exactly two content versions. Existing canonical-store tests also cover unchanged projections and extractor-version reprojection. |
| `LOOM-0108-MIGRATION` | Checkpoint schema is durable and compatible with the current local store | Schema version 3 adds `index_jobs`; `store::tests::migrates_v2_checkpoint_schema_without_overwriting_existing_marker` opens a version-2 marker, creates the checkpoint table transactionally, and records version 3. Unknown and pre-alpha version 1 markers remain fail-closed. |

## Current target-device reproduction

The focused current-main Rust run is retained at `/tmp/loom-0102-focused-current.log`
(`sha256:067ba921fbbceb562010d8bc1b6d05b75f445484579fbb11bf428259d2651435`) and the MSRV run at
`/tmp/loom-0102-msrv-current.log`
(`sha256:7ad063c8e87ee22fe8026d31c866374f7780d279941c511e03e2bb454b000ec1`). Each reports 93
passing tests and zero failures. The native log includes 36 store/unit tests, the 3-test durable
index integration suite, the 2-test cancellation suite, and the schema compatibility migration
tests; the MSRV run includes the same durable and migration assertions.

The source-equivalence record `/tmp/loom-0108-source-equivalence.log` has SHA-256
`08923a150b189a5bf4dbf76167c88feeb421716da87e2978d8599732c5093860`: durable implementation and
test paths are unchanged between `421bc6d` and current `main`, and current formatting passes.

The full workspace pipe was attempted on this device during the adjacent 0107 validation and hit
`ENOSPC` while compiling Tauri dependencies; it is retained at `/tmp/loom-0107-full-current.SNOOaO`
as a resource no-go, not a hidden pass. No hosted CI or unavailable hardware substitutes for the
current Rust evidence.

## Historical full-pipe corroboration

The earlier full run at implementation commit `4ffbc6307ba8c93048bbf69474a92593132b2f93` indexed
3/3 synthetic fixtures with completeness 1.0, exact-source
Recall@1 1.0, Recall@5 1.0, anchor precision 1.0, false-positive rate 0.0, median latency
0.190542 ms, and p95 latency 0.399375 ms.

The mixed corpus contained supported Markdown, an unsupported binary, an 8,388,609-byte Markdown
file, and an outside-root symlink. Initial indexing reported `discovered=4`, `indexed=2`,
`skipped=1`, and one bounded-size failure; the outside marker remained unreachable. Replacing the
oversized file recovered it on the next index with `indexed=1`, `unchanged=2`, `skipped=1`, and no
failures. Final stats were 3 artifacts, 3 versions, 3 passages, and 250 indexed bytes.

The first full harness attempt at that implementation family (`1d945c5`) failed only on two markdownlint
column-alignment diagnostics in the new table. The table was corrected in `4ffbc63`, markdown lint
was rerun clean, and the complete harness above was rerun from that corrected commit.

The implementation/evidence merge is PR [#240](https://github.com/AlisinaDevelo/LOOM/pull/240),
merge commit `c4365878809541026e50222e902a899ce83b8994`. The roadmap write closed issue [#17](https://github.com/AlisinaDevelo/LOOM/issues/17)
at `2026-08-25T03:56:26Z`.

The v2 reconciler preflight planned one `update_issue` for `0108` and no label, milestone, parent,
or dependency changes. Its retained fingerprints are:

```text
/tmp/loom-0108-roadmap-plan-before.json  sha256:4b3853c1745d1e47855b8c65f93d023041317e45b92159bb94fd608216e40b48
/tmp/loom-0108-roadmap-apply.log         sha256:a6bc8ee1d54a07e0a62c30555001e68a9761701775b8e0d6eef105b1fe6a80ec
/tmp/loom-0108-roadmap-plan-after.json   sha256:de11779046e5cbcf4ea4e7bbe88d19c641676058a6cb88254f80357b752941f1
/tmp/loom-0108-roadmap-plan-second.json  sha256:de11779046e5cbcf4ea4e7bbe88d19c641676058a6cb88254f80357b752941f1
```

Post-apply verification reports 154 active issues, 4 retired issues, 20 milestones, 13 phases,
141 parent edges, 314 dependency edges, 22 closed issues, and `mutation_count: 0`.

## Log and digest record

The retained run directory is `/tmp/loom-0108-device-final.aHrVpm`. Its `summary.txt`, command
record, individual logs, and `log-sha256.txt` are the source evidence for this entry:

```text
clippy.log              sha256:6c8d37f47ad8df3ccd1aed9818a4a39b2eb5bd70686df87c54a62f0d5003d7b4
fmt.log                 sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
mixed-corpus.log        sha256:2b64834674b04d462f8bac73be5d62bc07d722929c46956e570d540f02359c95
npm-check.log           sha256:45be88a3605ef151ca49ad85bc3c1857bed1e2fb5865d302f9531c0e6b927e8a
npm-install.log         sha256:fb475c05c6b905f4df0e4bdbff4ee53e57cca5202b70dd42aaabba382cc1f8bf
retrieval-benchmark.log sha256:f9829fd9e87f9ff9f29c2c233a1cfe5d2fe997f1fb8cb32b0ee2d2bc5b23b971
rust-msrv-check.log     sha256:9116032755af23c3e54622e381d34afa9aa4fc7bc785c0058cd71dc498430cbf
rust-msrv-tests.log     sha256:37618f50b1dd9d52a647f3603e8a3717feb6c414d5e6217ee117274615ad1476
rust-workspace.log      sha256:4fa049089e07a9dd96a9b8b26db214c7475ff672d120254b405ad6a13c848974
security-check.log      sha256:c926ea4e1be775594a3a85e088dd3984f0ba85c12ecae78d016028f9ab41dca3
tauri-build.log         sha256:5c6a6530cdbd50f24fab4b5c6eed360772fd4fc201670a0d6d5056d08528ccc9
```

## Limitations and closure boundary

The current focused run proves durable-index behavior on the specified Mac; it does not claim a
hosted Actions result, a different OS/architecture, forced power-loss hardware behavior,
notarization, a third-party security audit, or a `cargo-audit` result. The fault hook simulates
termination at a durable unit boundary without killing the test runner. The repository currently
has no protected-branch policy configured; the issue was closed after the owner-reviewed merge and
roadmap reconciliation. Future full-pipe runs should reclaim at least 1 GiB before compiling the
workspace, and desktop captures must remain cropped to the relevant evidence panel.
