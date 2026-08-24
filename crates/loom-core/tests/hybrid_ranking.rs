use loom_core::{
    fuse_hybrid_candidates, EvidenceAnchor, EvidenceExcerpt, EvidenceSegment, HybridRankConfig,
    HybridRankInput, Library, LoomError,
};
use std::fs;
use tempfile::tempdir;

fn candidate(
    passage_id: &str,
    title: &str,
    source_uri: &str,
    text: &str,
    lexical_rank: Option<u32>,
    semantic_rank: Option<u32>,
    source_modified_ns: Option<i64>,
) -> HybridRankInput {
    HybridRankInput {
        artifact_id: format!("artifact-{passage_id}"),
        version_id: format!("version-{passage_id}"),
        passage_id: passage_id.into(),
        title: title.into(),
        media_type: "text/plain".into(),
        source_uri: source_uri.into(),
        content_hash: format!("blake3:{passage_id}"),
        passage_text: text.into(),
        excerpt: EvidenceExcerpt {
            segments: vec![EvidenceSegment {
                text: text.into(),
                highlighted: false,
            }],
        },
        anchor: EvidenceAnchor::Text {
            char_start: 0,
            char_end: text.chars().count() as u64,
            line_start: 1,
            line_end: 1,
        },
        source_modified_ns,
        lexical_rank,
        semantic_rank,
    }
}

#[test]
fn fusion_is_deterministic_and_retains_per_signal_evidence() {
    let inputs = vec![
        candidate(
            "lexical-winner",
            "Retry anomalies",
            "/corpus/retry-anomalies.txt",
            "Retry anomalies require an exact source anchor.",
            Some(1),
            Some(3),
            Some(10),
        ),
        candidate(
            "semantic-winner",
            "Database notes",
            "/corpus/database-notes.txt",
            "Idempotent effects keep a database retry safe.",
            Some(3),
            Some(1),
            Some(20),
        ),
        candidate(
            "semantic-only",
            "Recovery notes",
            "/corpus/recovery-notes.txt",
            "Retry recovery can resume from a durable checkpoint.",
            None,
            Some(2),
            None,
        ),
    ];
    let config = HybridRankConfig::default();
    let first = fuse_hybrid_candidates("retry anomalies", inputs.clone(), &config).unwrap();
    let second = fuse_hybrid_candidates("retry anomalies", inputs, &config).unwrap();

    assert_eq!(first, second);
    assert_eq!(first[0].passage_id, "lexical-winner");
    assert_eq!(first[0].rank, 1);
    assert_eq!(first[0].signals.lexical_rank, Some(1));
    assert_eq!(first[0].signals.semantic_rank, Some(3));
    assert!(first[0].signals.exact_match);
    assert_eq!(first[0].signals.path_token_overlap, 1.0);
    assert_eq!(first[0].signals.recency_score, 0.0);
    assert!(first[0].score.is_finite());
    assert!(first[0].match_reason.contains("hybrid-rank-v1"));
    assert_eq!(first[2].signals.semantic_rank, Some(2));
    assert_eq!(first[2].signals.lexical_rank, None);
    assert_eq!(first[2].signals.recency_score, 0.0);
}

#[test]
fn exact_path_and_recency_signals_are_bounded_and_explainable() {
    let ranked = fuse_hybrid_candidates(
        "canonical URL",
        vec![
            candidate(
                "exact",
                "Canonical URL capture",
                "/corpus/saved-web/canonical-url.txt",
                "A canonical URL is retained.",
                Some(5),
                Some(5),
                Some(10),
            ),
            candidate(
                "recent",
                "Web capture",
                "/corpus/saved-web/other.txt",
                "A saved page has a stable title.",
                Some(1),
                Some(1),
                Some(20),
            ),
        ],
        &HybridRankConfig::default(),
    )
    .unwrap();

    let exact = ranked
        .iter()
        .find(|candidate| candidate.passage_id == "exact")
        .unwrap();
    let recent = ranked
        .iter()
        .find(|candidate| candidate.passage_id == "recent")
        .unwrap();
    assert!(exact.signals.exact_match);
    assert!(exact.signals.path_token_overlap > 0.0);
    assert_eq!(exact.signals.recency_score, 0.0);
    assert_eq!(recent.signals.recency_score, 1.0);
    for hit in ranked {
        assert!((0.0..=1.0).contains(&hit.signals.path_token_overlap));
        assert!((0.0..=1.0).contains(&hit.signals.recency_score));
        assert!(hit.signals.lexical_rrf >= 0.0);
        assert!(hit.signals.semantic_rrf >= 0.0);
        assert!(hit.score.is_finite());
    }
}

#[test]
fn unsupported_semantic_only_candidates_are_not_admitted() {
    let ranked = fuse_hybrid_candidates(
        "retry anomalies",
        vec![
            candidate(
                "expected",
                "Retry notes",
                "/corpus/retry.txt",
                "Retry anomalies need an exact source anchor.",
                Some(1),
                Some(1),
                None,
            ),
            candidate(
                "unsupported",
                "Database notes",
                "/corpus/database.txt",
                "Durable checkpoints keep recovery safe.",
                None,
                Some(1),
                None,
            ),
        ],
        &HybridRankConfig::default(),
    )
    .unwrap();

    assert_eq!(ranked.len(), 1);
    assert_eq!(ranked[0].passage_id, "expected");
}

#[test]
fn invalid_or_empty_queries_fail_closed() {
    let input = candidate(
        "one",
        "One",
        "/corpus/one.txt",
        "one",
        Some(1),
        Some(1),
        None,
    );
    assert!(fuse_hybrid_candidates("", vec![input.clone()], &HybridRankConfig::default()).is_err());
    assert!(
        fuse_hybrid_candidates(&"x".repeat(513), vec![input], &HybridRankConfig::default())
            .is_err()
    );
}

#[test]
fn library_hybrid_search_is_evidence_bound_and_requires_a_healthy_semantic_index() {
    let directory = tempdir().unwrap();
    let source = directory.path().join("retry.md");
    let distractor = directory.path().join("other.md");
    fs::write(&source, "Retry anomalies need an exact source anchor.\n").unwrap();
    fs::write(
        &distractor,
        "A separate note discusses idempotent effects.\n",
    )
    .unwrap();

    let library = Library::open_in_memory().unwrap();
    library.index_path(&source).unwrap();
    library.index_path(&distractor).unwrap();
    assert!(matches!(
        library.hybrid_search("retry anomalies", 5),
        Err(LoomError::SemanticIndexUnavailable(_))
    ));
    library.semantic_rebuild().unwrap();

    let hits = library.hybrid_search("retry anomalies", 5).unwrap();
    assert!(!hits.is_empty());
    assert_eq!(hits.len(), 1);
    let hit = &hits[0];
    assert!(hit.source_uri.ends_with("retry.md"));
    assert_eq!(hit.signals.lexical_rank, Some(1));
    assert!(hit.signals.semantic_rank.is_some());
    assert!(hit.signals.exact_match);
    assert!(hit
        .excerpt
        .segments
        .iter()
        .map(|segment| segment.text.as_str())
        .collect::<String>()
        .contains("Retry anomalies"));
    assert!(matches!(hit.anchor, EvidenceAnchor::Text { .. }));
    assert_eq!(
        hit.match_reason,
        "hybrid-rank-v1 weighted reciprocal-rank fusion"
    );
}
