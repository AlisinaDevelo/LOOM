# Issue 0311 — browser extension message boundary device evidence

- Issue: [#74](https://github.com/AlisinaDevelo/LOOM/issues/74)
- Implementation PR: [#299](https://github.com/AlisinaDevelo/LOOM/pull/299), building on 0300/0301
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

The native-host device contract reported: `PASS (1 accepted, 4 deterministic rejections, 3 unpaired
callers refused, recovery safe)`, run against the built `loom-native-host` binary.

## Acceptance mapping

| Artifact ID | Acceptance criterion | Evidence on this device |
| --- | --- | --- |
| `LOOM-0311-ALLOWLIST` | Messages use a versioned allowlist with source URL, user action, bounded metadata, and a one-time capture reference | Protocol v1 exact-key envelopes with single-use `request_id` and intent token (`docs/protocol/browser-capture-v1.md`); `rejects_unknown_fields_before_authentication` and `rejects_replay_and_accepts_new_counter_after_integrity_failure` passed. |
| `LOOM-0311-REJECT` | The boundary rejects cookies, passwords, arbitrary JavaScript, page scripts, and unbounded HTML by default | `boundary_rejects_downgrade_oversize_and_nested_secrets_without_writing` (nested password, 64 KiB+ selection, 8 MiB+ snapshot, oversized frame), `rejects_untrusted_payload_without_writing_anything`, and `rejects_remote_or_event_attributes_even_when_tags_are_allowed` passed. |
| `LOOM-0311-THREATS` | Threat-model and negative fixtures cover replay, origin spoofing, oversized payloads, and extension downgrade | The threat table gains an origin-spoofing row. `caller_identity_is_parsed_for_chrome_and_firefox_launches` and `spoofed_malformed_or_missing_callers_are_refused` passed, the device contract refused three unpaired callers before any I/O, and the downgrade and oversize cases above passed. |

## Limits

No interactive browser session was run; that gate belongs to 0301 (#30).
