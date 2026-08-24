# Retrieval benchmark v0

This tiny rights-clean fixture verifies the first truth path: explicit local ingestion, FTS5
retrieval, and exact source recovery. It is a smoke test, not evidence of real-world quality.

Run:

```bash
cargo run -p loom-cli -- benchmark \
  --corpus benchmarks/retrieval/v0/corpus \
  --queries benchmarks/retrieval/v0/queries.jsonl
```

Each JSONL record identifies a source class, expected source filename, exact character/line
evidence anchor, and an explicit `acceptable_alternatives` list (empty when the primary source is
the only valid answer). The runner validates the sibling manifest's query count, raw fixture
hashes, extractor identity/version, passage hashes, anchors, alternatives, and index completeness
before scoring search. The report includes exact-source Recall@1/5, anchor precision,
false-positive rate, index completeness, and median/p95 query latency, overall and by source class.
The manifest's v0 gate requires Recall@1 >= 1.0, Recall@5 >= 1.0, anchor precision >= 1.0,
false-positive rate <= 0.0, and index completeness >= 1.0; the command exits non-zero when any
threshold or source-backed query fails, so CI enforces the same gate without uploading corpus
content. This v0 corpus exercises only local text; later rights-clean revisions add PDF, screenshot,
and saved-web fixtures, hard negatives, graded qrels, MRR/nDCG, index-cost, and evidence-open
measurements. No private user content belongs in this corpus.
