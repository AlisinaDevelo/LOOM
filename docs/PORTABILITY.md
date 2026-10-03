# Portable export and encrypted backup

Status: roadmap `0305`. Implemented in `crates/loom-core/src/portable.rs` and
`crates/loom-core/src/backup.rs`, and exposed as `loom export`, `loom import-export`,
`loom backup`, and `loom restore`. An AI-assisted security review (recorded in
`docs/evidence/0305-device.md`) found no critical or high issues, and its medium and low
findings are fixed below. No independent human review of the design has been performed.

## What is exported

A portable export is one JSON document:

```json
{
  "format": "loom.portable-export",
  "format_version": 1,
  "library_schema_version": 10,
  "exported_at": "2026-09-28T12:00:00.000Z",
  "settings": {"ocr_enabled": "1", "retention_days": "90"},
  "tables": {"passages": {"columns": ["id", "..."], "rows": [["...", "..."]]}},
  "digest": "blake3:…"
}
```

`tables` holds every canonical row, with columns named as in the live schema:

| Table | Contents |
| --- | --- |
| `source_roots` | Selected files and folders, enabled state, and consent generation |
| `artifacts`, `artifact_locators`, `artifact_versions` | Source identity, locators, content hashes, extractor identity, warnings, and extraction metadata |
| `passages` | Passage text, text hashes, and exact character, line, page, or pixel anchors |
| `relationships`, `relationship_compactions` | Typed provenance links with origin, method, evidence passage, confidence, and metadata, plus digest-linked records of compacted edges |
| `bookmark_imports`, `bookmark_records`, `bookmark_import_items`, `bookmark_import_failures` | Bookmark exports with their connector metadata, records, per-import outcomes, and per-record failures |

`settings` carries the user settings `ocr_enabled` and `retention_days`. Derived state is never
exported: the FTS5 projection is refilled from passages on import, and semantic vectors, index
checkpoints, captures, and caches can be rebuilt. Source files themselves are not copied; an
imported artifact whose original path is missing shows as unavailable.

The `digest` is BLAKE3 over the schema version, settings, and tables. It detects accidental
corruption of a plaintext export; it is not a security control, and a plaintext export can be
edited and resealed. Treat exports from others as untrusted input. **A plaintext export contains
your passage text and paths.** `loom export` writes it readable only by you (0600 on Unix), but it
is unencrypted at rest; use an encrypted backup for anything that leaves the machine. Inputs to
`import-export` and `restore` must be regular files of at most 2 GiB.

## Compatibility policy

- An import checks the format, format version, schema version, and digest, and it only goes into
  a library with no canonical rows.
- `format_version` 1 is the only format. A future format bump will keep reading version 1.
- This build imports exports made at library schema 6 through 10 (`IMPORTABLE_SCHEMA_VERSIONS`).
  Schema 6 exports have no bookmark tables; schema 7 exports have no connector metadata columns or
  per-record import failures, which take their documented defaults; schema 8 exports have no
  relationship compaction summaries. Schema 9 exports have no consent generation, which defaults
  to zero; schema 10 exports preserve it and the enabled state without re-authorizing revoked roots.
  Import clears runtime checkpoints and rotates the local authorization incarnation, so an old
  worker cannot inherit restored root IDs/generations. Neither runtime value is exported.
  Foreign-key-valid bookmark rows must also agree on source-root ownership and export locator;
  cross-scope imports, records, items, and failure-resolution links are refused with full rollback.
  Each new schema release keeps
  at least the previous schema's exports importable.
- Every exported table and column must exist in the live schema, and every required column must
  be present. Unknown tables or columns, such as those from a newer LOOM, are refused rather than
  dropped.
- Rows are inserted in one transaction with deferred foreign keys. SQLite's foreign-key and
  integrity checks must pass before commit, so a failed validation leaves the library empty.
  Integers outside SQLite's 64-bit range are rejected rather than converted. The FTS5 health check
  runs after commit; if it fails, the import reports an error with the rows already present, and
  `loom fts-repair` rebuilds the index.

## Encrypted backup format

```text
"LOOMBAK1" | header length (u32 LE) | header JSON | chunk…
chunk = ciphertext length (u32 LE) | XChaCha20-Poly1305 ciphertext with 16-byte tag
```

The header records the format version, the KDF and its parameters, the salt, the AEAD, its nonce
prefix, and the 64 KiB chunk size. The plaintext is the portable export above.

- **Key derivation:** Argon2id v1.3, 64 MiB memory, 3 iterations, 1 lane, 16-byte random salt,
  32-byte key. Writing refuses anything below the OWASP floor of 19 MiB and 2 iterations. Reading
  checks the header before deriving a key and refuses more than 256 MiB, 6 iterations, or 4 lanes,
  so a crafted file cannot demand unbounded work. Hex fields must be lowercase ASCII. Passwords
  must be at least 12 bytes; there is no strength meter.
- **Encryption:** XChaCha20-Poly1305 in the STREAM construction. Each chunk's 24-byte nonce is a
  random 19-byte prefix, a 32-bit big-endian chunk counter, and a final-chunk flag byte. Every
  chunk authenticates the magic, header length, and header as associated data.
- **What fails:** a wrong password; any changed byte in the header or ciphertext; reordered,
  dropped, duplicated, or appended chunks; and truncation. Tests flip every byte of a sample backup.
- **Secrets:** the password is read from `--password-file` or `LOOM_BACKUP_PASSWORD`, never from a
  command-line argument, and is never written or logged. LOOM warns when the password file is
  readable by other users. An environment variable is visible to other processes of the same user,
  so prefer the file. Wiping is best effort: the password, the derived key, the cipher's key
  schedule, and the decrypted buffer are zeroized when dropped, but copies made while serializing or
  parsing the export, SQLite's own page cache, and Argon2's working memory are not.
- **Writes:** a backup is written to a temporary file, synced, and linked into place; an existing
  file is never overwritten. A restore authenticates the whole backup, then imports it into a
  staging database in a private (0700) directory next to the destination. It checkpoints the WAL,
  syncs, links the finished file into place, and removes the staging directory. A failed restore
  leaves nothing at the destination; staging left by a crash is removed by the next restore into
  the same folder once it is an hour old. Linking needs a filesystem with hard links; FAT and exFAT
  drives are not supported as a destination.
- **Plaintext at rest:** only the backup file is encrypted. The restored library, like every LOOM
  library, is not encrypted at rest, and the staging database is plaintext while a restore runs.

Not covered: key escrow or recovery (a lost password means a lost backup), hiding the backup's
approximate size, and protection against malware on the machine that holds the password.

## Commands

```text
loom --database .loom/library.sqlite3 export library.json
loom --database new.sqlite3 import-export library.json
LOOM_BACKUP_PASSWORD='…' loom --database .loom/library.sqlite3 backup library.loombak
loom --database restored.sqlite3 restore library.loombak --password-file ~/.loom-backup-password
```
