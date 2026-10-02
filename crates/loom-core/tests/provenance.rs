use std::fs;

use loom_core::{
    Library, LoomError, RelationshipInput, RelationshipKind, RelationshipOrigin, SearchRequest,
};
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

#[test]
fn version_history_keeps_old_metadata_but_only_resolves_the_current_source() {
    let (directory, library, source_id, target_id, _) = indexed_pair();
    let original = library.artifact_version_history(&source_id, 20).unwrap();
    let old_version = original.versions[0].version_id.clone();
    let old_reference = original.versions[0].evidence.as_ref().unwrap();
    fs::write(
        directory.path().join("source.md"),
        "new current source evidence",
    )
    .unwrap();
    library
        .index_path(directory.path().join("source.md"))
        .unwrap();

    let history = library.artifact_version_history(&source_id, 20).unwrap();
    assert_eq!(history.artifact.artifact_id, source_id);
    assert_eq!(history.versions.len(), 2);
    assert!(!history.truncated);
    assert!(history.versions[0].is_current);
    assert_eq!(history.versions[1].version_id, old_version);
    assert!(!history.versions[1].is_current);
    assert!(history.versions[1].evidence.is_none());
    assert!(library.resolve_verified_evidence(old_reference).is_err());
    assert!(library
        .resolve_verified_artifact_path(
            &old_reference.artifact_id,
            &old_reference.version_id,
            &old_reference.content_hash,
        )
        .is_err());
    let reference = history.versions[0].evidence.as_ref().unwrap();
    let verified = library.resolve_verified_evidence(reference).unwrap();
    assert_eq!(verified.artifact_id, source_id);
    assert_eq!(verified.version_id, history.versions[0].version_id);
    assert_eq!(verified.passage_text, "new current source evidence");
    let mut forged = reference.clone();
    forged.artifact_id = target_id;
    assert!(library.resolve_verified_evidence(&forged).is_err());

    fs::write(
        directory.path().join("source.md"),
        "changed after inspection",
    )
    .unwrap();
    assert!(matches!(
        library.resolve_verified_evidence(reference),
        Err(LoomError::ArtifactStale(_))
    ));
}

#[test]
fn revoked_source_history_has_no_evidence_action_and_unknown_ids_fail_closed() {
    let (directory, library, source_id, _, _) = indexed_pair();
    let selected = library.artifact_version_history(&source_id, 20).unwrap();
    let reference = selected.versions[0].evidence.as_ref().unwrap();
    let source = directory.path().join("source.md").canonicalize().unwrap();
    library
        .revoke_source_root(source.to_str().unwrap())
        .unwrap();
    let history = library.artifact_version_history(&source_id, 20).unwrap();
    assert_eq!(history.artifact.state, "missing");
    assert!(history
        .versions
        .iter()
        .all(|version| version.evidence.is_none()));
    assert!(library.resolve_verified_evidence(reference).is_err());
    assert!(library
        .resolve_verified_artifact_path(
            &reference.artifact_id,
            &reference.version_id,
            &reference.content_hash,
        )
        .is_err());
    assert!(library
        .artifact_version_history("not-an-artifact-id", 20)
        .is_err());
    assert!(matches!(
        library.artifact_version_history("11111111-1111-4111-8111-111111111111", 20),
        Err(LoomError::ArtifactNotFound(_))
    ));
}

#[test]
fn version_history_clamps_limits_and_discloses_truncation() {
    let (directory, library, source_id, _, _) = indexed_pair();
    for version in 0..101 {
        fs::write(
            directory.path().join("source.md"),
            format!("source version {version}"),
        )
        .unwrap();
        library
            .index_path(directory.path().join("source.md"))
            .unwrap();
    }
    let bounded = library
        .artifact_version_history(&source_id, u32::MAX)
        .unwrap();
    assert_eq!(bounded.versions.len(), 100);
    assert!(bounded.truncated);
    assert!(bounded.versions[0].is_current);
    assert_eq!(
        bounded
            .versions
            .iter()
            .filter(|version| version.evidence.is_some())
            .count(),
        1
    );
    let minimum = library.artifact_version_history(&source_id, 0).unwrap();
    assert_eq!(minimum.versions.len(), 1);
    assert!(minimum.truncated);
    assert_eq!(minimum.versions[0], bounded.versions[0]);
}

#[test]
fn bookmark_version_history_is_metadata_only_without_a_local_file_scope() {
    let directory = tempdir().unwrap();
    let export = directory.path().join("bookmarks.html");
    fs::write(&export, include_str!("fixtures/bookmarks/chrome.html")).unwrap();
    let library = Library::open_in_memory().unwrap();
    library.import_bookmarks(&export).unwrap();
    let bookmark = library.list_bookmarks(1).unwrap().remove(0);
    let history = library
        .artifact_version_history(&bookmark.artifact_id, 20)
        .unwrap();
    assert_eq!(history.artifact.source_uri, Some(bookmark.url));
    assert_eq!(history.versions.len(), 1);
    assert!(history.versions[0].is_current);
    assert!(history.versions[0].evidence.is_none());
    assert!(!history.truncated);
}

#[test]
fn empty_local_source_history_has_no_passage_or_verified_evidence_action() {
    let (directory, library, source_id, _, _) = indexed_pair();
    let source = directory.path().join("source.md");
    fs::write(&source, "").unwrap();
    library.index_path(&source).unwrap();
    let observation = library.inspect_source(&source).unwrap();
    assert!(observation.passages.is_empty());
    let history = library.artifact_version_history(&source_id, 20).unwrap();
    assert!(history.versions[0].is_current);
    assert!(history.versions[0].evidence.is_none());
}
