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
    /// Index an explicitly selected text, Markdown, or PDF file or directory.
    Index { path: PathBuf },
    /// Search active passages and print evidence-backed hits.
    Search {
        query: String,
        #[arg(long, default_value_t = 10)]
        limit: u32,
    },
    /// Print canonical library counts.
    Stats,
    /// Print the canonical extraction identity, warnings, and anchors for one indexed source.
    Inspect { path: PathBuf },
    /// Compare canonical passages with the derived FTS5 projection.
    FtsHealth,
    /// Repair the derived FTS5 projection and print before/after evidence.
    FtsRepair,
    /// Print local OCR policy and derived-record counts.
    OcrStatus,
    /// Enable local image OCR for subsequent indexing runs.
    OcrEnable,
    /// Disable local image OCR and purge all derived OCR records.
    OcrDisable,
    /// Purge derived OCR records without changing the enable policy.
    OcrPurge,
    /// Print the disposable semantic-index manifest and health state.
    SemanticStatus,
    /// Rebuild the versioned local semantic derivative from canonical passages.
    SemanticRebuild,
    /// Measure local provider candidates on the active passage corpus.
    SemanticBenchmark,
    /// Delete semantic vectors and their manifest without changing canonical records.
    SemanticDrop,
    /// Search the rebuilt semantic derivative and print evidence-bound candidates.
    SemanticSearch {
        query: String,
        #[arg(long, default_value_t = 10)]
        limit: u32,
    },
    /// Search the experimental evidence-bound hybrid ranker.
    HybridSearch {
        query: String,
        #[arg(long, default_value_t = 10)]
        limit: u32,
    },
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
    #[serde(default)]
    mean_reciprocal_rank: f64,
    #[serde(default)]
    reformulation_success: f64,
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
    #[serde(default)]
    reformulations: Vec<String>,
    #[serde(default)]
    negative: bool,
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
    PdfPage {
        page: u32,
        char_start: u64,
        char_end: u64,
        line_start: u64,
        line_end: u64,
        contains: String,
    },
    ImageRegion {
        char_start: u64,
        char_end: u64,
        line_start: u64,
        line_end: u64,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        image_width: u32,
        image_height: u32,
        orientation: u8,
        scale_milli: u32,
        confidence_milli: u32,
        contains: String,
    },
}

#[derive(Debug, Serialize)]
struct BenchmarkFailure {
    id: String,
    source_type: String,
    stage: String,
    kind: String,
    expected_file: String,
    returned: Vec<String>,
}

#[derive(Debug, Serialize)]
struct BenchmarkMetrics {
    queries: usize,
    positive_queries: usize,
    negative_queries: usize,
    exact_source_recall_at_1: f64,
    exact_source_recall_at_5: f64,
    mean_reciprocal_rank: f64,
    anchor_precision: f64,
    false_positive_rate: f64,
    reformulation_queries: usize,
    reformulation_success: f64,
    negative_no_result_rate: f64,
    median_latency_ms: f64,
    p95_latency_ms: f64,
}

#[derive(Debug, Default)]
struct BenchmarkAccumulator {
    queries: usize,
    positive_queries: usize,
    negative_queries: usize,
    top_one: usize,
    top_five: usize,
    mrr_sum: f64,
    anchor_correct: usize,
    anchor_candidates: usize,
    returned: usize,
    false_positives: usize,
    reformulation_queries: usize,
    reformulation_successes: usize,
    negative_no_result: usize,
    latencies: Vec<f64>,
}

#[derive(Debug, Serialize)]
struct BenchmarkIndexMetrics {
    discovered: u64,
    indexed: u64,
    skipped: u64,
    failures: usize,
    completeness: f64,
    index_elapsed_ms: f64,
    source_bytes_read: u64,
    database_bytes: u64,
    database_bytes_per_source_byte: f64,
}

#[derive(Debug, Serialize)]
struct BenchmarkReport {
    schema_version: u32,
    thresholds: BenchmarkThresholds,
    index: BenchmarkIndexMetrics,
    overall: BenchmarkMetrics,
    by_source_type: BTreeMap<String, BenchmarkMetrics>,
    failure_taxonomy_by_source_type: BTreeMap<String, BTreeMap<String, usize>>,
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
        Command::Inspect { path } => {
            let library = Library::open(arguments.database)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&library.inspect_source(path)?)?
            );
        }
        Command::FtsHealth => {
            let library = Library::open(arguments.database)?;
            println!("{}", serde_json::to_string_pretty(&library.fts_health()?)?);
        }
        Command::FtsRepair => {
            let library = Library::open(arguments.database)?;
            println!("{}", serde_json::to_string_pretty(&library.repair_fts()?)?);
        }
        Command::OcrStatus => {
            let library = Library::open(arguments.database)?;
            println!("{}", serde_json::to_string_pretty(&library.ocr_status()?)?);
        }
        Command::OcrEnable => {
            let library = Library::open(arguments.database)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&library.set_ocr_enabled(true)?)?
            );
        }
        Command::OcrDisable => {
            let library = Library::open(arguments.database)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&library.set_ocr_enabled(false)?)?
            );
        }
        Command::OcrPurge => {
            let library = Library::open(arguments.database)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&library.purge_ocr_records()?)?
            );
        }
        Command::SemanticStatus => {
            let library = Library::open(arguments.database)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&library.semantic_status()?)?
            );
        }
        Command::SemanticRebuild => {
            let library = Library::open(arguments.database)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&library.semantic_rebuild()?)?
            );
        }
        Command::SemanticBenchmark => {
            let library = Library::open(arguments.database)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&library.semantic_provider_benchmark()?)?
            );
        }
        Command::SemanticDrop => {
            let library = Library::open(arguments.database)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&library.semantic_drop()?)?
            );
        }
        Command::SemanticSearch { query, limit } => {
            let library = Library::open(arguments.database)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&library.semantic_search(&query, limit)?)?
            );
        }
        Command::HybridSearch { query, limit } => {
            let library = Library::open(arguments.database)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&library.hybrid_search(&query, limit)?)?
            );
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
    let database_path = temporary.path().join("benchmark.sqlite3");
    let library = Library::open(&database_path)?;
    let index_started = Instant::now();
    let index = library.index_path(&corpus)?;
    let index_elapsed_ms = index_started.elapsed().as_secs_f64() * 1_000.0;
    if !index.failures.is_empty() {
        return Err(format!(
            "benchmark corpus had indexing failures: {:?}",
            index.failures
        )
        .into());
    }
    validate_manifest_outputs(&manifest, &manifest_path, &corpus, &library, &index)?;
    let database_bytes = database_size(&database_path);
    let source_bytes_read = index.bytes_read;
    let database_bytes_per_source_byte = if source_bytes_read == 0 {
        0.0
    } else {
        database_bytes as f64 / source_bytes_read as f64
    };

    let mut overall = BenchmarkAccumulator::default();
    let mut categories: BTreeMap<String, BenchmarkAccumulator> = BTreeMap::new();
    let mut failure_taxonomy_by_source_type: BTreeMap<String, BTreeMap<String, usize>> =
        BTreeMap::new();
    let mut failures = Vec::new();
    for query in query_set {
        let started = Instant::now();
        let hits = library.search(&SearchRequest {
            text: query.query.clone(),
            limit: 5,
        })?;
        let latency = started.elapsed().as_secs_f64() * 1_000.0;
        let evaluation = evaluate_query(&corpus, &fixture_sources, &query, &hits);
        let reformulation_success = if query.negative || query.reformulations.is_empty() {
            None
        } else {
            let mut success = false;
            let mut returned = Vec::new();
            for reformulation in &query.reformulations {
                let reformulated_hits = library.search(&SearchRequest {
                    text: reformulation.clone(),
                    limit: 5,
                })?;
                returned.extend(reformulated_hits.iter().map(|hit| hit.source_uri.clone()));
                let reformulated =
                    evaluate_query(&corpus, &fixture_sources, &query, &reformulated_hits);
                success |= reformulated.top_five && reformulated.anchor_correct;
            }
            if !success {
                record_failure(
                    &mut failures,
                    &mut failure_taxonomy_by_source_type,
                    &query,
                    "reformulation",
                    "reformulation_failed",
                    returned,
                );
            }
            Some(success)
        };

        update_accumulator(&mut overall, &evaluation, reformulation_success, latency);
        update_accumulator(
            categories.entry(query.source_type.clone()).or_default(),
            &evaluation,
            reformulation_success,
            latency,
        );
        if let Some(kind) = evaluation.failure_kind {
            record_failure(
                &mut failures,
                &mut failure_taxonomy_by_source_type,
                &query,
                "primary",
                kind,
                hits.iter().map(|hit| hit.source_uri.clone()).collect(),
            );
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
        schema_version: manifest.schema_version,
        thresholds: manifest.thresholds,
        index: BenchmarkIndexMetrics {
            discovered: index.discovered,
            indexed: index.indexed,
            skipped: index.skipped,
            failures: index.failures.len(),
            completeness,
            index_elapsed_ms,
            source_bytes_read,
            database_bytes,
            database_bytes_per_source_byte,
        },
        overall: overall_metrics,
        by_source_type: categories
            .into_iter()
            .map(|(source_type, accumulator)| (source_type, finalize_metrics(accumulator)))
            .collect(),
        failure_taxonomy_by_source_type,
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
) -> Result<BTreeMap<String, Option<String>>, Box<dyn Error>> {
    if !(2..=3).contains(&manifest.schema_version) {
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
        (
            "mean_reciprocal_rank",
            manifest.thresholds.mean_reciprocal_rank,
        ),
        (
            "reformulation_success",
            manifest.thresholds.reformulation_success,
        ),
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
            .ok()
            .map(|text| text.replace("\r\n", "\n").replace('\r', "\n"));
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
        if !validate_expected_anchor(&query.expected_anchor, source.as_deref()) {
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
            if !validate_expected_anchor(&alternative.expected_anchor, source.as_deref()) {
                return Err(format!(
                    "query {} alternative anchor does not resolve to its declared source text",
                    query.id
                )
                .into());
            }
        }
        let mut reformulations = BTreeSet::new();
        for reformulation in &query.reformulations {
            if reformulation.trim().is_empty() || !reformulations.insert(reformulation) {
                return Err(format!(
                    "query {} contains an empty or duplicate reformulation",
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
#[derive(Debug)]
struct QueryEvaluation {
    negative: bool,
    top_one: bool,
    top_five: bool,
    anchor_correct: bool,
    mrr: f64,
    returned: usize,
    false_positives: usize,
    negative_no_result: bool,
    failure_kind: Option<&'static str>,
}

fn evaluate_query(
    corpus: &Path,
    fixture_sources: &BTreeMap<String, Option<String>>,
    query: &BenchmarkQuery,
    hits: &[loom_core::SearchHit],
) -> QueryEvaluation {
    let matches: Vec<bool> = hits
        .iter()
        .map(|hit| matching_expectation(corpus, &hit.source_uri, query).is_some())
        .collect();
    let returned = hits.len();
    if query.negative {
        return QueryEvaluation {
            negative: true,
            top_one: hits.is_empty(),
            top_five: hits.is_empty(),
            anchor_correct: hits.is_empty(),
            mrr: 0.0,
            returned,
            false_positives: returned,
            negative_no_result: hits.is_empty(),
            failure_kind: (!hits.is_empty()).then_some("false_positive"),
        };
    }

    let first_match = matches.iter().position(|matched| *matched);
    let anchor_correct = hits.iter().any(|hit| {
        matching_expectation(corpus, &hit.source_uri, query).is_some_and(
            |(expected_file, expected_anchor)| {
                anchor_matches(
                    expected_anchor,
                    hit,
                    fixture_sources
                        .get(expected_file)
                        .and_then(Option::as_deref),
                )
            },
        )
    });
    let top_one = first_match == Some(0);
    let top_five = first_match.is_some();
    let mrr = first_match.map_or(0.0, |index| 1.0 / (index as f64 + 1.0));
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
    QueryEvaluation {
        negative: false,
        top_one,
        top_five,
        anchor_correct,
        mrr,
        returned,
        false_positives: matches.iter().filter(|matched| !**matched).count(),
        negative_no_result: false,
        failure_kind,
    }
}

fn update_accumulator(
    accumulator: &mut BenchmarkAccumulator,
    evaluation: &QueryEvaluation,
    reformulation_success: Option<bool>,
    latency: f64,
) {
    accumulator.queries += 1;
    if evaluation.negative {
        accumulator.negative_queries += 1;
        accumulator.negative_no_result += usize::from(evaluation.negative_no_result);
    } else {
        accumulator.positive_queries += 1;
        accumulator.top_one += usize::from(evaluation.top_one);
        accumulator.top_five += usize::from(evaluation.top_five);
        accumulator.mrr_sum += evaluation.mrr;
        accumulator.anchor_candidates += usize::from(evaluation.top_five);
        accumulator.anchor_correct += usize::from(evaluation.anchor_correct);
    }
    if let Some(success) = reformulation_success {
        accumulator.reformulation_queries += 1;
        accumulator.reformulation_successes += usize::from(success);
    }
    accumulator.returned += evaluation.returned;
    accumulator.false_positives += evaluation.false_positives;
    accumulator.latencies.push(latency);
}

fn finalize_metrics(mut accumulator: BenchmarkAccumulator) -> BenchmarkMetrics {
    accumulator.latencies.sort_by(f64::total_cmp);
    let positive_denominator = accumulator.positive_queries.max(1) as f64;
    let reformulation_denominator = accumulator.reformulation_queries.max(1) as f64;
    let negative_denominator = accumulator.negative_queries.max(1) as f64;
    BenchmarkMetrics {
        queries: accumulator.queries,
        positive_queries: accumulator.positive_queries,
        negative_queries: accumulator.negative_queries,
        exact_source_recall_at_1: accumulator.top_one as f64 / positive_denominator,
        exact_source_recall_at_5: accumulator.top_five as f64 / positive_denominator,
        mean_reciprocal_rank: accumulator.mrr_sum / positive_denominator,
        anchor_precision: accumulator.anchor_correct as f64
            / accumulator.anchor_candidates.max(1) as f64,
        false_positive_rate: accumulator.false_positives as f64
            / accumulator.returned.max(1) as f64,
        reformulation_queries: accumulator.reformulation_queries,
        reformulation_success: accumulator.reformulation_successes as f64
            / reformulation_denominator,
        negative_no_result_rate: accumulator.negative_no_result as f64 / negative_denominator,
        median_latency_ms: median(&accumulator.latencies),
        p95_latency_ms: percentile(&accumulator.latencies, 0.95),
    }
}

fn record_failure(
    failures: &mut Vec<BenchmarkFailure>,
    taxonomy: &mut BTreeMap<String, BTreeMap<String, usize>>,
    query: &BenchmarkQuery,
    stage: &str,
    kind: &str,
    returned: Vec<String>,
) {
    *taxonomy
        .entry(query.source_type.clone())
        .or_default()
        .entry(kind.to_string())
        .or_default() += 1;
    failures.push(BenchmarkFailure {
        id: query.id.clone(),
        source_type: query.source_type.clone(),
        stage: stage.into(),
        kind: kind.into(),
        expected_file: query.expected_file.clone(),
        returned,
    });
}

fn database_size(path: &Path) -> u64 {
    [
        path.to_path_buf(),
        PathBuf::from(format!("{}-wal", path.display())),
        PathBuf::from(format!("{}-shm", path.display())),
    ]
    .into_iter()
    .filter_map(|path| fs::metadata(path).ok())
    .map(|metadata| metadata.len())
    .sum()
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
        && metrics.mean_reciprocal_rank + EPSILON >= thresholds.mean_reciprocal_rank
        && metrics.anchor_precision + EPSILON >= thresholds.anchor_precision
        && metrics.false_positive_rate <= thresholds.false_positive_rate + EPSILON
        && metrics.reformulation_success + EPSILON >= thresholds.reformulation_success
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
    expected_source: Option<&str>,
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
                && expected_source
                    .is_some_and(|source| expected_source_anchor_matches(expected, source))
                && phrase_is_highlighted(hit, contains)
        }
        (
            BenchmarkAnchor::PdfPage {
                page,
                char_start,
                char_end,
                line_start,
                line_end,
                contains,
            },
            EvidenceAnchor::PdfPage {
                page: actual_page,
                char_start: actual_char_start,
                char_end: actual_char_end,
                line_start: actual_line_start,
                line_end: actual_line_end,
            },
        ) => {
            actual_page == page
                && actual_char_start == char_start
                && actual_char_end == char_end
                && actual_line_start == line_start
                && actual_line_end == line_end
                && phrase_is_highlighted(hit, contains)
        }
        (
            BenchmarkAnchor::ImageRegion {
                char_start,
                char_end,
                line_start,
                line_end,
                x,
                y,
                width,
                height,
                image_width,
                image_height,
                orientation,
                scale_milli,
                confidence_milli,
                contains,
            },
            EvidenceAnchor::ImageRegion {
                char_start: actual_char_start,
                char_end: actual_char_end,
                line_start: actual_line_start,
                line_end: actual_line_end,
                x: actual_x,
                y: actual_y,
                width: actual_width,
                height: actual_height,
                image_width: actual_image_width,
                image_height: actual_image_height,
                orientation: actual_orientation,
                scale_milli: actual_scale_milli,
                confidence_milli: actual_confidence_milli,
            },
        ) => {
            actual_char_start == char_start
                && actual_char_end == char_end
                && actual_line_start == line_start
                && actual_line_end == line_end
                && actual_x == x
                && actual_y == y
                && actual_width == width
                && actual_height == height
                && actual_image_width == image_width
                && actual_image_height == image_height
                && actual_orientation == orientation
                && actual_scale_milli == scale_milli
                && actual_confidence_milli == confidence_milli
                && phrase_is_highlighted(hit, contains)
        }
        _ => false,
    }
}

fn validate_expected_anchor(expected: &BenchmarkAnchor, source: Option<&str>) -> bool {
    match expected {
        BenchmarkAnchor::Text { .. } => {
            source.is_some_and(|source| expected_source_anchor_matches(expected, source))
        }
        BenchmarkAnchor::PdfPage {
            page,
            char_start,
            char_end,
            line_start,
            line_end,
            contains,
        } => {
            *page > 0
                && char_end >= char_start
                && *line_start > 0
                && line_end >= line_start
                && !contains.is_empty()
        }
        BenchmarkAnchor::ImageRegion {
            char_start,
            char_end,
            line_start,
            line_end,
            x,
            y,
            width,
            height,
            image_width,
            image_height,
            orientation,
            scale_milli,
            confidence_milli,
            contains,
        } => {
            char_end >= char_start
                && *line_start > 0
                && line_end >= line_start
                && *width > 0
                && *height > 0
                && *image_width > 0
                && *image_height > 0
                && x.saturating_add(*width) <= *image_width
                && y.saturating_add(*height) <= *image_height
                && *orientation > 0
                && *scale_milli > 0
                && *confidence_milli <= 1_000
                && !contains.is_empty()
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
    } = expected
    else {
        return false;
    };
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

    use loom_core::{
        EvidenceAnchor, EvidenceExcerpt, EvidenceSegment, RankContributions, SearchHit,
    };

    use super::{
        benchmark_passes, expected_source_anchor_matches, finalize_metrics, fixture_path_matches,
        matching_expectation, median, percentile, phrase_is_highlighted, update_accumulator,
        validate_expected_anchor, BenchmarkAccumulator, BenchmarkAlternative, BenchmarkAnchor,
        BenchmarkQuery, BenchmarkThresholds, QueryEvaluation,
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
    fn multimodal_anchor_validation_rejects_bad_geometry() {
        let pdf = BenchmarkAnchor::PdfPage {
            page: 1,
            char_start: 2,
            char_end: 8,
            line_start: 1,
            line_end: 1,
            contains: "marker".into(),
        };
        assert!(validate_expected_anchor(&pdf, None));
        let image = BenchmarkAnchor::ImageRegion {
            char_start: 0,
            char_end: 6,
            line_start: 1,
            line_end: 1,
            x: 5,
            y: 5,
            width: 10,
            height: 10,
            image_width: 20,
            image_height: 20,
            orientation: 1,
            scale_milli: 1_000,
            confidence_milli: 900,
            contains: "marker".into(),
        };
        assert!(validate_expected_anchor(&image, None));
        let invalid = BenchmarkAnchor::ImageRegion {
            char_start: 0,
            char_end: 6,
            line_start: 1,
            line_end: 1,
            x: 15,
            y: 5,
            width: 10,
            height: 10,
            image_width: 20,
            image_height: 20,
            orientation: 1,
            scale_milli: 1_000,
            confidence_milli: 900,
            contains: "marker".into(),
        };
        assert!(!validate_expected_anchor(&invalid, None));
    }

    #[test]
    fn metrics_retain_mrr_reformulation_and_negative_counts() {
        let positive = QueryEvaluation {
            negative: false,
            top_one: false,
            top_five: true,
            anchor_correct: true,
            mrr: 0.5,
            returned: 2,
            false_positives: 1,
            negative_no_result: false,
            failure_kind: None,
        };
        let negative = QueryEvaluation {
            negative: true,
            top_one: true,
            top_five: true,
            anchor_correct: true,
            mrr: 0.0,
            returned: 0,
            false_positives: 0,
            negative_no_result: true,
            failure_kind: None,
        };
        let mut accumulator = BenchmarkAccumulator::default();
        update_accumulator(&mut accumulator, &positive, Some(true), 1.0);
        update_accumulator(&mut accumulator, &negative, None, 2.0);
        let metrics = finalize_metrics(accumulator);
        assert_eq!(metrics.positive_queries, 1);
        assert_eq!(metrics.negative_queries, 1);
        assert_eq!(metrics.mean_reciprocal_rank, 0.5);
        assert_eq!(metrics.reformulation_success, 1.0);
        assert_eq!(metrics.negative_no_result_rate, 1.0);
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
            contributions: RankContributions {
                lexical: 1.0,
                semantic: 0.0,
                metadata: 0.0,
                reranker: 0.0,
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
            reformulations: Vec::new(),
            negative: false,
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
            mean_reciprocal_rank: 0.0,
            reformulation_success: 0.0,
        };
        let passing = super::BenchmarkMetrics {
            queries: 3,
            positive_queries: 3,
            negative_queries: 0,
            exact_source_recall_at_1: 1.0,
            exact_source_recall_at_5: 1.0,
            mean_reciprocal_rank: 1.0,
            anchor_precision: 1.0,
            false_positive_rate: 0.0,
            reformulation_queries: 0,
            reformulation_success: 0.0,
            negative_no_result_rate: 1.0,
            median_latency_ms: 1.0,
            p95_latency_ms: 2.0,
        };
        assert!(benchmark_passes(&thresholds, &passing, 1.0));

        let false_positive_regression = super::BenchmarkMetrics {
            queries: passing.queries,
            positive_queries: passing.positive_queries,
            negative_queries: passing.negative_queries,
            exact_source_recall_at_1: passing.exact_source_recall_at_1,
            exact_source_recall_at_5: passing.exact_source_recall_at_5,
            mean_reciprocal_rank: passing.mean_reciprocal_rank,
            anchor_precision: passing.anchor_precision,
            false_positive_rate: 0.01,
            reformulation_queries: passing.reformulation_queries,
            reformulation_success: passing.reformulation_success,
            negative_no_result_rate: passing.negative_no_result_rate,
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
