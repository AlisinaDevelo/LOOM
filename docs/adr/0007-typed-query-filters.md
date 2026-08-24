# ADR 0007: typed, evidence-safe query filters

Status: accepted for the v0.2 Needle retrieval path

## Decision

LOOM accepts free text plus a small typed filter vocabulary:
\`after:\`, \`before:\`, \`type:\`, \`path:\`, and \`confidence:\`. Quoted text is an
exact phrase. The parser produces a storage-engine-independent \`ParsedQuery\`
containing a sanitized FTS expression and typed \`QueryFilters\`; it never accepts
raw SQLite operators or SQL fragments.

Date-only values are UTC midnights. \`after\` is inclusive and \`before\` is
exclusive. Source families map to canonical media types, paths are
case-insensitive locator substrings, and confidence is a bounded \`0..1\` value
whose image value comes from the OCR anchor. Text and PDF anchors have confidence
\`1.0\` because their extraction contract does not expose a probabilistic OCR
score.

## Ordering and evidence

The lexical path loads all FTS candidates, applies every typed filter against the
canonical media type, source locator, source modification time, and parsed
anchor, then performs deterministic score ordering and limit truncation. Semantic
and hybrid paths apply the same filter plan before secondary candidates are
ranked or fused. A filtered-out artifact therefore cannot re-enter through a
different ranking channel.

Every public lexical and hybrid result carries four explicit contribution values:
lexical, semantic, metadata, and reranker. Inactive stages are zero. The
experimental hybrid ranker reports weighted lexical/semantic/metadata
contributions and keeps reranker at zero until a separately evaluated reranker
exists.

## Consequences

The syntax is safe to expose in the desktop command bar and CLI without teaching
users FTS5 grammar. Helpful typed errors reject malformed dates, unsupported
source types, duplicate filters, invalid confidence thresholds, unmatched quotes,
and unsafe escapes. Filter-only queries are rejected so a typo cannot trigger an
unbounded library scan. The parser and library tests cover Unicode, escaping,
injection-shaped terms, time/type/path/confidence combinations, stable pagination,
and hybrid exclusion invariants.
