//! Canonical local storage, explicit-source ingestion, and evidence-first retrieval.

mod domain;
mod error;
mod ingest;
mod observe;
mod search;
mod store;

pub use domain::{
    ArtifactObservation, EvidenceAnchor, EvidenceExcerpt, EvidenceSegment, IndexCancellationToken,
    IndexCheckpoint, IndexFailure, IndexReport, LibraryStats, ObservationReport,
    OpenArtifactRequest, PassageObservation, SearchHit, SearchRequest, SourceRootInfo,
    SourceRootStatus,
};
pub use error::{LoomError, Result};
pub use observe::{coalesce_events, ObservationEvent, ObservationEventKind, ObservationPlan};
pub use store::{Library, LibraryLimits};
