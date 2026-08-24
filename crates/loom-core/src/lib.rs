//! Canonical local storage, explicit-source ingestion, and evidence-first retrieval.

mod domain;
mod error;
mod ingest;
mod observe;
mod ocr;
mod search;
mod semantic;
mod store;

pub use domain::{
    ArtifactObservation, EvidenceAnchor, EvidenceExcerpt, EvidenceSegment, EvidenceView,
    FtsHealthReport, FtsRepairReport, IndexCancellationToken, IndexCheckpoint, IndexFailure,
    IndexReport, LibraryStats, ObservationReport, OcrPurgeReport, OcrStatus, OpenArtifactRequest,
    PassageObservation, ResolveEvidenceRequest, SearchHit, SearchRequest, SemanticCandidate,
    SemanticDropReport, SemanticIndexConfig, SemanticIndexManifest, SemanticIndexStatus,
    SemanticProviderMeasurement, SemanticRebuildReport, SourceRootInfo, SourceRootStatus,
};
pub use error::{LoomError, Result};
pub use observe::{coalesce_events, ObservationEvent, ObservationEventKind, ObservationPlan};
pub use store::{Library, LibraryLimits};
