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
    pub passages: Vec<PassageObservation>,
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
    pub match_reason: String,
}
