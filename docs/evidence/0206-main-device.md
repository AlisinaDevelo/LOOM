# 0206 merged-main reproduction

This is the post-merge reproduction for issue 0206. The tested source is exactly
`12b825692fc7d50aa12ec32e60ea9d50a29b96f2` (PR #196 merge commit), checked out on local
`main` before the run.

## Target and run

- Device: Apple Silicon Mac (`arm64`), macOS 26.6.2 (build 25G83).
- Toolchains: Rust 1.96.0 / Cargo 1.96.0; MSRV Rust 1.88.0 / Cargo 1.88.0; Node v26.7.0;
  npm 11.19.0.
- Resource guard: `CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0`.
  These settings remove debug symbols and incremental cache only; they do not change runtime
  retrieval behavior. The runner also clears only its ignored LOOM `target/` between Rust and
  JS/Tauri phases.
- Evidence directory: `/tmp/loom-0206-main-device.xf0eDh` (created 2026-08-24 18:29:45 +0200).
- Summary SHA-256: `6c933cc525ce39e13c2996280e19c8f2053f941133a6a784bb95c85f80de21c8`.
- Commands SHA-256: `54a27992bdfff03438098275551858d521df288d3268f0d03bf5ddf8e78c7969`.
- Final runner status: `PASS`.

An initial attempt without the resource guard was interrupted before assertions when the device
had only 3.4 GiB free. It produced no acceptance result. The bounded rerun completed with the
same source and recorded the setting above.

## Results

- v0 compatibility: 3 fixtures indexed, Recall@1/5 `1.0`, anchor precision `1.0`, false-positive
  rate `0.0`, no failures. Log SHA-256:
  `016784c9f2466714632e227ce2160366a1c17b2ff26bba82c76ac6f1f4a6d8dc`.
- v1 multimodal: 8 fixtures and 11 queries indexed with completeness `1.0`; Recall@1/5 `0.90`,
  MRR `0.90`, anchor precision `1.0`, false-positive rate `0.153846`, reformulation success
  `1.0`, negative no-result rate `0.0`, median/p95 latency `0.208875/0.473167 ms`, and
  database/source-byte ratio `15.1054279`. Log SHA-256:
  `86c0b0548a7425eedff54bf0d62770c49e9cce9b9dc1ab58dcc11db997da1f63`.
- The same two declared local-text failures remain visible: q008 primary paraphrase no-result
  (its reformulation succeeds) and q009 hard-negative duplicate false positive.
- Semantic rebuild/drop/rebuild is repeatable, fails closed after drop, and returns evidence-bound
  search. Summary SHA-256:
  `e2ff422bca1bf669d9f01e2081e8aefd2b186001fc14f0ca51827527e8a0602c`.
- Mixed corpus behavior is unchanged: oversized input is rejected, an outside-root symlink is
  unreachable, and valid replacement content is recovered. Log SHA-256:
  `3fc0c245a70f973014da15de5fc1e80d618c49491286f461d440eff1258d8293`.

The full runner also passed formatting, Clippy with warnings denied, locked workspace tests, Rust
1.88 checks/tests, `npm run check`, local secret/audit checks, and a debug Tauri build. The
benchmark, mixed, and semantic logs are retained in the evidence directory; all top-level log
digests are in `log-sha256.txt`.

## MVP demo

The merged-main demo at `/tmp/loom-mvp-demo-main-0206.lnCnDZ/demo.log` indexed five selected
sources and recovered a text passage, PDF page anchor, and cropped OCR image-region anchor. It
reported OCR enabled with two derived passages and five artifacts/seven passages. Demo SHA-256:
`ae1939dc1a988d265255b522bb8eb3b0bb5a69872007de0ec2f192ff6b3ddcee`.

This reproduction confirms the merged code path on the target device; it does not upgrade the
synthetic benchmark into a real-world quality claim. The saved-web and scanned-page limitations
remain as documented in [the pre-merge evidence](0206-device.md).
