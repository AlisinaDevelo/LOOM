//! Canonical-domain adapter for the shared byte-only OCR implementation.

#[cfg(test)]
pub(crate) use loom_extraction::ocr::{inspect_image, ImagePixelBounds};
pub(crate) use loom_extraction::ocr::{
    ImageOcrRegion, IMAGE_OCR_EXTRACTOR_ID, IMAGE_OCR_EXTRACTOR_VERSION,
};

use crate::domain::{EvidenceAnchor, OcrConfidenceState};

pub(crate) fn anchor_confidence_state(anchor: &EvidenceAnchor) -> OcrConfidenceState {
    match anchor {
        EvidenceAnchor::ImageRegion {
            confidence_milli, ..
        } => match loom_extraction::ocr::confidence_state(*confidence_milli) {
            loom_extraction::ocr::OcrConfidenceState::Confirmed => OcrConfidenceState::Confirmed,
            loom_extraction::ocr::OcrConfidenceState::LowConfidence => {
                OcrConfidenceState::LowConfidence
            }
        },
        EvidenceAnchor::Text { .. } | EvidenceAnchor::PdfPage { .. } => {
            OcrConfidenceState::Confirmed
        }
    }
}
