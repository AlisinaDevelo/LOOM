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
    pub discovered: u64,
    pub indexed: u64,
    pub unchanged: u64,
    pub skipped: u64,
    pub bytes_read: u64,
    pub failures: Vec<IndexFailure>,
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
