# Portable export and encrypted backup

Status: roadmap `0305`. Implemented in `crates/loom-core/src/portable.rs` and
`crates/loom-core/src/backup.rs`, and exposed as `loom export`, `loom import-export`,
`loom backup`, and `loom restore`. The cryptographic design below has not had an independent
review.

## What is exported

A portable export is one JSON document:

```json
{
  "format": "loom.portable-export",
  "format_version": 1,
  "library_schema_version": 7,
  "exported_at": "2026-09-28T12:00:00.000Z",
  "settings": {"ocr_enabled": "1", "retention_days": "90"},
  "tables": {"passages": {"columns": ["id", "..."], "rows": [["...", "..."]]}},
  "digest": "blake3:…"
}
```

`tables` holds every canonical row, with columns named as in the live schema:

| Table | Contents |
| --- | --- |
| `source_roots` | Selected files and folders, and whether they are enabled |
| `artifacts`, `artifact_locators`, `artifact_versions` | Source identity, locators, content hashes, extractor identity, warnings, and extraction metadata |
| `passages` | Passage text, text hashes, and exact character, line, page, or pixel anchors |
| `relationships` | Typed provenance links with origin, method, evidence passage, confidence, and metadata |
| `bookmark_imports`, `bookmark_records`, `bookmark_import_items` | Bookmark exports, records, and per-import outcomes |

`settings` carries the user settings `ocr_enabled` and `retention_days`. Derived state is never
exported: the FTS5 projection is refilled from passages on import, and semantic vectors, index
checkpoints, captures, and caches can be rebuilt. Source files themselves are not copied; an
imported artifact whose original path is missing shows as unavailable.

The `digest` is BLAKE3 over the schema version, settings, and tables. It detects accidental
corruption of a plaintext export; it is not a security control. **A plaintext export contains
your passage text and paths.** Use an encrypted backup for anything that leaves the machine.

## Compatibility policy

- An import checks the format, format version, schema version, and digest, and it only goes into
  a library with no canonical rows.
- `format_version` 1 is the only format. A future format bump will keep reading version 1.
- This build imports exports made at library schema 6 and 7 (`IMPORTABLE_SCHEMA_VERSIONS`).
  Schema 6 exports have no bookmark tables. Each new schema release keeps at least the previous
  schema's exports importable.
- Every exported table and column must exist in the live schema, and every required column must
  be present. Unknown tables or columns, such as those from a newer LOOM, are refused rather than
  dropped.
- Rows are inserted in one transaction with deferred foreign keys. SQLite's foreign-key and
  integrity checks must pass before commit, so a failed import leaves the library empty. The FTS5
  health check then runs on the committed rows.

## Encrypted backup format

```text
"LOOMBAK1" | header length (u32 LE) | header JSON | chunk…
chunk = ciphertext length (u32 LE) | XChaCha20-Poly1305 ciphertext with 16-byte tag
```

The header records the format version, the KDF and its parameters, the salt, the AEAD, its nonce
prefix, and the 64 KiB chunk size. The plaintext is the portable export above.

- **Key derivation:** Argon2id v1.3, 64 MiB memory, 3 iterations, 1 lane, 16-byte random salt,
  32-byte key. When reading, parameters are bounded (at most 1 GiB, 16 iterations, 8 lanes) so a
  crafted file cannot demand unbounded work. Passwords must be at least 12 bytes.
- **Encryption:** XChaCha20-Poly1305 in the STREAM construction. Each chunk's 24-byte nonce is a
  random 19-byte prefix, a 32-bit big-endian chunk counter, and a final-chunk flag byte. Every
  chunk authenticates the magic, header length, and header as associated data.
- **What fails:** a wrong password; any changed byte in the header or ciphertext; reordered,
  dropped, duplicated, or appended chunks; and truncation. Tests flip every byte of a sample backup.
- **Secrets:** the password is read from `--password-file` or `LOOM_BACKUP_PASSWORD`, never from a
  command-line argument, and is never written or logged. The derived key and decrypted plaintext
  are zeroized when dropped.
- **Writes:** a backup is written to a temporary file, synced, and linked into place; an existing
  file is never overwritten. A restore authenticates the whole backup, imports it into a staging
  database next to the destination, and links it into place only after the import verifies. A
  failed restore leaves nothing at the destination.

Not covered: key escrow or recovery (a lost password means a lost backup), hiding the backup's
approximate size, and protection against malware on the machine that holds the password.

## Commands

```text
loom --database .loom/library.sqlite3 export library.json
loom --database new.sqlite3 import-export library.json
LOOM_BACKUP_PASSWORD='…' loom --database .loom/library.sqlite3 backup library.loombak
loom --database restored.sqlite3 restore library.loombak --password-file ~/.loom-backup-password
```
