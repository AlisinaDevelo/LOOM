use std::fs;

use loom_core::{EvidenceAnchor, Library, LoomError, SearchRequest};
use tempfile::tempdir;
use uuid::Uuid;

#[test]
fn public_search_results_are_source_backed_and_tuple_bound() {
    let directory = tempdir().unwrap();
    let source = directory.path().join("evidence.md");
    let source_text = "# Evidence\nSerializable isolation level prevents retry anomalies.\n";
    fs::write(&source, source_text).unwrap();
    let library = Library::open_in_memory().unwrap();
    let report = library.index_path(&source).unwrap();
    assert_eq!(report.indexed, 1);

    let hit = library
        .search(&SearchRequest {
            text: "\"retry anomalies\"".into(),
            limit: 10,
        })
        .unwrap()
        .into_iter()
        .next()
        .expect("the indexed phrase must be recoverable");

    Uuid::parse_str(&hit.artifact_id).unwrap();
    Uuid::parse_str(&hit.version_id).unwrap();
    Uuid::parse_str(&hit.passage_id).unwrap();
    assert_eq!(hit.rank, 1);
    assert!(hit.score.is_finite());
    assert_eq!(
        hit.source_uri,
        source.canonicalize().unwrap().display().to_string()
    );
    let content_hash = hit.content_hash.clone();
    assert_eq!(
        content_hash,
        format!("blake3:{}", blake3::hash(source_text.as_bytes()).to_hex())
    );
    assert!(hit.match_reason.contains("SQLite FTS5 BM25"));

    let highlighted: String = hit
        .excerpt
        .segments
        .iter()
        .filter(|segment| segment.highlighted)
        .map(|segment| segment.text.as_str())
        .collect();
    assert_eq!(highlighted, "retry anomalies");
    let EvidenceAnchor::Text {
        char_start,
        char_end,
        line_start,
        line_end,
    } = hit.anchor.clone()
    else {
        panic!("text fixture unexpectedly returned a PDF page anchor")
    };
    assert_eq!((line_start, line_end), (2, 2));
    let anchored: String = source_text
        .chars()
        .skip(char_start as usize)
        .take((char_end - char_start) as usize)
        .collect();
    assert_eq!(anchored, highlighted);

    let opened = library
        .resolve_verified_artifact_path(&hit.artifact_id, &hit.version_id, &content_hash)
        .unwrap();
    assert_eq!(opened, source.canonicalize().unwrap());
    assert!(matches!(
        library.resolve_verified_artifact_path(
            &hit.artifact_id,
            &hit.version_id,
            "blake3:wrong-content-hash",
        ),
        Err(LoomError::ArtifactStale(_))
    ));

    let injection_like = library
        .search(&SearchRequest {
            text: "\"retry anomalies\" OR 1=1 --".into(),
            limit: 10,
        })
        .unwrap();
    assert!(injection_like.is_empty());
}
