# ADR 0008: multimodal retrieval evaluation contract

Status: accepted for the v0.2 Needle evaluation work

## Decision

Keep the v0 text smoke benchmark as a backward-compatible regression and add a
versioned v1 benchmark for the supported evidence classes: local text, PDF pages,
saved-web records, cropped screenshot/OCR regions, and a rasterized scanned-page
stand-in. A benchmark query is a
known-item recovery task. It names the expected artifact, an exact evidence anchor,
and, where appropriate, an acceptable exact duplicate. A negative query is successful
only when the search returns no result.

The CLI validates fixture bytes, content hashes, extractor identity/version, passage
hashes, anchor geometry, query count, and manifest schema before indexing. The v1
report must retain overall and per-source-class results for exact-source Recall@1/5,
MRR, anchor precision, false-positive rate, negative no-result rate, reformulation
success, index completeness, latency, and index-cost measurements. Failures are
classified by source class and stage rather than collapsed into one quality score.

## Evidence policy

Every positive result must resolve to the expected artifact (or a declared exact
duplicate) and one matching text, page, or image-region anchor. A report may pass its
declared planning thresholds while still exposing a source-class failure; the known
failure list is part of the evidence. Thresholds are measurement hypotheses and do
not imply real-world quality.

The v1 screenshot fixture is a crop of the synthetic OCR fixture's non-background
bounding box. The crop is stored as a rights-clean test asset so screenshot checks do
not retain unrelated pixels. A second cropped raster fixture represents a scanned page
without claiming a scanned-PDF OCR implementation. Saved-web coverage is currently a
Markdown stand-in with URL, capture timestamp, and snapshot status; it does not claim
HTML archiving parity.

## Consequences

The benchmark can catch regressions in evidence anchors and indexing completeness
without uploading corpus content. It also makes the current gaps visible: lexical
retrieval may miss a paraphrase and may return exact duplicates for a hard negative.
Those are tracked ranking and query-planning problems, not reasons to weaken the
fixture or remove the negative. New media classes require a fixture, an anchor
schema, a source-class breakdown, and a reproducible target-device run.
