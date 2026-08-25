# LOOM 0107 device evidence

This artifact records the local CI, dependency, secret-scan, and release-hygiene verification
for issue [#16](https://github.com/AlisinaDevelo/LOOM/issues/16). The implementation is in
`3c0d2573f63b50971c4a2dc19f812c941107a171`; the issue is closed after that change and this
evidence was merged to `main`. GitHub Actions were not used as evidence during this validation
window.

## Device and toolchain

- Device: MacBook Pro 17,1, Apple M1, 8 GB
- OS: macOS 26.6.2 (25G83), `aarch64-apple-darwin` / arm64
- Native toolchain: `rustc 1.96.0` / Cargo 1.96.0, Node v26.7.0, npm 11.19.0
- Declared MSRV toolchain: `rustc 1.88.0` (`6b00bc3880198600130e1cf62b8f8a93494488cc`),
  Cargo 1.88.0
- Security tools: gitleaks 8.30.1; `cargo-audit` is not installed and is not claimed
- Runtime source under test: `3c0d2573f63b50971c4a2dc19f812c941107a171`

## Acceptance-criterion evidence map

| Artifact ID | Acceptance criterion | Retained evidence |
| --- | --- | --- |
| `LOOM-0107-CI-PARITY` | CI runs Rust fmt/clippy/tests and frontend lint/tests/build on supported runners | [`.github/workflows/ci.yml`](../../.github/workflows/ci.yml) contains roadmap, Rust, MSRV, advisory, frontend, and macOS Tauri jobs. The new [`scripts/test-ci-contract.py`](../../scripts/test-ci-contract.py) asserts those jobs and commands. Current focused Rust native/MSRV suites passed 93 tests each (`/tmp/loom-0102-focused-current.log`, `/tmp/loom-0102-msrv-current.log`); current frontend `npm run check` passed 23 tests (`/tmp/loom-0107-current-device.JPIIub/npm-check.log`). The full current pipe attempted Clippy and workspace tests, then hit the device `ENOSPC` boundary; it is retained as a no-go, not converted into a pass. |
| `LOOM-0107-DEPENDENCIES` | Dependabot covers Cargo, npm, and Actions; dependency review runs on pull requests | [`.github/dependabot.yml`](../../.github/dependabot.yml) declares weekly Cargo, npm, and GitHub Actions updates. [`.github/workflows/dependency-review.yml`](../../.github/workflows/dependency-review.yml) uses a pinned dependency-review action. Current `npm audit --audit-level=high` and locked `cargo metadata` both passed in the focused probe. |
| `LOOM-0107-SECRET-SCAN` | A secret scan passes before public push | [`scripts/security-check.sh`](../../scripts/security-check.sh) requires gitleaks and dependency checks. Current gitleaks 8.30.1 scanned the repository with `no leaks found`; npm audit reported zero vulnerabilities. |
| `LOOM-0107-RELEASE-HYGIENE` | Formatting and release hygiene are checked before publication | The focused probe passed `cargo fmt --all --check`, `git diff --check`, ShellCheck, actionlint, roadmap validation, the CI contract suite, and all 20 Python tests. `scripts/verify-device.sh` now records diff-check and CI-contract steps. No generated output is tracked. |

## Target-device reproduction

The checked-in harness is [`scripts/verify-device.sh`](../../scripts/verify-device.sh). The full
current-main run was intentionally attempted:

```text
bash scripts/verify-device.sh /tmp/loom-0107-full-current.SNOOaO
```

It passed formatting and diff-check, then failed Clippy while archiving `typenum` with
`No space left on device`; the workspace test subsequently failed while creating a temporary
directory for `tauri-utils`. With only 120 MiB free, later log redirections also hit the same
resource boundary. The runner therefore records an honest `ENOSPC` no-go rather than claiming a
full-pipe pass or substituting hosted CI.

The resource-boundary artifacts are retained:

```text
runner.log         sha256:c02e3e901d3391d7f155b54dba424801fafa9a98aaaf9d6bbde93b1aac030757
summary.txt        sha256:7a10c8cc528d3aca60b7ef08ec792ae68699a70cef0760dd2476b5aeca197a68
commands.txt       sha256:5d3cd261229957676693b25ca54f6a9a6717f0848247caa7ca572260b2f1cbad
clippy.log         sha256:71ff3acfade4cd530e6564bc9af3f329fcf606057b8061b198d596a1a7e49486
rust-workspace.log sha256:d21c3e10c0f1efeb6a7691d40373cddcbe460bd7794e7cc2ba7294ed8801d408
```

The resource-independent focused probe passed every check it could run without a large Rust
target: fmt, diff-check, the CI contract, all Python tests, roadmap validation, ShellCheck,
actionlint, gitleaks, npm audit, and cargo metadata. Its status file and log manifest are retained
under `/tmp/loom-0107-focused-device.ejNXO9` (manifest SHA-256
`925aada89ea7b28d3af8e28f6dafa7f0f9245727ee41f953dc5ce3f1f3e6937b`).

The focused Rust records used for the unchanged implementation are `/tmp/loom-0102-focused-current.log`
(`067ba921fbbceb562010d8bc1b6d05b75f445484579fbb11bf428259d2651435`) and
`/tmp/loom-0102-msrv-current.log`
(`7ad063c8e87ee22fe8026d31c866374f7780d279941c511e03e2bb454b000ec1`); each reports 93 passing
tests and zero failures.

The current frontend install/check also passed on this Mac:

```text
/tmp/loom-0107-current-device.JPIIub/npm-install.log  sha256:bbacb689fb0c829145033c5a0e39bff2b1fcb458b993473ecadaf839f978c9c0
/tmp/loom-0107-current-device.JPIIub/npm-check.log    sha256:595023f7c0d33da58b41aec70c14bbe27c938668fdfd7de9a3692d30c8188783
```

That check reported 23 frontend tests, TypeScript, and a successful Vite production build. The
Tauri debug build was separately rerun on the same current source family for 0106 and is retained
at `/tmp/loom-0106-tauri-current.log` (SHA-256
`cd111ad784787d6bce602e1580b6c5e4a5601379e5cf389c1df07726f58313f3`).

## Source and merged-main boundary

The Rust, Tauri, frontend, workflow, and dependency source paths at `3c0d257` are unchanged from
the runtime-tested `421bc6d469ba87a144495d0bf470d16ce44ec40f`; this change adds only the CI contract,
device-runner hygiene, and a corrected roadmap test. The focused current Rust logs therefore
exercise the same implementation, while the new static checks exercise the changed files directly.
The final merged `main` SHA is recorded by the merge PR and the subsequent roadmap reconciliation.

This evidence does not claim a cargo-audit result, a notarization result, another OS/architecture,
or a hosted Actions result. The repository currently has no protected-branch policy configured;
the merge was performed by the repository owner after the local checks above. Future full-pipe
runs should reclaim at least 1 GiB of device storage before compiling the workspace, and desktop
captures should remain cropped to the relevant evidence panel.
