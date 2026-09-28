# Issue 0313 — connector import fidelity device evidence

- Issue: [#76](https://github.com/AlisinaDevelo/LOOM/issues/76)
- Implementation PR: [#295](https://github.com/AlisinaDevelo/LOOM/pull/295)
- Roadmap status: `done`

## Target

| Field | Recorded value |
| --- | --- |
| Hardware | MacBook Pro 17,1; Apple M1; 8 GB |
| Operating system | macOS 26.6.2 (25G83), arm64 |
| Rust | rustc 1.96.0 / cargo 1.96.0; MSRV rustc 1.88.0 |
| JavaScript | Node v26.7.0 / npm 11.19.0 |
| Command | `scripts/verify-device.sh` (28 steps) |
| Merged-main SHA | `5b0e674` (includes #293 and #295) |
| Result | `status=PASS`, 28 PASS, 0 FAIL |
| `summary.txt` SHA-256 | `81bb5775e7ab8bbe1c1a2d76b13a3c7437edbebc6c2dbdb9438055661298a972` |
| `log-sha256.txt` SHA-256 | `bd3d5b42def823cd44c3da062dedf4c143c8425be7bc56a9f672d8227876efa9` |
| `rust-workspace.log` SHA-256 | `a7bf29a5779f377ffd73e2786c338835782a82210d541b2c331a70a6a87dec0e` |

The locked workspace test run reported 156 passed and 0 failed; the MSRV core run reported 114
passed and 0 failed. `npm run check`, the native-host contract, the security scan, and the Tauri
debug build passed.

## Acceptance mapping

Bookmark import is the only connector LOOM ships, so this evidence covers that importer.

| Artifact ID | Acceptance criterion | Evidence on this device |
| --- | --- | --- |
| `LOOM-0313-METADATA` | Each importer records its source application, export version, permissions, skipped fields, and original locator | `imports_record_source_application_version_permissions_and_skipped_fields` passed: a Firefox-shaped export records `firefox`, `netscape-bookmark-file-1`, `read_selected_file`, six skipped attribute names, and its canonical path; a plain Netscape export records `netscape_compatible`. |
| `LOOM-0313-REPLAY` | Rights-clean exports import twice with stable artifact identity and no duplicate relationships | `replaying_the_same_export_keeps_artifact_identity_and_adds_no_relationships` passed: a replay returns the same import, records, stats, and relationship count, and a second library produces the same URLs and entry hashes. `populated_v7_migration_adds_connector_metadata_without_rewriting_imports` passed: a migrated import replays to the same ID. |
| `LOOM-0313-FAILURES` | Malformed, partial, and revoked exports produce per-record failures that remain inspectable and retryable | `malformed_records_become_inspectable_failures_without_blocking_good_records` passed (three coded failures, no URL or title stored, a fixed export resolves them and keeps identity); `revoked_exports_stay_inspectable_and_retry_requires_reselection` passed; `structurally_broken_or_empty_exports_still_fail_closed` passed. |
