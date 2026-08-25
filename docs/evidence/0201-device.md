# Issue 0201 — private image OCR device evidence

This artifact records issue [#20](https://github.com/AlisinaDevelo/LOOM/issues/20) at
implementation commit [`ce09aee`](https://github.com/AlisinaDevelo/LOOM/commit/ce09aee8a3d0e1d26f6252f8880de1656ea4e9ba).
The OCR provider was exercised on the target Mac, not in hosted CI.
The implementation was merged through PR [#180](https://github.com/AlisinaDevelo/LOOM/pull/180).
The current-main focused rerun is recorded at the end of this artifact; the roadmap status is
advanced after that evidence is merged and reconciled.

## Reproduction environment

| Field | Recorded value |
| --- | --- |
| Hardware | MacBook Pro 17,1; Apple M1; 8 GB |
| Operating system | macOS 26.6.2 (25G83) |
| Architecture | arm64 / aarch64-apple-darwin |
| Rust | rustc 1.96.0 / cargo 1.96.0 |
| MSRV | rustc 1.88.0 / cargo 1.88.0 |
| JavaScript | Node v26.7.0 / npm 11.19.0 |
| Fixture | `tests/fixtures/ocr-golden.png`, SHA-256 `bbd4c82db14c56fc320cd2edd0b447d20889cfc0023d5f53fb0a724653f8f5c5` |

## Acceptance mapping

| ID | Acceptance criterion | Retained evidence |
| --- | --- | --- |
| `LOOM-0201-PROVIDER` | OCR runs locally and records provider/model version, confidence, normalized text, and pixel bounds | `native_vision_ocr_records_provider_metadata_and_pixel_evidence`; focused CLI log shows `loom.ocr` 0.1.0, provider `macos.vision`, model `VNRecognizeTextRequestRevision3`, two regions, confidence `1000`, and searchable `LOOM OCR marker` text. |
| `LOOM-0201-GEOMETRY` | Orientation, scale, and coordinate transforms are deterministic and bounded | `ocr::tests::oriented_dimensions_swap_for_rotated_exif_values`, `rotated_bounds_use_oriented_pixel_space`, `normalized_vision_bounds_become_top_left_pixels`, and `scale_is_fixed_point_and_rounds_without_float_drift`; the native fixture asserts every region is inside 1200×600 oriented pixel bounds with orientation 1 and scale 1000. |
| `LOOM-0201-POLICY` | Users can disable OCR and purge derived records | `disabling_ocr_purges_derived_records_and_reenable_recovers` and `explicit_purge_keeps_source_locator_but_removes_ocr_rows`; CLI `ocr-status`, `ocr-enable`, `ocr-disable`, and `ocr-purge`; Tauri commands and the desktop control are covered by the frontend test. |
| `LOOM-0201-FAILURE` | Malformed input fails closed and recovers | `malformed_image_fails_closed_then_recovers_without_stale_ocr` rejects malformed bytes, then indexes the same locator after replacement with the golden fixture. |

## Local pipe results

`scripts/verify-device.sh /tmp/loom-0201-device-final.FqO4Ht` completed with `status=PASS`.
The run executed `cargo fmt --check`, workspace clippy with warnings denied, the full workspace
test suite, Rust 1.88 check/tests, `npm ci`, frontend lint/tests/build, the retrieval benchmark,
the security check, the debug Tauri build, and the mixed bounded-failure corpus. The workspace log
contains four passing native image-OCR integration tests; the MSRV test log contains the same four
passes on the pinned toolchain.

The focused command reproduction used `/usr/bin/time -lp` around CLI index, inspect, and search.
It recorded 23,941 input bytes, two OCR passages, one source-backed image-region search hit, and
an OCR status of one derived version/two passages. The index command's peak resident set was
62,930,944 bytes; inspect/search peaks were 12,828,672 and 13,271,040 bytes respectively. The
focused log also retains the purge/recovery integration tests.

SHA-256 digests for retained logs:

```text
fmt.log  e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
clippy.log  8e025387486d43444be83ae88c7db991c7bcdc8950acfbbdae24a239d5fb9ddd
rust-workspace.log  411bfb85aef079a2c93a14963cadf8972e08a8bd6ed6f1917fdc5ae4805c9c4c
rust-msrv-check.log  dd149f2c57a633aa929c2780f2bfe8e4aac28e54e0485fc1192490a28ae75326
rust-msrv-tests.log  0a58189b17c72500c3b24d54bc76caf7daf85b770b453054bac5e4ff535e57a8
npm-check.log  be7c32d8b3c02615a0eb828683ff54aa86237001144769b69184ceca05d7d6a8
retrieval-benchmark.log  786001759c59bf5ad5d14b4158c2c1f974a097a18dea2a206e7221c7e47566e6
security-check.log  2b09dcf68f1355bada93d8c24fbb36e42227f7ef1246d56b986b9e0fb066eae6
tauri-build.log  e6e41ebd8f97d64a1457ca19ed6f61b2460d85bfd6bada5e2b41b76cf0c92ea0
mixed-corpus.log  19a4fe1726ef0187f66d154081747e2e12d990ec66e51aca1b47c2e8e1568213
image-ocr-focused.log  2b26c9e9e33c2de364a5ed12c855de3214a95ab0776edb26dd99dba1c5bd2084
```

## Limitations and decision

Vision is available only on macOS; non-macOS builds return an explicit unavailable error and the
native integration tests skip rather than claiming OCR evidence. The fixture covers a high-contrast
English PNG and unit-level rotations; handwriting, low contrast, tables, multilingual text, HEIC,
and image-only PDF rendering remain outside this issue. The database still contains derived OCR text
and is not encrypted at rest. No screenshot was used as acceptance evidence; any future UI capture
must be cropped to the relevant result/control.

The focused current-main rerun below is the code/device authority. The target repository currently
reports `main` unprotected; that governance limitation and the macOS-only provider boundary remain
explicit. Future desktop captures must be cropped to the relevant result/control panel.

## Merged-main reproduction

The same target-device harness was run against runtime-tested merged `main` commit
`eee1236710b98375e86b12187d545ed451ee2b7c` on the Mac specified above. The current main tip
`e5bcf782e0c5ea3efce27c7b3625fde50f6e25b9` adds only documentation and roadmap metadata after
that runtime-tested commit; no OCR source changed.

- Verification directory: `/tmp/loom-0110-main-device.QLkKl1`
- Harness summary SHA-256: `45bc997dcb26b8bc6cbd63a09fa17aed6c5d4ae968ef349be473b0b034e94e70`
- Commands SHA-256: `d840925fb008af9101dc3121870b79960c1b6924451df17a7851f2f6132bb209`
- Log manifest: `/tmp/loom-0110-main-device.QLkKl1/log-sha256.txt`
- Log manifest SHA-256: `ed539c48e8c9b648f0ae341ddf37f02107d5aacff5d5e2a19ae0d28612c57d64`

The full local pipe passed. Its workspace tests included native Vision OCR metadata/geometry,
malformed-input recovery, derived-record purge/re-enable, and source-backed image-region results;
the focused OCR contract also passed under Rust 1.88. No hosted Actions or unavailable hardware
substituted for this target-device evidence. The database-derived OCR and non-macOS limitations
remain explicit.

## Current merged-main focused rerun

The focused image-OCR suite was run against source-equivalent current `main` commit
`56203c442faeb6a72d52702c125db64e823fa6a9` (implementation source remains equivalent to
`87d1e03ffe2a43fed33826df58360e456ca4c753`) on the target Mac. The low-footprint log
`/tmp/loom-0200-03-focused-current.xxCDT1/focused.log` (SHA-256
`dc8a8d5d21c063ef17bb39bc927c1dc98f40a1ce5f89411fd23682fb68352f83`) records all 5 image-OCR
tests passed, including native Vision metadata/geometry, malformed recovery, and purge/re-enable.
The same current frontend check passed 23 tests, TypeScript, lint, and Vite build; no hosted Actions
result was used.

## Q03 roadmap reconciliation

Roadmap ID `0201` was marked done only after the focused rerun and was reconciled with the other
Q03 gates through merged PR [#249](https://github.com/AlisinaDevelo/LOOM/pull/249), commit
`c653ac45006cbd22201b1a16ec45e61e5fbb33e6`. The single-writer reconciler changed issues
`0200`–`0203` and then verified 154 active managed issues, 4 retired issues, 20 milestones, 13
phases, 141 parent edges, 314 dependency edges, and 31 closed issues. It reported no warnings and
the second plan was zero-delta.

```text
preflight plan: /tmp/loom-0200-03-roadmap-plan-before.json
  sha256: d60a99e4a37ec87d3031fe9ef4266459f3442d1d044a2e845f2f19850ab96da0
apply log: /tmp/loom-0200-03-roadmap-apply.log
  sha256: bd4d6ceb8e19293745ff0214044694cec35c67023cc507ebad459d790ad45c6a
after plan: /tmp/loom-0200-03-roadmap-plan-after.json
  sha256: de11779046e5cbcf4ea4e7bbe88d19c641676058a6cb88254f80357b752941f1
second plan: /tmp/loom-0200-03-roadmap-plan-second.json
  sha256: de11779046e5cbcf4ea4e7bbe88d19c641676058a6cb88254f80357b752941f1
```

The live issue for `0201` (#20) is closed and its title, milestone, labels, body marker, and
dependencies match the canonical manifest. No hosted Actions result was used.
