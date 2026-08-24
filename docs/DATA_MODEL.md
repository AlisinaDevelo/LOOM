# Data model

This document describes the schema currently created by LOOM schema version 7. The supported
version matrix and migration policy are maintained in [SCHEMA_COMPATIBILITY.md](SCHEMA_COMPATIBILITY.md).

## Identity and source records

|Table|Purpose|Important fields|
|---|---|---|
|schema_meta|Records the schema version|key, value|
|source_roots|An explicitly selected file or directory|kind, unique locator, enabled, timestamps|
|artifacts|A logical source under a root|title, media_type, state, active_version_id|
|artifact_locators|Resolves an artifact to a source location|kind, locator, active, first/last seen timestamps|
|artifact_versions|Immutable content observations|content_hash, byte_size, mtime, extractor/version, page_count, parse_warnings_json, extraction_metadata_json, status|
|passages|Normalized text and exact anchors|artifact version, ordinal, text, text hash, JSON locator, character/line/pixel offsets|
|relationships|Typed source-to-source relationships|source/target artifacts, kind, origin, optional evidence passage, method, confidence, metadata, relationship schema version|
|bookmark_imports|One source-faithful local browser export|selected export locator, Netscape format, BLAKE3 export hash, import timestamp|
|bookmark_records|Current bookmark metadata and artifact identity|folder path, title, URL, browser timestamps, entry hash, first import|
|bookmark_import_items|Per-import idempotence and merge history|import, bookmark, ordinal, entry hash, imported/unchanged/merged/conflict outcome|
|index_jobs|Durable progress for one root scan|discovery fingerprint, total/next unit, state, error, timestamps|
|semantic_index_meta|One disposable semantic-index manifest|provider/model/tokenizer, dimension, normalization, build parameters, revision, canonical digest/counts, vector bytes|
|semantic_embeddings|Rebuildable vector per active passage|passage hash, provider/model/tokenizer, dimension, normalization, build parameters, revision, encoded vector bytes|

File ingestion creates file locators. The bookmark connector creates URL locators only for URLs
present in an explicitly selected Netscape HTML export; it stores metadata and never resolves the
URL. Managed-copy locators remain future work.

An artifact is the logical identity of a source locator. A new content hash creates a new artifact
version and can become the active version. Re-indexing unchanged bytes with the same extractor
reuses the existing content version; a changed extractor identity/version creates a fresh
observation even when source bytes are unchanged. Older versions remain available to the canonical
store until a future retention policy defines otherwise.

## Passage records

passages stores the normalized passage text for one artifact version:

- id identifies the passage record;
- artifact_version_id links it to the observed source version;
- ordinal preserves passage order;
- text is normalized UTF-8 text;
- text_hash identifies the passage text;
- locator_json stores the serialized text or PDF-page anchor;
- char_start and char_end are character offsets;
- line_start and line_end are one-based line offsets;
- created_at records insertion time.

The current segmenter targets 1,000 characters and overlaps adjacent passages by 120 characters.
Offsets are calculated without splitting Unicode scalar boundaries. The source content hash is
BLAKE3 over the bytes read before line-ending normalization; the passage offsets therefore apply to
the normalized text stored in the passage.

PDF passages are segmented independently per one-based page. Their `pdf_page` anchor retains the
page number plus local character/line offsets, so a result can be opened against the original PDF
without treating a concatenated text projection as a page identity. `artifact_versions.page_count`
and `parse_warnings_json` retain parser/page outcomes; an empty text layer is an explicit bounded
failure rather than a successful zero-evidence index.

Image passages are one OCR region per passage. Their `image_region` anchor stores normalized-text
offsets plus clamped top-left pixel bounds in the EXIF-oriented image space, the encoded image
dimensions, EXIF orientation, fixed-point display scale, and OCR confidence. `extraction_metadata_json`
records the local provider, provider version, model revision, dimensions, orientation, scale, and
region count. The source image bytes remain at the original locator; LOOM stores only derived OCR
text and metadata.

## Relationship records

`relationships` is a canonical, source-to-source edge table. Every row has a relationship envelope
version, a typed known kind (`saved_from`, `screenshot_of`, `duplicate_of`, `previous_version_of`,
or `related`) or an `Unknown` string preserved verbatim for future readers, an origin (`observed`,
`inferred`, or `user_confirmed`), the method that produced it, optional passage evidence, optional
confidence in the inclusive range 0–1, creation time, and bounded JSON metadata. New edges require
both endpoint artifacts to exist; supplied evidence must belong to one endpoint. Repeating the
same source/target/kind/origin/method observation returns the existing row without replacing its
evidence.

The core exposes a bounded relationship listing that joins both endpoint artifacts to their active
source URI, version ID, content hash, title, media type, and lifecycle state. The desktop viewer
uses that projection to traverse a verified result to its related source and current version without
introducing a graph database or treating inferred edges as confirmed facts.

## Bookmark records

`bookmark_imports` records one local Chrome or Firefox Netscape HTML export by its canonical
selected-file locator, format, BLAKE3 content hash, and import time. The hash/locator/format tuple
is unique, so repeating an unchanged export returns an `unchanged` report without writing another
import. The parser is metadata-only: it preserves folder path, title, URL, `ADD_DATE`, and
`LAST_MODIFIED`, rejects executable URL schemes, and never performs a network request.

`bookmark_records` is the current source-faithful view keyed by URL and folder path. Each record
points to a searchable `text/x-bookmark` artifact and keeps its entry hash plus first import ID;
`bookmark_import_items` records every observed ordinal and whether it was imported, unchanged,
merged, or a duplicate-URL conflict. Bookmark artifact passages contain the title, URL, and folder
as an inspectable text anchor, while extraction metadata records the export locator and
`remote_fetch: false`. A changed export creates a new import and an immutable artifact version;
older versions remain available to the canonical store.

## Lexical index

passages_fts is an SQLite FTS5 external-content virtual table over passages. It indexes the passage
text with the unicode61 tokenizer and diacritic removal. Triggers mirror passage inserts, updates,
and deletes into the FTS5 table. The derived `passages_fts_vocab` and `passages_fts_instances`
projections expose tokenizer terms and indexed document IDs for health checks; all three projections
are rebuildable.

The FTS5 row is not an independent source of truth. Search joins it back to canonical passages,
versions, artifacts, and active locators before returning a result. BM25 rank is ordering data, not
evidence identity. Match display is projected from canonical passage text into structured
highlighted/unhighlighted segments, so characters in source text cannot be interpreted as formatting
instructions. See the [SQLite FTS5 reference](https://www.sqlite.org/fts5.html).

`fts_health` reports canonical passage count/digest, indexed-document count, expected vocabulary
digest, actual vocabulary digest, and the SQLite FTS5 integrity result. `repair_fts` reports the
before/after health objects after a transactional rebuild. Neither operation changes passages,
versions, anchors, or source identity.

## Semantic derivative

The semantic tables are derived and optional. semantic_index_meta records the one active provider
manifest and an ordered BLAKE3 digest of active passage IDs plus passage hashes. The manifest binds
provider/model identity, the tokenizer contract, vector dimensions and normalization, canonical
build parameters, and the index revision. Each row in semantic_embeddings binds one encoded vector
to the same passage hash and to every manifest compatibility field. The current provider is the
deterministic local loom.hash-embedding baseline; its 128 little-endian f32 values are L2-normalized.
The provider
benchmark also measures character n-gram and token-count candidates, but does not claim semantic
quality.

semantic-rebuild deletes and recreates only the two derivative tables from active canonical
passages. semantic-status reports stale, incomplete, incompatible, or unbuilt state, and
semantic-search refuses to search unless the manifest is healthy. Candidates include an artifact
ID, version ID, passage ID, source hash, passage hash, and structured text/page/region anchor, so a
semantic score cannot become unsupported evidence. semantic-drop is a lossless derivative purge;
canonical rows and lexical retrieval remain intact.

## Search result contract

A search result contains:

- rank and score;
- artifact, version, and passage identifiers;
- title, media type, and source URI;
- content hash;
- a structured source-text excerpt with explicit highlighted segments;
- the exact text anchor;
- a short match reason.

The UI uses the locator and anchor to show where the evidence came from. The evidence viewer sends
the result's artifact ID, version ID, passage ID, and content hash to `resolve_evidence`; the core
checks that the active record still matches, rereads the source through the stable-read path, and
returns the canonical passage plus its page/region anchor and extractor metadata. Missing or changed
bytes return an explicit stale-source error rather than showing different content under an old
excerpt. `open_artifact` uses the same verified tuple before handing the original path to the host.

## Lifecycle states

The schema allows active, missing, and tombstoned artifact states and ready, failed, and superseded
version states. A complete directory rescan marks deleted, unsupported, or failed-read prior
artifacts missing and excludes them from retrieval. Stored historical text is not purged; secure
erasure and a user-facing retention policy are not implemented.

## Compatibility

The schema is currently version 7. LOOM validates the expected shape before changing a known
database, refuses missing or unknown version markers, and fails malformed versions with a named
reason. Version 2 databases migrate transactionally by adding the `index_jobs` table, relationship
envelope defaults, bookmark tables, and recording version 7; the populated fixture and preservation
checks are
documented in [SCHEMA_COMPATIBILITY.md](SCHEMA_COMPATIBILITY.md). Pre-alpha version 1 databases are
rejected
because their content-version uniqueness contract omitted extractor identity; users of that
unpublished format must rebuild the local index from source files. Changes to identity, content
hashing, anchors, or FTS5 maintenance require an ordered migration plan, updated fixtures, and an
ADR. Future semantic indexes must reference canonical artifact versions and be rebuildable rather
than becoming a second authority.

## Durable indexing jobs

`index_jobs` is a checkpoint, not a second source of truth. One row is keyed by the selected source
root and canonical selection locator. The discovery fingerprint binds the checkpoint to the sorted
set of paths observed at job start; a changed selection resets progress rather than skipping newly
discovered work. Each supported ingestion unit advances `next_unit` in the same SQLite transaction
that creates or reuses its artifact version and passages. Unsupported or unreadable units advance
through a small reconciliation transaction and retain an explicit failure in the returned report.

The indexing report exposes the durable job ID as `run_id` and separates `attempted`, `indexed`,
`unchanged`, `skipped`, `failed`, and `cancelled` counts. A local cancellation token is checked only
between units. When cancellation is observed, the job remains `interrupted` with the explicit
`cancelled by request` reason; complete canonical versions and the next-unit checkpoint are kept so
the next invocation resumes from the exact boundary.

An interrupted or still-running row resumes from its durable `next_unit` when the fingerprint and
unit count match. Completion records `state = completed` and the next run starts a fresh scan, so
retries of unchanged bytes reuse the existing content version. The row is intentionally diagnostic
and rebuildable; canonical artifact, version, passage, and hash records remain authoritative. A
cancelled report identifies the same job ID and counts the remaining units as resumable; cancellation
never rolls back a previously committed complete unit.

## Observation hints and reconciliation

Observation events are bounded hints with an explicit `created`, `modified`, `removed`, `renamed`,
or `overflow` kind. The coalescer rejects relative paths, paths outside the enabled root, and
symlink resolutions that escape it. Duplicate changes collapse deterministically; rename records
both the previous removal and the new path, while overflow or an oversized batch requests a full
rescan. The reconciler then scans the enabled root and rechecks content hashes, so an event stream
cannot create, delete, or rename canonical evidence by assertion alone.

Enabled source roots are persisted in `source_roots`. On desktop startup the bounded observation
command reconciles those roots again; missing or revoked roots return explicit failures and never
fall back to a broader directory. A native event adapter may optimize the hint source later, but
the content-hash scan remains the correctness boundary.

## Persisted source scopes

`source_roots` stores the exact canonical locator selected by the user, its file/directory kind,
enabled state, and timestamps. The desktop exposes a derived availability status without storing a
write capability: available, missing, denied, wrong type, unsafe symlink, unavailable, or revoked.
The current direct-distribution build uses explicit re-selection through the native picker rather
than claiming a macOS security-scoped bookmark. Revocation disables future reconciliation and marks
the root's active artifacts missing so they are not searchable or openable; canonical historical
rows remain for a future retention/export policy. Re-selection is the only path that re-enables a
revoked locator.

## References

- [SQLite FTS5](https://www.sqlite.org/fts5.html)
- [W3C PROV-O](https://www.w3.org/TR/prov-o/) for provenance vocabulary that may inform future
  relationship records.
