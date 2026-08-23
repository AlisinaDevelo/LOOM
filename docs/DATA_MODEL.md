# Data model

This document describes the schema currently created by LOOM schema version 2. It is an
implementation contract for the pre-alpha slice, not a stable migration guarantee.

## Identity and source records

| Table             | Purpose                                   | Important fields                                                                          |
| ----------------- | ----------------------------------------- | ----------------------------------------------------------------------------------------- |
| schema_meta       | Records the schema version                | key, value                                                                                |
| source_roots      | An explicitly selected file or directory  | kind, unique locator, enabled, timestamps                                                 |
| artifacts         | A logical source under a root             | title, media_type, state, active_version_id                                               |
| artifact_locators | Resolves an artifact to a source location | kind, locator, active, first/last seen timestamps                                         |
| artifact_versions | Immutable content observations            | content_hash, hash_algorithm, byte_size, source mtime, extractor identity/version, status |
| passages          | Normalized text and exact anchors         | artifact version, ordinal, text, text hash, JSON locator, character/line offsets          |
| relationships     | Reserved source-to-source relationships   | source/target artifacts, optional evidence passage, method, confidence                    |

The current extractor creates file locators. The schema also names URL and managed-copy locator
kinds for future work; they are not part of the current supported input path.

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
- locator_json stores the serialized text anchor;
- char_start and char_end are character offsets;
- line_start and line_end are one-based line offsets;
- created_at records insertion time.

The current segmenter targets 1,000 characters and overlaps adjacent passages by 120 characters.
Offsets are calculated without splitting Unicode scalar boundaries. The source content hash is
BLAKE3 over the bytes read before line-ending normalization; the passage offsets therefore apply to
the normalized text stored in the passage.

## Lexical index

passages_fts is an SQLite FTS5 external-content virtual table over passages. It indexes the passage
text with the unicode61 tokenizer and diacritic removal. Triggers mirror passage inserts, updates,
and deletes into the FTS5 table.

The FTS5 row is not an independent source of truth. Search joins it back to canonical passages,
versions, artifacts, and active locators before returning a result. BM25 rank is ordering data, not
evidence identity. Match display is projected from canonical passage text into structured
highlighted/unhighlighted segments, so characters in source text cannot be interpreted as formatting
instructions. See the [SQLite FTS5 reference](https://www.sqlite.org/fts5.html).

## Search result contract

A search result contains:

- rank and score;
- artifact, version, and passage identifiers;
- title, media type, and source URI;
- content hash;
- a structured source-text excerpt with explicit highlighted segments;
- the exact text anchor;
- a short match reason.

The UI uses the locator and anchor to show where the evidence came from. Opening sends the result's
artifact ID, version ID, and content hash back to the core; the core checks that the active record
still matches and rereads the source through the stable-read path. Missing or changed bytes return
an explicit stale-source error rather than opening different content under an old excerpt.

## Lifecycle states

The schema allows active, missing, and tombstoned artifact states and ready, failed, and superseded
version states. A complete directory rescan marks deleted, unsupported, or failed-read prior
artifacts missing and excludes them from retrieval. Stored historical text is not purged; secure
erasure and a user-facing retention policy are not implemented.

## Compatibility

The schema is currently version 2. LOOM refuses missing, malformed, or unknown version markers; it
does not silently rewrite them. Pre-alpha version 1 databases are rejected because their
content-version uniqueness contract omitted extractor identity; users of that unpublished format
must rebuild the local index from source files. Changes to identity, content hashing, anchors, or
FTS5 maintenance require an ordered migration plan, updated fixtures, and an ADR. Future semantic
indexes must reference canonical artifact versions and be rebuildable rather than becoming a
second authority.

## References

- [SQLite FTS5](https://www.sqlite.org/fts5.html)
- [W3C PROV-O](https://www.w3.org/TR/prov-o/) for provenance vocabulary that may inform future
  relationship records.
