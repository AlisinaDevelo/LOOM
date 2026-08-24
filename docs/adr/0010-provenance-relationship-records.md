# ADR 0010: Store provenance as typed source-backed relationships

- Status: Proposed
- Date: 2026-08-25

## Context

LOOM must recover the original source object and explain why related artifacts appear
together. A vector-only association cannot preserve a screenshot-to-page link, a
duplicate family, or a version transition with an inspectable evidence anchor. A graph
database would add a second canonical store before the local archive is reliable.

## Decision

Store relationships in SQLite beside canonical artifacts, versions, locators, and
passages. Each row has a typed `kind`, an independent relationship schema version,
an `origin` (`observed`, `inferred`, or `user_confirmed`), a method, optional passage
evidence, bounded confidence, JSON object metadata, and a creation timestamp. The
identity tuple `(source, target, kind, origin, method)` is idempotent. Unknown kinds
are preserved as opaque strings so older readers can display or safely ignore future
connectors without rewriting the row.

The read path returns bounded endpoint projections (active locator, version, hash, and
state) through a Tauri command. It does not require a graph database and never renders
untrusted metadata as markup.

## Consequences

- Provenance remains portable in the SQLite archive and export format.
- Inferred edges must include both passage evidence and confidence; user-confirmed
  edges remain distinguishable.
- Relationship rows are source-backed and cascade when an endpoint is purged.
- Future graph traversal can be built over this contract without changing canonical
  artifact identity.
- The `relationships` table requires the v6 migration envelope; v2–v5 databases gain
  explicit defaults transactionally.

## Rejected alternatives

- A separate graph database: rejected until local export/restore and bounded traversal
  are reliable.
- Embedding-only links: rejected because they cannot provide a stable evidence anchor
  or preserve source lineage.
