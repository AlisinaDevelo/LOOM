# LOOM 0204 device evidence

This artifact records the experimental hybrid ranker and its ablation gate for issue
[#23](https://github.com/AlisinaDevelo/LOOM/issues/23), roadmap ID `0204`. It is deliberately an
evaluation result, not a claim that hybrid ranking has shipped in the desktop default.

## Device and toolchain

- Device: MacBook Pro 17,1, Apple M1, 8 GB
- OS: macOS 26.6.2 (25G83), `aarch64-apple-darwin` / arm64
- Native toolchain: `rustc 1.96.0` / Cargo 1.96.0, Node v26.7.0, npm 11.19.0
- Declared MSRV: `rustc 1.88.0` / Cargo 1.88.0
- Runtime-tested source commit for the passing follow-up: `88d0a8941a0f0914f908ac84c1916a7c4a5426f5`

## Acceptance-criterion evidence map

|Artifact ID|Acceptance criterion|Retained evidence|
|---|---|---|
|`LOOM-0204-FUSION`|Fusion uses a documented deterministic algorithm and records per-signal rank evidence|`crates/loom-core/src/ranking.rs`, ADR [`0006`](../adr/0006-hybrid-rank-fusion.md), and `hybrid_ranking.rs` cover weighted reciprocal-rank fusion, exact/path/recency bounds, deterministic tie-breaking, and serialized signal evidence.|
|`LOOM-0204-ABLATION`|An ablation report compares lexical-only, semantic-only, and hybrid retrieval|`scripts/hybrid-ablation.py` runs all three modes against `benchmarks/retrieval/v0` without uploading corpus data. The retained JSON report is listed below.|
|`LOOM-0204-GATE`|Hybrid mode ships only if its preregistered accuracy/latency gate passes|The initial run correctly held the gate because semantic-only candidates produced false positives. The follow-up admission guard passes the same preregistered gate at `eligible`; the desktop default remains lexical-only pending a separate product promotion decision.|

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

## Follow-up admission gate

The first result exposed a concrete failure: semantic-only candidates were admitted solely because
their vector rank was finite, so unrelated passages inflated the hybrid false-positive rate. The
follow-up at source commit `88d0a8941a0f0914f908ac84c1916a7c4a5426f5` adds a deterministic
evidence guard. Lexical candidates remain authoritative; a semantic-only candidate is admitted
only when at least half of the distinct query tokens occur in its canonical passage, title, or
source locator. This preserves a partial-token paraphrase path without allowing an unsupported
semantic tail to become a displayed result.

The final target-device ablation was run with:

```text
CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0 \
  python3 scripts/hybrid-ablation.py > /tmp/loom-0204-hybrid-admission.json
```

The report is retained in `/tmp/loom-0204-device.kL42gw/hybrid-ablation.log` with SHA-256
`c8d6a7eb4a80f86fe082b66da21413a45fac49063dc5fa51243876c906723a29`.

|Mode|Recall@1|Recall@5|Anchor precision|False-positive rate|Median ms|p95 ms|
|---|---:|---:|---:|---:|---:|---:|
|Lexical|1.0|1.0|1.0|0.0|10.974375|13.207458|
|Semantic|1.0|1.0|0.0|0.6666666667|10.201583|12.377708|
|Hybrid|1.0|1.0|1.0|0.0|10.143000|12.191834|

The gate is now `eligible`: accuracy, latency, lexical non-regression, completeness, and failure
checks all pass. The semantic-only slice remains intentionally visible as a diagnostic baseline;
its coarse anchors and false positives are not silently folded into the hybrid claim.

## Verification and limits

The original ablation indexed all three fixtures (`completeness=1.0`) before scoring. The original
hold run is `/tmp/loom-0204-device-final.ozZ6pE`; its evidence remains the reason the admission
guard was added. The follow-up full target-device run is `/tmp/loom-0204-device.kL42gw` with
status `PASS`.
Its summary SHA-256 is
`bca5abd3cfe69a7edbec7ce20e94fe7d6bd879b89f558f5f9a660e83d027e4b9`, commands SHA-256 is
`6e37fe32348909c53a450efc87a331c5d36969bf1d37abee85bbcb891468f5f2`, and the hybrid ablation
log SHA-256 is `c8d6a7eb4a80f86fe082b66da21413a45fac49063dc5fa51243876c906723a29`.
Format, warnings-denied
Clippy, workspace tests, Rust 1.88 check/tests, npm checks, v0/v1 retrieval, semantic contract,
hybrid ablation, security, Tauri debug build, and mixed-corpus recovery all passed. The focused Rust
suite now has five passing tests, including unsupported semantic-only candidate rejection.

The earlier hold run's summary SHA-256 is
`81b5e9e1f8932233dfec0b6ef723d4677ffee1461e93a26fe25c980cbc7e9247`, commands SHA-256 is
`204379bfc3ec06edab0d8c105335a82c71846a51acb88c2ebc7a307d11e7ae97`, and log manifest SHA-256
is `9956a235393b3592a861dcc374acec83ba0d5b8940c5dc57dc60546af77b1570`.
The original run's format, warnings-denied
Clippy, workspace tests, Rust 1.88 check/tests, npm checks, retrieval, semantic contract,
security, Tauri debug build, and mixed-corpus recovery all passed. The focused Rust suite then had four
passing tests. The Python ablation script exited `2` for the expected `hold` decision on that run;
the follow-up exits `0` for `eligible`.

Hybrid search still requires a healthy semantic derivative and is not wired into the desktop
default. Large-corpus quality, multilingual ranking, learned reranking, and participant evidence
remain future work.

The MVP demo was rerun on the same device/source family at `/tmp/loom-mvp-demo-0204.qjkd8f`.
It indexed five selected sources and recovered text, a PDF page, and a cropped OCR region; the
demo log SHA-256 is `01507dbbf23af025c0a95340a8d7c0507d567389a0b47e59450eeab3a25ec699`.

No screenshot was needed as acceptance evidence. If a future desktop capture is added, it must be
cropped to the relevant result/evidence panel.
