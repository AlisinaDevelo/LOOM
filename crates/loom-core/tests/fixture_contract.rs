use std::{fs, path::PathBuf};

use loom_core::{EvidenceAnchor, Library};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct FixtureManifest {
    fixtures: Vec<Fixture>,
}

#[derive(Debug, Deserialize)]
struct Fixture {
    path: String,
    content_hash: String,
    extractor_id: String,
    extractor_version: String,
    passages: Vec<Passage>,
}

#[derive(Debug, Deserialize)]
struct Passage {
    ordinal: u32,
    text_hash: String,
    anchor: EvidenceAnchor,
}

#[test]
fn every_rights_clean_fixture_round_trips_hash_extractor_and_anchor() {
    let fixture_root =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/retrieval/v0");
    let manifest: FixtureManifest = serde_json::from_slice(
        &fs::read(fixture_root.join("manifest.json")).expect("fixture manifest must exist"),
    )
    .expect("fixture manifest must be valid JSON");
    let corpus = fixture_root.join("corpus").canonicalize().unwrap();
    let library = Library::open_in_memory().unwrap();
    let report = library.index_path(&corpus).unwrap();

    assert_eq!(report.discovered, manifest.fixtures.len() as u64);
    assert_eq!(report.indexed, manifest.fixtures.len() as u64);
    assert_eq!(report.unchanged, 0);
    assert_eq!(report.skipped, 0);
    assert!(report.failures.is_empty());

    for fixture in manifest.fixtures {
        let source = fixture_root.join(&fixture.path).canonicalize().unwrap();
        let bytes = fs::read(&source).unwrap();
        let observed_hash = format!("blake3:{}", blake3::hash(&bytes).to_hex());
        assert_eq!(observed_hash, fixture.content_hash, "fixture hash");

        let observation = library.inspect_source(&source).unwrap();
        assert_eq!(observation.source_uri, source.to_string_lossy());
        assert_eq!(observation.content_hash, fixture.content_hash);
        assert_eq!(observation.extractor_id, fixture.extractor_id);
        assert_eq!(observation.extractor_version, fixture.extractor_version);
        assert_eq!(observation.passages.len(), fixture.passages.len());
        for (expected, actual) in fixture.passages.iter().zip(observation.passages) {
            assert_eq!(actual.ordinal, expected.ordinal, "passage ordinal");
            assert_eq!(actual.text_hash, expected.text_hash, "passage hash");
            assert_eq!(actual.anchor, expected.anchor, "passage anchor");
        }
    }
}
