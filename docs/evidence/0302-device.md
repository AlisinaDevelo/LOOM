# Issue 0302 — provenance relationship evidence

- Issue: [#31](https://github.com/AlisinaDevelo/LOOM/issues/31)
- Implementation PR: [#223](https://github.com/AlisinaDevelo/LOOM/pull/223)
- Implementation commit: `ea16b95fb413d0cd30cd0b6ac07a45fc08c53c20`
- Merged-main SHA: `217903c82e500b85519781cf2beb3da403808d5e`
- Roadmap status: `review`
- Verification status: core, CLI, frontend, extension, and migration paths pass on the
  device; the desktop Tauri check and test-profile rebuild are resource-limited by local
  disk and the issue remains open for that gate.

## Target and retained evidence

| Field | Recorded value |
| --- | --- |
| Hardware | MacBook Pro 17,1; Apple M1; 8 GB |
| Operating system | macOS 26.6.2 (25G83), arm64 |
| Rust | rustc 1.96.0 / cargo 1.96.0 |
| JavaScript | Node v26.7.0 / npm 11.19.0 |
| Python | 3.9.6 |
| Merged-main run | `/tmp/loom-0302-merged-final.44MdBn` |
| Evidence manifest | `log-sha256.txt`, SHA-256 `3c33fdb06df48b0cf924b7f6f6bf3b20f7355ad4353bdea234b06ac23969f957` |
| Main protection | Ruleset `main-protection` (`21332347`), active on `refs/heads/main` |

The run directory retains merged-main core, CLI, frontend, extension, roadmap, Tauri
attempts, device disk, and toolchain logs. Its manifest is:

```text
4729924aefa8540b81df5b5ccafd11d6cf06333ec6629e39193d0995b0ba8ac0  loom-0302-merged-core-tests.log
901bb25afb64f7541928d748cac65991ecd91ff317615d006288cdb152a4c1e5  loom-0302-merged-cli-tests.log
9ee838a239a0ddb98bf46a495a27c11b75b18602456e255a15554fb0b6717a63  loom-0302-merged-frontend-check.log
b9a7949d0739b598583c8349deb0e4831999323f07024de621a270a3fe0ba244  loom-0302-merged-browser-tests.log
df36f3da734b1902d0f0e6711aeb03645bc9f04afad31993b9275c55f1a82c96  loom-0302-merged-roadmap.log
e9dca69c19b5746097fdfa0fe9c065c0408b567380d15322b5b8fd489f137e65  loom-0302-merged-tauri-check.log
037a75845b50ab29f4f4872508aaee55175e1e524a8763a7da46c8d968966bcc  loom-0302-merged-tauri-lib-check.log
1c75a57caf8bf81a23e3de99ac83be34da2305755f7073bb744e1b7886d00075  device-disk.txt
16e5956000df2cea068d6bba8a3805974eb81310297205c42e45d3d96bbf5a89  device-toolchain.txt
```

## Acceptance mapping

| Artifact ID | Acceptance criterion | Retained evidence |
| --- | --- | --- |
| `LOOM-0302-SCHEMA` | Versioned typed relationships preserve future kinds | `RelationshipKind` serializes known values and preserves `Unknown(String)`. The v6 table adds an independent relationship schema version, origin, metadata envelope, endpoint and confidence checks. `schema_compatibility` passes six tests, including populated v2, v3, v4, and v5 migrations without rewriting canonical rows. |
| `LOOM-0302-INFERENCE` | Inferred links record method, evidence, confidence, and time; confirmation stays distinct | `add_relationship` rejects inferred rows without both passage evidence and confidence, requires a non-empty method, records RFC3339 `created_at`, and stores `origin` separately. `provenance` covers round-trip metadata, unknown kinds, invalid evidence, NaN confidence, oversized/non-object metadata, and user-confirmed edges. |
| `LOOM-0302-TRAVERSE` | UI traverses source and versions without a graph database | `list_relationships` returns bounded source/target endpoint projections with active locator, version, hash, and state. The React test `traverses a verified result through source-backed relationship endpoints` exercises the evidence viewer and Tauri invocation. `cargo check --locked -p loom` passed before merge on this device. |

## Merged-main device checks

These commands were rerun at merged SHA `217903c82e500b85519781cf2beb3da403808d5e`.
Hosted GitHub Actions were not used as evidence.

```text
cargo test --locked -p loom-core --lib --tests  81 passed; 0 failed
cargo test --locked -p loom-cli                    9 passed; 0 failed
npm run check                                      passed
npm run test:browser-extension                     10 passed; 0 failed
python3 scripts/roadmap.py --validate-only        valid
cargo fmt --all -- --check                        passed
git diff --check                                  passed
```

`npm run check` ran ESLint, Markdown lint over 71 files, Vitest (21 tests), TypeScript,
and a Vite production build. The roadmap validator reported 154 active issues, 4 retired
issues, 20 milestones, 13 phases, 141 parent edges, and 314 dependency edges.

The merged-main `cargo check --locked -p loom` attempt reached the macOS dependency graph
but failed with `ENOSPC` while writing `sha2` metadata. The narrower `cargo check --locked
-p loom --lib` attempt reached `tauri` and failed with `ENOSPC` while writing its metadata.
The same desktop check passed on the implementation commit before merge; no merged-main
desktop pass is claimed until this Mac has sufficient free space. The earlier test-profile
attempt likewise failed while compiling `objc2-app-kit` for the same device-capacity reason.

## Failure, privacy, and recovery coverage

- Self-edges, empty/unknown kinds, missing passage bindings, non-finite confidence,
  inferred-without-evidence, non-object metadata, and oversized metadata fail closed before
  a row is written.
- Repeating an identical relationship is idempotent; purging an endpoint cascades the edge.
- Endpoint reads are bounded to 100 rows, expose hashes and active locators, and return an
  unavailable source state instead of inventing a path.
- Relationship metadata is stored as a bounded JSON object and is not rendered as HTML by
  the viewer. The relationship command is read-only; no network or account is required.
- v2–v5 migration fixtures verify defaults, canonical identity preservation, and FTS recovery.

No desktop screenshot was used as evidence. If a future handoff includes a visual capture,
it must be cropped to the relevant LOOM/result panel only, with no full desktop, credentials,
source paths, or private documents.

Issue #31 remains open in `review` until the merged-main desktop compile/test gate can be
rerun on a device with sufficient free storage and the packaged interactive viewer is
verified.
