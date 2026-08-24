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
