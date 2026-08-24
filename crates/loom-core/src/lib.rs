//! Canonical local storage, explicit-source ingestion, and evidence-first retrieval.

mod domain;
mod error;
mod ingest;
mod search;
mod store;

pub use domain::{
    ArtifactObservation, EvidenceAnchor, EvidenceExcerpt, EvidenceSegment, IndexCheckpoint,
    IndexFailure, IndexReport, LibraryStats, OpenArtifactRequest, PassageObservation, SearchHit,
    SearchRequest,
};
pub use error::{LoomError, Result};
pub use store::{Library, LibraryLimits};
