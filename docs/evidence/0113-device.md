# LOOM 0113 device evidence

This artifact records bounded indexing progress and cooperative cancellation for issue
[#66](https://github.com/AlisinaDevelo/LOOM/issues/66), roadmap ID `0113`. It extends the durable
`index_jobs` checkpoint from #0108: a report carries a stable run ID and explicit unit counts, the
desktop can request cancellation, and the worker stops only between complete SQLite units.

## Device and toolchain

- Device: MacBook Pro 17,1, Apple M1, 8 GB
- OS: macOS 26.6.2 (25G83), `aarch64-apple-darwin` / arm64
- Native toolchain: `rustc 1.96.0` / Cargo 1.96.0, Node v26.7.0, npm 11.19.0
- Declared MSRV toolchain: `rustc 1.88.0` (`6b00bc3880198600130e1cf62b8f8a93494488cc`),
  Cargo 1.88.0
- Source under test: `953ab69c0b26c20ef95973969dafbf2bae57bc5f`
  (`feature/issue-66-indexing-cancellation`)

## Acceptance-criterion evidence map

|Artifact ID|Acceptance criterion|Retained evidence|
|---|---|---|
|`LOOM-0113-PROTOCOL`|The job protocol reports discovered, attempted, indexed, skipped, failed, and cancelled counts with a stable run identifier|`IndexReport` now exposes `run_id`, `discovered`, `attempted`, `indexed`, `unchanged`, `skipped`, `failed`, `cancelled`, and `failures`. `cancellation_commits_complete_units_and_resumes_to_uninterrupted_rows` asserts the first run ID is non-empty, counts `3/1/1/0/0/0/2`, and observes the same run ID after resume. The mixed-corpus log retains the report fields for both the bounded failure and recovery passes.|
|`LOOM-0113-CANCEL`|Cancellation commits only complete artifact versions and leaves an explicit resumable checkpoint or terminal cancellation reason|The cancellation fixture requests a stop after one completed unit, finds exactly one searchable artifact, and verifies `index_jobs.state = interrupted`, `next_unit = 1`, `total_units = 3`, and `last_error = cancelled by request`. The pre-cancelled-token fixture proves zero units are written and all discovered units are reported cancelled.|
|`LOOM-0113-CONVERGENCE`|Interruption fixtures converge to the same canonical rows as an uninterrupted run|After cancellation, the resumed run completes the remaining two units with the original run ID. Its `LibraryStats` and every `ArtifactObservation` (source URI, BLAKE3 hash, extractor identity, passage hashes, and anchors) equal a fresh uninterrupted in-memory index. The durable checkpoint reaches `completed`.|
|`LOOM-0113-DESKTOP`|Users can request bounded cancellation through the desktop path|`cancel_indexing` owns a local `IndexCancellationToken`, is registered in the Tauri handler and capability manifest, and is invoked by the UI's “Stop indexing” control. The frontend test verifies the command call and the visible resumable-run notice. The Tauri debug build and desktop contract test passed.|

## Target-device reproduction

The checked-in harness is [`scripts/verify-device.sh`](../../scripts/verify-device.sh):

```text
bash scripts/verify-device.sh /tmp/loom-0113-device-final.UJgm1H
```

The run completed with `status=PASS` and exit code 0 at source commit
`953ab69c0b26c20ef95973969dafbf2bae57bc5f`. Format, clippy, the full Rust workspace, Rust 1.88
MSRV check and tests, `npm ci`, frontend lint/tests/build, retrieval benchmark, local security
check, Tauri debug build, and the mixed failure/recovery corpus all passed. The workspace includes
the two cancellation fixtures; the frontend check ran 2 files and 8 tests.

The rights-clean retrieval benchmark indexed 3/3 fixtures with completeness 1.0, exact-source
Recall@1 1.0, Recall@5 1.0, anchor precision 1.0, false-positive rate 0.0, median latency
0.378708 ms, and p95 latency 0.391167 ms.

The mixed corpus contained supported Markdown, an unsupported binary, an 8,388,609-byte Markdown
file, and an outside-root symlink. The first report retained `run_id`, `discovered=4`,
`attempted=4`, `indexed=2`, `skipped=1`, `failed=1`, and `cancelled=0`; the oversized input was
reported with a bounded failure and the outside marker remained unreachable. Replacing the file
recovered it with `indexed=1`, `unchanged=2`, `skipped=1`, `failed=0`, and `cancelled=0`. Final
stats were 3 artifacts, 3 versions, 3 passages, and 250 indexed bytes.

The separate cancellation test log reports 2 passing tests. The Python roadmap/activation contract
suite reports 19 passing tests, and roadmap validation reports 154 active issues, 4 retired issues,
20 milestones, 13 phases, 141 parent edges, and 314 dependency edges.

The local security check found no gitleaks secrets and `npm audit` reported zero vulnerabilities.
This is not a third-party audit or a complete dependency advisory statement; the repository still
has a GitHub Dependabot advisory for `glib`, and `cargo-audit` is not installed on this device.
No GitHub-hosted Actions result was used.

## Log and digest record

The retained run directory is `/tmp/loom-0113-device-final.UJgm1H`. Its `summary.txt`, command
record, individual logs, focused cancellation output, Python contract output, roadmap validation,
and `log-sha256.txt` are the source evidence for this entry:

```text
cancellation-tests.log  sha256:7cd2e10718183f8bc8eb494ff089efabf1c6b7080519f19c1df6371ef26a1e96
clippy.log              sha256:5b1407cea21d9e030840c641d4bfb894fc106b125d2300b0f0a0265bed398856
fmt.log                 sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
mixed-corpus.log        sha256:53558935f3b384f520704bbfa043c0ee38aaf7d38721ad8946b473fafed51183
npm-check.log           sha256:1ab2d05e16caf0bd877532a1c9211e132a967b10ea4fc6926861984838e62987
npm-install.log         sha256:85b4814e52ba71ae231930f0e73b027fadd1667b8aede7ff5c562e1a0a4e22fe
python-tests.log        sha256:9b8178ca61790f22a5e16f68f98c8b8f8d7872fb45c706f1f8bc7e79fca974dd
retrieval-benchmark.log sha256:a5c2589bccbc6714405f749e772d582ab35a751a08f97d4c31acfcb3a7f15bbb
roadmap-validation.log  sha256:df36f3da734b1902d0f0e6711aeb03645bc9f04afad31993b9275c55f1a82c96
rust-msrv-check.log     sha256:0cfc7b0da2afa882926f8b01eb1ec147da022206092e987472535bd97ba91275
rust-msrv-tests.log     sha256:bc4ccd172137f3dd4157947a00ebbb9bccc0a07a4aee696c60931b7d0ce8979e
rust-workspace.log      sha256:38ee9dc0763ee7c25386587b65d95ac5255f32c99e73073e61b568746f445211
security-check.log      sha256:f95158a58634c58a28c2310d050dc3f1006562bcfc20d79cd535f72663f8e2cd
tauri-build.log         sha256:94b49c85c9a589e2b81c37bd8f9978472432c03d7a388ce0fd60369250194b1a
```

## Limitations and closure gate

This run proves the bounded cancellation contract on the specified Mac; it does not claim another
OS/architecture, continuous progress streaming, cancellation inside one individual file read,
notarization, a third-party security audit, a `cargo-audit` result, or a large-library resource
benchmark. The current desktop stop action is cooperative and waits for the current bounded unit
to finish. Issue 66 remains open until independent review, a protected-main merge, and the same
reproduction against the merged `main` SHA are available.
