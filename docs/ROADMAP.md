# LOOM Roadmap

Roadmap window: 2026-08-23 through 2029-08-23.

This document describes product intent.
[GitHub Issues](https://github.com/AlisinaDevelo/LOOM/issues) are the execution truth: each roadmap
issue carries acceptance criteria, milestone, priority, native parent/dependency relationships, and
required closure evidence.

## Release sequence

| Milestone         | Window                  | Focus                                                                                                                                                                                                                                                  | Exit gate                                                                                                                                                                                                            |
| ----------------- | ----------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **v0.1 Thread**   | 2026-08-23 – 2026-11-30 | Harden the exact-text recovery foundation: canonical SQLite records, BLAKE3 identity, explicit text/Markdown ingestion, FTS5 queries, text/line anchors, evidence result cards, deterministic fixtures, and visible index failures                     | A clean fixture returns the expected source and anchor; rebuilds are deterministic; source opening works; unsupported or incomplete data is disclosed rather than presented as a match                               |
| **v0.2 Needle**   | 2026-12-01 – 2027-04-30 | Add exact-source multimodal retrieval: deterministic PDF extraction, private image OCR, page/region evidence, intentional screenshot capture, derived semantic candidates, hybrid ranking, safe filters, accessibility, and a source-class benchmark   | Held-out local-text/PDF/image fixtures recover the expected source and anchor; low-confidence OCR and unsupported layouts are visible; 100k-artifact performance and resource measurements meet a preregistered gate |
| **v0.3 Pattern**  | 2027-05-01 – 2027-10-31 | Add intentional browser capture, explicit-save extension workflows, artifact provenance, bookmark imports, exclusions/retention, portable encrypted export, and the first design-partner alpha                                                         | Live URLs and best-effort snapshots are clearly distinguished; provenance and deletion survive restart/export; pilot metrics separate capture, missing-index, wrong-source, and evidence-viewer failures             |
| **v1.0 Weave**    | 2027-11-01 – 2028-05-31 | Make LOOM a dependable Mac daily driver: durable jobs, version/duplicate lineage, private ranking, extractor SDK, read-only API/MCP, Mac integrations, recovery, signed/notarized distribution, and independent security review                        | Local export/restore and upgrade drills pass; v0 gates remain green; no critical/high security issue remains; a signed build meets the documented quality and retention gate                                         |
| **v1.x Tapestry** | 2028-06-01 – 2029-01-31 | Expand only behind evidence: reviewed optional E2EE sync, an iOS capture companion, measured local model packs, professional connectors, and bounded-capture or obligation experiments                                                                 | Sync/key recovery and connector isolation pass adversarial tests; local/offline search remains complete; experiments advance only through preregistered retained-use, accuracy, privacy, and cost gates              |
| **v2.0 Fabric**   | 2029-02-01 – 2029-08-23 | Build a portable ecosystem: measured Windows/Linux parity, explicitly bounded shared collections, a signed extension registry, evidence-grounded synthesis, provenance timelines/diffs, professional-mode evaluation, and a public benchmark/HCI study | Every generated claim links to valid evidence or abstains; lineage survives export/import; extension and sharing boundaries pass independent review; the three-year strategy review records continue/stop decisions  |

Dates are planning windows, not promises. A gate can stop, narrow, or reorder the next milestone.

## Dependency chain

Thread establishes canonical identity and evidence → Needle proves reliable indexing and user value
→ Pattern adds new source forms and anchors → Weave connects versions and libraries → Tapestry
exposes carefully bounded integrations and backup → Fabric earns the right to synthesize across
verified evidence.

The dependency is intentional. Embeddings, background capture, sync, and generated answers should
not become substitutes for a stable source identity or a measurable recovery baseline.

## Issue themes

### v0.1–v0.2: prove the contract and exact-source breadth

- Canonical artifact, version, passage, locator, and failure schema.
- Explicit selected-root and drag-and-drop ingestion with deterministic hashing.
- Search parser, ranking baseline, result/evidence presentation, and source opening.
- Index health, retry, rebuild, limits, reconciliation, and interruption behavior.
- Deterministic PDF and image/OCR extraction with page/region anchors and intentional screenshot
  capture.
- Derived semantic candidates, evaluated hybrid ranking, safe source/time filters, and accessible
  evidence inspection.
- Rights-clean, per-source retrieval fixtures, hard negatives, and a resource-aware evaluation
  harness.

### v0.3–v1.0: prove provenance and daily reliability

- Browser URL capture and clearly labeled best-effort local snapshots.
- Duplicate, moved-file, and version lineage.
- Obsidian, Zotero, and Raindrop importers with scoped permissions.
- Exclusions, retention, post-restart deletion, export/import, and restore drills.
- Durable jobs, private ranking, extractor SDK, read-only API/MCP, Mac integrations,
  signed/notarized release, and upgrade tests.

### v1.x–v2.0: earn optional scale

- Multilingual and local semantic ranking only when the benchmark justifies it.
- Optional E2EE sync and iOS capture only after reviewed key-recovery and restore drills.
- Opt-in bounded capture or passive-source import only after intentional-capture failure is
  measured.
- Evidence-linked synthesis, timelines, and diffs.
- Signed extension registry, platform parity, and a public community benchmark/study.
- Shared, team, or regulated workflows only as separately gated experiments.

Each issue should state whether it changes the canonical record format, accesses new user data,
introduces a model or external service, or changes retention. That makes dependencies and review
obligations visible before implementation.

## Global non-goals

Across all milestones, LOOM will not make cloud access mandatory, default to 24/7 surveillance, ship
a generic chat-first interface, silently rename or delete sources, or claim legal/forensic
completeness. Broad format support is not a goal by itself; a format enters the roadmap only when
LOOM can preserve its source identity and show trustworthy evidence.
