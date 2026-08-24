# Issue 0203 device evidence: rebuildable semantic index contract

- Issue: [#22](https://github.com/AlisinaDevelo/LOOM/issues/22)
- Implementation commit: d46dd803506d0a681bee451765e2276f37065432
- Implementation PR: [#182](https://github.com/AlisinaDevelo/LOOM/pull/182)
- Roadmap status: `review`; independent approval and protected-main enforcement remain pending.
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
is implementation/device-verified locally and has been reproduced against merged main at
`ae102616700a0913f21af118609e727df9617e26`. Main is not protected (the branch-protection API
returned 404) and no independent GitHub approval was present; those governance limitations remain
explicit.

## Promotion-branch reproduction

The stacked feature branches were merged in order and reconciled with the then-current main
snapshot. The resulting local promotion commit was
b563a9b5606f6d7688200160bea6806b220ae121. The same complete device harness was rerun against that
exact source commit before publishing the promotion PR.

- Verification directory: /tmp/loom-0203-integration-device.OHFaWh
- Summary SHA-256: 255f0d3f96d72540748f00d65494790ab6c628572322dbf651818d5b94142e11
- Log manifest: /tmp/loom-0203-integration-device.OHFaWh/log-sha256.txt
- Semantic summary SHA-256: cf558b82cc559be99de64bf1dd1a2edcf3ba4ef07e6dcb0141df39028599dc46
- Semantic rebuild timing: 0.69 s; maximum resident set size 13,336,576 bytes
- Retrieval benchmark: Recall@1/5 1.0, anchor precision 1.0, false-positive rate 0.0,
  completeness 1.0, median latency 0.192042 ms, p95 latency 0.57425 ms

All device steps passed again, including native Vision, MSRV, npm, semantic drop/rebuild,
security, Tauri, and mixed-corpus recovery. This is a promotion-branch reproduction; the final
merged-main reproduction is recorded below.

## Final merged-main reproduction

The portability fix was merged to main as
`ae102616700a0913f21af118609e727df9617e26`. The complete device harness was rerun against that
exact source commit on the target Mac.

- Verification directory: /tmp/loom-0203-main-fix-device.27JXBI
- Summary SHA-256: 6435cdb602d4a728ac0546a99b597f1f7e883bd49b2f25554a7ad3d34b24fbfc
- Commands SHA-256: a9625ca3f983fa7552d0ced9ac0b49ad0569b3b5a9c9e3c09bbbfa54ce727a8d
- Semantic summary SHA-256: b5a4a7631831fcb22819951733d143d0c675c5bb7d94fe1ee4bad81f73e70514
- Log manifest: /tmp/loom-0203-main-fix-device.27JXBI/log-sha256.txt
- Semantic rebuild: 0.66 s end-to-end; maximum resident set size 13,336,576 bytes
- Retrieval benchmark: Recall@1/5 1.0, anchor precision 1.0, false-positive rate 0.0,
  completeness 1.0, median latency 0.193875 ms, p95 latency 0.543208 ms

The full local pipe passed on this merged-main SHA, including rustfmt, warnings-denied Clippy,
workspace tests, Rust 1.88 MSRV checks/tests, npm checks, retrieval, semantic drop/rebuild,
security, Tauri debug build, and mixed-corpus recovery. No screenshot was needed; any future
desktop capture must be cropped to the relevant result/evidence panel.

## Current main reconciliation

The same target-device pipe is represented on the current main tip
`e5bcf782e0c5ea3efce27c7b3625fde50f6e25b9`. The runtime-tested source commit remains
`eee1236710b98375e86b12187d545ed451ee2b7c`; current main adds only documentation and roadmap
metadata after that run, so no semantic implementation changed. The retained run directory is
`/tmp/loom-0110-main-device.QLkKl1`, with summary SHA-256
`45bc997dcb26b8bc6cbd63a09fa17aed6c5d4ae968ef349be473b0b034e94e70`, commands SHA-256
`d840925fb008af9101dc3121870b79960c1b6924451df17a7851f2f6132bb209`, and log manifest SHA-256
`ed539c48e8c9b648f0ae341ddf37f02107d5aacff5d5e2a19ae0d28612c57d64`. The local pipe passed the
semantic drop/rebuild and fail-closed provider tests alongside the full retrieval, security,
frontend, Tauri, and mixed-corpus checks. No hosted Actions or unavailable hardware substituted;
future desktop captures must be cropped to the relevant evidence panel.
