//! Owned byte-only provider values; filesystem and canonical authority stay in the parent.

mod helper;
pub mod ocr;
mod output;
#[allow(unsafe_code)]
mod platform;
pub mod protocol;
mod supervisor;

pub use helper::serve_stdio;
pub use output::{extract_bytes, SourceOutput};
pub use supervisor::{ExtractionSupervisor, RunError, SupervisedOutput};

use serde::{Deserialize, Serialize};

pub const MAX_INPUT_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_TEXT_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
pub const WATCH_INTERVAL_MS: u32 = 25;

pub type Result<T> = std::result::Result<T, ExtractionError>;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ExtractionError {
    #[error("extractor protocol rejected: {0}")]
    Protocol(String),
    #[error("extractor output limit exceeded")]
    OutputLimit,
    #[error("source is not UTF-8 text")]
    InvalidText,
    #[error("PDF extraction failed: {0}")]
    PdfExtraction(String),
    #[error("image extraction failed: {0}")]
    ImageExtraction(String),
    #[error("OCR extraction failed: {0}")]
    OcrExtraction(String),
    #[error("OCR is unavailable: {0}")]
    OcrUnavailable(String),
    #[error("extractor resource enforcement is unavailable")]
    GuardUnavailable,
    #[error("trusted adjacent extractor is unavailable")]
    HelperUnavailable,
    #[error("extractor could not be launched")]
    Launch,
    #[error("extractor exited without a valid response")]
    ChildCrashed,
    #[error("extractor wall deadline exceeded")]
    WallTime,
    #[error("extractor CPU limit exceeded")]
    CpuTime,
    #[error("extractor sampled memory limit exceeded")]
    Memory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MediaKind {
    #[serde(rename = "text/plain")]
    Text,
    #[serde(rename = "text/markdown")]
    Markdown,
    #[serde(rename = "application/pdf")]
    Pdf,
    #[serde(rename = "image/png")]
    Png,
    #[serde(rename = "image/jpeg")]
    Jpeg,
    #[serde(rename = "image/gif")]
    Gif,
    #[serde(rename = "image/webp")]
    Webp,
}

impl MediaKind {
    pub fn from_mime(mime: &str) -> Result<Self> {
        match mime {
            "text/plain" => Ok(Self::Text),
            "text/markdown" => Ok(Self::Markdown),
            "application/pdf" => Ok(Self::Pdf),
            "image/png" => Ok(Self::Png),
            "image/jpeg" => Ok(Self::Jpeg),
            "image/gif" => Ok(Self::Gif),
            "image/webp" => Ok(Self::Webp),
            _ => Err(ExtractionError::Protocol("unsupported media".into())),
        }
    }

    pub fn mime(self) -> &'static str {
        match self {
            Self::Text => "text/plain",
            Self::Markdown => "text/markdown",
            Self::Pdf => "application/pdf",
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Gif => "image/gif",
            Self::Webp => "image/webp",
        }
    }

    pub fn is_image(self) -> bool {
        matches!(self, Self::Png | Self::Jpeg | Self::Gif | Self::Webp)
    }
}

/// Runtime-only policy, never source permission or portable canonical data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtractionBudget {
    pub wall_ms: u32,
    pub cpu_seconds: u32,
    pub memory_bytes: u64,
    pub address_space_bytes: u64,
    pub max_pdf_pages: u32,
    pub max_image_pixels: u64,
}

impl ExtractionBudget {
    pub fn for_media(media: MediaKind) -> Self {
        let (wall_ms, cpu_seconds, memory_mib) = if media.is_image() {
            (180_000, 120, 768)
        } else if media == MediaKind::Pdf {
            (30_000, 20, 512)
        } else {
            (5_000, 5, 256)
        };
        Self {
            wall_ms,
            cpu_seconds,
            memory_bytes: memory_mib * 1024 * 1024,
            // macOS includes very large shared-cache reservations in virtual
            // address accounting. This is not a physical-memory guarantee.
            address_space_bytes: if cfg!(target_os = "macos") {
                512 * 1024 * 1024 * 1024
            } else {
                2 * 1024 * 1024 * 1024
            },
            max_pdf_pages: 2048,
            max_image_pixels: 16_000_000,
        }
    }

    pub fn validate(self) -> Result<Self> {
        if !(25..=180_000).contains(&self.wall_ms)
            || !(1..=120).contains(&self.cpu_seconds)
            || !(64 * 1024 * 1024..=1024 * 1024 * 1024).contains(&self.memory_bytes)
            || !(64 * 1024 * 1024..=512 * 1024 * 1024 * 1024).contains(&self.address_space_bytes)
            || !(1..=2048).contains(&self.max_pdf_pages)
            || !(1..=16_000_000).contains(&self.max_image_pixels)
        {
            return Err(ExtractionError::Protocol("invalid resource budget".into()));
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestMetadata {
    pub media: MediaKind,
    pub budget: ExtractionBudget,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtractionMetrics {
    pub wall_ms: u64,
    pub cpu_ms: u64,
    pub peak_resident_bytes: u64,
    pub sample_interval_ms: u32,
    pub address_space_limit_installed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureCode {
    InvalidText,
    PdfExtraction,
    ImageExtraction,
    OcrExtraction,
    OcrUnavailable,
    OutputLimit,
    Protocol,
    GuardUnavailable,
    Memory,
}

impl FailureCode {
    pub fn from_error(error: &ExtractionError) -> Self {
        match error {
            ExtractionError::InvalidText => Self::InvalidText,
            ExtractionError::PdfExtraction(_) => Self::PdfExtraction,
            ExtractionError::ImageExtraction(_) => Self::ImageExtraction,
            ExtractionError::OcrExtraction(_) => Self::OcrExtraction,
            ExtractionError::OcrUnavailable(_) => Self::OcrUnavailable,
            ExtractionError::OutputLimit => Self::OutputLimit,
            ExtractionError::GuardUnavailable => Self::GuardUnavailable,
            ExtractionError::Memory => Self::Memory,
            _ => Self::Protocol,
        }
    }

    pub fn error(self) -> ExtractionError {
        match self {
            Self::InvalidText => ExtractionError::InvalidText,
            Self::PdfExtraction => {
                ExtractionError::PdfExtraction("provider refused source bytes".into())
            }
            Self::ImageExtraction => {
                ExtractionError::ImageExtraction("provider refused source bytes".into())
            }
            Self::OcrExtraction => {
                ExtractionError::OcrExtraction("provider refused source bytes".into())
            }
            Self::OcrUnavailable => {
                ExtractionError::OcrUnavailable("local provider is unavailable".into())
            }
            Self::OutputLimit => ExtractionError::OutputLimit,
            Self::Protocol => ExtractionError::Protocol("helper refused the request".into()),
            Self::GuardUnavailable => ExtractionError::GuardUnavailable,
            Self::Memory => ExtractionError::Memory,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum HelperResponse {
    Success {
        output: SourceOutput,
        metrics: ExtractionMetrics,
    },
    // No raw parser errors or source text enter diagnostics over the process boundary.
    Failure {
        code: FailureCode,
    },
}
