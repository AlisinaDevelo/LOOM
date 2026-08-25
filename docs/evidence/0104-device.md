# LOOM 0104 device evidence

This artifact records the source-backed search-result and exact-open verification for issue
[#13](https://github.com/AlisinaDevelo/LOOM/issues/13). The change is stacked on the canonical
store and bounded-ingestion work in PRs [#163](https://github.com/AlisinaDevelo/LOOM/pull/163) and
[#164](https://github.com/AlisinaDevelo/LOOM/pull/164). The original implementation run remains
retained for history; the current-main reproduction is recorded below. Issue #13 is closed after
the evidence and roadmap-status changes were merged to `main`.

## Device and toolchain

- Device: MacBook Pro 17,1, Apple M1, 8 GB
- OS: macOS 26.6.2 (25G83), `aarch64-apple-darwin` / arm64
- Native toolchain: `rustc 1.96.0` / Cargo 1.96.0, Node v26.7.0, npm 11.19.0
- Declared MSRV toolchain: `rustc 1.88.0` (`6b00bc3880198600130e1cf62b8f8a93494488cc`),
  Cargo 1.88.0
- Source under test: `ee1909a7b70847d7f8bcd52fd85a48e3f3e515eb`
  (`feature/issue-13-evidence-contract`)

## Acceptance-criterion evidence map

| Artifact ID | Acceptance criterion | Retained evidence |
| --- | --- | --- |
| `LOOM-0104-RESULT-CONTRACT` | Every displayed result resolves to one stable source-backed identity and evidence anchor | `crates/loom-core/tests/result_contract.rs::public_search_results_are_source_backed_and_tuple_bound` parses artifact/version/passage UUIDs, checks rank/finite score/source URI/BLAKE3 hash, verifies the structured highlighted excerpt, and proves the returned character/line anchor slices the original source to the highlighted text. |
| `LOOM-0104-SAFE-QUERY` | Plain terms and quoted phrases do not expose raw FTS/SQL operators | The result-contract test recovers the quoted phrase and asserts the injection-like `"retry anomalies" OR 1=1 --` request returns no false-positive hit; existing `search::tests::compiles_terms_and_phrases_without_fts_operators` and malformed-query tests also pass. |
| `LOOM-0104-TUPLE-OPEN` | Opening requires the stored artifact/version/hash tuple and revalidates bytes | The result-contract test opens the exact tuple successfully and rejects a wrong content hash with `LoomError::ArtifactStale`; the full workspace and MSRV suites also retain changed-source stale-open coverage. |
| `LOOM-0104-FAIL-CLOSED` | Failure and recovery preserve evidence integrity without arbitrary paths | The full target-device harness exercises the bounded oversized-file failure, unsupported binary, outside-root symlink, replacement recovery, and final source-backed search. No network, account, telemetry, generated answer, or arbitrary frontend path is introduced. |

## Target-device reproduction

The checked-in harness is [scripts/verify-device.sh](../../scripts/verify-device.sh):

```text
bash scripts/verify-device.sh /tmp/loom-0104-device-rerun.RnptMI
```

The run completed with `status=PASS` and exit code 0 at source commit
`ee1909a7b70847d7f8bcd52fd85a48e3f3e515eb`. Format, clippy, the Rust workspace, MSRV check and
tests, `npm ci`, `npm run check`, the retrieval benchmark, the Tauri debug build, and the mixed
corpus all passed. The Rust workspace reported 19 core unit tests, 1 fixture-contract integration
test, 1 result-contract integration test, 5 CLI tests, and no failures. The MSRV test run also
passed the 19 core unit tests and both integration tests.

The benchmark indexed 3/3 synthetic rights-clean fixtures with completeness 1.0, exact-source
Recall@1/5 1.0, anchor precision 1.0, false-positive rate 0.0, median latency 0.192667 ms, and
p95 latency 0.436375 ms.

The mixed corpus contained supported Markdown, an unsupported binary, an 8,388,609-byte Markdown
file, and an outside-root symlink. Initial indexing reported `discovered=4`, `indexed=2`,
`skipped=1`, and one bounded-size failure; the outside marker was not searchable. Replacing the
oversized file recovered it on the next index with `indexed=1`, `unchanged=2`, `skipped=1`, and no
failures. Final stats were 3 artifacts, 3 versions, 3 passages, and 250 indexed bytes.

The test was also mutation-checked: changing the expected line anchor to `(99, 99)` produced a
non-zero test exit and the expected `(2, 2)` versus `(99, 99)` assertion failure; the source was
then restored and the retained run passed.

## Log and digest record

The full log manifest is `/tmp/loom-0104-device-rerun.RnptMI/log-sha256.txt`:

```text
clippy.log              sha256:5bb2c20cc067ef32707d7d7a3b4ee5e3ffc8f18841d0e9b2c1eca3d07929c5d2
fmt.log                 sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
mixed-corpus.log        sha256:f727d929470bebec3feae611bde1dfe65c7c1d2f96a1ee85222981040f482809
npm-check.log            sha256:a99358156393e44b31e064d289adb42530131718192805ae6a93264adce6a7b1
npm-install.log          sha256:bbacb689fb0c829145033c5a0e39bff2b1fcb458b993473ecadaf839f978c9c0
retrieval-benchmark.log  sha256:588dbca532d49314d0cb192958e60c462811c35933add71a22c941cd29039f98
rust-msrv-check.log      sha256:a113781de65540b4cd3aa2a4cc5d6f77f4bed35e9ab0e988cc44a7a412373342
rust-msrv-tests.log      sha256:14e13fa35b851953c12c14b3c0a21c098ff153bd58eca90419e7d226a55efcb6
rust-workspace.log       sha256:288ba53ec7dad66ee7efebeef8251a849758e99e7ca9f1ae8fc557351f24c0ad
tauri-build.log          sha256:485c0cdd5f98d578c85ca61a6eaacfbb9a00b3437f3e9d7527b9695d173b4dc1
```

## Limitations and closure gate

This evidence covers the current local UTF-8 text/Markdown result contract. PDF/OCR, browser
capture, passive capture, and unavailable hardware were not substituted or claimed. A current-main
full workspace pipe was attempted and hit an honest `ENOSPC` resource boundary while another target
device build was compiling; it is retained as a no-go rather than a pass. The focused native and
Rust 1.88 core suites, exact anchored MVP search, and mixed-corpus recovery are the authoritative
0104 evidence.

## Merged-main reproduction

The same target-device harness was rerun against runtime-tested merged `main` commit
`eee1236710b98375e86b12187d545ed451ee2b7c` on the Mac specified above. The final main tip
`459f9defaebc699fa231de9672e9cd855b77f65a` adds only documentation and roadmap metadata after
that runtime-tested commit; no result-contract source changed.

- Verification directory: `/tmp/loom-0110-main-device.QLkKl1`
- Harness summary SHA-256: `45bc997dcb26b8bc6cbd63a09fa17aed6c5d4ae968ef349be473b0b034e94e70`
- Commands SHA-256: `d840925fb008af9101dc3121870b79960c1b6924451df17a7851f2f6132bb209`
- Log manifest: `/tmp/loom-0110-main-device.QLkKl1/log-sha256.txt`
- Log manifest SHA-256: `ed539c48e8c9b648f0ae341ddf37f02107d5aacff5d5e2a19ae0d28612c57d64`

The full local pipe passed: format, warnings-denied Clippy, workspace tests, Rust 1.88 MSRV
check/tests, `npm ci`, `npm run check`, retrieval benchmark, semantic contract, local security
scan, Tauri debug build, and mixed-corpus failure/recovery. The result-contract integration tests
passed source-backed identity/anchor checks, unsafe-query rejection, stale tuple rejection, and
mutation-negative coverage. Retrieval measured Recall@1/5 1.0, anchor precision 1.0,
false-positive rate 0.0, completeness 1.0, median latency 0.185 ms, and p95 latency 0.518041 ms.

The mixed corpus again reported `discovered=4`, `indexed=2`, `skipped=1`, and one bounded-size
failure; the outside-root symlink remained unreachable. Replacing the oversized file recovered it
on the next index with `indexed=1`, `unchanged=2`, `skipped=1`, no failures, and final stats of 3
artifacts, 3 versions, 3 passages, and 250 indexed bytes. No hosted CI or unavailable hardware
substituted for this target-device evidence. Any future desktop capture must be cropped to the
relevant evidence panel.

## Current-main acceptance reproduction

The passing focused core reproduction was run at
`421bc6d469ba87a144495d0bf470d16ce44ec40f`. The implementation/evidence merge
`39f48f8eafbb71184b90f8ae13fe50e550bbe7dd` and subsequent evidence-only commits changed no LOOM
source files under `crates/loom-core`, `crates/loom-cli`, `src-tauri`, or
`browser-extension`; the source-equivalence check was empty.

The native and Rust 1.88 commands each passed 93 loom-core tests with zero failures, including the
result-contract integration suite. The current-main MVP returned `retry anomalies` from the exact
source URI with a structured highlighted excerpt and character/line anchor; the mixed corpus
covered unsupported input, the oversized boundary, outside-root symlink, and recovery.

| Artifact | SHA-256 |
| --- | --- |
| `/tmp/loom-0102-focused-current.log` | `067ba921fbbceb562010d8bc1b6d05b75f445484579fbb11bf428259d2651435` |
| `/tmp/loom-0102-msrv-current.log` | `7ad063c8e87ee22fe8026d31c866374f7780d279941c511e03e2bb454b000ec1` |
| `/tmp/loom-0102-mixed-current.log` | `87b22f78c8f602d83144561b79ba3c854e078c5bdffb11f5af92239c482f79df` |
| `/tmp/loom-0102-mvp-demo.log` | `99a66c5636731e2397ccef0010b7f1a4a85fe5e4fcd94ddda3a131d694cf99b0` |

The complete current-main pipe is retained at `/tmp/loom-0102-current-main.B40RI8`; formatting and
Clippy passed, while workspace/MSRV compilation stopped at `ENOSPC`. Hosted CI was not used as a
substitute, and no unavailable hardware was involved.
