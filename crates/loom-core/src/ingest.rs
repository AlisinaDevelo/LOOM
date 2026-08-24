use std::{
    fs::{self, File, Metadata, OpenOptions},
    io::Read,
    panic::{catch_unwind, AssertUnwindSafe},
    path::{Path, PathBuf},
    time::SystemTime,
};

#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

use walkdir::WalkDir;

use crate::{
    domain::EvidenceAnchor,
    error::{io_error, LoomError, Result},
    ocr::{self, ImageOcrRegion},
};

pub(crate) const EXTRACTOR_ID: &str = "loom.text";
pub(crate) const EXTRACTOR_VERSION: &str = "0.1.0";
pub(crate) const PDF_EXTRACTOR_ID: &str = "loom.pdf";
pub(crate) const PDF_EXTRACTOR_VERSION: &str = "0.1.0";
#[cfg(test)]
const DEFAULT_MAX_PDF_PAGES: usize = 2_048;

#[derive(Debug)]
pub(crate) struct StableDocument {
    pub(crate) raw_hash: String,
    pub(crate) byte_size: u64,
    pub(crate) modified_ns: Option<i64>,
    pub(crate) normalized_text: String,
    pub(crate) media_type: &'static str,
    pub(crate) pdf_pages: Option<Vec<(u32, String)>>,
    pub(crate) page_count: Option<u32>,
    pub(crate) parse_warnings: Vec<String>,
    pub(crate) image_regions: Option<Vec<ImageOcrRegion>>,
    pub(crate) extraction_metadata: serde_json::Value,
}

#[derive(Debug)]
pub(crate) struct PassageDraft {
    pub(crate) ordinal: u32,
    pub(crate) text: String,
    pub(crate) text_hash: String,
    pub(crate) anchor: EvidenceAnchor,
}

pub(crate) fn discover(path: &Path, max_files: usize) -> Result<Vec<PathBuf>> {
    let metadata = fs::symlink_metadata(path).map_err(|source| io_error(path, source))?;
    if metadata.file_type().is_symlink() {
        return Err(LoomError::InvalidPath(format!(
            "symbolic links are not followed: {}",
            path.display()
        )));
    }

    if metadata.is_file() {
        return Ok(vec![path.to_path_buf()]);
    }
    if !metadata.is_dir() {
        return Err(LoomError::InvalidPath(format!(
            "not a regular file or directory: {}",
            path.display()
        )));
    }

    let mut files = Vec::new();
    for entry in WalkDir::new(path).follow_links(false).sort_by_file_name() {
        let entry = entry.map_err(|error| {
            let error_path = error.path().unwrap_or(path).to_path_buf();
            io_error(
                error_path,
                error
                    .into_io_error()
                    .unwrap_or_else(|| std::io::Error::other("directory traversal failed")),
            )
        })?;
        if entry.file_type().is_symlink() || !entry.file_type().is_file() {
            continue;
        }
        files.push(entry.into_path());
        if files.len() > max_files {
            return Err(LoomError::InvalidPath(format!(
                "source contains more than the {max_files}-file request limit: {}",
                path.display()
            )));
        }
    }
    Ok(files)
}

pub(crate) fn supported_media_type(path: &Path) -> Option<&'static str> {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("txt") => Some("text/plain"),
        Some("md" | "markdown") => Some("text/markdown"),
        Some("pdf") => Some("application/pdf"),
        Some("png") => Some("image/png"),
        Some("jpg" | "jpeg") => Some("image/jpeg"),
        Some("gif") => Some("image/gif"),
        Some("webp") => Some("image/webp"),
        _ => None,
    }
}

#[cfg(test)]
pub(crate) fn read_stable(path: &Path, root: &Path, max_bytes: u64) -> Result<StableDocument> {
    read_stable_with_limits(path, root, max_bytes, DEFAULT_MAX_PDF_PAGES)
}

#[cfg(test)]
pub(crate) fn read_stable_with_limits(
    path: &Path,
    root: &Path,
    max_bytes: u64,
    max_pdf_pages: usize,
) -> Result<StableDocument> {
    read_stable_with_limits_and_ocr(path, root, max_bytes, max_pdf_pages, true, None)
}

pub(crate) fn read_stable_with_limits_and_ocr(
    path: &Path,
    root: &Path,
    max_bytes: u64,
    max_pdf_pages: usize,
    ocr_enabled: bool,
    capture_metadata: Option<&serde_json::Value>,
) -> Result<StableDocument> {
    let media_type = supported_media_type(path)
        .ok_or_else(|| LoomError::UnsupportedSource(path.display().to_string()))?;

    let stable = read_stable_bytes(path, root, max_bytes)?;
    let raw_hash = format!("blake3:{}", blake3::hash(&stable.bytes).to_hex());
    if media_type == "application/pdf" {
        let extraction = extract_pdf_pages(&stable.bytes, path, max_pdf_pages)?;
        let normalized_text = extraction
            .pages
            .iter()
            .map(|(_, text)| text.as_str())
            .collect::<Vec<_>>()
            .join("\n\n");
        return Ok(StableDocument {
            raw_hash,
            byte_size: stable.bytes.len() as u64,
            modified_ns: stable.modified_ns,
            normalized_text,
            media_type,
            page_count: Some(extraction.page_count),
            pdf_pages: Some(extraction.pages),
            parse_warnings: extraction.warnings,
            image_regions: None,
            extraction_metadata: serde_json::json!({}),
        });
    }
    if media_type.starts_with("image/") {
        if !ocr_enabled {
            return Err(LoomError::OcrDisabled);
        }
        let extraction = ocr::extract_image(&stable.bytes)?;
        return Ok(StableDocument {
            raw_hash,
            byte_size: stable.bytes.len() as u64,
            modified_ns: stable.modified_ns,
            normalized_text: extraction.normalized_text,
            media_type,
            pdf_pages: None,
            page_count: None,
            parse_warnings: extraction.warnings,
            image_regions: Some(extraction.regions),
            extraction_metadata: with_capture_metadata(extraction.metadata, capture_metadata),
        });
    }
    let text = String::from_utf8(stable.bytes).map_err(|_| {
        LoomError::InvalidPath(format!("source is not UTF-8 text: {}", path.display()))
    })?;
    let normalized_text = text.replace("\r\n", "\n").replace('\r', "\n");
    Ok(StableDocument {
        raw_hash,
        byte_size: text.len() as u64,
        modified_ns: stable.modified_ns,
        normalized_text,
        media_type,
        pdf_pages: None,
        page_count: None,
        parse_warnings: Vec::new(),
        image_regions: None,
        extraction_metadata: serde_json::json!({}),
    })
}

fn with_capture_metadata(
    mut extraction_metadata: serde_json::Value,
    capture_metadata: Option<&serde_json::Value>,
) -> serde_json::Value {
    let Some(capture_metadata) = capture_metadata else {
        return extraction_metadata;
    };
    if let Some(object) = extraction_metadata.as_object_mut() {
        object.insert("capture".into(), capture_metadata.clone());
        extraction_metadata
    } else {
        serde_json::json!({
            "extractor": extraction_metadata,
            "capture": capture_metadata,
        })
    }
}

struct PdfExtraction {
    page_count: u32,
    pages: Vec<(u32, String)>,
    warnings: Vec<String>,
}

fn extract_pdf_pages(bytes: &[u8], path: &Path, max_pdf_pages: usize) -> Result<PdfExtraction> {
    let extraction = catch_unwind(AssertUnwindSafe(|| {
        if bytes
            .windows(b"/Encrypt".len())
            .any(|window| window == b"/Encrypt")
        {
            return Err(LoomError::PdfExtraction(format!(
                "encrypted PDF requires an explicit password and was not indexed: {}",
                path.display()
            )));
        }
        let document = pdf_extract::Document::load_mem(bytes).map_err(|error| {
            LoomError::PdfExtraction(format!("malformed PDF at {}: {error}", path.display()))
        })?;
        if document.is_encrypted() {
            return Err(LoomError::PdfExtraction(format!(
                "encrypted PDF requires an explicit password and was not indexed: {}",
                path.display()
            )));
        }
        let page_numbers = document.get_pages().keys().copied().collect::<Vec<_>>();
        if page_numbers.is_empty() {
            return Err(LoomError::PdfExtraction(format!(
                "PDF has no pages: {}",
                path.display()
            )));
        }
        if page_numbers.len() > max_pdf_pages {
            return Err(LoomError::PdfExtraction(format!(
                "PDF has {} pages, exceeding the {max_pdf_pages}-page limit: {}",
                page_numbers.len(),
                path.display()
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
            let text = normalize_pdf_text(&text);
            if text.trim().is_empty() {
                warnings.push(format!("page {page} contains no extractable text"));
            }
            pages.push((page, text));
        }
        if pages.iter().all(|(_, text)| text.trim().is_empty()) {
            return Err(LoomError::PdfExtraction(format!(
                "PDF contains no extractable text (image-only or unsupported fonts): {}",
                path.display()
            )));
        }
        Ok(PdfExtraction {
            page_count: pages.len() as u32,
            pages,
            warnings,
        })
    }))
    .map_err(|_| {
        LoomError::PdfExtraction(format!(
            "PDF parser rejected malformed input without a recoverable error: {}",
            path.display()
        ))
    })?;
    extraction
}

fn normalize_pdf_text(text: &str) -> String {
    text.replace("\r\n", "\n")
        .replace('\r', "\n")
        .trim_matches('\n')
        .to_string()
}

/// Reads a source through a no-follow descriptor and verifies that it stayed within `root`.
///
/// The path is checked before and after the descriptor read. On Unix, the descriptor is opened
/// with `O_NOFOLLOW` and its device/inode identity is compared with the path metadata. This does
/// not make arbitrary external filesystem mutation impossible, but it closes the usual symlink
/// replacement and path-rebinding windows without requiring unsafe `openat` bindings.
pub(crate) fn read_stable_hash(path: &Path, root: &Path, max_bytes: u64) -> Result<String> {
    let stable = read_stable_bytes(path, root, max_bytes)?;
    Ok(format!("blake3:{}", blake3::hash(&stable.bytes).to_hex()))
}

struct StableBytes {
    bytes: Vec<u8>,
    modified_ns: Option<i64>,
}

fn read_stable_bytes(path: &Path, root: &Path, max_bytes: u64) -> Result<StableBytes> {
    for _ in 0..3 {
        match read_stable_bytes_once(path, root, max_bytes) {
            Err(LoomError::SourceChanged(_)) => continue,
            result => return result,
        }
    }
    Err(LoomError::SourceChanged(path.display().to_string()))
}

fn read_stable_bytes_once(path: &Path, root: &Path, max_bytes: u64) -> Result<StableBytes> {
    let canonical_root = fs::canonicalize(root).map_err(|source| io_error(root, source))?;
    let before_path = fs::symlink_metadata(path).map_err(|source| io_error(path, source))?;
    if before_path.file_type().is_symlink() || !before_path.is_file() {
        return Err(LoomError::InvalidPath(format!(
            "source is not a regular non-symlink file: {}",
            path.display()
        )));
    }
    let canonical_before = fs::canonicalize(path).map_err(|source| io_error(path, source))?;
    ensure_within_root(&canonical_root, &canonical_before, path)?;

    let file =
        open_readonly_no_follow(&canonical_before).map_err(|source| io_error(path, source))?;
    let before_file = file.metadata().map_err(|source| io_error(path, source))?;
    if !metadata_matches(&before_path, &before_file) {
        return Err(LoomError::SourceChanged(path.display().to_string()));
    }
    if before_file.len() > max_bytes {
        return Err(LoomError::InvalidPath(format!(
            "source exceeds the {max_bytes}-byte limit: {}",
            path.display()
        )));
    }

    let capacity = usize::try_from(before_file.len()).map_err(|_| {
        LoomError::InvalidPath(format!(
            "source size exceeds the platform memory range: {}",
            path.display()
        ))
    })?;
    let mut bytes = Vec::with_capacity(capacity);
    (&file)
        .take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|source| io_error(path, source))?;

    let after_file = file.metadata().map_err(|source| io_error(path, source))?;
    let after_path = fs::symlink_metadata(path).map_err(|source| io_error(path, source))?;
    if after_path.file_type().is_symlink()
        || !metadata_matches(&before_file, &after_file)
        || !metadata_matches(&after_path, &after_file)
        || bytes.len() as u64 > max_bytes
    {
        if bytes.len() as u64 > max_bytes && metadata_matches(&before_file, &after_file) {
            return Err(LoomError::InvalidPath(format!(
                "source exceeds the {max_bytes}-byte limit: {}",
                path.display()
            )));
        }
        return Err(LoomError::SourceChanged(path.display().to_string()));
    }
    let canonical_after = fs::canonicalize(path).map_err(|source| io_error(path, source))?;
    ensure_within_root(&canonical_root, &canonical_after, path)?;

    Ok(StableBytes {
        bytes,
        modified_ns: modified_ns(after_file.modified().ok()),
    })
}

fn ensure_within_root(root: &Path, candidate: &Path, original: &Path) -> Result<()> {
    if candidate == root || candidate.starts_with(root) {
        Ok(())
    } else {
        Err(LoomError::InvalidPath(format!(
            "source escapes the selected root: {}",
            original.display()
        )))
    }
}

fn open_readonly_no_follow(path: &Path) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW);
    options.open(path)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileIdentity {
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
}

fn file_identity(metadata: &Metadata) -> FileIdentity {
    #[cfg(unix)]
    {
        FileIdentity {
            device: metadata.dev(),
            inode: metadata.ino(),
        }
    }
    #[cfg(not(unix))]
    {
        FileIdentity {}
    }
}

fn metadata_matches(left: &Metadata, right: &Metadata) -> bool {
    left.is_file()
        && right.is_file()
        && left.len() == right.len()
        && modified(left) == modified(right)
        && file_identity(left) == file_identity(right)
}

fn modified(metadata: &fs::Metadata) -> Option<SystemTime> {
    metadata.modified().ok()
}

fn modified_ns(value: Option<SystemTime>) -> Option<i64> {
    value
        .and_then(|time| time.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos().min(i64::MAX as u128) as i64)
}

pub(crate) fn split_passages(
    text: &str,
    target_chars: usize,
    overlap_chars: usize,
) -> Vec<PassageDraft> {
    let characters: Vec<char> = text.chars().collect();
    if characters.is_empty() {
        return Vec::new();
    }

    let mut drafts = Vec::new();
    let mut start = 0usize;
    let mut ordinal = 0u32;
    while start < characters.len() {
        let hard_end = start.saturating_add(target_chars).min(characters.len());
        let mut end = hard_end;
        if hard_end < characters.len() {
            let soft_floor = start.saturating_add(target_chars.saturating_mul(3) / 5);
            if let Some(relative) = characters[soft_floor..hard_end]
                .iter()
                .rposition(|character| character.is_whitespace())
            {
                end = soft_floor + relative + 1;
            }
        }
        if end <= start {
            end = hard_end.max(start + 1);
        }

        let passage_text: String = characters[start..end].iter().collect();
        if !passage_text.trim().is_empty() {
            let line_start = 1 + characters[..start]
                .iter()
                .filter(|character| **character == '\n')
                .count();
            let line_end = line_start
                + passage_text
                    .chars()
                    .filter(|character| *character == '\n')
                    .count();
            drafts.push(PassageDraft {
                ordinal,
                text_hash: format!("blake3:{}", blake3::hash(passage_text.as_bytes()).to_hex()),
                text: passage_text,
                anchor: EvidenceAnchor::Text {
                    char_start: start as u64,
                    char_end: end as u64,
                    line_start: line_start as u64,
                    line_end: line_end as u64,
                },
            });
            ordinal += 1;
        }

        if end == characters.len() {
            break;
        }
        start = end.saturating_sub(overlap_chars.min(end - start - 1));
    }
    drafts
}

pub(crate) fn split_pdf_passages(
    pages: &[(u32, String)],
    target_chars: usize,
    overlap_chars: usize,
) -> Vec<PassageDraft> {
    let mut passages = Vec::new();
    let mut ordinal = 0u32;
    for (page, text) in pages {
        for mut passage in split_passages(text, target_chars, overlap_chars) {
            let EvidenceAnchor::Text {
                char_start,
                char_end,
                line_start,
                line_end,
            } = passage.anchor
            else {
                unreachable!("text segmentation always emits text anchors")
            };
            passage.ordinal = ordinal;
            passage.anchor = EvidenceAnchor::PdfPage {
                page: *page,
                char_start,
                char_end,
                line_start,
                line_end,
            };
            ordinal = ordinal.saturating_add(1);
            passages.push(passage);
        }
    }
    passages
}

pub(crate) fn split_image_passages(regions: &[ImageOcrRegion]) -> Vec<PassageDraft> {
    regions
        .iter()
        .enumerate()
        .map(|(ordinal, region)| PassageDraft {
            ordinal: ordinal as u32,
            text: region.text.clone(),
            text_hash: format!("blake3:{}", blake3::hash(region.text.as_bytes()).to_hex()),
            anchor: EvidenceAnchor::ImageRegion {
                char_start: region.char_start,
                char_end: region.char_end,
                line_start: region.line_start,
                line_end: region.line_end,
                x: region.bounds.x,
                y: region.bounds.y,
                width: region.bounds.width,
                height: region.bounds.height,
                image_width: region.image_width,
                image_height: region.image_height,
                orientation: region.orientation,
                scale_milli: region.scale_milli,
                confidence_milli: region.confidence_milli,
            },
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::{discover, read_stable_hash, split_passages, split_pdf_passages};
    use crate::{EvidenceAnchor, LoomError};

    #[test]
    fn discovery_fails_closed_when_the_file_limit_is_exceeded() {
        let directory = tempdir().unwrap();
        for name in ["a.md", "b.md", "c.md"] {
            fs::write(directory.path().join(name), name).unwrap();
        }

        let error = discover(directory.path(), 2).unwrap_err();
        assert!(matches!(error, LoomError::InvalidPath(_)));
        assert!(error.to_string().contains("2-file request limit"));
    }

    #[test]
    fn passage_offsets_cover_unicode_without_splitting_characters() {
        let text = "alpha βeta\n".repeat(40);
        let passages = split_passages(&text, 80, 8);
        assert!(passages.len() > 1);
        for passage in passages {
            let EvidenceAnchor::Text {
                char_start,
                char_end,
                ..
            } = passage.anchor
            else {
                unreachable!("text segmentation always emits text anchors")
            };
            let recovered: String = text
                .chars()
                .skip(char_start as usize)
                .take((char_end - char_start) as usize)
                .collect();
            assert_eq!(recovered, passage.text);
        }
    }

    #[test]
    fn pdf_passage_anchors_preserve_page_and_local_offsets() {
        let pages = vec![
            (1, "first page evidence".to_string()),
            (2, "second page evidence".to_string()),
        ];
        let passages = split_pdf_passages(&pages, 80, 8);
        assert_eq!(passages.len(), 2);
        assert_eq!(passages[0].ordinal, 0);
        assert_eq!(passages[1].ordinal, 1);
        assert_eq!(
            passages[0].anchor,
            EvidenceAnchor::PdfPage {
                page: 1,
                char_start: 0,
                char_end: 19,
                line_start: 1,
                line_end: 1,
            }
        );
        assert_eq!(
            passages[1].anchor,
            EvidenceAnchor::PdfPage {
                page: 2,
                char_start: 0,
                char_end: 20,
                line_start: 1,
                line_end: 1,
            }
        );
    }

    #[cfg(unix)]
    #[test]
    fn stable_reads_reject_symlinks_and_paths_outside_root() {
        use std::os::unix::fs::symlink;

        let directory = tempdir().unwrap();
        let outside = tempdir().unwrap();
        let outside_source = outside.path().join("outside.md");
        fs::write(&outside_source, "outside the selected root").unwrap();
        let symlink_path = directory.path().join("linked.md");
        symlink(&outside_source, &symlink_path).unwrap();

        assert!(read_stable_hash(&symlink_path, directory.path(), 1024).is_err());
        assert!(read_stable_hash(&outside_source, directory.path(), 1024).is_err());
    }
}
