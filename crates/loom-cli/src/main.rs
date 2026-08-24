use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fs::{self, File},
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    time::Instant,
};

use clap::{Parser, Subcommand};
use loom_core::{EvidenceAnchor, Library, SearchRequest};
use serde::{Deserialize, Serialize};

#[derive(Debug, Parser)]
#[command(name = "loom", version, about = "Evidence-first local retrieval")]
struct Arguments {
    #[arg(long, global = true, default_value = ".loom/library.sqlite3")]
    database: PathBuf,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Index an explicitly selected text file or directory.
    Index { path: PathBuf },
    /// Search active passages and print evidence-backed hits.
    Search {
        query: String,
        #[arg(long, default_value_t = 10)]
        limit: u32,
    },
    /// Print canonical library counts.
    Stats,
    /// Evaluate exact-artifact recovery on a rights-clean JSONL query set.
    Benchmark {
        #[arg(long)]
        corpus: PathBuf,
        #[arg(long)]
        queries: PathBuf,
    },
}

#[derive(Debug, Deserialize)]
struct BenchmarkManifest {
    schema_version: u32,
    query_count: usize,
    thresholds: BenchmarkThresholds,
    fixtures: Vec<BenchmarkFixture>,
}

#[derive(Debug, Deserialize, Serialize)]
struct BenchmarkThresholds {
    exact_source_recall_at_1: f64,
    exact_source_recall_at_5: f64,
    anchor_precision: f64,
    false_positive_rate: f64,
    index_completeness: f64,
}

#[derive(Debug, Deserialize)]
struct BenchmarkFixture {
    path: String,
    content_hash: String,
    extractor_id: String,
    extractor_version: String,
    passages: Vec<BenchmarkPassage>,
}

#[derive(Debug, Deserialize)]
struct BenchmarkPassage {
    ordinal: u32,
    text_hash: String,
    anchor: EvidenceAnchor,
}

#[derive(Debug, Deserialize)]
struct BenchmarkQuery {
    id: String,
    query: String,
    source_type: String,
    expected_file: String,
    expected_anchor: BenchmarkAnchor,
    #[serde(default)]
    acceptable_alternatives: Vec<BenchmarkAlternative>,
}

#[derive(Debug, Deserialize)]
struct BenchmarkAlternative {
    expected_file: String,
    expected_anchor: BenchmarkAnchor,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum BenchmarkAnchor {
    Text {
        char_start: u64,
        char_end: u64,
        line_start: u64,
        line_end: u64,
        contains: String,
    },
}

#[derive(Debug, Serialize)]
struct BenchmarkFailure {
    id: String,
    kind: String,
    expected_file: String,
    returned: Vec<String>,
}

#[derive(Debug, Serialize)]
struct BenchmarkMetrics {
    queries: usize,
    exact_source_recall_at_1: f64,
    exact_source_recall_at_5: f64,
    anchor_precision: f64,
    false_positive_rate: f64,
    median_latency_ms: f64,
    p95_latency_ms: f64,
}

#[derive(Debug, Default)]
struct BenchmarkAccumulator {
    queries: usize,
    top_one: usize,
    top_five: usize,
    anchor_correct: usize,
    anchor_candidates: usize,
    returned: usize,
    false_positives: usize,
    latencies: Vec<f64>,
}

#[derive(Debug, Serialize)]
struct BenchmarkIndexMetrics {
    discovered: u64,
    indexed: u64,
    skipped: u64,
    failures: usize,
    completeness: f64,
}

#[derive(Debug, Serialize)]
struct BenchmarkReport {
    schema_version: u32,
    thresholds: BenchmarkThresholds,
    index: BenchmarkIndexMetrics,
    overall: BenchmarkMetrics,
    by_source_type: BTreeMap<String, BenchmarkMetrics>,
    failures: Vec<BenchmarkFailure>,
}

fn main() -> Result<(), Box<dyn Error>> {
    let arguments = Arguments::parse();
    match arguments.command {
        Command::Index { path } => {
            let library = Library::open(arguments.database)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&library.index_path(path)?)?
            );
        }
        Command::Search { query, limit } => {
            let library = Library::open(arguments.database)?;
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &library.search(&SearchRequest { text: query, limit })?
                )?
            );
        }
        Command::Stats => {
            let library = Library::open(arguments.database)?;
            println!("{}", serde_json::to_string_pretty(&library.stats()?)?);
        }
        Command::Benchmark { corpus, queries } => run_benchmark(&corpus, &queries)?,
    }
    Ok(())
}

fn run_benchmark(corpus: &Path, queries: &Path) -> Result<(), Box<dyn Error>> {
    let corpus = corpus.canonicalize()?;
    let query_set = load_queries(queries)?;
    let manifest_path = queries
        .parent()
        .ok_or("benchmark query path has no parent directory")?
        .join("manifest.json");
    let manifest: BenchmarkManifest = serde_json::from_reader(File::open(&manifest_path)?)?;
    let fixture_sources = validate_manifest_inputs(&manifest, &manifest_path, &corpus, &query_set)?;

    let temporary = tempfile::tempdir()?;
    let library = Library::open(temporary.path().join("benchmark.sqlite3"))?;
    let index = library.index_path(&corpus)?;
    if !index.failures.is_empty() {
        return Err(format!(
            "benchmark corpus had indexing failures: {:?}",
            index.failures
        )
        .into());
    }
    validate_manifest_outputs(&manifest, &manifest_path, &corpus, &library, &index)?;

    let mut overall = BenchmarkAccumulator::default();
    let mut categories: BTreeMap<String, BenchmarkAccumulator> = BTreeMap::new();
    let mut failures = Vec::new();
    for query in query_set {
        let started = Instant::now();
        let hits = library.search(&SearchRequest {
            text: query.query.clone(),
            limit: 5,
        })?;
        let latency = started.elapsed().as_secs_f64() * 1_000.0;
        let matches: Vec<bool> = hits
            .iter()
            .map(|hit| matching_expectation(&corpus, &hit.source_uri, &query).is_some())
            .collect();
        let top_one = matches.first() == Some(&true);
        let (top_five, anchor_correct) = {
            let expected_hit = hits
                .iter()
                .find(|hit| matching_expectation(&corpus, &hit.source_uri, &query).is_some());
            let top_five = expected_hit.is_some();
            let anchor_correct = expected_hit.is_some_and(|hit| {
                let (expected_file, expected_anchor) =
                    matching_expectation(&corpus, &hit.source_uri, &query)
                        .expect("the expected hit must have a matching expectation");
                let expected_source = fixture_sources
                    .get(expected_file)
                    .expect("validated benchmark expectations must have source text");
                anchor_matches(expected_anchor, hit, expected_source)
            });
            (top_five, anchor_correct)
        };
        let returned = hits.len();
        let false_positives = matches.iter().filter(|is_match| !**is_match).count();

        update_accumulator(
            &mut overall,
            top_one,
            top_five,
            top_five,
            anchor_correct,
            returned,
            false_positives,
            latency,
        );
        update_accumulator(
            categories.entry(query.source_type.clone()).or_default(),
            top_one,
            top_five,
            top_five,
            anchor_correct,
            returned,
            false_positives,
            latency,
        );

        let failure_kind = if hits.is_empty() {
            Some("no_results")
        } else if !top_five {
            Some("wrong_source")
        } else if !top_one {
            Some("wrong_source_at_rank_1")
        } else if !anchor_correct {
            Some("wrong_anchor")
        } else {
            None
        };
        if let Some(kind) = failure_kind {
            failures.push(BenchmarkFailure {
                id: query.id,
                kind: kind.into(),
                expected_file: query.expected_file,
                returned: hits.into_iter().map(|hit| hit.source_uri).collect(),
            });
        }
    }
    if overall.queries == 0 {
        return Err("benchmark query set is empty".into());
    }

    let supported = index.discovered.saturating_sub(index.skipped);
    let completeness = if supported == 0 {
        1.0
    } else {
        (index.indexed + index.unchanged) as f64 / supported as f64
    };
    let overall_metrics = finalize_metrics(overall);
    let passed = benchmark_passes(&manifest.thresholds, &overall_metrics, completeness)
        && index.failures.is_empty();
    let report = BenchmarkReport {
        schema_version: 2,
        thresholds: manifest.thresholds,
        index: BenchmarkIndexMetrics {
            discovered: index.discovered,
            indexed: index.indexed,
            skipped: index.skipped,
            failures: index.failures.len(),
            completeness,
        },
        overall: overall_metrics,
        by_source_type: categories
            .into_iter()
            .map(|(source_type, accumulator)| (source_type, finalize_metrics(accumulator)))
            .collect(),
        failures,
    };
    println!("{}", serde_json::to_string_pretty(&report)?);
    if !passed {
        std::process::exit(2);
    }
    Ok(())
}

fn load_queries(path: &Path) -> Result<Vec<BenchmarkQuery>, Box<dyn Error>> {
    let input = BufReader::new(File::open(path)?);
    let mut queries = Vec::new();
    let mut ids = BTreeSet::new();
    for line in input.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let query: BenchmarkQuery = serde_json::from_str(&line)?;
        if !ids.insert(query.id.clone()) {
            return Err(format!("duplicate benchmark query id: {}", query.id).into());
        }
        queries.push(query);
    }
    if queries.is_empty() {
        return Err("benchmark query set is empty".into());
    }
    Ok(queries)
}

fn validate_manifest_inputs(
    manifest: &BenchmarkManifest,
    manifest_path: &Path,
    corpus: &Path,
    queries: &[BenchmarkQuery],
) -> Result<BTreeMap<String, String>, Box<dyn Error>> {
    if manifest.schema_version != 2 {
        return Err(format!(
            "unsupported benchmark manifest schema version: {}",
            manifest.schema_version
        )
        .into());
    }
    let threshold_values = [
        (
            "exact_source_recall_at_1",
            manifest.thresholds.exact_source_recall_at_1,
        ),
        (
            "exact_source_recall_at_5",
            manifest.thresholds.exact_source_recall_at_5,
        ),
        ("anchor_precision", manifest.thresholds.anchor_precision),
        (
            "false_positive_rate",
            manifest.thresholds.false_positive_rate,
        ),
        ("index_completeness", manifest.thresholds.index_completeness),
    ];
    if threshold_values
        .iter()
        .any(|(_, value)| !value.is_finite() || !(0.0..=1.0).contains(value))
    {
        let invalid = threshold_values
            .iter()
            .find(|(_, value)| !value.is_finite() || !(0.0..=1.0).contains(value))
            .map(|(name, value)| format!("{name}={value}"))
            .unwrap_or_else(|| "unknown".into());
        return Err(
            format!("benchmark threshold must be finite and within 0..=1: {invalid}").into(),
        );
    }
    if manifest.query_count != queries.len() {
        return Err(format!(
            "manifest declares {} queries but the query set contains {}",
            manifest.query_count,
            queries.len()
        )
        .into());
    }
    if manifest.fixtures.is_empty() {
        return Err("benchmark manifest contains no fixtures".into());
    }

    let manifest_directory = manifest_path
        .parent()
        .ok_or("benchmark manifest path has no parent directory")?;
    let mut fixture_sources = BTreeMap::new();
    for fixture in &manifest.fixtures {
        let fixture_path = manifest_directory.join(&fixture.path).canonicalize()?;
        let relative = fixture_path.strip_prefix(corpus).map_err(|_| {
            format!(
                "manifest fixture escapes the benchmark corpus: {}",
                fixture.path
            )
        })?;
        let relative = relative
            .to_str()
            .ok_or("benchmark fixture path is not valid UTF-8")?
            .to_string();
        let bytes = fs::read(&fixture_path)?;
        let actual_hash = format!("blake3:{}", blake3::hash(&bytes).to_hex());
        if actual_hash != fixture.content_hash {
            return Err(format!(
                "fixture hash mismatch for {}: expected {}, observed {}",
                fixture.path, fixture.content_hash, actual_hash
            )
            .into());
        }
        let text = String::from_utf8(bytes)
            .map_err(|_| format!("benchmark fixture is not UTF-8: {}", fixture.path))?
            .replace("\r\n", "\n")
            .replace('\r', "\n");
        if fixture_sources.insert(relative.clone(), text).is_some() {
            return Err(format!("duplicate benchmark fixture path: {relative}").into());
        }
    }

    for query in queries {
        let source = fixture_sources.get(&query.expected_file).ok_or_else(|| {
            format!(
                "query {} references fixture absent from manifest: {}",
                query.id, query.expected_file
            )
        })?;
        if !expected_source_anchor_matches(&query.expected_anchor, source) {
            return Err(format!(
                "query {} expected anchor does not resolve to its declared source text",
                query.id
            )
            .into());
        }
        let mut expected_files = BTreeSet::from([query.expected_file.as_str()]);
        for alternative in &query.acceptable_alternatives {
            if !expected_files.insert(alternative.expected_file.as_str()) {
                return Err(format!(
                    "query {} repeats an acceptable alternative fixture: {}",
                    query.id, alternative.expected_file
                )
                .into());
            }
            let source = fixture_sources
                .get(&alternative.expected_file)
                .ok_or_else(|| {
                    format!(
                        "query {} references alternative fixture absent from manifest: {}",
                        query.id, alternative.expected_file
                    )
                })?;
            if !expected_source_anchor_matches(&alternative.expected_anchor, source) {
                return Err(format!(
                    "query {} alternative anchor does not resolve to its declared source text",
                    query.id
                )
                .into());
            }
        }
    }
    Ok(fixture_sources)
}

fn validate_manifest_outputs(
    manifest: &BenchmarkManifest,
    manifest_path: &Path,
    corpus: &Path,
    library: &Library,
    index: &loom_core::IndexReport,
) -> Result<(), Box<dyn Error>> {
    let expected_count = u64::try_from(manifest.fixtures.len())?;
    if index.discovered != expected_count
        || index.indexed + index.unchanged != expected_count
        || index.skipped != 0
    {
        return Err(format!(
            "manifest/index completeness mismatch: {} fixtures, {} discovered, {} indexed, {} unchanged, {} skipped",
            expected_count, index.discovered, index.indexed, index.unchanged, index.skipped
        )
        .into());
    }

    let manifest_directory = manifest_path
        .parent()
        .ok_or("benchmark manifest path has no parent directory")?;
    for fixture in &manifest.fixtures {
        let fixture_path = manifest_directory.join(&fixture.path).canonicalize()?;
        if !fixture_path.starts_with(corpus) {
            return Err(format!("fixture escaped canonical corpus: {}", fixture.path).into());
        }
        let observation = library.inspect_source(&fixture_path)?;
        if observation.source_uri != fixture_path.to_string_lossy()
            || observation.content_hash != fixture.content_hash
            || observation.extractor_id != fixture.extractor_id
            || observation.extractor_version != fixture.extractor_version
        {
            return Err(format!(
                "canonical observation mismatch for fixture {}",
                fixture.path
            )
            .into());
        }
        if observation.passages.len() != fixture.passages.len() {
            return Err(format!(
                "passage count mismatch for {}: expected {}, observed {}",
                fixture.path,
                fixture.passages.len(),
                observation.passages.len()
            )
            .into());
        }
        for (expected, actual) in fixture.passages.iter().zip(&observation.passages) {
            if actual.ordinal != expected.ordinal
                || actual.text_hash != expected.text_hash
                || actual.anchor != expected.anchor
            {
                return Err(format!(
                    "extractor passage mismatch for {} at ordinal {}",
                    fixture.path, expected.ordinal
                )
                .into());
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn update_accumulator(
    accumulator: &mut BenchmarkAccumulator,
    top_one: bool,
    top_five: bool,
    anchor_candidate: bool,
    anchor_correct: bool,
    returned: usize,
    false_positives: usize,
    latency: f64,
) {
    accumulator.queries += 1;
    accumulator.top_one += usize::from(top_one);
    accumulator.top_five += usize::from(top_five);
    accumulator.anchor_candidates += usize::from(anchor_candidate);
    accumulator.anchor_correct += usize::from(anchor_correct);
    accumulator.returned += returned;
    accumulator.false_positives += false_positives;
    accumulator.latencies.push(latency);
}

fn finalize_metrics(mut accumulator: BenchmarkAccumulator) -> BenchmarkMetrics {
    accumulator.latencies.sort_by(f64::total_cmp);
    let denominator = accumulator.queries.max(1) as f64;
    BenchmarkMetrics {
        queries: accumulator.queries,
        exact_source_recall_at_1: accumulator.top_one as f64 / denominator,
        exact_source_recall_at_5: accumulator.top_five as f64 / denominator,
        anchor_precision: accumulator.anchor_correct as f64
            / accumulator.anchor_candidates.max(1) as f64,
        false_positive_rate: accumulator.false_positives as f64
            / accumulator.returned.max(1) as f64,
        median_latency_ms: median(&accumulator.latencies),
        p95_latency_ms: percentile(&accumulator.latencies, 0.95),
    }
}

fn fixture_path_matches(corpus: &Path, source_uri: &str, expected_file: &str) -> bool {
    PathBuf::from(source_uri)
        .strip_prefix(corpus)
        .is_ok_and(|relative| relative == Path::new(expected_file))
}

fn matching_expectation<'a>(
    corpus: &Path,
    source_uri: &str,
    query: &'a BenchmarkQuery,
) -> Option<(&'a str, &'a BenchmarkAnchor)> {
    if fixture_path_matches(corpus, source_uri, &query.expected_file) {
        return Some((&query.expected_file, &query.expected_anchor));
    }
    query
        .acceptable_alternatives
        .iter()
        .find(|alternative| fixture_path_matches(corpus, source_uri, &alternative.expected_file))
        .map(|alternative| {
            (
                alternative.expected_file.as_str(),
                &alternative.expected_anchor,
            )
        })
}

fn benchmark_passes(
    thresholds: &BenchmarkThresholds,
    metrics: &BenchmarkMetrics,
    completeness: f64,
) -> bool {
    const EPSILON: f64 = 1e-12;
    metrics.exact_source_recall_at_1 + EPSILON >= thresholds.exact_source_recall_at_1
        && metrics.exact_source_recall_at_5 + EPSILON >= thresholds.exact_source_recall_at_5
        && metrics.anchor_precision + EPSILON >= thresholds.anchor_precision
        && metrics.false_positive_rate <= thresholds.false_positive_rate + EPSILON
        && completeness + EPSILON >= thresholds.index_completeness
}

fn median(sorted_values: &[f64]) -> f64 {
    if sorted_values.is_empty() {
        return 0.0;
    }
    let middle = sorted_values.len() / 2;
    if sorted_values.len().is_multiple_of(2) {
        (sorted_values[middle - 1] + sorted_values[middle]) / 2.0
    } else {
        sorted_values[middle]
    }
}

fn percentile(sorted_values: &[f64], quantile: f64) -> f64 {
    if sorted_values.is_empty() {
        return 0.0;
    }
    let rank = (quantile * sorted_values.len() as f64).ceil() as usize;
    sorted_values[rank.clamp(1, sorted_values.len()) - 1]
}

fn anchor_matches(
    expected: &BenchmarkAnchor,
    hit: &loom_core::SearchHit,
    expected_source: &str,
) -> bool {
    match (expected, &hit.anchor) {
        (
            BenchmarkAnchor::Text {
                char_start,
                char_end,
                line_start,
                line_end,
                contains,
            },
            EvidenceAnchor::Text {
                char_start: actual_char_start,
                char_end: actual_char_end,
                line_start: actual_line_start,
                line_end: actual_line_end,
            },
        ) => {
            actual_char_start == char_start
                && actual_char_end == char_end
                && actual_line_start == line_start
                && actual_line_end == line_end
                && expected_source_anchor_matches(expected, expected_source)
                && phrase_is_highlighted(hit, contains)
        }
    }
}

fn expected_source_anchor_matches(expected: &BenchmarkAnchor, source: &str) -> bool {
    let BenchmarkAnchor::Text {
        char_start,
        char_end,
        line_start,
        line_end,
        contains,
    } = expected;
    if char_end < char_start {
        return false;
    }
    let characters = source.chars().collect::<Vec<_>>();
    let Ok(start) = usize::try_from(*char_start) else {
        return false;
    };
    let Ok(end) = usize::try_from(*char_end) else {
        return false;
    };
    if end > characters.len() || start > end {
        return false;
    }
    let actual_text = characters[start..end].iter().collect::<String>();
    let actual_line_start = 1 + characters[..start]
        .iter()
        .filter(|character| **character == '\n')
        .count() as u64;
    let actual_line_end = actual_line_start
        + characters[start..end]
            .iter()
            .filter(|character| **character == '\n')
            .count() as u64;
    actual_text == *contains && actual_line_start == *line_start && actual_line_end == *line_end
}

fn phrase_is_highlighted(hit: &loom_core::SearchHit, phrase: &str) -> bool {
    let source = hit
        .excerpt
        .segments
        .iter()
        .flat_map(|segment| {
            segment
                .text
                .chars()
                .map(move |character| (character, segment.highlighted))
        })
        .collect::<Vec<_>>();
    let phrase = phrase.chars().collect::<Vec<_>>();
    !phrase.is_empty()
        && source.windows(phrase.len()).any(|window| {
            window
                .iter()
                .map(|(character, _)| *character)
                .eq(phrase.iter().copied())
                && window
                    .iter()
                    .all(|(character, highlighted)| character.is_whitespace() || *highlighted)
        })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use loom_core::{EvidenceAnchor, EvidenceExcerpt, EvidenceSegment, SearchHit};

    use super::{
        benchmark_passes, expected_source_anchor_matches, fixture_path_matches,
        matching_expectation, median, percentile, phrase_is_highlighted, BenchmarkAlternative,
        BenchmarkAnchor, BenchmarkQuery, BenchmarkThresholds,
    };

    #[test]
    fn percentile_uses_nearest_rank() {
        let values = [1.0, 2.0, 3.0, 4.0, 5.0];
        assert_eq!(percentile(&values, 0.5), 3.0);
        assert_eq!(percentile(&values, 0.95), 5.0);
        assert_eq!(percentile(&[], 0.95), 0.0);
    }

    #[test]
    fn median_averages_the_middle_pair() {
        assert_eq!(median(&[1.0, 2.0, 3.0, 4.0]), 2.5);
        assert_eq!(median(&[1.0, 2.0, 3.0]), 2.0);
        assert_eq!(median(&[]), 0.0);
    }

    #[test]
    fn fixture_match_is_relative_and_not_a_filename_suffix() {
        let corpus = PathBuf::from("/fixtures");
        assert!(fixture_path_matches(
            &corpus,
            "/fixtures/notes.md",
            "notes.md"
        ));
        assert!(!fixture_path_matches(
            &corpus,
            "/fixtures/other-notes.md",
            "notes.md"
        ));
        assert!(!fixture_path_matches(
            &corpus,
            "/elsewhere/notes.md",
            "notes.md"
        ));
    }

    #[test]
    fn expected_anchor_requires_exact_source_offsets_and_lines() {
        let source = "first line\nexact phrase here\n";
        let expected = BenchmarkAnchor::Text {
            char_start: 11,
            char_end: 23,
            line_start: 2,
            line_end: 2,
            contains: "exact phrase".into(),
        };
        assert!(expected_source_anchor_matches(&expected, source));
    }

    #[test]
    fn phrase_requires_highlighted_non_whitespace_characters() {
        let hit = SearchHit {
            rank: 1,
            score: 1.0,
            artifact_id: "artifact".into(),
            version_id: "version".into(),
            passage_id: "passage".into(),
            title: "fixture".into(),
            media_type: "text/plain".into(),
            source_uri: "/fixture".into(),
            content_hash: "blake3:hash".into(),
            excerpt: EvidenceExcerpt {
                segments: vec![
                    EvidenceSegment {
                        text: "exact".into(),
                        highlighted: true,
                    },
                    EvidenceSegment {
                        text: " ".into(),
                        highlighted: false,
                    },
                    EvidenceSegment {
                        text: "phrase".into(),
                        highlighted: true,
                    },
                ],
            },
            anchor: EvidenceAnchor::Text {
                char_start: 0,
                char_end: 12,
                line_start: 1,
                line_end: 1,
            },
            match_reason: "fixture".into(),
        };
        assert!(phrase_is_highlighted(&hit, "exact phrase"));
        assert!(!phrase_is_highlighted(&hit, "exact phrase missing"));
    }

    #[test]
    fn matching_expectation_accepts_declared_alternative_sources() {
        let query = BenchmarkQuery {
            id: "q".into(),
            query: "term".into(),
            source_type: "local_text".into(),
            expected_file: "primary.md".into(),
            expected_anchor: BenchmarkAnchor::Text {
                char_start: 0,
                char_end: 4,
                line_start: 1,
                line_end: 1,
                contains: "term".into(),
            },
            acceptable_alternatives: vec![BenchmarkAlternative {
                expected_file: "alternate.md".into(),
                expected_anchor: BenchmarkAnchor::Text {
                    char_start: 2,
                    char_end: 6,
                    line_start: 1,
                    line_end: 1,
                    contains: "term".into(),
                },
            }],
        };
        let corpus = PathBuf::from("/fixtures");
        let (file, _) = matching_expectation(&corpus, "/fixtures/alternate.md", &query).unwrap();
        assert_eq!(file, "alternate.md");
        assert!(matching_expectation(&corpus, "/fixtures/unrelated.md", &query).is_none());
    }

    #[test]
    fn benchmark_thresholds_reject_regressions_and_incomplete_indexes() {
        let thresholds = BenchmarkThresholds {
            exact_source_recall_at_1: 1.0,
            exact_source_recall_at_5: 1.0,
            anchor_precision: 1.0,
            false_positive_rate: 0.0,
            index_completeness: 1.0,
        };
        let passing = super::BenchmarkMetrics {
            queries: 3,
            exact_source_recall_at_1: 1.0,
            exact_source_recall_at_5: 1.0,
            anchor_precision: 1.0,
            false_positive_rate: 0.0,
            median_latency_ms: 1.0,
            p95_latency_ms: 2.0,
        };
        assert!(benchmark_passes(&thresholds, &passing, 1.0));

        let false_positive_regression = super::BenchmarkMetrics {
            queries: passing.queries,
            exact_source_recall_at_1: passing.exact_source_recall_at_1,
            exact_source_recall_at_5: passing.exact_source_recall_at_5,
            anchor_precision: passing.anchor_precision,
            false_positive_rate: 0.01,
            median_latency_ms: passing.median_latency_ms,
            p95_latency_ms: passing.p95_latency_ms,
        };
        assert!(!benchmark_passes(
            &thresholds,
            &false_positive_regression,
            1.0
        ));
        assert!(!benchmark_passes(&thresholds, &passing, 0.99));
    }
}
