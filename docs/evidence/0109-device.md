# LOOM 0109 device evidence

This artifact records approved-root observation hints, deterministic coalescing, content-hash
reconciliation, and restart recovery for issue [#18](https://github.com/AlisinaDevelo/LOOM/issues/18).
The change is stacked on durable-index PR [#171](https://github.com/AlisinaDevelo/LOOM/pull/171);
issue 18 remains open until independent review and a protected-main policy are available. The
current merged-main reproduction is recorded below.

## Device and toolchain

- Device: MacBook Pro 17,1, Apple M1, 8 GB
- OS: macOS 26.6.2 (25G83), `aarch64-apple-darwin` / arm64
- Native toolchain: `rustc 1.96.0` / Cargo 1.96.0, Node v26.7.0, npm 11.19.0
- Declared MSRV toolchain: `rustc 1.88.0` (`6b00bc3880198600130e1cf62b8f8a93494488cc`),
  Cargo 1.88.0
- Source under test: `738dfac97f4cbecc979079d7df12a0177d54d5a7`
  (`feature/issue-18-observation`)

## Acceptance-criterion evidence map

| Artifact ID | Acceptance criterion | Retained evidence |
| --- | --- | --- |
| `LOOM-0109-SCOPE-RESTART` | Observation scopes stay within approved roots and survive relaunch | `Library::reconcile_approved_roots` reads only enabled persisted `source_roots`; `reconcile_events` rejects a root that is not enabled and the coalescer rejects out-of-scope or symlink-escaping paths. The integration test drops and reopens the library, then reconciles the persisted root with no failures. The Tauri command is exposed with the single scoped `allow-reconcile-approved-roots` permission and invoked during desktop startup. |
| `LOOM-0109-COALESCE` | Debounced hints trigger safe content-hash reconciliation | `observe::coalesce_events` deterministically collapses duplicate create/modify/remove/rename hints, records both sides of a rename, normalizes `/var`/`/private` path aliases, and requests a full rescan for overflow or oversized batches. `Library::reconcile_events` always rescans the approved root, so watcher order is never canonical truth. |
| `LOOM-0109-RENAME-DELETE` | Rename and deletion reconcile without stale searchable evidence | The target-device integration fixture renames `old.md`, deletes `retained.md`, and sends the coalesced hints. The old marker disappears, the renamed marker remains source-backed, and no failure is reported. |
| `LOOM-0109-MISSED-EVENT` | Overflow/missed-event recovery converges through a full scan | The same fixture adds a new file and sends an `overflow` event; the full rescan makes its marker searchable. A restart then runs `reconcile_approved_roots` and reports one scanned root, zero failed roots, one full rescan, and no failures. |

## Target-device reproduction

The checked-in harness is [`scripts/verify-device.sh`](../../scripts/verify-device.sh):

```text
bash scripts/verify-device.sh /tmp/loom-0109-device-rerun.eMgOjj
```

The run completed with `status=PASS` and exit code 0 at source commit
`738dfac97f4cbecc979079d7df12a0177d54d5a7`. Format, clippy, the full Rust workspace, MSRV check
and tests, `npm ci`, `npm run check`, retrieval benchmark, local security check, Tauri debug build,
and mixed failure/recovery corpus all passed. The Rust workspace included 23 core unit tests, 3
durable/observation integration tests, fixture/result contracts, and CLI tests; the frontend check
included 2 files and 6 tests.

The retrieval benchmark indexed 3/3 synthetic fixtures with completeness 1.0, exact-source
Recall@1 1.0, Recall@5 1.0, anchor precision 1.0, false-positive rate 0.0, median latency
0.193458 ms, and p95 latency 0.425208 ms.

The mixed corpus contained supported Markdown, an unsupported binary, an 8,388,609-byte Markdown
file, and an outside-root symlink. Initial indexing reported `discovered=4`, `indexed=2`,
`skipped=1`, and one bounded-size failure; the outside marker remained unreachable. Replacing the
oversized file recovered it on the next index with `indexed=1`, `unchanged=2`, `skipped=1`, and no
failures. Final stats were 3 artifacts, 3 versions, 3 passages, and 250 indexed bytes.

## Log and digest record

The retained run directory is `/tmp/loom-0109-device-rerun.eMgOjj`. Its `summary.txt`, command
record, individual logs, and `log-sha256.txt` are the source evidence for this entry:

```text
clippy.log              sha256:dccad65a474a0e64d7c677fc96c87f30265067841005a793f4d82605fea6fe77
fmt.log                 sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
mixed-corpus.log        sha256:d90f5a3eb0adb51beaa875dac40e13136e47e9452c877874f2c1c58292be096f
npm-check.log           sha256:cdd7c05eb60b6a910728f71ad504379b5b459aac9d4647529b25919df0030a96
npm-install.log         sha256:bbacb689fb0c829145033c5a0e39bff2b1fcb458b993473ecadaf839f978c9c0
retrieval-benchmark.log sha256:f6543829037e5cdd872004364c0b564756e22b86c5cc980171cfb8ec5b8f7707
rust-msrv-check.log     sha256:fe7a55c0f416dda3213ffd6adbb45f4e3084400a09246cae860154f1a093722f
rust-msrv-tests.log     sha256:51db48f69706b320bf5eba25498c32caba96f5eccf369e75e804d41435e524fb
rust-workspace.log      sha256:48eab1540a0e6de36e82a1150135344dcdf3f066ef5f8f9da1f3a3dd82e233e1
security-check.log      sha256:8bbd30f4da5b54b078926531ae0c45850eb90689a9d12b701379101acf2764e8
tauri-build.log         sha256:883e22f7931142c09051cc0e0bb6e7f2d5482180606ed3b6bd6c303a63aaa76b
```

## Limitations and closure gate

This run proves the bounded persisted-root reconciliation boundary on the specified Mac; it does
not claim native FSEvents parity, a continuous watcher latency/resource benchmark, another
OS/architecture, notarization, a third-party security audit, or a cargo-audit result. The current
desktop path performs a startup reconciliation of persisted roots; a native event adapter can be
added later without changing the scope/content-hash contract. The post-merge reproduction below
satisfies the code and target-device evidence portion; independent review and a protected-main
policy remain required before issue 18 can close.

## Merged-main reproduction

The same target-device harness was rerun against runtime-tested merged `main` commit
`eee1236710b98375e86b12187d545ed451ee2b7c` on the Mac specified above. The final main tip
`8af236898ae17d898faa82d4acf351c322ac1898` adds only documentation and roadmap metadata after
that runtime-tested commit; no observation source changed.

- Verification directory: `/tmp/loom-0110-main-device.QLkKl1`
- Harness summary SHA-256: `45bc997dcb26b8bc6cbd63a09fa17aed6c5d4ae968ef349be473b0b034e94e70`
- Commands SHA-256: `d840925fb008af9101dc3121870b79960c1b6924451df17a7851f2f6132bb209`
- Log manifest: `/tmp/loom-0110-main-device.QLkKl1/log-sha256.txt`
- Log manifest SHA-256: `ed539c48e8c9b648f0ae341ddf37f02107d5aacff5d5e2a19ae0d28612c57d64`

The full local pipe passed: format, warnings-denied Clippy, workspace tests, Rust 1.88 MSRV
check/tests, `npm ci`, `npm run check`, retrieval benchmark, semantic contract, local security
scan, Tauri debug build, and mixed-corpus failure/recovery. Observation integration tests passed
approved-root scope/restart, deterministic coalescing, rename/delete reconciliation, overflow
full-rescan recovery, and persisted-root restart behavior. No hosted CI or unavailable hardware
substituted for this target-device evidence. Future desktop captures must be cropped to the
relevant evidence panel.
