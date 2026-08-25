# LOOM 0114 device evidence

This artifact records the FTS5 consistency and repair contract for issue
[#67](https://github.com/AlisinaDevelo/LOOM/issues/67), roadmap ID `0114`. Canonical passage
rows remain authoritative; health compares them with a rebuildable tokenizer projection, and
repair captures before/after evidence for one transactional rebuild. The implementation was
merged through PR [#179](https://github.com/AlisinaDevelo/LOOM/pull/179). The current-main
focused reproduction is recorded below; the roadmap status is advanced after that evidence is
merged and reconciled.

## Device and toolchain

- Device: MacBook Pro 17,1, Apple M1, 8 GB
- OS: macOS 26.6.2 (25G83), `aarch64-apple-darwin` / arm64
- Native toolchain: `rustc 1.96.0` / Cargo 1.96.0, Node v26.7.0, npm 11.19.0
- Declared MSRV toolchain: `rustc 1.88.0` (`6b00bc3880198600130e1cf62b8f8a93494488cc`),
  Cargo 1.88.0
- Source under test: `7916ac150ad44a6027f050124c40a3c01e05af03`
  (`feature/issue-67-fts-repair`)

## Acceptance-criterion evidence map

|Artifact ID|Acceptance criterion|Retained evidence|
|---|---|---|
|`LOOM-0114-HEALTH`|A health command compares canonical passage counts and hashes with the external-content FTS5 index|`Library::fts_health` reports canonical passage count/digest, indexed-document count, expected tokenizer vocabulary digest, actual vocabulary digest, and the SQLite FTS5 integrity result. `corrupted_fts_is_detected_repaired_transactionally_and_repeatable` asserts the healthy baseline and the deliberate drift report. The CLI `fts-health` output is retained in `cli-health.log`.|
|`LOOM-0114-TRANSACTION`|Repair runs in a transaction, is repeatable, and reports the before/after derivative digest without modifying canonical source rows|`Library::repair_fts` issues the FTS5 `rebuild` command inside one SQLite transaction and returns both health reports. The fixture compares `before` with the unhealthy report, verifies the canonical `ArtifactObservation` is unchanged, reruns repair, and asserts the same healthy derivative digest. The CLI `fts-repair` before/after JSON is retained in `cli-repair.log`.|
|`LOOM-0114-CORRUPTION`|A deliberately corrupted fixture is detected, repaired, and returns identical ranked evidence after repair|The fixture deletes one FTS5 row through the FTS5 delete command, observes `indexed_passages=0`, a vocabulary digest mismatch, and no search hit, then repairs and asserts the original ranked `SearchHit` vector is identical. The focused test log records 2 passing tests, including the empty zero-row projection case.|

## Target-device reproduction

The checked-in harness is [`scripts/verify-device.sh`](../../scripts/verify-device.sh):

```text
bash scripts/verify-device.sh /tmp/loom-0114-device-final.TefOWV
```

The run completed with `status=PASS` and exit code 0 at source commit
`7916ac150ad44a6027f050124c40a3c01e05af03`. Format, workspace clippy, the full Rust workspace,
Rust 1.88 MSRV check and tests, `npm ci`, frontend checks, retrieval benchmark, local security
check, Tauri debug build, and the mixed failure/recovery corpus all passed. No GitHub-hosted
Actions result was used.

The rights-clean retrieval benchmark indexed 3/3 fixtures with completeness 1.0, exact-source
Recall@1 1.0, Recall@5 1.0, anchor precision 1.0, false-positive rate 0.0, median latency
0.230167 ms, and p95 latency 0.632209 ms.

The mixed corpus contained supported Markdown, an unsupported binary, an 8,388,609-byte Markdown
file, and an outside-root symlink. The first report retained `run_id=e93b4569-4453-40ab-b800-
c59a7bdc6453`, `discovered=4`, `attempted=4`, `indexed=2`, `skipped=1`, `failed=1`, and
`cancelled=0`; the oversized input was reported with a bounded failure and the outside marker
remained unreachable. Replacing the file recovered it with `indexed=1`, `unchanged=2`, `skipped=1`,
`failed=0`, and `cancelled=0`. Final stats were 3 artifacts, 3 versions, 3 passages, and 250
indexed bytes.

The standalone FTS5 fixture reports 2 passing tests. The CLI fixture indexes one source, reports a
healthy count/digest match, and reports identical before/after values for repeatable repair. The
Python roadmap/activation contract suite passed, and roadmap validation reports 154 active issues,
4 retired issues, 20 milestones, 13 phases, 141 parent edges, and 314 dependency edges.

The local security check found no gitleaks secrets and `npm audit` reported zero vulnerabilities.
This is not a third-party audit or a complete dependency advisory statement; the repository still
has a GitHub Dependabot advisory for `glib`, and `cargo-audit` is not installed on this device.

## Log and digest record

The retained run directory is `/tmp/loom-0114-device-final.TefOWV`. Its `summary.txt`, command
record, individual logs, focused FTS5 output, CLI output, Python contract output, roadmap
validation, and `log-sha256.txt` are the source evidence for this entry:

```text
cli-health.log       sha256:582ff709bb1d0ac210834591d4f13324e829f652a24310fb4074c271af51941d
cli-index.log        sha256:409d33791d5450b7ffec4f636bd4a12381a81319c19fe81cd32f3787b5e35d93
cli-repair.log       sha256:7602d7206bf9f05e8ba228cb60a7ca278fcf4ea246d7f7d7ad701fe12178c66f
clippy.log           sha256:5843f56a64ffafbbe4534cfbeaaf7fba5b1de26b75de835965a8f6dcf8f285fe
fmt.log              sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
fts-repair-tests.log sha256:927b916c4f55dfa1be04baf6a706ce7839ca93ae9666c13e065880ef327728d2
mixed-corpus.log     sha256:6f4ff4d2102da298d846dd0d828f569f35afadb0e8ea05c6043886b269e71d16
npm-check.log        sha256:a322a7254d123e1e2f10e836572881283c7bfb5b04b1890d9d7d85ba46ffbd6c
npm-install.log      sha256:bbacb689fb0c829145033c5a0e39bff2b1fcb458b993473ecadaf839f978c9c0
python-tests.log     sha256:acaa091e21bde4fe8fd480cb2d6087143ab6080782a67577eae96af6add7fc3a
retrieval-benchmark.log sha256:650064abd9bf3e4d609b38bf2588cc5439c1e7c296f9f7ad7544fd70fb1
roadmap-validation.log  sha256:df36f3da734b1902d0f0e6711aeb03645bc9f04afad31993b9275c55f1a82c96
rust-msrv-check.log  sha256:b7557b78247882da6d2a6707115a8ca1a4cbf49061b8fae2140135b8b6ad2e6d
rust-msrv-tests.log  sha256:971699471d3dc791dfbd09b8b57eeebaaea989840235388f442f94af02c25397
rust-workspace.log   sha256:b2150aa8852465f90d2e6f99b49aaeb1395325825b653a7545e900553cb3f35e
security-check.log   sha256:e89aaf82d1c39ec5e5d6cea1211decb31e3666038e30355f078e3e7809841eb6
tauri-build.log      sha256:e138814b2f463278bb306d221b0534050d6c4ce9ab74a449c136efc65cf38220
```

## Limitations and closure gate

This run proves the FTS5 repair contract on the specified Mac; it does not claim another
OS/architecture, a third-party security audit, a `cargo-audit` result, concurrent multi-writer
repair behavior, notarization, or a large-library resource benchmark. Health and repair are local
diagnostics over the canonical SQLite database; they do not reconstruct missing canonical source
bytes. A default profile clean rebuild attempted on current `main` exhausted the device filesystem
before the test binary was produced (`/tmp/loom-0112-focused-current.mjj5wg/source-roots.log`,
SHA-256 `8232052f286f7da873fb5420033ab6b0ec238364b9d0f3f54b5150dcc89b2d03`). The successful
focused rerun below used `CARGO_PROFILE_DEV_DEBUG=0`, `CARGO_INCREMENTAL=0`, and one build job;
no hosted Actions result or protected-branch enforcement was used.

## Current merged-main focused reproduction

The focused FTS5 suite was run against source-equivalent merged `main`: implementation source
commit `87d1e03ffe2a43fed33826df58360e456ca4c753`, with documentation-only current tip
`9c75c6faa0cc232f8e930df886b5fe032207faea`, on the Mac specified above. Both FTS5 tests passed:
healthy zero-row projection and deliberate corruption detection, transactional repair, repeatable
derivative digest, and unchanged canonical evidence. The same low-footprint run also passed the
five source-scope and two cancellation tests used by adjacent v0.1 gates.

- Verification directory: `/tmp/loom-0112-14-focused-current.WFJUFQ`
- Focused log SHA-256: `99827fb413c87051a3a2085c134e05cff0ab44d858b5cf30bdfc1789819d32e2`
- Toolchain: `rustc 1.96.0` / Cargo `1.96.0`, arm64 Apple Silicon
- Result: source roots `5 passed`, cancellation `2 passed`, FTS repair `2 passed`

This focused rerun does not substitute for the unavailable full workspace pipe; the historical
full-pipe artifact above remains labeled with its original source commit. Future desktop captures
must be cropped to the relevant evidence panel.

## Historical merged-main full-pipe reproduction

The earlier target-device harness passed against runtime-tested merged `main` commit
`eee1236710b98375e86b12187d545ed451ee2b7c`; its retained logs and limitations remain useful for
the broader integration claim, but it predates the current documentation-only commits and is not
used to represent the focused current-main rerun above.
