# LOOM 0110 device evidence

This artifact records the v0.1 activation and recovery decision contract for issue [#63](https://github.com/AlisinaDevelo/LOOM/issues/63).
The gate is intentionally a product hypothesis: its machine-readable status is `hypothesis` and
`measurement_status` is `not_run` until a rights-clean benchmark and a consented participant study
produce retained evidence.

## Device and toolchain

- Device: MacBook Pro 17,1, Apple M1, 8 GB
- OS: macOS 26.6.2 (25G83), `aarch64-apple-darwin` / arm64
- Native toolchain: `rustc 1.96.0` / Cargo 1.96.0, Node v26.7.0, npm 11.19.0
- Declared MSRV toolchain: `rustc 1.88.0` (`6b00bc3880198600130e1cf62b8f8a93494488cc`),
  Cargo 1.88.0
- Source under test: `7901e28a3d7c17a105a154d08506417eddf09bde`
  (`feature/issue-63-activation-gate`)

## Acceptance-criterion evidence map

|Artifact ID|Acceptance criterion|Retained evidence|
|---|---|---|
|`LOOM-0110-THRESHOLDS`|Activation thresholds and decision outcomes are explicit, machine-readable, and not presented as measured claims|`benchmarks/retrieval/v0/gate.json` declares `status: hypothesis`, `measurement_status: not_run`, six numeric thresholds, and explicit `advance`, `narrow`, and `stop` rules. `docs/ACTIVATION_GATE.md` explains that a narrow or stop decision is valid.|
|`LOOM-0110-FIXTURE`|The benchmark fixture is rights-clean and reproducible on the target device|The target-device harness indexed 3/3 synthetic fixtures with completeness 1.0, exact-source Recall@1 1.0, Recall@5 1.0, anchor precision 1.0, false-positive rate 0.0, median latency 0.456167 ms, and p95 latency 0.924458 ms. These are smoke-fixture observations only; they do not satisfy the participant-study gate.|
|`LOOM-0110-WORKSHEET`|Participant measurement records aggregate outcomes while excluding private source material|`docs/studies/v0.1-participant-worksheet.md` bounds the study to 12–20 Mac design partners, records failure classes and aggregate timings, and prohibits raw source text, screenshots, URLs, credentials, raw queries, and private documents. `tests/test_activation_gate.py` asserts the privacy and failure-class fields.|
|`LOOM-0110-TRACEABILITY`|Claims and future decisions link back to canonical product/evaluation/roadmap documents|The activation document links `README.md`, `docs/EVALUATION.md`, `docs/PRODUCT.md`, and `docs/ROADMAP.md`; the Python contract tests verify those links and fixture/worksheet paths.|

## Target-device reproduction

The checked-in harness is [`scripts/verify-device.sh`](../../scripts/verify-device.sh):

```text
bash scripts/verify-device.sh /tmp/loom-0110-device-final.1RGC65
```

The run completed with `status=PASS` and exit code 0 at source commit
`7901e28a3d7c17a105a154d08506417eddf09bde`. Format, clippy, the full Rust workspace, MSRV check
and tests, `npm ci`, `npm run check`, retrieval benchmark, local security check, Tauri debug build,
and the mixed failure/recovery corpus all passed. The Python contract suite ran 19 tests with no
failures; the frontend check ran 2 files and 6 tests; the Rust MSRV run passed 23 core unit tests,
3 durable/observation integration tests, one fixture contract test, and one result contract test.

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

The retained run directory is `/tmp/loom-0110-device-final.1RGC65`. Its `summary.txt`, command
record, individual logs, and `log-sha256.txt` are the source evidence for this entry:

```text
clippy.log              sha256:0674b29b50c3dec56646820dd880f72135797e8fbae612e62ca8370ee4ddce1c
fmt.log                 sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
mixed-corpus.log        sha256:fcda1ee53fdf6560cbcd8bbc5d27e8dc1f913deee30273a7caf8a0566b83bdda
npm-check.log           sha256:71eca57ff2fd90d220e67d3115308d23180904e85db4ff472a73ed3ff2271aa2
npm-install.log         sha256:85b4814e52ba71ae231930f0e73b027fadd1667b8aede7ff5c562e1a0a4e22fe
retrieval-benchmark.log sha256:5e5ee074e34fe67b9838ffe486f190c576c06e84ecd40bcb44a3b43a42725a5f
rust-msrv-check.log     sha256:b1a9e5d24cd1646a54d70c6bcfd6f741b63b6001e2188f1aa294ee13a1b9dc9a
rust-msrv-tests.log     sha256:fec2acdcbeec3d7868a2f1d6f02242dadc574760f7a80c9d459ae677369d0dc7
rust-workspace.log      sha256:36748ac3c91d04827e3e58c15e99a9f101c5f03693f4185770cc2e2d50fb5cef
security-check.log      sha256:5b4660630783ef7bd8b7921cea5a7e013dd583ecf1934431c9318c0732fecd3e
tauri-build.log         sha256:362e37d8356c187f5398fa0eb499c5758c0dd279f2f06bc89988f9b0d198d788
```

## Limitations and closure gate

This run proves the contract is explicit, privacy-safe by construction, and reproducible on the
specified Mac. It does not claim that the activation gate has passed: no 12–20 participant study,
returning-participant observation, capture-friction aggregate, or held-out personal corpus has
been collected. It also does not claim another OS/architecture, notarization, a third-party
security audit, or a cargo-audit result. The post-merge reproduction below satisfies the code and
device-evidence portion of the gate; Issue 63 remains open until independent review and a
protected-main policy are available.

## Merged-main reproduction

The same `scripts/verify-device.sh` harness was rerun against the current merged `main` commit
`eee1236710b98375e86b12187d545ed451ee2b7c` on the target Mac. This is a documentation-only merge
after the runtime-tested code commit `ae102616700a0913f21af118609e727df9617e26`; no runtime source
changed between the tested code and the current main tip.

- Verification directory: `/tmp/loom-0110-main-device.QLkKl1`
- Target: MacBook Pro 17,1, Apple M1, 8 GB; macOS 26.6.2 (25G83); arm64
- Toolchains: Rust/Cargo 1.96.0; Rust/Cargo 1.88.0 MSRV; Node v26.7.0; npm 11.19.0
- Harness summary SHA-256: `45bc997dcb26b8bc6cbd63a09fa17aed6c5d4ae968ef349be473b0b034e94e70`
- Commands SHA-256: `d840925fb008af9101dc3121870b79960c1b6924451df17a7851f2f6132bb209`
- Semantic summary SHA-256: `918a467ce0c167d6c5c7c4b58a6abe4d86800d2dd249834334c3e5161bcb049f`
- Log manifest: `/tmp/loom-0110-main-device.QLkKl1/log-sha256.txt`
- Log manifest SHA-256: `ed539c48e8c9b648f0ae341ddf37f02107d5aacff5d5e2a19ae0d28612c57d64`
- Semantic digest manifest SHA-256: `0362c5d85828e233a1ecd960ca5df369b4925396d975eb8e77a32d220f155b41`
- Python contract/roadmap suite: 19 tests passed; output SHA-256:
  `9c807693c7739522372f08678fc1caf623fd2c5faff97ae29aaec4176e636be5`

The full target-device pipe passed: format, warnings-denied Clippy, workspace tests, Rust 1.88
MSRV check/tests, `npm ci`, `npm run check`, retrieval benchmark, semantic contract, local
security scan, Tauri debug build, and mixed-corpus failure/recovery. Retrieval measured Recall@1/5
1.0, anchor precision 1.0, false-positive rate 0.0, completeness 1.0, median latency 0.185 ms,
and p95 latency 0.518041 ms. Semantic rebuild took 0.55 s end-to-end with a 13,369,344-byte
maximum resident set size; the derivative reported `rebuild_repeatable: true`,
`drop_fails_closed: true`, and `evidence_bound_search: true`. The synthetic fixture remained
three passages/533 bytes and the benchmark binary was 18,853,720 bytes.

The mixed corpus exercised a supported Markdown file, an unsupported binary, an 8,388,609-byte
file, and an outside-root symlink. The first index reported `discovered=4`, `indexed=2`,
`skipped=1`, and one bounded-size failure; the outside marker stayed unreachable. Replacing the
oversized file recovered it on the next index with `indexed=1`, `unchanged=2`, `skipped=1`, no
failures, and final stats of 3 artifacts, 3 versions, 3 passages, and 250 indexed bytes.

One earlier post-merge attempt at `/tmp/loom-0110-main-device.YSik8I` was stopped by the device's
full disk during the Rust linker (`errno=28`, no space left on device); its result is not counted
as a test failure. After removing only task-owned generated caches and preserving unrelated Trash
entries, the complete rerun above passed. No hosted CI or unavailable hardware substituted for
this target-device evidence. No screenshot was needed; any future desktop capture must be cropped
to the relevant result/evidence panel.
