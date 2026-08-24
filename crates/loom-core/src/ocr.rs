//! Bounded image geometry and the local OCR provider boundary.
//!
//! Vision returns normalized rectangles with an origin at the lower-left. LOOM converts those
//! values once into oriented, top-left pixel coordinates and stores only fixed-point metadata in
//! SQLite. The conversion is pure Rust and therefore testable on every target; the provider call
//! itself is supplied by the macOS-only `loom-ocr-macos` crate.

use std::io::{BufReader, Cursor};

use exif::{In, Reader as ExifReader, Tag};
use image::ImageReader;
use serde_json::json;

use crate::error::{LoomError, Result};

pub(crate) const IMAGE_OCR_EXTRACTOR_ID: &str = "loom.ocr";
pub(crate) const IMAGE_OCR_EXTRACTOR_VERSION: &str = "0.1.0";
pub(crate) const DEFAULT_SCALE_MILLI: u32 = 1_000;
const MAX_IMAGE_PIXELS: u64 = 100_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ImagePixelBounds {
    pub(crate) x: u32,
    pub(crate) y: u32,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ImageOcrRegion {
    pub(crate) text: String,
    pub(crate) confidence_milli: u32,
    pub(crate) bounds: ImagePixelBounds,
    pub(crate) char_start: u64,
    pub(crate) char_end: u64,
    pub(crate) line_start: u64,
    pub(crate) line_end: u64,
    pub(crate) image_width: u32,
    pub(crate) image_height: u32,
    pub(crate) orientation: u8,
    pub(crate) scale_milli: u32,
}

#[derive(Debug)]
pub(crate) struct ImageExtraction {
    pub(crate) normalized_text: String,
    pub(crate) regions: Vec<ImageOcrRegion>,
    pub(crate) metadata: serde_json::Value,
    pub(crate) warnings: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ImageProperties {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) orientation: u8,
}

pub(crate) fn inspect_image(bytes: &[u8]) -> Result<ImageProperties> {
    if bytes.is_empty() {
        return Err(LoomError::ImageExtraction("image bytes are empty".into()));
    }
    let reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|error| {
            LoomError::ImageExtraction(format!("could not identify image: {error}"))
        })?;
    let (width, height) = reader.into_dimensions().map_err(|error| {
        LoomError::ImageExtraction(format!("could not read image dimensions: {error}"))
    })?;
    if width == 0 || height == 0 {
        return Err(LoomError::ImageExtraction(
            "image dimensions must be non-zero".into(),
        ));
    }
    if u64::from(width) * u64::from(height) > MAX_IMAGE_PIXELS {
        return Err(LoomError::ImageExtraction(format!(
            "image dimensions exceed the {MAX_IMAGE_PIXELS}-pixel limit"
        )));
    }
    Ok(ImageProperties {
        width,
        height,
        orientation: exif_orientation(bytes),
    })
}

fn exif_orientation(bytes: &[u8]) -> u8 {
    let mut reader = BufReader::new(Cursor::new(bytes));
    let Ok(exif) = ExifReader::new().read_from_container(&mut reader) else {
        return 1;
    };
    let Some(field) = exif.get_field(Tag::Orientation, In::PRIMARY) else {
        return 1;
    };
    match field.value.get_uint(0) {
        Some(value @ 1..=8) => value as u8,
        _ => 1,
    }
}

pub(crate) fn oriented_dimensions(width: u32, height: u32, orientation: u8) -> (u32, u32) {
    if (5..=8).contains(&orientation) {
        (height, width)
    } else {
        (width, height)
    }
}

/// Converts a normalized lower-left rectangle into a clamped top-left pixel rectangle.
pub(crate) fn normalized_to_pixel_bounds(
    normalized: loom_ocr_macos::NormalizedBounds,
    encoded_width: u32,
    encoded_height: u32,
    orientation: u8,
) -> Result<ImagePixelBounds> {
    if encoded_width == 0 || encoded_height == 0 {
        return Err(LoomError::ImageExtraction(
            "image dimensions must be non-zero".into(),
        ));
    }
    if !(1..=8).contains(&orientation)
        || ![
            normalized.x,
            normalized.y,
            normalized.width,
            normalized.height,
        ]
        .into_iter()
        .all(f32::is_finite)
    {
        return Err(LoomError::OcrExtraction(
            "provider returned invalid image geometry".into(),
        ));
    }
    let (width, height) = oriented_dimensions(encoded_width, encoded_height, orientation);
    let x = scaled_round(normalized.x, width).min(width.saturating_sub(1));
    let y =
        scaled_round(1.0 - normalized.y - normalized.height, height).min(height.saturating_sub(1));
    let right = scaled_round(normalized.x + normalized.width, width)
        .max(x.saturating_add(1))
        .min(width);
    let bottom = scaled_round(1.0 - normalized.y, height)
        .max(y.saturating_add(1))
        .min(height);
    Ok(ImagePixelBounds {
        x,
        y,
        width: right
            .saturating_sub(x)
            .max(1)
            .min(width.saturating_sub(x).max(1)),
        height: bottom
            .saturating_sub(y)
            .max(1)
            .min(height.saturating_sub(y).max(1)),
    })
}

fn scaled_round(value: f32, extent: u32) -> u32 {
    let value = (value as f64).clamp(0.0, 1.0);
    (value * extent as f64).round().clamp(0.0, extent as f64) as u32
}

#[cfg(test)]
pub(crate) fn scaled_pixel_bounds(bounds: ImagePixelBounds, scale_milli: u32) -> ImagePixelBounds {
    let scale = scale_milli.max(1) as u64;
    let scale_one = 1_000u64;
    let scale_value = |value: u32| -> u32 {
        ((u64::from(value) * scale + scale_one / 2) / scale_one).min(u64::from(u32::MAX)) as u32
    };
    ImagePixelBounds {
        x: scale_value(bounds.x),
        y: scale_value(bounds.y),
        width: scale_value(bounds.width),
        height: scale_value(bounds.height),
    }
}

pub(crate) fn extract_image(bytes: &[u8]) -> Result<ImageExtraction> {
    let properties = inspect_image(bytes)?;
    let provider =
        loom_ocr_macos::recognize(bytes, properties.orientation).map_err(|error| match error {
            loom_ocr_macos::OcrError::Unavailable(message) => LoomError::OcrUnavailable(message),
            loom_ocr_macos::OcrError::InvalidInput(message)
            | loom_ocr_macos::OcrError::Provider(message) => LoomError::OcrExtraction(message),
        })?;
    if provider.regions.is_empty() {
        return Err(LoomError::OcrExtraction(
            "provider returned no text regions".into(),
        ));
    }

    let (image_width, image_height) =
        oriented_dimensions(properties.width, properties.height, properties.orientation);
    let mut normalized_text = String::new();
    let mut regions = Vec::with_capacity(provider.regions.len());
    for (index, region) in provider.regions.into_iter().enumerate() {
        if index > 0 {
            normalized_text.push('\n');
        }
        let char_start = normalized_text.chars().count() as u64;
        normalized_text.push_str(&region.text);
        let char_end = normalized_text.chars().count() as u64;
        let bounds = normalized_to_pixel_bounds(
            region.bounds,
            properties.width,
            properties.height,
            properties.orientation,
        )?;
        regions.push(ImageOcrRegion {
            text: region.text,
            confidence_milli: (region.confidence.clamp(0.0, 1.0) * 1_000.0).round() as u32,
            bounds,
            char_start,
            char_end,
            line_start: index as u64 + 1,
            line_end: index as u64 + 1,
            image_width,
            image_height,
            orientation: properties.orientation,
            scale_milli: DEFAULT_SCALE_MILLI,
        });
    }

    let metadata = json!({
        "kind": "image_ocr",
        "provider_id": provider.provider_id,
        "provider_version": provider.provider_version,
        "model_version": provider.model_version,
        "image_width": image_width,
        "image_height": image_height,
        "encoded_width": properties.width,
        "encoded_height": properties.height,
        "orientation": properties.orientation,
        "scale_milli": DEFAULT_SCALE_MILLI,
        "region_count": regions.len(),
    });
    Ok(ImageExtraction {
        normalized_text,
        regions,
        metadata,
        warnings: provider.warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        normalized_to_pixel_bounds, oriented_dimensions, scaled_pixel_bounds, ImagePixelBounds,
    };
    use loom_ocr_macos::NormalizedBounds;

    #[test]
    fn oriented_dimensions_swap_for_rotated_exif_values() {
        assert_eq!(oriented_dimensions(120, 80, 1), (120, 80));
        assert_eq!(oriented_dimensions(120, 80, 6), (80, 120));
        assert_eq!(oriented_dimensions(120, 80, 8), (80, 120));
    }

    #[test]
    fn normalized_vision_bounds_become_top_left_pixels() {
        let bounds = normalized_to_pixel_bounds(
            NormalizedBounds {
                x: 0.25,
                y: 0.5,
                width: 0.5,
                height: 0.25,
            },
            800,
            400,
            1,
        )
        .unwrap();
        assert_eq!(
            bounds,
            ImagePixelBounds {
                x: 200,
                y: 100,
                width: 400,
                height: 100,
            }
        );
    }

    #[test]
    fn rotated_bounds_use_oriented_pixel_space() {
        let bounds = normalized_to_pixel_bounds(
            NormalizedBounds {
                x: 0.0,
                y: 0.0,
                width: 0.5,
                height: 0.5,
            },
            800,
            400,
            6,
        )
        .unwrap();
        assert_eq!(bounds.x, 0);
        assert_eq!(bounds.y, 400);
        assert_eq!(bounds.width, 200);
        assert_eq!(bounds.height, 400);
    }

    #[test]
    fn scale_is_fixed_point_and_rounds_without_float_drift() {
        assert_eq!(
            scaled_pixel_bounds(
                ImagePixelBounds {
                    x: 3,
                    y: 5,
                    width: 11,
                    height: 13,
                },
                2_000,
            ),
            ImagePixelBounds {
                x: 6,
                y: 10,
                width: 22,
                height: 26,
            }
        );
    }
}
