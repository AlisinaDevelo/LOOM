# LOOM 0205 device evidence

This artifact records the typed query parser and pre-ranking filter contract for
[issue #24](https://github.com/AlisinaDevelo/LOOM/issues/24), roadmap ID 0205.
It is implementation evidence for the v0.2 retrieval path; it does not claim that
hybrid ranking has passed the separate 0204 quality gate.

## Target and toolchain

|Field|Value|
|---|---|
|Hardware|MacBook Pro 17,1; Apple M1; 8 GB|
|OS|macOS 26.6.2 (25G83)|
|Architecture|arm64 / aarch64-apple-darwin|
|Rust|1.96.0; MSRV check and tests with Rust 1.88.0|
|Node/npm|Node v26.7.0; npm 11.19.0|
|Source SHA|6d2fced9e66e5fd8f11f2a4ae01b9931ad41f504|

The exact target-device run is /tmp/loom-0205-device-final.qWZOnb.
Its summary SHA-256 is
959340e4e3279f1cafebc20e5b9065526359900a69b60b04be87aa80f862e428,
commands SHA-256 is
9300a471a1bea43a0b9700e1e755a601ff10e6cc50b9c796b5b51b3f3539571f, and
the log manifest SHA-256 is
44aa8b37fbb4810e1dca21131504d904e466002040889b5d9ba6a701cfa080e7.
The harness summary records status=PASS and the exact source SHA above.

## Acceptance mapping

|Acceptance criterion|Evidence|
|---|---|
|Typed parser, documented filters, helpful errors|crates/loom-core/src/search.rs, docs/QUERY.md, and ADR 0007 implement and document after, before, type, path, and confidence, exact quoted phrases, RFC3339/date parsing, duplicate-filter checks, bounded confidence, Unicode, escaping, and injection-shaped input rejection.|
|Contributions and pre-ranking stable pagination|SearchHit and HybridSearchHit retain lexical, semantic, metadata, and reranker contribution values. Library::search filters canonical media type, locator, source modification time, and anchor before deterministic ordering and limit truncation. Repeated filtered searches return the same passage ID.|
|Property-style escaping/Unicode/adversarial and typed combinations|The parser test runs 128 deterministic Unicode/path combinations plus escaped quotes, FTS-looking operators, malformed dates, invalid source types, invalid confidence, and filter-only rejection. query_filters.rs covers time/type/path/confidence combinations and contribution values.|
|Secondary ranking cannot reintroduce exclusions|filtered_semantic_candidates_cannot_reenter_hybrid_results indexes identical Markdown/TXT evidence, rebuilds the local semantic derivative, and proves type:markdown hybrid results contain only the Markdown artifact. Semantic filtering happens before candidate ranking/fusion.|

## Full local pipe

Every step in scripts/verify-device.sh passed on the target Mac:

- rustfmt and warnings-denied workspace Clippy;
- workspace tests, Rust 1.88 workspace check, and Rust 1.88 core tests;
- npm ci and the full frontend check (lint, Vitest, typecheck, build);
- rights-clean retrieval benchmark;
- semantic derivative contract and drop/rebuild recovery;
- local security check;
- Tauri debug build without bundling;
- mixed-corpus bounded-failure recovery and outside-root isolation.

No source content or user data left the device. No screenshot was needed as
acceptance evidence. If a future desktop capture is added, crop it to the
relevant result/evidence panel only and omit unrelated windows or raw source
content.

## Boundaries

The filter parser deliberately rejects filter-only queries to avoid accidental
full-library scans. Path matching is a case-insensitive locator substring, not a
glob language. Confidence is an evidence-anchor contract: image OCR reports its
stored confidence, while text/PDF anchors are treated as deterministic 1.0.
The hybrid result path remains experimental and subject to the independent 0204
false-positive gate.
