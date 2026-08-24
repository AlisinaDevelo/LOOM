use std::fs;

use loom_core::{CaptureBounds, CaptureContext, CaptureMode, Library, SearchRequest};
use tempfile::tempdir;

const FIXTURE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/fixtures/ocr-golden.png"
));

#[test]
fn intentional_capture_metadata_is_recorded_before_ocr_and_duplicates_are_stable() {
    if !cfg!(target_os = "macos") {
        return;
    }
    let directory = tempdir().unwrap();
    let source = directory.path().join("capture.png");
    let database = directory.path().join("library.sqlite3");
    fs::write(&source, FIXTURE).unwrap();
    let context = CaptureContext {
        mode: CaptureMode::Region,
        captured_at: "2026-08-24T20:00:00Z".into(),
        display_scale_milli: 2_000,
        bounds: CaptureBounds {
            x: 12,
            y: 24,
            width: 1200,
            height: 600,
        },
        app_name: Some("Safari".into()),
        window_title: Some("LOOM capture test".into()),
        source: "macOS screencapture".into(),
    };

    let library = Library::open(&database).unwrap();
    let first = library.index_captured_image(&source, &context).unwrap();
    assert_eq!(first.indexed, 1);
    let observation = library.inspect_source(&source).unwrap();
    assert_eq!(observation.extraction_metadata["capture"]["mode"], "region");
    assert_eq!(
        observation.extraction_metadata["capture"]["display_scale_milli"],
        2_000
    );
    assert_eq!(
        observation.extraction_metadata["capture"]["bounds"]["width"],
        1_200
    );
    assert_eq!(
        observation.extraction_metadata["capture"]["app_name"],
        "Safari"
    );

    let repeated = library.index_captured_image(&source, &context).unwrap();
    assert_eq!(repeated.unchanged, 1);
    assert_eq!(library.inspect_source(&source).unwrap(), observation);

    let purge = library
        .purge_source_root(&source.canonicalize().unwrap().to_string_lossy())
        .unwrap();
    assert_eq!(purge.artifacts_deleted, 1);
    assert_eq!(purge.versions_deleted, 1);
    assert_eq!(purge.passages_deleted, 2);
    assert!(library
        .search(&SearchRequest {
            text: "LOOM OCR marker".into(),
            limit: 5,
        })
        .unwrap()
        .is_empty());
}
