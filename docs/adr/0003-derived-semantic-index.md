# ADR 0003: A semantic index is derived and optional

- Status: Accepted for future work
- Date: 2026-08-23

## Context

Semantic retrieval may improve recall for paraphrases, but vectors, embedding models, and
provider-specific metadata introduce a new dependency and a new privacy boundary. The current
supported slice has no semantic index.

## Decision

If a semantic index is added, it will be a derived representation of canonical artifact versions
and passages. It must be rebuildable, versioned by its embedding/extractor identity, and
discardable without losing source text, hashes, versions, or exact anchors. Semantic candidates
must resolve back to canonical passage records before they are shown as evidence.

The index must not silently send local content to a network provider. A local model or an
explicitly consented provider requires a separate design for collection, retention, failure, and
capabilities.

## Consequences

Positive:

- Semantic search can evolve without making vectors the source of truth.
- Model changes can be evaluated and reindexed independently.
- Evidence remains tied to a concrete source version.

Negative:

- There will be additional storage, CPU, and migration cost.
- Model or embedding version changes can alter ranking.
- A failed or unavailable semantic index must not make lexical evidence unavailable.

## Out of scope

No model, embedding runtime, vector database, remote inference, or semantic ranking is part of the
current pre-alpha slice.
