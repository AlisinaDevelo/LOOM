# LOOM 0103 device evidence

This artifact records the bounded text/Markdown ingestion verification for issue
[#12](https://github.com/AlisinaDevelo/LOOM/issues/12). The original implementation run remains
retained for history; the current merged-main reproduction is recorded below. Issue #12 remains
open until independent review and a protected-main policy are available.

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
passive capture, and unavailable hardware were not substituted or claimed. The post-merge
reproduction below satisfies the code and target-device evidence portion; independent review and a
protected-main policy remain required before #12 can close.

## Merged-main reproduction

The same target-device harness was rerun against runtime-tested merged `main` commit
`eee1236710b98375e86b12187d545ed451ee2b7c` on the Mac specified above. The final main tip
`9dea7801b2881eac9220daf0dfd0e0ff097b27a6` adds only documentation and roadmap metadata after
that runtime-tested commit; no bounded-ingestion source changed.

- Verification directory: `/tmp/loom-0110-main-device.QLkKl1`
- Harness summary SHA-256: `45bc997dcb26b8bc6cbd63a09fa17aed6c5d4ae968ef349be473b0b034e94e70`
- Commands SHA-256: `d840925fb008af9101dc3121870b79960c1b6924451df17a7851f2f6132bb209`
- Log manifest: `/tmp/loom-0110-main-device.QLkKl1/log-sha256.txt`
- Log manifest SHA-256: `ed539c48e8c9b648f0ae341ddf37f02107d5aacff5d5e2a19ae0d28612c57d64`

The full local pipe passed: format, warnings-denied Clippy, workspace tests, Rust 1.88 MSRV
check/tests, `npm ci`, `npm run check`, retrieval benchmark, semantic contract, local security
scan, Tauri debug build, and mixed-corpus failure/recovery. The unreadable-source test exercised
mode-000 failure, hidden search results, permission restoration, and recovery. The retrieval
fixture indexed 3/3 sources with completeness 1.0, Recall@1/5 1.0, anchor precision 1.0,
false-positive rate 0.0, median latency 0.185 ms, and p95 latency 0.518041 ms.

The mixed corpus reported `discovered=4`, `indexed=2`, `skipped=1`, and one bounded-size failure
on the first index; an outside-root symlink remained unreachable. Replacing the oversized file
recovered it on the next index with `indexed=1`, `unchanged=2`, `skipped=1`, no failures, and final
stats of 3 artifacts, 3 versions, 3 passages, and 250 indexed bytes. No hosted CI or unavailable
hardware substituted for this target-device evidence. Any future desktop capture must be cropped
to the relevant evidence panel.
