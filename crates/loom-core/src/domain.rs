use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use serde::{Deserialize, Serialize};

/// A versioned locator that lets the UI return to the evidence behind a result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EvidenceAnchor {
    /// Character and line offsets into LOOM's normalized UTF-8 text projection.
    Text {
        char_start: u64,
        char_end: u64,
        line_start: u64,
        line_end: u64,
    },
    /// Character and line offsets within one extracted PDF page.
    PdfPage {
        page: u32,
        char_start: u64,
        char_end: u64,
        line_start: u64,
        line_end: u64,
    },
    /// Character and line offsets for one OCR region in an image's oriented pixel space.
    ///
    /// Coordinates are integer pixels after the EXIF orientation transform. Fixed-point fields
    /// keep the canonical evidence record deterministic across FFI and database round-trips.
    ImageRegion {
        char_start: u64,
        char_end: u64,
        line_start: u64,
        line_end: u64,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        image_width: u32,
        image_height: u32,
        orientation: u8,
        scale_milli: u32,
        confidence_milli: u32,
    },
}

/// Confidence outcome exposed by OCR-backed evidence and indexing failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OcrConfidenceState {
    Confirmed,
    LowConfidence,
    NoReadableText,
}

impl OcrConfidenceState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Confirmed => "confirmed",
            Self::LowConfidence => "low_confidence",
            Self::NoReadableText => "no_readable_text",
        }
    }
}

/// One source that could not be indexed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexFailure {
    pub source: String,
    pub reason: String,
}

/// A bounded summary of an explicit indexing operation.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexReport {
    /// Stable durable identifier for this indexing run. A resumed run keeps the same ID.
    pub run_id: String,
    pub discovered: u64,
    /// Units processed in this invocation, including unchanged, skipped, and failed units.
    pub attempted: u64,
    pub indexed: u64,
    pub unchanged: u64,
    pub skipped: u64,
    pub failed: u64,
    /// Units left unprocessed because cancellation was requested at a safe boundary.
    pub cancelled: u64,
    pub bytes_read: u64,
    pub failures: Vec<IndexFailure>,
}

/// Cooperative cancellation shared by an indexing worker and its local UI/controller.
///
/// Cancellation is observed between bounded ingestion units. The current unit is allowed to
/// finish its SQLite transaction so canonical artifacts and the durable checkpoint never expose
/// a partially committed version.
#[derive(Debug, Clone)]
pub struct IndexCancellationToken(Arc<AtomicBool>);

impl IndexCancellationToken {
    pub fn new() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

impl Default for IndexCancellationToken {
    fn default() -> Self {
        Self::new()
    }
}

/// Consistency evidence for the canonical passage rows and derived FTS5 projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FtsHealthReport {
    pub canonical_passages: u64,
    pub indexed_passages: u64,
    /// BLAKE3 over ordered canonical passage row IDs and text hashes.
    pub canonical_digest: String,
    /// BLAKE3 over the tokenizer vocabulary expected from canonical passage text.
    pub expected_derivative_digest: String,
    /// BLAKE3 over the vocabulary currently present in the FTS5 projection.
    pub derivative_digest: String,
    pub integrity_ok: bool,
    pub integrity_error: Option<String>,
    pub healthy: bool,
}

/// Before/after evidence for one transactional FTS5 repair.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FtsRepairReport {
    pub before: FtsHealthReport,
    pub after: FtsHealthReport,
}

/// Durable progress for the most recent indexing job for an explicitly selected root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexCheckpoint {
    pub job_id: String,
    pub state: String,
    pub next_unit: u64,
    pub total_units: u64,
    pub last_error: Option<String>,
}

/// A bounded summary of a persisted-root observation pass.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservationReport {
    pub roots_scanned: u64,
    pub roots_failed: u64,
    pub events_received: u64,
    pub paths_coalesced: u64,
    pub full_rescans: u64,
    pub indexed: u64,
    pub unchanged: u64,
    pub skipped: u64,
    pub bytes_read: u64,
    pub failures: Vec<IndexFailure>,
}

/// Availability of a persisted, explicitly selected source root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceRootStatus {
    Available,
    Missing,
    Denied,
    WrongType,
    Unsafe,
    Revoked,
    Unavailable,
}

/// A persisted source scope exposed to the desktop UI.
///
/// LOOM's current direct-distribution build uses an explicit re-selection path instead of
/// pretending to hold a security-scoped bookmark. Ingestion opens source bytes read-only, and the
/// scope record contains no write capability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceRootInfo {
    pub locator: String,
    pub kind: String,
    pub enabled: bool,
    pub read_only: bool,
    pub status: SourceRootStatus,
}

/// Canonical library counts. Derived-index files are intentionally excluded.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryStats {
    pub source_roots: u64,
    pub artifacts: u64,
    pub versions: u64,
    pub passages: u64,
    pub indexed_bytes: u64,
}

/// A bounded estimate for one class of bytes retained under LOOM's application data directory.
///
/// The estimate is intentionally explicit about its category and source. Canonical source files
/// remain user-owned and are never removed by storage cleanup; only paths inside LOOM's known
/// disposable directories are eligible for cleanup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageEntry {
    pub category: String,
    pub path: String,
    pub source_uri: Option<String>,
    pub bytes: u64,
    pub files: u64,
    pub exists: bool,
}

/// Read-only accounting for canonical records, derived indexes, SQLite sidecars, and disposable
/// local files. It never follows symbolic links while walking known storage directories.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageInspection {
    pub database_path: Option<String>,
    pub generated_at: String,
    pub entries: Vec<StorageEntry>,
    pub total_bytes: u64,
    pub canonical_bytes: u64,
    pub derived_bytes: u64,
    pub disposable_bytes: u64,
    pub source_bytes: u64,
}

/// Counts and retained paths removed by an explicit deletion operation.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeletionReport {
    pub selector: String,
    pub artifacts_deleted: u64,
    pub versions_deleted: u64,
    pub passages_deleted: u64,
    pub relationships_deleted: u64,
    pub bookmark_records_deleted: u64,
    pub files_deleted: u64,
    pub bytes_deleted: u64,
    pub paths: Vec<String>,
}

/// Explicit retention policy. `None` means retention cleanup is disabled.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetentionPolicy {
    pub days: Option<u32>,
}

/// Result of applying the configured retention policy at a known clock instant.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetentionReport {
    pub policy: RetentionPolicy,
    pub evaluated_at: String,
    pub cutoff: Option<String>,
    pub deletion: DeletionReport,
}

/// Canonical extractor output for one stored passage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PassageObservation {
    pub ordinal: u32,
    pub text_hash: String,
    pub anchor: EvidenceAnchor,
}

/// Read-only canonical state for an explicitly indexed source.
///
/// The evaluation harness uses this projection to prove that fixture bytes were processed by the
/// expected extractor and produced the expected passage anchors. Derived FTS state is excluded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactObservation {
    pub source_uri: String,
    pub content_hash: String,
    pub extractor_id: String,
    pub extractor_version: String,
    pub page_count: Option<u32>,
    pub parse_warnings: Vec<String>,
    pub extraction_metadata: serde_json::Value,
    pub passages: Vec<PassageObservation>,
}

/// Whether local image OCR is enabled and how many derived records currently exist.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OcrStatus {
    pub enabled: bool,
    pub derived_versions: u64,
    pub derived_passages: u64,
}

/// Counts retained after deleting derived OCR records.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OcrPurgeReport {
    pub artifacts_affected: u64,
    pub versions_deleted: u64,
    pub passages_deleted: u64,
}

/// The user-selected capture surface. Capture is always an explicit command; there is no
/// background or periodic mode in the canonical API.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureMode {
    Screen,
    Window,
    Region,
}

/// Pixel-space bounds retained for an intentional capture. Coordinates are relative to the
/// captured image when the native picker does not expose the display origin.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureBounds {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// Provenance recorded before image OCR runs for an intentional capture.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureContext {
    pub mode: CaptureMode,
    pub captured_at: String,
    pub display_scale_milli: u32,
    pub bounds: CaptureBounds,
    pub app_name: Option<String>,
    pub window_title: Option<String>,
    pub source: String,
}

/// Result of one explicit capture/import operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureReport {
    pub status: String,
    pub source_uri: String,
    pub content_hash: String,
    pub byte_size: u64,
    pub duplicate: bool,
    pub context: CaptureContext,
}

/// Counts removed when a user purges the capture source root.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapturePurgeReport {
    pub artifacts_deleted: u64,
    pub versions_deleted: u64,
    pub passages_deleted: u64,
}

/// One metadata-only entry from a Netscape HTML bookmark export.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BookmarkEntry {
    pub folder_path: String,
    pub title: String,
    pub url: String,
    pub added_at: Option<String>,
    pub modified_at: Option<String>,
}

/// Parsed bookmark export. The parser never resolves the URLs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BookmarkExport {
    pub format: String,
    pub bookmarks: Vec<BookmarkEntry>,
}

/// Result of one local bookmark metadata import.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BookmarkImportReport {
    pub import_id: String,
    pub source_uri: String,
    pub format: String,
    pub content_hash: String,
    pub discovered: u64,
    pub imported: u64,
    pub unchanged: u64,
    pub merged: u64,
    pub conflicts: u64,
    pub failed: u64,
    pub remote_fetches: u64,
    pub failures: Vec<IndexFailure>,
}

/// Durable metadata for one current bookmark record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BookmarkRecord {
    pub id: String,
    pub artifact_id: String,
    pub import_id: String,
    pub source_uri: String,
    pub folder_path: String,
    pub title: String,
    pub url: String,
    pub added_at: Option<String>,
    pub modified_at: Option<String>,
    pub entry_hash: String,
    pub import_count: u64,
}

/// Stable relationship vocabulary. Unknown values are preserved so a newer connector can be
/// opened by an older reader without silently changing the graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelationshipKind {
    SavedFrom,
    ScreenshotOf,
    DuplicateOf,
    PreviousVersionOf,
    Related,
    Unknown(String),
}

impl RelationshipKind {
    pub fn as_str(&self) -> &str {
        match self {
            Self::SavedFrom => "saved_from",
            Self::ScreenshotOf => "screenshot_of",
            Self::DuplicateOf => "duplicate_of",
            Self::PreviousVersionOf => "previous_version_of",
            Self::Related => "related",
            Self::Unknown(value) => value.as_str(),
        }
    }

    pub fn from_value(value: impl Into<String>) -> Self {
        let value = value.into();
        match value.as_str() {
            "saved_from" => Self::SavedFrom,
            "screenshot_of" => Self::ScreenshotOf,
            "duplicate_of" => Self::DuplicateOf,
            "previous_version_of" => Self::PreviousVersionOf,
            "related" => Self::Related,
            _ => Self::Unknown(value),
        }
    }
}

impl Serialize for RelationshipKind {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for RelationshipKind {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Ok(Self::from_value(value))
    }
}

/// How a relationship entered the local graph. Inferred edges never masquerade as user
/// confirmation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationshipOrigin {
    Observed,
    Inferred,
    UserConfirmed,
}

impl RelationshipOrigin {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Observed => "observed",
            Self::Inferred => "inferred",
            Self::UserConfirmed => "user_confirmed",
        }
    }
}

/// Request to create or retrieve one source-backed relationship.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RelationshipInput {
    pub source_artifact_id: String,
    pub target_artifact_id: String,
    pub kind: RelationshipKind,
    pub origin: RelationshipOrigin,
    pub evidence_passage_id: Option<String>,
    pub confidence: Option<f64>,
    pub method: String,
    #[serde(default = "empty_metadata")]
    pub metadata: serde_json::Value,
}

fn empty_metadata() -> serde_json::Value {
    serde_json::json!({})
}

/// Canonical relationship row. The schema version versions the relationship envelope independently
/// from the SQLite migration marker.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RelationshipRecord {
    pub id: String,
    pub schema_version: u32,
    pub source_artifact_id: String,
    pub target_artifact_id: String,
    pub kind: RelationshipKind,
    pub origin: RelationshipOrigin,
    pub evidence_passage_id: Option<String>,
    pub confidence: Option<f64>,
    pub method: String,
    pub metadata: serde_json::Value,
    pub created_at: String,
}

/// Endpoint projection used by the UI to traverse a relationship without a graph database.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelationshipEndpoint {
    pub artifact_id: String,
    pub title: String,
    pub media_type: String,
    pub source_uri: Option<String>,
    pub version_id: Option<String>,
    pub content_hash: Option<String>,
    pub state: String,
}

/// One relationship with both source-backed endpoint projections.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RelationshipView {
    pub relationship: RelationshipRecord,
    pub source: RelationshipEndpoint,
    pub target: RelationshipEndpoint,
}

/// A user search request crossing the Tauri IPC boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchRequest {
    pub text: String,
    #[serde(default = "default_limit")]
    pub limit: u32,
}

const fn default_limit() -> u32 {
    20
}

/// The source version and hash a caller is asking LOOM to open.
///
/// Binding the open operation to a search result prevents a changed source path from being
/// presented as the bytes that produced the result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenArtifactRequest {
    pub artifact_id: String,
    pub version_id: String,
    pub content_hash: String,
}

/// The source version and passage a caller is asking LOOM to show as evidence.
///
/// The passage identifier is deliberately bound to the same artifact/version/hash tuple used
/// for opening the original. A viewer must never silently substitute a newer file or an unrelated
/// passage when the source has moved, changed, or been re-indexed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolveEvidenceRequest {
    pub artifact_id: String,
    pub version_id: String,
    pub passage_id: String,
    pub content_hash: String,
}

/// One source-derived segment in a structured evidence excerpt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceSegment {
    pub text: String,
    pub highlighted: bool,
}

/// An excerpt whose match styling cannot collide with characters in source text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceExcerpt {
    pub segments: Vec<EvidenceSegment>,
}

/// Canonical source-backed state for the evidence viewer.
///
/// `passage_text` is returned from SQLite after the active version/hash/passage tuple has been
/// verified. The UI may style it, but it cannot claim that an unverified client-side excerpt is
/// the source evidence. `anchor` is the stored page or image-region locator for that passage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvidenceView {
    pub artifact_id: String,
    pub version_id: String,
    pub passage_id: String,
    pub title: String,
    pub media_type: String,
    pub source_uri: String,
    pub content_hash: String,
    pub passage_text: String,
    pub anchor: EvidenceAnchor,
    pub confidence_state: OcrConfidenceState,
    pub page_count: Option<u32>,
    pub extractor_id: String,
    pub extractor_version: String,
    pub extraction_metadata: serde_json::Value,
}

/// Versioned configuration for a derived semantic embedding provider.
///
/// This metadata is part of the derivative contract, never canonical source identity. A provider
/// change must produce a new index revision or be rejected rather than mixing incompatible vectors.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticIndexConfig {
    pub provider_id: String,
    pub model_id: String,
    /// Tokenization contract used to turn source passages into provider inputs.
    pub tokenizer: String,
    pub dimension: u32,
    pub normalization: String,
    /// Canonical, versioned build parameters for the derived vector representation.
    pub build_parameters: String,
    pub index_revision: String,
}

impl Default for SemanticIndexConfig {
    fn default() -> Self {
        Self {
            provider_id: "loom.hash-embedding".into(),
            model_id: "hashed-tokens-v1".into(),
            tokenizer: "unicode-alnum-lower-v1".into(),
            dimension: 128,
            normalization: "l2".into(),
            build_parameters: "hash-token=1.0;hash-bigram=0.5;vector=float32-le-v1".into(),
            index_revision: "semantic-v1".into(),
        }
    }
}

/// A rebuild manifest for the disposable semantic index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticIndexManifest {
    pub config: SemanticIndexConfig,
    pub source_digest: String,
    pub canonical_passages: u64,
    pub indexed_passages: u64,
    pub vector_bytes: u64,
}

/// Health and compatibility state for the semantic derivative.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticIndexStatus {
    pub healthy: bool,
    pub canonical_passages: u64,
    pub indexed_passages: u64,
    pub canonical_digest: String,
    pub vector_bytes: u64,
    pub manifest: Option<SemanticIndexManifest>,
    pub reason: Option<String>,
}

/// Counts retained after explicitly retiring the semantic derivative.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticDropReport {
    pub embeddings_deleted: u64,
    pub manifest_deleted: bool,
}

/// Evidence-bound candidate returned by semantic retrieval.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SemanticCandidate {
    pub rank: u32,
    pub score: f64,
    pub artifact_id: String,
    pub version_id: String,
    pub passage_id: String,
    pub title: String,
    pub media_type: String,
    pub source_uri: String,
    pub content_hash: String,
    pub passage_hash: String,
    pub passage_text: String,
    pub anchor: EvidenceAnchor,
    pub model_id: String,
    pub index_revision: String,
}

/// Summary returned after rebuilding the disposable semantic derivative.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticRebuildReport {
    pub manifest: SemanticIndexManifest,
    pub rebuilt_passages: u64,
}

/// Device measurement for one local provider candidate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticProviderMeasurement {
    pub provider_id: String,
    pub model_id: String,
    pub dimension: u32,
    pub sample_count: u64,
    pub vector_bytes: u64,
    pub elapsed_micros: u64,
}

/// One ranked, source-backed search result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchHit {
    pub rank: u32,
    pub score: f64,
    pub artifact_id: String,
    pub version_id: String,
    pub passage_id: String,
    pub title: String,
    pub media_type: String,
    pub source_uri: String,
    pub content_hash: String,
    pub excerpt: EvidenceExcerpt,
    pub anchor: EvidenceAnchor,
    pub confidence_state: OcrConfidenceState,
    /// Explainable contributions from each retrieval stage. A stage that did not participate is
    /// reported as zero rather than omitted so callers can compare lexical and hybrid results.
    pub contributions: RankContributions,
    pub match_reason: String,
}

/// Stable score contributions retained on every public search result.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RankContributions {
    pub lexical: f64,
    pub semantic: f64,
    pub metadata: f64,
    pub reranker: f64,
}
