# ADR 0005: Use a deterministic local provider for the semantic-index contract

- Status: Accepted
- Date: 2026-08-24

## Context

The semantic index must be useful for contract and recovery testing without making canonical
evidence depend on a model download, a network provider, or an opaque vector service. The current
MVP needs a rebuildable derivative with explicit provider identity, model identity, dimension,
normalization, revision, passage hash, and source digest. It also needs a device measurement that
can be repeated on the target Mac before a neural provider is considered.

## Decision

The first provider is loom.hash-embedding / hashed-tokens-v1. It lowercases Unicode
alphanumeric tokens, hashes tokens and adjacent bigrams with BLAKE3 into 128 signed buckets, and
L2-normalizes the resulting little-endian f32 vector. Vectors are stored as SQLite BLOBs in the
derived semantic_embeddings table and searched with a deterministic cosine scan ordered by score
then passage ID. The current implementation is MPL-2.0 project code using the existing BLAKE3
dependency; it downloads no model and makes no network request.

Canonical artifacts, versions, passages, hashes, and anchors remain authoritative. The vector
projection is disposable: it is rebuilt transactionally, checked against an ordered canonical
passage digest, and can be dropped without changing lexical retrieval. A provider or model change
must change the manifest and revision rather than mixing records.

## Target-device measurement

The following run used the rights-clean retrieval corpus (3 active passages, 533 source bytes) on
the target Mac documented in the 0203 evidence file. The debug loom binary was 18,853,416 bytes;
all provider candidates are compiled into the same binary, so this is a common binary footprint,
not a claimed per-provider delta. Vector storage is 512 bytes per passage (1,536 bytes for this
corpus).

|Provider candidate|Dimension|Passages|Vector bytes|Embedding time|License/model constraint|
|---|---:|---:|---:|---:|---|
|BLAKE3 token + bigram hash (chosen)|128|3|1,536|600 us|MPL-2.0 project code; no model download|
|BLAKE3 character trigram hash|128|3|1,536|2,916 us|MPL-2.0 project code; no model download|
|BLAKE3 token-count hash|128|3|1,536|235 us|MPL-2.0 project code; no model download|

The timed rebuild, including the local CLI process, took 0.95 s with a 13,336,576-byte maximum
resident set size on this run. These are footprint and execution measurements, not semantic
quality results. The benchmark does not claim that the hash baseline beats a neural model.

Neural runtimes and external vector stores remain deferred. A future option must be evaluated on
the same corpus and device for recall, latency, binary/model size, memory, model license, offline
behavior, and reproducible rebuilds before it can replace or supplement this provider. Native
SQLite extensions or a separate vector database would also add packaging, migration, and license
review; they are not prerequisites for the current linear scan.

## Consequences

- The contract is deterministic and testable on a clean, offline checkout.
- Rebuild and drop failures are visible without putting canonical evidence at risk.
- The hash baseline is intentionally a recall/contract scaffold, not a production semantic-quality
  claim.
- A later provider can be compared without changing canonical schema or result anchors.
- Linear scan is appropriate for the current bounded corpus; a larger corpus requires a measured
  index before changing the storage contract.

## References

- [ADR 0003: A semantic index is derived and optional](0003-derived-semantic-index.md)
- [Architecture](../ARCHITECTURE.md)
- [Data model](../DATA_MODEL.md)
