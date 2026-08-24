# Retrieval benchmark v1

This is a small, synthetic, rights-clean benchmark for the multimodal retrieval
contract. It is a reproducibility fixture, not a claim about production-quality
retrieval or a representative user corpus.

The corpus contains seven deliberately boring artifacts:

- local Markdown and plain text, including an exact duplicate and a date query;
- a hard negative with overlapping vocabulary;
- a saved-web Markdown stand-in carrying a URL, capture time, and snapshot status;
- a two-page synthetic PDF with page anchors; and
- a cropped PNG screenshot and a rasterized synthetic scanned-page stand-in, each
  with two OCR regions.

The screenshot is derived from the repository's synthetic OCR fixture. Its original
non-background bounding box was `878x191` at `(x=84, y=170)`; `ocr-cropped.png` is
that text-bearing region only. The crop is intentional: it avoids retaining unrelated
canvas pixels while preserving the coordinates needed to check image evidence. The
fixture is visually checked on the target Mac and contains no private screen content.

Run the benchmark with:

```bash
cargo run --locked -p loom-cli -- benchmark \
  --corpus benchmarks/retrieval/v1/corpus \
  --queries benchmarks/retrieval/v1/queries.jsonl
```

Each query names a source class, expected artifact, and exact text/page/region anchor.
Positive queries may list an exact duplicate as an acceptable alternative. Negative
queries must return no result. One positive query also has a paraphrase reformulation;
the report scores whether the reformulation recovers the expected source and anchor.

The report includes, overall and by source class (local text, PDF, saved web,
screenshot, and scanned page):

- exact-source Recall@1 and Recall@5;
- mean reciprocal rank (MRR) and anchor precision;
- false-positive rate and negative no-result rate;
- reformulation success;
- index completeness and median/p95 query latency; and
- index elapsed time, source bytes read, database bytes, and database-to-source-byte
  amplification.

Failures are classified by source class and stage, including `no_results`,
`wrong_source`, `anchor_mismatch`, `false_positive`, `index_failure`, and
`reformulation_failed`. The baseline intentionally retains known failures instead of
changing queries or thresholds to hide them. In particular, the local-text slice
contains an ambiguous paraphrase and a hard negative, so its slice metrics may be
below the overall gate even when the command exits successfully under the declared
planning thresholds. Treat those failures as the next ranking work, not as evidence
that the problem is solved.

The manifest records raw content hashes, extractor identity/version, passage hashes,
anchors, thresholds, and the CC0/synthetic provenance statement. Re-running after a
fixture or extractor change must update those records deliberately; the validator
rejects silent drift.
