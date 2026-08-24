use std::fs;

use loom_core::{
    IndexCheckpoint, Library, LibraryStats, LoomError, ObservationEvent, ObservationEventKind,
    SearchRequest,
};
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

#[test]
fn approved_root_observation_reconciles_rename_delete_overflow_and_restart() {
    let directory = tempdir().unwrap();
    let database = tempdir().unwrap();
    let old = directory.path().join("old.md");
    let retained = directory.path().join("retained.md");
    let renamed = directory.path().join("renamed.md");
    fs::write(&old, "rename source marker").unwrap();
    fs::write(&retained, "delete source marker").unwrap();
    let library = Library::open(database.path().join("library.sqlite3")).unwrap();
    library.index_path(directory.path()).unwrap();

    fs::rename(&old, &renamed).unwrap();
    fs::remove_file(&retained).unwrap();
    let report = library
        .reconcile_events(
            directory.path(),
            &[
                ObservationEvent {
                    kind: ObservationEventKind::Renamed,
                    path: renamed.clone(),
                    previous_path: Some(old.clone()),
                },
                ObservationEvent {
                    kind: ObservationEventKind::Removed,
                    path: retained.clone(),
                    previous_path: None,
                },
                ObservationEvent {
                    kind: ObservationEventKind::Modified,
                    path: renamed.clone(),
                    previous_path: None,
                },
            ],
            20,
        )
        .unwrap();
    assert_eq!(report.roots_scanned, 1);
    assert_eq!(report.events_received, 3);
    assert_eq!(report.paths_coalesced, 3);
    assert_eq!(report.full_rescans, 1);
    assert!(report.failures.is_empty());
    assert!(library
        .search(&SearchRequest {
            text: "delete source".into(),
            limit: 10,
        })
        .unwrap()
        .is_empty());
    assert_eq!(
        library
            .search(&SearchRequest {
                text: "rename source".into(),
                limit: 10,
            })
            .unwrap()
            .len(),
        1
    );

    fs::write(directory.path().join("overflow.md"), "overflow marker").unwrap();
    let overflow = library
        .reconcile_events(
            directory.path(),
            &[ObservationEvent {
                kind: ObservationEventKind::Overflow,
                path: directory.path().to_path_buf(),
                previous_path: None,
            }],
            20,
        )
        .unwrap();
    assert_eq!(overflow.events_received, 1);
    assert_eq!(overflow.full_rescans, 1);
    assert!(
        library
            .search(&SearchRequest {
                text: "overflow marker".into(),
                limit: 10,
            })
            .unwrap()
            .len()
            == 1
    );

    drop(library);
    let restarted = Library::open(database.path().join("library.sqlite3")).unwrap();
    let startup = restarted.reconcile_approved_roots().unwrap();
    assert_eq!(startup.roots_scanned, 1);
    assert_eq!(startup.roots_failed, 0);
    assert_eq!(startup.full_rescans, 1);
    assert!(startup.failures.is_empty());
}

#[test]
fn event_reconciliation_rejects_unapproved_roots() {
    let approved = tempdir().unwrap();
    let unapproved = tempdir().unwrap();
    let database = tempdir().unwrap();
    fs::write(unapproved.path().join("secret.md"), "must not enter").unwrap();
    let library = Library::open(database.path().join("library.sqlite3")).unwrap();
    fs::write(approved.path().join("approved.md"), "approved marker").unwrap();
    library.index_path(approved.path()).unwrap();

    let result = library.reconcile_events(
        unapproved.path(),
        &[ObservationEvent {
            kind: ObservationEventKind::Modified,
            path: unapproved.path().join("secret.md"),
            previous_path: None,
        }],
        20,
    );
    assert!(
        matches!(result, Err(LoomError::InvalidPath(message)) if message.contains("not an enabled approved source"))
    );
    assert!(library
        .search(&SearchRequest {
            text: "must not enter".into(),
            limit: 10,
        })
        .unwrap()
        .is_empty());
}
