# Device evidence: LOOM-0207

This record closes the 100k-artifact performance gate from merged `main`.

| Field | Value |
| --- | --- |
| Source commit | `e8314967cbf8d99dc216bd78f5db2cf62b302c9c` |
| Device | Apple Silicon Mac, `arm64` |
| OS | macOS 26.6.2 (25G83) |
| Rust | `rustc 1.96.0`, MSRV `rustc 1.88.0` |
| Node/npm | Node `v26.7.0`, npm `11.19.0` |
| Pipe | `CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_PROFILE_DEV_STRIP=symbols CARGO_PROFILE_TEST_STRIP=symbols CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 bash scripts/verify-device.sh /tmp/loom-0207-main-device-authoritative.r9i72X` |
| Pipe result | `PASS` |

## Acceptance-criterion evidence map

| Artifact ID | Criterion | Retained evidence |
| --- | --- | --- |
| `LOOM-0207-HARNESS` | Reproducible 10k/100k harness with documented corpus, hardware, OS, and cache conditions | `scripts/performance-budget.py`, generator version `loom-performance-corpus-v1`, seed `20260824`, 20,000-file generation shards, one selected root, two runs per scale, and `performance/report.json` (SHA-256 `b8053b8cf37ab66dab048f6e9bb361631e11d256c6444b2e765dd1436207bd62`) |
| `LOOM-0207-BUDGET` | Numeric pre-optimization budgets and measured throughput, query, memory, disk, CPU proxy, and rebuild cost | The report's `pre_optimization_budgets`, `scales`, and `release_gate` fields; all six 100k checks passed |
| `LOOM-0207-PROFILE` | Variance, limitations, and profiling-informed decision | Four `/usr/bin/time -lp` profiles, per-scale variance, generator limitations, and the explicit no-optimization-until-bottleneck decision in the report |
| `LOOM-0207-GATE` | Every exceeded budget receives a release disposition | 100k `release_gate.status = pass`, `exceeded_count = 0`, `all_exceedances_have_disposition = true` |

## Scale results

The synthetic corpus is rights-clean local Markdown/plain text (70% Markdown, 30% plain text), with
one deterministic evidence query per scale. “Cold” is the first query after opening/indexing a new
SQLite connection; “warm” is 31 repeated queries in that process. The macOS page cache was not
flushed and battery energy was not claimed.

| Scale | Runs | Indexed | Source bytes | Index throughput | Warm p95 | Max RSS | DB amplification | FTS rebuild |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 10,000 | 2 | 10,000/run | 2,110,000/run | 1,645–1,699 artifacts/s | 1.047–1.074 ms | 30.4 MiB | 11.49–11.52× | 0.520–0.549 s |
| 100,000 | 2 | 100,000/run | 21,100,000/run | 1,101–1,149 artifacts/s | 0.420–0.429 ms | 105.7–121.6 MiB | 10.12–10.13× | 6.179–6.377 s |

The 100k release budgets were: throughput ≥100 artifacts/s, warm p95 ≤25 ms, RSS ≤1 GiB,
database amplification ≤128×, CPU proxy ≤20 seconds per 1,000 artifacts, and FTS rebuild ≤120 s.
The worst observed values were 1,100.67 artifacts/s, 0.419667 ms, 127,483,904 bytes RSS,
10.1312×, 0.662 CPU seconds per 1,000 artifacts, and 6.377 s respectively.

## Related device checks

The same merged-main pipe also passed formatting, clippy, the Rust workspace, Rust 1.88 check and
tests, v0 and v1 retrieval, the hybrid ablation gate, semantic rebuild/drop/rebuild, mixed-corpus
outside-root and oversized-source recovery, npm checks, the local security check, and the Tauri
debug build. The v1 diagnostic continues to retain its known q008 paraphrase miss and q009
hard-negative duplicate; it is not hidden by the scale gate. The screenshot fixtures remain the
deliberately cropped OCR images from v1 (rather than full-screen captures).

The hybrid gate was eligible with Recall@1/5 `1.0`, anchor precision `1.0`, and false-positive rate
`0.0`; its separate semantic-only diagnostic remains Recall@1/5 `1.0` with anchor precision `0.0`
and false-positive rate `0.6667`, as documented in the earlier hybrid evidence.

## Reproduction artifacts and hashes

Evidence directory: `/tmp/loom-0207-main-device-authoritative.r9i72X`.

- `commands.txt` SHA-256: `b9ce374200104373fa32b327a401d70a76cf5d654563350f56ff7110e7cbd316`
- `summary.txt` SHA-256: `0d3d297926fc739674a6a8bf6dbc0ee7665025fa0d6c8993466b0b0e61f030e9`
- `performance/report.json` SHA-256: `b8053b8cf37ab66dab048f6e9bb361631e11d256c6444b2e765dd1436207bd62`
- staged CLI SHA-256: `8ea3f70a1e695b5ea4d0ef7c3c11f67597f91c2e1f89778113be856bc72f99f0`
- top-level log hashes: `/tmp/loom-0207-main-device-authoritative.r9i72X/log-sha256.txt`
- raw 10k/100k timing and JSON hashes: `/tmp/loom-0207-main-device-authoritative.r9i72X/performance/`

The implementation was reviewed and merged through [PR #201](https://github.com/AlisinaDevelo/LOOM/pull/201),
the staged-binary follow-up [PR #202](https://github.com/AlisinaDevelo/LOOM/pull/202), and the
sequential-toolchain follow-up [PR #203](https://github.com/AlisinaDevelo/LOOM/pull/203). Hosted
GitHub Actions were queued but were not used as acceptance evidence.
