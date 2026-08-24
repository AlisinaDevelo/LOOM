use std::path::PathBuf;

/// Result type used throughout the LOOM core.
pub type Result<T> = std::result::Result<T, LoomError>;

/// Failures are explicit so callers can distinguish unsafe input from local I/O damage.
#[derive(Debug, thiserror::Error)]
pub enum LoomError {
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("I/O error for {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("invalid source path: {0}")]
    InvalidPath(String),

    #[error("unsupported source: {0}")]
    UnsupportedSource(String),

    #[error("PDF extraction failed: {0}")]
    PdfExtraction(String),

    #[error("image extraction failed: {0}")]
    ImageExtraction(String),

    #[error("OCR extraction failed: {0}")]
    OcrExtraction(String),

    #[error("image OCR is disabled")]
    OcrDisabled,

    #[error("OCR is unavailable: {0}")]
    OcrUnavailable(String),

    #[error("source changed while it was being read: {0}")]
    SourceChanged(String),

    #[error("invalid query: {0}")]
    InvalidQuery(String),

    #[error("could not project exact search evidence: {0}")]
    EvidenceProjection(String),

    #[error("semantic index unavailable: {0}")]
    SemanticIndexUnavailable(String),

    #[error("semantic index is incompatible: {0}")]
    SemanticIndexIncompatible(String),

    #[error("artifact not found: {0}")]
    ArtifactNotFound(String),

    #[error("artifact is stale or unavailable: {0}")]
    ArtifactStale(String),

    #[error("unsupported or invalid library schema version: {0}")]
    UnsupportedSchemaVersion(String),

    #[error("index job interrupted: {0}")]
    IndexInterrupted(String),

    #[error("library lock is unavailable")]
    LockPoisoned,

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

pub(crate) fn io_error(path: impl Into<PathBuf>, source: std::io::Error) -> LoomError {
    LoomError::Io {
        path: path.into(),
        source,
    }
}
