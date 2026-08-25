# LOOM Product

## The job

LOOM is a local-first macOS retrieval tool for recovering the exact original source object a person
remembers: a screenshot, document, page, bookmark, or file. Its unit of value is a successful
recovery with visible evidence and provenance, not a plausible generated answer.

The initial implementation is deliberately narrower than that long-term promise. It currently
indexes explicitly selected text, Markdown, bounded text-based PDF files, and images with local
macOS Vision OCR, stores canonical records in SQLite, assigns BLAKE3 content hashes, and searches
with SQLite FTS5. Image results retain provider/model metadata and oriented pixel-region anchors;
browser capture and cross-object provenance work described below is planned work, not current
support.

### User story

> I remember a phrase, visual detail, approximate time, or source—not the filename. Show me the
> original object, explain why it matched, and let me open the evidence.

A successful result should identify:

- the artifact and version;
- the original path or URL, when available;
- a stable content hash;
- the matching passage or, later, a page, region, timestamp, or other precise anchor;
- the excerpt and match reason;
- whether the result is an original, a preserved snapshot, or a derived representation.

If the index is incomplete or no evidence supports a result, LOOM should say so. It should not turn
an uncertain match into an authoritative summary.

## Portfolio boundary

- **Primary object:** an explicit local source artifact—file, document, screenshot, page, or
  bookmark—together with its stable version, content hash, and evidence anchor.
- **Primary question:** “Where is the exact thing I remember, and what source evidence makes this
  match trustworthy?”
- **Explicit non-goals:** LOOM is not an ambient event journal or continuous activity recorder,
  and it is not a TypeScript architecture analyzer. Its boundary is intentional local source
  retrieval and exact evidence viewing.

## Current validation decision

The 154-issue program is an option set, not a near-term build queue. After the current v0.3
foundation gates, LOOM's next product proof is the existing design-partner alpha (roadmap `0306`):
known-item recovery, evidence-open success, capture friction, privacy/storage cost, and repeated
use. Do not add roadmap scope or broaden into passive capture, sync, or connector breadth until
that study produces a continue, narrow, or stop decision.

## Who LOOM is for first

| Segment                                       | First job to solve                                                                            | Why now                                                                             |
| --------------------------------------------- | --------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------- |
| Developers and technical writers              | Recover a command, design note, issue, or reference seen across code, docs, and browser tabs  | Repeated context switching creates a concrete, testable recovery problem            |
| Researchers and graduate students             | Recover the exact paper passage, figure, PDF page, or saved web source behind a note          | Source identity and citation quality matter more than conversational breadth        |
| Lawyers, journalists, analysts, and designers | Recover the source artifact behind a decision, claim, visual reference, or client deliverable | Provenance and inspectability can justify a dedicated tool                          |
| General “where did I see that?” users         | Search screenshots, files, and bookmarks with little setup                                    | A later expansion after capture friction, privacy, and support costs are understood |

The first release is not aimed at replacing team knowledge bases, enterprise document management, or
general-purpose chat.

## Product principles

1. **Evidence before synthesis.** Find and open the source before offering an interpretation.
2. **Provenance is part of the result.** Path, URL, time, version, hash, and anchor are user-facing
   data, not hidden implementation details.
3. **Local and intentional by default.** The user chooses what enters the library. Continuous
   capture is an opt-in capability, never the default assumption.
4. **Canonical data, derived indexes.** Originals, metadata, hashes, and extraction records must
   remain portable and rebuildable; search indexes and embeddings are disposable derivatives.
5. **Graceful uncertainty.** No match, incomplete indexing, low-confidence OCR, and stale sources
   must be distinguishable from a confirmed recovery.
6. **Measure recovery, not eloquence.** Quality is evaluated with exact-source retrieval,
   evidence-open success, latency, completeness, and false-positive rates.
7. **Small, inspectable boundaries.** Importers and extractors must disclose their scope, version,
   limits, and data handling.

## Product loop

1. **Capture or import:** the user selects a folder, file, screenshot, PDF, URL, or supported
   library.
2. **Index:** LOOM records the source, version, hash, extractor, and evidence anchors; failures
   remain visible.
3. **Retrieve:** the user searches with remembered words, quoted fragments, metadata, or later
   semantic cues.
4. **Inspect:** the result shows the excerpt, anchor, source identity, and match explanation.
5. **Open or restore:** the user opens the original or a clearly labeled preserved snapshot.
6. **Correct:** user feedback improves ranking and identifies missing connectors without silently
   changing the source.

## Non-goals

- A generic chatbot or answer engine that hides its sources.
- Always-on screen, audio, clipboard, or notification surveillance as the default workflow.
- A cloud account that is required to search a local library.
- Autonomous renaming, deletion, rewriting, or “clean-up” of source files.
- A promise of legal, forensic, archival, or evidentiary completeness.
- Team collaboration, administrative controls, or enterprise ingestion before the local recovery
  contract is reliable.
- Supporting every file type before each supported type has deterministic extraction and an
  inspectable evidence anchor.

## Hypotheses and gates

The following are hypotheses to test, not claims that have already been validated. A GitHub Issue
for each pilot or release should record the threshold before measurement, the fixture or
participants, and the result.

| Hypothesis | Test | Gate |
| --- | --- | --- |
| Exact recovery is a recurring job worth a dedicated tool | A two-week diary with 12–20 Mac design partners, each contributing a privacy-safe task set | Participants repeatedly choose source recovery over a generic search or summary path, and report a meaningful reduction in time to the correct source |
| Visible evidence increases trust and correction speed | Compare result cards with and without source/anchor inspection in realistic tasks | Users can distinguish confirmed, incomplete, and low-confidence results; no critical misattribution remains unresolved |
| Intentional capture is sufficient for the first audience | Measure selected-folder, drag-and-drop, screenshot, and URL workflows before considering passive capture | Priority tasks remain recoverable without requiring default 24/7 recording; capture friction and missed-source causes are documented |
| A canonical local library can remain dependable | Rebuild, upgrade, and interruption tests on the same corpus | Stable identifiers and hashes survive rebuilds; failures are retryable and visible; no source data is lost |
| LOOM can earn a defensible quality lead | Maintain a rights-clean, held-out benchmark with exact-source labels and hard negatives | Report Recall@1/5/10, MRR or nDCG, evidence-open success, p95 latency, and index completeness before making comparative claims |
| Local privacy is a product advantage, not just a constraint | Threat-model file permissions, OCR, snapshots, logs, exports, and optional sync | No critical secret-leak or authorization defect; the app remains useful offline and its data boundaries are understandable |
| A sustainable product can preserve the local-first boundary | Test setup, repeated use, willingness to pay, support burden, and optional-service costs by segment | Each expansion has a preregistered continue, narrow, or stop rule; local search and export never require recurring cloud revenue |

The release gates in [ROADMAP.md](ROADMAP.md) are sequenced around these tests. Passing a gate
permits the next experiment; it does not guarantee market demand or a particular feature.
