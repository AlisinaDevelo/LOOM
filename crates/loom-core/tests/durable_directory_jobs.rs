use std::fs;

use loom_core::{JobPriority, JobState, JobWorker, Library, LoomError, SearchRequest};
use tempfile::{tempdir, TempDir};

fn fixture() -> (TempDir, Library, std::path::PathBuf) {
    let temporary = tempdir().unwrap();
    let root = temporary.path().join("selected");
    fs::create_dir(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let library = Library::open(temporary.path().join("library.sqlite3")).unwrap();
    library.set_ocr_enabled(false).unwrap();
    library.index_path(&root).unwrap(); // Explicitly approve an empty directory, not its parent.
    (temporary, library, root)
}

fn worker(library: &Library) -> JobWorker {
    let started = std::time::Instant::now();
    loop {
        match library.acquire_job_worker() {
            Err(LoomError::JobWorkerBusy)
                if started.elapsed() < std::time::Duration::from_millis(500) =>
            {
                // A just-released fixture FD can be transiently inherited by a parallel helper.
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            result => {
                return result
                    .unwrap()
                    .with_extractor_path(env!("CARGO_BIN_EXE_loom-core-test-extractor"))
                    .unwrap()
            }
        }
    }
}

#[test]
fn directory_quanta_are_durable_fair_and_do_not_spend_successful_retry_attempts() {
    let (_temporary, library, root) = fixture();
    for index in 0..7 {
        fs::write(
            root.join(format!("{index}.md")),
            format!("Quantum marker {index}"),
        )
        .unwrap();
    }
    let job = library
        .enqueue_index_directory(&root, "directory", JobPriority::Normal)
        .unwrap();
    assert_eq!(job.directory_progress.as_ref().unwrap().total_units, 7);
    assert!(matches!(
        library.enqueue_index_file(&root, "wrong", JobPriority::Normal),
        Err(LoomError::InvalidPath(_))
    ));
    for expected in 1..=7 {
        let mut worker = worker(&library);
        let settled = worker.run_next().unwrap().unwrap();
        assert_eq!(settled.id, job.id);
        assert_eq!(settled.state, JobState::Queued);
        assert_eq!(settled.attempts, 0);
        assert_eq!(settled.directory_progress.unwrap().next_unit, expected);
        drop(worker); // A fresh process/owner must resume the same manifest.
        assert_eq!(library.stats().unwrap().artifacts, u64::from(expected));
    }
    let completed = worker(&library).run_next().unwrap().unwrap();
    assert_eq!(completed.state, JobState::Completed);
    assert_eq!(completed.result.unwrap()["indexed"], 7);
    assert!(completed.directory_progress.is_none());
    assert_eq!(
        library
            .enqueue_index_directory(&root, "directory", JobPriority::Normal)
            .unwrap()
            .id,
        job.id
    );
}

#[test]
fn failed_discovery_or_unapproved_scope_never_admits_or_reactivates_a_source() {
    let (_temporary, library, root) = fixture();
    let locator = root.to_str().unwrap();
    library.revoke_source_root(locator).unwrap();
    assert!(matches!(
        library.enqueue_index_directory(&root, "revoked", JobPriority::Normal),
        Err(LoomError::SourceRevoked(_))
    ));
    assert!(!library.source_roots().unwrap()[0].enabled);
    let outside = root.parent().unwrap().join("unselected");
    fs::create_dir(&outside).unwrap();
    assert!(library
        .enqueue_index_directory(&outside, "unselected", JobPriority::Normal)
        .is_err());
    library.index_path(&root).unwrap();
    let mut deep = root.clone();
    for _ in 0..33 {
        deep.push("child");
        fs::create_dir(&deep).unwrap();
    }
    assert!(library
        .enqueue_index_directory(&root, "deep", JobPriority::Normal)
        .unwrap_err()
        .to_string()
        .contains("depth limit"));
    assert!(library.background_jobs(128).unwrap().is_empty());
}

#[test]
fn changed_final_namespace_does_not_hide_unvisited_previous_evidence() {
    let (_temporary, library, root) = fixture();
    let old = root.join("old.md");
    fs::write(&old, "Previous recoverable marker").unwrap();
    library.index_path(&root).unwrap();
    fs::remove_file(&old).unwrap();
    fs::write(root.join("new.md"), "New marker").unwrap();
    let job = library
        .enqueue_index_directory(&root, "namespace", JobPriority::Normal)
        .unwrap();
    let mut worker = worker(&library);
    assert_eq!(worker.run_next().unwrap().unwrap().state, JobState::Queued);
    fs::write(root.join("late.md"), "Outside frozen manifest").unwrap();
    let refusal = worker.run_next().unwrap().unwrap();
    assert_eq!(refusal.id, job.id);
    assert_eq!(refusal.state, JobState::Retryable);
    assert_eq!(
        library
            .search(&SearchRequest {
                text: "\"Previous recoverable marker\"".into(),
                limit: 5
            })
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn stable_empty_completion_reconciles_but_cancelled_work_does_not() {
    let (_temporary, library, root) = fixture();
    let old = root.join("old.md");
    fs::write(&old, "Previous recoverable marker").unwrap();
    library.index_path(&root).unwrap();
    fs::remove_file(&old).unwrap();
    let cancelled = library
        .enqueue_index_directory(&root, "cancel", JobPriority::Normal)
        .unwrap();
    library.cancel_background_job(&cancelled.id).unwrap();
    let request = SearchRequest {
        text: "\"Previous recoverable marker\"".into(),
        limit: 5,
    };
    assert_eq!(library.search(&request).unwrap().len(), 1);
    library
        .enqueue_index_directory(&root, "complete", JobPriority::Normal)
        .unwrap();
    assert_eq!(
        worker(&library).run_next().unwrap().unwrap().state,
        JobState::Completed
    );
    assert!(library.search(&request).unwrap().is_empty());
}

fn runtime_counts(temporary: &TempDir) -> (i64, i64, i64) {
    let connection = rusqlite::Connection::open(temporary.path().join("library.sqlite3")).unwrap();
    connection.query_row("SELECT (SELECT COUNT(*) FROM background_jobs),
        (SELECT COUNT(*) FROM background_directory_manifests), (SELECT COUNT(*) FROM background_directory_units)", [],
        |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?))).unwrap()
}

#[test]
fn admission_rollback_does_not_retain_a_partial_manifest_or_change_consent() {
    let (temporary, library, root) = fixture();
    for name in ["a.md", "b.md"] {
        fs::write(root.join(name), name).unwrap();
    }
    let before = library.export_portable().unwrap().digest;
    let connection = rusqlite::Connection::open(temporary.path().join("library.sqlite3")).unwrap();
    connection
        .execute_batch(
            "CREATE TRIGGER refuse_second_unit BEFORE INSERT ON background_directory_units
        WHEN NEW.ordinal=1 BEGIN SELECT RAISE(ABORT, 'injected admission failure'); END;",
        )
        .unwrap();
    assert!(library
        .enqueue_index_directory(&root, "admission", JobPriority::Normal)
        .is_err());
    assert_eq!(runtime_counts(&temporary), (0, 0, 0));
    assert_eq!(library.export_portable().unwrap().digest, before);
}

#[test]
fn root_replacement_and_child_symlink_do_not_publish_or_escape_scope() {
    for mutation in ["root", "child-link"] {
        let (temporary, library, root) = fixture();
        let child = root.join("a.md");
        fs::write(&child, "Private selected marker").unwrap();
        let job = library
            .enqueue_index_directory(&root, "replacement", JobPriority::Normal)
            .unwrap();
        if mutation == "root" {
            fs::rename(&root, root.with_extension("original")).unwrap();
            fs::create_dir(&root).unwrap();
            fs::write(&child, "Wrong replacement marker").unwrap();
        } else {
            #[cfg(unix)]
            {
                fs::remove_file(&child).unwrap();
                let outside = root.parent().unwrap().join("outside.md");
                fs::write(&outside, "Wrong escaped marker").unwrap();
                std::os::unix::fs::symlink(&outside, &child).unwrap();
            }
        }
        let before = library.export_portable().unwrap().digest;
        assert_ne!(
            worker(&library).run_next().unwrap().unwrap().state,
            JobState::Completed
        );
        assert_eq!(library.export_portable().unwrap().digest, before);
        library.cancel_background_job(&job.id).unwrap();
        assert_eq!(runtime_counts(&temporary).1, 0);
    }
}

#[test]
fn foreign_file_or_nested_directory_ownership_rolls_admission_back() {
    for nested in [false, true] {
        let (temporary, library, root) = fixture();
        let foreign_root = root.join("nested");
        fs::create_dir(&foreign_root).unwrap();
        let child = foreign_root.join("owned.md");
        fs::write(&child, "Foreign source marker").unwrap();
        library
            .index_path(if nested { &foreign_root } else { &child })
            .unwrap();
        let before = library.export_portable().unwrap().digest;
        assert!(matches!(
            library.enqueue_index_directory(&root, "overlap", JobPriority::Normal),
            Err(LoomError::SourceRevoked(_))
        ));
        assert_eq!(runtime_counts(&temporary), (0, 0, 0));
        assert_eq!(library.export_portable().unwrap().digest, before);
    }
}

#[test]
fn canonical_only_delete_and_update_fence_empty_create_purge_aba() {
    for mutation in ["delete", "deactivate", "move"] {
        let (temporary, library, root) = fixture();
        let child = root.join("a.md");
        fs::write(&child, "Canonical source marker").unwrap();
        let job = library
            .enqueue_index_directory(&root, "empty-admission", JobPriority::Normal)
            .unwrap();
        library.index_path(&root).unwrap(); // Another canonical writer creates the formerly absent row.
        let connection =
            rusqlite::Connection::open(temporary.path().join("library.sqlite3")).unwrap();
        connection.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        match mutation {
            "delete" => {
                connection.execute("DELETE FROM artifacts", []).unwrap();
            }
            "deactivate" => {
                connection
                    .execute("UPDATE artifact_locators SET active=0", [])
                    .unwrap();
            }
            "move" => {
                connection
                    .execute("UPDATE artifact_locators SET locator=locator||'.moved'", [])
                    .unwrap();
            }
            _ => unreachable!(),
        }
        assert!(library.background_job(&job.id).is_err());
        assert_eq!(runtime_counts(&temporary), (0, 0, 0));
        assert!(worker(&library).run_next().unwrap().is_none());
    }
}

#[test]
fn root_purge_removes_empty_manifests_and_does_not_touch_another_root() {
    let (temporary, library, root) = fixture();
    let job = library
        .enqueue_index_directory(&root, "empty", JobPriority::Normal)
        .unwrap();
    let other = root.with_extension("second");
    fs::create_dir(&other).unwrap();
    library.index_path(&other).unwrap();
    let retained = library
        .enqueue_index_directory(&other, "other", JobPriority::Normal)
        .unwrap();
    library.purge_root(root.to_str().unwrap()).unwrap();
    assert!(library.background_job(&job.id).is_err());
    assert_eq!(
        library.background_job(&retained.id).unwrap().state,
        JobState::Queued
    );
    assert_eq!(runtime_counts(&temporary), (1, 1, 0));
}

#[test]
fn skip_policy_progress_and_ocr_rotation_are_explicit() {
    let (temporary, library, root) = fixture();
    fs::write(root.join("a.bin"), b"unsupported source bytes").unwrap();
    fs::write(root.join("b.png"), b"not passed to OCR while disabled").unwrap();
    let job = library
        .enqueue_index_directory(&root, "skip", JobPriority::Normal)
        .unwrap();
    let mut runner = worker(&library);
    for next in 1..=2 {
        let quantum = runner.run_next().unwrap().unwrap();
        assert_eq!(quantum.state, JobState::Queued);
        let progress = quantum.directory_progress.unwrap();
        assert_eq!(progress.next_unit, next);
        assert_eq!(progress.skipped, next);
        assert_eq!(library.stats().unwrap().artifacts, 0);
    }
    assert_eq!(
        runner.run_next().unwrap().unwrap().state,
        JobState::Completed
    );
    drop(runner);
    assert!(library
        .background_job(&job.id)
        .unwrap()
        .directory_progress
        .is_none());
    library
        .enqueue_index_directory(&root, "policy", JobPriority::Normal)
        .unwrap();
    library.set_ocr_enabled(true).unwrap();
    let cancelled = worker(&library).run_next().unwrap().unwrap();
    assert_eq!(cancelled.state, JobState::Cancelled);
    assert_eq!(runtime_counts(&temporary).1, 0);
    library.purge_ocr_records().unwrap();
    assert!(library.background_jobs(128).unwrap().is_empty());
}

#[test]
fn export_excludes_manifest_restore_clears_it_and_malformed_runtime_refuses_restore() {
    let (temporary, library, root) = fixture();
    fs::write(root.join("a.md"), "Restore source marker").unwrap();
    library
        .enqueue_index_directory(&root, "restore", JobPriority::Normal)
        .unwrap();
    let archive = library.export_portable().unwrap();
    assert!(!archive
        .tables
        .keys()
        .any(|key| key.starts_with("background_")));
    assert!(!archive
        .settings
        .contains_key("background_job_schema_version"));
    let connection = rusqlite::Connection::open(temporary.path().join("library.sqlite3")).unwrap();
    // Portable import intentionally requires an empty canonical library. This fixture-only
    // canonical controller does not know runtime rows; retain them to test restore fencing.
    connection.execute("DELETE FROM source_roots", []).unwrap();
    let empty_before = library.export_portable().unwrap().digest;
    let trigger: String = connection
        .query_row(
            "SELECT sql FROM sqlite_master WHERE name='background_directory_locator_deleted'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    connection
        .execute_batch("DROP TRIGGER background_directory_locator_deleted;")
        .unwrap();
    assert!(library.import_portable(&archive).is_err());
    assert_eq!(library.export_portable().unwrap().digest, empty_before);
    assert_eq!(runtime_counts(&temporary), (1, 1, 1));
    connection.execute_batch(&trigger).unwrap();
    let mut runner = worker(&library);
    library.import_portable(&archive).unwrap();
    assert_eq!(runtime_counts(&temporary), (0, 0, 0));
    assert!(runner.run_next().is_err());
}

#[test]
fn bounded_reconciliation_refuses_before_hiding_any_previous_artifact() {
    let (temporary, library, root) = fixture();
    for name in ["a.md", "b.md"] {
        fs::write(root.join(name), "Previous retained source marker").unwrap();
    }
    library.index_path(&root).unwrap();
    for name in ["a.md", "b.md"] {
        fs::remove_file(root.join(name)).unwrap();
    }
    let limited = Library::open_with_limits(
        temporary.path().join("library.sqlite3"),
        loom_core::LibraryLimits {
            max_files_per_request: 1,
            ..Default::default()
        },
    )
    .unwrap();
    limited
        .enqueue_index_directory(&root, "reconcile-bound", JobPriority::Normal)
        .unwrap();
    let before = library.export_portable().unwrap().digest;
    let failure = worker(&limited).run_next().unwrap().unwrap();
    assert_eq!(failure.state, JobState::Failed);
    assert!(failure
        .last_error
        .unwrap()
        .contains("configured file limit"));
    assert_eq!(library.export_portable().unwrap().digest, before);
}

#[test]
fn directory_yields_allow_other_due_work_and_refuse_conflicting_delivery() {
    let (_temporary, library, root) = fixture();
    for index in 0..7 {
        fs::write(root.join(format!("{index}.md")), "Fair scheduling marker").unwrap();
    }
    let directory = library
        .enqueue_index_directory(&root, "fair", JobPriority::Normal)
        .unwrap();
    assert_eq!(
        library
            .enqueue_index_directory(&root, "fair", JobPriority::Normal)
            .unwrap()
            .id,
        directory.id
    );
    assert!(library
        .enqueue_index_directory(&root, "fair", JobPriority::High)
        .is_err());
    let maintenance = library
        .enqueue_fts_repair("maintenance", JobPriority::Normal)
        .unwrap();
    let mut runner = worker(&library);
    assert_eq!(runner.run_next().unwrap().unwrap().id, directory.id);
    assert_eq!(runner.run_next().unwrap().unwrap().id, maintenance.id);
    assert_eq!(
        runner
            .run_next()
            .unwrap()
            .unwrap()
            .directory_progress
            .unwrap()
            .next_unit,
        2
    );
    fs::write(root.join("extra.md"), "Changed manifest membership").unwrap();
    assert!(library
        .enqueue_index_directory(&root, "fair", JobPriority::Normal)
        .is_err());
}

#[test]
fn control_character_children_and_traversal_payloads_are_refused() {
    let (temporary, library, root) = fixture();
    let bad = root.join("control\n.md");
    fs::write(&bad, "Invalid locator marker").unwrap();
    assert!(library
        .enqueue_index_directory(&root, "control", JobPriority::Normal)
        .is_err());
    assert_eq!(runtime_counts(&temporary), (0, 0, 0));
    fs::remove_file(bad).unwrap();
    let child = root.join("a.md");
    fs::write(&child, "Valid marker").unwrap();
    for relative in ["../outside.md", "/absolute.md", "./a.md"] {
        let job = library
            .enqueue_index_directory(
                &root,
                &format!("bad-{}", library.background_jobs(128).unwrap().len()),
                JobPriority::Normal,
            )
            .unwrap();
        let connection =
            rusqlite::Connection::open(temporary.path().join("library.sqlite3")).unwrap();
        connection
            .execute(
                "UPDATE background_directory_units SET relative_path=?1 WHERE job_id=?2",
                rusqlite::params![relative, job.id],
            )
            .unwrap();
        assert_eq!(
            worker(&library).run_next().unwrap().unwrap().state,
            JobState::Failed
        );
        assert_eq!(library.stats().unwrap().artifacts, 0);
    }
}
