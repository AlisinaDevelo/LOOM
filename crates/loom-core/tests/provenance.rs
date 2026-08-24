use std::fs;

use loom_core::{Library, RelationshipInput, RelationshipKind, RelationshipOrigin, SearchRequest};
use serde_json::json;
use tempfile::tempdir;

fn indexed_pair() -> (tempfile::TempDir, Library, String, String, String) {
    let directory = tempdir().unwrap();
    let source = directory.path().join("source.md");
    let target = directory.path().join("target.md");
    fs::write(&source, "source evidence for provenance").unwrap();
    fs::write(&target, "target artifact for provenance").unwrap();
    let library = Library::open_in_memory().unwrap();
    library.index_path(&source).unwrap();
    library.index_path(&target).unwrap();
    let source_hit = library
        .search(&SearchRequest {
            text: "source evidence".into(),
            limit: 1,
        })
        .unwrap()
        .remove(0);
    let target_hit = library
        .search(&SearchRequest {
            text: "target artifact".into(),
            limit: 1,
        })
        .unwrap()
        .remove(0);
    (
        directory,
        library,
        source_hit.artifact_id,
        target_hit.artifact_id,
        source_hit.passage_id,
    )
}

#[test]
fn relationship_round_trip_preserves_typed_metadata_unknown_kinds_and_endpoints() {
    let (_directory, library, source_id, target_id, passage_id) = indexed_pair();
    let relationship = library
        .add_relationship(&RelationshipInput {
            source_artifact_id: source_id.clone(),
            target_artifact_id: target_id.clone(),
            kind: RelationshipKind::SavedFrom,
            origin: RelationshipOrigin::Inferred,
            evidence_passage_id: Some(passage_id),
            confidence: Some(0.85),
            method: "browser-capture-v1".into(),
            metadata: json!({"redirects": 1, "scope": "user_action"}),
        })
        .unwrap();

    assert_eq!(relationship.schema_version, 1);
    assert_eq!(relationship.kind, RelationshipKind::SavedFrom);
    assert_eq!(relationship.origin, RelationshipOrigin::Inferred);
    assert_eq!(relationship.confidence, Some(0.85));
    assert_eq!(relationship.metadata["redirects"], 1);

    let views = library.list_relationships(&source_id, 10).unwrap();
    assert_eq!(views.len(), 1);
    assert_eq!(views[0].relationship.id, relationship.id);
    assert_eq!(views[0].source.artifact_id, source_id);
    assert_eq!(views[0].target.artifact_id, target_id);
    assert!(views[0].source.version_id.is_some());
    assert!(views[0].target.content_hash.is_some());

    let unknown = library
        .add_relationship(&RelationshipInput {
            source_artifact_id: views[0].target.artifact_id.clone(),
            target_artifact_id: views[0].source.artifact_id.clone(),
            kind: RelationshipKind::Unknown("future_connector_edge".into()),
            origin: RelationshipOrigin::UserConfirmed,
            evidence_passage_id: None,
            confidence: None,
            method: "user".into(),
            metadata: json!({"note": "kept for a future reader"}),
        })
        .unwrap();
    assert_eq!(
        unknown.kind,
        RelationshipKind::Unknown("future_connector_edge".into())
    );
    assert_eq!(unknown.origin, RelationshipOrigin::UserConfirmed);
    assert_eq!(library.list_relationships(&source_id, 10).unwrap().len(), 2);
}

#[test]
fn invalid_relationships_fail_closed_without_writing_rows() {
    let (_directory, library, source_id, target_id, passage_id) = indexed_pair();
    let invalid = [
        RelationshipInput {
            source_artifact_id: source_id.clone(),
            target_artifact_id: source_id.clone(),
            kind: RelationshipKind::DuplicateOf,
            origin: RelationshipOrigin::Observed,
            evidence_passage_id: None,
            confidence: Some(0.5),
            method: "test".into(),
            metadata: json!({}),
        },
        RelationshipInput {
            source_artifact_id: source_id.clone(),
            target_artifact_id: target_id.clone(),
            kind: RelationshipKind::Unknown(String::new()),
            origin: RelationshipOrigin::Observed,
            evidence_passage_id: None,
            confidence: Some(0.5),
            method: "test".into(),
            metadata: json!({}),
        },
        RelationshipInput {
            source_artifact_id: source_id.clone(),
            target_artifact_id: target_id.clone(),
            kind: RelationshipKind::Related,
            origin: RelationshipOrigin::Observed,
            evidence_passage_id: Some("missing-passage".into()),
            confidence: Some(0.5),
            method: "test".into(),
            metadata: json!({}),
        },
        RelationshipInput {
            source_artifact_id: source_id.clone(),
            target_artifact_id: target_id.clone(),
            kind: RelationshipKind::Related,
            origin: RelationshipOrigin::Observed,
            evidence_passage_id: Some(passage_id),
            confidence: Some(f64::NAN),
            method: "test".into(),
            metadata: json!([]),
        },
        RelationshipInput {
            source_artifact_id: source_id.clone(),
            target_artifact_id: target_id.clone(),
            kind: RelationshipKind::SavedFrom,
            origin: RelationshipOrigin::Inferred,
            evidence_passage_id: None,
            confidence: None,
            method: "browser-capture-v1".into(),
            metadata: json!({}),
        },
    ];

    for input in invalid {
        assert!(library.add_relationship(&input).is_err());
    }
    assert!(library
        .list_relationships(&source_id, 10)
        .unwrap()
        .is_empty());
}

#[test]
fn duplicate_relationships_are_idempotent_and_source_purge_cascades() {
    let (directory, library, source_id, target_id, _) = indexed_pair();
    let input = RelationshipInput {
        source_artifact_id: source_id.clone(),
        target_artifact_id: target_id,
        kind: RelationshipKind::DuplicateOf,
        origin: RelationshipOrigin::Observed,
        evidence_passage_id: None,
        confidence: Some(1.0),
        method: "content-hash".into(),
        metadata: json!({}),
    };
    let first = library.add_relationship(&input).unwrap();
    let second = library.add_relationship(&input).unwrap();
    assert_eq!(first.id, second.id);
    assert_eq!(library.list_relationships(&source_id, 10).unwrap().len(), 1);

    let source_path = directory.path().join("source.md").canonicalize().unwrap();
    library
        .purge_source_root(source_path.to_str().unwrap())
        .unwrap();
    assert!(library
        .list_relationships(&source_id, 10)
        .unwrap()
        .is_empty());
}
