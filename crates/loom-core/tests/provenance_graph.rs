//! Provenance graph bounds and compaction (roadmap 0312).
//!
//! The documented scale is 1,000 artifacts and 50,000 relationships, with one hub artifact
//! carrying 5,000 edges; traversal must read through the source/target indexes and return edges
//! in a deterministic (created_at, id) order.

use std::{fs, time::Instant};

use loom_core::{Library, RelationshipInput, RelationshipKind, RelationshipOrigin, SearchRequest};
use rusqlite::{params, Connection};
use serde_json::json;
use tempfile::tempdir;

const ARTIFACTS: usize = 1_000;
const EDGES: usize = 50_000;
const HUB_EDGES: usize = 5_000;

fn artifact_id(index: usize) -> String {
    format!("00000000-0000-4000-8000-{index:012}")
}

fn edge_id(index: usize) -> String {
    format!("11111111-0000-4000-8000-{index:012}")
}

/// Builds the synthetic graph directly in SQLite. Endpoints are deterministic, and creation times
/// deliberately repeat so ordering must fall back to the ID.
fn synthetic_graph(database: &std::path::Path) {
    drop(Library::open(database).unwrap());
    let mut connection = Connection::open(database).unwrap();
    let transaction = connection.transaction().unwrap();
    transaction
        .execute(
            "INSERT INTO source_roots(id, kind, locator, enabled, created_at, last_seen_at)
             VALUES ('root', 'directory', '/synthetic', 1, '2027-01-01T00:00:00Z', '2027-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
    for index in 0..ARTIFACTS {
        transaction
            .execute(
                "INSERT INTO artifacts(id, source_root_id, title, media_type, state, created_at, last_seen_at)
                 VALUES (?1, 'root', ?2, 'text/plain', 'active', '2027-01-01T00:00:00Z', '2027-01-01T00:00:00Z')",
                params![artifact_id(index), format!("synthetic-{index}")],
            )
            .unwrap();
    }
    for index in 0..EDGES {
        // The first HUB_EDGES edges all touch artifact 0, alternating direction.
        let (source, target) = if index < HUB_EDGES {
            let other = 1 + index % (ARTIFACTS - 1);
            if index % 2 == 0 {
                (0, other)
            } else {
                (other, 0)
            }
        } else {
            let source = 1 + (index * 7) % (ARTIFACTS - 1);
            let target = 1 + (index * 13 + 5) % (ARTIFACTS - 1);
            if source == target {
                (source, 1 + (target % (ARTIFACTS - 2)) + 1)
            } else {
                (source, target)
            }
        };
        let (origin, confidence): (&str, Option<f64>) = match index % 3 {
            0 => ("observed", None),
            1 => ("inferred", Some(0.5)),
            _ => ("user_confirmed", None),
        };
        transaction
            .execute(
                "INSERT INTO relationships(
                    id, source_artifact_id, target_artifact_id, kind, evidence_passage_id,
                    confidence, method, relationship_schema_version, origin, metadata_json, created_at
                 ) VALUES (?1, ?2, ?3, 'related', NULL, ?4, 'synthetic', 1, ?5, '{}', ?6)",
                params![
                    edge_id(index),
                    artifact_id(source),
                    artifact_id(target),
                    confidence,
                    origin,
                    format!("2027-01-01T00:00:{:02}Z", (EDGES - index) % 60)
                ],
            )
            .unwrap();
    }
    transaction.commit().unwrap();
}

const LIST_QUERY: &str = "SELECT id FROM relationships
     WHERE source_artifact_id = ?1 OR target_artifact_id = ?1
     ORDER BY created_at, id LIMIT ?2";

#[test]
fn synthetic_graph_traversal_is_indexed_bounded_and_deterministic() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("graph.sqlite3");
    synthetic_graph(&database);

    let connection = Connection::open(&database).unwrap();
    let plan = connection
        .prepare(&format!("EXPLAIN QUERY PLAN {LIST_QUERY}"))
        .unwrap()
        .query_map(params![artifact_id(0), 100], |row| row.get::<_, String>(3))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
        .join(" | ");
    // Both sides of the OR are index searches; the planner may pick either source-leading index.
    assert!(plan.contains("MULTI-INDEX OR"), "{plan}");
    assert_eq!(
        plan.matches("SEARCH relationships USING INDEX").count(),
        2,
        "{plan}"
    );
    assert!(plan.contains("relationships_target_idx"), "{plan}");
    assert!(!plan.contains("SCAN relationships"), "full scan: {plan}");

    let mut expected = connection
        .prepare(
            "SELECT id, created_at FROM relationships
             WHERE source_artifact_id = ?1 OR target_artifact_id = ?1",
        )
        .unwrap()
        .query_map([artifact_id(0)], |row| {
            Ok((row.get::<_, String>(1)?, row.get::<_, String>(0)?))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(expected.len(), HUB_EDGES);
    expected.sort();
    let expected = expected
        .into_iter()
        .take(100)
        .map(|(_, id)| id)
        .collect::<Vec<_>>();
    drop(connection);

    let library = Library::open(&database).unwrap();
    let started = Instant::now();
    let first = library
        .list_relationships(&artifact_id(0), 500)
        .unwrap()
        .into_iter()
        .map(|view| view.relationship.id)
        .collect::<Vec<_>>();
    let elapsed = started.elapsed();
    assert_eq!(first.len(), 100, "traversal is capped at 100 edges");
    assert_eq!(first, expected);
    assert!(
        elapsed.as_secs_f64() < 2.0,
        "hub traversal took {elapsed:?}"
    );
    drop(library);

    let reopened = Library::open(&database).unwrap();
    let second = reopened
        .list_relationships(&artifact_id(0), 100)
        .unwrap()
        .into_iter()
        .map(|view| view.relationship.id)
        .collect::<Vec<_>>();
    assert_eq!(second, first);
}

#[test]
fn schema_checks_reject_malformed_envelopes_written_directly() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("graph.sqlite3");
    synthetic_graph(&database);
    let connection = Connection::open(&database).unwrap();
    let insert = |id: &str, kind: &str, method: &str, origin: &str, confidence: Option<f64>| {
        connection.execute(
            "INSERT INTO relationships(
                id, source_artifact_id, target_artifact_id, kind, confidence, method, origin,
                metadata_json, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, '{}', '2027-01-02T00:00:00Z')",
            params![
                id,
                artifact_id(1),
                artifact_id(2),
                kind,
                confidence,
                method,
                origin
            ],
        )
    };
    assert!(insert("bad-1", "related", "synthetic", "inferred", None).is_err());
    assert!(insert("bad-2", " ", "synthetic", "observed", None).is_err());
    assert!(insert("bad-3", "related", "  ", "observed", None).is_err());
    assert!(insert("bad-4", "related", "synthetic", "guessed", None).is_err());
    assert!(insert("bad-5", "related", "synthetic", "inferred", Some(1.5)).is_err());
    assert!(insert("ok-1", "related", "synthetic", "inferred", Some(0.4)).is_ok());
    assert!(connection
        .execute(
            "UPDATE relationships SET confidence = NULL WHERE id = 'ok-1'",
            [],
        )
        .is_err());
}

#[test]
fn per_artifact_budget_stops_unbounded_growth() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("graph.sqlite3");
    let notes = directory.path().join("notes");
    fs::create_dir(&notes).unwrap();
    fs::write(notes.join("a.md"), "budget source artifact").unwrap();
    fs::write(notes.join("b.md"), "budget target artifact").unwrap();
    let library = Library::open(&database).unwrap();
    library.index_path(&notes).unwrap();
    let find = |text: &str| {
        library
            .search(&SearchRequest {
                text: text.into(),
                limit: 1,
            })
            .unwrap()
            .remove(0)
            .artifact_id
    };
    let (source, target) = (find("budget source"), find("budget target"));

    let connection = Connection::open(&database).unwrap();
    let transaction = connection.unchecked_transaction().unwrap();
    for index in 0..10_000 {
        transaction
            .execute(
                "INSERT INTO relationships(
                    id, source_artifact_id, target_artifact_id, kind, method, origin,
                    metadata_json, created_at
                 ) VALUES (?1, ?2, ?3, ?4, 'synthetic', 'observed', '{}', '2027-01-01T00:00:00Z')",
                params![edge_id(index), source, target, format!("kind-{index}")],
            )
            .unwrap();
    }
    transaction.commit().unwrap();

    let error = library
        .add_relationship(&RelationshipInput {
            source_artifact_id: source.clone(),
            target_artifact_id: target,
            kind: RelationshipKind::Related,
            origin: RelationshipOrigin::Observed,
            evidence_passage_id: None,
            confidence: None,
            method: "user".into(),
            metadata: json!({}),
        })
        .unwrap_err()
        .to_string();
    assert!(error.contains("compact redundant edges"), "{error}");
}

fn edge(
    source: &str,
    target: &str,
    origin: RelationshipOrigin,
    confidence: Option<f64>,
    method: &str,
    evidence: Option<&str>,
) -> RelationshipInput {
    RelationshipInput {
        source_artifact_id: source.into(),
        target_artifact_id: target.into(),
        kind: RelationshipKind::Related,
        origin,
        evidence_passage_id: evidence.map(Into::into),
        confidence,
        method: method.into(),
        metadata: json!({}),
    }
}

#[test]
fn compaction_removes_only_dominated_inferred_edges_and_preserves_lineage() {
    let directory = tempdir().unwrap();
    let notes = directory.path().join("notes");
    fs::create_dir(&notes).unwrap();
    for name in ["alpha", "bravo", "charlie", "delta", "echo"] {
        fs::write(
            notes.join(format!("{name}.md")),
            format!("{name} compaction note"),
        )
        .unwrap();
    }
    let library = Library::open(directory.path().join("library.sqlite3")).unwrap();
    library.index_path(&notes).unwrap();
    let hit = |name: &str| {
        library
            .search(&SearchRequest {
                text: format!("{name} compaction"),
                limit: 1,
            })
            .unwrap()
            .remove(0)
    };
    let alpha = hit("alpha");
    let (a, p) = (alpha.artifact_id.as_str(), Some(alpha.passage_id.as_str()));
    let [b, c, d, e] = ["bravo", "charlie", "delta", "echo"].map(|name| hit(name).artifact_id);
    use RelationshipOrigin::{Inferred, Observed, UserConfirmed};

    // a→b: a confirmed edge outranks two inferred ones.
    let confirmed = library
        .add_relationship(&edge(a, &b, UserConfirmed, None, "user", None))
        .unwrap();
    let ab_high = library
        .add_relationship(&edge(a, &b, Inferred, Some(0.9), "m1", p))
        .unwrap();
    let ab_low = library
        .add_relationship(&edge(a, &b, Inferred, Some(0.5), "m2", p))
        .unwrap();
    // a→c: equal confidence; the earlier edge is kept.
    let ac_first = library
        .add_relationship(&edge(a, &c, Inferred, Some(0.8), "m1", p))
        .unwrap();
    let ac_second = library
        .add_relationship(&edge(a, &c, Inferred, Some(0.8), "m2", p))
        .unwrap();
    // a→d: observed edges are never compacted, even when duplicated.
    let ad_one = library
        .add_relationship(&edge(a, &d, Observed, None, "scan-1", None))
        .unwrap();
    let ad_two = library
        .add_relationship(&edge(a, &d, Observed, None, "scan-2", None))
        .unwrap();
    // a→e: a lone inferred edge is its own lineage.
    let ae = library
        .add_relationship(&edge(a, &e, Inferred, Some(0.3), "m1", p))
        .unwrap();

    let first = library.compact_relationships(1).unwrap();
    assert_eq!((first.removed, first.remaining), (1, 2));
    let second = library.compact_relationships(100).unwrap();
    assert_eq!((second.removed, second.remaining), (2, 0));
    let third = library.compact_relationships(100).unwrap();
    assert_eq!(third, Default::default());

    let remaining = library
        .list_relationships(a, 100)
        .unwrap()
        .into_iter()
        .map(|view| view.relationship.id)
        .collect::<std::collections::BTreeSet<_>>();
    for kept in [&confirmed, &ac_first, &ad_one, &ad_two, &ae] {
        assert!(remaining.contains(&kept.id), "{} was removed", kept.method);
    }
    for removed in [&ab_high, &ab_low, &ac_second] {
        assert!(
            !remaining.contains(&removed.id),
            "{} was kept",
            removed.method
        );
    }

    let compactions = library.list_relationship_compactions(10).unwrap();
    assert_eq!(compactions.len(), 2);
    let mut summarized = std::collections::BTreeMap::new();
    for compaction in &compactions {
        let digest = format!(
            "blake3:{}",
            blake3::hash(
                serde_json::to_string(&compaction.removed)
                    .unwrap()
                    .as_bytes()
            )
            .to_hex()
        );
        assert_eq!(digest, compaction.removed_digest);
        for entry in &compaction.removed {
            summarized.insert(entry.removed.id.clone(), entry.kept_relationship_id.clone());
            assert!(remaining.contains(&entry.kept_relationship_id));
        }
    }
    assert_eq!(summarized[&ab_high.id], confirmed.id);
    assert_eq!(summarized[&ab_low.id], confirmed.id);
    assert_eq!(summarized[&ac_second.id], ac_first.id);
    assert_eq!(summarized.len(), 3);

    // Compaction summaries travel with a portable export.
    let export = library.export_portable().unwrap();
    assert_eq!(export.tables["relationship_compactions"].rows.len(), 2);
    let restored = Library::open_in_memory().unwrap();
    restored.import_portable(&export).unwrap();
    assert_eq!(restored.list_relationship_compactions(10).unwrap().len(), 2);
}
