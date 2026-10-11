use std::fs;

use loom_core::{IndexCancellationToken, Library, LibraryStats, SearchRequest};
use tempfile::tempdir;

#[test]
fn cancellation_commits_complete_units_and_resumes_to_uninterrupted_rows() {
    let root = tempdir().unwrap();
    let database = tempdir().unwrap();
    let sources = [
        ("01-first.md", "first cancellation marker"),
        ("02-second.md", "second cancellation marker"),
        ("03-third.md", "third cancellation marker"),
    ];
    for (name, text) in sources {
        fs::write(root.path().join(name), text).unwrap();
    }

    let library = Library::open(database.path().join("library.sqlite3")).unwrap();
    let cancelled = library
        .index_path_with_cancellation_after(root.path(), 1)
        .unwrap();
    assert!(!cancelled.run_id.is_empty());
    assert_eq!(cancelled.discovered, 3);
    assert_eq!(cancelled.attempted, 1);
    assert_eq!(cancelled.indexed, 1);
    assert_eq!(cancelled.unchanged, 0);
    assert_eq!(cancelled.skipped, 0);
    assert_eq!(cancelled.failed, 0);
    assert_eq!(cancelled.cancelled, 2);
    assert!(cancelled.failures.is_empty());

    assert_eq!(marker_hits(&library, "first cancellation"), 1);
    assert_eq!(marker_hits(&library, "second cancellation"), 0);
    let interrupted = library
        .index_checkpoint(root.path())
        .unwrap()
        .expect("cancellation must retain a checkpoint");
    assert_eq!(interrupted.job_id, cancelled.run_id);
    assert_eq!(interrupted.state, "interrupted");
    assert_eq!(interrupted.next_unit, 1);
    assert_eq!(interrupted.total_units, 3);
    assert_eq!(
        interrupted.last_error.as_deref(),
        Some("cancelled by request")
    );
    assert_eq!(library.stats().unwrap().artifacts, 1);

    let resumed = library.index_path(root.path()).unwrap();
    assert_eq!(resumed.run_id, cancelled.run_id);
    assert_eq!(resumed.discovered, 3);
    assert_eq!(resumed.attempted, 2);
    assert_eq!(resumed.indexed, 2);
    assert_eq!(resumed.cancelled, 0);
    assert_eq!(resumed.failed, 0);
    assert!(resumed.failures.is_empty());
    assert_eq!(
        library
            .index_checkpoint(root.path())
            .unwrap()
            .unwrap()
            .state,
        "completed"
    );
    assert_eq!(library.stats().unwrap().artifacts, 3);

    let uninterrupted = Library::open_in_memory().unwrap();
    let complete = uninterrupted.index_path(root.path()).unwrap();
    assert_eq!(complete.attempted, 3);
    assert_eq!(complete.cancelled, 0);
    let expected_bytes = sources.iter().map(|(_, text)| text.len() as u64).sum();
    assert_eq!(
        library.stats().unwrap(),
        LibraryStats {
            source_roots: 1,
            artifacts: 3,
            versions: 3,
            passages: 3,
            indexed_bytes: expected_bytes,
        }
    );
    for (name, _) in sources {
        let source = root.path().join(name);
        assert_eq!(
            library.inspect_source(&source).unwrap(),
            uninterrupted.inspect_source(&source).unwrap()
        );
    }
}

#[test]
fn resumed_scan_revalidates_an_edited_completed_prefix() {
    let root = tempdir().unwrap();
    let database = tempdir().unwrap();
    let unchanged = root.path().join("01-unchanged.md");
    let edited = root.path().join("02-edited.md");
    fs::write(&unchanged, "unchangedprefix evidence").unwrap();
    fs::write(&edited, "oldprefix evidence").unwrap();
    fs::write(root.path().join("03-pending.md"), "pendingprefix evidence").unwrap();
    let library = Library::open(database.path().join("library.sqlite3")).unwrap();
    let cancelled = library
        .index_path_with_cancellation_after(root.path(), 2)
        .unwrap();
    assert_eq!(cancelled.indexed, 2);
    let unchanged_before = library.inspect_source(&unchanged).unwrap();
    let edited_before = library.inspect_source(&edited).unwrap();
    let metadata = fs::metadata(&edited).unwrap();
    let modified = metadata.modified().unwrap();
    fs::write(&edited, "newprefix evidence").unwrap();
    fs::File::open(&edited)
        .unwrap()
        .set_modified(modified)
        .unwrap();
    assert_eq!(fs::metadata(&edited).unwrap().len(), metadata.len());
    assert_eq!(fs::metadata(&edited).unwrap().modified().unwrap(), modified);

    let resumed = library.index_path(root.path()).unwrap();
    assert_eq!(marker_hits(&library, "newprefix"), 1);
    assert_eq!(marker_hits(&library, "oldprefix"), 0);
    assert_eq!(marker_hits(&library, "pendingprefix"), 1);
    assert_eq!(resumed.run_id, cancelled.run_id);
    assert_eq!(resumed.attempted, 2);
    assert_eq!(resumed.indexed, 2);
    assert!(resumed.failures.is_empty());
    assert_eq!(
        library.inspect_source(&unchanged).unwrap(),
        unchanged_before
    );
    assert_ne!(
        library.inspect_source(&edited).unwrap().content_hash,
        edited_before.content_hash
    );
    assert_eq!(library.stats().unwrap().versions, 4);
    assert_eq!(library.index_path(root.path()).unwrap().unchanged, 3);
    assert_eq!(library.stats().unwrap().versions, 4);
}

#[test]
fn resumed_scan_retries_a_recovered_completed_failure() {
    let root = tempdir().unwrap();
    let database = tempdir().unwrap();
    let recovered = root.path().join("01-recovered.md");
    fs::write(&recovered, [0xff]).unwrap();
    fs::write(
        root.path().join("02-pending.md"),
        "pending recovery evidence",
    )
    .unwrap();
    let library = Library::open(database.path().join("library.sqlite3")).unwrap();
    let cancelled = library
        .index_path_with_cancellation_after(root.path(), 1)
        .unwrap();
    assert_eq!(cancelled.failed, 1);
    fs::write(&recovered, "recoveredprefix evidence").unwrap();

    let resumed = library.index_path(root.path()).unwrap();
    assert_eq!(marker_hits(&library, "recoveredprefix"), 1);
    assert_eq!(resumed.run_id, cancelled.run_id);
    assert_eq!(resumed.attempted, 2);
    assert_eq!(resumed.indexed, 2);
    assert!(resumed.failures.is_empty());
}

#[test]
fn resumed_scan_keeps_unsupported_and_ocr_disabled_prefix_units_skipped() {
    let root = tempdir().unwrap();
    let database = tempdir().unwrap();
    fs::write(
        root.path().join("01-unsupported.bin"),
        "unsupported evidence",
    )
    .unwrap();
    fs::write(
        root.path().join("02-disabled.png"),
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/ocr-golden.png"
        )),
    )
    .unwrap();
    let pending_text = "pendingprefix evidence";
    fs::write(root.path().join("03-pending.md"), pending_text).unwrap();
    let library = Library::open(database.path().join("library.sqlite3")).unwrap();
    library.set_ocr_enabled(false).unwrap();
    let cancelled = library
        .index_path_with_cancellation_after(root.path(), 2)
        .unwrap();
    assert_eq!(cancelled.skipped, 2);
    assert_eq!(cancelled.bytes_read, 0);

    let resumed = library.index_path(root.path()).unwrap();
    assert_eq!(resumed.run_id, cancelled.run_id);
    assert_eq!(resumed.attempted, 1);
    assert_eq!(resumed.indexed, 1);
    assert_eq!(resumed.skipped, 0);
    assert_eq!(resumed.bytes_read, pending_text.len() as u64);
    assert!(resumed.failures.is_empty());
    assert_eq!(library.ocr_status().unwrap().derived_versions, 0);
}

#[test]
fn pre_cancelled_resume_keeps_the_completed_prefix_and_checkpoint() {
    let root = tempdir().unwrap();
    let database = tempdir().unwrap();
    let first = root.path().join("01-first.md");
    fs::write(&first, "oldprefix evidence").unwrap();
    fs::write(root.path().join("02-pending.md"), "pendingprefix evidence").unwrap();
    let library = Library::open(database.path().join("library.sqlite3")).unwrap();
    library
        .index_path_with_cancellation_after(root.path(), 1)
        .unwrap();
    let before = library.index_checkpoint(root.path()).unwrap();
    let canonical_before = library.export_portable().unwrap().tables;
    fs::write(&first, "newprefix evidence").unwrap();
    let token = IndexCancellationToken::new();
    token.cancel();

    let report = library
        .index_path_with_cancellation(root.path(), &token)
        .unwrap();
    assert_eq!(report.attempted, 0);
    assert_eq!(report.bytes_read, 0);
    assert_eq!(report.cancelled, 1);
    assert_eq!(library.index_checkpoint(root.path()).unwrap(), before);
    // Explicit selection may refresh root last-seen time; canonical evidence stays untouched.
    let after = library.export_portable().unwrap().tables;
    for table in [
        "artifacts",
        "artifact_locators",
        "artifact_versions",
        "passages",
    ] {
        assert_eq!(after[table], canonical_before[table]);
    }
}

#[test]
fn pre_cancelled_token_reports_all_remaining_units_without_writing() {
    let root = tempdir().unwrap();
    let database = tempdir().unwrap();
    fs::write(root.path().join("one.md"), "pre-cancelled marker").unwrap();
    fs::write(root.path().join("two.md"), "another pre-cancelled marker").unwrap();

    let library = Library::open(database.path().join("library.sqlite3")).unwrap();
    let token = IndexCancellationToken::new();
    token.cancel();
    let report = library
        .index_path_with_cancellation(root.path(), &token)
        .unwrap();

    assert!(!report.run_id.is_empty());
    assert_eq!(report.discovered, 2);
    assert_eq!(report.attempted, 0);
    assert_eq!(report.indexed, 0);
    assert_eq!(report.cancelled, 2);
    assert_eq!(report.failed, 0);
    assert!(report.failures.is_empty());
    assert_eq!(library.stats().unwrap().artifacts, 0);
    let checkpoint = library.index_checkpoint(root.path()).unwrap().unwrap();
    assert_eq!(checkpoint.state, "interrupted");
    assert_eq!(checkpoint.next_unit, 0);
    assert_eq!(
        checkpoint.last_error.as_deref(),
        Some("cancelled by request")
    );
}

fn marker_hits(library: &Library, query: &str) -> usize {
    library
        .search(&SearchRequest {
            text: query.into(),
            limit: 10,
        })
        .unwrap()
        .len()
}
