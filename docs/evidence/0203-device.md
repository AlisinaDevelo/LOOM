# Issue 0203 device evidence: rebuildable semantic index contract

- Issue: [#22](https://github.com/AlisinaDevelo/LOOM/issues/22)
- Implementation commit: d46dd803506d0a681bee451765e2276f37065432
- Verification run: /tmp/loom-0203-device-final.mpz6X2
- Verification status: PASS
- Verification date: 2026-08-24

## Target and toolchain

The complete pipe ran on the target device, not in hosted CI:

|Field|Value|
|---|---|
|Hardware|MacBook Pro 17,1; Apple M1; 8 GB|
|OS|macOS 26.6.2 (25G83)|
|Architecture|arm64 / aarch64-apple-darwin|
|Rust|1.96.0; MSRV check and tests with Rust 1.88.0|
|Node/npm|Node v26.7.0; npm 11.19.0|
|Source SHA|d46dd803506d0a681bee451765e2276f37065432|

The retained device summary is
facda9ff34338e7c117d2e4a8de50781be338485e842a51ad53ab6d94657eab0.

## Acceptance mapping

|Acceptance criterion|Evidence and retained artifact|
|---|---|
|Compare local provider options with corpus size, latency, binary size, and license constraints|ADR [0005](../adr/0005-semantic-index-provider.md) records the measured 3-passage/533-byte corpus, 18,853,416-byte debug binary, three provider timings, vector footprints, and MPL-2.0/no-model-download constraints. Raw benchmark JSON SHA-256: cd9a882f3829d120bd1fa6e5a6f5b07665f357f1de1c2a1602fc5a701348eac5.|
|Bind each vector to passage hash, model identity, dimension, normalization, and index revision|semantic_rebuild_is_evidence_bound_and_repeatable and tampered_passage_binding_fails_closed_without_changing_canonical_rows verify the stored binding. The first manifest JSON SHA-256 is 121f4a14c1d8ea6af20924f3b09c015bfc09929f85b035b7714cba963f2104ca; it records loom.hash-embedding, hashed-tokens-v1, dimension 128, L2 normalization, and semantic-v1.|
|Delete/rebuild produces equivalent top-k without changing canonical passages|The semantic contract script drops the derivative, verifies unavailable status, rebuilds, and compares passage ID, score bits, and manifest. Summary JSON SHA-256: 4c4be3850706d0d3b6ad1652d3f45f02c09cf9fda2efd6e4e3ef91d0ffeda560; first and repeated search JSON SHA-256: cdf2004f74ad105496d60493493ff2556e56a62f9453bed299497fd28cf265bc.|

## Negative, privacy, and recovery checks

- Before build and after drop, semantic status was unhealthy and search refused to run; status JSON
  SHA-256 was e994191bc6518e48792c287088e62b70212dbe49305b5f3f23e24350d1e5b8e4.
- A changed canonical source produced a digest-stale state and recovered after rebuild.
- A foreign provider manifest and a tampered passage hash both failed closed; canonical stats stayed
  unchanged. These cases are in the semantic integration-test section of rust-workspace.log.
- The provider is deterministic, local-only, and model-download-free. security-check.log reports no
  secret leaks and npm reports 0 vulnerabilities. No source files or user data were uploaded.
- The full mixed corpus rejected an outside-root symlink, reported bounded oversized/unsupported
  outcomes, and recovered the changed oversized file on the next index.

## Full device pipe

Every step in scripts/verify-device.sh passed:

- rustfmt, workspace Clippy with warnings denied, workspace tests, Rust 1.88 workspace check and
  core tests;
- npm ci, the complete npm check (lint, tests, typecheck, build);
- exact-source retrieval benchmark: Recall@1/5 1.0, anchor precision 1.0, false-positive rate 0.0,
  index completeness 1.0, median latency 0.192542 ms, p95 latency 0.548875 ms;
- semantic contract benchmark and drop/rebuild recovery;
- security scan and Tauri debug build;
- mixed corpus failure/recovery and outside-root isolation.

The timed semantic rebuild took 0.95 s end-to-end with a 13,336,576-byte maximum resident set size.
Its retained resource file SHA-256 is
e0e87193f0eaaa01f327264d52c95cd1131b64c099a1c0f01ace0965c3fabe9e.
The full log hash list is in /tmp/loom-0203-device-final.mpz6X2/log-sha256.txt.

No screenshot was needed as acceptance evidence for this contract. If a future handoff captures
the desktop, crop it to the relevant result/evidence panel and omit unrelated windows or source
content.

## Limitations and next gate

The hash provider is a deterministic contract baseline, not a semantic-quality claim. The current
linear scan is appropriate only for the bounded MVP corpus. A neural model, approximate index,
browser capture, or sync layer requires a separate measured decision and privacy review. This issue
is implementation/device-verified locally; it is not independently reviewed or merged to protected
main yet.
