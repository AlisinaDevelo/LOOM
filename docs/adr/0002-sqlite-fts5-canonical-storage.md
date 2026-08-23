# ADR 0002: SQLite and FTS5 as the canonical local store

- Status: Accepted
- Date: 2026-08-23

## Context

The first ingestion path needs durable source identity, content versions, passage anchors, and
lexical search without a service dependency. A second storage system would make it harder to explain
which record is authoritative.

## Decision

Use SQLite for canonical local records: source roots, artifacts, locators, versions, passages, and
future relationships. Use an external-content SQLite FTS5 table for lexical retrieval over canonical
passages. Maintain FTS5 through SQLite triggers and join search results back to canonical rows
before returning evidence.

Use BLAKE3 source hashes, immutable content observations, normalized passage text, and explicit
character/line anchors. The current connection uses foreign keys and WAL journaling. FTS5 ranking
orders candidates; structured source-derived segments and anchors drive evidence display.

## Consequences

Positive:

- One local database contains identity, evidence, and the lexical index.
- FTS5 provides a documented, offline query path.
- Rebuilding the derived FTS5 table does not change source identity or anchors.

Negative:

- SQLite remains a local file that requires backup and access-control care.
- FTS5 is lexical; it does not solve semantic similarity or OCR.
- Schema and trigger migrations must preserve the source-backed result contract.

## References

- [SQLite FTS5](https://www.sqlite.org/fts5.html)
- [SQLite WAL](https://www.sqlite.org/wal.html)
