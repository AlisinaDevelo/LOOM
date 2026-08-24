# LOOM 0204 device evidence

This artifact records the experimental hybrid ranker and its ablation gate for issue
[#23](https://github.com/AlisinaDevelo/LOOM/issues/23), roadmap ID `0204`. It is deliberately an
evaluation result, not a claim that hybrid ranking has shipped in the desktop default.

## Device and toolchain

- Device: MacBook Pro 17,1, Apple M1, 8 GB
- OS: macOS 26.6.2 (25G83), `aarch64-apple-darwin` / arm64
- Native toolchain: `rustc 1.96.0` / Cargo 1.96.0, Node v26.7.0, npm 11.19.0
- Declared MSRV: `rustc 1.88.0` / Cargo 1.88.0
- Runtime-tested source commit: `edfe25d19a088688f9c6e4c645341d7ed2826717`

## Acceptance-criterion evidence map

|Artifact ID|Acceptance criterion|Retained evidence|
|---|---|---|
|`LOOM-0204-FUSION`|Fusion uses a documented deterministic algorithm and records per-signal rank evidence|`crates/loom-core/src/ranking.rs`, ADR [`0006`](../adr/0006-hybrid-rank-fusion.md), and `hybrid_ranking.rs` cover weighted reciprocal-rank fusion, exact/path/recency bounds, deterministic tie-breaking, and serialized signal evidence.|
|`LOOM-0204-ABLATION`|An ablation report compares lexical-only, semantic-only, and hybrid retrieval|`scripts/hybrid-ablation.py` runs all three modes against `benchmarks/retrieval/v0` without uploading corpus data. The retained JSON report is listed below.|
|`LOOM-0204-GATE`|Hybrid mode ships only if its preregistered accuracy/latency gate passes|The first run keeps the gate at `hold`: hybrid meets Recall@1/5, anchor precision, latency, and lexical non-regression, but false-positive rate is `0.6666666667` against the `0.0` threshold. The desktop default remains lexical-only.|

## Target-device ablation

Command:

```text
python3 scripts/hybrid-ablation.py > /tmp/loom-0204-hybrid-ablation-final2.json
```

The command built the local CLI, indexed the rights-clean three-file corpus, rebuilt the local
semantic derivative, and evaluated three queries in each mode. No source content left the device.
The retained report SHA-256 is
`09fd08b142325399314a04fc1be7ea984ddea9db74f5887909cdc738ed86ab9c`.

|Mode|Recall@1|Recall@5|Anchor precision|False-positive rate|Median ms|p95 ms|
|---|---:|---:|---:|---:|---:|---:|
|Lexical|1.0|1.0|1.0|0.0|10.508625000000048|10.548292000000071|
|Semantic|1.0|1.0|0.0|0.6666666667|9.880749999999994|10.158541999999994|
|Hybrid|1.0|1.0|1.0|0.6666666667|11.002375000000008|11.376082999999927|

The semantic-only anchor result is intentionally coarse because semantic candidates retain the
passage anchor rather than a lexical query span. Hybrid preserves the lexical anchor when a
lexical candidate exists, but its union still exposes semantic false positives on this fixture.
That is a real gate failure, not a reason to hide candidates or lower the threshold.

## Verification and limits

The ablation indexed all three fixtures (`completeness=1.0`) before scoring. The exact-commit
target-device run is `/tmp/loom-0204-device-final.ozZ6pE` with status `PASS`.
Its summary SHA-256 is
`81b5e9e1f8932233dfec0b6ef723d4677ffee1461e93a26fe25c980cbc7e9247`, commands SHA-256 is
`204379bfc3ec06edab0d8c105335a82c71846a51acb88c2ebc7a307d11e7ae97`, and log manifest SHA-256
is `9956a235393b3592a861dcc374acec83ba0d5b8940c5dc57dc60546af77b1570`. Format, warnings-denied
Clippy, workspace tests, Rust 1.88 check/tests, npm checks, retrieval, semantic contract,
security, Tauri debug build, and mixed-corpus recovery all passed. The focused Rust suite has four
passing tests covering deterministic fusion, bounded signals, invalid-query failure, and
evidence-bound `Library::hybrid_search`. The Python ablation script passes its own execution but
exits `2` for the expected `hold` decision. Hybrid search requires a healthy semantic derivative
and is not wired into the desktop default. Large-corpus quality, multilingual ranking, learned
reranking, and participant evidence remain future work.

No screenshot was needed as acceptance evidence. If a future desktop capture is added, it must be
cropped to the relevant result/evidence panel.
