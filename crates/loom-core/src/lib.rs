//! Canonical local storage, explicit-source ingestion, and evidence-first retrieval.

mod bookmarks;
mod domain;
mod error;
mod ingest;
mod observe;
mod ocr;
mod ranking;
mod search;
mod semantic;
mod store;

pub use bookmarks::parse_bookmark_export;
pub use domain::{
    ArtifactObservation, BookmarkEntry, BookmarkExport, BookmarkImportReport, BookmarkRecord,
    CaptureBounds, CaptureContext, CaptureMode, CapturePurgeReport, CaptureReport, DeletionReport,
    EvidenceAnchor, EvidenceExcerpt, EvidenceSegment, EvidenceView, FtsHealthReport,
    FtsRepairReport, IndexCancellationToken, IndexCheckpoint, IndexFailure, IndexReport,
    LibraryStats, ObservationReport, OcrConfidenceState, OcrPurgeReport, OcrStatus,
    OpenArtifactRequest, PassageObservation, RankContributions, RelationshipEndpoint,
    RelationshipInput, RelationshipKind, RelationshipOrigin, RelationshipRecord, RelationshipView,
    ResolveEvidenceRequest, RetentionPolicy, RetentionReport, SearchHit, SearchRequest,
    SemanticCandidate, SemanticDropReport, SemanticIndexConfig, SemanticIndexManifest,
    SemanticIndexStatus, SemanticProviderMeasurement, SemanticRebuildReport, SourceRootInfo,
    SourceRootStatus, StorageEntry, StorageInspection,
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
