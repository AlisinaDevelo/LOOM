# ADR 0012: Make retention and deletion explicit, inspectable, and bounded

## Context

LOOM stores canonical passages alongside rebuildable FTS, OCR, semantic, and SQLite sidecar state.
Revocation hides a source from retrieval but is not deletion, and a user needs to know what remains
before deciding whether to remove it. Broad recursive cleanup would risk touching unrelated local
data.

## Decision

- Retain the opened database path and inspect only canonical source estimates, SQLite sidecars,
  captures, and a fixed allowlist of disposable directories.
- Expose explicit artifact, exact-root, and RFC3339-cutoff deletion operations. Each operation uses
  a transaction, cascades canonical/derived rows, rebuilds FTS5, checkpoints/vacuums SQLite, and
  returns counts and paths removed from disposable storage.
- Store retention as an inert `schema_meta.retention_days` policy. Applying it is a separate,
  visible operation and uses a deterministic cutoff for testability.
- Never follow symlinks, scan unknown sibling directories, or delete user-owned source files as a
  side effect of disposable cleanup.

## Consequences

Users can inspect approximate source/derived/disposable storage and recover from accidental scope
selection without relying on a hidden background policy. Deletion remains local and auditable, but
it is not a cryptographic secure-erasure promise for SSD snapshots, backups, swap, or source files
outside LOOM's managed capture surface. Future encrypted backup/sync must preserve these explicit
states and extend the same evidence contract.
