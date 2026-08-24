use std::fs;

use loom_core::{Library, SearchRequest};
use rusqlite::Connection;
use tempfile::tempdir;

#[test]
fn corrupted_fts_is_detected_repaired_transactionally_and_repeatable() {
    let directory = tempdir().unwrap();
    let source = directory.path().join("fts.md");
    let database = directory.path().join("library.sqlite3");
    fs::write(
        &source,
        "FTS repair preserves canonical evidence and ranked recovery.",
    )
    .unwrap();

    let library = Library::open(&database).unwrap();
    library.index_path(&source).unwrap();
    let baseline_observation = library.inspect_source(&source).unwrap();
    let baseline_hits = search(&library, "ranked recovery");
    assert_eq!(baseline_hits.len(), 1);
    let healthy = library.fts_health().unwrap();
    assert!(healthy.healthy);
    assert_eq!(healthy.canonical_passages, 1);
    assert_eq!(healthy.indexed_passages, 1);
    assert_eq!(
        healthy.expected_derivative_digest,
        healthy.derivative_digest
    );

    let (rowid, text): (i64, String) = {
        let connection = Connection::open(&database).unwrap();
        connection
            .query_row("SELECT rowid, text FROM passages", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .unwrap()
    };
    let connection = Connection::open(&database).unwrap();
    connection
        .execute(
            "INSERT INTO passages_fts(passages_fts, rowid, text)
             VALUES ('delete', ?1, ?2)",
            rusqlite::params![rowid, text],
        )
        .unwrap();
    drop(connection);

    let unhealthy = library.fts_health().unwrap();
    assert!(!unhealthy.healthy);
    assert_eq!(unhealthy.canonical_passages, 1);
    assert_eq!(unhealthy.indexed_passages, 0);
    assert_ne!(
        unhealthy.expected_derivative_digest,
        unhealthy.derivative_digest
    );
    assert!(search(&library, "ranked recovery").is_empty());

    let repaired = library.repair_fts().unwrap();
    assert!(serde_json::to_string(&repaired).unwrap().contains("before"));
    assert_eq!(repaired.before, unhealthy);
    assert!(repaired.after.healthy);
    assert_eq!(repaired.after.canonical_passages, 1);
    assert_eq!(repaired.after.indexed_passages, 1);
    assert_eq!(
        repaired.after.expected_derivative_digest,
        repaired.after.derivative_digest
    );
    assert_eq!(
        library.inspect_source(&source).unwrap(),
        baseline_observation
    );
    assert_eq!(search(&library, "ranked recovery"), baseline_hits);

    let repeated = library.repair_fts().unwrap();
    assert!(repeated.before.healthy);
    assert!(repeated.after.healthy);
    assert_eq!(
        repeated.before.derivative_digest,
        repeated.after.derivative_digest
    );
    assert_eq!(repeated.after, library.fts_health().unwrap());
}

#[test]
fn empty_library_has_a_healthy_zero_row_fts_projection() {
    let library = Library::open_in_memory().unwrap();
    let report = library.fts_health().unwrap();
    assert_eq!(report.canonical_passages, 0);
    assert_eq!(report.indexed_passages, 0);
    assert!(report.integrity_ok);
    assert!(report.healthy);
    let repaired = library.repair_fts().unwrap();
    assert!(repaired.before.healthy);
    assert!(repaired.after.healthy);
    assert_eq!(repaired.after, report);
}

fn search(library: &Library, query: &str) -> Vec<loom_core::SearchHit> {
    library
        .search(&SearchRequest {
            text: query.into(),
            limit: 10,
        })
        .unwrap()
}
