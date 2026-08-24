# LOOM 0105 device evidence

This artifact records the retrieval benchmark v0 schema, threshold gate, and reproducibility
verification for issue [#14](https://github.com/AlisinaDevelo/LOOM/issues/14). The change is stacked
on the source-backed result contract in PR [#166](https://github.com/AlisinaDevelo/LOOM/pull/166);
issue #14 stays open until review, merge, and a repeat against the merged `main` SHA.

## Device and toolchain

- Device: MacBook Pro 17,1, Apple M1, 8 GB
- OS: macOS 26.6.2 (25G83), `aarch64-apple-darwin` / arm64
- Native toolchain: `rustc 1.96.0` / Cargo 1.96.0, Node v26.7.0, npm 11.19.0
- Declared MSRV toolchain: `rustc 1.88.0` (`6b00bc3880198600130e1cf62b8f8a93494488cc`),
  Cargo 1.88.0
- Source under test: `4a35c1cdd87f61c19a09fd9d3f6bbb52b978d53b`
  (`feature/issue-14-benchmark-v0`)

## Acceptance-criterion evidence map

| Artifact ID | Acceptance criterion | Retained evidence |
| --- | --- | --- |
| `LOOM-0105-SCHEMA` | Benchmark records source class, expected artifact/anchor, and acceptable alternatives | `benchmarks/retrieval/v0/queries.jsonl` gives every query an explicit `acceptable_alternatives` list; the runner validates each alternative fixture and exact anchor. Unit test `tests::matching_expectation_accepts_declared_alternative_sources` proves alternate-source selection. |
| `LOOM-0105-METRICS` | Deterministic command reports required retrieval and resource metrics | `loom benchmark` validates raw fixture hashes, extractor identity/version, passage hashes/anchors, index completeness, and emits Recall@1/5, anchor precision, false-positive rate, completeness, median/p95 latency, source-type breakdown, and failure details. The retained run reports all required fields for 3 local-text queries. |
| `LOOM-0105-THRESHOLD` | CI/local gate rejects quality regressions without private corpus data | `manifest.json` declares Recall@1 >= 1.0, Recall@5 >= 1.0, anchor precision >= 1.0, false-positive rate <= 0.0, and completeness >= 1.0. `benchmark_passes` is exercised by a passing case and rejected false-positive/incomplete-index cases; the command exits non-zero when a threshold fails. The corpus is synthetic CC0 text only. |
| `LOOM-0105-FAILURE-RECOVERY` | Benchmark and retrieval changes remain bounded and source-safe | The full harness also exercises unsupported/binary input, the 8,388,609-byte limit, outside-root symlink containment, replacement recovery, exact anchors, stale opening, and no-network local operation. |

## Target-device reproduction

The checked-in harness is [scripts/verify-device.sh](../../scripts/verify-device.sh):

```text
bash scripts/verify-device.sh /tmp/loom-0105-device-final.uWE5Mi
```

The run completed with `status=PASS` and exit code 0 at source commit
`4a35c1cdd87f61c19a09fd9d3f6bbb52b978d53b`. Format, clippy, the Rust workspace, MSRV check and
tests, `npm ci`, `npm run check`, the retrieval benchmark, the Tauri debug build, and the mixed
corpus all passed. The Rust workspace reported 7 CLI unit tests, 19 core unit tests, 1
fixture-contract integration test, 1 result-contract integration test, and 5 CLI tests, with no
failures. The MSRV test run passed the 19 core unit tests and both integration tests.

The benchmark indexed 3/3 synthetic fixtures with completeness 1.0, exact-source Recall@1/5 1.0,
anchor precision 1.0, false-positive rate 0.0, median latency 0.410125 ms, and p95 latency
0.574208 ms. The report included the declared threshold object and an empty failure list.

The mixed corpus contained supported Markdown, an unsupported binary, an 8,388,609-byte Markdown
file, and an outside-root symlink. Initial indexing reported `discovered=4`, `indexed=2`,
`skipped=1`, and one bounded-size failure; the outside marker was not searchable. Replacing the
oversized file recovered it on the next index with `indexed=1`, `unchanged=2`, `skipped=1`, and no
failures. Final stats were 3 artifacts, 3 versions, 3 passages, and 250 indexed bytes.

## Log and digest record

The full log manifest is `/tmp/loom-0105-device-final.uWE5Mi/log-sha256.txt`:

```text
clippy.log              sha256:713ac216ed094853204d90f0aa3bb1ba9983c4f8603ad9766f392136ecff1441
fmt.log                 sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
mixed-corpus.log        sha256:627fbe5554ac2637438e2f2bb9c46e7ca1ff020e188adebcc5093456529cf618
npm-check.log           sha256:e53cc73aed8637464fecf50724834cb73b55cb9acb09f0809185285fcb69917d
npm-install.log         sha256:85b4814e52ba71ae231930f0e73b027fadd1667b8aede7ff5c562e1a0a4e22fe
retrieval-benchmark.log sha256:092157636d2d1f4c1f7f47528653890bff06cc2a748253685598617e2423c033
rust-msrv-check.log     sha256:3c71c150625fb029a7551260ae3caf6df97b293ea57be4bdba7d509d83b00698
rust-msrv-tests.log     sha256:267dcc7d101c7eab54c74343edd4039353b1cae18c6c8674438678db237f4958
rust-workspace.log      sha256:2b7137634972c32886281598aeecca9ee44953dc723c621cd2883d2e85bbcfce
tauri-build.log         sha256:a87dcd019b2d9c611bb8ec6357b3731c883dc2bb336912a1d7440066d5061e9d
```

## Limitations and closure gate

This is a deterministic, rights-clean local-text smoke benchmark, not evidence of real-world
retrieval quality. PDF/OCR, screenshots, saved web pages, passive capture, and unavailable
hardware were not substituted or claimed. Review, protected-main merge, the same reproduction
against the merged SHA, and final issue-linked evidence remain required before #14 can close.
