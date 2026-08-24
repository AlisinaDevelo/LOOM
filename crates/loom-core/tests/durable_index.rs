use std::fs;

use loom_core::{IndexCheckpoint, Library, LibraryStats, LoomError, SearchRequest};
use tempfile::tempdir;

fn checkpoint(library: &Library, root: &std::path::Path) -> IndexCheckpoint {
    library
        .index_checkpoint(root)
        .expect("checkpoint query succeeds")
        .expect("indexing creates a checkpoint")
}

#[test]
fn interrupted_job_commits_units_and_resumes_idempotently() {
    let directory = tempdir().unwrap();
    let database = tempdir().unwrap();
    fs::write(
        directory.path().join("first.md"),
        "first durable recovery marker",
    )
    .unwrap();
    fs::write(
        directory.path().join("second.md"),
        "second durable recovery marker",
    )
    .unwrap();
    let library = Library::open(database.path().join("library.sqlite3")).unwrap();

    let interrupted = library.index_path_with_fault(directory.path(), Some(1));
    assert!(matches!(
        interrupted,
        Err(LoomError::IndexInterrupted(ref job_id)) if !job_id.is_empty()
    ));
    assert_eq!(
        library
            .search(&SearchRequest {
                text: "first durable".into(),
                limit: 10,
            })
            .unwrap()
            .len(),
        1
    );
    assert!(library
        .search(&SearchRequest {
            text: "second durable".into(),
            limit: 10,
        })
        .unwrap()
        .is_empty());
    assert_eq!(
        library.stats().unwrap(),
        LibraryStats {
            source_roots: 1,
            artifacts: 1,
            versions: 1,
            passages: 1,
            indexed_bytes: 29,
        }
    );

    let interrupted_checkpoint = checkpoint(&library, directory.path());
    assert_eq!(interrupted_checkpoint.state, "interrupted");
    assert_eq!(interrupted_checkpoint.next_unit, 1);
    assert_eq!(interrupted_checkpoint.total_units, 2);
    assert!(interrupted_checkpoint.last_error.is_some());

    let resumed = library.index_path(directory.path()).unwrap();
    assert_eq!(resumed.indexed, 1);
    assert_eq!(resumed.unchanged, 0);
    assert_eq!(resumed.failures, Vec::new());
    assert_eq!(
        library
            .search(&SearchRequest {
                text: "second durable".into(),
                limit: 10,
            })
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        checkpoint(&library, directory.path()),
        IndexCheckpoint {
            job_id: interrupted_checkpoint.job_id,
            state: "completed".into(),
            next_unit: 2,
            total_units: 2,
            last_error: None,
        }
    );

    let retry = library.index_path(directory.path()).unwrap();
    assert_eq!(retry.indexed, 0);
    assert_eq!(retry.unchanged, 2);
    assert_eq!(retry.failures, Vec::new());
    assert_eq!(library.stats().unwrap().versions, 2);
}
