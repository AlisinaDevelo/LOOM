# LOOM Research

Research snapshot: 2026-08-23. This is a public-source review of the retrieval,
knowledge-management, screenshot-memory, and local-AI landscape relevant to LOOM.

## How to read this memo

**Evidence** means an official product page, vendor documentation, source repository,
operating-system documentation, or research paper. A vendor page establishes public positioning and
described capability; it is not an independent quality benchmark.

**Inference** means a product or roadmap decision derived from that evidence. Inferences are labeled
so they can be revisited as the market changes.

## Market map

| Surface                           | Representative primary sources                                                                                                                                                                                                                | What the evidence says                                                                                                                         | Implication for LOOM                                                                                                                         |
| --------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------- |
| Operating-system baseline         | [Spotlight](https://support.apple.com/en-gb/guide/imac/apd10f8d1038/mac), [Live Text](https://support.apple.com/en-gb/guide/mac-help/mchl4d69efd3/mac)                                                                                        | macOS already searches documents, images, and bookmarks, and can expose text inside images                                                     | **Inference:** “search my Mac” is not enough positioning. LOOM must make exact-source recovery and evidence inspection obvious               |
| Mature local knowledge managers   | [DEVONthink](https://www.devontechnologies.com/apps/devonthink), [Keep It](https://reinventedsoftware.com/keepit/), [Obsidian storage](https://obsidian.md/help/data-storage) and [Search](https://obsidian.md/help/Plugins/Search)           | Local databases, OCR, full-text search, links, versions, or Markdown vault workflows are established expectations                              | **Inference:** fast search, selected libraries, durable local data, and openable originals are table stakes                                  |
| Web and reading libraries         | [Raindrop](https://raindrop.io/?lang=en_gb), [Raindrop full-text and permanent library](https://blog.raindrop.io/full-text-search-permanent-library-and-more/), [Readwise Reader search](https://docs.readwise.io/reader/docs/faqs/searching) | Web pages, PDFs, highlights, full text, and saved copies are packaged as a reading workflow; archive success has limits                        | **Inference:** URLs and snapshots need explicit status and provenance. LOOM should never imply that a web copy is guaranteed                 |
| Research and document archives    | [Zotero search](https://www.zotero.org/support/preferences/search), [Paperless-ngx usage](https://github.com/paperless-ngx/paperless-ngx/blob/dev/docs/usage.md)                                                                              | PDF/EPUB/HTML/text extraction, OCR-oriented workflows, original files, versions, checksums, and metadata are proven use cases                  | **Inference:** source identity, extraction version, and reproducible evidence are more valuable than a chat wrapper                          |
| Local semantic and AI file search | [Capd](https://capd.jxd.dev/), [Bookmarker](https://bookmarker.cc/), [Fenn](https://www.usefenn.com/), [Index](https://www.index-app.com/), [SKRY](https://apps.apple.com/us/app/skry-private-ai-file-search/id6758132924?mt=12)              | Newer Mac products combine local files, screenshots, OCR, semantic search, citations, or on-device models; positioning and pricing vary widely | **Inference:** “private local AI search” is a crowded claim. LOOM needs a narrower, measurable exact-source contract                         |
| Screenshot and ambient memory     | [Memento Capture](https://mementocapture.com/), [Screenpipe](https://screenpipe.com/), [Screenpipe search docs](https://docs.screenpipe.com/search-screen-history), [OpenRecall](https://github.com/openrecall/openrecall)                    | Screen history, OCR, app/time context, APIs, and local search are available, with materially different capture and privacy models              | **Inference:** intentional capture and clear retention controls should precede any bounded passive mode                                      |
| Research direction                | [ScreenTrack](https://arxiv.org/abs/2001.10898), [Scrapbook](https://arxiv.org/abs/2209.12318), [omni-macos](https://arxiv.org/abs/2608.05543)                                                                                                | Studies and recent systems explore visual-history retrieval, screenshot bookmarks, metadata, and on-device multimodal search                   | **Inference:** the problem is credible, but LOOM must publish task-level recovery measurements rather than borrow general “AI memory” claims |

The market also includes Apple-adjacent tools such as
[Raycast File Search](https://manual.raycast.com/file-search), and screenshot-focused products such
as [Mirowl](https://www.mirowl.com/), [Magnifind](https://magnifind.app/),
[Gisti](https://gisti.app/), and [Screengrep](https://www.screengrep.com/). Their public positioning
reinforces the category overlap; it does not establish comparative retrieval quality.

One reliability signal is [Rewind’s shutdown notice](https://rewind.ai/what-happened-to-rewind/),
which says the product was discontinued after the Meta acquisition. **Inference:** portable local
records and export are product requirements, not optional goodwill.

## Table stakes

The following are supported by the market evidence above or by macOS platform constraints:

- Selected folders and incremental indexing with visible progress, failures, limits, and rebuild
  controls.
- Keyword search that is fast and predictable, with optional semantic ranking rather than
  semantic-only behavior.
- OCR and document extraction where the result can point back to the image, page, or source region.
- A one-step path from result to original path, URL, or clearly labeled snapshot.
- Local-first operation, explainable permissions, and retention/deletion controls.
- Stable metadata, duplicate/version handling, and export or backup of the canonical library.
- A macOS distribution path that respects
  [sandbox file access](https://developer.apple.com/documentation/security/accessing-files-from-the-macos-app-sandbox),
  [TCC privacy controls](https://support.apple.com/en-ie/guide/security/secddd1d86a6/web),
  [Vision text recognition](https://developer.apple.com/documentation/vision/recognizing-text-in-images),
  and
  [notarization](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution).

These are market expectations, not a claim that every competitor implements every item well.

## Potential differentiation

This is LOOM’s proposed wedge, and therefore an **inference to validate**, not a defensible moat
today:

1. **Evidence-first retrieval contract.** Every useful result carries an artifact/version identity,
   hash, source locator, extractor or OCR provenance, and a precise anchor. “No evidence, no answer”
   is the default behavior.
2. **A provenance graph across source forms.** A screenshot, browser URL, saved page, PDF, moved
   file, and note can be related without pretending they are the same object. Version and duplicate
   families become navigable.
3. **Intentional memory rather than surveillance.** The user can capture a source at the moment it
   matters, with optional retention and exclusions, before LOOM considers passive screen history.
4. **A public recovery benchmark.** Exact top-k recovery, evidence-open success, index completeness,
   false positives, latency, and resource use are more meaningful than a generic chatbot demo.
5. **Portable local records.** Canonical metadata and source references remain inspectable and
   exportable even when a model, connector, or vendor disappears.

The defensibility test is repeated use plus better held-out recovery, not the existence of a novel
embedding model.

## Early-user research and outreach

Start with 12–20 Mac users across developers, technical writers, researchers, and graduate students.
Ask each participant to log two weeks of real “where did I see that?” tasks, using a privacy-safe
corpus of roughly 30–50 representative artifacts where possible. Add a small second cohort of
lawyers, journalists, analysts, or designers only after the source-handling and permission model is
stable.

Recruit through personal professional networks, research and developer communities, and one-to-one
design-partner conversations. The ask is a bounded diary and task session, not access to private
screen history.

Record locally or with explicit consent:

- exact-source Recall@1/5/10 and reciprocal rank;
- time from query to correct source;
- evidence-open and anchor success;
- false-positive and “no result” rates;
- index completeness and extraction failures;
- capture/setup friction;
- CPU, memory, disk, battery, and rebuild cost;
- whether visible provenance changes trust or correction behavior.

Do not use private user content in the public benchmark. Publish synthetic or rights-clean fixtures
and the evaluation procedure instead.

## Major risks and roadmap consequences

| Risk                                             | Why it matters                                                            | Consequence                                                                                               |
| ------------------------------------------------ | ------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------- |
| Apple commoditization                            | Spotlight and Live Text already cover broad discovery                     | Ship a source-and-evidence workflow, not another global search box                                        |
| Feature convergence and crowded pricing          | Local AI, OCR, screenshot, and semantic search claims are widespread      | Narrow the promise; publish recovery evidence; avoid feature-count marketing                              |
| Cold start and capture friction                  | A library is only useful when the needed source entered it                | Begin with explicit import and high-value connectors; measure missed-source causes before passive capture |
| TCC, sandbox, and permission failure             | macOS privacy boundaries can make indexing incomplete or surprising       | Make scope, denial, retry, and deletion visible; notarize and threat-model before broad distribution      |
| Secrets in screenshots and copied text           | Personal archives can contain credentials, tokens, health, or client data | Local-only defaults, exclusions, retention controls, redaction experiments, and no hidden telemetry       |
| OCR, layout, multilingual, and web-archive error | A text hit can point to the wrong page or stale copy                      | Store extractor/version/confidence and anchor type; label snapshots and low-confidence results            |
| Source identity drift                            | Files move, URLs change, and multiple captures may represent one work     | Content hashes plus version/duplicate/provenance relationships are core schema, not post-processing       |
| Sync and vendor dependence                       | Cloud sync expands attack surface; products can disappear                 | Keep local canonical data first; add encrypted backup/sync only after restore and threat-model gates      |
| Liability and over-trust                         | “Citation” can be mistaken for legal or forensic proof                    | Avoid completeness claims; preserve uncertainty and exact source context                                  |

## Research decisions to carry into issues

- Use SQLite/FTS5-style lexical retrieval as the dependable base; add embeddings only when an
  evaluation shows a real recovery gain.
- Treat screenshot, PDF, and browser support as evidence-anchor work, not merely new parsers.
- Do not make always-on capture a prerequisite for the first release.
- Treat every connector and extractor as a permissioned, versioned boundary.
- Re-run this market review at each major release; vendor pages and platform behavior change.
- Re-test adoption, willingness to pay, support burden, connector maintenance, and optional-service
  cost before mobile, cross-platform, relay, or professional expansion. A documented stop decision
  is a successful research result when demand or economics do not justify the risk.
