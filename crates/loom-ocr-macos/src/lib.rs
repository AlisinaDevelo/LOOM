//! A small, local-only OCR provider boundary.
//!
//! The macOS implementation is intentionally isolated here because the generated Objective-C
//! bindings require a small audited unsafe boundary. The core crate only sees owned Rust values;
//! it never stores Objective-C objects or sends source bytes over a process/network boundary.

#![allow(unsafe_code)]

use std::fmt;

pub const PROVIDER_ID: &str = "macos.vision";
pub const PROVIDER_VERSION: &str = "0.1.0";
pub const MODEL_FAMILY: &str = "VNRecognizeTextRequestRevision";

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NormalizedBounds {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OcrRegion {
    pub text: String,
    pub confidence: f32,
    pub bounds: NormalizedBounds,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OcrResult {
    pub provider_id: String,
    pub provider_version: String,
    pub model_version: String,
    pub regions: Vec<OcrRegion>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OcrError {
    Unavailable(String),
    InvalidInput(String),
    Provider(String),
}

impl fmt::Display for OcrError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable(message) => write!(formatter, "OCR unavailable: {message}"),
            Self::InvalidInput(message) => write!(formatter, "invalid OCR input: {message}"),
            Self::Provider(message) => write!(formatter, "OCR provider failed: {message}"),
        }
    }
}

impl std::error::Error for OcrError {}

#[cfg(target_os = "macos")]
mod native {
    use super::{
        NormalizedBounds, OcrError, OcrRegion, OcrResult, MODEL_FAMILY, PROVIDER_ID,
        PROVIDER_VERSION,
    };
    use objc2::rc::autoreleasepool;
    use objc2::AnyThread;
    use objc2_foundation::{NSArray, NSData, NSDictionary};
    use objc2_image_io::CGImagePropertyOrientation;
    use objc2_vision::{
        VNImageOption, VNImageRequestHandler, VNRecognizeTextRequest, VNRequest,
        VNRequestTextRecognitionLevel,
    };

    fn orientation(value: u8) -> Result<CGImagePropertyOrientation, OcrError> {
        match value {
            1 => Ok(CGImagePropertyOrientation::Up),
            2 => Ok(CGImagePropertyOrientation::UpMirrored),
            3 => Ok(CGImagePropertyOrientation::Down),
            4 => Ok(CGImagePropertyOrientation::DownMirrored),
            5 => Ok(CGImagePropertyOrientation::LeftMirrored),
            6 => Ok(CGImagePropertyOrientation::Right),
            7 => Ok(CGImagePropertyOrientation::RightMirrored),
            8 => Ok(CGImagePropertyOrientation::Left),
            other => Err(OcrError::InvalidInput(format!(
                "EXIF orientation must be 1..=8, got {other}"
            ))),
        }
    }

    pub fn recognize(bytes: &[u8], orientation_value: u8) -> Result<OcrResult, OcrError> {
        autoreleasepool(|_| recognize_inner(bytes, orientation_value))
    }

    fn recognize_inner(bytes: &[u8], orientation_value: u8) -> Result<OcrResult, OcrError> {
        if bytes.is_empty() {
            return Err(OcrError::InvalidInput("image bytes are empty".into()));
        }
        let orientation = orientation(orientation_value)?;
        let data = NSData::with_bytes(bytes);
        let request = VNRecognizeTextRequest::new();
        request.setRecognitionLevel(VNRequestTextRecognitionLevel::Accurate);
        request.setUsesLanguageCorrection(true);
        request.setAutomaticallyDetectsLanguage(true);
        let options = NSDictionary::<VNImageOption, objc2::runtime::AnyObject>::new();
        let handler = unsafe {
            VNImageRequestHandler::initWithData_orientation_options(
                VNImageRequestHandler::alloc(),
                &data,
                orientation,
                &options,
            )
        };
        let requests = NSArray::<VNRequest>::from_slice(&[request.as_ref()]);
        handler
            .performRequests_error(&requests)
            .map_err(|error| OcrError::Provider(format!("Vision request failed: {error:?}")))?;

        // The revision is read from the configured request so the evidence record identifies the
        // actual OS model instead of assuming the SDK's current revision forever.
        let model_version = format!("{MODEL_FAMILY}{}", unsafe { request.revision() });

        let mut regions = Vec::new();
        if let Some(observations) = request.results() {
            for index in 0..observations.count() {
                let observation = observations.objectAtIndex(index);
                let candidates = observation.topCandidates(1);
                if candidates.count() == 0 {
                    continue;
                }
                let candidate = candidates.objectAtIndex(0);
                let text = candidate.string().to_string();
                let text = text.trim().to_string();
                if text.is_empty() {
                    continue;
                }
                // Vision's observation accessors are generated as unsafe because they cross the
                // Objective-C ABI. The request and typed result collection above establish the
                // object invariants before these two reads.
                let bounds = unsafe { observation.boundingBox() };
                let confidence = candidate.confidence();
                let bounds = NormalizedBounds {
                    x: bounds.origin.x as f32,
                    y: bounds.origin.y as f32,
                    width: bounds.size.width as f32,
                    height: bounds.size.height as f32,
                };
                if ![bounds.x, bounds.y, bounds.width, bounds.height, confidence]
                    .into_iter()
                    .all(f32::is_finite)
                    || bounds.x < 0.0
                    || bounds.y < 0.0
                    || bounds.width < 0.0
                    || bounds.height < 0.0
                    || bounds.x + bounds.width > 1.001
                    || bounds.y + bounds.height > 1.001
                    || !(0.0..=1.0).contains(&confidence)
                {
                    return Err(OcrError::Provider(
                        "Vision returned an invalid normalized region".into(),
                    ));
                }
                regions.push(OcrRegion {
                    text,
                    confidence,
                    bounds,
                });
            }
        }

        let mut warnings = Vec::new();
        if regions.is_empty() {
            warnings.push("Vision returned no recognized text regions".into());
        }
        Ok(OcrResult {
            provider_id: PROVIDER_ID.into(),
            provider_version: PROVIDER_VERSION.into(),
            model_version,
            regions,
            warnings,
        })
    }
}

#[cfg(target_os = "macos")]
pub use native::recognize;

#[cfg(not(target_os = "macos"))]
pub fn recognize(_bytes: &[u8], _orientation: u8) -> Result<OcrResult, OcrError> {
    Err(OcrError::Unavailable(
        "the macOS Vision provider is unavailable on this target".into(),
    ))
}
