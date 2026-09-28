# Issue 0312 — provenance graph bounds device evidence

- Issue: [#75](https://github.com/AlisinaDevelo/LOOM/issues/75)
- Implementation PR: [#300](https://github.com/AlisinaDevelo/LOOM/pull/300)
- Roadmap status: `done`

## Target

| Field | Recorded value |
| --- | --- |
| Hardware | MacBook Pro 17,1; Apple M1; 8 GB |
| Operating system | macOS 26.6.2 (25G83), arm64 |
| Rust | rustc 1.96.0 / cargo 1.96.0; MSRV rustc 1.88.0 |
| JavaScript | Node v26.7.0 / npm 11.19.0 |
| Command | `scripts/verify-device.sh` (28 steps) |
| Merged-main SHA | `74dcb80` (includes #299 and #300) |
| Result | `status=PASS`, 28 PASS, 0 FAIL |
| `summary.txt` SHA-256 | `fda0b6bbc0c4800f4dac328a98d6aa358151a9d0e302fd0a4a50582b4a2f0041` |
| `log-sha256.txt` SHA-256 | `29c1e15beb00bdf587a7806a4fb6943647f6e8d73a5002f37946f73797bbe4d4` |
| `rust-workspace.log` SHA-256 | `1284c33a74e968877a43a6331451ad41f43564ad0ba4d462e3a28177d2fb201f` |

The locked workspace test run reported 164 passed and 0 failed; the MSRV core run reported 119
passed and 0 failed. `npm run check`, the security scan, and the Tauri debug build passed.

## Acceptance mapping

| Artifact ID | Acceptance criterion | Evidence on this device |
| --- | --- | --- |
| `LOOM-0312-SCHEMA` | Relationship types, evidence requirements, confidence, and provenance method are versioned and constrained by schema checks | `schema_checks_reject_malformed_envelopes_written_directly` passed (empty kind or method, inferred without confidence, unknown origin, out-of-range confidence, and a confidence-clearing update are refused by SQLite). `v8_migration_adds_relationship_indexes_checks_and_compaction_table` passed. |
| `LOOM-0312-COMPACTION` | Compaction preserves user-visible lineage and emits a digest-linked summary for removed redundant edges | `compaction_removes_only_dominated_inferred_edges_and_preserves_lineage` passed: observed and user-confirmed edges are kept, each removed edge names a surviving edge, digests recompute, and summaries survive portable export and import. |
| `LOOM-0312-SCALE` | A synthetic graph fixture proves deterministic traversal order and bounded query cost at the documented scale | `synthetic_graph_traversal_is_indexed_bounded_and_deterministic` passed on 1,000 artifacts and 50,000 edges with a 5,000-edge hub: the plan is a two-index OR search with no table scan, order matches `(created_at, id)` across a reopen, and traversal is capped at 100. `per_artifact_budget_stops_unbounded_growth` passed. |
