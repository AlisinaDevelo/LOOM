# Schema compatibility

This is the compatibility contract for the current canonical SQLite store. Canonical source,
version, passage, anchor, and relationship rows are authoritative; FTS5 and checkpoint state are
derived or diagnostic and may be rebuilt.

## Version matrix

|Database state|Open behavior|Data guarantee|Recovery path|
|---|---|---|---|
|No database or empty SQLite file|Create schema version 6 transactionally|Creates the canonical tables, FTS5/vocabulary projections, PDF/image extraction metadata, typed relationship envelope columns, triggers, and `schema_meta` marker|Index explicitly selected sources|
|Schema version 6 with the expected shape|Open without changing canonical rows|Validate required tables/columns; rebuild FTS5 only when its row count is inconsistent|Reopen, then re-index approved roots if source files are available|
|Schema version 5 with the expected shape|Run the reviewed v5→v6 transaction|Preserve hashes, extractor identity/version, anchors, relationship rows, and source identity; add relationship schema version, origin, and metadata defaults|Reopen and re-index approved roots|
|Schema version 4 with the expected shape|Run the reviewed v4→v6 transaction|Preserve hashes, extractor identity/version, anchors, relationship rows, and source identity; add extraction metadata and relationship fields with safe defaults|Reopen and re-index approved roots|
|Schema version 3 with the expected shape|Run the reviewed v3→v6 transaction|Preserve hashes, extractor identity/version, anchors, relationship rows, and source identity; add PDF page/warning, extraction metadata, and relationship fields with safe defaults|Reopen and re-index approved roots|
|Schema version 2 with the expected shape|Run the reviewed v2→v6 transaction|Preserve hashes, extractor identity/version, anchors, relationship rows, and source identity; add `index_jobs`, PDF metadata, extraction metadata, and relationship fields with safe defaults|Reopen and re-index approved roots|
|Schema version 1|Reject before migration|The marker is not overwritten because its content-version uniqueness contract is incompatible|Rebuild from the original source files or export outside LOOM|
|Missing marker on a non-empty database|Reject before migration|No tables or marker are created|Recover from a known LOOM export or rebuild from source files|
|Malformed known version or unknown future version|Reject before migration with a named `UnsupportedSchemaVersion` reason|The existing marker and canonical rows are left untouched|Use a compatible LOOM release or rebuild from source files|

## Rebuild and migration rules

- The checked-in migration fixture is [`tests/fixtures/schema-v2.sql`](../tests/fixtures/schema-v2.sql).
  It contains populated source, version, passage, and relationship rows rather than only an empty
  marker.
- Opening a version-2 fixture creates `index_jobs`, PDF/image metadata defaults, relationship
  envelope defaults, and records version 6 in one transaction. The migration never recomputes or
  replaces canonical hashes, extractor identity, anchors, or relationships.
- The FTS5 table and its vocabulary projections are disposable. On open, LOOM deterministically
  issues the FTS5 `rebuild` command from canonical passages. `fts-health` compares a fresh scratch
  tokenizer projection with the current vocabulary; `fts-repair` rebuilds only derived state. A
  full source re-index remains the recovery path for missing canonical records.
- A malformed version-2 marker fails with a named reason such as `schema version 2 is missing
  required table \`source_roots\`` before any new table is created. Unknown and pre-alpha versions
  remain untouched.
- Version-3 databases receive `parse_warnings_json` (`[]`), nullable `page_count`,
  `extraction_metadata_json` (`{}`), and relationship envelope defaults in a single transaction.
  Existing canonical hashes, passages, anchors, and relationship rows are not rewritten; the
  version marker advances only after the columns and derived projections succeed.
- The compatibility tests cover create/open, populated v2 migration, derived-index rebuild,
  malformed-v2 refusal, unknown-version refusal, and canonical-row preservation on reopen.

## Support policy

Schema version 6 is the supported local format. Versions 2, 3, 4, and 5 are supported by reviewed
transactional migrations. Version 1 and unknown/future versions are intentionally rejected; LOOM
does not promise to infer or rewrite an unrecognized format. Users with a rejected database keep
the original file and must either use a compatible release, restore a known export, or rebuild from
the original sources. Portable export/import is a later roadmap capability and is not silently
substituted for a failed migration.
