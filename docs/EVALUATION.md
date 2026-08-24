# Evaluation

LOOM is pre-alpha. Evaluation currently verifies the exact-source path and basic regression
behavior; it does not establish real-world retrieval quality.

The v0.1 activation decision contract is [ACTIVATION_GATE.md](ACTIVATION_GATE.md). Its thresholds
are hypotheses until a held-out rights-clean benchmark and a consented 12–20 participant study
produce retained evidence.

## Current smoke fixture

The fixture in benchmarks/retrieval/v0/ contains synthetic, rights-clean text and Markdown files,
three JSONL queries, exact character/line anchors, and expected source filenames. Its manifest
identifies the corpus as CC0-licensed and synthetic and records every fixture's raw content hash,
extractor identity/version, passage hash, and passage anchor. No private user content belongs in
the fixture.

Run it with:

```text
cargo run --locked -p loom-cli -- benchmark \
  --corpus benchmarks/retrieval/v0/corpus \
  --queries benchmarks/retrieval/v0/queries.jsonl
```

The CLI reports exact-source Recall@1 and Recall@5, anchor precision, false-positive rate, median
and p95 latency, index completeness, and failures. The command exits with status 2 when an expected
source is not at rank one, is not in the top five, has an incorrect exact anchor, or the index has
failures. Before querying, it also fails on a manifest/query-count mismatch, changed fixture bytes,
an unexpected extractor projection, or incomplete indexing. The current gate is therefore source
and anchor recovery plus fixture reproducibility, not a target latency or quality score.

## Regression checks

The current checkout has passed:

- cargo test --workspace --locked, including ingestion offset, unsupported-file, search
  sanitization, and versioned source recovery tests;
- npm run check, including lint, Vitest, TypeScript, and the Vite production build.

The current synthetic fixture run indexed 3 files with no failures and recovered all 3 expected
sources at rank one and in the top five, with anchor precision 1.0. Its latency output is a
measurement of this local run, not a performance target.

These checks are observations of the current checkout, not a promise that every environment or
future commit will pass.

## Protocol

When changing ingestion, segmentation, ranking, or evidence rendering:

1. Add or update a rights-clean fixture and document its provenance.
2. State the expected source file and, when relevant, the expected passage region.
3. Run the unit, UI, and smoke checks that are relevant to the change.
4. Report failures, skipped files, unsupported input, and environment limitations.
5. Keep tuning queries and parameters separate from any future held-out evaluation set.

Future evaluation may add graded passage or region judgments, Recall@10, MRR, nDCG, p95 latency,
index completeness, and evidence-open success. Those metrics are not implemented by the current
fixture contract; the activation gate defines them as explicit measurement targets rather than
claims about the current smoke fixture.

## References

- [SQLite FTS5](https://www.sqlite.org/fts5.html), including tokenizer and BM25 ranking behavior.
- [CITATION.cff](../CITATION.cff) for software citation metadata.
