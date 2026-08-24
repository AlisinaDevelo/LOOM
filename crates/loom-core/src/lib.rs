//! Canonical local storage, explicit-source ingestion, and evidence-first retrieval.

mod domain;
mod error;
mod ingest;
mod observe;
mod ocr;
mod ranking;
mod search;
mod semantic;
mod store;

pub use domain::{
    ArtifactObservation, CaptureBounds, CaptureContext, CaptureMode, CapturePurgeReport,
    CaptureReport, EvidenceAnchor, EvidenceExcerpt, EvidenceSegment, EvidenceView, FtsHealthReport,
    FtsRepairReport, IndexCancellationToken, IndexCheckpoint, IndexFailure, IndexReport,
    LibraryStats, ObservationReport, OcrPurgeReport, OcrStatus, OpenArtifactRequest,
    PassageObservation, RankContributions, ResolveEvidenceRequest, SearchHit, SearchRequest,
    SemanticCandidate, SemanticDropReport, SemanticIndexConfig, SemanticIndexManifest,
    SemanticIndexStatus, SemanticProviderMeasurement, SemanticRebuildReport, SourceRootInfo,
    SourceRootStatus,
};
pub use error::{LoomError, Result};
pub use observe::{coalesce_events, ObservationEvent, ObservationEventKind, ObservationPlan};
pub use ranking::{
    fuse_hybrid_candidates, HybridRankConfig, HybridRankInput, HybridSearchHit,
    HybridSignalEvidence,
};
pub use search::{
    parse_query, ConfidenceFilter, ConfidenceOperator, ParsedQuery, QueryFilters, SourceTypeFilter,
};
pub use store::{Library, LibraryLimits};
