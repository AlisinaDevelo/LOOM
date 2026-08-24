use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard},
};

use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use uuid::Uuid;

use crate::{
    domain::{
        ArtifactObservation, EvidenceAnchor, FtsHealthReport, FtsRepairReport,
        IndexCancellationToken, IndexCheckpoint, IndexFailure, IndexReport, LibraryStats,
        ObservationReport, PassageObservation, SearchHit, SearchRequest, SourceRootInfo,
        SourceRootStatus,
    },
    error::{io_error, LoomError, Result},
    ingest::{self, PassageDraft, StableDocument, EXTRACTOR_ID, EXTRACTOR_VERSION},
    observe::{self, ObservationEvent},
    search::{collision_free_markers, compile_query, project_fts_evidence},
};

const SCHEMA_VERSION: i64 = 3;
const PREVIOUS_SCHEMA_VERSION: i64 = 2;
const LEGACY_SCHEMA_TABLES: &[&str] = &[
    "source_roots",
    "artifacts",
    "artifact_locators",
    "artifact_versions",
    "passages",
    "relationships",
];
const CURRENT_SCHEMA_TABLES: &[&str] = &[
    "source_roots",
    "artifacts",
    "artifact_locators",
    "artifact_versions",
    "passages",
    "relationships",
    "index_jobs",
];

/// Resource boundaries applied to every ingestion request.
#[derive(Debug, Clone, Copy)]
pub struct LibraryLimits {
    pub max_file_bytes: u64,
    pub max_files_per_request: usize,
    pub passage_target_chars: usize,
    pub passage_overlap_chars: usize,
}

impl Default for LibraryLimits {
    fn default() -> Self {
        Self {
            max_file_bytes: 8 * 1024 * 1024,
            max_files_per_request: 20_000,
            passage_target_chars: 1_000,
            passage_overlap_chars: 120,
        }
    }
}

/// A single-process modular-monolith library backed by canonical SQLite records.
pub struct Library {
    connection: Mutex<Connection>,
    limits: LibraryLimits,
}

impl Library {
    /// Opens or creates a persistent library.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_with_limits(path, LibraryLimits::default())
    }

    /// Opens a persistent library with explicit ingestion limits.
    pub fn open_with_limits(path: impl AsRef<Path>, limits: LibraryLimits) -> Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|source| io_error(parent, source))?;
        }
        let connection = Connection::open(path)?;
        Self::from_connection(connection, limits)
    }

    /// Opens an isolated in-memory library for tests and evaluation.
    pub fn open_in_memory() -> Result<Self> {
        Self::from_connection(Connection::open_in_memory()?, LibraryLimits::default())
    }

    fn from_connection(mut connection: Connection, limits: LibraryLimits) -> Result<Self> {
        configure(&connection)?;
        migrate(&mut connection)?;
        Ok(Self {
            connection: Mutex::new(connection),
            limits,
        })
    }

    /// Indexes one explicitly selected regular file or directory.
    pub fn index_path(&self, selected_path: impl AsRef<Path>) -> Result<IndexReport> {
        let cancellation = IndexCancellationToken::new();
        self.index_path_with_options(selected_path, &cancellation, None, None)
    }

    /// Indexes one explicitly selected regular file or directory with cooperative cancellation.
    ///
    /// The token is observed between bounded ingestion units. A cancellation request therefore
    /// never interrupts a canonical artifact/version transaction midway through its commit.
    pub fn index_path_with_cancellation(
        &self,
        selected_path: impl AsRef<Path>,
        cancellation: &IndexCancellationToken,
    ) -> Result<IndexReport> {
        self.index_path_with_options(selected_path, cancellation, None, None)
    }

    /// Reconciles an in-scope event batch against the approved root's current bytes.
    ///
    /// Events are hints only: even a non-overflow batch triggers a content-hash root scan. This
    /// deliberately favors correctness over trusting a lossy or reordered watcher stream.
    pub fn reconcile_events(
        &self,
        selected_root: impl AsRef<Path>,
        events: &[ObservationEvent],
        max_events: usize,
    ) -> Result<ObservationReport> {
        let selected_path = canonical_selected_root(selected_root.as_ref())?;
        let selected_uri = utf8_path(&selected_path)?;
        if !self.is_approved_root(&selected_uri)? {
            return Err(LoomError::InvalidPath(format!(
                "root is not an enabled approved source: {selected_uri}"
            )));
        }
        let plan = observe::coalesce_events(&selected_path, events, max_events)?;
        if plan.events_received == 0 {
            return Ok(ObservationReport::default());
        }
        let index = self.index_path(&selected_path)?;
        Ok(observation_from_index(
            &index,
            plan.events_received,
            plan.paths_coalesced,
        ))
    }

    /// Reconciles every enabled root persisted by an earlier explicit selection.
    ///
    /// This bounded startup/polling pass is the restart-safe observation fallback until a native
    /// event adapter is selected. Missing or revoked roots become explicit failures and cannot
    /// widen the scan to an arbitrary directory.
    pub fn reconcile_approved_roots(&self) -> Result<ObservationReport> {
        let roots = self.approved_root_specs()?;
        let mut report = ObservationReport::default();
        for (root, kind) in roots {
            report.roots_scanned += 1;
            let status = source_root_status(&root, &kind, true);
            if status != SourceRootStatus::Available {
                report.roots_failed += 1;
                report.full_rescans += 1;
                report.failures.push(IndexFailure {
                    source: root,
                    reason: format!("persisted source root is {status:?}"),
                });
                continue;
            }
            match self.index_path(&root) {
                Ok(index) => merge_observation_index(&mut report, &index),
                Err(error) => {
                    report.roots_failed += 1;
                    report.full_rescans += 1;
                    report.failures.push(IndexFailure {
                        source: root,
                        reason: error.to_string(),
                    });
                }
            }
        }
        Ok(report)
    }

    /// Lists persisted user-selected roots without widening their access scope.
    ///
    /// Status is derived from the exact persisted locator. A missing, denied, moved, or unsafe
    /// root is reported rather than replaced with a broader fallback directory.
    pub fn source_roots(&self) -> Result<Vec<SourceRootInfo>> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare("SELECT locator, kind, enabled FROM source_roots ORDER BY locator")?;
        let rows = statement.query_map([], |row| {
            let locator: String = row.get(0)?;
            let kind: String = row.get(1)?;
            let enabled: i64 = row.get(2)?;
            Ok(SourceRootInfo {
                status: source_root_status(&locator, &kind, enabled != 0),
                locator,
                kind,
                enabled: enabled != 0,
                read_only: true,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// Revokes a previously persisted root and hides its active evidence.
    ///
    /// The canonical rows remain on disk for explicit local retention/export policy, but revoked
    /// artifacts are no longer searchable or openable. Re-selection through the folder picker is
    /// the only path that re-enables the exact root.
    pub fn revoke_source_root(&self, locator: &str) -> Result<SourceRootInfo> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        let exists: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM source_roots WHERE locator = ?1)",
            [locator],
            |row| row.get(0),
        )?;
        if !exists {
            return Err(LoomError::InvalidPath(format!(
                "source root is not persisted: {locator}"
            )));
        }
        transaction.execute(
            "UPDATE source_roots SET enabled = 0, last_seen_at = ?1 WHERE locator = ?2",
            params![Utc::now().to_rfc3339(), locator],
        )?;
        transaction.execute(
            "UPDATE artifacts SET state = 'missing', last_seen_at = ?1
             WHERE source_root_id = (SELECT id FROM source_roots WHERE locator = ?2)
               AND state = 'active'",
            params![Utc::now().to_rfc3339(), locator],
        )?;
        transaction.commit()?;
        drop(connection);
        self.source_roots()?
            .into_iter()
            .find(|root| root.locator == locator)
            .ok_or_else(|| {
                LoomError::InvalidPath(format!(
                    "source root disappeared during revocation: {locator}"
                ))
            })
    }

    /// Returns the durable checkpoint for a selected file or directory, when one exists.
    pub fn index_checkpoint(
        &self,
        selected_path: impl AsRef<Path>,
    ) -> Result<Option<IndexCheckpoint>> {
        let requested_path = selected_path.as_ref();
        let requested_metadata = fs::symlink_metadata(requested_path)
            .map_err(|source| io_error(requested_path, source))?;
        if requested_metadata.file_type().is_symlink() {
            return Err(LoomError::InvalidPath(format!(
                "symbolic links are not followed: {}",
                requested_path.display()
            )));
        }
        let selected_path = requested_path
            .canonicalize()
            .map_err(|source| io_error(requested_path, source))?;
        let selected_uri = utf8_path(&selected_path)?;
        let connection = self.lock()?;
        connection
            .query_row(
                "SELECT j.id, j.state, j.next_unit, j.total_units, j.last_error
                 FROM index_jobs j
                 JOIN source_roots r ON r.id = j.source_root_id
                 WHERE r.locator = ?1 AND j.selection_locator = ?1",
                [&selected_uri],
                |row| {
                    Ok(IndexCheckpoint {
                        job_id: row.get(0)?,
                        state: row.get(1)?,
                        next_unit: row.get::<_, i64>(2)?.max(0) as u64,
                        total_units: row.get::<_, i64>(3)?.max(0) as u64,
                        last_error: row.get(4)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    /// Test-only fault injection that interrupts after `units` completed units.
    ///
    /// The hook is intentionally explicit and is not used by the normal indexing path. It lets
    /// the integration suite simulate a process termination at a durable unit boundary without
    /// relying on timing or killing the test runner.
    #[doc(hidden)]
    pub fn index_path_with_fault(
        &self,
        selected_path: impl AsRef<Path>,
        interrupt_after_units: Option<usize>,
    ) -> Result<IndexReport> {
        let cancellation = IndexCancellationToken::new();
        self.index_path_with_options(selected_path, &cancellation, interrupt_after_units, None)
    }

    /// Deterministic cancellation hook used by integration fixtures.
    #[doc(hidden)]
    pub fn index_path_with_cancellation_after(
        &self,
        selected_path: impl AsRef<Path>,
        cancel_after_units: usize,
    ) -> Result<IndexReport> {
        let cancellation = IndexCancellationToken::new();
        self.index_path_with_options(selected_path, &cancellation, None, Some(cancel_after_units))
    }

    fn index_path_with_options(
        &self,
        selected_path: impl AsRef<Path>,
        cancellation: &IndexCancellationToken,
        interrupt_after_units: Option<usize>,
        cancel_after_units: Option<usize>,
    ) -> Result<IndexReport> {
        let requested_path = selected_path.as_ref();
        let requested_metadata = fs::symlink_metadata(requested_path)
            .map_err(|source| io_error(requested_path, source))?;
        if requested_metadata.file_type().is_symlink() {
            return Err(LoomError::InvalidPath(format!(
                "symbolic links are not followed: {}",
                requested_path.display()
            )));
        }
        let selected_path = requested_path
            .canonicalize()
            .map_err(|source| io_error(requested_path, source))?;
        let selected_uri = utf8_path(&selected_path)?;
        let discovered = ingest::discover(&selected_path, self.limits.max_files_per_request)?;

        let discovery_fingerprint = discovery_fingerprint(&discovered);
        let root_id = {
            let mut connection = self.lock()?;
            ensure_source_root(&mut connection, &selected_uri, selected_path.is_dir())?
        };
        let job = self.start_index_job(
            &root_id,
            &selected_uri,
            &discovery_fingerprint,
            discovered.len(),
        )?;

        let mut report = IndexReport {
            run_id: job.job_id.clone(),
            discovered: discovered.len() as u64,
            ..IndexReport::default()
        };
        let mut seen = HashSet::new();
        let mut units_processed_this_run = 0usize;
        for path in &discovered {
            if ingest::supported_media_type(path).is_some() {
                if let Ok(locator) = utf8_path(path) {
                    seen.insert(locator);
                }
            }
        }
        for (unit, path) in discovered
            .into_iter()
            .enumerate()
            .skip(job.next_unit as usize)
        {
            if cancel_after_units.is_some_and(|limit| units_processed_this_run >= limit) {
                cancellation.cancel();
            }
            if cancellation.is_cancelled() {
                report.cancelled = report
                    .discovered
                    .saturating_sub(job.next_unit.saturating_add(report.attempted));
                self.interrupt_index_job(&job.job_id, "cancelled by request")?;
                return Ok(report);
            }
            if interrupt_after_units.is_some_and(|limit| units_processed_this_run >= limit) {
                let message = format!(
                    "fault injection after {} completed unit(s)",
                    units_processed_this_run
                );
                self.interrupt_index_job(&job.job_id, &message)?;
                return Err(LoomError::IndexInterrupted(job.job_id));
            }
            report.attempted += 1;
            let locator = match utf8_path(&path) {
                Ok(locator) => locator,
                Err(error) => {
                    report.failed += 1;
                    report.failures.push(IndexFailure {
                        source: path.display().to_string(),
                        reason: error.to_string(),
                    });
                    self.advance_index_job(&job.job_id, unit as u64 + 1)?;
                    units_processed_this_run += 1;
                    continue;
                }
            };
            if ingest::supported_media_type(&path).is_none() {
                report.skipped += 1;
                if let Err(error) = self.mark_locator_missing_and_advance(
                    &root_id,
                    &locator,
                    &job.job_id,
                    unit as u64 + 1,
                ) {
                    report.failures.push(IndexFailure {
                        source: path.display().to_string(),
                        reason: format!("could not reconcile source state: {error}"),
                    });
                    report.failed += 1;
                }
                units_processed_this_run += 1;
                continue;
            }
            match ingest::read_stable(&path, &selected_path, self.limits.max_file_bytes) {
                Ok(document) => {
                    let bytes = document.byte_size;
                    report.bytes_read += bytes;
                    match self.index_document_with_checkpoint(
                        &root_id,
                        &path,
                        document,
                        &job.job_id,
                        unit as u64 + 1,
                    ) {
                        Ok(true) => {
                            report.indexed += 1;
                        }
                        Ok(false) => {
                            report.unchanged += 1;
                        }
                        Err(error) => {
                            let reason = match self.mark_locator_missing_and_advance(
                                &root_id,
                                &locator,
                                &job.job_id,
                                unit as u64 + 1,
                            ) {
                                Ok(()) => error.to_string(),
                                Err(reconcile_error) => format!(
                                    "{error}; could not reconcile source state: {reconcile_error}"
                                ),
                            };
                            report.failures.push(IndexFailure {
                                source: path.display().to_string(),
                                reason,
                            });
                            report.failed += 1;
                        }
                    }
                }
                Err(error) => {
                    let reason = match self.mark_locator_missing_and_advance(
                        &root_id,
                        &locator,
                        &job.job_id,
                        unit as u64 + 1,
                    ) {
                        Ok(()) => error.to_string(),
                        Err(reconcile_error) => {
                            format!("{error}; could not reconcile source state: {reconcile_error}")
                        }
                    };
                    report.failures.push(IndexFailure {
                        source: path.display().to_string(),
                        reason,
                    });
                    report.failed += 1;
                }
            }
            units_processed_this_run += 1;
        }
        if cancellation.is_cancelled() {
            report.cancelled = report
                .discovered
                .saturating_sub(job.next_unit.saturating_add(report.attempted));
            self.interrupt_index_job(&job.job_id, "cancelled by request")?;
            return Ok(report);
        }
        if selected_path.is_dir() {
            if let Err(error) = self.reconcile_directory(&root_id, &seen) {
                self.fail_index_job(&job.job_id, &error.to_string())?;
                return Err(error);
            }
        }
        self.complete_index_job(
            &job.job_id,
            report
                .failures
                .first()
                .map(|failure| failure.reason.as_str()),
        )?;
        Ok(report)
    }

    fn index_document_with_checkpoint(
        &self,
        root_id: &str,
        path: &Path,
        document: StableDocument,
        job_id: &str,
        next_unit: u64,
    ) -> Result<bool> {
        self.index_document_with_extractor_and_checkpoint(
            root_id,
            path,
            document,
            EXTRACTOR_ID,
            EXTRACTOR_VERSION,
            Some((job_id, next_unit)),
        )
    }

    #[cfg(test)]
    fn index_document_with_extractor(
        &self,
        root_id: &str,
        path: &Path,
        document: StableDocument,
        extractor_id: &str,
        extractor_version: &str,
    ) -> Result<bool> {
        self.index_document_with_extractor_and_checkpoint(
            root_id,
            path,
            document,
            extractor_id,
            extractor_version,
            None,
        )
    }

    fn index_document_with_extractor_and_checkpoint(
        &self,
        root_id: &str,
        path: &Path,
        document: StableDocument,
        extractor_id: &str,
        extractor_version: &str,
        checkpoint: Option<(&str, u64)>,
    ) -> Result<bool> {
        let source_uri = utf8_path(path)?;
        let title = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or(&source_uri)
            .to_string();
        let passages = ingest::split_passages(
            &document.normalized_text,
            self.limits.passage_target_chars,
            self.limits.passage_overlap_chars,
        );
        let now = Utc::now().to_rfc3339();
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;

        let artifact_id: String = transaction
            .query_row(
                "SELECT artifact_id FROM artifact_locators WHERE kind = 'file' AND locator = ?1 AND active = 1",
                [&source_uri],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or_else(|| Uuid::new_v4().to_string());

        transaction.execute(
            "INSERT INTO artifacts(id, source_root_id, title, media_type, state, created_at, last_seen_at)
             VALUES (?1, ?2, ?3, ?4, 'active', ?5, ?5)
             ON CONFLICT(id) DO UPDATE SET title = excluded.title, media_type = excluded.media_type,
               source_root_id = excluded.source_root_id, state = 'active', last_seen_at = excluded.last_seen_at",
            params![artifact_id, root_id, title, document.media_type, now],
        )?;
        transaction.execute(
            "INSERT INTO artifact_locators(id, artifact_id, kind, locator, active, first_seen_at, last_seen_at)
             VALUES (?1, ?2, 'file', ?3, 1, ?4, ?4)
             ON CONFLICT(kind, locator) DO UPDATE SET artifact_id = excluded.artifact_id,
               active = 1, last_seen_at = excluded.last_seen_at",
            params![Uuid::new_v4().to_string(), artifact_id, source_uri, now],
        )?;

        let current_projection: Option<(String, String, String)> = transaction
            .query_row(
                "SELECT v.content_hash, v.extractor_id, v.extractor_version FROM artifacts a
                 JOIN artifact_versions v ON v.id = a.active_version_id WHERE a.id = ?1",
                [&artifact_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        if current_projection.as_ref().is_some_and(|projection| {
            projection.0 == document.raw_hash
                && projection.1 == extractor_id
                && projection.2 == extractor_version
        }) {
            if let Some((job_id, next_unit)) = checkpoint {
                update_index_job_checkpoint(&transaction, job_id, next_unit, &now)?;
            }
            transaction.commit()?;
            return Ok(false);
        }

        let existing_version: Option<String> = transaction
            .query_row(
                "SELECT id FROM artifact_versions
                 WHERE artifact_id = ?1 AND content_hash = ?2
                   AND extractor_id = ?3 AND extractor_version = ?4",
                params![
                    artifact_id,
                    document.raw_hash,
                    extractor_id,
                    extractor_version
                ],
                |row| row.get(0),
            )
            .optional()?;
        let version_id = existing_version.unwrap_or_else(|| Uuid::new_v4().to_string());
        let inserted = transaction.execute(
            "INSERT OR IGNORE INTO artifact_versions(
                id, artifact_id, content_hash, hash_algorithm, byte_size, source_modified_ns,
                extractor_id, extractor_version, status, created_at
             ) VALUES (?1, ?2, ?3, 'blake3', ?4, ?5, ?6, ?7, 'ready', ?8)",
            params![
                version_id,
                artifact_id,
                document.raw_hash,
                sql_i64(document.byte_size, "artifact byte size")?,
                document.modified_ns,
                extractor_id,
                extractor_version,
                now
            ],
        )?;
        if inserted > 0 {
            insert_passages(&transaction, &version_id, &passages, &now)?;
        }
        transaction.execute(
            "UPDATE artifacts SET active_version_id = ?1, last_seen_at = ?2 WHERE id = ?3",
            params![version_id, now, artifact_id],
        )?;
        if let Some((job_id, next_unit)) = checkpoint {
            update_index_job_checkpoint(&transaction, job_id, next_unit, &now)?;
        }
        transaction.commit()?;
        Ok(true)
    }

    /// Searches active versions and returns direct evidence locators.
    pub fn search(&self, request: &SearchRequest) -> Result<Vec<SearchHit>> {
        let compiled = compile_query(&request.text)?;
        let limit = request.limit.clamp(1, 100);
        let connection = self.lock()?;
        let candidates = {
            let mut statement = connection.prepare_cached(
                "SELECT
                    a.id, v.id, p.id, a.title, a.media_type, l.locator, v.content_hash,
                    p.text, p.locator_json, bm25(passages_fts), p.rowid
                 FROM passages_fts
                 JOIN passages p ON p.rowid = passages_fts.rowid
                 JOIN artifact_versions v ON v.id = p.artifact_version_id
                 JOIN artifacts a ON a.id = v.artifact_id AND a.active_version_id = v.id
                 JOIN artifact_locators l ON l.artifact_id = a.id AND l.active = 1
                 WHERE passages_fts MATCH ?1 AND a.state = 'active'
                 ORDER BY bm25(passages_fts), a.title, a.id, p.ordinal
                 LIMIT ?2",
            )?;
            let rows =
                statement.query_map(params![compiled.match_expression.as_str(), limit], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, String>(7)?,
                        row.get::<_, String>(8)?,
                        row.get::<_, f64>(9)?,
                        row.get::<_, i64>(10)?,
                    ))
                })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        let mut highlight_statement = connection.prepare_cached(
            "SELECT highlight(passages_fts, 0, ?2, ?3)
             FROM passages_fts
             WHERE rowid = ?1 AND passages_fts MATCH ?4",
        )?;

        let mut hits = Vec::new();
        for (index, row) in candidates.into_iter().enumerate() {
            let (
                artifact_id,
                version_id,
                passage_id,
                title,
                media_type,
                source_uri,
                content_hash,
                passage_text,
                locator_json,
                raw_bm25,
                passage_rowid,
            ) = row;
            let (start_marker, end_marker) = collision_free_markers(&passage_text);
            let highlighted: String = highlight_statement.query_row(
                params![
                    passage_rowid,
                    start_marker,
                    end_marker,
                    compiled.match_expression.as_str()
                ],
                |highlighted_row| highlighted_row.get(0),
            )?;
            let passage_anchor: EvidenceAnchor = serde_json::from_str(&locator_json)?;
            let (excerpt, anchor) = project_fts_evidence(
                &passage_text,
                &highlighted,
                &passage_anchor,
                &start_marker,
                &end_marker,
            )?;
            hits.push(SearchHit {
                rank: index as u32 + 1,
                score: 1.0 / (1.0 + raw_bm25.abs()),
                artifact_id,
                version_id,
                passage_id,
                title,
                media_type,
                source_uri,
                content_hash,
                excerpt,
                anchor,
                match_reason: "SQLite FTS5 BM25 over the active source passage".into(),
            });
        }
        Ok(hits)
    }

    /// Verifies a search result against the active version and current source bytes.
    pub fn resolve_verified_artifact_path(
        &self,
        artifact_id: &str,
        version_id: &str,
        content_hash: &str,
    ) -> Result<PathBuf> {
        Uuid::parse_str(artifact_id)
            .map_err(|_| LoomError::ArtifactNotFound(artifact_id.to_string()))?;
        Uuid::parse_str(version_id)
            .map_err(|_| LoomError::ArtifactStale(artifact_id.to_string()))?;
        let connection = self.lock()?;
        let source: Option<(String, String, String, String, i64)> = connection
            .query_row(
                "SELECT l.locator, r.locator, v.id, v.content_hash, v.byte_size FROM artifacts a
                 JOIN artifact_versions v ON v.id = a.active_version_id
                 JOIN artifact_locators l ON l.artifact_id = a.id AND l.active = 1
                 JOIN source_roots r ON r.id = a.source_root_id
                 WHERE a.id = ?1 AND a.state = 'active' AND l.kind = 'file'",
                [artifact_id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .optional()?;
        drop(connection);
        let (source, root, stored_version_id, stored_hash, byte_size) =
            source.ok_or_else(|| LoomError::ArtifactNotFound(artifact_id.to_string()))?;
        if stored_version_id != version_id || stored_hash != content_hash {
            return Err(LoomError::ArtifactStale(artifact_id.to_string()));
        }

        let byte_size = u64::try_from(byte_size)
            .map_err(|_| LoomError::ArtifactStale(artifact_id.to_string()))?;
        let expected_locator = source.clone();
        let expected_root = root.clone();
        let expected_version_id = stored_version_id.clone();
        let expected_hash = stored_hash.clone();
        let path = PathBuf::from(&source);
        let root_path = PathBuf::from(&root);
        let actual_hash = ingest::read_stable_hash(&path, &root_path, byte_size)
            .map_err(|_| LoomError::ArtifactStale(artifact_id.to_string()))?;
        if actual_hash != stored_hash {
            return Err(LoomError::ArtifactStale(artifact_id.to_string()));
        }

        let connection = self.lock()?;
        let current: Option<(String, String, String, String)> = connection
            .query_row(
                "SELECT l.locator, r.locator, v.id, v.content_hash FROM artifacts a
                 JOIN artifact_versions v ON v.id = a.active_version_id
                 JOIN artifact_locators l ON l.artifact_id = a.id AND l.active = 1
                 JOIN source_roots r ON r.id = a.source_root_id
                 WHERE a.id = ?1 AND a.state = 'active' AND l.kind = 'file'",
                [artifact_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?;
        if current.as_ref()
            != Some(&(
                expected_locator,
                expected_root,
                expected_version_id,
                expected_hash,
            ))
        {
            return Err(LoomError::ArtifactStale(artifact_id.to_string()));
        }
        Ok(path)
    }

    fn start_index_job(
        &self,
        root_id: &str,
        selection_locator: &str,
        discovery_fingerprint: &str,
        total_units: usize,
    ) -> Result<IndexJobProgress> {
        let total_units = sql_i64(total_units as u64, "index job unit count")?;
        let now = Utc::now().to_rfc3339();
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        let existing: Option<(String, String, i64, i64, String)> = transaction
            .query_row(
                "SELECT id, state, next_unit, total_units, discovery_fingerprint
                 FROM index_jobs
                 WHERE source_root_id = ?1 AND selection_locator = ?2",
                params![root_id, selection_locator],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .optional()?;
        let (job_id, next_unit) = match existing {
            Some((job_id, state, next_unit, previous_total, previous_fingerprint))
                if matches!(state.as_str(), "running" | "interrupted")
                    && previous_total == total_units
                    && previous_fingerprint == discovery_fingerprint =>
            {
                transaction.execute(
                    "UPDATE index_jobs
                     SET state = 'running', updated_at = ?1, last_error = NULL
                     WHERE id = ?2",
                    params![now, job_id],
                )?;
                (job_id, next_unit.max(0) as u64)
            }
            Some((job_id, ..)) => {
                transaction.execute(
                    "UPDATE index_jobs
                     SET state = 'running', discovery_fingerprint = ?1, total_units = ?2,
                         next_unit = 0, started_at = ?3, updated_at = ?3,
                         completed_at = NULL, last_error = NULL
                     WHERE id = ?4",
                    params![discovery_fingerprint, total_units, now, job_id],
                )?;
                (job_id, 0)
            }
            None => {
                let job_id = Uuid::new_v4().to_string();
                transaction.execute(
                    "INSERT INTO index_jobs(
                        id, source_root_id, selection_locator, discovery_fingerprint,
                        total_units, next_unit, state, last_error, started_at, updated_at,
                        completed_at
                     ) VALUES (?1, ?2, ?3, ?4, ?5, 0, 'running', NULL, ?6, ?6, NULL)",
                    params![
                        job_id,
                        root_id,
                        selection_locator,
                        discovery_fingerprint,
                        total_units,
                        now
                    ],
                )?;
                (job_id, 0)
            }
        };
        transaction.commit()?;
        Ok(IndexJobProgress { job_id, next_unit })
    }

    fn is_approved_root(&self, locator: &str) -> Result<bool> {
        let connection = self.lock()?;
        connection
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM source_roots WHERE locator = ?1 AND enabled = 1
                )",
                [locator],
                |row| row.get(0),
            )
            .map_err(Into::into)
    }

    fn approved_root_specs(&self) -> Result<Vec<(String, String)>> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare("SELECT locator, kind FROM source_roots WHERE enabled = 1 ORDER BY locator")?;
        let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    fn mark_locator_missing_and_advance(
        &self,
        root_id: &str,
        locator: &str,
        job_id: &str,
        next_unit: u64,
    ) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        transaction.execute(
            "UPDATE artifacts SET state = 'missing'
             WHERE source_root_id = ?1 AND state = 'active' AND id IN (
                 SELECT artifact_id FROM artifact_locators
                 WHERE artifact_id = artifacts.id AND kind = 'file' AND active = 1 AND locator = ?2
             )",
            params![root_id, locator],
        )?;
        update_index_job_checkpoint(&transaction, job_id, next_unit, &now)?;
        transaction.commit()?;
        Ok(())
    }

    fn interrupt_index_job(&self, job_id: &str, message: &str) -> Result<()> {
        let connection = self.lock()?;
        connection.execute(
            "UPDATE index_jobs SET state = 'interrupted', last_error = ?1, updated_at = ?2
             WHERE id = ?3",
            params![message, Utc::now().to_rfc3339(), job_id],
        )?;
        Ok(())
    }

    fn fail_index_job(&self, job_id: &str, message: &str) -> Result<()> {
        let connection = self.lock()?;
        connection.execute(
            "UPDATE index_jobs SET state = 'failed', last_error = ?1, updated_at = ?2
             WHERE id = ?3",
            params![message, Utc::now().to_rfc3339(), job_id],
        )?;
        Ok(())
    }

    fn complete_index_job(&self, job_id: &str, last_error: Option<&str>) -> Result<()> {
        let connection = self.lock()?;
        connection.execute(
            "UPDATE index_jobs
             SET state = 'completed', next_unit = total_units, last_error = ?1,
                 updated_at = ?2, completed_at = ?2
             WHERE id = ?3",
            params![last_error, Utc::now().to_rfc3339(), job_id],
        )?;
        Ok(())
    }

    fn advance_index_job(&self, job_id: &str, next_unit: u64) -> Result<()> {
        let connection = self.lock()?;
        let transaction = connection.unchecked_transaction()?;
        update_index_job_checkpoint(&transaction, job_id, next_unit, &Utc::now().to_rfc3339())?;
        transaction.commit()?;
        Ok(())
    }

    fn reconcile_directory(&self, root_id: &str, seen: &HashSet<String>) -> Result<()> {
        let mut connection = self.lock()?;
        let candidates: Vec<(String, String)> = {
            let mut statement = connection.prepare(
                "SELECT a.id, l.locator
                 FROM artifacts a
                 JOIN artifact_locators l ON l.artifact_id = a.id
                   AND l.kind = 'file' AND l.active = 1
                 WHERE a.source_root_id = ?1 AND a.state = 'active'",
            )?;
            let rows = statement.query_map([root_id], |row| Ok((row.get(0)?, row.get(1)?)))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        let transaction = connection.transaction()?;
        for (artifact_id, locator) in candidates {
            if !seen.contains(&locator) {
                transaction.execute(
                    "UPDATE artifacts SET state = 'missing'
                     WHERE id = ?1 AND source_root_id = ?2 AND state = 'active'",
                    params![artifact_id, root_id],
                )?;
            }
        }
        transaction.commit()?;
        Ok(())
    }

    /// Returns the active canonical extractor projection for one indexed source.
    pub fn inspect_source(&self, source_path: impl AsRef<Path>) -> Result<ArtifactObservation> {
        let requested_path = source_path.as_ref();
        let requested_metadata = fs::symlink_metadata(requested_path)
            .map_err(|source| io_error(requested_path, source))?;
        if requested_metadata.file_type().is_symlink() || !requested_metadata.is_file() {
            return Err(LoomError::InvalidPath(format!(
                "source is not a regular non-symlink file: {}",
                requested_path.display()
            )));
        }
        let source_path = requested_path
            .canonicalize()
            .map_err(|source| io_error(requested_path, source))?;
        let source_uri = utf8_path(&source_path)?;
        let connection = self.lock()?;
        let version: Option<(String, String, String, String)> = connection
            .query_row(
                "SELECT v.id, v.content_hash, v.extractor_id, v.extractor_version
                 FROM artifact_locators l
                 JOIN artifacts a ON a.id = l.artifact_id
                 JOIN artifact_versions v ON v.id = a.active_version_id
                 WHERE l.kind = 'file' AND l.locator = ?1 AND l.active = 1
                   AND a.state = 'active'",
                [&source_uri],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?;
        let (version_id, content_hash, extractor_id, extractor_version) =
            version.ok_or_else(|| LoomError::ArtifactNotFound(source_uri.clone()))?;
        let passages = {
            let mut statement = connection.prepare_cached(
                "SELECT ordinal, text_hash, locator_json
                 FROM passages WHERE artifact_version_id = ?1 ORDER BY ordinal",
            )?;
            let rows = statement.query_map([version_id], |row| {
                let ordinal: i64 = row.get(0)?;
                let locator_json: String = row.get(2)?;
                let anchor = serde_json::from_str(&locator_json).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        2,
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                })?;
                let ordinal = u32::try_from(ordinal).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        0,
                        rusqlite::types::Type::Integer,
                        Box::new(error),
                    )
                })?;
                Ok(PassageObservation {
                    ordinal,
                    text_hash: row.get(1)?,
                    anchor,
                })
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        Ok(ArtifactObservation {
            source_uri,
            content_hash,
            extractor_id,
            extractor_version,
            passages,
        })
    }

    /// Compares canonical passage content with the derived FTS5 vocabulary and row coverage.
    pub fn fts_health(&self) -> Result<FtsHealthReport> {
        let connection = self.lock()?;
        fts_health(&connection)
    }

    /// Rebuilds the disposable FTS5 projection in one transaction and retains before/after proof.
    ///
    /// Canonical passage rows are read for the health comparison but are never updated by this
    /// operation. Re-running repair on a healthy projection is a no-op with the same digest.
    pub fn repair_fts(&self) -> Result<FtsRepairReport> {
        let mut connection = self.lock()?;
        let before = fts_health(&connection)?;
        let transaction = connection.transaction()?;
        transaction.execute(
            "INSERT INTO passages_fts(passages_fts) VALUES ('rebuild')",
            [],
        )?;
        transaction.commit()?;
        let after = fts_health(&connection)?;
        Ok(FtsRepairReport { before, after })
    }

    /// Returns canonical record counts and source byte totals.
    pub fn stats(&self) -> Result<LibraryStats> {
        let connection = self.lock()?;
        let indexed_bytes: i64 = connection.query_row(
            "SELECT COALESCE(SUM(byte_size), 0) FROM artifact_versions",
            [],
            |row| row.get(0),
        )?;
        Ok(LibraryStats {
            source_roots: count(&connection, "source_roots")?,
            artifacts: count(&connection, "artifacts")?,
            versions: count(&connection, "artifact_versions")?,
            passages: count(&connection, "passages")?,
            indexed_bytes: indexed_bytes.max(0) as u64,
        })
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>> {
        self.connection.lock().map_err(|_| LoomError::LockPoisoned)
    }
}

fn canonical_selected_root(requested_path: &Path) -> Result<PathBuf> {
    let metadata =
        fs::symlink_metadata(requested_path).map_err(|source| io_error(requested_path, source))?;
    if metadata.file_type().is_symlink() {
        return Err(LoomError::InvalidPath(format!(
            "symbolic links are not followed: {}",
            requested_path.display()
        )));
    }
    requested_path
        .canonicalize()
        .map_err(|source| io_error(requested_path, source))
}

fn source_root_status(locator: &str, kind: &str, enabled: bool) -> SourceRootStatus {
    if !enabled {
        return SourceRootStatus::Revoked;
    }
    match fs::symlink_metadata(locator) {
        Ok(metadata) if metadata.file_type().is_symlink() => SourceRootStatus::Unsafe,
        Ok(metadata) if kind == "file" && metadata.is_file() => match fs::File::open(locator) {
            Ok(_) => SourceRootStatus::Available,
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
                SourceRootStatus::Denied
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => SourceRootStatus::Missing,
            Err(_) => SourceRootStatus::Unavailable,
        },
        Ok(metadata) if kind == "directory" && metadata.is_dir() => match fs::read_dir(locator) {
            Ok(_) => SourceRootStatus::Available,
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
                SourceRootStatus::Denied
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => SourceRootStatus::Missing,
            Err(_) => SourceRootStatus::Unavailable,
        },
        Ok(_) => SourceRootStatus::WrongType,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => SourceRootStatus::Missing,
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
            SourceRootStatus::Denied
        }
        Err(_) => SourceRootStatus::Unavailable,
    }
}

fn observation_from_index(
    index: &IndexReport,
    events_received: u64,
    paths_coalesced: u64,
) -> ObservationReport {
    let mut report = ObservationReport {
        roots_scanned: 1,
        events_received,
        paths_coalesced,
        ..ObservationReport::default()
    };
    merge_observation_index(&mut report, index);
    report
}

fn merge_observation_index(report: &mut ObservationReport, index: &IndexReport) {
    report.full_rescans += 1;
    report.indexed += index.indexed;
    report.unchanged += index.unchanged;
    report.skipped += index.skipped;
    report.bytes_read += index.bytes_read;
    report.failures.extend(index.failures.iter().cloned());
}

struct IndexJobProgress {
    job_id: String,
    next_unit: u64,
}

fn update_index_job_checkpoint(
    transaction: &Transaction<'_>,
    job_id: &str,
    next_unit: u64,
    now: &str,
) -> Result<()> {
    transaction.execute(
        "UPDATE index_jobs
         SET next_unit = ?1, updated_at = ?2
         WHERE id = ?3 AND state = 'running'",
        params![sql_i64(next_unit, "index job progress")?, now, job_id],
    )?;
    Ok(())
}

fn discovery_fingerprint(paths: &[PathBuf]) -> String {
    let mut hasher = blake3::Hasher::new();
    for path in paths {
        hasher.update(path.to_string_lossy().as_bytes());
        hasher.update(&[0]);
    }
    format!("blake3:{}", hasher.finalize().to_hex())
}

fn configure(connection: &Connection) -> Result<()> {
    connection.busy_timeout(std::time::Duration::from_secs(5))?;
    connection.execute_batch(
        "PRAGMA foreign_keys = ON;
         PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         PRAGMA temp_store = MEMORY;",
    )?;
    Ok(())
}

fn migrate(connection: &mut Connection) -> Result<()> {
    let existing_version = stored_schema_version(connection)?;
    if let Some(version) = existing_version {
        if version != SCHEMA_VERSION && version != PREVIOUS_SCHEMA_VERSION {
            return Err(LoomError::UnsupportedSchemaVersion(version.to_string()));
        }
        validate_schema_shape(connection, version)?;
    }
    let transaction = connection.transaction()?;
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_meta(
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
         ) STRICT;

         CREATE TABLE IF NOT EXISTS source_roots(
            id TEXT PRIMARY KEY,
            kind TEXT NOT NULL CHECK(kind IN ('file', 'directory')),
            locator TEXT NOT NULL UNIQUE,
            enabled INTEGER NOT NULL DEFAULT 1 CHECK(enabled IN (0, 1)),
            created_at TEXT NOT NULL,
            last_seen_at TEXT NOT NULL
         ) STRICT;

         CREATE TABLE IF NOT EXISTS artifacts(
            id TEXT PRIMARY KEY,
            source_root_id TEXT NOT NULL REFERENCES source_roots(id),
            title TEXT NOT NULL,
            media_type TEXT NOT NULL,
            state TEXT NOT NULL CHECK(state IN ('active', 'missing', 'tombstoned')),
            active_version_id TEXT REFERENCES artifact_versions(id) ON DELETE SET NULL,
            created_at TEXT NOT NULL,
            last_seen_at TEXT NOT NULL
         ) STRICT;

         CREATE TABLE IF NOT EXISTS artifact_locators(
            id TEXT PRIMARY KEY,
            artifact_id TEXT NOT NULL REFERENCES artifacts(id) ON DELETE CASCADE,
            kind TEXT NOT NULL CHECK(kind IN ('file', 'url', 'managed_copy')),
            locator TEXT NOT NULL,
            active INTEGER NOT NULL DEFAULT 1 CHECK(active IN (0, 1)),
            first_seen_at TEXT NOT NULL,
            last_seen_at TEXT NOT NULL,
            UNIQUE(kind, locator)
         ) STRICT;
         CREATE UNIQUE INDEX IF NOT EXISTS artifact_one_active_locator
           ON artifact_locators(artifact_id) WHERE active = 1;

         CREATE TABLE IF NOT EXISTS artifact_versions(
            id TEXT PRIMARY KEY,
            artifact_id TEXT NOT NULL REFERENCES artifacts(id) ON DELETE CASCADE,
            content_hash TEXT NOT NULL,
            hash_algorithm TEXT NOT NULL,
            byte_size INTEGER NOT NULL CHECK(byte_size >= 0),
            source_modified_ns INTEGER,
            extractor_id TEXT NOT NULL,
            extractor_version TEXT NOT NULL,
            status TEXT NOT NULL CHECK(status IN ('ready', 'failed', 'superseded')),
            created_at TEXT NOT NULL,
            UNIQUE(artifact_id, content_hash, extractor_id, extractor_version)
         ) STRICT;

         CREATE TABLE IF NOT EXISTS passages(
            id TEXT PRIMARY KEY,
            artifact_version_id TEXT NOT NULL REFERENCES artifact_versions(id) ON DELETE CASCADE,
            ordinal INTEGER NOT NULL CHECK(ordinal >= 0),
            text TEXT NOT NULL,
            text_hash TEXT NOT NULL,
            locator_json TEXT NOT NULL CHECK(json_valid(locator_json)),
            char_start INTEGER NOT NULL CHECK(char_start >= 0),
            char_end INTEGER NOT NULL CHECK(char_end >= char_start),
            line_start INTEGER NOT NULL CHECK(line_start >= 1),
            line_end INTEGER NOT NULL CHECK(line_end >= line_start),
            created_at TEXT NOT NULL,
            UNIQUE(artifact_version_id, ordinal)
         ) STRICT;

         CREATE TABLE IF NOT EXISTS relationships(
            id TEXT PRIMARY KEY,
            source_artifact_id TEXT NOT NULL REFERENCES artifacts(id) ON DELETE CASCADE,
            target_artifact_id TEXT NOT NULL REFERENCES artifacts(id) ON DELETE CASCADE,
            kind TEXT NOT NULL,
            evidence_passage_id TEXT REFERENCES passages(id) ON DELETE SET NULL,
            confidence REAL,
            method TEXT NOT NULL,
            created_at TEXT NOT NULL,
            CHECK(source_artifact_id <> target_artifact_id)
         ) STRICT;

         CREATE TABLE IF NOT EXISTS index_jobs(
            id TEXT PRIMARY KEY,
            source_root_id TEXT NOT NULL REFERENCES source_roots(id) ON DELETE CASCADE,
            selection_locator TEXT NOT NULL,
            discovery_fingerprint TEXT NOT NULL,
            total_units INTEGER NOT NULL CHECK(total_units >= 0),
            next_unit INTEGER NOT NULL CHECK(next_unit >= 0 AND next_unit <= total_units),
            state TEXT NOT NULL CHECK(state IN ('running', 'interrupted', 'completed', 'failed')),
            last_error TEXT,
            started_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            completed_at TEXT,
            UNIQUE(source_root_id, selection_locator)
         ) STRICT;

         CREATE VIRTUAL TABLE IF NOT EXISTS passages_fts USING fts5(
            text,
            content = 'passages',
            content_rowid = 'rowid',
            tokenize = 'unicode61 remove_diacritics 2'
         );
         CREATE VIRTUAL TABLE IF NOT EXISTS passages_fts_vocab
           USING fts5vocab(passages_fts, 'row');
         CREATE VIRTUAL TABLE IF NOT EXISTS passages_fts_instances
           USING fts5vocab(passages_fts, 'instance');

         CREATE TRIGGER IF NOT EXISTS passages_ai AFTER INSERT ON passages BEGIN
            INSERT INTO passages_fts(rowid, text) VALUES (new.rowid, new.text);
         END;
         CREATE TRIGGER IF NOT EXISTS passages_ad AFTER DELETE ON passages BEGIN
            INSERT INTO passages_fts(passages_fts, rowid, text)
              VALUES ('delete', old.rowid, old.text);
         END;
         CREATE TRIGGER IF NOT EXISTS passages_au AFTER UPDATE ON passages BEGIN
            INSERT INTO passages_fts(passages_fts, rowid, text)
              VALUES ('delete', old.rowid, old.text);
            INSERT INTO passages_fts(rowid, text) VALUES (new.rowid, new.text);
         END;",
    )?;
    transaction.execute(
        "INSERT INTO schema_meta(key, value) VALUES ('schema_version', ?1)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [SCHEMA_VERSION.to_string()],
    )?;
    transaction.commit()?;
    connection.execute_batch("PRAGMA trusted_schema = OFF;")?;
    rebuild_fts(connection)?;
    Ok(())
}

fn validate_schema_shape(connection: &Connection, version: i64) -> Result<()> {
    let tables = if version == SCHEMA_VERSION {
        CURRENT_SCHEMA_TABLES
    } else {
        LEGACY_SCHEMA_TABLES
    };
    for table in tables {
        let exists: bool = connection.query_row(
            "SELECT EXISTS(
                SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1
             )",
            [table],
            |row| row.get(0),
        )?;
        if !exists {
            return Err(LoomError::UnsupportedSchemaVersion(format!(
                "schema version {version} is missing required table `{table}`"
            )));
        }
    }
    for (table, column) in [
        ("source_roots", "id"),
        ("source_roots", "kind"),
        ("source_roots", "locator"),
        ("source_roots", "enabled"),
        ("source_roots", "created_at"),
        ("source_roots", "last_seen_at"),
        ("artifacts", "id"),
        ("artifacts", "source_root_id"),
        ("artifacts", "title"),
        ("artifacts", "media_type"),
        ("artifacts", "state"),
        ("artifacts", "active_version_id"),
        ("artifacts", "created_at"),
        ("artifacts", "last_seen_at"),
        ("artifact_locators", "id"),
        ("artifact_locators", "artifact_id"),
        ("artifact_locators", "kind"),
        ("artifact_locators", "locator"),
        ("artifact_locators", "active"),
        ("artifact_locators", "first_seen_at"),
        ("artifact_locators", "last_seen_at"),
        ("artifact_versions", "id"),
        ("artifact_versions", "artifact_id"),
        ("artifact_versions", "content_hash"),
        ("artifact_versions", "hash_algorithm"),
        ("artifact_versions", "byte_size"),
        ("artifact_versions", "source_modified_ns"),
        ("artifact_versions", "extractor_id"),
        ("artifact_versions", "extractor_version"),
        ("artifact_versions", "status"),
        ("artifact_versions", "created_at"),
        ("passages", "id"),
        ("passages", "artifact_version_id"),
        ("passages", "ordinal"),
        ("passages", "text"),
        ("passages", "text_hash"),
        ("passages", "locator_json"),
        ("passages", "char_start"),
        ("passages", "char_end"),
        ("passages", "line_start"),
        ("passages", "line_end"),
        ("passages", "created_at"),
        ("relationships", "id"),
        ("relationships", "source_artifact_id"),
        ("relationships", "target_artifact_id"),
        ("relationships", "kind"),
        ("relationships", "evidence_passage_id"),
        ("relationships", "confidence"),
        ("relationships", "method"),
        ("relationships", "created_at"),
    ]
    .into_iter()
    .chain((version == SCHEMA_VERSION).then_some(("index_jobs", "id")))
    .chain((version == SCHEMA_VERSION).then_some(("index_jobs", "source_root_id")))
    .chain((version == SCHEMA_VERSION).then_some(("index_jobs", "selection_locator")))
    .chain((version == SCHEMA_VERSION).then_some(("index_jobs", "discovery_fingerprint")))
    .chain((version == SCHEMA_VERSION).then_some(("index_jobs", "total_units")))
    .chain((version == SCHEMA_VERSION).then_some(("index_jobs", "next_unit")))
    .chain((version == SCHEMA_VERSION).then_some(("index_jobs", "state")))
    .chain((version == SCHEMA_VERSION).then_some(("index_jobs", "last_error")))
    .chain((version == SCHEMA_VERSION).then_some(("index_jobs", "started_at")))
    .chain((version == SCHEMA_VERSION).then_some(("index_jobs", "updated_at")))
    .chain((version == SCHEMA_VERSION).then_some(("index_jobs", "completed_at")))
    {
        let exists: bool = connection.query_row(
            "SELECT EXISTS(
                SELECT 1 FROM pragma_table_info(?1) WHERE name = ?2
             )",
            rusqlite::params![table, column],
            |row| row.get(0),
        )?;
        if !exists {
            return Err(LoomError::UnsupportedSchemaVersion(format!(
                "schema version {version} is missing required column `{table}.{column}`"
            )));
        }
    }
    Ok(())
}

fn rebuild_fts(connection: &Connection) -> Result<()> {
    connection.execute(
        "INSERT INTO passages_fts(passages_fts) VALUES ('rebuild')",
        [],
    )?;
    Ok(())
}

fn fts_health(connection: &Connection) -> Result<FtsHealthReport> {
    let canonical_passages: i64 =
        connection.query_row("SELECT COUNT(*) FROM passages", [], |row| row.get(0))?;
    let indexed_passages: i64 = connection.query_row(
        "SELECT COUNT(DISTINCT doc) FROM passages_fts_instances",
        [],
        |row| row.get(0),
    )?;
    let canonical_digest = canonical_passage_digest(connection)?;
    let expected_derivative_digest = expected_fts_digest(connection)?;
    let derivative_digest = vocabulary_digest(connection, "passages_fts_vocab")?;
    let integrity_error = connection
        .execute(
            "INSERT INTO passages_fts(passages_fts) VALUES ('integrity-check')",
            [],
        )
        .err()
        .map(|error| error.to_string());
    let healthy = canonical_passages.max(0) as u64 == indexed_passages.max(0) as u64
        && expected_derivative_digest == derivative_digest
        && integrity_error.is_none();
    Ok(FtsHealthReport {
        canonical_passages: canonical_passages.max(0) as u64,
        indexed_passages: indexed_passages.max(0) as u64,
        canonical_digest,
        expected_derivative_digest,
        derivative_digest,
        integrity_ok: integrity_error.is_none(),
        integrity_error,
        healthy,
    })
}

fn canonical_passage_digest(connection: &Connection) -> Result<String> {
    let mut hasher = blake3::Hasher::new();
    let mut statement =
        connection.prepare("SELECT rowid, text_hash FROM passages ORDER BY rowid")?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
    })?;
    for row in rows {
        let (rowid, text_hash) = row?;
        hasher.update(&rowid.to_le_bytes());
        hasher.update(text_hash.as_bytes());
        hasher.update(&[0]);
    }
    Ok(format!("blake3:{}", hasher.finalize().to_hex()))
}

fn expected_fts_digest(connection: &Connection) -> Result<String> {
    connection.execute_batch(
        "DROP TABLE IF EXISTS temp.loom_expected_fts_vocab;
         DROP TABLE IF EXISTS temp.loom_expected_fts;
         CREATE VIRTUAL TABLE temp.loom_expected_fts USING fts5(
             text, tokenize = 'unicode61 remove_diacritics 2'
         );
         INSERT INTO temp.loom_expected_fts(rowid, text)
             SELECT rowid, text FROM passages;
         CREATE VIRTUAL TABLE temp.loom_expected_fts_vocab
             USING fts5vocab(loom_expected_fts, 'row');",
    )?;
    let result = vocabulary_digest(connection, "temp.loom_expected_fts_vocab");
    connection.execute_batch(
        "DROP TABLE IF EXISTS temp.loom_expected_fts_vocab;
         DROP TABLE IF EXISTS temp.loom_expected_fts;",
    )?;
    result
}

fn vocabulary_digest(connection: &Connection, table: &str) -> Result<String> {
    debug_assert!(matches!(
        table,
        "passages_fts_vocab" | "temp.loom_expected_fts_vocab"
    ));
    let mut hasher = blake3::Hasher::new();
    let query = format!("SELECT term, doc, cnt FROM {table} ORDER BY term");
    let mut statement = connection.prepare(&query)?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
        ))
    })?;
    for row in rows {
        let (term, doc, count) = row?;
        hasher.update(term.as_bytes());
        hasher.update(&[0]);
        hasher.update(&doc.to_le_bytes());
        hasher.update(&count.to_le_bytes());
    }
    Ok(format!("blake3:{}", hasher.finalize().to_hex()))
}

fn stored_schema_version(connection: &Connection) -> Result<Option<i64>> {
    let table_exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'schema_meta')",
        [],
        |row| row.get(0),
    )?;
    if !table_exists {
        let has_existing_schema: bool = connection.query_row(
            "SELECT EXISTS(
                SELECT 1 FROM sqlite_master
                WHERE type IN ('table', 'view', 'trigger', 'index')
                  AND name NOT LIKE 'sqlite_%'
             )",
            [],
            |row| row.get(0),
        )?;
        if has_existing_schema {
            return Err(LoomError::UnsupportedSchemaVersion(
                "schema_version table is missing from a non-empty database".into(),
            ));
        }
        return Ok(None);
    }
    let value: Option<String> = connection
        .query_row(
            "SELECT value FROM schema_meta WHERE key = 'schema_version'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    let value = value.ok_or_else(|| {
        LoomError::UnsupportedSchemaVersion("schema_version record is missing".into())
    })?;
    value.parse::<i64>().map(Some).map_err(|_| {
        LoomError::UnsupportedSchemaVersion(format!("schema_version is not an integer: {value}"))
    })
}

fn ensure_source_root(
    connection: &mut Connection,
    locator: &str,
    directory: bool,
) -> Result<String> {
    let now = Utc::now().to_rfc3339();
    let existing: Option<String> = connection
        .query_row(
            "SELECT id FROM source_roots WHERE locator = ?1",
            [locator],
            |row| row.get(0),
        )
        .optional()?;
    let id = existing.unwrap_or_else(|| Uuid::new_v4().to_string());
    connection.execute(
        "INSERT INTO source_roots(id, kind, locator, enabled, created_at, last_seen_at)
         VALUES (?1, ?2, ?3, 1, ?4, ?4)
         ON CONFLICT(locator) DO UPDATE SET enabled = 1, last_seen_at = excluded.last_seen_at",
        params![
            id,
            if directory { "directory" } else { "file" },
            locator,
            now
        ],
    )?;
    Ok(id)
}

fn insert_passages(
    transaction: &Transaction<'_>,
    version_id: &str,
    passages: &[PassageDraft],
    now: &str,
) -> Result<()> {
    let mut statement = transaction.prepare_cached(
        "INSERT INTO passages(
            id, artifact_version_id, ordinal, text, text_hash, locator_json,
            char_start, char_end, line_start, line_end, created_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
    )?;
    for passage in passages {
        let EvidenceAnchor::Text {
            char_start,
            char_end,
            line_start,
            line_end,
        } = passage.anchor;
        statement.execute(params![
            Uuid::new_v4().to_string(),
            version_id,
            passage.ordinal,
            passage.text,
            passage.text_hash,
            serde_json::to_string(&passage.anchor)?,
            sql_i64(char_start, "passage start")?,
            sql_i64(char_end, "passage end")?,
            sql_i64(line_start, "line start")?,
            sql_i64(line_end, "line end")?,
            now
        ])?;
    }
    Ok(())
}

fn count(connection: &Connection, table: &str) -> Result<u64> {
    let sql = format!("SELECT COUNT(*) FROM {table}");
    let value: i64 = connection.query_row(&sql, [], |row| row.get(0))?;
    Ok(value.max(0) as u64)
}

fn sql_i64(value: u64, field: &str) -> Result<i64> {
    i64::try_from(value)
        .map_err(|_| LoomError::InvalidPath(format!("{field} exceeds SQLite's integer range")))
}

fn utf8_path(path: &Path) -> Result<String> {
    path.to_str().map(ToOwned::to_owned).ok_or_else(|| {
        LoomError::InvalidPath(format!("path is not valid UTF-8: {}", path.display()))
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::{Library, LibraryLimits};
    use crate::{ingest, EvidenceAnchor, LoomError, SearchRequest};

    #[test]
    fn indexes_searches_versions_and_verifies_original() {
        let directory = tempdir().unwrap();
        let source = directory.path().join("isolation.md");
        fs::write(
            &source,
            "# Database notes\nSerializable isolation prevents retry anomalies.\n",
        )
        .unwrap();
        let library = Library::open(directory.path().join("loom.sqlite3")).unwrap();

        let first = library.index_path(&source).unwrap();
        assert_eq!(first.indexed, 1);

        let hits = library
            .search(&SearchRequest {
                text: "\"retry anomalies\"".into(),
                limit: 10,
            })
            .unwrap();
        assert_eq!(hits.len(), 1);
        let hit = hits[0].clone();

        let second = library.index_path(&source).unwrap();
        assert_eq!(second.unchanged, 1);
        assert_eq!(second.bytes_read, fs::metadata(&source).unwrap().len());
        let unchanged_hit = library
            .search(&SearchRequest {
                text: "\"retry anomalies\"".into(),
                limit: 10,
            })
            .unwrap()
            .remove(0);
        assert_eq!(unchanged_hit.artifact_id, hit.artifact_id);
        assert_eq!(unchanged_hit.version_id, hit.version_id);
        let unchanged_stats = library.stats().unwrap();
        assert_eq!(unchanged_stats.artifacts, 1);
        assert_eq!(unchanged_stats.versions, 1);
        assert_eq!(unchanged_stats.passages, 1);

        assert_eq!(
            library
                .resolve_verified_artifact_path(
                    &hit.artifact_id,
                    &hit.version_id,
                    &hit.content_hash,
                )
                .unwrap(),
            source.canonicalize().unwrap()
        );
        assert!(matches!(
            library.resolve_verified_artifact_path(
                &hit.artifact_id,
                &hit.version_id,
                "blake3:wrong-hash",
            ),
            Err(LoomError::ArtifactStale(_))
        ));

        fs::write(
            &source,
            "# Database notes\nSerializable isolation prevents write skew here.\n",
        )
        .unwrap();
        assert!(matches!(
            library.resolve_verified_artifact_path(
                &hit.artifact_id,
                &hit.version_id,
                &hit.content_hash,
            ),
            Err(LoomError::ArtifactStale(_))
        ));
        assert_eq!(library.index_path(&source).unwrap().indexed, 1);
        let stats = library.stats().unwrap();
        assert_eq!(stats.artifacts, 1);
        assert_eq!(stats.versions, 2);
        assert_eq!(stats.passages, 2);
        assert!(library
            .search(&SearchRequest {
                text: "anomalies".into(),
                limit: 10,
            })
            .unwrap()
            .is_empty());
        let updated = library
            .search(&SearchRequest {
                text: "write skew".into(),
                limit: 10,
            })
            .unwrap();
        assert_eq!(updated.len(), 1);
        assert_eq!(updated[0].artifact_id, hit.artifact_id);
        assert_ne!(updated[0].version_id, hit.version_id);
    }

    #[test]
    fn empty_database_migration_records_schema_and_runtime_guards() {
        let directory = tempdir().unwrap();
        let database = directory.path().join("empty.sqlite3");
        let library = Library::open(&database).unwrap();
        let connection = library.lock().unwrap();

        let schema_version: String = connection
            .query_row(
                "SELECT value FROM schema_meta WHERE key = 'schema_version'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(schema_version, "3");

        let foreign_keys: i64 = connection
            .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
            .unwrap();
        assert_eq!(foreign_keys, 1);
        let journal_mode: String = connection
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .unwrap();
        assert_eq!(journal_mode.to_ascii_lowercase(), "wal");
        let busy_timeout_ms: i64 = connection
            .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
            .unwrap();
        assert_eq!(busy_timeout_ms, 5_000);
        let trusted_schema: i64 = connection
            .query_row("PRAGMA trusted_schema", [], |row| row.get(0))
            .unwrap();
        assert_eq!(trusted_schema, 0);

        for table in [
            "source_roots",
            "artifacts",
            "artifact_locators",
            "artifact_versions",
            "passages",
            "relationships",
            "index_jobs",
            "passages_fts_vocab",
            "passages_fts_instances",
        ] {
            let exists: bool = connection
                .query_row(
                    "SELECT EXISTS(
                        SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1
                    )",
                    [table],
                    |row| row.get(0),
                )
                .unwrap();
            assert!(exists, "migration did not create {table}");
        }
        for table in [
            "source_roots",
            "artifacts",
            "artifact_locators",
            "artifact_versions",
            "passages",
            "relationships",
            "index_jobs",
            "passages_fts_vocab",
            "passages_fts_instances",
        ] {
            let rows: i64 = connection
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(rows, 0, "empty migration populated {table}");
        }
    }

    #[test]
    fn migrates_v2_checkpoint_schema_without_overwriting_existing_marker() {
        let directory = tempdir().unwrap();
        let database = directory.path().join("v2.sqlite3");
        let connection = rusqlite::Connection::open(&database).unwrap();
        connection
            .execute_batch(include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../tests/fixtures/schema-v2.sql"
            )))
            .unwrap();
        drop(connection);

        let library = Library::open(&database).unwrap();
        let connection = library.lock().unwrap();
        let schema_version: String = connection
            .query_row(
                "SELECT value FROM schema_meta WHERE key = 'schema_version'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(schema_version, "3");
        let checkpoint_table: bool = connection
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'index_jobs'
                )",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(checkpoint_table);
    }

    #[test]
    fn deleting_an_artifact_cascades_canonical_rows_and_fts_state() {
        let directory = tempdir().unwrap();
        let removed = directory.path().join("removed.md");
        let retained = directory.path().join("retained.md");
        fs::write(&removed, "private marker to remove").unwrap();
        fs::write(&retained, "public marker to retain").unwrap();
        let library = Library::open_in_memory().unwrap();
        library.index_path(directory.path()).unwrap();

        let removed_hit = library
            .search(&SearchRequest {
                text: "\"private marker\"".into(),
                limit: 10,
            })
            .unwrap()
            .remove(0);
        let retained_hit = library
            .search(&SearchRequest {
                text: "\"public marker\"".into(),
                limit: 10,
            })
            .unwrap()
            .remove(0);

        {
            let connection = library.lock().unwrap();
            connection
                .execute(
                    "INSERT INTO relationships(
                        id, source_artifact_id, target_artifact_id, kind, method, created_at
                     ) VALUES (?1, ?2, ?3, 'related', 'test', '2026-01-01T00:00:00Z')",
                    rusqlite::params![
                        "relationship-under-test",
                        removed_hit.artifact_id,
                        retained_hit.artifact_id,
                    ],
                )
                .unwrap();
            let relationship_count: i64 = connection
                .query_row("SELECT COUNT(*) FROM relationships", [], |row| row.get(0))
                .unwrap();
            assert_eq!(relationship_count, 1);
        }

        {
            let connection = library.lock().unwrap();
            connection
                .execute(
                    "DELETE FROM artifacts WHERE id = ?1",
                    [&removed_hit.artifact_id],
                )
                .unwrap();
            let orphan_count: i64 = connection
                .query_row(
                    "SELECT COUNT(*) FROM artifact_locators WHERE artifact_id = ?1",
                    [&removed_hit.artifact_id],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(orphan_count, 0);
            let version_count: i64 = connection
                .query_row(
                    "SELECT COUNT(*) FROM artifact_versions WHERE artifact_id = ?1",
                    [&removed_hit.artifact_id],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(version_count, 0);
            let passage_count: i64 = connection
                .query_row(
                    "SELECT COUNT(*) FROM passages WHERE artifact_version_id = ?1",
                    [&removed_hit.version_id],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(passage_count, 0);
            let relationship_count: i64 = connection
                .query_row("SELECT COUNT(*) FROM relationships", [], |row| row.get(0))
                .unwrap();
            assert_eq!(relationship_count, 0);
            let foreign_key_errors: i64 = connection
                .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(foreign_key_errors, 0);
        }

        assert!(library
            .search(&SearchRequest {
                text: "\"private marker\"".into(),
                limit: 10,
            })
            .unwrap()
            .is_empty());
        assert_eq!(
            library
                .search(&SearchRequest {
                    text: "\"public marker\"".into(),
                    limit: 10,
                })
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn skips_unsupported_files_without_reading_them() {
        let directory = tempdir().unwrap();
        fs::write(directory.path().join("notes.md"), "recover the exact thing").unwrap();
        fs::write(directory.path().join("secret.bin"), [0, 159, 146, 150]).unwrap();
        let library = Library::open_in_memory().unwrap();
        let report = library.index_path(directory.path()).unwrap();
        assert_eq!(report.indexed, 1);
        assert!(report.skipped >= 1);
        assert!(report.failures.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_source_is_reported_and_recovers_after_permissions_restore() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempdir().unwrap();
        let source = directory.path().join("restricted.md");
        fs::write(&source, "permission recovery marker").unwrap();
        let canonical_source = source.canonicalize().unwrap();
        let library = Library::open_in_memory().unwrap();
        assert_eq!(library.index_path(directory.path()).unwrap().indexed, 1);

        let mut permissions = fs::metadata(&source).unwrap().permissions();
        permissions.set_mode(0o0);
        fs::set_permissions(&source, permissions).unwrap();
        assert!(
            fs::File::open(&source).is_err(),
            "the unreadable-input test requires a non-privileged target user"
        );

        let failed = library.index_path(directory.path()).unwrap();
        assert_eq!(failed.discovered, 1);
        assert_eq!(failed.indexed, 0);
        assert_eq!(failed.failures.len(), 1);
        assert_eq!(
            failed.failures[0].source,
            canonical_source.display().to_string()
        );
        assert!(failed.failures[0].reason.contains("I/O error"));
        assert!(library
            .search(&SearchRequest {
                text: "permission recovery".into(),
                limit: 10,
            })
            .unwrap()
            .is_empty());

        let mut restored = fs::metadata(&source).unwrap().permissions();
        restored.set_mode(0o600);
        fs::set_permissions(&source, restored).unwrap();
        let recovered = library.index_path(directory.path()).unwrap();
        assert_eq!(recovered.unchanged, 1);
        assert!(recovered.failures.is_empty());
        assert_eq!(
            library
                .search(&SearchRequest {
                    text: "permission recovery".into(),
                    limit: 10,
                })
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn complete_directory_rescan_hides_deleted_artifacts() {
        let directory = tempdir().unwrap();
        let deleted = directory.path().join("deleted.md");
        let retained = directory.path().join("retained.md");
        fs::write(&deleted, "a disappearing retrieval marker").unwrap();
        fs::write(&retained, "a retained retrieval marker").unwrap();
        let library = Library::open_in_memory().unwrap();

        library.index_path(directory.path()).unwrap();
        assert!(!library
            .search(&SearchRequest {
                text: "disappearing".into(),
                limit: 10,
            })
            .unwrap()
            .is_empty());

        fs::remove_file(&deleted).unwrap();
        let report = library.index_path(directory.path()).unwrap();
        assert_eq!(report.failures.len(), 0);
        assert!(library
            .search(&SearchRequest {
                text: "disappearing".into(),
                limit: 10,
            })
            .unwrap()
            .is_empty());
        assert!(!library
            .search(&SearchRequest {
                text: "retained".into(),
                limit: 10,
            })
            .unwrap()
            .is_empty());
    }

    #[test]
    fn failed_directory_reread_hides_previous_artifact() {
        let directory = tempdir().unwrap();
        let source = directory.path().join("too-large.md");
        fs::write(&source, "this source is deliberately too large").unwrap();
        let database = directory.path().join("loom.sqlite3");
        let library = Library::open(&database).unwrap();
        library.index_path(directory.path()).unwrap();
        assert!(!library
            .search(&SearchRequest {
                text: "deliberately".into(),
                limit: 10,
            })
            .unwrap()
            .is_empty());
        drop(library);

        let limits = LibraryLimits {
            max_file_bytes: 4,
            ..LibraryLimits::default()
        };
        let limited = Library::open_with_limits(&database, limits).unwrap();
        let report = limited.index_path(directory.path()).unwrap();
        assert_eq!(report.failures.len(), 1);
        assert!(limited
            .search(&SearchRequest {
                text: "deliberately".into(),
                limit: 10,
            })
            .unwrap()
            .is_empty());
    }

    #[test]
    fn fts_projection_uses_diacritic_and_token_boundary_semantics() {
        let directory = tempdir().unwrap();
        let source = directory.path().join("tokens.md");
        let source_text = "A café owner will concatenate records. A cat naps.";
        fs::write(&source, source_text).unwrap();
        let library = Library::open_in_memory().unwrap();
        library.index_path(&source).unwrap();

        let cafe = library
            .search(&SearchRequest {
                text: "cafe".into(),
                limit: 10,
            })
            .unwrap();
        assert_eq!(cafe.len(), 1);
        assert_eq!(highlighted_text(&cafe[0]), "café");
        assert_eq!(anchored_text(source_text, &cafe[0].anchor), "café");

        let cat = library
            .search(&SearchRequest {
                text: "cat".into(),
                limit: 10,
            })
            .unwrap();
        assert_eq!(cat.len(), 1);
        assert_eq!(highlighted_text(&cat[0]), "cat");
        assert_eq!(anchored_text(source_text, &cat[0].anchor), "cat");
    }

    #[test]
    fn identical_bytes_are_reprojected_when_the_extractor_version_changes() {
        let directory = tempdir().unwrap();
        let source = directory.path().join("extractor.md");
        fs::write(&source, "stable bytes need versioned projections").unwrap();
        let library = Library::open_in_memory().unwrap();
        assert_eq!(library.index_path(&source).unwrap().indexed, 1);
        let canonical_source = source.canonicalize().unwrap();
        let root_id: String = library
            .lock()
            .unwrap()
            .query_row("SELECT id FROM source_roots", [], |row| row.get(0))
            .unwrap();
        let document =
            ingest::read_stable(&canonical_source, &canonical_source, 8 * 1024 * 1024).unwrap();

        assert!(library
            .index_document_with_extractor(
                &root_id,
                &canonical_source,
                document,
                "loom.text",
                "0.2.0",
            )
            .unwrap());
        let stats = library.stats().unwrap();
        assert_eq!(stats.artifacts, 1);
        assert_eq!(stats.versions, 2);
        assert_eq!(stats.passages, 2);
        let observation = library.inspect_source(&source).unwrap();
        assert_eq!(observation.extractor_id, "loom.text");
        assert_eq!(observation.extractor_version, "0.2.0");
        assert_eq!(observation.passages.len(), 1);
        assert_eq!(
            highlighted_text(
                &library
                    .search(&SearchRequest {
                        text: "versioned".into(),
                        limit: 10,
                    })
                    .unwrap()[0]
            ),
            "versioned"
        );
    }

    #[test]
    fn refuses_pre_alpha_v1_schema_without_overwriting_it() {
        let directory = tempdir().unwrap();
        let database = directory.path().join("v1.sqlite3");
        let connection = rusqlite::Connection::open(&database).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE schema_meta(key TEXT PRIMARY KEY, value TEXT NOT NULL) STRICT;
                 INSERT INTO schema_meta(key, value) VALUES ('schema_version', '1');",
            )
            .unwrap();
        drop(connection);

        assert!(matches!(
            Library::open(&database),
            Err(LoomError::UnsupportedSchemaVersion(version)) if version == "1"
        ));
        let connection = rusqlite::Connection::open(&database).unwrap();
        let version: String = connection
            .query_row(
                "SELECT value FROM schema_meta WHERE key = 'schema_version'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(version, "1");
    }

    #[test]
    fn refuses_a_nonempty_database_without_a_schema_marker() {
        let directory = tempdir().unwrap();
        let database = directory.path().join("unversioned.sqlite3");
        let connection = rusqlite::Connection::open(&database).unwrap();
        connection
            .execute_batch("CREATE TABLE legacy_record(id TEXT PRIMARY KEY) STRICT;")
            .unwrap();
        drop(connection);

        assert!(matches!(
            Library::open(&database),
            Err(LoomError::UnsupportedSchemaVersion(version))
                if version.contains("schema_version table is missing")
        ));
        let connection = rusqlite::Connection::open(&database).unwrap();
        let schema_meta_exists: bool = connection
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM sqlite_master
                    WHERE type = 'table' AND name = 'schema_meta'
                 )",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(!schema_meta_exists);
    }

    #[test]
    fn refuses_an_unknown_schema_version_without_overwriting_it() {
        let directory = tempdir().unwrap();
        let database = directory.path().join("future.sqlite3");
        let connection = rusqlite::Connection::open(&database).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE schema_meta(key TEXT PRIMARY KEY, value TEXT NOT NULL) STRICT;
                 INSERT INTO schema_meta(key, value) VALUES ('schema_version', '99');",
            )
            .unwrap();
        drop(connection);

        assert!(matches!(
            Library::open(&database),
            Err(LoomError::UnsupportedSchemaVersion(version)) if version == "99"
        ));
        let connection = rusqlite::Connection::open(&database).unwrap();
        let version: String = connection
            .query_row(
                "SELECT value FROM schema_meta WHERE key = 'schema_version'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(version, "99");
    }

    fn highlighted_text(hit: &crate::SearchHit) -> String {
        hit.excerpt
            .segments
            .iter()
            .filter(|segment| segment.highlighted)
            .map(|segment| segment.text.as_str())
            .collect()
    }

    fn anchored_text(source: &str, anchor: &EvidenceAnchor) -> String {
        let EvidenceAnchor::Text {
            char_start,
            char_end,
            ..
        } = anchor;
        source
            .chars()
            .skip(*char_start as usize)
            .take((*char_end - *char_start) as usize)
            .collect()
    }
}
