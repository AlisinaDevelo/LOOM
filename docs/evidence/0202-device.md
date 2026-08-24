# Issue 0202 — verified evidence viewer device evidence

This artifact records issue [#21](https://github.com/AlisinaDevelo/LOOM/issues/21) at
implementation commit [`b98862e`](https://github.com/AlisinaDevelo/LOOM/commit/b98862ebc36c0e5b3222c6415335da4b4084621a).
The viewer and native OCR provider were exercised on the target Mac, not in hosted CI.

## Reproduction environment

| Field | Recorded value |
| --- | --- |
| Hardware | MacBook Pro 17,1; Apple M1; 8 GB |
| Operating system | macOS 26.6.2 (25G83) |
| Architecture | arm64 / aarch64-apple-darwin |
| Rust | rustc 1.96.0 / cargo 1.96.0 |
| MSRV | rustc 1.88.0 / cargo 1.88.0 |
| JavaScript | Node v26.7.0 / npm 11.19.0 |
| Code SHA | `b98862ebc36c0e5b3222c6415335da4b4084621a` |

## Acceptance mapping

| ID | Acceptance criterion | Retained evidence |
| --- | --- | --- |
| `LOOM-0202-VERIFY` | A result resolves to its active source version and passage, or fails honestly when stale | `resolve_verified_evidence` checks artifact/version/hash/path, then requires the active passage row; `golden_pdf_records_page_anchors_warnings_and_verified_navigation` verifies a PDF page-3 passage and rejects the same tuple after source replacement with `ArtifactStale`. |
| `LOOM-0202-PDF` | PDF navigation keeps the expected page and visible evidence region | Rust returns the canonical `pdf_page` anchor, page count, extractor metadata, and passage text; the frontend test renders `PDF page 2`, `Page 2 of 3`, and the verified passage in the evidence panel. |
| `LOOM-0202-IMAGE` | Image OCR regions remain visible across zoom, rotation, and HiDPI projection | `projectImageRegion` tests cover 0/90-degree geometry, swapped canvas dimensions, zoom/device scale, and clamping; the frontend test renders the image-region map and rotates it from 0 to 90 degrees. |
| `LOOM-0202-FAILURE` | Missing, changed, or mismatched originals produce a recoverable error | Core stale tuple test and the frontend stale-state test show `Source needs attention`, retain the result card, and tell the user to re-index and retry; no unverified passage is displayed. |
| `LOOM-0202-CONTRACT` | The desktop command is explicitly scoped and local | Tauri command-contract test and capability file cover `resolve_evidence`; no filesystem, shell, network, or broad process permission was added. |

## Local pipe results

`./scripts/verify-device.sh /tmp/loom-0202-device-final.I9bmyS` completed with
`status=PASS`. The run executed formatting, warnings-as-errors clippy, the full current-toolchain
workspace tests, Rust 1.88 workspace check and core tests, `npm ci`, frontend lint/tests/build,
the retrieval benchmark, security check, debug Tauri build, and the mixed bounded-failure corpus.

The retained run reported 28 Rust unit tests plus all workspace integration suites passing on both
toolchains, 16 frontend tests passing, zero markdown/code lint issues, a successful debug Tauri
build, and a mixed corpus that kept an outside-root symlink unreachable while recovering after a
bounded oversized-file failure. The native Vision tests completed in the final run after the
provider was wrapped in an explicit Objective-C autorelease pool; this prevents repeated local OCR
calls from wedging the target-device test process.

SHA-256 digests for retained logs:

```text
fmt.log  e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
clippy.log  9c22cd2e4cb9956ecb0f228670febc5b8ae1370029b5a09604aa5efbe8a39cd6
rust-workspace.log  74997600ea9172c66c82045f5a4e452955a4cf16465f333dea6f237bb2f59856
rust-msrv-check.log  132d8f9813fd0518ff77c1a062a2e99512c1239c3e418eef63423374de32ac3b
rust-msrv-tests.log  73b210f1852e6ee2b87fad820f3684b56d2772ffc9bb1f9f9b6f7260150b3bc6
npm-install.log  bbacb689fb0c829145033c5a0e39bff2b1fcb458b993473ecadaf839f978c9c0
npm-check.log  1794807342886990f0e0f5b385e7981bd93405338305869bfd078c1842eff128
retrieval-benchmark.log  d5362261d67991c07ccb7cb196a2d80af2591b8914fe11791d412d9ffb5f39b3
security-check.log  b7c1b5f28d96864d5f8846a97b8a710b7032be5a4bd19761acfef5f8f59ed668
tauri-build.log  01f2d7c1200dce5528714e2bd254b24e8d93709725651285624c1b46b343a926
mixed-corpus.log  a8ef3b30789d2ba86a947c3fb9191a6aebb5ad19d16fcaa17c0fbe218ff867d6
```

## Limitations and decision

The evidence panel renders the exact canonical passage, PDF page label, and OCR-region geometry;
the original PDF/image remains the authority and is opened through the verified path. This slice
does not rasterize PDF pages or copy image bytes into the webview, so it does not claim a full
document renderer. OCR confidence, coordinates, extractor identity, and source hash remain visible
so a user can open the original when visual inspection is required. No screenshot was used as
acceptance evidence; any future UI capture must be cropped to the relevant result/viewer region.

Merge remains gated on independent review and protected `main`; the target repository currently
reports `main` unprotected, so no self-merge is claimed here.
