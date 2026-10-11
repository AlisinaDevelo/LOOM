#![cfg(unix)]

use std::{fs, path::Path};

use loom_core::{JobPriority, JobQueuePolicy, JobState, Library, LoomError, SearchRequest};
use rusqlite::{types::Value, Connection};
use tempfile::tempdir;

fn roots(connection: &Connection) -> Vec<(String, String, String, i64, i64, String)> {
    connection
        .prepare("SELECT id,locator,kind,enabled,scope_generation,last_seen_at FROM source_roots ORDER BY id")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

fn folder(path: &Path) {
    fs::create_dir(path).unwrap();
    fs::write(
        path.join("one.md"),
        "Synthetic first queued folder marker.\n",
    )
    .unwrap();
}

fn runtime_state(connection: &Connection) -> Vec<(String, Vec<Vec<Value>>)> {
    [
        "background_jobs",
        "background_job_runtime",
        "background_directory_manifests",
        "background_directory_units",
        "background_semantic_builds",
        "background_semantic_units",
        "background_semantic_active",
        "semantic_index_meta",
        "semantic_embeddings",
    ]
    .into_iter()
    .map(|table| {
        let mut statement = connection
            .prepare(&format!("SELECT * FROM {table} ORDER BY rowid"))
            .unwrap();
        let columns = statement.column_count();
        let rows = statement
            .query_map([], |row| (0..columns).map(|index| row.get(index)).collect())
            .unwrap()
            .collect::<rusqlite::Result<Vec<Vec<Value>>>>()
            .unwrap();
        (table.to_owned(), rows)
    })
    .collect()
}

#[test]
fn first_folder_selection_admits_without_reading_content_and_preserves_refresh_only_admission() {
    let temporary = tempdir().unwrap();
    let root = temporary.path().join("selected");
    folder(&root);
    // Invalid UTF-8 proves selection does not first run foreground extraction.
    fs::write(root.join("unreadable.md"), [0xff, 0xfe]).unwrap();
    let database = temporary.path().join("library.sqlite3");
    let library = Library::open(&database).unwrap();
    assert!(matches!(
        library.enqueue_index_directory(&root, "refresh", JobPriority::Normal),
        Err(LoomError::SourceRevoked(_))
    ));
    assert!(library.source_roots().unwrap().is_empty());
    let job = library
        .select_and_enqueue_directory(&root, "selected", JobPriority::Normal)
        .unwrap();
    assert_eq!(job.state, JobState::Queued);
    assert_eq!(job.directory_progress.unwrap().total_units, 2);
    assert_eq!(library.stats().unwrap().artifacts, 0);
    assert_eq!(library.stats().unwrap().passages, 0);
    let connection = Connection::open(database).unwrap();
    let granted = roots(&connection);
    assert_eq!(granted.len(), 1);
    assert_eq!((&granted[0].2, granted[0].3), (&"directory".to_owned(), 1));
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM index_jobs", [], |row| row
                .get::<_, u32>(0))
            .unwrap(),
        0
    );
}

#[test]
fn selected_folder_runs_real_quanta_and_replays_without_duplicate_versions_or_consent_updates() {
    let temporary = tempdir().unwrap();
    let root = temporary.path().join("selected");
    folder(&root);
    let database = temporary.path().join("library.sqlite3");
    let library = Library::open(&database).unwrap();
    let admitted = library
        .select_and_enqueue_directory(&root, "selected", JobPriority::Normal)
        .unwrap();
    let connection = Connection::open(&database).unwrap();
    let consent = roots(&connection);
    assert_eq!(
        library
            .select_and_enqueue_directory(&root, "selected", JobPriority::Normal)
            .unwrap(),
        admitted
    );
    assert_eq!(roots(&connection), consent);
    let helper = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("loom-core-test-extractor");
    let mut worker = library
        .acquire_job_worker()
        .unwrap()
        .with_extractor_path(helper)
        .unwrap();
    assert_eq!(worker.run_next().unwrap().unwrap().state, JobState::Queued);
    let completed = worker.run_next().unwrap().unwrap();
    assert_eq!(completed.state, JobState::Completed);
    assert_eq!(
        library
            .select_and_enqueue_directory(&root, "selected", JobPriority::Normal)
            .unwrap(),
        completed
    );
    assert_eq!(roots(&connection), consent);
    assert_eq!(library.stats().unwrap().versions, 1);
    assert_eq!(
        library
            .search(&SearchRequest {
                text: "\"first queued folder marker\"".into(),
                limit: 5
            })
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn replay_cannot_reenable_revoked_consent_or_replace_the_original_capability() {
    let temporary = tempdir().unwrap();
    let root = temporary.path().join("selected");
    folder(&root);
    let database = temporary.path().join("library.sqlite3");
    let library = Library::open(&database).unwrap();
    let admitted = library
        .select_and_enqueue_directory(&root, "selected", JobPriority::Normal)
        .unwrap();
    library
        .revoke_source_root(root.canonicalize().unwrap().to_str().unwrap())
        .unwrap();
    let connection = Connection::open(&database).unwrap();
    let revoked = roots(&connection);
    assert_eq!(revoked[0].3, 0);
    assert_eq!(
        library
            .select_and_enqueue_directory(&root, "selected", JobPriority::Normal)
            .unwrap(),
        admitted
    );
    assert_eq!(roots(&connection), revoked);
    let fresh = library
        .select_and_enqueue_directory(&root, "fresh-choice", JobPriority::Normal)
        .unwrap();
    assert_ne!(fresh.id, admitted.id);
    assert_eq!(roots(&connection)[0].3, 1);
    assert!(roots(&connection)[0].4 > revoked[0].4);
}

#[test]
fn conflicting_selection_key_leaves_both_scopes_and_original_work_unchanged() {
    let temporary = tempdir().unwrap();
    let first = temporary.path().join("first");
    let second = temporary.path().join("second");
    folder(&first);
    folder(&second);
    let database = temporary.path().join("library.sqlite3");
    let library = Library::open(&database).unwrap();
    let admitted = library
        .select_and_enqueue_directory(&first, "same", JobPriority::Normal)
        .unwrap();
    let connection = Connection::open(&database).unwrap();
    let before = roots(&connection);
    for (path, priority) in [(&second, JobPriority::Normal), (&first, JobPriority::High)] {
        let error = library
            .select_and_enqueue_directory(path, "same", priority)
            .unwrap_err();
        assert!(
            error.to_string().contains("idempotency key conflict"),
            "{error}"
        );
    }
    assert_eq!(roots(&connection), before);
    assert_eq!(library.background_jobs(128).unwrap(), [admitted]);
}

#[test]
fn failed_discovery_and_backpressure_do_not_grant_or_reactivate_a_folder() {
    for failure in ["depth", "capacity"] {
        for state in ["new", "enabled", "revoked"] {
            let temporary = tempdir().unwrap();
            let root = temporary.path().join("selected");
            folder(&root);
            let database = temporary.path().join("library.sqlite3");
            let library = Library::open(&database).unwrap();
            if state != "new" {
                library.index_path(&root).unwrap();
                if state == "revoked" {
                    library
                        .revoke_source_root(root.canonicalize().unwrap().to_str().unwrap())
                        .unwrap();
                }
            }
            if failure == "depth" {
                let mut deep = root.clone();
                for _ in 0..33 {
                    deep.push("child");
                    fs::create_dir(&deep).unwrap();
                }
            } else {
                library
                    .set_job_queue_policy(JobQueuePolicy {
                        max_pending: 1,
                        ..JobQueuePolicy::default()
                    })
                    .unwrap();
                library
                    .enqueue_fts_repair("occupied", JobPriority::Normal)
                    .unwrap();
            }
            let connection = Connection::open(&database).unwrap();
            let consent = roots(&connection);
            let canonical = library.export_portable().unwrap().digest;
            let jobs = library.background_jobs(128).unwrap();
            let error = library
                .select_and_enqueue_directory(&root, "selected", JobPriority::Normal)
                .unwrap_err();
            let expected = if failure == "depth" {
                "depth limit"
            } else {
                "admission capacity reached"
            };
            assert!(
                error.to_string().contains(expected),
                "{failure}/{state}: {error}"
            );
            assert_eq!(roots(&connection), consent, "{failure}/{state}");
            assert_eq!(library.export_portable().unwrap().digest, canonical);
            assert_eq!(library.background_jobs(128).unwrap(), jobs);
        }
    }
}

#[test]
fn manifest_write_failure_rolls_back_consent_queue_and_generation_counters() {
    let temporary = tempdir().unwrap();
    let root = temporary.path().join("selected");
    folder(&root);
    let database = temporary.path().join("library.sqlite3");
    let library = Library::open(&database).unwrap();
    let connection = Connection::open(&database).unwrap();
    connection.execute_batch("CREATE TRIGGER refuse_selected_manifest BEFORE INSERT ON background_directory_units BEGIN SELECT RAISE(ABORT,'injected manifest failure'); END;").unwrap();
    let canonical = library.export_portable().unwrap().digest;
    let sequence: i64 = connection
        .query_row(
            "SELECT next_sequence FROM background_job_runtime",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let error = library
        .select_and_enqueue_directory(&root, "selected", JobPriority::Normal)
        .unwrap_err();
    assert!(
        error.to_string().contains("injected manifest failure"),
        "{error}"
    );
    assert!(roots(&connection).is_empty());
    assert!(library.background_jobs(128).unwrap().is_empty());
    assert_eq!(library.export_portable().unwrap().digest, canonical);
    assert_eq!(
        connection
            .query_row(
                "SELECT next_sequence FROM background_job_runtime",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        sequence
    );
}

#[test]
fn selection_cannot_adopt_another_exact_file_scope_or_change_an_existing_root_kind() {
    for conflict in [
        "exact-child",
        "nested-root",
        "tombstone",
        "inactive",
        "changed-kind",
    ] {
        let temporary = tempdir().unwrap();
        let root = temporary.path().join("selected.md");
        let database = temporary.path().join("library.sqlite3");
        let library = Library::open(&database).unwrap();
        if conflict == "exact-child" {
            folder(&root);
            library.index_path(root.join("one.md")).unwrap();
        } else if conflict == "changed-kind" {
            fs::write(&root, "Original selected file marker").unwrap();
            library.index_path(&root).unwrap();
            fs::remove_file(&root).unwrap();
            folder(&root);
        } else if conflict == "nested-root" {
            folder(&root);
            let nested = root.join("nested");
            folder(&nested);
            library.index_path(&nested).unwrap();
        } else {
            folder(&root);
            library.index_path(&root).unwrap();
            let connection = Connection::open(&database).unwrap();
            let statement = if conflict == "tombstone" {
                "UPDATE artifacts SET state='tombstoned'"
            } else {
                "UPDATE artifact_locators SET active=0"
            };
            connection.execute(statement, []).unwrap();
        }
        let connection = Connection::open(&database).unwrap();
        let consent = roots(&connection);
        let canonical = library.export_portable().unwrap().digest;
        let error = library
            .select_and_enqueue_directory(&root, "selected", JobPriority::Normal)
            .unwrap_err();
        if conflict == "changed-kind" {
            assert!(matches!(error, LoomError::InvalidPath(_)), "{error}");
        } else {
            assert!(matches!(error, LoomError::SourceRevoked(_)), "{error}");
        }
        assert_eq!(roots(&connection), consent, "{conflict}");
        assert_eq!(library.export_portable().unwrap().digest, canonical);
        assert!(library.background_jobs(128).unwrap().is_empty());
    }
}

#[test]
fn replay_of_changed_membership_does_not_mutate_the_original_manifest_or_consent() {
    let temporary = tempdir().unwrap();
    let root = temporary.path().join("selected");
    folder(&root);
    let database = temporary.path().join("library.sqlite3");
    let library = Library::open(&database).unwrap();
    let admitted = library
        .select_and_enqueue_directory(&root, "same", JobPriority::Normal)
        .unwrap();
    let connection = Connection::open(database).unwrap();
    let consent = roots(&connection);
    let queue = runtime_state(&connection);
    fs::write(root.join("later.md"), "Added after admission").unwrap();
    assert_eq!(
        library
            .select_and_enqueue_directory(&root, "same", JobPriority::Normal)
            .unwrap(),
        admitted
    );
    assert_eq!(runtime_state(&connection), queue);
    assert_eq!(roots(&connection), consent);
    assert_eq!(library.stats().unwrap().artifacts, 0);
}

#[test]
fn reselect_fences_semantic_state_but_replay_conflict_and_write_failure_preserve_it() {
    for representation in ["legacy", "published", "staged"] {
        for request in ["replay", "conflict", "write-failure", "success"] {
            let temporary = tempdir().unwrap();
            let root = temporary.path().join("selected");
            folder(&root);
            let database = temporary.path().join("library.sqlite3");
            let library = Library::open(&database).unwrap();
            let original = library
                .select_and_enqueue_directory(&root, "original", JobPriority::Normal)
                .unwrap();
            library
                .revoke_source_root(root.canonicalize().unwrap().to_str().unwrap())
                .unwrap();
            let other = temporary.path().join("other.md");
            fs::write(&other, "Separate synthetic semantic source marker").unwrap();
            library.index_path(&other).unwrap();
            if representation == "legacy" {
                library.semantic_rebuild().unwrap();
            } else {
                let job = library
                    .enqueue_semantic_rebuild("semantic", JobPriority::High)
                    .unwrap();
                let mut worker = library.acquire_job_worker().unwrap();
                assert_eq!(worker.run_next().unwrap().unwrap().id, job.id);
                if representation == "published" {
                    assert_eq!(
                        worker.run_next().unwrap().unwrap().state,
                        JobState::Completed
                    );
                }
            }
            let connection = Connection::open(&database).unwrap();
            if request == "write-failure" {
                connection.execute_batch("CREATE TRIGGER refuse_selected_manifest BEFORE INSERT ON background_directory_units BEGIN SELECT RAISE(ABORT,'injected semantic admission failure'); END;").unwrap();
            }
            let consent = roots(&connection);
            let canonical = library.export_portable().unwrap().digest;
            let operational = runtime_state(&connection);
            match request {
                "replay" => assert_eq!(
                    library
                        .select_and_enqueue_directory(&root, "original", JobPriority::Normal)
                        .unwrap(),
                    original
                ),
                "conflict" => {
                    let error = library
                        .select_and_enqueue_directory(&root, "original", JobPriority::High)
                        .unwrap_err();
                    assert!(error.to_string().contains("idempotency key conflict"));
                }
                "write-failure" => {
                    let error = library
                        .select_and_enqueue_directory(&root, "fresh", JobPriority::Normal)
                        .unwrap_err();
                    assert!(
                        error
                            .to_string()
                            .contains("injected semantic admission failure"),
                        "{error}"
                    );
                }
                "success" => {
                    library
                        .select_and_enqueue_directory(&root, "fresh", JobPriority::Normal)
                        .unwrap();
                    for table in [
                        "background_semantic_builds",
                        "background_semantic_units",
                        "background_semantic_active",
                        "semantic_index_meta",
                        "semantic_embeddings",
                    ] {
                        assert_eq!(
                            connection
                                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| row
                                    .get::<_, u32>(
                                    0
                                ))
                                .unwrap(),
                            0
                        );
                    }
                    let reselected = library
                        .source_roots()
                        .unwrap()
                        .into_iter()
                        .find(|entry| {
                            entry.locator == root.canonicalize().unwrap().to_str().unwrap()
                        })
                        .unwrap();
                    assert!(reselected.enabled);
                    continue;
                }
                _ => unreachable!(),
            }
            assert_eq!(roots(&connection), consent, "{representation}/{request}");
            assert_eq!(
                runtime_state(&connection),
                operational,
                "{representation}/{request}"
            );
            assert_eq!(library.export_portable().unwrap().digest, canonical);
        }
    }
}
