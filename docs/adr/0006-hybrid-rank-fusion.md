# ADR 0006: evidence-bound hybrid rank fusion

Status: experimental, evaluation-gated

## Decision

LOOM keeps lexical FTS5 search as the default desktop path and evaluates hybrid ranking as a
separate, local-only derivative. `hybrid-rank-v1` uses weighted reciprocal-rank fusion with a
constant of 60 and these preregistered weights:

| Signal | Weight | Definition |
| --- | ---: | --- |
| Lexical rank | 0.45 | `1 / (60 + lexical_rank)` when FTS5 returns the passage |
| Semantic rank | 0.35 | `1 / (60 + semantic_rank)` when the healthy semantic index returns the passage |
| Exact match | 0.10 | Normalized query phrase occurs in passage, title, or source URI |
| Path overlap | 0.05 | Fraction of distinct query tokens present in title/source URI |
| Recency | 0.05 | Min–max normalized source modification timestamp within the candidate set |

The output retains the canonical artifact/version/passage tuple, original anchor and excerpt, and
all signal values. Ties resolve by passage ID. A missing or incompatible semantic derivative fails
closed; the ranker never silently changes the default lexical behavior.

## Promotion gate

The benchmark runner compares lexical-only, semantic-only, and hybrid retrieval on the same
rights-clean fixture. Hybrid promotion requires all of the following:

- manifest exact-source Recall@1/5, anchor precision, and false-positive thresholds;
- p95 hybrid query latency no greater than 1,000 ms on the target device;
- hybrid Recall@1 no lower than lexical-only Recall@1;
- no unanchored or source-mismatched result.

The gate is measured by `scripts/hybrid-ablation.py`; a failed gate exits non-zero and leaves the
desktop default unchanged. The v0 smoke corpus is intentionally small and is not a semantic-quality
claim.

## Consequences

Hybrid candidates can broaden recall but also introduce semantic false positives and coarse
passage anchors. Keeping the experiment separate makes those failures visible, preserves source
fidelity, and gives later benchmark work a stable contract for changing weights or adding a
reranker.
