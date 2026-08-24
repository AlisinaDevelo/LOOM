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
| [0005](0005-semantic-index-provider.md) | Use a deterministic local provider for the semantic-index contract | Accepted |
| [0009](0009-browser-capture-protocol.md) | Bound browser capture to explicit, authenticated saves | Proposed |
| [0010](0010-provenance-relationship-records.md) | Store provenance as typed source-backed relationships | Proposed |

New decisions should state context, the decision, consequences, and what is deliberately out of
scope. Superseding a decision should link the replacement record.
