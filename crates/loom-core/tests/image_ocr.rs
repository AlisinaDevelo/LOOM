use std::fs;

use loom_core::{EvidenceAnchor, Library, ResolveEvidenceRequest, SearchRequest};
use rusqlite::Connection;
use tempfile::tempdir;

const FIXTURE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/fixtures/ocr-golden.png"
));

#[test]
fn native_vision_ocr_records_provider_metadata_and_pixel_evidence() {
    if !cfg!(target_os = "macos") {
        return;
    }
    let directory = tempdir().unwrap();
    let source = directory.path().join("golden.png");
    let database = directory.path().join("library.sqlite3");
    fs::write(&source, FIXTURE).unwrap();

    let library = Library::open(&database).unwrap();
    let first = library.index_path(&source).unwrap();
    assert_eq!(first.indexed, 1, "this test requires native macOS Vision");
    assert!(
        first.failures.is_empty(),
        "OCR failed: {:?}",
        first.failures
    );

    let observation = library.inspect_source(&source).unwrap();
    assert_eq!(observation.extractor_id, "loom.ocr");
    assert_eq!(observation.extractor_version, "0.1.0");
    assert_eq!(
        observation.extraction_metadata["provider_id"],
        "macos.vision"
    );
    assert_eq!(
        observation.extraction_metadata["model_version"],
        "VNRecognizeTextRequestRevision3"
    );
    assert_eq!(observation.extraction_metadata["image_width"], 1200);
    assert_eq!(observation.extraction_metadata["image_height"], 600);
    assert_eq!(observation.passages.len(), 2);
    for passage in &observation.passages {
        let EvidenceAnchor::ImageRegion {
            x,
            y,
            width,
            height,
            image_width,
            image_height,
            orientation,
            scale_milli,
            confidence_milli,
            ..
        } = passage.anchor
        else {
            panic!("OCR passage lost its image-region anchor")
        };
        assert!(x < image_width && y < image_height);
        assert!(width > 0 && height > 0);
        assert!(x.saturating_add(width) <= image_width);
        assert!(y.saturating_add(height) <= image_height);
        assert_eq!(orientation, 1);
        assert_eq!(scale_milli, 1_000);
        assert!(confidence_milli > 0);
    }

    let hit = library
        .search(&SearchRequest {
            text: "LOOM OCR marker".into(),
            limit: 5,
        })
        .unwrap()
        .into_iter()
        .next()
        .expect("Vision text must be searchable");
    assert!(matches!(hit.anchor, EvidenceAnchor::ImageRegion { .. }));
    assert!(hit
        .excerpt
        .segments
        .iter()
        .map(|segment| segment.text.as_str())
        .collect::<String>()
        .contains("LOOM"));

    let evidence = library
        .resolve_verified_evidence(&ResolveEvidenceRequest {
            artifact_id: hit.artifact_id.clone(),
            version_id: hit.version_id.clone(),
            passage_id: hit.passage_id.clone(),
            content_hash: hit.content_hash.clone(),
        })
        .unwrap();
    assert_eq!(evidence.media_type, "image/png");
    assert!(matches!(
        evidence.anchor,
        EvidenceAnchor::ImageRegion {
            image_width: 1200,
            image_height: 600,
            ..
        }
    ));
    assert!(evidence.passage_text.contains("LOOM OCR marker"));

    let repeated = library.index_path(&source).unwrap();
    assert_eq!(repeated.unchanged, 1);
    assert_eq!(library.inspect_source(&source).unwrap(), observation);
}

#[test]
fn malformed_image_fails_closed_then_recovers_without_stale_ocr() {
    if !cfg!(target_os = "macos") {
        return;
    }
    let directory = tempdir().unwrap();
    let source = directory.path().join("recover.png");
    let database = directory.path().join("library.sqlite3");
    fs::write(&source, b"not an image").unwrap();

    let library = Library::open(&database).unwrap();
    let failed = library.index_path(&source).unwrap();
    assert_eq!(failed.failed, 1);
    assert!(failed.failures[0]
        .reason
        .contains("image extraction failed"));

    fs::write(&source, FIXTURE).unwrap();
    let recovered = library.index_path(&source).unwrap();
    assert_eq!(recovered.indexed, 1);
    assert!(library
        .search(&SearchRequest {
            text: "Evidence stays on this Mac".into(),
            limit: 5,
        })
        .unwrap()
        .iter()
        .any(|hit| matches!(hit.anchor, EvidenceAnchor::ImageRegion { .. })));
}

#[test]
fn disabling_ocr_purges_derived_records_and_reenable_recovers() {
    if !cfg!(target_os = "macos") {
        return;
    }
    let directory = tempdir().unwrap();
    let source = directory.path().join("policy.png");
    let database = directory.path().join("library.sqlite3");
    fs::write(&source, FIXTURE).unwrap();

    let library = Library::open(&database).unwrap();
    library.index_path(&source).unwrap();
    assert_eq!(library.ocr_status().unwrap().derived_versions, 1);
    assert_eq!(library.ocr_status().unwrap().derived_passages, 2);

    let purge = library.set_ocr_enabled(false).unwrap();
    assert_eq!(purge.artifacts_affected, 1);
    assert_eq!(purge.versions_deleted, 1);
    assert_eq!(purge.passages_deleted, 2);
    assert!(!library.ocr_status().unwrap().enabled);
    assert!(library
        .search(&SearchRequest {
            text: "LOOM OCR marker".into(),
            limit: 5,
        })
        .unwrap()
        .is_empty());

    let skipped = library.index_path(&source).unwrap();
    assert_eq!(skipped.skipped, 1);
    assert_eq!(skipped.failed, 0);
    assert!(matches!(
        library.inspect_source(&source),
        Err(loom_core::LoomError::ArtifactNotFound(_))
    ));
    drop(library);
    let library = Library::open(&database).unwrap();
    assert!(!library.ocr_status().unwrap().enabled);

    let connection = Connection::open(&database).unwrap();
    let enabled: String = connection
        .query_row(
            "SELECT value FROM schema_meta WHERE key = 'ocr_enabled'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(enabled, "0");
    drop(connection);

    library.set_ocr_enabled(true).unwrap();
    let recovered = library.index_path(&source).unwrap();
    assert_eq!(recovered.indexed, 1);
    assert_eq!(library.ocr_status().unwrap().derived_versions, 1);
    assert!(!library
        .search(&SearchRequest {
            text: "LOOM OCR marker".into(),
            limit: 5,
        })
        .unwrap()
        .is_empty());
}

#[test]
fn explicit_purge_keeps_source_locator_but_removes_ocr_rows() {
    if !cfg!(target_os = "macos") {
        return;
    }
    let directory = tempdir().unwrap();
    let source = directory.path().join("purge.png");
    let database = directory.path().join("library.sqlite3");
    fs::write(&source, FIXTURE).unwrap();
    let library = Library::open(&database).unwrap();
    library.index_path(&source).unwrap();

    let purge = library.purge_ocr_records().unwrap();
    assert_eq!(purge.versions_deleted, 1);
    assert_eq!(purge.passages_deleted, 2);
    assert_eq!(library.ocr_status().unwrap().derived_versions, 0);
    assert!(library
        .source_roots()
        .unwrap()
        .iter()
        .any(|root| root.enabled));
}
