# Issue 0305 — portable export and encrypted backup device evidence

- Issue: [#34](https://github.com/AlisinaDevelo/LOOM/issues/34)
- Implementation PR: [#293](https://github.com/AlisinaDevelo/LOOM/pull/293)
- Roadmap status: `review`

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

| Artifact ID | Acceptance criterion | Evidence on this device |
| --- | --- | --- |
| `LOOM-0305-SCHEMA` | Export schema and compatibility policy are public and cover artifacts, passages, relationships, settings, and evidence metadata | `docs/PORTABILITY.md`; `export_import_round_trip_preserves_every_canonical_row_and_setting` passed. |
| `LOOM-0305-CRYPTO` | Encrypted archives use a reviewed password/KDF/AEAD design and never persist plaintext secrets | The Argon2id + XChaCha20-Poly1305 STREAM design is documented in `docs/PORTABILITY.md`. `backup::tests` (six tests, including every single-byte flip, truncation, reordering, and hostile headers) and `encrypted_backup_restores_into_a_new_library` (no plaintext in the file) passed. **No independent review of the design has been performed.** |
| `LOOM-0305-ROUNDTRIP` | Round-trip and corrupted/tampered archive tests pass across two schema revisions | `imports_exports_from_both_supported_schema_revisions` (schema 6, 7, and 8; 5 refused), `corrupted_and_hostile_exports_fail_closed_without_partial_rows`, and `tampered_truncated_or_wrong_password_backups_never_create_a_library` passed. |

## Closure gate

Issue #34 remains open in `review` until the cryptographic design has an independent review.
