# LOOM query syntax

LOOM keeps the query language independent from SQLite and FTS5. Free text is matched
lexically; quoted text is treated as one exact phrase. The following typed filters can
be appended to the same query:

| Filter | Example | Semantics |
| --- | --- | --- |
| \`after:\` | \`after:2026-01-01\` | Inclusive UTC source modification lower bound |
| \`before:\` | \`before:2026-02-01\` | Exclusive UTC source modification upper bound |
| \`type:\` | \`type:pdf\`, \`type:image\`, \`type:markdown\` | Source family; an exact MIME type such as \`image/png\` is also accepted |
| \`path:\` | \`path:"research notes"\` | Case-insensitive substring of the canonical source locator |
| \`confidence:\` | \`confidence:>=0.90\` | OCR/evidence confidence in the inclusive range \`0..1\`; text and PDF anchors have confidence \`1.0\`. OCR results also expose `confirmed` or `low_confidence`; no-readable-text inputs are indexing failures. |

RFC3339 timestamps are accepted in addition to \`YYYY-MM-DD\`, for example
\`after:2026-01-01T09:30:00Z\`. A filter may appear once per key. Queries must include
at least one free-text term; filter-only queries fail with a helpful error instead of
scanning the entire library.

Examples:

\`\`\`text
"retry anomalies" after:2026-01-01 type:markdown
terminal path:"bug report" before:2026-03-01
"OCR marker" type:image confidence:>=0.80
\`\`\`

Filters are applied to canonical metadata and evidence anchors before result ranking
and pagination. Semantic and hybrid retrieval apply the same filter plan before adding
secondary candidates, so an excluded artifact cannot re-enter through another ranker.
Every returned lexical result includes lexical, semantic, metadata, and reranker
contribution fields; inactive stages are represented as zero.

The parser quotes all free-text terms before passing them to FTS5. Storage-engine
operators, SQL fragments, malformed dates, unsupported source types, invalid
confidence values, unmatched quotes, and unsafe escapes are rejected as typed query
errors.
