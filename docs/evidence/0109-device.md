# LOOM 0109 device evidence

This artifact records approved-root observation hints, deterministic coalescing, content-hash
reconciliation, and restart recovery for issue [#18](https://github.com/AlisinaDevelo/LOOM/issues/18).
The observation implementation is present on current `main`; this evidence closes the roadmap item
after the evidence/status merge. Hosted Actions are not used as evidence here.

## Device and toolchain

- Device: MacBook Pro 17,1, Apple M1, 8 GB
- OS: macOS 26.6.2 (25G83), `aarch64-apple-darwin` / arm64
- Native toolchain: `rustc 1.96.0` / Cargo 1.96.0; declared MSRV `rustc 1.88.0`
- Runtime-tested implementation baseline: `421bc6d469ba87a144495d0bf470d16ce44ec40f`
- Current main at evidence preparation: `7648eca51c7832c0ae0fe297b8882d8700f8cfcb`

## Acceptance-criterion evidence map

| Artifact ID | Acceptance criterion | Retained evidence |
| --- | --- | --- |
| `LOOM-0109-SCOPE-RESTART` | Observation scopes stay within approved roots and survive relaunch | `Library::reconcile_approved_roots` reads only enabled persisted `source_roots`; `reconcile_events` rejects a root that is not enabled and the coalescer rejects out-of-scope or symlink-escaping paths. The integration test drops and reopens the library, then reconciles the persisted root with no failures. The Tauri command is exposed with the single scoped `allow-reconcile-approved-roots` permission and invoked during desktop startup. |
| `LOOM-0109-COALESCE` | Debounced hints trigger safe content-hash reconciliation | `observe::coalesce_events` deterministically collapses duplicate create/modify/remove/rename hints, records both sides of a rename, normalizes `/var`/`/private` path aliases, and requests a full rescan for overflow or oversized batches. `Library::reconcile_events` always rescans the approved root, so watcher order is never canonical truth. |
| `LOOM-0109-RENAME-DELETE` | Rename and deletion reconcile without stale searchable evidence | The target-device integration fixture renames `old.md`, deletes `retained.md`, and sends the coalesced hints. The old marker disappears, the renamed marker remains source-backed, and no failure is reported. |
| `LOOM-0109-MISSED-EVENT` | Overflow/missed-event recovery converges through a full scan | The same fixture adds a new file and sends an `overflow` event; the full rescan makes its marker searchable. A restart then runs `reconcile_approved_roots` and reports one scanned root, zero failed roots, one full rescan, and no failures. |

## Current target-device reproduction

The focused current-main Rust run is retained at `/tmp/loom-0102-focused-current.log`
(`sha256:067ba921fbbceb562010d8bc1b6d05b75f445484579fbb11bf428259d2651435`) and the MSRV run at
`/tmp/loom-0102-msrv-current.log`
(`sha256:7ad063c8e87ee22fe8026d31c866374f7780d279941c511e03e2bb454b000ec1`). Each reports 93
passing tests and zero failures. The native run includes the durable/observation integration suite
and source-root persistence tests; the MSRV run repeats the same observation and restart cases.

The source-equivalence record `/tmp/loom-0108-source-equivalence.log` (SHA-256
`08923a150b189a5bf4dbf76167c88feeb421716da87e2978d8599732c5093860`) proves the observation paths
are unchanged between runtime baseline `421bc6d` and current `main`, with formatting passing.

The full workspace pipe was attempted on this device during the adjacent 0107 validation and hit
`ENOSPC` while compiling Tauri dependencies; it is retained at `/tmp/loom-0107-full-current.SNOOaO`
as a resource no-go, not a hidden pass. No hosted CI or unavailable hardware substitutes for the
current observation evidence.

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

## Limitations and closure boundary

This run proves the bounded persisted-root reconciliation boundary on the specified Mac; it does
not claim native FSEvents parity, a continuous watcher latency/resource benchmark, another
OS/architecture, notarization, a third-party security audit, or a `cargo-audit` result. The current
desktop path performs startup reconciliation of persisted roots; a native event adapter can be
added later without changing the scope/content-hash contract. The repository currently has no
protected-branch policy configured; the issue was closed after the owner-reviewed merge and roadmap
reconciliation. Future full-pipe runs should reclaim at least 1 GiB before compiling the workspace,
and desktop captures must remain cropped to the relevant evidence panel.

The implementation/evidence merge is PR [#242](https://github.com/AlisinaDevelo/LOOM/pull/242),
merge commit `cd4c7887dc8d60605fd016ce183bb90b07aa2bc1`. The roadmap write closed issue [#18](https://github.com/AlisinaDevelo/LOOM/issues/18)
at `2026-08-25T04:08:58Z`.

The v2 reconciler preflight planned one `update_issue` for `0109` and no label, milestone, parent,
or dependency changes. Its retained fingerprints are:

```text
/tmp/loom-0109-roadmap-plan-before.json  sha256:6186f06330f723f1e1c627a8b803b0c7b35dd10f62f98c6c810c7a2f74a5f3ec
/tmp/loom-0109-roadmap-apply.log         sha256:b569e14aa78d50274bafa4b89a09d4d29efbfeb4f0ed0b40ace2bc86a30cf27b
/tmp/loom-0109-roadmap-plan-after.json   sha256:de11779046e5cbcf4ea4e7bbe88d19c641676058a6cb88254f80357b752941f1
/tmp/loom-0109-roadmap-plan-second.json  sha256:de11779046e5cbcf4ea4e7bbe88d19c641676058a6cb88254f80357b752941f1
```

Post-apply verification reports 154 active issues, 4 retired issues, 20 milestones, 13 phases,
141 parent edges, 314 dependency edges, 23 closed issues, and `mutation_count: 0`.
