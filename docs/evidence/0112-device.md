# LOOM 0112 device evidence

This artifact records the persisted, revocable source-scope contract for issue
[#65](https://github.com/AlisinaDevelo/LOOM/issues/65). It covers exact user-selected locators,
bounded status reporting for missing/denied/moved/unsafe roots, read-only reconciliation, and
explicit revocation/re-selection. The current direct-distribution build uses explicit
re-selection; it does not claim a sandbox security-scoped bookmark implementation. The
implementation was merged through PR [#177](https://github.com/AlisinaDevelo/LOOM/pull/177). The
current-main focused reproduction is recorded below; the roadmap status is advanced after that
evidence is merged and reconciled. This direct-distribution build uses explicit re-selection
rather than a sandbox security-scoped bookmark.

## Device and toolchain

- Device: MacBook Pro 17,1, Apple M1, 8 GB
- OS: macOS 26.6.2 (25G83), `aarch64-apple-darwin` / arm64
- Native toolchain: `rustc 1.96.0` / Cargo 1.96.0, Node v26.7.0, npm 11.19.0
- Declared MSRV toolchain: `rustc 1.88.0` (`6b00bc3880198600130e1cf62b8f8a93494488cc`),
  Cargo 1.88.0
- Source under test: `e042a48b4a34d1d255b5ea6d05909cd3c61a15e1`
  (`feature/issue-65-source-root-access`)

## Acceptance-criterion evidence map

|Artifact ID|Acceptance criterion|Retained evidence|
|---|---|---|
|`LOOM-0112-PERSIST`|A selected file or directory reopens after relaunch through a persisted scope, with no arbitrary-home fallback|`persisted_root_reopens_and_reconciles_only_the_selected_locator` indexes one canonical directory, drops and reopens `Library`, reconciles the persisted locator, and finds a marker added after the simulated relaunch. `approved_root_specs` reads only persisted locators and kinds.|
|`LOOM-0112-STATUS`|Denied, stale/moved, wrong-type, unsafe, revoked, and unavailable outcomes are bounded and visible|`SourceRootStatus` is serialized as explicit statuses. `source_root_status` rejects symlink replacement as `Unsafe`, reports permission failure as `Denied`, missing paths as `Missing`, wrong file/directory shape as `WrongType`, disabled rows as `Revoked`, and other filesystem failures as `Unavailable`. The focused suite proves missing, denied, and unsafe cases; the reconciliation report records a named failure and never widens scope.|
|`LOOM-0112-REVOKE`|Revocation removes future indexing access while retaining an explicit re-selection path|`revocation_hides_existing_evidence_until_explicit_reselection` verifies the transaction disables the exact root, marks its active artifacts missing, hides prior evidence from search, scans zero roots afterward, and restores access only when the user explicitly passes the path to `index_path`. `moved_root_reports_missing_and_requires_explicit_reselection` proves the same behavior after a rename.|
|`LOOM-0112-READONLY`|Stored scope access is read-only and source bytes remain unchanged|`SourceRootInfo.read_only` is always `true`; reconciliation opens selected files/directories for inspection only. The relaunch fixture snapshots the original bytes and asserts they are identical after indexing and reconciliation. Documentation calls out the read-only boundary.|
|`LOOM-0112-DESKTOP`|Desktop controls expose saved scopes, status, re-selection, and revoke actions|Tauri commands `list_source_roots` and `revoke_source_root` are registered, permissioned, and built. The frontend renders saved-scope status and bounded explanations, invokes re-selection only with an explicit user path, and exposes revoke for enabled scopes. `src/App.test.tsx` covers the visible status/revoke behavior.|

## Target-device reproduction

The checked-in harness is [`scripts/verify-device.sh`](../../scripts/verify-device.sh):

```text
bash scripts/verify-device.sh /tmp/loom-0112-device-final-rerun.1kinz9
```

The run completed with `status=PASS` and exit code 0 at source commit
`e042a48b4a34d1d255b5ea6d05909cd3c61a15e1`. Format, clippy, the full Rust workspace, Rust
1.88 MSRV check and tests, `npm ci`, frontend lint/tests/build, retrieval benchmark, local
security check, Tauri debug build, and the mixed failure/recovery corpus all passed. The focused
source-scope suite ran 5 tests, the frontend check ran 2 files and 7 tests, and the separate
Python contract suite ran 19 tests with no failures.

The rights-clean retrieval benchmark indexed 3/3 fixtures with completeness 1.0, exact-source
Recall@1 1.0, Recall@5 1.0, anchor precision 1.0, false-positive rate 0.0, median latency
0.194959 ms, and p95 latency 0.4195 ms.

The mixed corpus contained supported Markdown, an unsupported binary, an 8,388,609-byte
Markdown file, and an outside-root symlink. Initial indexing reported `discovered=4`, `indexed=2`,
`skipped=1`, and one bounded-size failure; the outside marker remained unreachable. Replacing the
oversized file recovered it on the next index with `indexed=1`, `unchanged=2`, `skipped=1`, and no
failures. Final stats were 3 artifacts, 3 versions, 3 passages, and 250 indexed bytes.

The local security check found no gitleaks secrets and `npm audit` reported zero vulnerabilities.
This is not a third-party audit or a complete dependency advisory statement; the repository still
has a GitHub Dependabot advisory for `glib`, and `cargo-audit` is not installed on this device.
No GitHub-hosted Actions result was used.

## Log and digest record

The retained run directory is `/tmp/loom-0112-device-final-rerun.1kinz9`. Its `summary.txt`,
command record, individual logs, Python contract output, and `log-sha256.txt` are the source
evidence for this entry:

```text
clippy.log              sha256:80d053c033d9e6e1b5b57c733187c2d7b2ad83b3580ddca18f2f17f126025ff0
fmt.log                 sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
mixed-corpus.log        sha256:137472edaa221c30ceba945c899dbe2c1d77956552b247f41a925906137517f2
npm-check.log           sha256:481552c4079eefa3c8f0ef492b9eedd59fd186a9e7dcb8c9f8a8f3e4044718ae
npm-install.log         sha256:85b4814e52ba71ae231930f0e73b027fadd1667b8aede7ff5c562e1a0a4e22fe
python-tests.log        sha256:95f02de7def0e9edf6e728afd22f6a4bab5d20d31693170c910fe0e34fc05f16
retrieval-benchmark.log sha256:4c786e2b7edbb2cbfc62559052b574ff52eac726bd4552c0ea62022be4e25526
rust-msrv-check.log     sha256:03117ef19c38c8e7de3771fc1fa8b7fa5039c950f3a66dd5baf503e66a267358
rust-msrv-tests.log     sha256:250b27a2e40f53874c0f04aeccbb65ca8c36b0d1975601c7f77617516a868578
rust-workspace.log      sha256:8daa3c8174cfbdca88fb24940566c75d3ee3be8b8b82a00d311cb4cc4d2d2ec0
security-check.log      sha256:56fc057adb3fe36ff235ee7b34a6737ca96b8e8a5fd731d5efbcd635990cdd0a
tauri-build.log         sha256:c42bf746b6523fbc1e85e412d51f91982b89f9262f1ea76b40275b9f9f7c3a30
```

## Limitations and closure gate

This run proves the persisted-scope contract on the specified Mac; it does not claim another
OS/architecture, a sandbox security-scoped bookmark, notarization, a third-party security audit,
a `cargo-audit` result, or a large-library resource benchmark. The current implementation keeps
historical rows when a scope is revoked; retention/purge policy remains separate work. A default
profile clean rebuild attempted on current `main` exhausted the device filesystem before the
test binary was produced (`/tmp/loom-0112-focused-current.mjj5wg/source-roots.log`, SHA-256
`8232052f286f7da873fb5420033ab6b0ec238364b9d0f3f54b5150dcc89b2d03`). The successful focused
rerun below used `CARGO_PROFILE_DEV_DEBUG=0`, `CARGO_INCREMENTAL=0`, and one build job; it is
the current code/device authority for the source-scope criterion. No hosted Actions result or
protected-branch enforcement was used.

## Current merged-main focused reproduction

The focused core suites were run against merged `main` commit
`87d1e03ffe2a43fed33826df58360e456ca4c753` on the Mac specified above. The source-scope suite
passed all five tests: persisted relaunch, revocation, moved-root re-selection, denied scope,
and symlink replacement. The same low-footprint run also passed the two cancellation tests and
the two FTS5 repair tests used by adjacent v0.1 gates.

- Verification directory: `/tmp/loom-0112-14-focused-current.WFJUFQ`
- Focused log SHA-256: `99827fb413c87051a3a2085c134e05cff0ab44d858b5cf30bdfc1789819d32e2`
- Toolchain: `rustc 1.96.0` / Cargo `1.96.0`, arm64 Apple Silicon
- Result: source roots `5 passed`, cancellation `2 passed`, FTS repair `2 passed`

This focused rerun does not substitute for the unavailable full workspace pipe; the historical
full-pipe artifact above remains labeled with its original source commit. The direct-distribution
re-selection limit and the requirement to crop future desktop captures to the evidence panel
remain explicit.

## Historical merged-main full-pipe reproduction

The earlier target-device harness passed against runtime-tested merged `main` commit
`eee1236710b98375e86b12187d545ed451ee2b7c`; its retained logs and limitations remain useful for
the broader integration claim, but it predates the current documentation-only commits and is not
used to represent the focused current-main rerun above.
