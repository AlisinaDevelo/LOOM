use std::fs;

use loom_core::{Library, LibraryLimits, LoomError, SearchRequest};
use tempfile::tempdir;

#[test]
fn failed_first_selection_and_reselection_do_not_grant_a_root() {
    for initial_state in ["new", "enabled", "revoked"] {
        let temporary = tempdir().unwrap();
        let root = temporary.path().join("selected");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("old.md"), "Selected source marker").unwrap();
        let database = temporary.path().join("library.sqlite3");
        let library = Library::open(&database).unwrap();
        if initial_state != "new" {
            library.index_path(&root).unwrap();
            if initial_state == "revoked" {
                library
                    .revoke_source_root(root.canonicalize().unwrap().to_str().unwrap())
                    .unwrap();
            }
        }
        let connection = rusqlite::Connection::open(&database).unwrap();
        let root_snapshot = || {
            connection
                .prepare("SELECT id, enabled, scope_generation, last_seen_at FROM source_roots ORDER BY id")
                .unwrap()
                .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?, row.get::<_, String>(3)?)))
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap()
        };
        let before = root_snapshot();
        let checkpoint = library.index_checkpoint(&root).unwrap();
        let mut deep = root.clone();
        for _ in 0..33 {
            deep.push("child");
            fs::create_dir(&deep).unwrap();
        }
        assert!(library
            .index_path(&root)
            .unwrap_err()
            .to_string()
            .contains("depth limit"));
        assert_eq!(
            root_snapshot(),
            before,
            "{initial_state}: failed discovery changed consent"
        );
        assert_eq!(library.index_checkpoint(&root).unwrap(), checkpoint);
        if initial_state != "enabled" {
            assert_eq!(library.reconcile_approved_roots().unwrap().roots_scanned, 0);
        }
    }
}

#[test]
fn failed_discovery_neither_publishes_partial_files_nor_reconciles_missing_sources() {
    let temporary = tempdir().unwrap();
    let root = temporary.path().join("selected");
    fs::create_dir(&root).unwrap();
    let existing = root.join("old.md");
    fs::write(&existing, "Preserved source marker").unwrap();
    let library = Library::open(temporary.path().join("library.sqlite3")).unwrap();
    library.index_path(&root).unwrap();
    let stats = library.stats().unwrap();
    let checkpoint = library.index_checkpoint(&root).unwrap();
    let request = SearchRequest {
        text: "\"Preserved source marker\"".into(),
        limit: 5,
    };
    let hits = serde_json::to_value(library.search(&request).unwrap()).unwrap();
    fs::remove_file(&existing).unwrap();
    fs::write(root.join("new.md"), "Unpublished source marker").unwrap();
    let mut deep = root.clone();
    for _ in 0..33 {
        deep.push("child");
        fs::create_dir(&deep).unwrap();
    }
    let error = library.index_path(&root).unwrap_err();
    assert!(error.to_string().contains("depth limit"));
    assert_eq!(library.stats().unwrap(), stats);
    assert_eq!(library.index_checkpoint(&root).unwrap(), checkpoint);
    assert_eq!(
        serde_json::to_value(library.search(&request).unwrap()).unwrap(),
        hits
    );
    assert!(library
        .search(&SearchRequest {
            text: "\"Unpublished source marker\"".into(),
            limit: 5
        })
        .unwrap()
        .is_empty());
}

#[test]
fn changed_algorithm_or_bounds_restarts_an_interrupted_checkpoint() {
    for reason in ["legacy-fingerprint", "bounds"] {
        let temporary = tempdir().unwrap();
        let root = temporary.path().join("selected");
        fs::create_dir(&root).unwrap();
        let first = root.join("first.md");
        let second = root.join("second.md");
        fs::write(&first, "Original first marker").unwrap();
        fs::write(&second, "Second source marker").unwrap();
        let database = temporary.path().join("library.sqlite3");
        let library = Library::open(&database).unwrap();
        assert!(matches!(
            library.index_path_with_fault(&root, Some(1)),
            Err(LoomError::IndexInterrupted(_))
        ));
        let original = library.index_checkpoint(&root).unwrap().unwrap();
        assert_eq!(original.next_unit, 1);
        fs::write(&first, "Refreshed first marker").unwrap();
        if reason == "legacy-fingerprint" {
            // The exact old path-only hash must not resume past changed first.md.
            let mut hasher = blake3::Hasher::new();
            for path in [&first, &second] {
                let path = path.canonicalize().unwrap();
                hasher.update(path.to_string_lossy().as_bytes());
                hasher.update(&[0]);
            }
            rusqlite::Connection::open(&database)
                .unwrap()
                .execute(
                    "UPDATE index_jobs SET discovery_fingerprint=?1 WHERE id=?2",
                    [
                        format!("blake3:{}", hasher.finalize().to_hex()),
                        original.job_id.clone(),
                    ],
                )
                .unwrap();
        }
        drop(library);
        let library = Library::open_with_limits(
            &database,
            LibraryLimits {
                max_files_per_request: if reason == "bounds" { 10 } else { 20_000 },
                ..LibraryLimits::default()
            },
        )
        .unwrap();
        let report = library.index_path(&root).unwrap();
        assert_eq!(report.attempted, 2, "{reason}: stale checkpoint was reused");
        assert!(report.failures.is_empty());
        assert_eq!(
            library.index_checkpoint(&root).unwrap().unwrap().job_id,
            original.job_id
        );
        assert_eq!(
            library
                .search(&SearchRequest {
                    text: "\"Refreshed first marker\"".into(),
                    limit: 5
                })
                .unwrap()
                .len(),
            1
        );
    }
}
