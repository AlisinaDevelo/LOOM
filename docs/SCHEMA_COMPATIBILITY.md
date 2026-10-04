# Schema compatibility

This is the compatibility contract for the current canonical SQLite store. Canonical source,
version, passage, anchor, and relationship rows are authoritative; FTS5 and checkpoint state are
derived or diagnostic and may be rebuilt.

## Version matrix

|Database state|Open behavior|Data guarantee|Recovery path|
|---|---|---|---|
|No database or empty SQLite file|Create schema version 10 transactionally|Creates canonical tables, consent generations, FTS5/vocabulary projections, extraction metadata, graph bounds/compaction, bookmark import history, triggers, and `schema_meta` marker|Index or import explicitly selected sources|
|Schema version 10 with the expected shape|Open without changing canonical rows|Validate required tables/columns; deterministically rebuild derived FTS5 from canonical passages|Reopen, then re-index approved roots or imports|
|Schema version 9 with the expected shape|Run the reviewed v9→v10 transaction|Preserve all canonical rows; add `source_roots.scope_generation` with a nonnegative zero default|Reopen; revocation and re-selection invalidate older consent generations|
|Schema version 8 with the expected shape|Run the reviewed v8→v10 transaction|Preserve all canonical rows; add graph indexes, envelope checks, compaction table, and consent generations|Reopen; run `compact-relationships` if the graph has redundant inferred edges|
|Schema version 7 with the expected shape|Run the reviewed v7→v10 transaction|Preserve all canonical rows and import IDs; add connector metadata with safe defaults, import failures, graph bounds, and consent generations|Reopen and replay or re-select bookmark exports|
|Schema version 6 with the expected shape|Run the reviewed v6→v10 transaction|Preserve all canonical rows; add bookmark history, graph bounds, and consent generations|Reopen and re-index approved roots or imports|
|Schema version 5 with the expected shape|Run the reviewed v5→v10 transaction|Preserve hashes, extractor identity, anchors, relationships, and source identity; add relationship envelope defaults and later additive fields|Reopen and re-index approved roots|
|Schema version 4 with the expected shape|Run the reviewed v4→v10 transaction|Preserve canonical evidence; add extraction metadata and later additive fields with safe defaults|Reopen and re-index approved roots|
|Schema version 3 with the expected shape|Run the reviewed v3→v10 transaction|Preserve canonical evidence; add PDF page/warning metadata and later additive fields|Reopen and re-index approved roots|
|Schema version 2 with the expected shape|Run the reviewed v2→v10 transaction|Preserve canonical evidence; add `index_jobs`, extraction metadata, and later additive fields with safe defaults|Reopen and re-index approved roots|
|Schema version 1|Reject before migration|The marker is not overwritten because its content-version uniqueness contract is incompatible|Rebuild from the original source files or export outside LOOM|
|Missing marker on a non-empty database|Reject before migration|No tables or marker are created|Recover from a known LOOM export or rebuild from source files|
|Malformed known version or unknown future version|Reject before migration with a named `UnsupportedSchemaVersion` reason|The existing marker and canonical rows are left untouched|Use a compatible LOOM release or rebuild from source files|

## Rebuild and migration rules

- The checked-in migration fixture is [`tests/fixtures/schema-v2.sql`](../tests/fixtures/schema-v2.sql).
  It contains populated source, version, passage, and relationship rows rather than only an empty
  marker.
- Opening a version-2 fixture creates `index_jobs`, PDF/image metadata defaults, relationship
  envelope defaults, bookmark tables, consent generations, and records version 10 in one transaction.
  The migration never recomputes or
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
  version marker advances only after the columns, bookmark tables, and derived projections succeed.
- Version-6 databases receive the bookmark import, current-record, and per-item outcome tables plus
  the URL lookup index. The migration is additive and leaves existing artifacts, versions, passages,
  and relationships byte-for-byte intact.
- Version-7 databases receive `source_application` and `export_version` (`unknown`),
  `permissions_json` (`["read_selected_file"]`), `skipped_fields_json` (`[]`), and `status`
  (`complete`) on `bookmark_imports`, plus the empty `bookmark_import_failures` table. Import IDs,
  records, and artifacts are unchanged, so replaying an export after migration returns the same
  import.
- Version-8 databases receive the relationship source, target, and edge indexes, the
  `relationships_envelope_insert`/`_update` check triggers, and the empty
  `relationship_compactions` table. Existing relationship rows are not rewritten; the triggers only
  check new and updated rows.
- Version-9 databases receive `source_roots.scope_generation` with a zero default and nonnegative
  constraint. Root IDs, enabled states, hashes, passage IDs, anchors, and checkpoints are unchanged.
  A version-10 database missing this column is refused before any migration writes occur.
- The compatibility tests cover create/open, populated v2 migration, derived-index rebuild,
  malformed-v2 refusal, unknown-version refusal, and canonical-row preservation on reopen.

## Support policy

The background queue has its own operational version, independent of canonical schema 10.
Runtime v5 adds directory manifests/quanta and locator deletion/update fences. Recognized v4/v3/v2
and legacy queues require explicit `upgrade-job-runtime` under kernel worker ownership; ordinary
canonical open never silently upgrades them. Existing FTS/file jobs survive the transactional upgrade.
Unknown layouts, stale v5 objects under older markers and invalid copied rows roll back. Older
workers must refuse v5 before claiming work. Portable exports exclude runtime rows/indexes/triggers;
restore validates the runtime and clears its manifests and claims. See [background jobs](BACKGROUND_JOBS.md).

Runtime v5 supports only the exact checked-in layout, including unit checksums and all ownership
fences. Earlier unpushed, unreleased development prototypes also used the v5 marker; they are not
supported migration inputs. A marker alone never authorizes migration or deletion: an unrecognized
shape remains canonically readable but refuses queue operations and destructive runtime changes.
Private runtime namespace checks are case-insensitive, matching SQLite identifier semantics.

Schema version 10 is the supported local format. Versions 2 through 9 are supported by reviewed
transactional migrations. Version 1 and unknown/future versions are intentionally rejected; LOOM
does not promise to infer or rewrite an unrecognized format. Users with a rejected database keep
the original file and must either use a compatible release, restore a known export, or rebuild from
the original sources. [Portable export/import](PORTABILITY.md) is a separate, explicit operation
and is never silently substituted for a failed migration.
