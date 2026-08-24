# LOOM 0103 device evidence

This artifact records the bounded text/Markdown ingestion verification for issue
[#12](https://github.com/AlisinaDevelo/LOOM/issues/12). The implementation is stacked on the
canonical-store work in PR [#163](https://github.com/AlisinaDevelo/LOOM/pull/163); issue #12 stays
open until review, merge, and a repeat against the merged `main` SHA.

## Device and toolchain

- Device: MacBook Pro 17,1, Apple M1, 8 GB
- OS: macOS 26.6.2 (25G83), `aarch64-apple-darwin` / arm64
- Native toolchain: `rustc 1.96.0` / Cargo 1.96.0, Node v26.7.0, npm 11.19.0
- Declared MSRV toolchain: `rustc 1.88.0` (`6b00bc3880198600130e1cf62b8f8a93494488cc`),
  Cargo 1.88.0
- Source under test: `b541b2250b43b86f0d43e97e8375bce254744022`
  (`feature/issue-12-bounded-ingestion`)

## Acceptance-criterion evidence map

| Artifact ID | Acceptance criterion | Retained evidence |
| --- | --- | --- |
| `LOOM-0103-EXPLICIT-ROOT` | Explicit file/directory selection and documented symlink rules | `ingest::tests::stable_reads_reject_symlinks_and_paths_outside_root`; `store::tests::indexes_searches_versions_and_verifies_original`; mixed-corpus outside-root marker remains unreachable |
| `LOOM-0103-BLAKE3-REINDEX` | BLAKE3 identity and stale replacement behavior | `store::tests::indexes_searches_versions_and_verifies_original`; unchanged reindex preserves IDs, changed bytes create a new version, stale opening is rejected |
| `LOOM-0103-PASSAGE-OFFSETS` | Deterministic bounded passage offsets | `ingest::tests::passage_offsets_cover_unicode_without_splitting_characters`; fixture contract verifies hashes and anchors for all checked-in fixtures |
| `LOOM-0103-BOUNDED-FAILURE` | Oversized, binary, unreadable, and unsupported inputs report safely | `store::tests::skips_unsupported_files_without_reading_them`; `store::tests::failed_directory_reread_hides_previous_artifact`; `store::tests::unreadable_source_is_reported_and_recovers_after_permissions_restore`; mixed corpus exercises the 8,388,609-byte boundary |

## Target-device reproduction

The checked-in harness is [scripts/verify-device.sh](../../scripts/verify-device.sh):

```text
bash scripts/verify-device.sh /tmp/loom-0103-device-final.On1m0T
```

The run completed with `status=PASS` at source commit `b541b2250b43b86f0d43e97e8375bce254744022`.
Rust workspace tests reported 19 core unit tests, 1 fixture-contract integration test, 5 CLI
tests, and no failures. Format, clippy, MSRV check/tests, `npm run check`, and the Tauri debug
build passed.

The benchmark indexed 3/3 fixtures with completeness 1.0, exact-source Recall@1/5 1.0, anchor
precision 1.0, false-positive rate 0.0, median latency 0.593291 ms, and p95 latency 1.215750 ms.

The mixed corpus contained supported Markdown, an unsupported binary, an 8,388,609-byte Markdown
file, and an outside-root symlink. Initial indexing reported `discovered=4`, `indexed=2`,
`skipped=1`, and one bounded-size failure; the outside marker was not searchable. Replacing the
oversized file recovered it on the next index with `indexed=1`, `unchanged=2`, `skipped=1`, and no
failures. Final stats were 3 artifacts, 3 versions, 3 passages, and 250 indexed bytes.

The unreadable-source test changes a previously indexed file to mode `000`, confirms an
`IndexReport` failure and hidden search result, restores mode `0600`, and confirms recovery. It
requires the normal non-privileged target user; the target Mac satisfied that condition.

## Log and digest record

The full log manifest is `/tmp/loom-0103-device-final.On1m0T/log-sha256.txt`:

```text
clippy.log              sha256:d8e7be27693f18bdd0bb2bc7fac7ff672e29e0751c2e6f3e6d114e7550864c9f
fmt.log                 sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
mixed-corpus.log        sha256:1176255a3433ef27c0cc6f31457b97ffe43371d791398f102363dc750912f80a
npm-check.log           sha256:602c63d6be3ce265dcc28982432ea2f1e5c4a14a4971206c1e9b54a45e2c3388
npm-install.log         sha256:bbacb689fb0c829145033c5a0e39bff2b1fcb458b993473ecadaf839f978c9c0
retrieval-benchmark.log sha256:a2f353cb13c811425bd696128ea611440d9eee243ea1e16adaeecf1637f90d9e
rust-msrv-check.log     sha256:9ecb800b556b58de6cc1dbe7ed15219b70db52d4a8b80ccce0b8636d61ac0bfb
rust-msrv-tests.log     sha256:7022af6d28f4cc30ac364039968c0b35fab718567d5d5d53ebd90880429004cf
rust-workspace.log      sha256:ec93eae49bc060541c49addaafdbe567414692405dcf4ad16e90cc0fe95e30ee
tauri-build.log         sha256:f3a42079bb6324c4bf6a678cc6f4cb7ed51f41080b2524686e57ad9246af9a3f
```

## Limitations and closure gate

This evidence covers explicit local UTF-8 text and Markdown only. PDF/OCR, browser capture,
passive capture, and unavailable hardware were not substituted or claimed. Review, protected-main
merge, the same reproduction against the merged SHA, and final issue-linked evidence remain
required before #12 can close.

## Merged-main reproduction

The same target-device harness was rerun against protected `main` at
`6b97dc0e493a0fd63810ae1294cde7f2d558d273` after the issue-12 implementation
merged. The evidence directory is `/tmp/loom-merged-main-device.RiWdYT`.

- Device: MacBook Pro 17,1, Apple M1, 8 GB; macOS 26.6.2 (25G83), arm64
- Native Rust/Cargo: 1.96.0; declared MSRV Rust/Cargo: 1.88.0
- Node v26.7.0; npm 11.19.0
- Format, clippy, workspace tests, MSRV check/tests, npm check, retrieval benchmark,
  Tauri debug build, and mixed corpus — pass
- Mixed corpus: initial `discovered=4`, `indexed=2`, `skipped=1`, one bounded
  oversized-source failure; outside-root symlink remained unreachable; recovery
  reindex reported `indexed=1`, `unchanged=2`, `skipped=1`, no failures; final
  stats were 3 artifacts, 3 versions, 3 passages, 250 indexed bytes
- Summary SHA-256: `f802d4596498c683b58870dd344117ca66ffb375ca8ebcf100a01ee12de44136`
- Log manifest SHA-256: `7224bdaffdd0bccb9ace38f0a52278dfbae02b3409a378cd0d118a8e85f408f7`

This is the merged-main reproduction record; the issue may close only after this
artifact is linked in its discussion and the repository review policy is satisfied.
