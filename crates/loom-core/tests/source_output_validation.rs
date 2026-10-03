use std::fs;

use loom_core::Library;
use tempfile::tempdir;

#[test]
fn foreground_rejects_forbidden_text_controls_but_accepts_layout_controls() {
    for (name, text) in [
        ("nul.md", "visible\u{0}text"),
        ("escape.md", "visible\u{1b}text"),
    ] {
        let directory = tempdir().unwrap();
        let source = directory.path().join(name);
        fs::write(&source, text).unwrap();
        let library = Library::open_in_memory().unwrap();

        let report = library.index_path(&source).unwrap();
        assert_eq!(report.indexed, 0);
        assert_eq!(report.failed, 1);
        assert_eq!(library.stats().unwrap().versions, 0);
    }

    let directory = tempdir().unwrap();
    let source = directory.path().join("layout.md");
    fs::write(&source, "first line\n\tsecond line\nthird line").unwrap();
    let library = Library::open_in_memory().unwrap();

    let report = library.index_path(&source).unwrap();
    assert_eq!(report.indexed, 1);
    assert_eq!(library.stats().unwrap().versions, 1);
}

#[test]
fn foreground_rejects_normalized_text_over_two_mebibytes() {
    const MAX_TEXT_BYTES: usize = 2 * 1024 * 1024;
    let directory = tempdir().unwrap();
    let source = directory.path().join("oversized.md");
    fs::write(&source, "x".repeat(MAX_TEXT_BYTES + 1)).unwrap();
    let library = Library::open_in_memory().unwrap();

    let report = library.index_path(&source).unwrap();
    assert_eq!(report.indexed, 0);
    assert_eq!(report.failed, 1);
    assert_eq!(library.stats().unwrap().versions, 0);
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn real_queued_extraction_matches_foreground_text_rejections() {
    use loom_core::{JobPriority, JobState};

    for (name, text) in [
        ("nul.md", "visible\u{0}text".to_string()),
        ("escape.md", "visible\u{1b}text".to_string()),
        ("oversized.md", "x".repeat(2 * 1024 * 1024 + 1)),
    ] {
        let directory = tempdir().unwrap();
        let source = directory.path().join(name);
        fs::write(&source, text).unwrap();
        let source = source.canonicalize().unwrap();
        let library = Library::open(directory.path().join("library.sqlite3")).unwrap();
        let foreground = library.index_path(&source).unwrap();
        assert_eq!(foreground.failed, 1);
        assert_eq!(library.stats().unwrap().versions, 0);

        let job = library
            .enqueue_index_file(&source, "reject-invalid-output", JobPriority::Normal)
            .unwrap();
        let mut worker = library
            .acquire_job_worker()
            .unwrap()
            .with_extractor_path(env!("CARGO_BIN_EXE_loom-core-test-extractor"))
            .unwrap();
        let result = worker.run_next().unwrap().unwrap();
        assert_eq!(result.id, job.id);
        assert_eq!(result.state, JobState::Failed);
        assert_eq!(library.stats().unwrap().versions, 0);
        assert_eq!(library.stats().unwrap().passages, 0);
    }
}
