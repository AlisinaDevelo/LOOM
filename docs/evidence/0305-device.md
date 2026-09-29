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

## Security review (2026-09-29)

An AI-assisted security review (a separate reviewing agent, not the author) read `backup.rs`,
`portable.rs`, the CLI commands, and this design, ran the tests, and ran throwaway experiments. It
found the cryptographic core sound. It checked the nonce layout, the truncation, reordering,
extension, and header binding, the error oracle, and SQL identifier handling, and found no critical
or high issues. Its findings and their resolution in #305:

| Severity | Finding | Resolution |
| --- | --- | --- |
| Medium | Restore staged a plaintext database with default permissions; a crash could orphan it; `loom export` wrote 0644 and could leave a partial file; none of this was documented | Staging now lives in a 0700 directory removed on every path, and stale staging older than an hour is swept. Exports are written 0600 through a temp file and hard link. `PORTABILITY.md` states what is plaintext at rest. |
| Medium | Header KDF bounds (1 GiB, 16 passes, 8 lanes) allowed a crafted file to force a long, memory-heavy derivation before authentication; no floor when writing | Reading refuses more than 256 MiB, 6 passes, or 4 lanes before deriving. Writing refuses less than 19 MiB or 2 passes. |
| Medium | Zeroization claims overstated: the cipher's key copy was not wiped, buffers regrew, and the CLI password was a plain `String` | `chacha20poly1305` `zeroize` feature enabled; decrypt buffer preallocated; CLI password and password-file contents held in `Zeroizing`; claims narrowed to best effort. |
| Low | A non-ASCII salt or nonce prefix panicked the hex decoder before authentication | Strict lowercase ASCII hex decoding; regression test. |
| Low | Unbounded reads of backup and export files, including FIFOs | Regular files of at most 2 GiB only. |
| Low | FTS check after commit contradicted "failed import leaves the library empty" | Documented precisely; `fts-repair` is the recovery. |
| Low | Restore deleted the staging WAL without checkpointing, and did not fsync | WAL checkpointed and switched to a rollback journal before handoff; file and directory synced. |
| Low | Thin password policy | World-readable password files now warn; the rest is documented. |
| Info | Integers above `i64::MAX` were imported as REAL | Rejected. |

The review is AI-assisted and does not replace an independent human review of the design.

## Closure gate

Issue #34 remains open in `review` until the cryptographic design has an independent review.
