# Architecture decision records

These records capture decisions that constrain the pre-alpha LOOM architecture. They are concise
and implementation-oriented; current code and the privacy/threat documents remain authoritative
for the supported slice.

| ADR | Decision | Status |
| --- | --- | --- |
| [0001](0001-local-first-evidence-first.md) | Keep the first path local and evidence-first | Accepted |
| [0002](0002-sqlite-fts5-canonical-storage.md) | Use SQLite plus FTS5 for canonical local storage and lexical retrieval | Accepted |
| [0003](0003-derived-semantic-index.md) | Treat a semantic index as optional, derived, and rebuildable | Accepted for future work |
| [0004](0004-mpl-2-license.md) | Use the Mozilla Public License 2.0 | Accepted |

New decisions should state context, the decision, consequences, and what is deliberately out of
scope. Superseding a decision should link the replacement record.
