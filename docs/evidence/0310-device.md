# Issue 0310 — capture lifecycle device evidence

- Issue: [#73](https://github.com/AlisinaDevelo/LOOM/issues/73)
- Implementation PR: [#292](https://github.com/AlisinaDevelo/LOOM/pull/292)
- Merged-main SHA under test: `f0352715f41985dfba4d1aa6f8c48806d4d0f16d`
- Roadmap status: `done`

## Target

| Field | Recorded value |
| --- | --- |
| Hardware | MacBook Pro 17,1; Apple M1; 8 GB |
| Operating system | macOS 26.6.2 (25G83), arm64 |
| Rust | rustc 1.96.0 / cargo 1.96.0; MSRV rustc 1.88.0 |
| JavaScript | Node v26.7.0 / npm 11.19.0 |
| Command | `scripts/verify-device.sh` (28 steps) |
| Result | `status=PASS`, 28 PASS, 0 FAIL |
| `summary.txt` SHA-256 | `935a5c1f77e804911a5675a0eaecf1f2f8a954f51585d8920b19269da660bb31` |
| `log-sha256.txt` SHA-256 | `c406efb31007b6d56536ec34c4c36d87a19f34a4dea8ede1dde1b8640b32555c` |
| `rust-workspace.log` SHA-256 | `36ca1b4ec5c9799ba50cd8d46a759e77d135f2741ef3ef9b3a977b270359550d` |
| `npm-check.log` SHA-256 | `ea5090e2d7f58a1ddd8cfac9da90fe25338d89e4be536a5d63f702d08d13d61b` |

The locked workspace test run reported 139 passed and 0 failed; the MSRV core run reported 97
passed and 0 failed. `npm run check` passed, and the browser-extension suite reported 12 passed.

## Acceptance mapping

| Artifact ID | Acceptance criterion | Evidence on this device |
| --- | --- | --- |
| `LOOM-0310-STATES` | The protocol models requested, accepted, rejected, expired, deleted, and revoked states with timestamps and source identity | `lifecycle::tests::transition_fixture_covers_every_state_and_event` checks all 36 state/event pairs in `crates/loom-browser-capture/fixtures/lifecycle-v1.json`; `terminal_states_accept_no_event` and `rejection_codes_map_to_rejected_or_expired` pass. `tests::accepted_capture_records_requested_and_accepted_transitions` shows a host-accepted record carries the user-gesture and acceptance timestamps. |
| `LOOM-0310-DELETION` | Deletion removes searchable derivatives and does not silently preserve a managed copy | `tests::lifecycle_fixture_revoke_repair_delete_leaves_no_managed_copy` accepts a real capture through the native host, revokes, re-pairs, revokes, and deletes it, then finds no spool file containing its URLs, title, selection, or snapshot bytes; the tombstone keeps only the allowed fields. `rejected_and_expired_requests_persist_nothing` and `deletion_rejects_unknown_and_malformed_request_ids` pass. Spooled captures are not indexed into the searchable library yet, so no searchable rows exist to remove. |
| `LOOM-0310-COPY` | UI copy and privacy documentation identify URL, snapshot, credential, and referrer handling per state | `lifecycle::tests::protocol_documents_every_state_message` and the extension test `lifecycle copy matches the protocol and maps host responses` pass; the per-state table is in `docs/protocol/browser-capture-v1.md` and summarized in `docs/PRIVACY.md`. |

## Limits

Hosted GitHub Actions results are not used as evidence here. No consented interactive browser
session was run for this item, and none is required by its criteria; that gate belongs to 0301.
