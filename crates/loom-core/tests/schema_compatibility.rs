use std::fs;

use loom_core::{EvidenceAnchor, Library, LoomError, SearchRequest};
use rusqlite::Connection;
use tempfile::tempdir;

const V2_FIXTURE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/fixtures/schema-v2.sql"
));

#[test]
fn populated_v2_migration_preserves_canonical_identity_and_evidence() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("v2.sqlite3");
    Connection::open(&database)
        .unwrap()
        .execute_batch(V2_FIXTURE)
        .unwrap();

    let library = Library::open(&database).unwrap();
    assert_preserved_v2_rows(&library, &database);
    drop(library);

    let reopened = Library::open(&database).unwrap();
    assert_preserved_v2_rows(&reopened, &database);
}

#[test]
fn populated_v3_migration_adds_pdf_metadata_without_rewriting_canonical_rows() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("v3.sqlite3");
    let connection = Connection::open(&database).unwrap();
    connection.execute_batch(V2_FIXTURE).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE index_jobs(
                id TEXT PRIMARY KEY,
                source_root_id TEXT NOT NULL REFERENCES source_roots(id) ON DELETE CASCADE,
                selection_locator TEXT NOT NULL,
                discovery_fingerprint TEXT NOT NULL,
                total_units INTEGER NOT NULL CHECK(total_units >= 0),
                next_unit INTEGER NOT NULL CHECK(next_unit >= 0 AND next_unit <= total_units),
                state TEXT NOT NULL CHECK(state IN ('running', 'interrupted', 'completed', 'failed')),
                last_error TEXT,
                started_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                completed_at TEXT,
                UNIQUE(source_root_id, selection_locator)
            ) STRICT;
            UPDATE schema_meta SET value = '3' WHERE key = 'schema_version';",
        )
        .unwrap();
    drop(connection);

    let library = Library::open(&database).unwrap();
    let connection = Connection::open(&database).unwrap();
    let schema_version: String = connection
        .query_row(
            "SELECT value FROM schema_meta WHERE key = 'schema_version'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(schema_version, "5");
    let (hash, warnings, page_count): (String, String, Option<i64>) = connection
        .query_row(
            "SELECT content_hash, parse_warnings_json, page_count
             FROM artifact_versions WHERE id = 'version-v2'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(hash, "blake3:fixture-v2-hash");
    assert_eq!(warnings, "[]");
    assert_eq!(page_count, None);
    assert_eq!(
        library
            .search(&SearchRequest {
                text: "migration preserves anchors".into(),
                limit: 10,
            })
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn opening_a_library_rebuilds_a_missing_derived_fts_projection() {
    let directory = tempdir().unwrap();
    let source = directory.path().join("rebuild.md");
    let database = directory.path().join("library.sqlite3");
    fs::write(&source, "derived index rebuild marker").unwrap();

    let library = Library::open(&database).unwrap();
    library.index_path(&source).unwrap();
    assert_eq!(search_marker(&library), 1);
    drop(library);

    let connection = Connection::open(&database).unwrap();
    connection.execute("DELETE FROM passages_fts", []).unwrap();
    drop(connection);

    let reopened = Library::open(&database).unwrap();
    assert_eq!(search_marker(&reopened), 1);
}

#[test]
fn populated_v4_migration_adds_extraction_metadata_without_rewriting_rows() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("v4.sqlite3");
    let source = directory.path().join("v4.md");
    fs::write(&source, "v4 migration marker").unwrap();
    {
        let library = Library::open(&database).unwrap();
        library.index_path(&source).unwrap();
    }
    let connection = Connection::open(&database).unwrap();
    connection
        .execute_batch(
            "ALTER TABLE artifact_versions DROP COLUMN extraction_metadata_json;
             UPDATE schema_meta SET value = '4' WHERE key = 'schema_version';",
        )
        .unwrap();
    drop(connection);

    let library = Library::open(&database).unwrap();
    let connection = Connection::open(&database).unwrap();
    let schema_version: String = connection
        .query_row(
            "SELECT value FROM schema_meta WHERE key = 'schema_version'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(schema_version, "5");
    let metadata: String = connection
        .query_row(
            "SELECT extraction_metadata_json FROM artifact_versions",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(metadata, "{}");
    assert_eq!(
        library
            .search(&SearchRequest {
                text: "v4 migration marker".into(),
                limit: 5,
            })
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn malformed_v2_marker_fails_closed_without_creating_new_tables() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("malformed-v2.sqlite3");
    let connection = Connection::open(&database).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE schema_meta(key TEXT PRIMARY KEY, value TEXT NOT NULL) STRICT;
             INSERT INTO schema_meta(key, value) VALUES ('schema_version', '2');",
        )
        .unwrap();
    drop(connection);

    let error = match Library::open(&database) {
        Ok(_) => panic!("malformed version-2 database unexpectedly opened"),
        Err(error) => error,
    };
    assert!(matches!(
        error,
        LoomError::UnsupportedSchemaVersion(message)
            if message == "schema version 2 is missing required table `source_roots`"
    ));

    let connection = Connection::open(&database).unwrap();
    let version: String = connection
        .query_row(
            "SELECT value FROM schema_meta WHERE key = 'schema_version'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(version, "2");
    let index_jobs_exists: bool = connection
        .query_row(
            "SELECT EXISTS(
                SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'index_jobs'
             )",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!index_jobs_exists);
}

fn assert_preserved_v2_rows(library: &Library, database: &std::path::Path) {
    let connection = Connection::open(database).unwrap();
    let schema_version: String = connection
        .query_row(
            "SELECT value FROM schema_meta WHERE key = 'schema_version'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(schema_version, "5");

    let (hash, extractor_id, extractor_version): (String, String, String) = connection
        .query_row(
            "SELECT content_hash, extractor_id, extractor_version
             FROM artifact_versions WHERE id = 'version-v2'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(hash, "blake3:fixture-v2-hash");
    assert_eq!(extractor_id, "loom.text");
    assert_eq!(extractor_version, "0.1.0");

    let (locator_json, char_start, char_end, line_start, line_end): (String, i64, i64, i64, i64) =
        connection
            .query_row(
                "SELECT locator_json, char_start, char_end, line_start, line_end
                 FROM passages WHERE id = 'passage-v2'",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .unwrap();
    assert_eq!(
        serde_json::from_str::<EvidenceAnchor>(&locator_json).unwrap(),
        EvidenceAnchor::Text {
            char_start: 0,
            char_end: 34,
            line_start: 1,
            line_end: 1,
        }
    );
    assert_eq!((char_start, char_end, line_start, line_end), (0, 34, 1, 1));

    let relationship: (String, String, String, f64) = connection
        .query_row(
            "SELECT source_artifact_id, target_artifact_id, evidence_passage_id, confidence
             FROM relationships WHERE id = 'relationship-v2'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(
        relationship,
        (
            "artifact-v2".into(),
            "artifact-v2-target".into(),
            "passage-v2".into(),
            0.75
        )
    );

    let index_jobs_exists: bool = connection
        .query_row(
            "SELECT EXISTS(
                SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'index_jobs'
             )",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(index_jobs_exists);
    drop(connection);

    let hits = library
        .search(&SearchRequest {
            text: "\"migration preserves anchors\"".into(),
            limit: 10,
        })
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].content_hash, "blake3:fixture-v2-hash");
    assert_eq!(
        hits[0].anchor,
        EvidenceAnchor::Text {
            char_start: 7,
            char_end: 34,
            line_start: 1,
            line_end: 1,
        }
    );
}

fn search_marker(library: &Library) -> usize {
    library
        .search(&SearchRequest {
            text: "\"derived index rebuild marker\"".into(),
            limit: 10,
        })
        .unwrap()
        .len()
}
