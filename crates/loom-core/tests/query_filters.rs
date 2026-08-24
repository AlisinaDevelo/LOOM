use std::fs;

use loom_core::{EvidenceAnchor, Library, SearchRequest};
use tempfile::tempdir;

#[test]
fn search_filters_before_limit_and_keeps_contributions() {
    let directory = tempdir().unwrap();
    let allowed = directory.path().join("allowed-notes.md");
    let excluded = directory.path().join("excluded-notes.txt");
    fs::write(&allowed, "needle exact recovery marker\n").unwrap();
    fs::write(&excluded, "needle exact recovery marker\n").unwrap();

    let library = Library::open_in_memory().unwrap();
    library.index_path(&allowed).unwrap();
    library.index_path(&excluded).unwrap();

    let hits = library
        .search(&SearchRequest {
            text: "needle after:2000-01-01 before:2100-01-01 type:markdown path:allowed confidence:>=0.99"
                .into(),
            limit: 1,
        })
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert!(hits[0].source_uri.ends_with("allowed-notes.md"));
    assert_eq!(hits[0].rank, 1);
    assert!(hits[0].contributions.lexical.is_finite());
    assert_eq!(hits[0].contributions.semantic, 0.0);
    assert_eq!(hits[0].contributions.metadata, 1.0);
    assert_eq!(hits[0].contributions.reranker, 0.0);

    let stable_again = library
        .search(&SearchRequest {
            text: "needle type:markdown path:allowed".into(),
            limit: 1,
        })
        .unwrap();
    assert_eq!(stable_again[0].passage_id, hits[0].passage_id);
}

#[test]
fn filtered_semantic_candidates_cannot_reenter_hybrid_results() {
    let directory = tempdir().unwrap();
    let allowed = directory.path().join("allowed.md");
    let excluded = directory.path().join("excluded.txt");
    fs::write(&allowed, "needle shared recovery marker\n").unwrap();
    fs::write(&excluded, "needle shared recovery marker\n").unwrap();

    let library = Library::open_in_memory().unwrap();
    library.index_path(&allowed).unwrap();
    library.index_path(&excluded).unwrap();
    library.semantic_rebuild().unwrap();

    let hits = library.hybrid_search("needle type:markdown", 10).unwrap();
    assert!(!hits.is_empty());
    assert!(hits.iter().all(|hit| hit.media_type == "text/markdown"));
    assert!(hits
        .iter()
        .all(|hit| hit.source_uri.ends_with("allowed.md")));
}

#[test]
fn confidence_filter_uses_image_anchor_confidence_and_time_errors_fail_closed() {
    let filters = loom_core::parse_query("marker confidence:>=0.8")
        .unwrap()
        .filters;
    let high = EvidenceAnchor::ImageRegion {
        char_start: 0,
        char_end: 6,
        line_start: 1,
        line_end: 1,
        x: 0,
        y: 0,
        width: 20,
        height: 10,
        image_width: 20,
        image_height: 10,
        orientation: 1,
        scale_milli: 1_000,
        confidence_milli: 850,
    };
    assert!(filters.matches("image/png", "/tmp/capture.png", Some(1), &high));
    assert!(loom_core::parse_query("marker after:tomorrow").is_err());
}
