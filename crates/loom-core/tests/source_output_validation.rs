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
