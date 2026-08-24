# LOOM 0106 device evidence

This artifact records the Tauri desktop truth-path and accessibility smoke verification for issue
[#15](https://github.com/AlisinaDevelo/LOOM/issues/15). The change is stacked on benchmark PR
[#167](https://github.com/AlisinaDevelo/LOOM/pull/167); issue #15 stays open until review, merge,
and a repeat against the merged `main` SHA.

## Device and toolchain

- Device: MacBook Pro 17,1, Apple M1, 8 GB
- OS: macOS 26.6.2 (25G83), `aarch64-apple-darwin` / arm64
- Native toolchain: `rustc 1.96.0` / Cargo 1.96.0, Node v26.7.0, npm 11.19.0
- Declared MSRV toolchain: `rustc 1.88.0` (`6b00bc3880198600130e1cf62b8f8a93494488cc`),
  Cargo 1.88.0
- Source under test: `a0d1d3b28f6d916f603a857b628f7d5f23791098`
  (`feature/issue-15-desktop-truth`)

## Acceptance-criterion evidence map

| Artifact ID | Acceptance criterion | Retained evidence |
| --- | --- | --- |
| `LOOM-0106-UI-STATES` | UI exposes empty, loading, result, no-result, and partial-failure states | `src/App.test.tsx` covers the initial empty state, pending-search loading state and disabled duplicate search, source-backed result/open state, no-evidence state, and partial indexing failure with usable stats retained. |
| `LOOM-0106-IPC-CONTRACT` | Tauri commands remain source-backed and scoped | The desktop test asserts `open_artifact` receives exactly the result's artifact/version/hash tuple. `src-tauri/src/lib.rs::tests::desktop_contract_stays_local_and_command_scoped` checks the four intended commands and rejects filesystem, shell, HTTP, process, and notification permission namespaces. |
| `LOOM-0106-CSP` | Capabilities and CSP expose no remote content | The Tauri contract test checks `connect-src` is limited to `ipc: http://ipc.localhost`, the frontend distribution is local, and the capability file contains no broad remote or host-process permissions. The Tauri debug build completed on the target Mac. |
| `LOOM-0106-A11Y` | Keyboard and screen-reader smoke path is navigable | React Testing Library queries the semantic search role, accessible textbox label, status/alert live regions, headings, and named buttons; the primary flow submits the search form through an Enter-key event and verifies result/open actions. This is an automated accessibility-tree smoke test, not a claim of VoiceOver certification. |
| `LOOM-0106-FAILURE-RECOVERY` | UI remains safe under indexing and source failures | The full device harness exercises unsupported/binary input, oversized input, outside-root symlink containment, replacement recovery, stale source opening, no-network operation, and the UI's partial-failure report. |

## Target-device reproduction

The checked-in harness is [scripts/verify-device.sh](../../scripts/verify-device.sh):

```text
bash scripts/verify-device.sh /tmp/loom-0106-device-final.rwx5S7
```

The run completed with `status=PASS` and exit code 0 at source commit
`a0d1d3b28f6d916f603a857b628f7d5f23791098`. Format, clippy, the Rust workspace, MSRV check and
tests, `npm ci`, `npm run check`, the retrieval benchmark, the Tauri debug build, and the mixed
corpus all passed. Frontend tests reported 2 files and 6 tests passed. The Rust workspace
reported the desktop capability test, 7 CLI unit tests, 19 core unit tests, 1 fixture-contract
integration test, 1 result-contract integration test, and 5 CLI tests, with no failures. The MSRV
test run passed the 19 core unit tests and both integration tests.

The benchmark indexed 3/3 synthetic fixtures with completeness 1.0, exact-source Recall@1/5 1.0,
anchor precision 1.0, false-positive rate 0.0, median latency 0.224084 ms, and p95 latency
0.421459 ms.

The mixed corpus contained supported Markdown, an unsupported binary, an 8,388,609-byte Markdown
file, and an outside-root symlink. Initial indexing reported `discovered=4`, `indexed=2`,
`skipped=1`, and one bounded-size failure; the outside marker was not searchable. Replacing the
oversized file recovered it on the next index with `indexed=1`, `unchanged=2`, `skipped=1`, and no
failures. Final stats were 3 artifacts, 3 versions, 3 passages, and 250 indexed bytes.

## Log and digest record

The full log manifest is `/tmp/loom-0106-device-final.rwx5S7/log-sha256.txt`:

```text
clippy.log              sha256:ff94209bb8ee43a3f2c9e2de9c4df61a4441cb38ee979f66ab028d791bd05ee1
fmt.log                 sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
mixed-corpus.log        sha256:08ae444f32fd0bd4d3c11336c7234a153aad50f371a01572f911211536528f56
npm-check.log           sha256:123c9fcf943e2c09fad49c7b99fb0afbd2d7a634ec7eb66057321f32ec96736a
npm-install.log         sha256:bbacb689fb0c829145033c5a0e39bff2b1fcb458b993473ecadaf839f978c9c0
retrieval-benchmark.log sha256:78fb8c1cec8587d78679be1755aa0b4291bfbd882adc20b877d370dc3ee9d010
rust-msrv-check.log     sha256:57e0142510f7938e3e455e86a3b4015a89963d74d64a7b78b29e8b2b112ba57a
rust-msrv-tests.log     sha256:d9c3c5dcafd175e192bf4126a37aee43d9a5b136111a62422deeff8faf8ae7cf
rust-workspace.log      sha256:fad151cc00f4319386e1ca1384ddb16fc7db5ea52e49baf76b3b674c0c024dfc
tauri-build.log         sha256:5e1c2bc3aea0342443db13f02ac810787444e7d64a83ca66e93ae45c5216a873
```

## Limitations and closure gate

The automated accessibility smoke covers the DOM accessibility tree and keyboard submission; a
manual VoiceOver pass and signing/notarization are not claimed by this run. PDF/OCR, browser
capture, passive capture, and unavailable hardware were not substituted or claimed. Review,
protected-main merge, the same reproduction against the merged SHA, and final issue-linked
evidence remain required before #15 can close.
