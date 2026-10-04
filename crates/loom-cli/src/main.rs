use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fs::{self, File},
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    time::Instant,
};

use clap::{Parser, Subcommand};
use loom_core::{
    BackupOptions, EvidenceAnchor, JobPriority, JobWorker, Library, LibraryLimits, PortableExport,
    SearchRequest,
};
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
    /// Import a local Chrome/Firefox Netscape HTML bookmark export without fetching URLs.
    ImportBookmarks { path: PathBuf },
    /// List imported bookmark metadata and its original export provenance.
    Bookmarks {
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
    /// List bookmark imports with their connector metadata and per-record failures.
    BookmarkImports {
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
    /// Re-read the original export of a recorded bookmark import.
    RetryBookmarkImport { import_id: String },
    /// Remove redundant inferred relationships and record a digest-linked summary.
    CompactRelationships {
        #[arg(long, default_value_t = 1000)]
        max_removals: u32,
    },
    /// List recorded relationship compactions with every removed edge.
    RelationshipCompactions {
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
    /// Search active passages and print evidence-backed hits.
    Search {
        query: String,
        #[arg(long, default_value_t = 10)]
        limit: u32,
    },
    /// Print canonical library counts.
    Stats,
    /// Print canonical, derived, sidecar, and disposable storage estimates.
    StorageInspect,
    /// Permanently delete one artifact and its evidence.
    PurgeArtifact { artifact_id: String },
    /// Permanently delete all artifacts under one exact persisted root locator.
    PurgeRoot { locator: String },
    /// Permanently delete artifacts created before an RFC3339 timestamp.
    PurgeBefore { cutoff: String },
    /// Print the configured local retention policy.
    RetentionStatus,
    /// Set or clear the local retention policy; omission disables it.
    RetentionSet {
        #[arg(long)]
        days: Option<u32>,
    },
    /// Apply the configured local retention policy now.
    RetentionApply,
    /// Remove known disposable local files and SQLite sidecars.
    PurgeDisposable,
    /// Write a plaintext portable export of every canonical row and setting to a new file.
    Export { path: PathBuf },
    /// Import a portable export into the (empty) library at --database.
    ImportExport { path: PathBuf },
    /// Write a password-encrypted backup to a new file. The password is read from
    /// --password-file or LOOM_BACKUP_PASSWORD, never from the command line.
    Backup {
        path: PathBuf,
        #[arg(long)]
        password_file: Option<PathBuf>,
    },
    /// Restore an encrypted backup into a new library at --database, which must not exist.
    Restore {
        backup: PathBuf,
        #[arg(long)]
        password_file: Option<PathBuf>,
    },
    /// Print the canonical extraction identity, warnings, and anchors for one indexed source.
    Inspect { path: PathBuf },
    /// Compare canonical passages with the derived FTS5 projection.
    FtsHealth,
    /// Repair the derived FTS5 projection and print before/after evidence.
    FtsRepair,
    /// Inspect durable, opt-in background jobs.
    Jobs {
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
    /// Queue a real FTS repair, deduplicated by a stable caller-supplied identifier.
    EnqueueFtsRepair {
        idempotency_key: String,
        #[arg(long, conflicts_with = "low")]
        high: bool,
        #[arg(long)]
        low: bool,
    },
    /// Queue one already-approved regular file; never grants scope or scans a directory.
    EnqueueIndexFile {
        path: PathBuf,
        idempotency_key: String,
        #[arg(long, conflicts_with = "low")]
        high: bool,
        #[arg(long)]
        low: bool,
    },
    /// Discover and queue one already-approved directory; run-next-job processes one file quantum.
    EnqueueIndexDirectory {
        path: PathBuf,
        idempotency_key: String,
        #[arg(long, conflicts_with = "low")]
        high: bool,
        #[arg(long)]
        low: bool,
    },
    /// Explicitly upgrade a recognized operational runtime under exclusive worker ownership.
    UpgradeJobRuntime,
    /// Request durable cancellation; a running transaction may already have completed.
    CancelJob { id: String },
    /// Explicitly forget a terminal job's diagnostic record and idempotency key.
    ForgetJob { id: String },
    /// Acquire exclusive worker ownership and run at most one due typed job.
    RunNextJob,
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
    /// Measure a selected corpus, query path, and rebuild cost in one process.
    Performance {
        #[arg(long)]
        corpus: PathBuf,
        /// Explicit disjoint indexing roots inside the corpus; repeat for bounded batches.
        #[arg(long)]
        index_root: Vec<PathBuf>,
        #[arg(long)]
        query: String,
        #[arg(long, default_value_t = 31)]
        warm_queries: usize,
        /// Per-request file bound, not the total library size.
        #[arg(long, default_value_t = 20_000)]
        max_files: usize,
    },
}

#[derive(Debug, Deserialize)]
struct BenchmarkManifest {
    schema_version: u32,
    query_count: usize,
    thresholds: BenchmarkThresholds,
    fixtures: Vec<BenchmarkFixture>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct BenchmarkThresholds {
    exact_source_recall_at_1: f64,
    exact_source_recall_at_5: f64,
    anchor_precision: f64,
    false_positive_rate: f64,
    index_completeness: f64,
    #[serde(default)]
    mean_reciprocal_rank: Option<f64>,
    #[serde(default)]
    reformulation_success: Option<f64>,
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

#[derive(Debug, Clone, Serialize)]
struct BenchmarkMetrics {
    queries: usize,
    positive_queries: usize,
    negative_queries: usize,
    exact_source_recall_at_1: Option<f64>,
    exact_source_recall_at_5: Option<f64>,
    mean_reciprocal_rank: Option<f64>,
    anchor_precision: Option<f64>,
    false_positive_rate: Option<f64>,
    reformulation_queries: usize,
    reformulation_success: Option<f64>,
    negative_no_result_rate: Option<f64>,
    median_latency_ms: Option<f64>,
    p95_latency_ms: Option<f64>,
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
    fixture_schema_version: u32,
    thresholds: BenchmarkThresholds,
    index: BenchmarkIndexMetrics,
    overall: BenchmarkMetrics,
    by_source_type: BTreeMap<String, BenchmarkMetrics>,
    failure_taxonomy_by_source_type: BTreeMap<String, BTreeMap<String, usize>>,
    failures: Vec<BenchmarkFailure>,
}

#[derive(Debug, Serialize)]
struct PerformanceQueryMetrics {
    query: String,
    cold_connection_latency_ms: f64,
    cold_hit_count: usize,
    cold_has_evidence: bool,
    first_source_uri: Option<String>,
    warm_queries: usize,
    warm_median_latency_ms: f64,
    warm_p95_latency_ms: f64,
    warm_min_latency_ms: f64,
    warm_max_latency_ms: f64,
}

#[derive(Debug, Serialize)]
struct PerformanceIndexMetrics {
    discovered: u64,
    indexed: u64,
    unchanged: u64,
    skipped: u64,
    failures: usize,
    source_bytes_read: u64,
    elapsed_ms: f64,
    artifacts_per_second: f64,
    completeness: f64,
}

#[derive(Debug, Serialize)]
struct PerformanceIndexBatch {
    root: String,
    report: loom_core::IndexReport,
}

#[derive(Debug, Serialize)]
struct PerformanceRebuildMetrics {
    elapsed_ms: f64,
    report: loom_core::FtsRepairReport,
}

#[derive(Debug, Serialize)]
struct PerformanceReport {
    schema_version: u32,
    corpus: String,
    max_files: usize,
    cache_conditions: &'static str,
    open_elapsed_ms: f64,
    index: PerformanceIndexMetrics,
    index_batches: Vec<PerformanceIndexBatch>,
    query: PerformanceQueryMetrics,
    fts_rebuild: PerformanceRebuildMetrics,
    stats: loom_core::LibraryStats,
    database_bytes: u64,
    database_bytes_per_source_byte: f64,
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
        Command::ImportBookmarks { path } => {
            let library = Library::open(arguments.database)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&library.import_bookmarks(path)?)?
            );
        }
        Command::Bookmarks { limit } => {
            let library = Library::open(arguments.database)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&library.list_bookmarks(limit)?)?
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
        Command::Export { path } => {
            let library = Library::open(arguments.database)?;
            let export = library.export_portable()?;
            write_private_file(&path, &serde_json::to_vec(&export)?)?;
            println!(
                "{}",
                serde_json::json!({
                    "path": path,
                    "rows": export.row_count(),
                    "digest": export.digest,
                    "encrypted": false,
                })
            );
        }
        Command::ImportExport { path } => {
            let export: PortableExport =
                serde_json::from_slice(&loom_core::read_bounded_file(&path)?)?;
            let library = Library::open(arguments.database)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&library.import_portable(&export)?)?
            );
        }
        Command::Backup {
            path,
            password_file,
        } => {
            let password = read_backup_password(password_file.as_deref())?;
            let library = Library::open(arguments.database)?;
            let report = library.write_encrypted_backup(
                &path,
                password.as_bytes(),
                BackupOptions::default(),
            )?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        Command::Restore {
            backup,
            password_file,
        } => {
            let password = read_backup_password(password_file.as_deref())?;
            let report = Library::restore_encrypted_backup(
                &backup,
                password.as_bytes(),
                &arguments.database,
            )?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        Command::BookmarkImports { limit } => {
            let library = Library::open(arguments.database)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&library.list_bookmark_imports(limit)?)?
            );
        }
        Command::RetryBookmarkImport { import_id } => {
            let library = Library::open(arguments.database)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&library.retry_bookmark_import(&import_id)?)?
            );
        }
        Command::CompactRelationships { max_removals } => {
            let library = Library::open(arguments.database)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&library.compact_relationships(max_removals)?)?
            );
        }
        Command::RelationshipCompactions { limit } => {
            let library = Library::open(arguments.database)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&library.list_relationship_compactions(limit)?)?
            );
        }
        Command::Stats => {
            let library = Library::open(arguments.database)?;
            println!("{}", serde_json::to_string_pretty(&library.stats()?)?);
        }
        Command::StorageInspect => {
            let library = Library::open(arguments.database)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&library.inspect_storage()?)?
            );
        }
        Command::PurgeArtifact { artifact_id } => {
            let library = Library::open(arguments.database)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&library.purge_artifact(&artifact_id)?)?
            );
        }
        Command::PurgeRoot { locator } => {
            let library = Library::open(arguments.database)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&library.purge_root(&locator)?)?
            );
        }
        Command::PurgeBefore { cutoff } => {
            let library = Library::open(arguments.database)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&library.purge_before(&cutoff)?)?
            );
        }
        Command::RetentionStatus => {
            let library = Library::open(arguments.database)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&library.retention_policy()?)?
            );
        }
        Command::RetentionSet { days } => {
            let library = Library::open(arguments.database)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&library.set_retention_days(days)?)?
            );
        }
        Command::RetentionApply => {
            let library = Library::open(arguments.database)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&library.apply_retention()?)?
            );
        }
        Command::PurgeDisposable => {
            let library = Library::open(arguments.database)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&library.purge_disposable_storage()?)?
            );
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
        Command::Jobs { limit } => {
            let library = Library::open_for_jobs(arguments.database)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&library.background_jobs(limit)?)?
            );
        }
        Command::EnqueueFtsRepair {
            idempotency_key,
            high,
            low,
        } => {
            let library = Library::open_for_jobs(arguments.database)?;
            let priority = if high {
                JobPriority::High
            } else if low {
                JobPriority::Low
            } else {
                JobPriority::Normal
            };
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &library.enqueue_fts_repair(&idempotency_key, priority)?
                )?
            );
        }
        Command::EnqueueIndexFile {
            path,
            idempotency_key,
            high,
            low,
        } => {
            let library = Library::open_for_jobs(arguments.database)?;
            let priority = if high {
                JobPriority::High
            } else if low {
                JobPriority::Low
            } else {
                JobPriority::Normal
            };
            println!(
                "{}",
                serde_json::to_string_pretty(&library.enqueue_index_file(
                    path,
                    &idempotency_key,
                    priority
                )?)?
            );
        }
        Command::UpgradeJobRuntime => {
            JobWorker::upgrade_runtime(arguments.database)?;
            println!("{{\"background_job_schema_version\":5}}");
        }
        Command::EnqueueIndexDirectory {
            path,
            idempotency_key,
            high,
            low,
        } => {
            let library = Library::open_for_jobs(arguments.database)?;
            let priority = if high {
                JobPriority::High
            } else if low {
                JobPriority::Low
            } else {
                JobPriority::Normal
            };
            println!(
                "{}",
                serde_json::to_string_pretty(&library.enqueue_index_directory(
                    path,
                    &idempotency_key,
                    priority
                )?)?
            );
        }
        Command::CancelJob { id } => {
            let library = Library::open_for_jobs(arguments.database)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&library.cancel_background_job(&id)?)?
            );
        }
        Command::ForgetJob { id } => {
            let library = Library::open_for_jobs(arguments.database)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&library.forget_background_job(&id)?)?
            );
        }
        Command::RunNextJob => {
            println!(
                "{}",
                serde_json::to_string_pretty(&JobWorker::open(arguments.database)?.run_next()?)?
            );
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
        Command::Performance {
            corpus,
            index_root,
            query,
            warm_queries,
            max_files,
        } => run_performance(
            &arguments.database,
            &corpus,
            &query,
            warm_queries,
            max_files,
            &index_root,
        )?,
    }
    Ok(())
}

/// Writes a new file readable only by the owner (0600 on Unix) through a temporary file and a hard
/// link, so a failed write never leaves a partial file and an existing file is never replaced.
fn write_private_file(path: &Path, bytes: &[u8]) -> Result<(), Box<dyn Error>> {
    use std::io::Write;
    let name = path
        .file_name()
        .ok_or("export path has no file name")?
        .to_string_lossy()
        .into_owned();
    let temporary = path.with_file_name(format!(".{name}.{}.tmp", std::process::id()));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let written = options.open(&temporary).and_then(|mut file| {
        file.write_all(bytes)?;
        file.sync_all()
    });
    let committed = written.and_then(|()| fs::hard_link(&temporary, path));
    let _ = fs::remove_file(&temporary);
    committed.map_err(Into::into)
}

/// Reads the backup password from a file (first line) or LOOM_BACKUP_PASSWORD. Command-line
/// arguments are visible to other local processes, so a password flag is deliberately absent.
/// The password and the file contents are wiped from memory when dropped.
fn read_backup_password(
    password_file: Option<&Path>,
) -> Result<zeroize::Zeroizing<String>, Box<dyn Error>> {
    let password = match password_file {
        Some(path) => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = fs::metadata(path)?.permissions().mode();
                if mode & 0o077 != 0 {
                    eprintln!(
                        "warning: {} is readable by other users; restrict it with chmod 600",
                        path.display()
                    );
                }
            }
            let contents = zeroize::Zeroizing::new(fs::read_to_string(path)?);
            zeroize::Zeroizing::new(contents.lines().next().unwrap_or_default().to_owned())
        }
        None => zeroize::Zeroizing::new(
            std::env::var("LOOM_BACKUP_PASSWORD")
                .map_err(|_| "set LOOM_BACKUP_PASSWORD or pass --password-file")?,
        ),
    };
    if password.is_empty() {
        return Err("backup password is empty".into());
    }
    Ok(password)
}

fn performance_roots(corpus: &Path, selected: &[PathBuf]) -> Result<Vec<PathBuf>, Box<dyn Error>> {
    if selected.len() > 4_096 {
        return Err("performance accepts at most 4096 explicit indexing roots".into());
    }
    // Preserve the core's final-component no-follow policy before resolving
    // a selected alias, including the default corpus selection.
    for path in std::iter::once(corpus).chain(selected.iter().map(PathBuf::as_path)) {
        if fs::symlink_metadata(path)?.file_type().is_symlink() {
            return Err("performance selections must not be final-component symlinks".into());
        }
    }
    let corpus = corpus.canonicalize()?;
    if selected.is_empty() {
        return Ok(vec![corpus]);
    }
    let mut roots = selected
        .iter()
        .map(|root| root.canonicalize())
        .collect::<std::io::Result<Vec<_>>>()?;
    if roots.iter().any(|root| !root.starts_with(&corpus)) {
        return Err("performance indexing roots must be inside the selected corpus".into());
    }
    roots.sort_unstable();
    if roots.windows(2).any(|pair| pair[1].starts_with(&pair[0])) {
        return Err("performance indexing roots must not duplicate or overlap".into());
    }
    Ok(roots)
}

fn index_performance_roots(
    library: &Library,
    roots: &[PathBuf],
) -> Result<Vec<PerformanceIndexBatch>, Box<dyn Error>> {
    let mut batches = Vec::with_capacity(roots.len());
    for root in roots {
        let report = library.index_path(root)?;
        if !report.failures.is_empty() || report.failed != 0 || report.cancelled != 0 {
            return Err(format!(
                "performance batch {} was incomplete: {:?}",
                root.display(),
                report
            )
            .into());
        }
        batches.push(PerformanceIndexBatch {
            root: root.display().to_string(),
            report,
        });
    }
    Ok(batches)
}

fn summarize_performance_index(
    batches: &[PerformanceIndexBatch],
    elapsed_ms: f64,
) -> Result<PerformanceIndexMetrics, Box<dyn Error>> {
    let sum = |field: fn(&loom_core::IndexReport) -> u64| {
        batches.iter().try_fold(0_u64, |total, batch| {
            total
                .checked_add(field(&batch.report))
                .ok_or("performance batch count overflow")
        })
    };
    let discovered = sum(|report| report.discovered)?;
    let indexed = sum(|report| report.indexed)?;
    let unchanged = sum(|report| report.unchanged)?;
    let skipped = sum(|report| report.skipped)?;
    let supported = discovered
        .checked_sub(skipped)
        .ok_or("invalid performance batch counts")?;
    let recovered = indexed
        .checked_add(unchanged)
        .ok_or("performance batch count overflow")?;
    if recovered != supported {
        return Err("performance batches did not index every supported artifact".into());
    }
    Ok(PerformanceIndexMetrics {
        discovered,
        indexed,
        unchanged,
        skipped,
        failures: batches
            .iter()
            .map(|batch| batch.report.failures.len())
            .sum(),
        source_bytes_read: sum(|report| report.bytes_read)?,
        elapsed_ms,
        artifacts_per_second: if elapsed_ms > 0.0 {
            indexed as f64 / (elapsed_ms / 1_000.0)
        } else {
            0.0
        },
        completeness: 1.0,
    })
}

fn run_performance(
    database: &Path,
    corpus: &Path,
    query: &str,
    warm_queries: usize,
    max_files: usize,
    selected_roots: &[PathBuf],
) -> Result<(), Box<dyn Error>> {
    if query.trim().is_empty() {
        return Err("performance query must not be empty".into());
    }
    if max_files == 0 {
        return Err("performance max-files must be greater than zero".into());
    }
    // Validate all explicit selections before opening SQLite or indexing the first batch.
    let roots = performance_roots(corpus, selected_roots)?;
    let corpus = corpus.canonicalize()?;
    let warm_queries = warm_queries.clamp(1, 1_000);
    let limits = LibraryLimits {
        max_files_per_request: max_files,
        ..LibraryLimits::default()
    };

    let open_started = Instant::now();
    let library = Library::open_with_limits(database, limits)?;
    let open_elapsed_ms = open_started.elapsed().as_secs_f64() * 1_000.0;

    let index_started = Instant::now();
    let index_batches = index_performance_roots(&library, &roots)?;
    let index_elapsed_ms = index_started.elapsed().as_secs_f64() * 1_000.0;
    let index = summarize_performance_index(&index_batches, index_elapsed_ms)?;

    // The first query is a cold-connection observation. We deliberately do not attempt to drop
    // the operating-system page cache: that would require privileged, destructive operations on
    // macOS and would not be a portable product check. The report names this condition explicitly.
    let cold_started = Instant::now();
    let cold_hits = library.search(&SearchRequest {
        text: query.to_string(),
        limit: 5,
    })?;
    let cold_connection_latency_ms = cold_started.elapsed().as_secs_f64() * 1_000.0;
    let first_source_uri = cold_hits.first().map(|hit| hit.source_uri.clone());
    let cold_has_evidence = !cold_hits.is_empty()
        && cold_hits.iter().all(|hit| {
            !hit.source_uri.is_empty()
                && !hit.artifact_id.is_empty()
                && !hit.version_id.is_empty()
                && !hit.passage_id.is_empty()
        });

    let mut warm_latencies = Vec::with_capacity(warm_queries);
    for _ in 0..warm_queries {
        let started = Instant::now();
        let hits = library.search(&SearchRequest {
            text: query.to_string(),
            limit: 5,
        })?;
        if hits.is_empty() || hits.iter().any(|hit| hit.source_uri.is_empty()) {
            return Err("performance query lost its evidence-backed hit".into());
        }
        warm_latencies.push(started.elapsed().as_secs_f64() * 1_000.0);
    }
    warm_latencies.sort_by(f64::total_cmp);
    let warm_median_latency_ms = median(&warm_latencies);
    let warm_p95_latency_ms = percentile(&warm_latencies, 0.95);
    let warm_min_latency_ms = warm_latencies.first().copied().unwrap_or(0.0);
    let warm_max_latency_ms = warm_latencies.last().copied().unwrap_or(0.0);

    let rebuild_started = Instant::now();
    let rebuild = library.repair_fts()?;
    let rebuild_elapsed_ms = rebuild_started.elapsed().as_secs_f64() * 1_000.0;
    let stats = library.stats()?;
    let database_bytes = database_size(database);
    let database_bytes_per_source_byte = if index.source_bytes_read == 0 {
        0.0
    } else {
        database_bytes as f64 / index.source_bytes_read as f64
    };

    let report = PerformanceReport {
        schema_version: 2,
        corpus: corpus.display().to_string(),
        max_files,
        cache_conditions:
            "cold = first query after open/index; warm = repeated query; OS page cache not dropped",
        open_elapsed_ms,
        index,
        index_batches,
        query: PerformanceQueryMetrics {
            query: query.to_string(),
            cold_connection_latency_ms,
            cold_hit_count: cold_hits.len(),
            cold_has_evidence,
            first_source_uri,
            warm_queries,
            warm_median_latency_ms,
            warm_p95_latency_ms,
            warm_min_latency_ms,
            warm_max_latency_ms,
        },
        fts_rebuild: PerformanceRebuildMetrics {
            elapsed_ms: rebuild_elapsed_ms,
            report: rebuild,
        },
        stats,
        database_bytes,
        database_bytes_per_source_byte,
    };
    println!("{}", serde_json::to_string_pretty(&report)?);
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
        schema_version: 4,
        fixture_schema_version: manifest.schema_version,
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
            manifest.thresholds.mean_reciprocal_rank.unwrap_or(0.0),
        ),
        (
            "reformulation_success",
            manifest.thresholds.reformulation_success.unwrap_or(0.0),
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
    let ratio = |numerator: f64, denominator: usize| {
        (denominator > 0).then(|| numerator / denominator as f64)
    };
    BenchmarkMetrics {
        queries: accumulator.queries,
        positive_queries: accumulator.positive_queries,
        negative_queries: accumulator.negative_queries,
        exact_source_recall_at_1: ratio(accumulator.top_one as f64, accumulator.positive_queries),
        exact_source_recall_at_5: ratio(accumulator.top_five as f64, accumulator.positive_queries),
        mean_reciprocal_rank: ratio(accumulator.mrr_sum, accumulator.positive_queries),
        anchor_precision: ratio(
            accumulator.anchor_correct as f64,
            accumulator.anchor_candidates,
        ),
        false_positive_rate: ratio(accumulator.false_positives as f64, accumulator.returned),
        reformulation_queries: accumulator.reformulation_queries,
        reformulation_success: ratio(
            accumulator.reformulation_successes as f64,
            accumulator.reformulation_queries,
        ),
        negative_no_result_rate: ratio(
            accumulator.negative_no_result as f64,
            accumulator.negative_queries,
        ),
        median_latency_ms: (!accumulator.latencies.is_empty())
            .then(|| median(&accumulator.latencies)),
        p95_latency_ms: (!accumulator.latencies.is_empty())
            .then(|| percentile(&accumulator.latencies, 0.95)),
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
    metrics
        .exact_source_recall_at_1
        .is_some_and(|value| value + EPSILON >= thresholds.exact_source_recall_at_1)
        && metrics
            .exact_source_recall_at_5
            .is_some_and(|value| value + EPSILON >= thresholds.exact_source_recall_at_5)
        && thresholds.mean_reciprocal_rank.is_none_or(|threshold| {
            metrics
                .mean_reciprocal_rank
                .is_some_and(|value| value + EPSILON >= threshold)
        })
        && metrics
            .anchor_precision
            .is_some_and(|value| value + EPSILON >= thresholds.anchor_precision)
        && metrics
            .false_positive_rate
            .is_some_and(|value| value <= thresholds.false_positive_rate + EPSILON)
        && thresholds.reformulation_success.is_none_or(|threshold| {
            metrics
                .reformulation_success
                .is_some_and(|value| value + EPSILON >= threshold)
        })
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
    use clap::Parser;
    use std::path::PathBuf;

    use loom_core::{
        EvidenceAnchor, EvidenceExcerpt, EvidenceSegment, OcrConfidenceState, RankContributions,
        SearchHit,
    };

    use super::{
        benchmark_passes, expected_source_anchor_matches, finalize_metrics, fixture_path_matches,
        matching_expectation, median, percentile, phrase_is_highlighted, update_accumulator,
        validate_expected_anchor, BenchmarkAccumulator, BenchmarkAlternative, BenchmarkAnchor,
        BenchmarkQuery, BenchmarkThresholds, QueryEvaluation,
    };

    #[test]
    fn background_job_commands_parse_and_reject_conflicting_priorities() {
        for arguments in [
            vec!["loom", "jobs", "--limit", "20"],
            vec!["loom", "enqueue-fts-repair", "device-check", "--high"],
            vec!["loom", "cancel-job", "job-id"],
            vec!["loom", "run-next-job"],
        ] {
            assert!(super::Arguments::try_parse_from(arguments).is_ok());
        }
        assert!(super::Arguments::try_parse_from([
            "loom",
            "enqueue-fts-repair",
            "device-check",
            "--high",
            "--low",
        ])
        .is_err());
    }

    #[test]
    fn performance_accepts_explicit_repeated_index_roots() {
        let parsed = super::Arguments::try_parse_from([
            "loom",
            "performance",
            "--corpus",
            "/corpus",
            "--query",
            "marker",
            "--index-root",
            "/corpus/shard-001",
            "--index-root",
            "/corpus/shard-000",
        ])
        .unwrap();
        let super::Command::Performance { index_root, .. } = parsed.command else {
            panic!("not a performance command");
        };
        assert_eq!(
            index_root,
            vec![
                PathBuf::from("/corpus/shard-001"),
                PathBuf::from("/corpus/shard-000")
            ]
        );
    }

    #[test]
    fn performance_roots_are_canonical_disjoint_and_bounded() {
        let corpus = tempfile::tempdir().unwrap();
        let root = corpus.path().canonicalize().unwrap();
        let first = root.join("shard-000");
        let second = root.join("shard-001");
        std::fs::create_dir(&first).unwrap();
        std::fs::create_dir(&second).unwrap();
        assert_eq!(
            super::performance_roots(&root, &[]).unwrap(),
            vec![root.clone()]
        );
        assert_eq!(
            super::performance_roots(&root, &[second.clone(), first.clone()]).unwrap(),
            vec![first.clone(), second]
        );
        for invalid in [
            vec![first.clone(), first.clone()],
            vec![root.clone(), first.clone()],
            vec![first; 4_097],
        ] {
            assert!(super::performance_roots(&root, &invalid).is_err());
        }
    }

    #[test]
    fn performance_rejects_outside_roots_before_creating_a_library() {
        let corpus = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let database = corpus.path().join("not-created.sqlite3");
        let result = super::run_performance(
            &database,
            corpus.path(),
            "marker",
            1,
            1,
            &[outside.path().to_owned()],
        );
        assert!(result.is_err());
        assert!(!database.exists());
    }

    #[cfg(unix)]
    #[test]
    fn performance_rejects_a_root_linked_outside_the_corpus() {
        let corpus = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let link = corpus.path().join("shard-escape");
        std::os::unix::fs::symlink(outside.path(), &link).unwrap();
        assert!(super::performance_roots(corpus.path(), &[link]).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn performance_rejects_inside_corpus_root_aliases_before_opening_sqlite() {
        let corpus = tempfile::tempdir().unwrap();
        let target = corpus.path().join("real-shard");
        let alias = corpus.path().join("alias");
        std::fs::create_dir(&target).unwrap();
        std::os::unix::fs::symlink(&target, &alias).unwrap();
        assert!(super::performance_roots(corpus.path(), std::slice::from_ref(&alias)).is_err());
        let database = corpus.path().join("not-created.sqlite3");
        assert!(super::run_performance(&database, &alias, "marker", 1, 1, &[]).is_err());
        assert!(!database.exists());
    }

    #[test]
    fn performance_batches_preserve_per_request_limits_and_count_the_whole_library() {
        let corpus = tempfile::tempdir().unwrap();
        let root = corpus.path().canonicalize().unwrap();
        let first = root.join("shard-000");
        let second = root.join("shard-001");
        std::fs::create_dir(&first).unwrap();
        std::fs::create_dir(&second).unwrap();
        std::fs::write(first.join("first.md"), "first marker").unwrap();
        std::fs::write(second.join("second.md"), "second marker").unwrap();
        let database = tempfile::tempdir().unwrap();
        let library = loom_core::Library::open_with_limits(
            database.path().join("library.sqlite3"),
            loom_core::LibraryLimits {
                max_files_per_request: 1,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(library.index_path(&root).is_err());
        let batches =
            super::index_performance_roots(&library, &[first.clone(), second.clone()]).unwrap();
        assert_eq!(batches.len(), 2);
        assert_eq!(batches[0].root, first.to_string_lossy());
        assert_eq!(batches[1].root, second.to_string_lossy());
        assert!(batches
            .iter()
            .all(|batch| !batch.report.run_id.is_empty() && batch.report.indexed == 1));
        let summary = super::summarize_performance_index(&batches, 1.0).unwrap();
        assert_eq!(summary.discovered, 2);
        assert_eq!(summary.indexed, 2);
        assert_eq!(summary.completeness, 1.0);
        assert_eq!(library.stats().unwrap().artifacts, 2);
        let unchanged = super::index_performance_roots(&library, &[first, second]).unwrap();
        let summary = super::summarize_performance_index(&unchanged, 1.0).unwrap();
        assert_eq!(summary.indexed, 0);
        assert_eq!(summary.unchanged, 2);
        assert_eq!(summary.completeness, 1.0);
    }

    #[test]
    fn performance_batch_failure_preserves_completed_batches_but_returns_no_summary() {
        let corpus = tempfile::tempdir().unwrap();
        let first = corpus.path().join("shard-000");
        let second = corpus.path().join("shard-001");
        std::fs::create_dir(&first).unwrap();
        std::fs::create_dir(&second).unwrap();
        std::fs::write(first.join("first.md"), "first marker").unwrap();
        std::fs::write(second.join("second.md"), "second marker").unwrap();
        std::fs::write(second.join("third.md"), "third marker").unwrap();
        let database = tempfile::tempdir().unwrap();
        let library = loom_core::Library::open_with_limits(
            database.path().join("library.sqlite3"),
            loom_core::LibraryLimits {
                max_files_per_request: 1,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(super::index_performance_roots(&library, &[first, second]).is_err());
        assert_eq!(library.stats().unwrap().artifacts, 1);
    }

    #[test]
    fn performance_summary_rejects_partial_or_overflowing_counts() {
        let mut batches = vec![super::PerformanceIndexBatch {
            root: "fixture".into(),
            report: loom_core::IndexReport {
                discovered: 2,
                indexed: 1,
                ..Default::default()
            },
        }];
        assert!(super::summarize_performance_index(&batches, 1.0).is_err());
        batches[0].report.discovered = u64::MAX;
        batches.push(super::PerformanceIndexBatch {
            root: "other fixture".into(),
            report: loom_core::IndexReport {
                discovered: 1,
                ..Default::default()
            },
        });
        assert!(super::summarize_performance_index(&batches, 1.0).is_err());
    }

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
        assert_eq!(metrics.mean_reciprocal_rank, Some(0.5));
        assert_eq!(metrics.reformulation_success, Some(1.0));
        assert_eq!(metrics.negative_no_result_rate, Some(1.0));
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
            confidence_state: OcrConfidenceState::Confirmed,
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
            mean_reciprocal_rank: None,
            reformulation_success: None,
        };
        let passing = super::BenchmarkMetrics {
            queries: 3,
            positive_queries: 3,
            negative_queries: 0,
            exact_source_recall_at_1: Some(1.0),
            exact_source_recall_at_5: Some(1.0),
            mean_reciprocal_rank: Some(1.0),
            anchor_precision: Some(1.0),
            false_positive_rate: Some(0.0),
            reformulation_queries: 0,
            reformulation_success: None,
            negative_no_result_rate: None,
            median_latency_ms: Some(1.0),
            p95_latency_ms: Some(2.0),
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
            false_positive_rate: Some(0.01),
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
        let missing_recall = super::BenchmarkMetrics {
            exact_source_recall_at_1: None,
            ..passing.clone()
        };
        assert!(!benchmark_passes(&thresholds, &missing_recall, 1.0));
        let missing_mrr = super::BenchmarkMetrics {
            mean_reciprocal_rank: None,
            ..passing.clone()
        };
        let requires_mrr = BenchmarkThresholds {
            mean_reciprocal_rank: Some(0.0),
            ..thresholds.clone()
        };
        assert!(!benchmark_passes(&requires_mrr, &missing_mrr, 1.0));
        assert!(benchmark_passes(&thresholds, &missing_mrr, 1.0));
        let requires_reformulation = BenchmarkThresholds {
            reformulation_success: Some(0.0),
            ..thresholds
        };
        assert!(!benchmark_passes(&requires_reformulation, &passing, 1.0));
    }

    #[test]
    fn empty_metric_categories_are_unmeasured_instead_of_invented_zeroes() {
        let metrics = finalize_metrics(BenchmarkAccumulator::default());
        let json = serde_json::to_value(metrics).unwrap();
        for name in [
            "exact_source_recall_at_1",
            "exact_source_recall_at_5",
            "mean_reciprocal_rank",
            "anchor_precision",
            "false_positive_rate",
            "reformulation_success",
            "negative_no_result_rate",
            "median_latency_ms",
            "p95_latency_ms",
        ] {
            assert!(json[name].is_null(), "{name} has no observations");
        }
    }

    #[test]
    fn negative_only_metrics_do_not_invent_recall_and_measured_failure_stays_zero() {
        let mut accumulator = BenchmarkAccumulator::default();
        let negative = QueryEvaluation {
            negative: true,
            top_one: false,
            top_five: false,
            anchor_correct: false,
            mrr: 0.0,
            returned: 0,
            false_positives: 0,
            negative_no_result: true,
            failure_kind: None,
        };
        update_accumulator(&mut accumulator, &negative, None, 1.0);
        let metrics = finalize_metrics(accumulator);
        assert_eq!(metrics.exact_source_recall_at_1, None);
        assert_eq!(metrics.false_positive_rate, None);
        assert_eq!(metrics.reformulation_success, None);
        assert_eq!(metrics.negative_no_result_rate, Some(1.0));

        let mut failed = BenchmarkAccumulator::default();
        let unsupported = QueryEvaluation {
            returned: 1,
            false_positives: 1,
            negative_no_result: false,
            ..negative
        };
        update_accumulator(&mut failed, &unsupported, None, 2.0);
        let metrics = finalize_metrics(failed);
        assert_eq!(metrics.negative_no_result_rate, Some(0.0));
        assert_eq!(metrics.false_positive_rate, Some(1.0));
    }
}
