# LOOM 0107 device evidence

This artifact records the local CI, dependency, secret-scan, and release-hygiene verification
for issue [#16](https://github.com/AlisinaDevelo/LOOM/issues/16). The change is stacked on desktop
truth PR [#168](https://github.com/AlisinaDevelo/LOOM/pull/168); issue #16 stays open until review,
merge, and the same reproduction against the merged `main` SHA.

## Device and toolchain

- Device: MacBook Pro 17,1, Apple M1, 8 GB
- OS: macOS 26.6.2 (25G83), `aarch64-apple-darwin` / arm64
- Native toolchain: `rustc 1.96.0` / Cargo 1.96.0, Node v26.7.0, npm 11.19.0
- Declared MSRV toolchain: `rustc 1.88.0` (`6b00bc3880198600130e1cf62b8f8a93494488cc`),
  Cargo 1.88.0
- Security tools: gitleaks 8.30.1; `cargo-audit` was not installed, so no cargo-audit result is
  claimed
- Source under test: `a244dc2790a3e1799c55093d293165eb27911cbc`
  (`feature/issue-16-ci-security`)

## Acceptance-criterion evidence map

| Artifact ID | Acceptance criterion | Retained evidence |
| --- | --- | --- |
| `LOOM-0107-CI-PARITY` | CI runs Rust fmt/clippy/tests and frontend lint/tests/build on supported runners | Existing [`.github/workflows/ci.yml`](../../.github/workflows/ci.yml) defines roadmap, Rust, MSRV, advisory, frontend, and macOS Tauri jobs. [`scripts/verify-device.sh`](../../scripts/verify-device.sh) runs the equivalent local target-device checks and recorded PASS for fmt, clippy, the full Rust workspace, MSRV check/tests, `npm ci`, `npm run check`, retrieval benchmark, Tauri debug build, and the mixed corpus. Hosted GitHub Actions were not used as evidence this week. |
| `LOOM-0107-DEPENDENCIES` | Dependabot covers Cargo, npm, and Actions; dependency review runs on pull requests | [`.github/dependabot.yml`](../../.github/dependabot.yml) contains weekly Cargo, npm, and GitHub Actions updates. [`.github/workflows/dependency-review.yml`](../../.github/workflows/dependency-review.yml) invokes the pinned dependency-review action for pull requests. The local run also completed `npm audit --audit-level=high` with zero vulnerabilities and locked `cargo metadata`. |
| `LOOM-0107-SECRET-SCAN` | A secret scan passes before public push | [`scripts/security-check.sh`](../../scripts/security-check.sh) requires gitleaks, runs a redacted repository scan, and runs the dependency checks. On this device gitleaks 8.30.1 scanned 26 commits and about 961 KB with `no leaks found`; npm audit reported `found 0 vulnerabilities`. |
| `LOOM-0107-RELEASE-HYGIENE` | Formatting and release hygiene are checked before publication | `cargo fmt --all --check`, `git diff --check HEAD^ HEAD`, the full local harness, and the final worktree inspection passed. The commit contains source/docs only; no target, build, coverage, cache, log, or binary output is tracked. |

## Target-device reproduction

The checked-in harness is [`scripts/verify-device.sh`](../../scripts/verify-device.sh):

```text
bash scripts/verify-device.sh /tmp/loom-0107-device-rerun.YjqVTP
```

The run completed with `status=PASS` and exit code 0 at source commit
`a244dc2790a3e1799c55093d293165eb27911cbc`. Format, clippy, the Rust workspace, MSRV check and
tests, `npm ci`, `npm run check`, the retrieval benchmark, the local security check, the Tauri
debug build, and the mixed corpus all passed. The frontend check and Rust test logs contain no
failures.

The retrieval benchmark indexed 3/3 synthetic fixtures with completeness 1.0, exact-source
Recall@1 1.0, Recall@5 1.0, anchor precision 1.0, false-positive rate 0.0, median latency
0.416208 ms, and p95 latency 0.433584 ms.

The mixed corpus contained supported Markdown, an unsupported binary, an 8,388,609-byte Markdown
file, and an outside-root symlink. Initial indexing reported `discovered=4`, `indexed=2`,
`skipped=1`, and one bounded-size failure. The outside marker remained unreachable. Replacing the
oversized file recovered it on the next index with `indexed=1`, `unchanged=2`, `skipped=1`, and no
failures. Final stats were 3 artifacts, 3 versions, 3 passages, and 250 indexed bytes.

## Log and digest record

The retained run directory is `/tmp/loom-0107-device-rerun.YjqVTP`. Its `summary.txt`, command
record, individual logs, and `log-sha256.txt` are the source evidence for this entry:

```text
clippy.log              sha256:665713f13f1b6c51ffcb5e158a1b13311fccbc686da992e30a7d1ed4378e5d39
fmt.log                 sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
mixed-corpus.log        sha256:a24047caebbd37e1df72220f6afae4c22611a34c128d4892c3329ca6050b2a1b
npm-check.log           sha256:22220de792bf15b505d474c10f590a654e48cd8f9e32ec7f071eeb5ba915369c
npm-install.log         sha256:dc4ec24415b0273a6bc19da31dce328ccf706477e3977f6f841e737d67d7601b
retrieval-benchmark.log sha256:17e75b776c71a0d4047efd9dd251e84469ff0599167568eaf6c10a162d9b7b49
rust-msrv-check.log     sha256:b295cfeaaf8e1f948fdce11e8289d785a1983d9f746c60c322cdc557e46121d1
rust-msrv-tests.log     sha256:01aa5ac17e2c63e89ca334545c012fefe6aa82262b762234b9462dc8b8bade2e
rust-workspace.log      sha256:a06b910b3363ad25d1e502c61ea5ae74ab1c3eaee2e52f39c66e9b00c96c2b8f
security-check.log      sha256:644143efbad880eab6c4a34964914c15f485d3c55078b81167c400a059beff46
tauri-build.log         sha256:48a383632604f9c3df642514113aba8d760cff9f38e88e639075838c1b45d4f4
```

## Limitations and closure gate

This run proves local parity and supply-chain hygiene on the specified Mac; it does not claim a
hosted Actions result, a cargo-audit result, notarization, a third-party security audit, or a
different OS/architecture. Review, protected-main merge, the same reproduction against the merged
SHA, and final issue-linked evidence remain required before #16 can close.
