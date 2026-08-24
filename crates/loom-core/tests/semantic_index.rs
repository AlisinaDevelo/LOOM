use std::fs;

use loom_core::{Library, LoomError};
use rusqlite::Connection;
use tempfile::tempdir;

#[test]
fn semantic_rebuild_is_evidence_bound_and_repeatable() {
    let directory = tempdir().unwrap();
    let source = directory.path().join("notes.md");
    let database = directory.path().join("library.sqlite3");
    fs::write(
        &source,
        "# Retrieval notes\nSQLite retry anomalies need an exact source anchor.\n",
    )
    .unwrap();

    let library = Library::open(&database).unwrap();
    library.index_path(&source).unwrap();
    let canonical_stats = library.stats().unwrap();
    assert!(!library.semantic_status().unwrap().healthy);

    let measurements = library.semantic_provider_benchmark().unwrap();
    assert_eq!(measurements.len(), 3);
    assert!(measurements.iter().all(|item| {
        item.sample_count == 1 && item.vector_bytes > 0 && item.elapsed_micros < 1_000_000
    }));

    let rebuilt = library.semantic_rebuild().unwrap();
    assert_eq!(rebuilt.rebuilt_passages, canonical_stats.passages);
    assert_eq!(rebuilt.manifest.config.provider_id, "loom.hash-embedding");
    assert_eq!(rebuilt.manifest.config.dimension, 128);
    assert_eq!(rebuilt.manifest.config.normalization, "l2");
    assert_eq!(rebuilt.manifest.config.index_revision, "semantic-v1");

    let healthy = library.semantic_status().unwrap();
    assert!(healthy.healthy, "semantic status: {healthy:?}");
    let first = library.semantic_search("retry source", 5).unwrap();
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].rank, 1);
    assert!(first[0].score > 0.0);
    assert_eq!(first[0].media_type, "text/markdown");
    assert!(first[0].source_uri.ends_with("notes.md"));
    assert_eq!(first[0].model_id, "hashed-tokens-v1");
    assert!(first[0].passage_text.contains("SQLite retry anomalies"));
    assert!(matches!(
        first[0].anchor,
        loom_core::EvidenceAnchor::Text { .. }
    ));

    let dropped = library.semantic_drop().unwrap();
    assert_eq!(dropped.embeddings_deleted, 1);
    assert!(dropped.manifest_deleted);
    assert!(!library.semantic_status().unwrap().healthy);
    assert_eq!(library.stats().unwrap(), canonical_stats);
    assert!(matches!(
        library.semantic_search("retry source", 5),
        Err(LoomError::SemanticIndexUnavailable(_))
    ));

    library.semantic_rebuild().unwrap();
    let second = library.semantic_search("retry source", 5).unwrap();
    assert_eq!(first.len(), second.len());
    assert_eq!(first[0].passage_id, second[0].passage_id);
    assert_eq!(first[0].score.to_bits(), second[0].score.to_bits());
    assert_eq!(library.stats().unwrap(), canonical_stats);
}

#[test]
fn tampered_passage_binding_fails_closed_without_changing_canonical_rows() {
    let directory = tempdir().unwrap();
    let source = directory.path().join("binding.md");
    let database = directory.path().join("library.sqlite3");
    fs::write(&source, "binding marker\n").unwrap();
    let library = Library::open(&database).unwrap();
    library.index_path(&source).unwrap();
    library.semantic_rebuild().unwrap();
    let canonical_stats = library.stats().unwrap();
    drop(library);

    let connection = Connection::open(&database).unwrap();
    connection
        .execute(
            "UPDATE semantic_embeddings SET passage_hash = 'blake3:tampered'",
            [],
        )
        .unwrap();
    drop(connection);

    let library = Library::open(&database).unwrap();
    let status = library.semantic_status().unwrap();
    assert!(!status.healthy);
    assert!(status
        .reason
        .as_deref()
        .is_some_and(|reason| reason.contains("stale passage hashes")));
    assert!(matches!(
        library.semantic_search("binding marker", 5),
        Err(LoomError::SemanticIndexUnavailable(_))
    ));
    assert_eq!(library.stats().unwrap(), canonical_stats);
}

#[test]
fn semantic_digest_detects_source_version_changes_and_recovers() {
    let directory = tempdir().unwrap();
    let source = directory.path().join("changing.txt");
    let database = directory.path().join("library.sqlite3");
    fs::write(&source, "first source marker\n").unwrap();
    let library = Library::open(&database).unwrap();
    library.index_path(&source).unwrap();
    library.semantic_rebuild().unwrap();
    assert!(library.semantic_status().unwrap().healthy);

    fs::write(&source, "second source marker\n").unwrap();
    library.index_path(&source).unwrap();
    let stale = library.semantic_status().unwrap();
    assert!(!stale.healthy);
    assert!(stale
        .reason
        .as_deref()
        .is_some_and(|reason| reason.contains("digest changed")));
    assert!(matches!(
        library.semantic_search("second source", 5),
        Err(LoomError::SemanticIndexUnavailable(_))
    ));

    library.semantic_rebuild().unwrap();
    assert!(library.semantic_status().unwrap().healthy);
    let recovered = library.semantic_search("second source", 5).unwrap();
    assert_eq!(recovered.len(), 1);
    assert!(recovered[0].source_uri.ends_with("changing.txt"));
}

#[test]
fn incompatible_manifest_fails_closed_without_touching_canonical_rows() {
    let directory = tempdir().unwrap();
    let source = directory.path().join("manifest.md");
    let database = directory.path().join("library.sqlite3");
    fs::write(&source, "manifest compatibility marker\n").unwrap();
    let library = Library::open(&database).unwrap();
    library.index_path(&source).unwrap();
    library.semantic_rebuild().unwrap();
    let canonical_stats = library.stats().unwrap();
    drop(library);

    let connection = Connection::open(&database).unwrap();
    connection
        .execute(
            "UPDATE semantic_index_meta SET model_id = 'foreign-model' WHERE slot = 1",
            [],
        )
        .unwrap();
    drop(connection);

    let library = Library::open(&database).unwrap();
    let status = library.semantic_status().unwrap();
    assert!(!status.healthy);
    assert!(status
        .reason
        .as_deref()
        .is_some_and(|reason| reason.contains("provider manifest")));
    assert!(matches!(
        library.semantic_search("manifest compatibility", 5),
        Err(LoomError::SemanticIndexUnavailable(_))
    ));
    assert_eq!(library.stats().unwrap(), canonical_stats);
}
