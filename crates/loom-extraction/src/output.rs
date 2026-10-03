use std::panic::{catch_unwind, AssertUnwindSafe};

use serde::{Deserialize, Serialize};

use crate::{ocr, ExtractionBudget, ExtractionError, MediaKind, Result, MAX_TEXT_BYTES};

/// Exactly one representation; PDF/image text is not duplicated on the wire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SourceOutput {
    Text {
        text: String,
    },
    Pdf {
        page_count: u32,
        pages: Vec<(u32, String)>,
        warnings: Vec<String>,
    },
    Image {
        regions: Vec<ocr::ImageOcrRegion>,
        metadata: serde_json::Value,
        warnings: Vec<String>,
    },
}

impl SourceOutput {
    /// Validate helper-owned values using the strict worker resource budget.
    pub fn validate(&self, media: MediaKind, budget: ExtractionBudget) -> Result<()> {
        self.validate_with_limits(
            media,
            budget,
            budget.max_pdf_pages as usize,
            budget.max_image_pixels,
        )
    }

    /// Validate output shape with explicit publication limits after the runtime budget is checked.
    ///
    /// The helper budget remains strict, while foreground indexing can retain its existing
    /// configured PDF and 100M-pixel limits without weakening the worker's resource contract.
    pub fn validate_with_limits(
        &self,
        media: MediaKind,
        budget: ExtractionBudget,
        max_pdf_pages: usize,
        max_image_pixels: u64,
    ) -> Result<()> {
        budget.validate()?;
        let invalid = || ExtractionError::Protocol("invalid evidence geometry or media".into());
        match self {
            Self::Text { text } if matches!(media, MediaKind::Text | MediaKind::Markdown) => {
                if text.len() > MAX_TEXT_BYTES {
                    return Err(ExtractionError::OutputLimit);
                }
                if invalid_text_controls(text) {
                    return Err(invalid());
                }
            }
            Self::Pdf {
                page_count,
                pages,
                warnings,
            } if media == MediaKind::Pdf => {
                validate_warnings(warnings)?;
                if *page_count == 0 || pages.len() != *page_count as usize {
                    return Err(invalid());
                }
                if (*page_count as usize) > max_pdf_pages {
                    return Err(ExtractionError::OutputLimit);
                }
                let mut total = pages.len().saturating_sub(1) * 2;
                for (index, (page, text)) in pages.iter().enumerate() {
                    total = total
                        .checked_add(text.len())
                        .ok_or(ExtractionError::OutputLimit)?;
                    if *page != index as u32 + 1 || invalid_text_controls(text) {
                        return Err(invalid());
                    }
                }
                if total > MAX_TEXT_BYTES {
                    return Err(ExtractionError::OutputLimit);
                }
                if pages.iter().all(|(_, text)| text.trim().is_empty()) {
                    return Err(invalid());
                }
            }
            Self::Image {
                regions,
                metadata,
                warnings,
            } if media.is_image() => {
                validate_warnings(warnings)?;
                if regions.is_empty() {
                    return Err(invalid());
                }
                if regions.len() > 8192 {
                    return Err(ExtractionError::OutputLimit);
                }
                let first = &regions[0];
                if first.image_width == 0
                    || first.image_height == 0
                    || u64::from(first.image_width) * u64::from(first.image_height)
                        > max_image_pixels
                    || !(1..=8).contains(&first.orientation)
                    || first.scale_milli != 1000
                {
                    return Err(invalid());
                }
                let mut chars = 0u64;
                let mut bytes = regions.len().saturating_sub(1);
                let mut low = 0u64;
                for (index, region) in regions.iter().enumerate() {
                    if index != 0 {
                        chars += 1;
                    }
                    let end = chars
                        .checked_add(region.text.chars().count() as u64)
                        .ok_or_else(invalid)?;
                    let rect = region.bounds;
                    if region.text.trim().is_empty()
                        || invalid_text_controls(&region.text)
                        || region.confidence_milli > 1000
                        || region.image_width != first.image_width
                        || region.image_height != first.image_height
                        || region.orientation != first.orientation
                        || region.scale_milli != first.scale_milli
                        || rect.width == 0
                        || rect.height == 0
                        || rect
                            .x
                            .checked_add(rect.width)
                            .is_none_or(|x| x > first.image_width)
                        || rect
                            .y
                            .checked_add(rect.height)
                            .is_none_or(|y| y > first.image_height)
                        || region.char_start != chars
                        || region.char_end != end
                        || region.line_start != index as u64 + 1
                        || region.line_end != index as u64 + 1
                    {
                        return Err(invalid());
                    }
                    bytes = bytes
                        .checked_add(region.text.len())
                        .ok_or(ExtractionError::OutputLimit)?;
                    if region.confidence_milli < ocr::LOW_CONFIDENCE_THRESHOLD_MILLI {
                        low += 1;
                    }
                    chars = end;
                }
                if bytes > MAX_TEXT_BYTES {
                    return Err(ExtractionError::OutputLimit);
                }
                validate_image_metadata(metadata, first, regions.len(), low)?;
            }
            _ => return Err(invalid()),
        }
        Ok(())
    }
}

fn invalid_text_controls(text: &str) -> bool {
    text.chars()
        .any(|character| character.is_control() && !matches!(character, '\n' | '\t'))
}

fn validate_warnings(warnings: &[String]) -> Result<()> {
    if warnings.len() > 128 || warnings.iter().map(String::len).sum::<usize>() > 16_384 {
        return Err(ExtractionError::OutputLimit);
    }
    Ok(())
}

fn validate_image_metadata(
    metadata: &serde_json::Value,
    first: &ocr::ImageOcrRegion,
    count: usize,
    low: u64,
) -> Result<()> {
    const KEYS: &[&str] = &[
        "kind",
        "provider_id",
        "provider_version",
        "model_version",
        "language",
        "image_width",
        "image_height",
        "encoded_width",
        "encoded_height",
        "orientation",
        "scale_milli",
        "region_count",
        "confidence_threshold_milli",
        "low_confidence_regions",
        "confidence_state",
    ];
    let invalid = || ExtractionError::Protocol("invalid provider metadata".into());
    let fields = metadata.as_object().ok_or_else(invalid)?;
    if fields.len() != KEYS.len() || !KEYS.iter().all(|key| fields.contains_key(*key)) {
        return Err(invalid());
    }
    let model = metadata["model_version"].as_str().ok_or_else(invalid)?;
    let revision = model
        .strip_prefix(loom_ocr_macos::MODEL_FAMILY)
        .ok_or_else(invalid)?;
    let encoded_width = metadata["encoded_width"]
        .as_u64()
        .and_then(|n| u32::try_from(n).ok())
        .ok_or_else(invalid)?;
    let encoded_height = metadata["encoded_height"]
        .as_u64()
        .and_then(|n| u32::try_from(n).ok())
        .ok_or_else(invalid)?;
    if revision.is_empty()
        || revision.len() > 10
        || !revision.bytes().all(|b| b.is_ascii_digit())
        || metadata["kind"] != "image_ocr"
        || metadata["provider_id"] != loom_ocr_macos::PROVIDER_ID
        || metadata["provider_version"] != loom_ocr_macos::PROVIDER_VERSION
        || metadata["language"] != "auto"
        || metadata["image_width"] != first.image_width
        || metadata["image_height"] != first.image_height
        || metadata["orientation"] != first.orientation
        || metadata["scale_milli"] != first.scale_milli
        || encoded_width == 0
        || encoded_height == 0
        || ocr::oriented_dimensions(encoded_width, encoded_height, first.orientation)
            != (first.image_width, first.image_height)
        || metadata["region_count"] != count
        || metadata["low_confidence_regions"] != low
        || metadata["confidence_threshold_milli"] != ocr::LOW_CONFIDENCE_THRESHOLD_MILLI
        || metadata["confidence_state"]
            != if low == 0 {
                "confirmed"
            } else {
                "low_confidence"
            }
    {
        return Err(invalid());
    }
    Ok(())
}

/// The single byte-to-evidence implementation shared by foreground and the helper.
/// No filesystem path or canonical authority enters this function.
pub fn extract_bytes(
    media: MediaKind,
    bytes: &[u8],
    max_pdf_pages: usize,
    max_image_pixels: u64,
) -> Result<SourceOutput> {
    if media == MediaKind::Pdf {
        return extract_pdf(bytes, max_pdf_pages);
    }
    if media.is_image() {
        let properties = ocr::inspect_image(bytes)?;
        if u64::from(properties.width) * u64::from(properties.height) > max_image_pixels {
            return Err(ExtractionError::OutputLimit);
        }
        let extraction = ocr::extract_image(bytes)?;
        return Ok(SourceOutput::Image {
            regions: extraction.regions,
            metadata: extraction.metadata,
            warnings: extraction.warnings,
        });
    }
    let text = std::str::from_utf8(bytes).map_err(|_| ExtractionError::InvalidText)?;
    Ok(SourceOutput::Text {
        text: text.replace("\r\n", "\n").replace('\r', "\n"),
    })
}

fn extract_pdf(bytes: &[u8], max_pdf_pages: usize) -> Result<SourceOutput> {
    catch_unwind(AssertUnwindSafe(|| {
        if bytes
            .windows(b"/Encrypt".len())
            .any(|window| window == b"/Encrypt")
        {
            return Err(ExtractionError::PdfExtraction(
                "encrypted PDF requires an explicit password and was not indexed".into(),
            ));
        }
        let document = pdf_extract::Document::load_mem(bytes)
            .map_err(|error| ExtractionError::PdfExtraction(format!("malformed PDF: {error}")))?;
        if document.is_encrypted() {
            return Err(ExtractionError::PdfExtraction(
                "encrypted PDF requires an explicit password and was not indexed".into(),
            ));
        }
        let page_numbers = document.get_pages().keys().copied().collect::<Vec<_>>();
        if page_numbers.is_empty() {
            return Err(ExtractionError::PdfExtraction("PDF has no pages".into()));
        }
        if page_numbers.len() > max_pdf_pages {
            return Err(ExtractionError::PdfExtraction(format!(
                "PDF has {} pages, exceeding the {max_pdf_pages}-page limit",
                page_numbers.len()
            )));
        }
        let mut pages = Vec::with_capacity(page_numbers.len());
        let mut warnings = Vec::new();
        for page in page_numbers {
            let mut text = String::new();
            let mut output = pdf_extract::PlainTextOutput::new(&mut text);
            if let Err(error) = pdf_extract::output_doc_page(&document, &mut output, page) {
                warnings.push(format!("page {page} extraction failed: {error}"));
                pages.push((page, String::new()));
                continue;
            }
            let text = text
                .replace("\r\n", "\n")
                .replace('\r', "\n")
                .trim_matches('\n')
                .to_string();
            if text.trim().is_empty() {
                warnings.push(format!("page {page} contains no extractable text"));
            }
            pages.push((page, text));
        }
        if pages.iter().all(|(_, text)| text.trim().is_empty()) {
            return Err(ExtractionError::PdfExtraction(
                "PDF contains no extractable text (image-only or unsupported fonts)".into(),
            ));
        }
        Ok(SourceOutput::Pdf {
            page_count: pages.len() as u32,
            pages,
            warnings,
        })
    }))
    .map_err(|_| {
        ExtractionError::PdfExtraction(
            "PDF parser rejected malformed input without a recoverable error".into(),
        )
    })?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_normalization_is_shared_and_media_cannot_be_substituted() {
        let output = extract_bytes(
            MediaKind::Markdown,
            b"first\r\nsecond\rthird",
            2048,
            16_000_000,
        )
        .unwrap();
        assert_eq!(
            output,
            SourceOutput::Text {
                text: "first\nsecond\nthird".into()
            }
        );
        output
            .validate(
                MediaKind::Markdown,
                ExtractionBudget::for_media(MediaKind::Markdown),
            )
            .unwrap();
        assert!(output
            .validate(MediaKind::Pdf, ExtractionBudget::for_media(MediaKind::Pdf))
            .is_err());
    }

    #[test]
    fn oversized_text_and_noncontiguous_pdf_pages_are_refused() {
        let text = SourceOutput::Text {
            text: "x".repeat(MAX_TEXT_BYTES + 1),
        };
        assert_eq!(
            text.validate(
                MediaKind::Text,
                ExtractionBudget::for_media(MediaKind::Text)
            ),
            Err(ExtractionError::OutputLimit)
        );
        let pdf = SourceOutput::Pdf {
            page_count: 1,
            pages: vec![(2, "evidence".into())],
            warnings: vec![],
        };
        assert!(pdf
            .validate(MediaKind::Pdf, ExtractionBudget::for_media(MediaKind::Pdf))
            .is_err());
    }

    #[test]
    fn image_geometry_and_provider_metadata_are_validated_before_publication() {
        let output = SourceOutput::Image {
            regions: vec![ocr::ImageOcrRegion {
                text: "é源".into(),
                confidence_milli: 900,
                bounds: ocr::ImagePixelBounds {
                    x: 5,
                    y: 8,
                    width: 20,
                    height: 10,
                },
                char_start: 0,
                char_end: 2,
                line_start: 1,
                line_end: 1,
                image_width: 100,
                image_height: 60,
                orientation: 1,
                scale_milli: 1000,
            }],
            metadata: serde_json::json!({
                "kind": "image_ocr", "provider_id": loom_ocr_macos::PROVIDER_ID,
                "provider_version": loom_ocr_macos::PROVIDER_VERSION,
                "model_version": format!("{}3", loom_ocr_macos::MODEL_FAMILY),
                "language": "auto", "image_width": 100, "image_height": 60,
                "encoded_width": 100, "encoded_height": 60, "orientation": 1,
                "scale_milli": 1000, "region_count": 1, "confidence_threshold_milli": 800,
                "low_confidence_regions": 0, "confidence_state": "confirmed"
            }),
            warnings: vec![],
        };
        let budget = ExtractionBudget::for_media(MediaKind::Png);
        output.validate(MediaKind::Png, budget).unwrap();
        let original = serde_json::to_value(&output).unwrap();
        for (field, value) in [
            (
                "bounds",
                serde_json::json!({"x": u32::MAX, "y": 0, "width": 20, "height": 1}),
            ),
            (
                "bounds",
                serde_json::json!({"x": 0, "y": 0, "width": 0, "height": 1}),
            ),
            ("char_end", serde_json::json!(5)), // Byte count is not a character anchor.
            ("char_start", serde_json::json!(1)),
            ("line_end", serde_json::json!(2)),
            ("confidence_milli", serde_json::json!(1001)),
            ("orientation", serde_json::json!(0)),
            ("scale_milli", serde_json::json!(2000)),
            ("text", serde_json::json!("é\u{0}")),
            ("text", serde_json::json!("é\r")),
        ] {
            let mut altered = original.clone();
            altered["regions"][0][field] = value;
            let parsed: SourceOutput = serde_json::from_value(altered).unwrap();
            assert!(parsed.validate(MediaKind::Png, budget).is_err(), "{field}");
        }
        for (field, value) in [
            ("region_count", serde_json::json!(2)),
            ("provider_id", serde_json::json!("unknown")),
            ("model_version", serde_json::json!("unsupported")),
            ("encoded_width", serde_json::json!(101)),
            ("source_uri", serde_json::json!("must-not-be-in-output")),
        ] {
            let mut altered = original.clone();
            altered["metadata"][field] = value;
            let parsed: SourceOutput = serde_json::from_value(altered).unwrap();
            assert!(parsed.validate(MediaKind::Png, budget).is_err(), "{field}");
        }
        let mut altered = original;
        altered["regions"][0]["bounds"]["extra"] = serde_json::json!(1);
        assert!(serde_json::from_value::<SourceOutput>(altered).is_err());
    }

    #[test]
    fn foreground_image_limit_can_exceed_the_strict_queue_budget() {
        let output = SourceOutput::Image {
            regions: vec![ocr::ImageOcrRegion {
                text: "synthetic".into(),
                confidence_milli: 900,
                bounds: ocr::ImagePixelBounds {
                    x: 0,
                    y: 0,
                    width: 1,
                    height: 1,
                },
                char_start: 0,
                char_end: 9,
                line_start: 1,
                line_end: 1,
                image_width: 5_000,
                image_height: 4_000,
                orientation: 1,
                scale_milli: 1_000,
            }],
            metadata: serde_json::json!({
                "kind": "image_ocr", "provider_id": loom_ocr_macos::PROVIDER_ID,
                "provider_version": loom_ocr_macos::PROVIDER_VERSION,
                "model_version": format!("{}3", loom_ocr_macos::MODEL_FAMILY),
                "language": "auto", "image_width": 5_000, "image_height": 4_000,
                "encoded_width": 5_000, "encoded_height": 4_000, "orientation": 1,
                "scale_milli": 1_000, "region_count": 1, "confidence_threshold_milli": 800,
                "low_confidence_regions": 0, "confidence_state": "confirmed"
            }),
            warnings: vec![],
        };
        let budget = ExtractionBudget::for_media(MediaKind::Png);

        assert!(output.validate(MediaKind::Png, budget).is_err());
        output
            .validate_with_limits(MediaKind::Png, budget, 2_048, 100_000_000)
            .unwrap();
    }

    #[test]
    fn text_and_pdf_responses_refuse_nul_and_non_layout_control_characters() {
        for text in ["visible\u{0}", "visible\u{1b}", "visible\r"] {
            assert!(SourceOutput::Text { text: text.into() }
                .validate(
                    MediaKind::Text,
                    ExtractionBudget::for_media(MediaKind::Text)
                )
                .is_err());
            assert!(SourceOutput::Pdf {
                page_count: 1,
                pages: vec![(1, text.into())],
                warnings: vec![]
            }
            .validate(MediaKind::Pdf, ExtractionBudget::for_media(MediaKind::Pdf))
            .is_err());
        }
        SourceOutput::Text {
            text: "layout\n\t源".into(),
        }
        .validate(
            MediaKind::Text,
            ExtractionBudget::for_media(MediaKind::Text),
        )
        .unwrap();
    }
}
