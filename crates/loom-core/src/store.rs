use std::{
    collections::{BTreeMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard},
};

use chrono::{DateTime, Duration, Utc};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use uuid::Uuid;

use crate::{
    bookmarks::{self, BOOKMARK_EXTRACTOR_ID, BOOKMARK_EXTRACTOR_VERSION},
    domain::{
        ArtifactObservation, ArtifactVersionHistory, ArtifactVersionSummary, BookmarkEntry,
        BookmarkImportFailure, BookmarkImportReport, BookmarkImportSummary, BookmarkRecord,
        CompactedRelationship, DeletionReport, EvidenceAnchor, EvidenceExcerpt, EvidenceSegment,
        EvidenceView, FtsHealthReport, FtsRepairReport, IndexCancellationToken, IndexCheckpoint,
        IndexFailure, IndexReport, LibraryStats, ObservationReport, OcrPurgeReport, OcrStatus,
        PassageObservation, RankContributions, RelationshipCompaction,
        RelationshipCompactionReport, RelationshipEndpoint, RelationshipInput, RelationshipKind,
        RelationshipOrigin, RelationshipRecord, RelationshipView, ResolveEvidenceRequest,
        RetentionPolicy, RetentionReport, SearchHit, SearchRequest, SemanticCandidate,
        SemanticDropReport, SemanticIndexConfig, SemanticIndexManifest, SemanticIndexStatus,
        SemanticProviderMeasurement, SemanticRebuildReport, SourceRootInfo, SourceRootStatus,
        StorageEntry, StorageInspection,
    },
    error::{io_error, LoomError, Result},
    ingest::{
        self, PassageDraft, StableDocument, EXTRACTOR_ID, EXTRACTOR_VERSION, PDF_EXTRACTOR_ID,
        PDF_EXTRACTOR_VERSION,
    },
    observe::{self, ObservationEvent},
    ocr::{anchor_confidence_state, IMAGE_OCR_EXTRACTOR_ID},
    ranking::{fuse_hybrid_candidates, HybridRankConfig, HybridRankInput, HybridSearchHit},
    search::{collision_free_markers, compile_query, project_fts_evidence},
    semantic::{
        cosine_similarity, decode_vector, encode_vector, measure_providers, HashEmbeddingProvider,
    },
};

type BookmarkRecordProjection = (
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    String,
    String,
);

const SCHEMA_VERSION: i64 = 10;
const GRAPH_BOUNDS_SCHEMA_VERSION: i64 = 9;
const CONNECTOR_SCHEMA_VERSION: i64 = 8;
const BOOKMARK_SCHEMA_VERSION: i64 = 7;
/// Upper bound on relationships touching one artifact. With the source/target indexes this bounds
/// the rows any single traversal reads. Documented in docs/DATA_MODEL.md.
pub(crate) const MAX_RELATIONSHIPS_PER_ARTIFACT: i64 = 10_000;
const RELATIONSHIP_SCHEMA_VERSION: i64 = 6;
const PREVIOUS_SCHEMA_VERSION: i64 = 5;
const PREVIOUS_PREVIOUS_SCHEMA_VERSION: i64 = 4;
const V3_SCHEMA_VERSION: i64 = 3;
const LEGACY_SCHEMA_VERSION: i64 = 2;
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
    "bookmark_imports",
    "bookmark_records",
    "bookmark_import_items",
    "index_jobs",
];
const PRE_BOOKMARK_SCHEMA_TABLES: &[&str] = &[
    "source_roots",
    "artifacts",
    "artifact_locators",
    "artifact_versions",
    "passages",
    "relationships",
    "index_jobs",
];

/// Directories LOOM may create for disposable local derivatives. The names are deliberately
/// fixed so inspection and cleanup never recurse through an arbitrary application-data tree.
const DISPOSABLE_DIRECTORIES: &[(&str, &str)] = &[
    ("cache", "cache"),
    ("model-cache", "model_cache"),
    ("thumbnails", "thumbnails"),
    ("ocr-scratch", "ocr_scratch"),
    ("tmp-exports", "temporary_export"),
    ("logs", "log"),
];

type VersionProjection = (String, String, String, String, Option<i64>, String, String);
type SemanticMetaRow = (
    String,
    String,
    String,
    i64,
    String,
    String,
    String,
    String,
    i64,
    i64,
    i64,
);

/// Resource boundaries applied to every ingestion request.
#[derive(Debug, Clone, Copy)]
pub struct LibraryLimits {
    pub max_file_bytes: u64,
    pub max_files_per_request: usize,
    pub max_pdf_pages: usize,
    pub passage_target_chars: usize,
    pub passage_overlap_chars: usize,
}

impl Default for LibraryLimits {
    fn default() -> Self {
        Self {
            max_file_bytes: 8 * 1024 * 1024,
            max_files_per_request: 20_000,
            max_pdf_pages: 2_048,
            passage_target_chars: 1_000,
            passage_overlap_chars: 120,
        }
    }
}

/// A single-process modular-monolith library backed by canonical SQLite records.
pub struct Library {
    connection: Mutex<Connection>,
    limits: LibraryLimits,
    database_path: Option<PathBuf>,
    #[cfg(unix)]
    database_file_id: Option<(u64, u64)>,
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
        let path = path
            .canonicalize()
            .map_err(|source| io_error(path, source))?;
        Self::from_connection(connection, limits, Some(path))
    }

    /// Opens an isolated in-memory library for tests and evaluation.
    pub fn open_in_memory() -> Result<Self> {
        Self::from_connection(
            Connection::open_in_memory()?,
            LibraryLimits::default(),
            None,
        )
    }

    /// Checkpoints the WAL into the main database file and switches to a rollback journal, so the
    /// database is a single self-contained file that can be moved once this connection closes.
    pub(crate) fn checkpoint_for_handoff(&self) -> Result<()> {
        let connection = self.lock()?;
        let (busy, _, _): (i64, i64, i64) =
            connection.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })?;
        if busy != 0 {
            return Err(LoomError::PortableExport(
                "the restored database could not be checkpointed".into(),
            ));
        }
        let mode: String =
            connection.query_row("PRAGMA journal_mode = DELETE", [], |row| row.get(0))?;
        if !mode.eq_ignore_ascii_case("delete") {
            return Err(LoomError::PortableExport(
                "the restored database could not leave WAL mode".into(),
            ));
        }
        Ok(())
    }

    fn from_connection(
        mut connection: Connection,
        limits: LibraryLimits,
        database_path: Option<PathBuf>,
    ) -> Result<Self> {
        configure(&connection)?;
        migrate(&mut connection)?;
        ensure_semantic_schema(&connection)?;
        crate::jobs::ensure_schema(&connection)?;
        #[cfg(unix)]
        let database_file_id = database_path
            .as_deref()
            .map(database_file_identity)
            .transpose()?;
        Ok(Self {
            connection: Mutex::new(connection),
            limits,
            database_path,
            #[cfg(unix)]
            database_file_id,
        })
    }

    /// Indexes one explicitly selected regular file or directory.
    pub fn index_path(&self, selected_path: impl AsRef<Path>) -> Result<IndexReport> {
        let cancellation = IndexCancellationToken::new();
        self.index_path_with_options(selected_path, &cancellation, None, None, None, None)
    }

    /// Imports a Netscape HTML bookmark export as metadata-only, source-faithful records.
    ///
    /// The export is read once from the explicitly selected local file. URLs become locators and
    /// searchable metadata passages; they are never fetched as part of this operation.
    pub fn import_bookmarks(
        &self,
        selected_path: impl AsRef<Path>,
    ) -> Result<BookmarkImportReport> {
        self.import_bookmarks_with_authorization(selected_path, None)
    }

    fn import_bookmarks_with_authorization(
        &self,
        selected_path: impl AsRef<Path>,
        approved_authorization: Option<SourceAuthorization>,
    ) -> Result<BookmarkImportReport> {
        let requested_path = selected_path.as_ref();
        let metadata = fs::symlink_metadata(requested_path)
            .map_err(|source| io_error(requested_path, source))?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(LoomError::InvalidPath(format!(
                "bookmark export must be a regular file: {}",
                requested_path.display()
            )));
        }
        let path = requested_path
            .canonicalize()
            .map_err(|source| io_error(requested_path, source))?;
        let source_uri = utf8_path(&path)?;
        if let Some(authorization) = approved_authorization.as_ref() {
            authorization.verify_locator(&*self.lock()?, &source_uri)?;
        }
        let bytes = ingest::read_stable_selected_file(&path, self.limits.max_file_bytes)?;
        let content_hash = format!("blake3:{}", blake3::hash(&bytes).to_hex());
        let text = String::from_utf8(bytes).map_err(|_| {
            LoomError::InvalidPath(format!("bookmark export is not UTF-8: {}", path.display()))
        })?;
        let detailed = bookmarks::parse_bookmark_export_detailed(&text)?;
        if detailed.export.bookmarks.is_empty() && detailed.failures.is_empty() {
            return Err(LoomError::InvalidPath(
                "bookmark export contains no usable bookmarks".into(),
            ));
        }
        let export = detailed.export.clone();
        let authorization = {
            let mut connection = self.lock()?;
            match approved_authorization {
                Some(authorization) => authorization,
                None => ensure_source_root(&mut connection, &source_uri, false)?,
            }
        };
        let root_id = &authorization.root_id;
        let now = Utc::now().to_rfc3339();
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        authorization.verify(&transaction)?;
        validate_bookmark_scope_consistency(&transaction)?;
        let existing: Option<(String, i64)> = transaction
            .query_row(
                "SELECT id, (SELECT COUNT(*) FROM bookmark_import_items WHERE import_id = i.id)
                 FROM bookmark_imports i
                 WHERE source_locator = ?1 AND format = ?2 AND content_hash = ?3
                     AND source_root_id = ?4",
                params![source_uri, export.format, content_hash, root_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        if let Some((import_id, unchanged)) = existing {
            // Replaying identical bytes keeps the original import and artifact identity. An
            // explicit re-selection of a revoked export makes its records available again.
            transaction.execute(
                "UPDATE bookmark_imports
                 SET status = CASE WHEN EXISTS(
                        SELECT 1 FROM bookmark_import_failures
                        WHERE import_id = ?1 AND state = 'pending'
                     ) THEN 'partial' ELSE 'complete' END
                 WHERE id = ?1 AND status = 'revoked' AND source_root_id = ?2",
                params![import_id, root_id],
            )?;
            transaction.execute(
                "UPDATE artifacts SET state = 'active', last_seen_at = ?2
                 WHERE state = 'missing' AND source_root_id = ?3 AND id IN (
                    SELECT r.artifact_id FROM bookmark_records r
                    JOIN bookmark_import_items item ON item.bookmark_id = r.id
                    WHERE item.import_id = ?1
                 )",
                params![import_id, now, root_id],
            )?;
            // Retry entries that only failed because another export owned their URL; once that
            // export is purged, re-selecting this one imports them into the same import.
            let mut report = BookmarkImportReport::default();
            let blocked: Vec<u32> = {
                let mut statement = transaction.prepare(
                    "SELECT ordinal FROM bookmark_import_failures
                     WHERE import_id = ?1 AND state = 'pending' AND code = 'owned_by_other_export'
                     ORDER BY ordinal",
                )?;
                let rows = statement
                    .query_map([&import_id], |row| row.get(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                rows
            };
            for record_ordinal in blocked {
                let Some(index) = detailed
                    .entry_positions
                    .iter()
                    .position(|(ordinal, _)| *ordinal == record_ordinal)
                else {
                    continue;
                };
                let entry = &export.bookmarks[index];
                if bookmark_owned_by_other_export(&transaction, root_id, entry)? {
                    continue;
                }
                import_bookmark_entry(
                    &transaction,
                    root_id,
                    &source_uri,
                    &import_id,
                    entry,
                    index,
                    &now,
                    &mut report,
                )?;
                transaction.execute(
                    "UPDATE bookmark_import_failures
                     SET state = 'resolved', resolved_by_import_id = ?1
                     WHERE import_id = ?1 AND ordinal = ?2",
                    params![import_id, record_ordinal],
                )?;
            }
            transaction.execute(
                "UPDATE bookmark_imports SET status = 'complete'
                 WHERE id = ?1 AND status = 'partial' AND NOT EXISTS(
                    SELECT 1 FROM bookmark_import_failures WHERE import_id = ?1 AND state = 'pending'
                 )",
                [&import_id],
            )?;
            let failures = pending_import_failures(&transaction, &import_id)?;
            transaction.commit()?;
            return Ok(BookmarkImportReport {
                import_id,
                source_uri,
                format: export.format,
                content_hash,
                discovered: export.bookmarks.len() as u64,
                unchanged: unchanged.max(0) as u64,
                failed: failures.len() as u64,
                failures,
                remote_fetches: 0,
                ..report
            });
        }

        let import_id = Uuid::new_v4().to_string();
        let status = if detailed.failures.is_empty() && !detailed.truncated {
            "complete"
        } else {
            "partial"
        };
        transaction.execute(
            "INSERT INTO bookmark_imports(
                id, source_root_id, source_locator, format, content_hash, imported_at,
                source_application, export_version, permissions_json, skipped_fields_json, status
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                import_id,
                root_id,
                source_uri,
                export.format,
                content_hash,
                now,
                detailed.source_application,
                detailed.export_version,
                serde_json::to_string(bookmarks::BOOKMARK_IMPORT_PERMISSIONS)?,
                serde_json::to_string(&detailed.skipped_fields)?,
                status
            ],
        )?;
        for failure in &detailed.failures {
            transaction.execute(
                "INSERT INTO bookmark_import_failures(
                    import_id, ordinal, byte_offset, code, detail, state, created_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, 'pending', ?6)",
                params![
                    import_id,
                    failure.ordinal,
                    failure.byte_offset as i64,
                    failure.code,
                    failure.detail,
                    now
                ],
            )?;
        }
        // A newer import of the same export is the retry for every earlier pending failure;
        // anything still broken is recorded again as a pending failure of this import.
        transaction.execute(
            "UPDATE bookmark_import_failures
             SET state = 'resolved', resolved_by_import_id = ?1
             WHERE state = 'pending' AND import_id <> ?1 AND import_id IN (
                SELECT id FROM bookmark_imports WHERE source_locator = ?2
             )",
            params![import_id, source_uri],
        )?;
        let mut report = BookmarkImportReport {
            import_id: import_id.clone(),
            source_uri: source_uri.clone(),
            format: export.format.clone(),
            content_hash: content_hash.clone(),
            discovered: export.bookmarks.len() as u64,
            remote_fetches: 0,
            ..BookmarkImportReport::default()
        };
        for (ordinal, entry) in export.bookmarks.iter().enumerate() {
            let (record_ordinal, byte_offset) = detailed.entry_positions[ordinal];
            if bookmark_owned_by_other_export(&transaction, root_id, entry)? {
                record_owned_elsewhere(
                    &transaction,
                    &import_id,
                    record_ordinal,
                    byte_offset,
                    &now,
                )?;
                continue;
            }
            import_bookmark_entry(
                &transaction,
                root_id,
                &source_uri,
                &import_id,
                entry,
                ordinal,
                &now,
                &mut report,
            )?;
        }
        transaction.execute(
            "UPDATE bookmark_imports SET status = 'partial'
             WHERE id = ?1 AND status = 'complete' AND EXISTS(
                SELECT 1 FROM bookmark_import_failures WHERE import_id = ?1 AND state = 'pending'
             )",
            [&import_id],
        )?;
        report.failures = pending_import_failures(&transaction, &import_id)?;
        report.failed = report.failures.len() as u64;
        transaction.commit()?;
        Ok(report)
    }

    /// Lists bookmark imports newest first, with connector metadata and every per-record failure.
    pub fn list_bookmark_imports(&self, limit: u32) -> Result<Vec<BookmarkImportSummary>> {
        let limit = limit.clamp(1, 1_000);
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT id, source_locator, format, content_hash, imported_at, source_application,
                    export_version, permissions_json, skipped_fields_json, status,
                    (SELECT COUNT(*) FROM bookmark_import_items WHERE import_id = i.id)
             FROM bookmark_imports i
             ORDER BY imported_at DESC, id
             LIMIT ?1",
        )?;
        let rows = statement
            .query_map([limit], |row| {
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
                    row.get::<_, String>(9)?,
                    row.get::<_, i64>(10)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut failures = connection.prepare(
            "SELECT ordinal, byte_offset, code, detail, state, resolved_by_import_id, created_at
             FROM bookmark_import_failures WHERE import_id = ?1 ORDER BY ordinal",
        )?;
        let mut summaries = Vec::with_capacity(rows.len());
        for (
            id,
            source_uri,
            format,
            content_hash,
            imported_at,
            source_application,
            export_version,
            permissions,
            skipped_fields,
            status,
            items,
        ) in rows
        {
            let record_failures = failures
                .query_map([&id], |row| {
                    Ok(BookmarkImportFailure {
                        ordinal: row.get::<_, i64>(0)?.max(0) as u32,
                        byte_offset: row.get::<_, i64>(1)?.max(0) as u64,
                        code: row.get(2)?,
                        detail: row.get(3)?,
                        state: row.get(4)?,
                        resolved_by_import_id: row.get(5)?,
                        created_at: row.get(6)?,
                    })
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            summaries.push(BookmarkImportSummary {
                import_id: id,
                source_uri,
                format,
                content_hash,
                imported_at,
                source_application,
                export_version,
                permissions: serde_json::from_str(&permissions)?,
                skipped_fields: serde_json::from_str(&skipped_fields)?,
                status,
                items: items.max(0) as u64,
                failures: record_failures,
            });
        }
        Ok(summaries)
    }

    /// Re-reads the original export of a recorded import. A revoked export must be re-selected
    /// explicitly instead; retry never re-grants access that was withdrawn.
    pub fn retry_bookmark_import(&self, import_id: &str) -> Result<BookmarkImportReport> {
        let (locator, authorization) = {
            let connection = self.lock()?;
            connection
                .query_row(
                    "SELECT i.source_locator, r.id, r.scope_generation,
                        (SELECT value FROM schema_meta WHERE key = 'authorization_incarnation'), r.kind
                     FROM bookmark_imports i JOIN source_roots r ON r.id = i.source_root_id
                     WHERE i.id = ?1 AND i.status <> 'revoked' AND r.enabled = 1
                        AND r.locator = i.source_locator AND r.kind = 'file'",
                    [import_id],
                    |row| Ok((row.get::<_, String>(0)?, SourceAuthorization {
                        root_id: row.get(1)?, generation: row.get(2)?, incarnation: row.get(3)?, kind: row.get(4)?, ocr_policy: None,
                    })),
                )
                .optional()?
                .ok_or_else(|| {
                    LoomError::InvalidPath("bookmark import is unknown or its source is not enabled; explicitly re-select a revoked export".into())
                })?
        };
        self.import_bookmarks_with_authorization(locator, Some(authorization))
    }

    /// Lists bounded current bookmark records with their original export provenance.
    pub fn list_bookmarks(&self, limit: u32) -> Result<Vec<BookmarkRecord>> {
        let limit = limit.clamp(1, 1_000);
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT r.id, r.artifact_id, r.first_import_id, i.source_locator,
                    r.folder_path, r.title, r.url, r.added_at, r.modified_at, r.entry_hash,
                    (SELECT COUNT(*) FROM bookmark_import_items bi WHERE bi.bookmark_id = r.id)
             FROM bookmark_records r
             JOIN bookmark_imports i ON i.id = r.first_import_id
             ORDER BY r.folder_path, r.title, r.id
             LIMIT ?1",
        )?;
        let rows = statement
            .query_map([limit], |row| {
                Ok(BookmarkRecord {
                    id: row.get(0)?,
                    artifact_id: row.get(1)?,
                    import_id: row.get(2)?,
                    source_uri: row.get(3)?,
                    folder_path: row.get(4)?,
                    title: row.get(5)?,
                    url: row.get(6)?,
                    added_at: row.get(7)?,
                    modified_at: row.get(8)?,
                    entry_hash: row.get(9)?,
                    import_count: row.get::<_, i64>(10)?.max(0) as u64,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into);
        rows
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
        self.index_path_with_options(selected_path, cancellation, None, None, None, None)
    }

    /// Indexes one atomically written intentional screenshot. Capture context is attached to
    /// extractor input before OCR; provenance and evidence commit together under the current policy.
    pub fn index_captured_image(
        &self,
        selected_path: impl AsRef<Path>,
        context: &crate::CaptureContext,
    ) -> Result<IndexReport> {
        let path = selected_path.as_ref();
        if !path.is_file()
            || !matches!(ingest::supported_media_type(path), Some(media) if media.starts_with("image/"))
        {
            return Err(LoomError::UnsupportedSource(path.display().to_string()));
        }
        if !OcrPolicy::load(&*self.lock()?)?.enabled {
            return Err(LoomError::OcrDisabled);
        }
        let cancellation = IndexCancellationToken::new();
        let metadata = serde_json::to_value(context)?;
        self.index_path_with_options(path, &cancellation, None, None, Some(metadata), None)
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
        let authorization = self.approved_root_authorization(&selected_uri)?;
        let plan = observe::coalesce_events(&selected_path, events, max_events)?;
        if plan.events_received == 0 {
            return Ok(ObservationReport::default());
        }
        let index = self.index_path_with_options(
            &selected_path,
            &IndexCancellationToken::new(),
            None,
            None,
            None,
            Some(authorization),
        )?;
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
        for (root, kind, authorization) in roots {
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
            match self.index_path_with_options(
                &root,
                &IndexCancellationToken::new(),
                None,
                None,
                None,
                Some(authorization),
            ) {
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
            "UPDATE source_roots SET enabled = 0, scope_generation = scope_generation + 1,
                last_seen_at = ?1 WHERE locator = ?2",
            params![Utc::now().to_rfc3339(), locator],
        )?;
        transaction.execute(
            "UPDATE artifacts SET state = 'missing', last_seen_at = ?1
             WHERE source_root_id = (SELECT id FROM source_roots WHERE locator = ?2)
               AND state = 'active'",
            params![Utc::now().to_rfc3339(), locator],
        )?;
        transaction.execute(
            "UPDATE bookmark_imports SET status = 'revoked'
             WHERE source_root_id = (SELECT id FROM source_roots WHERE locator = ?1)",
            [locator],
        )?;
        transaction.execute(
            "UPDATE index_jobs SET state = 'failed', last_error = 'source authorization revoked',
                updated_at = ?1 WHERE source_root_id = (
                    SELECT id FROM source_roots WHERE locator = ?2
                ) AND state IN ('running', 'interrupted')",
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

    /// Adds one typed, evidence-bearing relationship and returns the durable record.
    ///
    /// Relationship identity is the source/target/kind/origin/method tuple. Repeating the same
    /// observation is idempotent and never overwrites a prior evidence record.
    pub fn add_relationship(&self, input: &RelationshipInput) -> Result<RelationshipRecord> {
        let metadata_json = validate_relationship_input(input)?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        for artifact_id in [&input.source_artifact_id, &input.target_artifact_id] {
            let exists: bool = transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM artifacts WHERE id = ?1)",
                [artifact_id],
                |row| row.get(0),
            )?;
            if !exists {
                return Err(LoomError::ArtifactNotFound(artifact_id.to_string()));
            }
        }
        if let Some(passage_id) = input.evidence_passage_id.as_deref() {
            let belongs: bool = transaction.query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM passages p
                    JOIN artifact_versions v ON v.id = p.artifact_version_id
                    WHERE p.id = ?1
                      AND v.artifact_id IN (?2, ?3)
                )",
                params![
                    passage_id,
                    input.source_artifact_id,
                    input.target_artifact_id
                ],
                |row| row.get(0),
            )?;
            if !belongs {
                return Err(LoomError::InvalidPath(format!(
                    "evidence passage is not attached to either relationship endpoint: {passage_id}"
                )));
            }
        }
        let existing: Option<String> = transaction
            .query_row(
                "SELECT id FROM relationships
                 WHERE source_artifact_id = ?1 AND target_artifact_id = ?2
                   AND kind = ?3 AND origin = ?4 AND method = ?5
                 ORDER BY created_at, id LIMIT 1",
                params![
                    input.source_artifact_id,
                    input.target_artifact_id,
                    input.kind.as_str(),
                    input.origin.as_str(),
                    input.method.trim()
                ],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(id) = existing {
            transaction.commit()?;
            return relationship_by_id(&connection, &id)?
                .ok_or_else(|| LoomError::ArtifactStale(id));
        }
        for artifact_id in [&input.source_artifact_id, &input.target_artifact_id] {
            let degree: i64 = transaction.query_row(
                "SELECT (SELECT COUNT(*) FROM relationships WHERE source_artifact_id = ?1)
                      + (SELECT COUNT(*) FROM relationships WHERE target_artifact_id = ?1)",
                [artifact_id],
                |row| row.get(0),
            )?;
            if degree >= MAX_RELATIONSHIPS_PER_ARTIFACT {
                return Err(LoomError::InvalidPath(format!(
                    "artifact {artifact_id} already has {degree} relationships; \
                     compact redundant edges before adding more"
                )));
            }
        }
        let id = Uuid::new_v4().to_string();
        let created_at = Utc::now().to_rfc3339();
        transaction.execute(
            "INSERT INTO relationships(
                id, source_artifact_id, target_artifact_id, kind, evidence_passage_id,
                confidence, method, relationship_schema_version, origin, metadata_json, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1, ?8, ?9, ?10)",
            params![
                id,
                input.source_artifact_id,
                input.target_artifact_id,
                input.kind.as_str(),
                input.evidence_passage_id,
                input.confidence,
                input.method.trim(),
                input.origin.as_str(),
                metadata_json,
                created_at,
            ],
        )?;
        transaction.commit()?;
        relationship_by_id(&connection, &id)?.ok_or_else(|| LoomError::ArtifactStale(id))
    }

    /// Lists bounded source-backed relationships touching an artifact, including active endpoint
    /// version/hash projections for UI traversal.
    pub fn list_relationships(
        &self,
        artifact_id: &str,
        limit: u32,
    ) -> Result<Vec<RelationshipView>> {
        validate_relationship_id(artifact_id, "artifact")?;
        let limit = limit.clamp(1, 100);
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT id, source_artifact_id, target_artifact_id, kind, evidence_passage_id,
                    confidence, method, relationship_schema_version, origin, metadata_json,
                    created_at
             FROM relationships
             WHERE source_artifact_id = ?1 OR target_artifact_id = ?1
             ORDER BY created_at, id
             LIMIT ?2",
        )?;
        let records = statement
            .query_map(params![artifact_id, limit], relationship_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        records
            .into_iter()
            .map(|relationship| {
                let source = relationship_endpoint(&connection, &relationship.source_artifact_id)?;
                let target = relationship_endpoint(&connection, &relationship.target_artifact_id)?;
                Ok(RelationshipView {
                    relationship,
                    source,
                    target,
                })
            })
            .collect()
    }

    /// Inspects version metadata without opening or substituting source bytes.
    ///
    /// Returns the current version first, then newest historical versions, with a hard limit of
    /// 100 rows and an explicit truncation flag. Historical and revoked/non-file versions never
    /// receive a viewer reference; current references still require `resolve_verified_evidence`.
    pub fn artifact_version_history(
        &self,
        artifact_id: &str,
        limit: u32,
    ) -> Result<ArtifactVersionHistory> {
        validate_relationship_id(artifact_id, "artifact")?;
        let limit = limit.clamp(1, 100) as usize;
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        let artifact = relationship_endpoint(&transaction, artifact_id)?;
        let can_view: bool = transaction.query_row(
            "SELECT EXISTS(
                SELECT 1 FROM artifacts a
                JOIN source_roots r ON r.id = a.source_root_id AND r.enabled = 1
                JOIN artifact_locators l ON l.artifact_id = a.id AND l.active = 1
                WHERE a.id = ?1 AND a.state = 'active' AND l.kind = 'file'
            )",
            [artifact_id],
            |row| row.get(0),
        )?;
        let mut versions = {
            let mut statement = transaction.prepare(
                "SELECT v.id, v.content_hash, v.byte_size, v.extractor_id, v.extractor_version,
                        v.created_at,
                        (SELECT p.id FROM passages p WHERE p.artifact_version_id = v.id
                         ORDER BY p.ordinal LIMIT 1)
                 FROM artifact_versions v WHERE v.artifact_id = ?1
                 ORDER BY (v.id = ?2) DESC, v.created_at DESC, v.id DESC LIMIT ?3",
            )?;
            let result = statement
                .query_map(
                    params![
                        artifact_id,
                        artifact.version_id.as_deref().unwrap_or(""),
                        (limit + 1) as i64
                    ],
                    |row| {
                        let version_id: String = row.get(0)?;
                        let content_hash: String = row.get(1)?;
                        let passage_id: Option<String> = row.get(6)?;
                        let is_current = artifact.version_id.as_deref() == Some(&version_id);
                        let evidence =
                            passage_id
                                .filter(|_| is_current && can_view)
                                .map(|passage_id| ResolveEvidenceRequest {
                                    artifact_id: artifact_id.into(),
                                    version_id: version_id.clone(),
                                    passage_id,
                                    content_hash: content_hash.clone(),
                                });
                        Ok(ArtifactVersionSummary {
                            version_id,
                            content_hash,
                            byte_size: u64::try_from(row.get::<_, i64>(2)?).map_err(|error| {
                                rusqlite::Error::FromSqlConversionFailure(
                                    2,
                                    rusqlite::types::Type::Integer,
                                    Box::new(error),
                                )
                            })?,
                            extractor_id: row.get(3)?,
                            extractor_version: row.get(4)?,
                            created_at: row.get(5)?,
                            is_current,
                            evidence,
                        })
                    },
                )?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            result
        };
        let truncated = versions.len() > limit;
        versions.truncate(limit);
        transaction.commit()?;
        Ok(ArtifactVersionHistory {
            artifact,
            versions,
            truncated,
        })
    }

    /// Removes redundant inferred relationships and records a digest-linked summary of them.
    ///
    /// An inferred edge is redundant when another edge links the same source, target, and kind with
    /// more authority: a user-confirmed or observed edge, or an inferred edge with higher
    /// confidence (ties go to the earlier edge, then the smaller ID). Observed and user-confirmed
    /// edges are never removed, so every source-to-target lineage a user can see is preserved by
    /// the kept edge. At most `max_removals` edges are removed per call.
    pub fn compact_relationships(&self, max_removals: u32) -> Result<RelationshipCompactionReport> {
        let max_removals = max_removals.clamp(1, 10_000);
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        let rank = "CASE {0}.origin WHEN 'user_confirmed' THEN 3 WHEN 'observed' THEN 2 ELSE 1 END";
        let dominated = format!(
            "SELECT e.id, (
                SELECT f.id FROM relationships f
                WHERE f.source_artifact_id = e.source_artifact_id
                  AND f.target_artifact_id = e.target_artifact_id
                  AND f.kind = e.kind AND f.id <> e.id
                  AND ({rank_f} > {rank_e}
                    OR ({rank_f} = {rank_e} AND (
                        COALESCE(f.confidence, 0) > COALESCE(e.confidence, 0)
                        OR (COALESCE(f.confidence, 0) = COALESCE(e.confidence, 0)
                            AND (f.created_at < e.created_at
                                 OR (f.created_at = e.created_at AND f.id < e.id))))))
                ORDER BY {rank_f} DESC, COALESCE(f.confidence, 0) DESC, f.created_at, f.id
                LIMIT 1
             ) AS kept_id
             FROM relationships e
             WHERE e.origin = 'inferred'",
            rank_f = rank.replace("{0}", "f"),
            rank_e = rank.replace("{0}", "e"),
        );
        let candidates: Vec<(String, String)> = {
            let mut statement = transaction.prepare(&format!(
                "SELECT id, kept_id FROM ({dominated}) WHERE kept_id IS NOT NULL ORDER BY id"
            ))?;
            let rows = statement
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows
        };
        // A kept edge may itself be redundant; each summary names the edge at the top of the chain.
        let removed_ids = candidates
            .iter()
            .take(max_removals as usize)
            .map(|(id, _)| id.clone())
            .collect::<std::collections::BTreeSet<_>>();
        let remaining = candidates.len().saturating_sub(removed_ids.len()) as u64;
        if removed_ids.is_empty() {
            transaction.commit()?;
            return Ok(RelationshipCompactionReport::default());
        }
        let kept_by = candidates.iter().cloned().collect::<BTreeMap<_, _>>();
        let mut removed = Vec::with_capacity(removed_ids.len());
        for id in &removed_ids {
            // Dominance is a strict order within an edge group, so the chain always ends.
            let mut kept = kept_by[id].clone();
            while let Some(next) = kept_by.get(&kept) {
                kept = next.clone();
            }
            let record = relationship_by_id(&transaction, id)?
                .ok_or_else(|| LoomError::ArtifactStale(id.clone()))?;
            removed.push(CompactedRelationship {
                removed: record,
                kept_relationship_id: kept,
            });
        }
        let summary_json = serde_json::to_string(&removed)?;
        let removed_digest = format!("blake3:{}", blake3::hash(summary_json.as_bytes()).to_hex());
        let compaction_id = Uuid::new_v4().to_string();
        let compacted_at = Utc::now().to_rfc3339();
        transaction.execute(
            "INSERT INTO relationship_compactions(
                id, compacted_at, removed_count, removed_digest, summary_json
             ) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                compaction_id,
                compacted_at,
                removed.len() as i64,
                removed_digest,
                summary_json
            ],
        )?;
        for id in &removed_ids {
            transaction.execute("DELETE FROM relationships WHERE id = ?1", [id])?;
        }
        transaction.commit()?;
        Ok(RelationshipCompactionReport {
            compaction_id: Some(compaction_id),
            removed: removed.len() as u64,
            remaining,
            removed_digest: Some(removed_digest),
        })
    }

    /// Lists relationship compactions newest first, with every removed edge and the edge kept in
    /// its place. Each summary's digest can be recomputed from its removed list.
    pub fn list_relationship_compactions(&self, limit: u32) -> Result<Vec<RelationshipCompaction>> {
        let limit = limit.clamp(1, 100);
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT id, compacted_at, removed_digest, summary_json
             FROM relationship_compactions ORDER BY compacted_at DESC, id LIMIT ?1",
        )?;
        let rows = statement
            .query_map([limit], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter()
            .map(|(id, compacted_at, removed_digest, summary_json)| {
                Ok(RelationshipCompaction {
                    id,
                    compacted_at,
                    removed_digest,
                    removed: serde_json::from_str(&summary_json)?,
                })
            })
            .collect()
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
        self.index_path_with_options(
            selected_path,
            &cancellation,
            interrupt_after_units,
            None,
            None,
            None,
        )
    }

    /// Deterministic cancellation hook used by integration fixtures.
    #[doc(hidden)]
    pub fn index_path_with_cancellation_after(
        &self,
        selected_path: impl AsRef<Path>,
        cancel_after_units: usize,
    ) -> Result<IndexReport> {
        let cancellation = IndexCancellationToken::new();
        self.index_path_with_options(
            selected_path,
            &cancellation,
            None,
            Some(cancel_after_units),
            None,
            None,
        )
    }

    fn index_path_with_options(
        &self,
        selected_path: impl AsRef<Path>,
        cancellation: &IndexCancellationToken,
        interrupt_after_units: Option<usize>,
        cancel_after_units: Option<usize>,
        capture_metadata: Option<serde_json::Value>,
        approved_authorization: Option<SourceAuthorization>,
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
        let mut authorization = {
            let mut connection = self.lock()?;
            match approved_authorization {
                Some(authorization) => {
                    authorization.verify_locator(&connection, &selected_uri)?;
                    authorization
                }
                None => ensure_source_root(&mut connection, &selected_uri, selected_path.is_dir())?,
            }
        };
        let discovered = ingest::discover(&selected_path, self.limits.max_files_per_request)?;
        if authorization.ocr_policy.is_none()
            && discovered.iter().any(|path| {
                ingest::supported_media_type(path).is_some_and(|media| media.starts_with("image/"))
            })
        {
            authorization.ocr_policy = Some(OcrPolicy::load(&*self.lock()?)?);
        }
        if capture_metadata.is_some()
            && !authorization
                .ocr_policy
                .as_ref()
                .is_some_and(|policy| policy.enabled)
        {
            return Err(LoomError::OcrDisabled);
        }
        let discovery_fingerprint = discovery_fingerprint(
            &discovered,
            authorization.ocr_policy.as_ref(),
            self.limits.max_files_per_request,
        );
        let job = self.start_index_job(
            &authorization,
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
            authorization.verify(&*self.lock()?)?;
            if cancel_after_units.is_some_and(|limit| units_processed_this_run >= limit) {
                cancellation.cancel();
            }
            if cancellation.is_cancelled() {
                report.cancelled = report
                    .discovered
                    .saturating_sub(job.next_unit.saturating_add(report.attempted));
                self.interrupt_index_job(&authorization, &job.job_id, "cancelled by request")?;
                return Ok(report);
            }
            if interrupt_after_units.is_some_and(|limit| units_processed_this_run >= limit) {
                let message =
                    format!("fault injection after {units_processed_this_run} completed unit(s)");
                self.interrupt_index_job(&authorization, &job.job_id, &message)?;
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
                    self.advance_index_job(&authorization, &job.job_id, unit as u64 + 1)?;
                    units_processed_this_run += 1;
                    continue;
                }
            };
            if ingest::supported_media_type(&path).is_none() {
                report.skipped += 1;
                if let Err(error) = self.mark_locator_missing_and_advance(
                    &authorization,
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
            match ingest::read_stable_with_limits_and_ocr(
                &path,
                &selected_path,
                self.limits.max_file_bytes,
                self.limits.max_pdf_pages,
                authorization
                    .ocr_policy
                    .as_ref()
                    .is_some_and(|policy| policy.enabled),
                capture_metadata.as_ref(),
            ) {
                Ok(document) => {
                    let bytes = document.byte_size;
                    report.bytes_read += bytes;
                    match self.index_document_with_checkpoint(
                        &authorization,
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
                            if matches!(
                                error,
                                LoomError::SourceRevoked(_)
                                    | LoomError::OcrPolicyChanged
                                    | LoomError::OcrUnavailable(_)
                            ) {
                                return Err(error);
                            }
                            let reason = match self.mark_locator_missing_and_advance(
                                &authorization,
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
                    if matches!(error, LoomError::OcrDisabled) {
                        report.skipped += 1;
                        if let Err(checkpoint_error) =
                            self.advance_index_job(&authorization, &job.job_id, unit as u64 + 1)
                        {
                            report.failed += 1;
                            report.failures.push(IndexFailure {
                                source: path.display().to_string(),
                                reason: format!(
                                    "OCR disabled but checkpoint could not advance: {checkpoint_error}"
                                ),
                            });
                        }
                        units_processed_this_run += 1;
                        continue;
                    }
                    let reason = match self.mark_locator_missing_and_advance(
                        &authorization,
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
            self.interrupt_index_job(&authorization, &job.job_id, "cancelled by request")?;
            return Ok(report);
        }
        if selected_path.is_dir() {
            if let Err(error) = self.reconcile_directory(&authorization, &seen) {
                // Revoked workers must not update even diagnostic state after losing consent.
                if !matches!(
                    error,
                    LoomError::SourceRevoked(_)
                        | LoomError::OcrPolicyChanged
                        | LoomError::OcrUnavailable(_)
                ) {
                    self.fail_index_job(&authorization, &job.job_id, &error.to_string())?;
                }
                return Err(error);
            }
        }
        self.complete_index_job(
            &authorization,
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
        authorization: &SourceAuthorization,
        path: &Path,
        document: StableDocument,
        job_id: &str,
        next_unit: u64,
    ) -> Result<bool> {
        let (extractor_id, extractor_version) = match document.media_type {
            "application/pdf" => (PDF_EXTRACTOR_ID, PDF_EXTRACTOR_VERSION),
            media_type if media_type.starts_with("image/") => (
                IMAGE_OCR_EXTRACTOR_ID,
                crate::ocr::IMAGE_OCR_EXTRACTOR_VERSION,
            ),
            _ => (EXTRACTOR_ID, EXTRACTOR_VERSION),
        };
        self.index_document_with_extractor_and_checkpoint(
            authorization,
            path,
            document,
            extractor_id,
            extractor_version,
            Some((job_id, next_unit)),
        )
    }

    #[cfg(test)]
    fn index_document_with_extractor(
        &self,
        authorization: &SourceAuthorization,
        path: &Path,
        document: StableDocument,
        extractor_id: &str,
        extractor_version: &str,
    ) -> Result<bool> {
        self.index_document_with_extractor_and_checkpoint(
            authorization,
            path,
            document,
            extractor_id,
            extractor_version,
            None,
        )
    }

    fn index_document_with_extractor_and_checkpoint(
        &self,
        authorization: &SourceAuthorization,
        path: &Path,
        document: StableDocument,
        extractor_id: &str,
        extractor_version: &str,
        checkpoint: Option<(&str, u64)>,
    ) -> Result<bool> {
        let prepared = PreparedIndexDocument::new(
            path,
            document,
            self.limits,
            extractor_id,
            extractor_version,
        )?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        let indexed =
            Self::commit_index_document(&transaction, authorization, &prepared, checkpoint)?;
        transaction.commit()?;
        Ok(indexed)
    }

    /// The caller owns the transaction, allowing canonical writes and parent completion to commit
    /// together. Only source-capability checks occur here, never extraction or source grants.
    fn commit_index_document(
        transaction: &Transaction<'_>,
        authorization: &SourceAuthorization,
        prepared: &PreparedIndexDocument,
        checkpoint: Option<(&str, u64)>,
    ) -> Result<bool> {
        let PreparedIndexDocument {
            source_uri,
            title,
            document,
            passages,
            extractor_id,
            extractor_version,
            parse_warnings_json,
            extraction_metadata_json,
            now,
        } = prepared;
        let page_count = document.page_count.map(i64::from);
        authorization.verify(transaction)?;
        if extractor_id == IMAGE_OCR_EXTRACTOR_ID {
            authorization.verify_ocr_result(transaction)?;
        }
        let root_id = &authorization.root_id;

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
                && projection.1 == *extractor_id
                && projection.2 == *extractor_version
        }) {
            if let Some((job_id, next_unit)) = checkpoint {
                update_index_job_checkpoint(transaction, authorization, job_id, next_unit, now)?;
            }
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
                extractor_id, extractor_version, parse_warnings_json, page_count,
                extraction_metadata_json, status, created_at
             ) VALUES (?1, ?2, ?3, 'blake3', ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'ready', ?11)",
            params![
                version_id,
                artifact_id,
                document.raw_hash,
                sql_i64(document.byte_size, "artifact byte size")?,
                document.modified_ns,
                extractor_id,
                extractor_version,
                parse_warnings_json,
                page_count,
                extraction_metadata_json,
                now
            ],
        )?;
        if inserted > 0 {
            insert_passages(transaction, &version_id, passages, now)?;
        }
        transaction.execute(
            "UPDATE artifacts SET active_version_id = ?1, last_seen_at = ?2 WHERE id = ?3",
            params![version_id, now, artifact_id],
        )?;
        if let Some((job_id, next_unit)) = checkpoint {
            update_index_job_checkpoint(transaction, authorization, job_id, next_unit, now)?;
        }
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
                    v.source_modified_ns, p.text, p.locator_json, bm25(passages_fts), p.rowid,
                    p.ordinal
                 FROM passages_fts
                 JOIN passages p ON p.rowid = passages_fts.rowid
                 JOIN artifact_versions v ON v.id = p.artifact_version_id
                 JOIN artifacts a ON a.id = v.artifact_id AND a.active_version_id = v.id
                 JOIN source_roots r ON r.id = a.source_root_id AND r.enabled = 1
                 JOIN artifact_locators l ON l.artifact_id = a.id AND l.active = 1
                 WHERE passages_fts MATCH ?1 AND a.state = 'active'",
            )?;
            let rows = statement.query_map(params![compiled.match_expression.as_str()], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, Option<i64>>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, String>(9)?,
                    row.get::<_, f64>(10)?,
                    row.get::<_, i64>(11)?,
                    row.get::<_, i64>(12)?,
                ))
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        let mut candidates = candidates
            .into_iter()
            .filter(
                |(
                    _artifact_id,
                    _version_id,
                    _passage_id,
                    _title,
                    media_type,
                    source_uri,
                    _content_hash,
                    source_modified_ns,
                    _passage_text,
                    locator_json,
                    _raw_bm25,
                    _passage_rowid,
                    _ordinal,
                )| {
                    serde_json::from_str::<EvidenceAnchor>(locator_json)
                        .map(|anchor| {
                            compiled.filters.matches(
                                media_type,
                                source_uri,
                                *source_modified_ns,
                                &anchor,
                            )
                        })
                        .unwrap_or(false)
                },
            )
            .collect::<Vec<_>>();
        // Filtering happens before this deterministic lexical order and page truncation. This is
        // intentionally explicit: a filtered-out row cannot be reintroduced by a later ranker.
        candidates.sort_by(|left, right| {
            left.10
                .total_cmp(&right.10)
                .then_with(|| left.3.cmp(&right.3))
                .then_with(|| left.0.cmp(&right.0))
                .then_with(|| left.11.cmp(&right.11))
        });
        candidates.truncate(limit as usize);
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
                _source_modified_ns,
                passage_text,
                locator_json,
                raw_bm25,
                passage_rowid,
                _ordinal,
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
            let confidence_state = anchor_confidence_state(&anchor);
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
                confidence_state,
                contributions: RankContributions {
                    lexical: 1.0 / (1.0 + raw_bm25.abs()),
                    semantic: 0.0,
                    metadata: if compiled.filters.is_empty() {
                        0.0
                    } else {
                        1.0
                    },
                    reranker: 0.0,
                },
                match_reason: "SQLite FTS5 BM25 over the active source passage".into(),
            });
        }
        Ok(hits)
    }

    /// Searches the lexical and semantic derivatives through the experimental hybrid ranker.
    ///
    /// The semantic derivative must be healthy; a missing or incompatible derivative fails closed
    /// instead of silently presenting a lexical-only result as a hybrid result. This method is not
    /// wired into the desktop default until the benchmark gate in issue 0204 passes.
    pub fn hybrid_search(&self, query: &str, limit: u32) -> Result<Vec<HybridSearchHit>> {
        let parsed = crate::search::parse_query(query)?;
        let limit = limit.clamp(1, 100);
        let candidate_limit = limit.saturating_mul(4).clamp(limit, 100);
        let lexical = self.search(&SearchRequest {
            text: query.to_string(),
            limit: candidate_limit,
        })?;
        let semantic = self.semantic_search_parsed(&parsed, candidate_limit)?;
        let mut inputs = BTreeMap::<String, HybridRankInput>::new();

        for hit in lexical {
            let passage_id = hit.passage_id.clone();
            inputs.insert(
                passage_id,
                HybridRankInput {
                    artifact_id: hit.artifact_id,
                    version_id: hit.version_id.clone(),
                    passage_id: hit.passage_id,
                    title: hit.title,
                    media_type: hit.media_type,
                    source_uri: hit.source_uri,
                    content_hash: hit.content_hash,
                    passage_text: hit
                        .excerpt
                        .segments
                        .iter()
                        .map(|segment| segment.text.as_str())
                        .collect(),
                    excerpt: hit.excerpt,
                    anchor: hit.anchor,
                    source_modified_ns: self.source_modified_ns(&hit.version_id)?,
                    lexical_rank: Some(hit.rank),
                    semantic_rank: None,
                },
            );
        }

        for candidate in semantic {
            if let Some(input) = inputs.get_mut(&candidate.passage_id) {
                input.semantic_rank = Some(candidate.rank);
                continue;
            }
            let passage_text = candidate.passage_text.clone();
            inputs.insert(
                candidate.passage_id.clone(),
                HybridRankInput {
                    artifact_id: candidate.artifact_id,
                    version_id: candidate.version_id.clone(),
                    passage_id: candidate.passage_id,
                    title: candidate.title,
                    media_type: candidate.media_type,
                    source_uri: candidate.source_uri,
                    content_hash: candidate.content_hash,
                    passage_text: passage_text.clone(),
                    excerpt: EvidenceExcerpt {
                        segments: vec![EvidenceSegment {
                            text: passage_text,
                            highlighted: false,
                        }],
                    },
                    anchor: candidate.anchor,
                    source_modified_ns: self.source_modified_ns(&candidate.version_id)?,
                    lexical_rank: None,
                    semantic_rank: Some(candidate.rank),
                },
            );
        }

        let mut hits = fuse_hybrid_candidates(
            &parsed.text,
            inputs.into_values().collect(),
            &HybridRankConfig::default(),
        )?;
        hits.truncate(limit as usize);
        for (index, hit) in hits.iter_mut().enumerate() {
            hit.rank = index as u32 + 1;
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
                 JOIN source_roots r ON r.id = a.source_root_id AND r.enabled = 1
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
                 JOIN source_roots r ON r.id = a.source_root_id AND r.enabled = 1
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

    /// Verifies a search result and returns the canonical passage for the evidence viewer.
    ///
    /// Source bytes are checked before and the active locator/version/passage row is checked after
    /// the hash read. If the source changed, disappeared, or the passage belongs to an older
    /// version, the caller receives `ArtifactStale` and can offer re-index/recovery instead of
    /// presenting misleading evidence.
    pub fn resolve_verified_evidence(
        &self,
        request: &ResolveEvidenceRequest,
    ) -> Result<EvidenceView> {
        let _path = self.resolve_verified_artifact_path(
            &request.artifact_id,
            &request.version_id,
            &request.content_hash,
        )?;
        Uuid::parse_str(&request.passage_id)
            .map_err(|_| LoomError::ArtifactStale(request.artifact_id.clone()))?;

        let connection = self.lock()?;
        let view: Option<EvidenceView> = connection
            .query_row(
                "SELECT
                    a.id, v.id, p.id, a.title, a.media_type, l.locator, v.content_hash,
                    p.text, p.locator_json, v.page_count, v.extractor_id, v.extractor_version,
                    v.extraction_metadata_json
                 FROM artifacts a
                 JOIN artifact_versions v ON v.id = a.active_version_id
                 JOIN passages p ON p.artifact_version_id = v.id
                 JOIN source_roots r ON r.id = a.source_root_id AND r.enabled = 1
                 JOIN artifact_locators l ON l.artifact_id = a.id AND l.active = 1
                 WHERE a.id = ?1 AND v.id = ?2 AND p.id = ?3
                   AND v.content_hash = ?4 AND a.state = 'active' AND l.kind = 'file'",
                params![
                    request.artifact_id,
                    request.version_id,
                    request.passage_id,
                    request.content_hash
                ],
                |row| {
                    let anchor_json: String = row.get(8)?;
                    let metadata_json: String = row.get(12)?;
                    let anchor: EvidenceAnchor =
                        serde_json::from_str(&anchor_json).map_err(|error| {
                            rusqlite::Error::FromSqlConversionFailure(
                                8,
                                rusqlite::types::Type::Text,
                                Box::new(error),
                            )
                        })?;
                    Ok(EvidenceView {
                        artifact_id: row.get(0)?,
                        version_id: row.get(1)?,
                        passage_id: row.get(2)?,
                        title: row.get(3)?,
                        media_type: row.get(4)?,
                        source_uri: row.get(5)?,
                        content_hash: row.get(6)?,
                        passage_text: row.get(7)?,
                        confidence_state: anchor_confidence_state(&anchor),
                        anchor,
                        page_count: row.get(9)?,
                        extractor_id: row.get(10)?,
                        extractor_version: row.get(11)?,
                        extraction_metadata: serde_json::from_str(&metadata_json).map_err(
                            |error| {
                                rusqlite::Error::FromSqlConversionFailure(
                                    12,
                                    rusqlite::types::Type::Text,
                                    Box::new(error),
                                )
                            },
                        )?,
                    })
                },
            )
            .optional()?;
        view.ok_or_else(|| LoomError::ArtifactStale(request.artifact_id.clone()))
    }

    fn start_index_job(
        &self,
        authorization: &SourceAuthorization,
        selection_locator: &str,
        discovery_fingerprint: &str,
        total_units: usize,
    ) -> Result<IndexJobProgress> {
        let total_units = sql_i64(total_units as u64, "index job unit count")?;
        let now = Utc::now().to_rfc3339();
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        authorization.verify(&transaction)?;
        let root_id = &authorization.root_id;
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

    fn approved_root_authorization(&self, locator: &str) -> Result<SourceAuthorization> {
        let connection = self.lock()?;
        connection
            .query_row(
                "SELECT id, scope_generation,
                    (SELECT value FROM schema_meta WHERE key = 'authorization_incarnation'), kind
                 FROM source_roots WHERE locator = ?1 AND enabled = 1",
                [locator],
                |row| {
                    Ok(SourceAuthorization {
                        root_id: row.get(0)?,
                        generation: row.get(1)?,
                        incarnation: row.get(2)?,
                        kind: row.get(3)?,
                        ocr_policy: None,
                    })
                },
            )
            .optional()?
            .ok_or_else(|| {
                LoomError::InvalidPath(format!("root is not an enabled approved source: {locator}"))
            })
    }

    fn approved_root_specs(&self) -> Result<Vec<(String, String, SourceAuthorization)>> {
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT locator, kind, id, scope_generation,
                (SELECT value FROM schema_meta WHERE key = 'authorization_incarnation') FROM source_roots
                WHERE enabled = 1 ORDER BY locator",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                SourceAuthorization {
                    root_id: row.get(2)?,
                    generation: row.get(3)?,
                    incarnation: row.get(4)?,
                    kind: row.get(1)?,
                    ocr_policy: None,
                },
            ))
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    fn mark_locator_missing_and_advance(
        &self,
        authorization: &SourceAuthorization,
        locator: &str,
        job_id: &str,
        next_unit: u64,
    ) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        authorization.verify(&transaction)?;
        transaction.execute(
            "UPDATE artifacts SET state = 'missing'
             WHERE source_root_id = ?1 AND state = 'active' AND id IN (
                 SELECT artifact_id FROM artifact_locators
                 WHERE artifact_id = artifacts.id AND kind = 'file' AND active = 1 AND locator = ?2
             )",
            params![authorization.root_id, locator],
        )?;
        update_index_job_checkpoint(&transaction, authorization, job_id, next_unit, &now)?;
        transaction.commit()?;
        Ok(())
    }

    fn interrupt_index_job(
        &self,
        authorization: &SourceAuthorization,
        job_id: &str,
        message: &str,
    ) -> Result<()> {
        let connection = self.lock()?;
        let transaction = connection.unchecked_transaction()?;
        authorization.verify(&transaction)?;
        let updated = transaction.execute(
            "UPDATE index_jobs SET state = 'interrupted', last_error = ?1, updated_at = ?2
             WHERE id = ?3 AND source_root_id = ?4 AND state = 'running'",
            params![
                message,
                Utc::now().to_rfc3339(),
                job_id,
                authorization.root_id
            ],
        )?;
        require_job_update(updated, job_id)?;
        transaction.commit()?;
        Ok(())
    }

    fn fail_index_job(
        &self,
        authorization: &SourceAuthorization,
        job_id: &str,
        message: &str,
    ) -> Result<()> {
        let connection = self.lock()?;
        let transaction = connection.unchecked_transaction()?;
        authorization.verify(&transaction)?;
        let updated = transaction.execute(
            "UPDATE index_jobs SET state = 'failed', last_error = ?1, updated_at = ?2
             WHERE id = ?3 AND source_root_id = ?4 AND state = 'running'",
            params![
                message,
                Utc::now().to_rfc3339(),
                job_id,
                authorization.root_id
            ],
        )?;
        require_job_update(updated, job_id)?;
        transaction.commit()?;
        Ok(())
    }

    fn complete_index_job(
        &self,
        authorization: &SourceAuthorization,
        job_id: &str,
        last_error: Option<&str>,
    ) -> Result<()> {
        let connection = self.lock()?;
        let transaction = connection.unchecked_transaction()?;
        authorization.verify(&transaction)?;
        let updated = transaction.execute(
            "UPDATE index_jobs
             SET state = 'completed', next_unit = total_units, last_error = ?1,
                 updated_at = ?2, completed_at = ?2
             WHERE id = ?3 AND source_root_id = ?4 AND state = 'running'",
            params![
                last_error,
                Utc::now().to_rfc3339(),
                job_id,
                authorization.root_id
            ],
        )?;
        require_job_update(updated, job_id)?;
        transaction.commit()?;
        Ok(())
    }

    fn advance_index_job(
        &self,
        authorization: &SourceAuthorization,
        job_id: &str,
        next_unit: u64,
    ) -> Result<()> {
        let connection = self.lock()?;
        let transaction = connection.unchecked_transaction()?;
        authorization.verify(&transaction)?;
        update_index_job_checkpoint(
            &transaction,
            authorization,
            job_id,
            next_unit,
            &Utc::now().to_rfc3339(),
        )?;
        transaction.commit()?;
        Ok(())
    }

    fn reconcile_directory(
        &self,
        authorization: &SourceAuthorization,
        seen: &HashSet<String>,
    ) -> Result<()> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        authorization.verify(&transaction)?;
        let root_id = &authorization.root_id;
        let candidates: Vec<(String, String)> = {
            let mut statement = transaction.prepare(
                "SELECT a.id, l.locator
                 FROM artifacts a
                 JOIN artifact_locators l ON l.artifact_id = a.id
                   AND l.kind = 'file' AND l.active = 1
                 WHERE a.source_root_id = ?1 AND a.state = 'active'",
            )?;
            let rows = statement.query_map([root_id], |row| Ok((row.get(0)?, row.get(1)?)))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
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
        let version: Option<VersionProjection> = connection
            .query_row(
                "SELECT v.id, v.content_hash, v.extractor_id, v.extractor_version,
                        v.page_count, v.parse_warnings_json, v.extraction_metadata_json
                 FROM artifact_locators l
                 JOIN artifacts a ON a.id = l.artifact_id
                 JOIN source_roots r ON r.id = a.source_root_id AND r.enabled = 1
                 JOIN artifact_versions v ON v.id = a.active_version_id
                WHERE l.kind = 'file' AND l.locator = ?1 AND l.active = 1
                   AND a.state = 'active'",
                [&source_uri],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                    ))
                },
            )
            .optional()?;
        let (
            version_id,
            content_hash,
            extractor_id,
            extractor_version,
            page_count,
            parse_warnings_json,
            extraction_metadata_json,
        ) = version.ok_or_else(|| LoomError::ArtifactNotFound(source_uri.clone()))?;
        let parse_warnings = serde_json::from_str(&parse_warnings_json)?;
        let extraction_metadata = serde_json::from_str(&extraction_metadata_json)?;
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
            page_count: page_count.and_then(|value| u32::try_from(value).ok()),
            parse_warnings,
            extraction_metadata,
            passages,
        })
    }

    /// Accounts for canonical source records, SQLite sidecars, and known disposable files.
    ///
    /// The report is an estimate rather than a forensic disk-usage claim: canonical source bytes
    /// come from the recorded version size, while database and disposable entries use filesystem
    /// sizes. Symlinks are never followed, and unknown sibling directories are not inspected.
    pub fn inspect_storage(&self) -> Result<StorageInspection> {
        let generated_at = Utc::now().to_rfc3339();
        let database_path = self.database_path.clone();
        let mut entries = Vec::new();
        let mut source_bytes = 0_u64;

        {
            let connection = self.lock()?;
            let mut statement = connection.prepare(
                "SELECT l.locator, COALESCE(SUM(v.byte_size), 0), COUNT(DISTINCT a.id)
                 FROM artifacts a
                 JOIN artifact_locators l ON l.artifact_id = a.id AND l.active = 1
                 LEFT JOIN artifact_versions v ON v.artifact_id = a.id
                 GROUP BY l.locator
                 ORDER BY l.locator",
            )?;
            let rows = statement.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?.max(0) as u64,
                    row.get::<_, i64>(2)?.max(0) as u64,
                ))
            })?;
            for row in rows {
                let (source_uri, bytes, files) = row?;
                source_bytes = source_bytes.saturating_add(bytes);
                entries.push(StorageEntry {
                    category: "source".into(),
                    path: source_uri.clone(),
                    source_uri: Some(source_uri),
                    bytes,
                    files,
                    exists: true,
                });
            }

            let canonical_bytes = storage_sql_bytes(
                &connection,
                "SELECT COALESCE(SUM(length(id) + length(title) + length(media_type) + length(state)), 0)
                 FROM artifacts",
            )?;
            if canonical_bytes > 0 {
                entries.push(StorageEntry {
                    category: "canonical_records".into(),
                    path: database_path
                        .as_ref()
                        .map(|path| path.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "<memory>".into()),
                    source_uri: None,
                    bytes: canonical_bytes,
                    files: 1,
                    exists: true,
                });
            }

            let derived_bytes = storage_sql_bytes(
                &connection,
                "SELECT
                    (SELECT COALESCE(SUM(length(text) + length(locator_json)), 0) FROM passages) +
                    (SELECT COALESCE(SUM(length(vector_blob)), 0) FROM semantic_embeddings) +
                    (SELECT COALESCE(SUM(length(term)), 0) FROM passages_fts_vocab)",
            )?;
            if derived_bytes > 0 {
                entries.push(StorageEntry {
                    category: "derived_records".into(),
                    path: database_path
                        .as_ref()
                        .map(|path| path.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "<memory>".into()),
                    source_uri: None,
                    bytes: derived_bytes,
                    files: 1,
                    exists: true,
                });
            }
        }

        if let Some(path) = database_path.as_ref() {
            let database_path_text = path.to_string_lossy().into_owned();
            let database_bytes = regular_file_size(path)?;
            if database_bytes > 0 {
                entries.push(StorageEntry {
                    category: "database".into(),
                    path: database_path_text,
                    source_uri: None,
                    bytes: database_bytes,
                    files: 1,
                    exists: true,
                });
            }
            for suffix in ["-wal", "-shm", "-journal"] {
                let sidecar = PathBuf::from(format!("{}{}", path.display(), suffix));
                let bytes = regular_file_size(&sidecar)?;
                if bytes > 0 {
                    entries.push(StorageEntry {
                        category: "sqlite_sidecar".into(),
                        path: sidecar.to_string_lossy().into_owned(),
                        source_uri: None,
                        bytes,
                        files: 1,
                        exists: true,
                    });
                }
            }
            if let Some(root) = path.parent() {
                for (directory, category) in DISPOSABLE_DIRECTORIES {
                    let directory_path = root.join(directory);
                    let (bytes, files) = directory_size(&directory_path)?;
                    if bytes > 0 || directory_path.exists() {
                        entries.push(StorageEntry {
                            category: (*category).into(),
                            path: directory_path.to_string_lossy().into_owned(),
                            source_uri: None,
                            bytes,
                            files,
                            exists: directory_path.exists(),
                        });
                    }
                }
                let captures = root.join("captures");
                let (bytes, files) = directory_size(&captures)?;
                if bytes > 0 || captures.exists() {
                    entries.push(StorageEntry {
                        category: "capture".into(),
                        path: captures.to_string_lossy().into_owned(),
                        source_uri: None,
                        bytes,
                        files,
                        exists: captures.exists(),
                    });
                }
            }
        }

        let total_bytes = entries
            .iter()
            .map(|entry| entry.bytes)
            .fold(0_u64, u64::saturating_add);
        let canonical_bytes = entries
            .iter()
            .filter(|entry| entry.category == "source" || entry.category == "canonical_records")
            .map(|entry| entry.bytes)
            .fold(0_u64, u64::saturating_add);
        let derived_bytes = entries
            .iter()
            .filter(|entry| {
                matches!(
                    entry.category.as_str(),
                    "derived_records" | "sqlite_sidecar"
                )
            })
            .map(|entry| entry.bytes)
            .fold(0_u64, u64::saturating_add);
        let disposable_bytes = entries
            .iter()
            .filter(|entry| {
                matches!(
                    entry.category.as_str(),
                    "cache"
                        | "model_cache"
                        | "thumbnails"
                        | "ocr_scratch"
                        | "temporary_export"
                        | "log"
                )
            })
            .map(|entry| entry.bytes)
            .fold(0_u64, u64::saturating_add);

        Ok(StorageInspection {
            database_path: database_path.map(|path| path.to_string_lossy().into_owned()),
            generated_at,
            entries,
            total_bytes,
            canonical_bytes,
            derived_bytes,
            disposable_bytes,
            source_bytes,
        })
    }

    /// Permanently deletes one artifact and every canonical/derived row attached to it.
    pub fn purge_artifact(&self, artifact_id: &str) -> Result<DeletionReport> {
        validate_relationship_id(artifact_id, "artifact")?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        let report = delete_artifact_transaction(&transaction, artifact_id)?;
        transaction.commit()?;
        drop(connection);
        self.finish_deletion(report)
    }

    /// Permanently deletes every artifact rooted at one exact persisted locator.
    pub fn purge_root(&self, locator: &str) -> Result<DeletionReport> {
        if locator.trim().is_empty() || locator.chars().any(char::is_control) {
            return Err(LoomError::InvalidPath(
                "source root locator is invalid".into(),
            ));
        }
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        let artifact_ids = artifact_ids_for_root(&transaction, locator)?;
        let mut report = DeletionReport {
            selector: format!("root:{locator}"),
            ..DeletionReport::default()
        };
        crate::jobs::purge_file_targets(&transaction, Some(locator), None, false)?;
        for artifact_id in artifact_ids {
            merge_deletion_reports(
                &mut report,
                delete_artifact_transaction(&transaction, &artifact_id)?,
            );
        }
        transaction.execute("DELETE FROM source_roots WHERE locator = ?1", [locator])?;
        transaction.commit()?;
        drop(connection);
        self.finish_deletion(report)
    }

    /// Permanently deletes artifacts created before an RFC3339 cutoff.
    pub fn purge_before(&self, cutoff: &str) -> Result<DeletionReport> {
        let cutoff = normalize_timestamp(cutoff)?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        let mut statement = transaction
            .prepare("SELECT id FROM artifacts WHERE created_at < ?1 ORDER BY created_at, id")?;
        let artifact_ids = statement
            .query_map([cutoff.as_str()], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(statement);
        let mut report = DeletionReport {
            selector: format!("before:{cutoff}"),
            ..DeletionReport::default()
        };
        for artifact_id in artifact_ids {
            merge_deletion_reports(
                &mut report,
                delete_artifact_transaction(&transaction, &artifact_id)?,
            );
        }
        transaction.commit()?;
        drop(connection);
        self.finish_deletion(report)
    }

    /// Returns the persisted retention policy without mutating the library.
    pub fn retention_policy(&self) -> Result<RetentionPolicy> {
        let connection = self.lock()?;
        let days = connection
            .query_row(
                "SELECT value FROM schema_meta WHERE key = 'retention_days'",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .and_then(|value| value.parse::<u32>().ok());
        Ok(RetentionPolicy { days })
    }

    /// Sets or clears the local retention policy. It does not delete until `apply_retention` is
    /// explicitly invoked, keeping destructive behavior visible and reversible at the policy step.
    pub fn set_retention_days(&self, days: Option<u32>) -> Result<RetentionPolicy> {
        if days.is_some_and(|days| days == 0 || days > 36_500) {
            return Err(LoomError::InvalidPath(
                "retention must be between 1 and 36500 days, or disabled".into(),
            ));
        }
        let connection = self.lock()?;
        connection.execute(
            "INSERT INTO schema_meta(key, value) VALUES ('retention_days', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [days.map_or_else(|| "".into(), |value| value.to_string())],
        )?;
        Ok(RetentionPolicy { days })
    }

    /// Applies the configured retention policy at the current UTC clock instant.
    pub fn apply_retention(&self) -> Result<RetentionReport> {
        self.apply_retention_at(&Utc::now().to_rfc3339())
    }

    /// Deterministic retention entry point used by device and migration tests.
    pub fn apply_retention_at(&self, evaluated_at: &str) -> Result<RetentionReport> {
        let evaluated_at = normalize_timestamp(evaluated_at)?;
        let policy = self.retention_policy()?;
        let Some(days) = policy.days else {
            return Ok(RetentionReport {
                policy,
                evaluated_at,
                ..RetentionReport::default()
            });
        };
        let evaluated = DateTime::parse_from_rfc3339(&evaluated_at)
            .map_err(|error| LoomError::InvalidPath(format!("invalid retention clock: {error}")))?
            .with_timezone(&Utc);
        let cutoff = (evaluated - Duration::days(i64::from(days))).to_rfc3339();
        let deletion = self.purge_before(&cutoff)?;
        Ok(RetentionReport {
            policy,
            evaluated_at,
            cutoff: Some(cutoff),
            deletion,
        })
    }

    /// Deletes files in LOOM's known disposable directories and checkpoints SQLite sidecars.
    /// SQLite retains ownership of live WAL/SHM/journal lifecycle; user-selected source files and
    /// managed captures are not touched by this operation.
    pub fn purge_disposable_storage(&self) -> Result<DeletionReport> {
        let mut report = DeletionReport {
            selector: "disposable-storage".into(),
            ..DeletionReport::default()
        };
        if let Some(database_path) = self.database_path.as_ref() {
            {
                let connection = self.lock()?;
                checkpoint_and_vacuum(&connection)?;
            }
            if let Some(root) = database_path.parent() {
                for (directory, _) in DISPOSABLE_DIRECTORIES {
                    remove_directory_contents(&root.join(directory), &mut report)?;
                }
            }
        }
        Ok(report)
    }

    /// Permanently removes one exact source root and its canonical evidence rows.
    ///
    /// This method is intentionally locator-bound and is used by the explicit capture purge
    /// control. It cannot broaden to a parent directory or delete another source root.
    pub fn purge_source_root(&self, locator: &str) -> Result<crate::CapturePurgeReport> {
        let report = self.purge_root(locator)?;
        Ok(crate::CapturePurgeReport {
            artifacts_deleted: report.artifacts_deleted,
            versions_deleted: report.versions_deleted,
            passages_deleted: report.passages_deleted,
        })
    }

    /// Returns the persisted local OCR policy and the number of derived OCR records.
    pub fn ocr_status(&self) -> Result<OcrStatus> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        let policy = OcrPolicy::load(&transaction)?;
        let derived_versions = count_where(
            &transaction,
            "artifact_versions",
            "extractor_id = 'loom.ocr'",
        )?;
        let derived_passages = count_where(
            &transaction,
            "passages",
            "artifact_version_id IN (SELECT id FROM artifact_versions WHERE extractor_id = 'loom.ocr')",
        )?;
        transaction.commit()?;
        Ok(OcrStatus {
            enabled: policy.enabled,
            derived_versions,
            derived_passages,
        })
    }

    /// Enables or disables image OCR. Disabling is destructive only to derived OCR records: the
    /// original image locator and bytes remain untouched and can be re-indexed after re-enabling.
    pub fn set_ocr_enabled(&self, enabled: bool) -> Result<OcrPurgeReport> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        transaction.execute(
            "INSERT INTO schema_meta(key, value) VALUES ('ocr_enabled', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [if enabled { "1" } else { "0" }],
        )?;
        rotate_ocr_policy_revision(&transaction)?;
        let report = if enabled {
            OcrPurgeReport::default()
        } else {
            purge_ocr_records_transaction(&transaction)?
        };
        transaction.commit()?;
        Ok(report)
    }

    /// Deletes all derived OCR versions/passages while retaining source locators and bytes.
    pub fn purge_ocr_records(&self) -> Result<OcrPurgeReport> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        rotate_ocr_policy_revision(&transaction)?;
        let report = purge_ocr_records_transaction(&transaction)?;
        transaction.commit()?;
        Ok(report)
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

    /// Rebuilds the disposable semantic vectors from active canonical passages.
    ///
    /// The operation deletes and recreates only derivative rows. Every vector is bound to the
    /// passage text hash and the provider manifest, so incompatible records cannot be mixed into a
    /// later search. Canonical artifacts, versions, passages, and anchors are never modified.
    pub fn semantic_rebuild(&self) -> Result<SemanticRebuildReport> {
        let provider = HashEmbeddingProvider::default();
        let config = provider.config().clone();
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        let passages = {
            let mut statement = transaction.prepare(
                "SELECT p.id, p.text, p.text_hash
                 FROM passages p
                 JOIN artifact_versions v ON v.id = p.artifact_version_id
                 JOIN artifacts a ON a.id = v.artifact_id AND a.active_version_id = v.id
                 JOIN source_roots r ON r.id = a.source_root_id AND r.enabled = 1
                 WHERE a.state = 'active'
                 ORDER BY p.id",
            )?;
            let rows = statement.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        let mut source_hasher = blake3::Hasher::new();
        for (passage_id, _, passage_hash) in &passages {
            source_hasher.update(passage_id.as_bytes());
            source_hasher.update(&[0]);
            source_hasher.update(passage_hash.as_bytes());
            source_hasher.update(&[0]);
        }
        let source_digest = format!("blake3:{}", source_hasher.finalize().to_hex());

        transaction.execute("DELETE FROM semantic_embeddings", [])?;
        transaction.execute("DELETE FROM semantic_index_meta", [])?;
        let mut insert = transaction.prepare(
            "INSERT INTO semantic_embeddings(
                passage_id, passage_hash, provider_id, model_id, tokenizer, dimension,
                normalization, build_parameters, index_revision, vector_blob, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        )?;
        let now = Utc::now().to_rfc3339();
        let mut vector_bytes = 0_u64;
        for (passage_id, text, passage_hash) in &passages {
            let vector = provider.embed(text);
            let encoded = encode_vector(&vector);
            vector_bytes = vector_bytes
                .checked_add(encoded.len() as u64)
                .ok_or_else(|| {
                    LoomError::SemanticIndexIncompatible("vector byte count overflow".into())
                })?;
            insert.execute(params![
                passage_id,
                passage_hash,
                config.provider_id,
                config.model_id,
                config.tokenizer,
                sql_i64(u64::from(config.dimension), "semantic dimension")?,
                config.normalization,
                config.build_parameters,
                config.index_revision,
                encoded,
                now,
            ])?;
        }
        drop(insert);
        let passage_count = passages.len() as u64;
        transaction.execute(
            "INSERT INTO semantic_index_meta(
                slot, provider_id, model_id, tokenizer, dimension, normalization,
                build_parameters, index_revision, source_digest, canonical_passages,
                indexed_passages, vector_bytes, built_at
             ) VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9, ?10, ?11)",
            params![
                config.provider_id,
                config.model_id,
                config.tokenizer,
                sql_i64(u64::from(config.dimension), "semantic dimension")?,
                config.normalization,
                config.build_parameters,
                config.index_revision,
                source_digest,
                sql_i64(passage_count, "semantic passage count")?,
                sql_i64(vector_bytes, "semantic vector bytes")?,
                now,
            ],
        )?;
        transaction.commit()?;
        Ok(SemanticRebuildReport {
            manifest: SemanticIndexManifest {
                config,
                source_digest,
                canonical_passages: passage_count,
                indexed_passages: passage_count,
                vector_bytes,
            },
            rebuilt_passages: passage_count,
        })
    }

    /// Measures the local provider candidates against the current active passage corpus.
    ///
    /// This is an architecture measurement, not a retrieval-quality claim. It reports vector
    /// footprint and elapsed embedding time for the deterministic token, character n-gram, and
    /// token-count baselines on this device.
    pub fn semantic_provider_benchmark(&self) -> Result<Vec<SemanticProviderMeasurement>> {
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT p.text
             FROM passages p
             JOIN artifact_versions v ON v.id = p.artifact_version_id
             JOIN artifacts a ON a.id = v.artifact_id AND a.active_version_id = v.id
             JOIN source_roots r ON r.id = a.source_root_id AND r.enabled = 1
             WHERE a.state = 'active'
             ORDER BY p.id",
        )?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        let passages = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(measure_providers(
            &passages,
            SemanticIndexConfig::default().dimension,
        ))
    }

    /// Reports whether the semantic derivative matches current active canonical passages.
    pub fn semantic_status(&self) -> Result<SemanticIndexStatus> {
        let connection = self.lock()?;
        let (canonical_passages, canonical_digest) = canonical_semantic_source(&connection)?;
        let config = SemanticIndexConfig::default();
        let meta: Option<SemanticMetaRow> = connection
            .query_row(
                "SELECT provider_id, model_id, tokenizer, dimension, normalization,
                        build_parameters, index_revision, source_digest, canonical_passages,
                        indexed_passages, vector_bytes
                 FROM semantic_index_meta WHERE slot = 1",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                        row.get(7)?,
                        row.get(8)?,
                        row.get(9)?,
                        row.get(10)?,
                    ))
                },
            )
            .optional()?;
        let indexed_passages = count(&connection, "semantic_embeddings")?;
        let vector_bytes = connection.query_row(
            "SELECT COALESCE(SUM(length(vector_blob)), 0) FROM semantic_embeddings",
            [],
            |row| row.get::<_, i64>(0),
        )?;
        let invalid_vectors: i64 = connection.query_row(
            "SELECT COUNT(*) FROM semantic_embeddings
             WHERE provider_id <> ?1 OR model_id <> ?2 OR tokenizer <> ?3
                OR dimension <> ?4 OR normalization <> ?5 OR build_parameters <> ?6
                OR index_revision <> ?7 OR length(vector_blob) <> ?8",
            params![
                config.provider_id,
                config.model_id,
                config.tokenizer,
                sql_i64(u64::from(config.dimension), "semantic dimension")?,
                config.normalization,
                config.build_parameters,
                config.index_revision,
                sql_i64(u64::from(config.dimension) * 4, "semantic vector size")?,
            ],
            |row| row.get(0),
        )?;
        let invalid_bindings: i64 = connection.query_row(
            "SELECT COUNT(*) FROM semantic_embeddings e
             JOIN passages p ON p.id = e.passage_id
             WHERE e.passage_hash <> p.text_hash",
            [],
            |row| row.get(0),
        )?;

        let Some((
            provider_id,
            model_id,
            tokenizer,
            dimension,
            normalization,
            build_parameters,
            index_revision,
            source_digest,
            stored_canonical,
            stored_indexed,
            stored_vector_bytes,
        )) = meta
        else {
            return Ok(SemanticIndexStatus {
                healthy: false,
                canonical_passages,
                indexed_passages,
                canonical_digest,
                vector_bytes: nonnegative_u64(vector_bytes),
                manifest: None,
                reason: Some("semantic index has not been built".into()),
            });
        };
        let stored_config = SemanticIndexConfig {
            provider_id,
            model_id,
            tokenizer,
            dimension: u32::try_from(dimension).map_err(|_| {
                LoomError::SemanticIndexIncompatible("manifest dimension is invalid".into())
            })?,
            normalization,
            build_parameters,
            index_revision,
        };
        let manifest = SemanticIndexManifest {
            config: stored_config.clone(),
            source_digest: source_digest.clone(),
            canonical_passages: nonnegative_u64(stored_canonical),
            indexed_passages: nonnegative_u64(stored_indexed),
            vector_bytes: nonnegative_u64(stored_vector_bytes),
        };
        let mut reasons = Vec::new();
        if stored_config != config {
            reasons.push("provider manifest does not match the current provider".into());
        }
        if source_digest != canonical_digest {
            reasons.push("canonical passage digest changed; rebuild required".into());
        }
        if nonnegative_u64(stored_canonical) != canonical_passages {
            reasons.push("manifest canonical passage count is inconsistent".into());
        }
        if nonnegative_u64(stored_indexed) != indexed_passages {
            reasons.push("manifest indexed passage count is inconsistent".into());
        }
        if invalid_vectors > 0 {
            reasons.push(format!("{invalid_vectors} vector records are incompatible"));
        }
        if invalid_bindings > 0 {
            reasons.push(format!(
                "{invalid_bindings} vector records have stale passage hashes"
            ));
        }
        if nonnegative_u64(stored_vector_bytes) != nonnegative_u64(vector_bytes) {
            reasons.push("manifest vector byte count is inconsistent".into());
        }
        Ok(SemanticIndexStatus {
            healthy: reasons.is_empty(),
            canonical_passages,
            indexed_passages,
            canonical_digest,
            vector_bytes: nonnegative_u64(vector_bytes),
            manifest: Some(manifest),
            reason: (!reasons.is_empty()).then(|| reasons.join("; ")),
        })
    }

    /// Removes the semantic derivative while leaving every canonical row untouched.
    pub fn semantic_drop(&self) -> Result<SemanticDropReport> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        let embeddings_deleted = transaction.execute("DELETE FROM semantic_embeddings", [])? as u64;
        let manifest_deleted = transaction.execute("DELETE FROM semantic_index_meta", [])? > 0;
        transaction.commit()?;
        Ok(SemanticDropReport {
            embeddings_deleted,
            manifest_deleted,
        })
    }

    /// Searches the rebuilt semantic derivative and returns only evidence-bound candidates.
    pub fn semantic_search(&self, query: &str, limit: u32) -> Result<Vec<SemanticCandidate>> {
        let parsed = crate::search::parse_query(query)?;
        self.semantic_search_parsed(&parsed, limit)
    }

    fn semantic_search_parsed(
        &self,
        parsed: &crate::search::ParsedQuery,
        limit: u32,
    ) -> Result<Vec<SemanticCandidate>> {
        let status = self.semantic_status()?;
        if !status.healthy {
            return Err(LoomError::SemanticIndexUnavailable(
                status
                    .reason
                    .unwrap_or_else(|| "semantic index is not ready".into()),
            ));
        }
        let manifest = status.manifest.ok_or_else(|| {
            LoomError::SemanticIndexUnavailable("semantic index has not been built".into())
        })?;
        let provider = HashEmbeddingProvider::default();
        let query_vector = provider.embed(&parsed.text);
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT e.passage_id, e.passage_hash, e.vector_blob, e.model_id, e.index_revision,
                    a.id, v.id, a.title, a.media_type, l.locator, v.content_hash,
                    v.source_modified_ns, p.text, p.locator_json
             FROM semantic_embeddings e
             JOIN passages p ON p.id = e.passage_id
             JOIN artifact_versions v ON v.id = p.artifact_version_id
             JOIN artifacts a ON a.id = v.artifact_id AND a.active_version_id = v.id
             JOIN source_roots r ON r.id = a.source_root_id AND r.enabled = 1
             JOIN artifact_locators l ON l.artifact_id = a.id AND l.active = 1 AND l.kind = 'file'
             WHERE a.state = 'active'
             ORDER BY e.passage_id",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Vec<u8>>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, String>(8)?,
                row.get::<_, String>(9)?,
                row.get::<_, String>(10)?,
                row.get::<_, Option<i64>>(11)?,
                row.get::<_, String>(12)?,
                row.get::<_, String>(13)?,
            ))
        })?;
        let rows = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        let mut candidates = Vec::with_capacity(rows.len());
        for (
            passage_id,
            passage_hash,
            vector_blob,
            model_id,
            index_revision,
            artifact_id,
            version_id,
            title,
            media_type,
            source_uri,
            content_hash,
            source_modified_ns,
            passage_text,
            locator_json,
        ) in rows
        {
            let vector =
                decode_vector(&vector_blob, manifest.config.dimension).ok_or_else(|| {
                    LoomError::SemanticIndexIncompatible(format!(
                        "vector for passage {passage_id} has an invalid dimension"
                    ))
                })?;
            let score = cosine_similarity(&query_vector, &vector).unwrap_or(0.0);
            let anchor = serde_json::from_str(&locator_json)?;
            if !parsed
                .filters
                .matches(&media_type, &source_uri, source_modified_ns, &anchor)
            {
                continue;
            }
            candidates.push(SemanticCandidate {
                rank: 0,
                score,
                artifact_id,
                version_id,
                passage_id,
                title,
                media_type,
                source_uri,
                content_hash,
                passage_hash,
                passage_text,
                anchor,
                model_id,
                index_revision,
            });
        }
        candidates.sort_by(|left, right| {
            right
                .score
                .total_cmp(&left.score)
                .then_with(|| left.passage_id.cmp(&right.passage_id))
        });
        candidates.truncate(limit.clamp(1, 100) as usize);
        for (index, candidate) in candidates.iter_mut().enumerate() {
            candidate.rank = index as u32 + 1;
        }
        Ok(candidates)
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

    pub(crate) fn lock(&self) -> Result<MutexGuard<'_, Connection>> {
        self.connection.lock().map_err(|_| LoomError::LockPoisoned)
    }

    pub(crate) fn job_database_path(&self) -> Result<PathBuf> {
        let path = self
            .database_path
            .as_ref()
            .ok_or_else(|| LoomError::JobQueue("a persistent library is required".into()))?;
        #[cfg(unix)]
        if Some(database_file_identity(path)?) != self.database_file_id {
            return Err(LoomError::JobQueue(
                "database identity changed since opening".into(),
            ));
        }
        Ok(path.clone())
    }

    pub(crate) fn open_job_worker(&self) -> Result<crate::jobs::JobWorker> {
        crate::jobs::JobWorker::open_with_limits(self.job_database_path()?, self.limits)
    }

    /// Opens an existing initialized queue without migration or an implicit FTS rebuild.
    /// Initialize new or older libraries with `Library::open` explicitly first.
    pub fn open_for_jobs(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_for_jobs_with_limits(path.as_ref(), LibraryLimits::default())
    }

    pub(crate) fn open_for_jobs_with_limits(path: &Path, limits: LibraryLimits) -> Result<Self> {
        Self::open_existing_job_library(path, limits, true)
    }

    pub(crate) fn open_existing_job_library(
        path: &Path,
        limits: LibraryLimits,
        validate_runtime: bool,
    ) -> Result<Self> {
        let path = path.canonicalize().map_err(|error| io_error(path, error))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if fs::metadata(&path)
                .map_err(|error| io_error(&path, error))?
                .nlink()
                != 1
            {
                return Err(LoomError::JobQueue(
                    "hard-linked database aliases are not supported by the queue".into(),
                ));
            }
        }
        let connection =
            Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        if stored_schema_version(&connection)? != Some(SCHEMA_VERSION) {
            return Err(LoomError::JobQueue(
                "worker requires the current validated library schema".into(),
            ));
        }
        validate_schema_shape(&connection, SCHEMA_VERSION)?;
        if validate_runtime {
            crate::jobs::validate_schema(&connection)?;
        }
        configure(&connection)?;
        connection.execute_batch("PRAGMA trusted_schema = OFF;")?;
        Ok(Self {
            connection: Mutex::new(connection),
            limits,
            #[cfg(unix)]
            database_file_id: Some(database_file_identity(&path)?),
            database_path: Some(path),
        })
    }

    pub(crate) fn job_repair_fts(
        &self,
        claim: &crate::jobs::JobClaim,
    ) -> Result<crate::jobs::BackgroundJob> {
        let mut connection = self.lock()?;
        let transaction =
            connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        claim.verify_operation(&transaction, "fts_repair", None)?;
        let before = fts_health(&transaction)?;
        rebuild_fts(&transaction)?;
        let after = fts_health(&transaction)?;
        if !after.healthy {
            return Err(LoomError::JobQueue(
                "FTS repair did not produce a healthy projection".into(),
            ));
        }
        let report = serde_json::to_string(&FtsRepairReport { before, after })?;
        let completed = claim.complete(&transaction, &report)?;
        transaction.commit()?;
        Ok(completed)
    }

    pub(crate) fn queue_file_locator(&self, path: &Path) -> Result<String> {
        canonical_queue_file(path, self.limits.max_file_bytes.min(8 * 1024 * 1024))
    }

    pub(crate) fn job_index_file(
        &self,
        claim: &crate::jobs::JobClaim,
        json: &str,
        supervisor: Option<&loom_extraction::ExtractionSupervisor>,
    ) -> Result<crate::jobs::BackgroundJob> {
        let prepared = self.prepare_file_job(claim, json, supervisor)?;
        self.publish_file_job(claim, &prepared)
    }

    pub(crate) fn prepare_file_job(
        &self,
        claim: &crate::jobs::JobClaim,
        json: &str,
        supervisor: Option<&loom_extraction::ExtractionSupervisor>,
    ) -> Result<PreparedFileJob> {
        let target = IndexFileTarget::parse(json)?;
        let snapshot = {
            let mut connection = self.lock()?;
            let transaction = connection.transaction()?;
            claim.verify_operation(&transaction, "index_file", Some(json))?;
            target
                .authorization
                .verify_locator(&transaction, &target.locator)?;
            let snapshot = canonical_file_snapshot(&transaction, &target.locator)?;
            target.verify_artifact_identity(&snapshot)?;
            if snapshot
                .as_ref()
                .is_some_and(|record| record.root_id != target.authorization.root_id)
            {
                return Err(LoomError::SourceRevoked(
                    target.authorization.root_id.clone(),
                ));
            }
            transaction.commit()?;
            snapshot
        };
        // Provider work owns no SQLite transaction and no interactive connection mutex.
        let path = Path::new(&target.locator);
        let media_type = ingest::supported_media_type(path)
            .ok_or_else(|| LoomError::UnsupportedSource("queued media is unsupported".into()))?;
        if media_type != target.media_type {
            return Err(LoomError::JobQueue("file media type changed".into()));
        }
        if media_type.starts_with("image/")
            && !target
                .authorization
                .ocr_policy
                .as_ref()
                .is_some_and(|p| p.enabled)
        {
            return Err(LoomError::OcrDisabled);
        }
        let stable =
            ingest::read_stable_bytes(path, path, self.limits.max_file_bytes.min(8 * 1024 * 1024))?;
        let media = loom_extraction::MediaKind::from_mime(media_type)?;
        let mut budget = loom_extraction::ExtractionBudget::for_media(media);
        budget.max_pdf_pages = self.limits.max_pdf_pages.min(2048) as u32;
        let adjacent;
        let supervisor = match supervisor {
            Some(supervisor) => supervisor,
            None => {
                adjacent = loom_extraction::ExtractionSupervisor::adjacent()?;
                &adjacent
            }
        };
        let output = supervisor
            .extract(&stable.bytes, media, budget, || {
                self.probe_file_job(claim, json, &target, &snapshot)
            })
            .map_err(|error| match error {
                loom_extraction::RunError::Extraction(error) => LoomError::from(error),
                loom_extraction::RunError::Interrupted(error) => error,
            })?;
        let document = ingest::stable_document_from_output(stable, media_type, output.source, None);
        enforce_file_job_output_bounds(&document)?;
        if self
            .limits
            .passage_target_chars
            .saturating_sub(self.limits.passage_overlap_chars)
            < 256
        {
            return Err(LoomError::JobQueue(
                "queued passage settings exceed the output budget".into(),
            ));
        }
        let (extractor_id, extractor_version) = match document.media_type {
            "application/pdf" => (PDF_EXTRACTOR_ID, PDF_EXTRACTOR_VERSION),
            media if media.starts_with("image/") => (
                IMAGE_OCR_EXTRACTOR_ID,
                crate::ocr::IMAGE_OCR_EXTRACTOR_VERSION,
            ),
            _ => (EXTRACTOR_ID, EXTRACTOR_VERSION),
        };
        if document.media_type != target.media_type {
            return Err(LoomError::JobQueue("file media type changed".into()));
        }
        let document = PreparedIndexDocument::new(
            path,
            document,
            self.limits,
            extractor_id,
            extractor_version,
        )?;
        if document.passages.len() > 8192 {
            return Err(LoomError::JobQueue("queued passages exceed 8192".into()));
        }
        Ok(PreparedFileJob {
            target,
            target_json: json.to_owned(),
            snapshot,
            document,
            extraction_metrics: output.metrics,
        })
    }

    /// Short DB-only probe: no source access or five-second SQLite busy wait while a child runs.
    fn probe_file_job(
        &self,
        claim: &crate::jobs::JobClaim,
        json: &str,
        target: &IndexFileTarget,
        snapshot: &Option<CanonicalFileSnapshot>,
    ) -> Result<()> {
        let mut connection = self.connection.try_lock().map_err(|error| match error {
            std::sync::TryLockError::Poisoned(_) => LoomError::LockPoisoned,
            std::sync::TryLockError::WouldBlock => {
                LoomError::Database(rusqlite::Error::SqliteFailure(
                    rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_BUSY),
                    None,
                ))
            }
        })?;
        connection.busy_timeout(std::time::Duration::ZERO)?;
        let result = (|| {
            let transaction = connection.transaction()?;
            claim.verify_operation(&transaction, "index_file", Some(json))?;
            target
                .authorization
                .verify_persisted_locator(&transaction, &target.locator)?;
            let current = canonical_file_snapshot(&transaction, &target.locator)?;
            target.verify_artifact_identity(&current)?;
            if &current != snapshot {
                return Err(LoomError::SourceChanged(target.locator.clone()));
            }
            transaction.commit()?;
            Ok(())
        })();
        let restored = connection
            .busy_timeout(std::time::Duration::from_secs(5))
            .map_err(LoomError::from);
        result.and(restored)
    }

    pub(crate) fn publish_file_job(
        &self,
        claim: &crate::jobs::JobClaim,
        prepared: &PreparedFileJob,
    ) -> Result<crate::jobs::BackgroundJob> {
        let mut connection = self.lock()?;
        let transaction =
            connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        claim.verify_operation(&transaction, "index_file", Some(&prepared.target_json))?;
        prepared
            .target
            .authorization
            .verify_locator(&transaction, &prepared.target.locator)?;
        let current = canonical_file_snapshot(&transaction, &prepared.target.locator)?;
        prepared.target.verify_artifact_identity(&current)?;
        if current != prepared.snapshot {
            return Err(LoomError::SourceChanged(prepared.target.locator.clone()));
        }
        // Revalidate only after obtaining the writer lock: a competing writer may have held it
        // while the user edited the file. This is a bounded byte read, not another extraction.
        ingest::verify_stable_hash(
            Path::new(&prepared.target.locator),
            self.limits.max_file_bytes.min(8 * 1024 * 1024),
            &prepared.document.document.raw_hash,
        )?;
        let indexed = Self::commit_index_document(
            &transaction,
            &prepared.target.authorization,
            &prepared.document,
            None,
        )?;
        let snapshot = canonical_file_snapshot(&transaction, &prepared.target.locator)?
            .ok_or_else(|| LoomError::JobQueue("published file has no canonical record".into()))?;
        let result = serde_json::to_string(&serde_json::json!({ "indexed": indexed,
            "artifact_id": snapshot.artifact_id, "version_id": snapshot.version_id,
            "content_hash": prepared.document.document.raw_hash,
            "extraction": prepared.extraction_metrics }))?;
        let completed = claim.complete(&transaction, &result)?;
        transaction.commit()?;
        Ok(completed)
    }

    fn finish_deletion(&self, report: DeletionReport) -> Result<DeletionReport> {
        let connection = self.lock()?;
        rebuild_fts(&connection)?;
        checkpoint_and_vacuum(&connection)?;
        Ok(report)
    }

    fn source_modified_ns(&self, version_id: &str) -> Result<Option<i64>> {
        let connection = self.lock()?;
        connection
            .query_row(
                "SELECT source_modified_ns FROM artifact_versions WHERE id = ?1",
                [version_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(Into::into)
    }
}

#[cfg(unix)]
pub(crate) fn database_file_identity(path: &Path) -> Result<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    let metadata = fs::metadata(path).map_err(|error| io_error(path, error))?;
    Ok((metadata.dev(), metadata.ino()))
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

struct PreparedIndexDocument {
    source_uri: String,
    title: String,
    document: StableDocument,
    passages: Vec<PassageDraft>,
    extractor_id: String,
    extractor_version: String,
    parse_warnings_json: String,
    extraction_metadata_json: String,
    now: String,
}

impl PreparedIndexDocument {
    fn new(
        path: &Path,
        document: StableDocument,
        limits: LibraryLimits,
        extractor_id: &str,
        extractor_version: &str,
    ) -> Result<Self> {
        let source_uri = utf8_path(path)?;
        let title = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or(&source_uri)
            .to_owned();
        let passages = if let Some(regions) = document.image_regions.as_deref() {
            ingest::split_image_passages(regions)
        } else if let Some(pages) = document.pdf_pages.as_deref() {
            ingest::split_pdf_passages(
                pages,
                limits.passage_target_chars,
                limits.passage_overlap_chars,
            )
        } else {
            ingest::split_passages(
                &document.normalized_text,
                limits.passage_target_chars,
                limits.passage_overlap_chars,
            )
        };
        let parse_warnings_json = serde_json::to_string(&document.parse_warnings)?;
        let extraction_metadata_json = serde_json::to_string(&document.extraction_metadata)?;
        Ok(Self {
            source_uri,
            title,
            document,
            passages,
            extractor_id: extractor_id.to_owned(),
            extractor_version: extractor_version.to_owned(),
            parse_warnings_json,
            extraction_metadata_json,
            now: Utc::now().to_rfc3339(),
        })
    }
}

/// Captured before extraction, not admission: an independent foreground writer can update a
/// selected file without changing its capability generation. Compare again inside publication.
#[derive(Debug, PartialEq, Eq)]
struct CanonicalFileSnapshot {
    artifact_id: String,
    root_id: String,
    version_id: Option<String>,
    state: String,
    locator_active: bool,
    last_seen_at: String,
}

fn canonical_file_snapshot(
    connection: &Connection,
    locator: &str,
) -> Result<Option<CanonicalFileSnapshot>> {
    connection
        .query_row(
            "SELECT a.id, a.source_root_id, a.active_version_id, a.state, l.active, a.last_seen_at
         FROM artifact_locators l JOIN artifacts a ON a.id=l.artifact_id
         WHERE l.kind='file' AND l.locator=?1",
            [locator],
            |row| {
                Ok(CanonicalFileSnapshot {
                    artifact_id: row.get(0)?,
                    root_id: row.get(1)?,
                    version_id: row.get(2)?,
                    state: row.get(3)?,
                    locator_active: row.get(4)?,
                    last_seen_at: row.get(5)?,
                })
            },
        )
        .optional()
        .map_err(Into::into)
}

pub(crate) struct PreparedFileJob {
    target: IndexFileTarget,
    target_json: String,
    snapshot: Option<CanonicalFileSnapshot>,
    document: PreparedIndexDocument,
    extraction_metrics: loom_extraction::ExtractionMetrics,
}

fn enforce_file_job_output_bounds(document: &StableDocument) -> Result<()> {
    const TEXT_LIMIT: usize = 2 * 1024 * 1024;
    let valid = document.normalized_text.len() <= TEXT_LIMIT
        && document.pdf_pages.as_ref().is_none_or(|pages| {
            pages.len() <= 2048
                && pages.iter().map(|(_, text)| text.len()).sum::<usize>() <= TEXT_LIMIT
        })
        && document.image_regions.as_ref().is_none_or(|regions| {
            regions.len() <= 8192
                && regions
                    .iter()
                    .map(|region| region.text.len())
                    .sum::<usize>()
                    <= TEXT_LIMIT
        })
        && document.parse_warnings.len() <= 128;
    // Count serialization bytes without allocating an unbounded JSON string.
    struct Budget(usize);
    impl std::io::Write for Budget {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > self.0 {
                return Err(std::io::Error::other("output budget exceeded"));
            }
            self.0 -= bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    if !valid
        || serde_json::to_writer(Budget(16_384), &document.parse_warnings).is_err()
        || serde_json::to_writer(Budget(65_536), &document.extraction_metadata).is_err()
    {
        return Err(LoomError::JobQueue(
            "queued extractor output exceeds its publication budget".into(),
        ));
    }
    Ok(())
}

/// Consent belongs to one generation of one exact selected root, not to a locator forever.
/// Verify inside the same SQLite transaction as every resulting write; a process-local mutex
/// cannot fence a different desktop/CLI connection.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceAuthorization {
    root_id: String,
    generation: i64,
    incarnation: String,
    kind: String,
    // Only scans containing images depend on OCR policy. This also fences their cleanup,
    // diagnostic progress, and failure writes, not just a successful provider result.
    ocr_policy: Option<OcrPolicy>,
}

/// Only an exact selected file capability is executable; no directory discovery or source grant.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct IndexFileTarget {
    pub(crate) locator: String,
    media_type: String,
    authorization: SourceAuthorization,
    // Explicit null means admission saw no artifact. A missing field must not acquire that
    // meaning after a later deletion. deserialize_with makes the field required in Serde.
    #[serde(deserialize_with = "required_artifact_identity")]
    artifact_id: Option<String>,
}

fn required_artifact_identity<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    serde::Deserialize::deserialize(deserializer)
}

pub(crate) fn canonical_queue_file(path: &Path, max_bytes: u64) -> Result<String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| io_error(path, error))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > max_bytes {
        return Err(LoomError::InvalidPath(
            "queued input must be one bounded regular file".into(),
        ));
    }
    let canonical = path.canonicalize().map_err(|error| io_error(path, error))?;
    if ingest::supported_media_type(&canonical).is_none() {
        return Err(LoomError::UnsupportedSource(
            canonical.display().to_string(),
        ));
    }
    let locator = utf8_path(&canonical)?;
    if locator.len() > 4096 || locator.chars().any(char::is_control) {
        return Err(LoomError::InvalidPath(
            "queued locator exceeds its supported bounds".into(),
        ));
    }
    Ok(locator)
}

impl IndexFileTarget {
    pub(crate) fn capture(connection: &Connection, locator: &str) -> Result<Self> {
        let authorization = connection
            .query_row(
                "SELECT id, scope_generation,
                (SELECT value FROM schema_meta WHERE key = 'authorization_incarnation'), kind
             FROM source_roots WHERE locator = ?1 AND kind = 'file' AND enabled = 1",
                [locator],
                |row| {
                    Ok(SourceAuthorization {
                        root_id: row.get(0)?,
                        generation: row.get(1)?,
                        incarnation: row.get(2)?,
                        kind: row.get(3)?,
                        ocr_policy: None,
                    })
                },
            )
            .optional()?
            .ok_or_else(|| LoomError::SourceRevoked(locator.to_owned()))?;
        let media_type = ingest::supported_media_type(Path::new(locator))
            .ok_or_else(|| LoomError::UnsupportedSource(locator.to_owned()))?
            .to_owned();
        let mut target = Self {
            locator: locator.to_owned(),
            media_type,
            authorization,
            artifact_id: canonical_file_snapshot(connection, locator)?
                .map(|snapshot| snapshot.artifact_id),
        };
        if target.media_type.starts_with("image/") {
            let policy = OcrPolicy::load(connection)?;
            if !policy.enabled {
                return Err(LoomError::OcrDisabled);
            }
            target.authorization.ocr_policy = Some(policy);
        }
        target.validate()?;
        target.authorization.verify_locator(connection, locator)?;
        Ok(target)
    }

    pub(crate) fn parse(json: &str) -> Result<Self> {
        if json.len() > 16_384 {
            return Err(LoomError::JobQueue("file target exceeds 16 KiB".into()));
        }
        let target: Self = serde_json::from_str(json)
            .map_err(|_| LoomError::JobQueue("invalid typed file target".into()))?;
        target.validate()?;
        Ok(target)
    }

    fn validate(&self) -> Result<()> {
        let auth = &self.authorization;
        let path = Path::new(&self.locator);
        let valid = self.locator.len() <= 4096
            && !self.locator.chars().any(char::is_control)
            && path.is_absolute()
            && !path.components().any(|part| {
                matches!(
                    part,
                    std::path::Component::ParentDir | std::path::Component::CurDir
                )
            })
            && ingest::supported_media_type(path) == Some(self.media_type.as_str())
            && auth.kind == "file"
            && auth.generation >= 0
            && Uuid::parse_str(&auth.root_id).is_ok()
            && Uuid::parse_str(&auth.incarnation).is_ok()
            && self
                .artifact_id
                .as_ref()
                .is_none_or(|id| Uuid::parse_str(id).is_ok())
            && if self.media_type.starts_with("image/") {
                auth.ocr_policy.as_ref().is_some_and(|policy| {
                    policy.enabled && Uuid::parse_str(&policy.revision).is_ok()
                })
            } else {
                auth.ocr_policy.is_none()
            };
        if !valid {
            return Err(LoomError::JobQueue("invalid file capability shape".into()));
        }
        Ok(())
    }

    fn verify_artifact_identity(&self, snapshot: &Option<CanonicalFileSnapshot>) -> Result<()> {
        if snapshot.as_ref().map(|record| record.artifact_id.as_str())
            != self.artifact_id.as_deref()
        {
            // An older binary may purge canonical evidence without knowing this runtime. A
            // retry must not convert that old admission into permission to create a new artifact.
            return Err(LoomError::SourceRevoked(self.authorization.root_id.clone()));
        }
        Ok(())
    }

    /// Creation from a previously empty selected root changes artifact identity as this job's
    /// own output. Repeating that completed request must still return the original result.
    pub(crate) fn same_completed_request(
        &self,
        original: &Self,
        job: &crate::BackgroundJob,
    ) -> bool {
        if original.artifact_id.is_some()
            || job.state != crate::JobState::Completed
            || self.artifact_id.is_none()
            || job
                .result
                .as_ref()
                .and_then(|result| result.get("artifact_id"))
                .and_then(serde_json::Value::as_str)
                != self.artifact_id.as_deref()
        {
            return false;
        }
        let mut normalized = self.clone();
        normalized.artifact_id = None;
        normalized == *original
    }
}

impl SourceAuthorization {
    fn verify_persisted_locator(&self, connection: &Connection, locator: &str) -> Result<()> {
        let valid: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM source_roots
             WHERE id=?1 AND locator=?2 AND enabled=1 AND scope_generation=?3 AND kind=?4
             AND (SELECT value FROM schema_meta WHERE key='authorization_incarnation')=?5)",
            params![
                self.root_id,
                locator,
                self.generation,
                self.kind,
                self.incarnation
            ],
            |row| row.get(0),
        )?;
        if !valid {
            return Err(LoomError::SourceRevoked(self.root_id.clone()));
        }
        if let Some(policy) = &self.ocr_policy {
            policy.verify_current(connection)?;
        }
        Ok(())
    }
    fn verify(&self, connection: &Connection) -> Result<()> {
        let locator: Option<String> = connection.query_row(
            "SELECT locator FROM source_roots
                WHERE id = ?1 AND enabled = 1 AND scope_generation = ?2 AND kind = ?4
                    AND (SELECT value FROM schema_meta WHERE key = 'authorization_incarnation') = ?3",
            params![self.root_id, self.generation, self.incarnation, self.kind],
            |row| row.get(0),
        ).optional()?;
        let locator = locator.ok_or_else(|| LoomError::SourceRevoked(self.root_id.clone()))?;
        // Source shape is part of the selected capability. A replacement directory/file or
        // symlink is not the same scope, even before a controller updates its persisted row.
        if source_root_status(&locator, &self.kind, true) != SourceRootStatus::Available {
            return Err(LoomError::SourceRevoked(self.root_id.clone()));
        }
        if let Some(policy) = &self.ocr_policy {
            policy.verify_current(connection)?;
        }
        Ok(())
    }

    fn verify_ocr_result(&self, connection: &Connection) -> Result<()> {
        let policy = self.ocr_policy.as_ref().ok_or_else(|| {
            LoomError::OcrUnavailable("prepared OCR result has no captured policy".into())
        })?;
        policy.verify_current(connection)?;
        if !policy.enabled {
            return Err(LoomError::OcrDisabled);
        }
        Ok(())
    }

    fn verify_locator(&self, connection: &Connection, locator: &str) -> Result<()> {
        self.verify(connection)?;
        let same_locator: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM source_roots WHERE id = ?1 AND locator = ?2)",
            params![self.root_id, locator],
            |row| row.get(0),
        )?;
        if !same_locator {
            return Err(LoomError::SourceRevoked(self.root_id.clone()));
        }
        Ok(())
    }
}

fn update_index_job_checkpoint(
    transaction: &Transaction<'_>,
    authorization: &SourceAuthorization,
    job_id: &str,
    next_unit: u64,
    now: &str,
) -> Result<()> {
    let updated = transaction.execute(
        "UPDATE index_jobs
         SET next_unit = ?1, updated_at = ?2
         WHERE id = ?3 AND source_root_id = ?4 AND state = 'running'",
        params![
            sql_i64(next_unit, "index job progress")?,
            now,
            job_id,
            authorization.root_id
        ],
    )?;
    require_job_update(updated, job_id)
}

fn require_job_update(updated: usize, job_id: &str) -> Result<()> {
    if updated != 1 {
        return Err(LoomError::IndexJobStale(job_id.to_owned()));
    }
    Ok(())
}

/// A foreign-key-valid archive can still cross-link bookmark consent between different roots.
pub(crate) fn validate_bookmark_scope_consistency(connection: &Connection) -> Result<()> {
    let inconsistent: bool = connection.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM bookmark_imports i JOIN source_roots r ON r.id = i.source_root_id
            WHERE i.source_locator <> r.locator OR r.kind <> 'file'
         ) OR EXISTS(
            SELECT 1 FROM bookmark_records b
            JOIN bookmark_imports i ON i.id = b.first_import_id
            JOIN artifacts a ON a.id = b.artifact_id
            WHERE i.source_root_id <> a.source_root_id
         ) OR EXISTS(
            SELECT 1 FROM bookmark_import_items item
            JOIN bookmark_imports i ON i.id = item.import_id
            JOIN bookmark_records b ON b.id = item.bookmark_id
            JOIN artifacts a ON a.id = b.artifact_id
            WHERE i.source_root_id <> a.source_root_id
         ) OR EXISTS(
            SELECT 1 FROM bookmark_import_failures f
            JOIN bookmark_imports i ON i.id = f.import_id
            JOIN bookmark_imports resolved ON resolved.id = f.resolved_by_import_id
            WHERE i.source_root_id <> resolved.source_root_id
         )",
        [],
        |row| row.get(0),
    )?;
    if inconsistent {
        return Err(LoomError::PortableExport(
            "bookmark scope ownership mismatch".into(),
        ));
    }
    Ok(())
}

fn discovery_fingerprint(
    paths: &[PathBuf],
    ocr_policy: Option<&OcrPolicy>,
    max_files: usize,
) -> String {
    let mut hasher = blake3::Hasher::new();
    crate::discovery::DiscoveryLimits::for_files(max_files).fingerprint(&mut hasher);
    for path in paths {
        let bytes = path.as_os_str().as_encoded_bytes();
        hasher.update(&(bytes.len() as u64).to_le_bytes());
        hasher.update(bytes);
    }
    if let Some(policy) = ocr_policy {
        hasher.update(b"\0ocr-policy\0");
        hasher.update(if policy.enabled { b"1" } else { b"0" });
        hasher.update(policy.revision.as_bytes());
    }
    format!("blake3:{}", hasher.finalize().to_hex())
}

fn validate_relationship_id(value: &str, label: &str) -> Result<()> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.len() > 256 || trimmed.chars().any(char::is_control) {
        return Err(LoomError::InvalidPath(format!(
            "{label} relationship identifier is empty, too long, or contains control characters"
        )));
    }
    Ok(())
}

fn validate_relationship_input(input: &RelationshipInput) -> Result<String> {
    validate_relationship_id(&input.source_artifact_id, "source artifact")?;
    validate_relationship_id(&input.target_artifact_id, "target artifact")?;
    if input.source_artifact_id.trim() == input.target_artifact_id.trim() {
        return Err(LoomError::InvalidPath(
            "relationship endpoints must be different artifacts".into(),
        ));
    }
    let kind = input.kind.as_str();
    if kind.is_empty()
        || kind.len() > 128
        || kind
            .chars()
            .any(|value| value.is_control() || value.is_whitespace())
    {
        return Err(LoomError::InvalidPath(
            "relationship kind is empty, too long, or contains whitespace".into(),
        ));
    }
    if input.method.trim().is_empty()
        || input.method.trim().len() > 256
        || input.method.chars().any(char::is_control)
    {
        return Err(LoomError::InvalidPath(
            "relationship method is empty, too long, or contains control characters".into(),
        ));
    }
    if input.origin == RelationshipOrigin::Inferred
        && (input.evidence_passage_id.is_none() || input.confidence.is_none())
    {
        return Err(LoomError::InvalidPath(
            "inferred relationships require evidence and confidence".into(),
        ));
    }
    if let Some(confidence) = input.confidence {
        if !confidence.is_finite() || !(0.0..=1.0).contains(&confidence) {
            return Err(LoomError::InvalidPath(
                "relationship confidence must be finite and between 0 and 1".into(),
            ));
        }
    }
    if !input.metadata.is_object() {
        return Err(LoomError::InvalidPath(
            "relationship metadata must be a JSON object".into(),
        ));
    }
    let metadata_json = serde_json::to_string(&input.metadata)?;
    if metadata_json.len() > 16 * 1024 {
        return Err(LoomError::InvalidPath(
            "relationship metadata exceeds the 16 KiB limit".into(),
        ));
    }
    if let Some(passage_id) = input.evidence_passage_id.as_deref() {
        validate_relationship_id(passage_id, "evidence passage")?;
    }
    Ok(metadata_json)
}

fn relationship_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RelationshipRecord> {
    let origin: String = row.get(8)?;
    let origin = match origin.as_str() {
        "observed" => RelationshipOrigin::Observed,
        "inferred" => RelationshipOrigin::Inferred,
        "user_confirmed" => RelationshipOrigin::UserConfirmed,
        _ => {
            return Err(rusqlite::Error::FromSqlConversionFailure(
                8,
                rusqlite::types::Type::Text,
                Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "unknown relationship origin",
                )),
            ))
        }
    };
    let metadata_json: String = row.get(9)?;
    let metadata = serde_json::from_str(&metadata_json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(9, rusqlite::types::Type::Text, Box::new(error))
    })?;
    Ok(RelationshipRecord {
        id: row.get(0)?,
        source_artifact_id: row.get(1)?,
        target_artifact_id: row.get(2)?,
        kind: RelationshipKind::from_value(row.get::<_, String>(3)?),
        evidence_passage_id: row.get(4)?,
        confidence: row.get(5)?,
        method: row.get(6)?,
        schema_version: row.get::<_, i64>(7)?.max(1) as u32,
        origin,
        metadata,
        created_at: row.get(10)?,
    })
}

fn relationship_by_id(connection: &Connection, id: &str) -> Result<Option<RelationshipRecord>> {
    connection
        .query_row(
            "SELECT id, source_artifact_id, target_artifact_id, kind, evidence_passage_id,
                    confidence, method, relationship_schema_version, origin, metadata_json,
                    created_at
             FROM relationships WHERE id = ?1",
            [id],
            relationship_from_row,
        )
        .optional()
        .map_err(Into::into)
}

fn relationship_endpoint(
    connection: &Connection,
    artifact_id: &str,
) -> Result<RelationshipEndpoint> {
    connection
        .query_row(
            "SELECT a.id, a.title, a.media_type, l.locator, v.id, v.content_hash, a.state
             FROM artifacts a
             LEFT JOIN artifact_locators l ON l.artifact_id = a.id AND l.active = 1
             LEFT JOIN artifact_versions v ON v.id = a.active_version_id
             WHERE a.id = ?1",
            [artifact_id],
            |row| {
                Ok(RelationshipEndpoint {
                    artifact_id: row.get(0)?,
                    title: row.get(1)?,
                    media_type: row.get(2)?,
                    source_uri: row.get(3)?,
                    version_id: row.get(4)?,
                    content_hash: row.get(5)?,
                    state: row.get(6)?,
                })
            },
        )
        .optional()?
        .ok_or_else(|| LoomError::ArtifactNotFound(artifact_id.to_string()))
}

fn ensure_relationship_column(
    transaction: &Transaction<'_>,
    column: &str,
    definition: &str,
) -> Result<()> {
    ensure_column(transaction, "relationships", column, definition)
}

fn ensure_column(
    transaction: &Transaction<'_>,
    table: &str,
    column: &str,
    definition: &str,
) -> Result<()> {
    let exists: bool = transaction.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM pragma_table_info(?1) WHERE name = ?2
         )",
        [table, column],
        |row| row.get(0),
    )?;
    if !exists {
        let statement = format!("ALTER TABLE {table} ADD COLUMN {column} {definition}");
        transaction.execute(&statement, [])?;
    }
    Ok(())
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

fn ensure_semantic_schema(connection: &Connection) -> Result<()> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS semantic_index_meta(
            slot INTEGER PRIMARY KEY CHECK(slot = 1),
            provider_id TEXT NOT NULL,
            model_id TEXT NOT NULL,
            tokenizer TEXT NOT NULL,
            dimension INTEGER NOT NULL CHECK(dimension > 0),
            normalization TEXT NOT NULL,
            build_parameters TEXT NOT NULL,
            index_revision TEXT NOT NULL,
            source_digest TEXT NOT NULL,
            canonical_passages INTEGER NOT NULL CHECK(canonical_passages >= 0),
            indexed_passages INTEGER NOT NULL CHECK(indexed_passages >= 0),
            vector_bytes INTEGER NOT NULL CHECK(vector_bytes >= 0),
            built_at TEXT NOT NULL
         ) STRICT;

         CREATE TABLE IF NOT EXISTS semantic_embeddings(
            passage_id TEXT PRIMARY KEY REFERENCES passages(id) ON DELETE CASCADE,
            passage_hash TEXT NOT NULL,
            provider_id TEXT NOT NULL,
            model_id TEXT NOT NULL,
            tokenizer TEXT NOT NULL,
            dimension INTEGER NOT NULL CHECK(dimension > 0),
            normalization TEXT NOT NULL,
            build_parameters TEXT NOT NULL,
            index_revision TEXT NOT NULL,
            vector_blob BLOB NOT NULL,
            created_at TEXT NOT NULL
         ) STRICT;

         CREATE INDEX IF NOT EXISTS semantic_embeddings_revision
           ON semantic_embeddings(index_revision, passage_id);",
    )?;
    ensure_semantic_column(
        connection,
        "semantic_index_meta",
        "tokenizer",
        "TEXT NOT NULL DEFAULT 'unicode-alnum-lower-v1'",
    )?;
    ensure_semantic_column(
        connection,
        "semantic_index_meta",
        "build_parameters",
        "TEXT NOT NULL DEFAULT 'hash-token=1.0;hash-bigram=0.5;vector=float32-le-v1'",
    )?;
    ensure_semantic_column(
        connection,
        "semantic_embeddings",
        "tokenizer",
        "TEXT NOT NULL DEFAULT 'unicode-alnum-lower-v1'",
    )?;
    ensure_semantic_column(
        connection,
        "semantic_embeddings",
        "build_parameters",
        "TEXT NOT NULL DEFAULT 'hash-token=1.0;hash-bigram=0.5;vector=float32-le-v1'",
    )?;
    Ok(())
}

fn ensure_semantic_column(
    connection: &Connection,
    table: &str,
    column: &str,
    definition: &str,
) -> Result<()> {
    let exists: bool = connection.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM pragma_table_info(?1) WHERE name = ?2
         )",
        params![table, column],
        |row| row.get(0),
    )?;
    if !exists {
        let statement = format!("ALTER TABLE {table} ADD COLUMN {column} {definition}");
        connection.execute(&statement, [])?;
    }
    Ok(())
}

fn canonical_semantic_source(connection: &Connection) -> Result<(u64, String)> {
    let mut statement = connection.prepare(
        "SELECT p.id, p.text_hash
         FROM passages p
         JOIN artifact_versions v ON v.id = p.artifact_version_id
         JOIN artifacts a ON a.id = v.artifact_id AND a.active_version_id = v.id
         JOIN source_roots r ON r.id = a.source_root_id AND r.enabled = 1
         WHERE a.state = 'active'
         ORDER BY p.id",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut count = 0_u64;
    let mut hasher = blake3::Hasher::new();
    for row in rows {
        let (passage_id, passage_hash) = row?;
        hasher.update(passage_id.as_bytes());
        hasher.update(&[0]);
        hasher.update(passage_hash.as_bytes());
        hasher.update(&[0]);
        count = count.saturating_add(1);
    }
    Ok((count, format!("blake3:{}", hasher.finalize().to_hex())))
}

fn nonnegative_u64(value: i64) -> u64 {
    value.max(0) as u64
}

fn migrate(connection: &mut Connection) -> Result<()> {
    let existing_version = stored_schema_version(connection)?;
    if let Some(version) = existing_version {
        if !matches!(
            version,
            SCHEMA_VERSION
                | GRAPH_BOUNDS_SCHEMA_VERSION
                | CONNECTOR_SCHEMA_VERSION
                | BOOKMARK_SCHEMA_VERSION
                | RELATIONSHIP_SCHEMA_VERSION
                | PREVIOUS_SCHEMA_VERSION
                | PREVIOUS_PREVIOUS_SCHEMA_VERSION
                | V3_SCHEMA_VERSION
                | LEGACY_SCHEMA_VERSION
        ) {
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
            scope_generation INTEGER NOT NULL DEFAULT 0 CHECK(scope_generation >= 0),
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
            parse_warnings_json TEXT NOT NULL DEFAULT '[]'
              CHECK(json_valid(parse_warnings_json)),
            page_count INTEGER CHECK(page_count IS NULL OR page_count >= 0),
            extraction_metadata_json TEXT NOT NULL DEFAULT '{}'
              CHECK(json_valid(extraction_metadata_json)),
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
            relationship_schema_version INTEGER NOT NULL DEFAULT 1
              CHECK(relationship_schema_version >= 1),
            origin TEXT NOT NULL DEFAULT 'observed'
              CHECK(origin IN ('observed', 'inferred', 'user_confirmed')),
            metadata_json TEXT NOT NULL DEFAULT '{}'
              CHECK(json_valid(metadata_json)),
            created_at TEXT NOT NULL,
            CHECK(source_artifact_id <> target_artifact_id),
            CHECK(confidence IS NULL OR (confidence >= 0.0 AND confidence <= 1.0))
         ) STRICT;

         CREATE TABLE IF NOT EXISTS bookmark_imports(
            id TEXT PRIMARY KEY,
            source_root_id TEXT NOT NULL REFERENCES source_roots(id) ON DELETE CASCADE,
            source_locator TEXT NOT NULL,
            format TEXT NOT NULL,
            content_hash TEXT NOT NULL,
            imported_at TEXT NOT NULL,
            source_application TEXT NOT NULL DEFAULT 'unknown',
            export_version TEXT NOT NULL DEFAULT 'unknown',
            permissions_json TEXT NOT NULL DEFAULT '[\"read_selected_file\"]'
              CHECK(json_valid(permissions_json)),
            skipped_fields_json TEXT NOT NULL DEFAULT '[]'
              CHECK(json_valid(skipped_fields_json)),
            status TEXT NOT NULL DEFAULT 'complete'
              CHECK(status IN ('complete', 'partial', 'revoked')),
            UNIQUE(source_locator, format, content_hash)
         ) STRICT;

         CREATE TABLE IF NOT EXISTS bookmark_import_failures(
            import_id TEXT NOT NULL REFERENCES bookmark_imports(id) ON DELETE CASCADE,
            ordinal INTEGER NOT NULL CHECK(ordinal >= 0),
            byte_offset INTEGER NOT NULL CHECK(byte_offset >= 0),
            code TEXT NOT NULL,
            detail TEXT NOT NULL,
            state TEXT NOT NULL DEFAULT 'pending' CHECK(state IN ('pending', 'resolved')),
            resolved_by_import_id TEXT REFERENCES bookmark_imports(id) ON DELETE SET NULL,
            created_at TEXT NOT NULL,
            PRIMARY KEY(import_id, ordinal)
         ) STRICT;

         CREATE TABLE IF NOT EXISTS bookmark_records(
            id TEXT PRIMARY KEY,
            artifact_id TEXT NOT NULL REFERENCES artifacts(id) ON DELETE CASCADE,
            folder_path TEXT NOT NULL,
            title TEXT NOT NULL,
            url TEXT NOT NULL,
            added_at TEXT,
            modified_at TEXT,
            entry_hash TEXT NOT NULL,
            first_import_id TEXT NOT NULL REFERENCES bookmark_imports(id) ON DELETE CASCADE,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            UNIQUE(url, folder_path)
         ) STRICT;
         CREATE INDEX IF NOT EXISTS bookmark_records_url_idx ON bookmark_records(url);

         CREATE TABLE IF NOT EXISTS bookmark_import_items(
            import_id TEXT NOT NULL REFERENCES bookmark_imports(id) ON DELETE CASCADE,
            bookmark_id TEXT NOT NULL REFERENCES bookmark_records(id) ON DELETE CASCADE,
            ordinal INTEGER NOT NULL CHECK(ordinal >= 0),
            entry_hash TEXT NOT NULL,
            outcome TEXT NOT NULL CHECK(outcome IN ('imported', 'unchanged', 'merged', 'conflict')),
            PRIMARY KEY(import_id, bookmark_id, ordinal)
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
    if existing_version.is_some_and(|version| version < SCHEMA_VERSION) {
        ensure_column(
            &transaction,
            "source_roots",
            "scope_generation",
            "INTEGER NOT NULL DEFAULT 0 CHECK(scope_generation >= 0)",
        )?;
    }
    if existing_version
        .is_some_and(|version| version == V3_SCHEMA_VERSION || version == LEGACY_SCHEMA_VERSION)
    {
        transaction.execute_batch(
            "ALTER TABLE artifact_versions
                ADD COLUMN parse_warnings_json TEXT NOT NULL DEFAULT '[]'
                  CHECK(json_valid(parse_warnings_json));
             ALTER TABLE artifact_versions
                ADD COLUMN page_count INTEGER CHECK(page_count IS NULL OR page_count >= 0);",
        )?;
    }
    if existing_version.is_some_and(|version| version < PREVIOUS_SCHEMA_VERSION) {
        transaction.execute_batch(
            "ALTER TABLE artifact_versions
                ADD COLUMN extraction_metadata_json TEXT NOT NULL DEFAULT '{}'
                  CHECK(json_valid(extraction_metadata_json));",
        )?;
    }
    if existing_version.is_some_and(|version| version == BOOKMARK_SCHEMA_VERSION) {
        for (column, definition) in [
            ("source_application", "TEXT NOT NULL DEFAULT 'unknown'"),
            ("export_version", "TEXT NOT NULL DEFAULT 'unknown'"),
            (
                "permissions_json",
                "TEXT NOT NULL DEFAULT '[\"read_selected_file\"]' CHECK(json_valid(permissions_json))",
            ),
            (
                "skipped_fields_json",
                "TEXT NOT NULL DEFAULT '[]' CHECK(json_valid(skipped_fields_json))",
            ),
            (
                "status",
                "TEXT NOT NULL DEFAULT 'complete' CHECK(status IN ('complete', 'partial', 'revoked'))",
            ),
        ] {
            ensure_column(&transaction, "bookmark_imports", column, definition)?;
        }
    }
    if existing_version.is_some_and(|version| version < RELATIONSHIP_SCHEMA_VERSION) {
        ensure_relationship_column(
            &transaction,
            "relationship_schema_version",
            "INTEGER NOT NULL DEFAULT 1",
        )?;
        ensure_relationship_column(&transaction, "origin", "TEXT NOT NULL DEFAULT 'observed'")?;
        ensure_relationship_column(
            &transaction,
            "metadata_json",
            "TEXT NOT NULL DEFAULT '{}' CHECK(json_valid(metadata_json))",
        )?;
    }
    // Provenance graph bounds (schema 9). Created after the relationship columns exist on every
    // migrated database. Evidence stays an insert-time rule in `add_relationship` because purging
    // an evidence passage legitimately clears it afterwards.
    transaction.execute_batch(
        "CREATE INDEX IF NOT EXISTS relationships_source_idx
           ON relationships(source_artifact_id, created_at, id);
         CREATE INDEX IF NOT EXISTS relationships_target_idx
           ON relationships(target_artifact_id, created_at, id);
         CREATE INDEX IF NOT EXISTS relationships_edge_idx
           ON relationships(source_artifact_id, target_artifact_id, kind);
         CREATE TRIGGER IF NOT EXISTS relationships_envelope_insert
         BEFORE INSERT ON relationships
         WHEN length(trim(new.kind)) = 0 OR length(trim(new.method)) = 0
           OR (new.origin = 'inferred' AND new.confidence IS NULL)
         BEGIN
            SELECT RAISE(ABORT, 'relationship envelope violates schema checks');
         END;
         CREATE TRIGGER IF NOT EXISTS relationships_envelope_update
         BEFORE UPDATE OF kind, method, origin, confidence ON relationships
         WHEN length(trim(new.kind)) = 0 OR length(trim(new.method)) = 0
           OR (new.origin = 'inferred' AND new.confidence IS NULL)
         BEGIN
            SELECT RAISE(ABORT, 'relationship envelope violates schema checks');
         END;
         CREATE TABLE IF NOT EXISTS relationship_compactions(
            id TEXT PRIMARY KEY,
            compacted_at TEXT NOT NULL,
            removed_count INTEGER NOT NULL CHECK(removed_count > 0),
            removed_digest TEXT NOT NULL,
            summary_json TEXT NOT NULL CHECK(json_valid(summary_json))
         ) STRICT;",
    )?;
    transaction.execute(
        "INSERT INTO schema_meta(key, value) VALUES ('schema_version', ?1)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [SCHEMA_VERSION.to_string()],
    )?;
    transaction.execute(
        "INSERT INTO schema_meta(key, value) VALUES ('ocr_enabled', '1')
         ON CONFLICT(key) DO NOTHING",
        [],
    )?;
    transaction.execute(
        "INSERT INTO schema_meta(key, value) VALUES ('authorization_incarnation', ?1)
         ON CONFLICT(key) DO NOTHING",
        [Uuid::new_v4().to_string()],
    )?;
    transaction.execute(
        "INSERT INTO schema_meta(key, value) VALUES ('ocr_policy_revision', ?1)
         ON CONFLICT(key) DO NOTHING",
        [Uuid::new_v4().to_string()],
    )?;
    transaction.commit()?;
    connection.execute_batch("PRAGMA trusted_schema = OFF;")?;
    rebuild_fts(connection)?;
    Ok(())
}

fn validate_schema_shape(connection: &Connection, version: i64) -> Result<()> {
    let tables = if version >= BOOKMARK_SCHEMA_VERSION {
        CURRENT_SCHEMA_TABLES
    } else if version != LEGACY_SCHEMA_VERSION {
        PRE_BOOKMARK_SCHEMA_TABLES
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
    .chain(
        (version >= PREVIOUS_PREVIOUS_SCHEMA_VERSION)
            .then_some(("artifact_versions", "parse_warnings_json")),
    )
    .chain(
        (version >= PREVIOUS_PREVIOUS_SCHEMA_VERSION)
            .then_some(("artifact_versions", "page_count")),
    )
    .chain(
        (version >= PREVIOUS_SCHEMA_VERSION)
            .then_some(("artifact_versions", "extraction_metadata_json")),
    )
    .chain(
        (version >= RELATIONSHIP_SCHEMA_VERSION)
            .then_some(("relationships", "relationship_schema_version")),
    )
    .chain((version >= RELATIONSHIP_SCHEMA_VERSION).then_some(("relationships", "origin")))
    .chain((version >= RELATIONSHIP_SCHEMA_VERSION).then_some(("relationships", "metadata_json")))
    .chain((version >= GRAPH_BOUNDS_SCHEMA_VERSION).then_some(("bookmark_imports", "id")))
    .chain((version >= GRAPH_BOUNDS_SCHEMA_VERSION).then_some(("bookmark_records", "id")))
    .chain(
        (version >= GRAPH_BOUNDS_SCHEMA_VERSION).then_some(("bookmark_import_items", "import_id")),
    )
    .chain((version >= SCHEMA_VERSION).then_some(("source_roots", "scope_generation")))
    .chain((version != LEGACY_SCHEMA_VERSION).then_some(("index_jobs", "id")))
    .chain((version != LEGACY_SCHEMA_VERSION).then_some(("index_jobs", "source_root_id")))
    .chain((version != LEGACY_SCHEMA_VERSION).then_some(("index_jobs", "selection_locator")))
    .chain((version != LEGACY_SCHEMA_VERSION).then_some(("index_jobs", "discovery_fingerprint")))
    .chain((version != LEGACY_SCHEMA_VERSION).then_some(("index_jobs", "total_units")))
    .chain((version != LEGACY_SCHEMA_VERSION).then_some(("index_jobs", "next_unit")))
    .chain((version != LEGACY_SCHEMA_VERSION).then_some(("index_jobs", "state")))
    .chain((version != LEGACY_SCHEMA_VERSION).then_some(("index_jobs", "last_error")))
    .chain((version != LEGACY_SCHEMA_VERSION).then_some(("index_jobs", "started_at")))
    .chain((version != LEGACY_SCHEMA_VERSION).then_some(("index_jobs", "updated_at")))
    .chain((version != LEGACY_SCHEMA_VERSION).then_some(("index_jobs", "completed_at")))
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
) -> Result<SourceAuthorization> {
    let kind = if directory { "directory" } else { "file" };
    if source_root_status(locator, kind, true) != SourceRootStatus::Available {
        return Err(LoomError::InvalidPath(
            "selected source is not currently available".into(),
        ));
    }
    let now = Utc::now().to_rfc3339();
    // RETURNING observes the same atomic upsert. There is no check-then-re-enable window.
    let authorization = connection.query_row(
        "INSERT INTO source_roots(id, kind, locator, enabled, created_at, last_seen_at)
         VALUES (?1, ?2, ?3, 1, ?4, ?4)
         ON CONFLICT(locator) DO UPDATE SET
            scope_generation = source_roots.scope_generation + CASE WHEN source_roots.enabled = 0 THEN 1 ELSE 0 END,
            enabled = 1, last_seen_at = excluded.last_seen_at
         WHERE source_roots.kind = excluded.kind
         RETURNING id, scope_generation,
            (SELECT value FROM schema_meta WHERE key = 'authorization_incarnation'), kind",
        params![
            Uuid::new_v4().to_string(),
            kind,
            locator,
            now
        ],
        |row| Ok(SourceAuthorization { root_id: row.get(0)?, generation: row.get(1)?, incarnation: row.get(2)?, kind: row.get(3)?, ocr_policy: None }),
    ).optional()?.ok_or_else(|| LoomError::InvalidPath("selected source changed file/directory kind; explicitly purge its old scope before selecting the replacement".into()))?;
    Ok(authorization)
}

fn pending_import_failures(
    transaction: &Transaction<'_>,
    import_id: &str,
) -> Result<Vec<IndexFailure>> {
    let mut statement = transaction.prepare(
        "SELECT ordinal, code, detail FROM bookmark_import_failures
         WHERE import_id = ?1 AND state = 'pending' ORDER BY ordinal",
    )?;
    let failures = statement
        .query_map([import_id], |row| {
            Ok(IndexFailure {
                source: format!("bookmark #{}", row.get::<_, i64>(0)?),
                reason: format!("{}: {}", row.get::<_, String>(1)?, row.get::<_, String>(2)?),
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(failures)
}

/// Imports one parsed bookmark into `import_id`, merging with an existing record when one exists.
#[allow(clippy::too_many_arguments)]
fn import_bookmark_entry(
    tx: &Transaction<'_>,
    root_id: &str,
    source_uri: &str,
    import_id: &str,
    entry: &BookmarkEntry,
    ordinal: usize,
    now: &str,
    report: &mut BookmarkImportReport,
) -> Result<()> {
    let entry_hash = bookmark_entry_hash(entry);
    let existing_record: Option<BookmarkRecordProjection> = tx
        .query_row(
            "SELECT b.id, b.title, b.entry_hash, b.added_at, b.modified_at,
                     b.artifact_id, a.source_root_id
                 FROM bookmark_records b JOIN artifacts a ON a.id = b.artifact_id
                 WHERE b.url = ?1 AND b.folder_path = ?2",
            params![entry.url, entry.folder_path],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                ))
            },
        )
        .optional()?;
    let same_url_elsewhere: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM bookmark_records WHERE url = ?1)",
        [&entry.url],
        |row| row.get(0),
    )?;
    let (bookmark_id, artifact_id, outcome) = if let Some((
        bookmark_id,
        old_title,
        old_hash,
        old_added_at,
        old_modified_at,
        artifact_id,
        existing_root_id,
    )) = existing_record
    {
        if existing_root_id != root_id {
            return Err(bookmark_scope_conflict());
        }
        let unchanged = old_hash == entry_hash
            && old_title == entry.title
            && old_added_at == entry.added_at
            && old_modified_at == entry.modified_at;
        if unchanged {
            upsert_bookmark_artifact(
                tx,
                root_id,
                source_uri,
                &artifact_id,
                entry,
                &entry_hash,
                now,
            )?;
            report.unchanged += 1;
            (bookmark_id, artifact_id, "unchanged")
        } else {
            upsert_bookmark_artifact(
                tx,
                root_id,
                source_uri,
                &artifact_id,
                entry,
                &entry_hash,
                now,
            )?;
            tx.execute(
                "UPDATE bookmark_records
                     SET title = ?1, added_at = ?2, modified_at = ?3, entry_hash = ?4,
                         updated_at = ?5
                     WHERE id = ?6",
                params![
                    entry.title,
                    entry.added_at,
                    entry.modified_at,
                    entry_hash,
                    now,
                    bookmark_id
                ],
            )?;
            report.merged += 1;
            (bookmark_id, artifact_id, "merged")
        }
    } else {
        let artifact_id =
            ensure_bookmark_artifact(tx, root_id, source_uri, entry, &entry_hash, now)?;
        let bookmark_id = Uuid::new_v4().to_string();
        tx.execute(
            "INSERT INTO bookmark_records(
                    id, artifact_id, folder_path, title, url, added_at, modified_at,
                    entry_hash, first_import_id, created_at, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)",
            params![
                bookmark_id,
                artifact_id,
                entry.folder_path,
                entry.title,
                entry.url,
                entry.added_at,
                entry.modified_at,
                entry_hash,
                import_id,
                now
            ],
        )?;
        report.imported += 1;
        let outcome = if same_url_elsewhere {
            report.conflicts += 1;
            "conflict"
        } else {
            "imported"
        };
        (bookmark_id, artifact_id, outcome)
    };
    tx.execute(
        "INSERT INTO bookmark_import_items(
                import_id, bookmark_id, ordinal, entry_hash, outcome
             ) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![import_id, bookmark_id, ordinal as i64, entry_hash, outcome],
    )?;
    let _ = artifact_id;
    Ok(())
}

/// True when the bookmark's URL identity already belongs to a different selected export. Each
/// bookmark artifact is owned by exactly one export's root, so revoking or purging one export never
/// changes another; an overlapping entry is recorded as a per-record failure instead.
fn bookmark_owned_by_other_export(
    tx: &Transaction<'_>,
    root_id: &str,
    entry: &BookmarkEntry,
) -> Result<bool> {
    let owners: Vec<String> = {
        let mut statement = tx.prepare(
            "SELECT a.source_root_id FROM bookmark_records b
             JOIN artifacts a ON a.id = b.artifact_id
             WHERE b.url = ?1 AND b.folder_path = ?2
             UNION
             SELECT a.source_root_id FROM artifact_locators l
             JOIN artifacts a ON a.id = l.artifact_id
             WHERE l.kind = 'url' AND l.locator = ?1 AND l.active = 1",
        )?;
        let rows = statement
            .query_map(params![entry.url, entry.folder_path], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    Ok(owners.iter().any(|owner| owner != root_id))
}

fn record_owned_elsewhere(
    tx: &Transaction<'_>,
    import_id: &str,
    record_ordinal: u32,
    byte_offset: usize,
    now: &str,
) -> Result<()> {
    tx.execute(
        "INSERT INTO bookmark_import_failures(
            import_id, ordinal, byte_offset, code, detail, state, created_at
         ) VALUES (?1, ?2, ?3, 'owned_by_other_export', ?4, 'pending', ?5)
         ON CONFLICT(import_id, ordinal) DO NOTHING",
        params![
            import_id,
            record_ordinal,
            byte_offset as i64,
            "this URL is already imported from another selected export; purge that export to import it here",
            now
        ],
    )?;
    Ok(())
}

fn bookmark_entry_hash(entry: &BookmarkEntry) -> String {
    let canonical = format!(
        "{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}",
        entry.folder_path,
        entry.title,
        entry.url,
        entry.added_at.as_deref().unwrap_or_default(),
        entry.modified_at.as_deref().unwrap_or_default()
    );
    format!("blake3:{}", blake3::hash(canonical.as_bytes()).to_hex())
}

fn bookmark_passage(entry: &BookmarkEntry) -> String {
    if entry.folder_path.is_empty() {
        format!("{}\n{}", entry.title, entry.url)
    } else {
        format!("{}\n{}\n{}", entry.title, entry.url, entry.folder_path)
    }
}

fn ensure_bookmark_artifact(
    transaction: &Transaction<'_>,
    root_id: &str,
    source_uri: &str,
    entry: &BookmarkEntry,
    entry_hash: &str,
    now: &str,
) -> Result<String> {
    let artifact_id: String = transaction
        .query_row(
            "SELECT artifact_id FROM artifact_locators
             WHERE kind = 'url' AND locator = ?1 AND active = 1",
            [&entry.url],
            |row| row.get(0),
        )
        .optional()?
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let existing_root: Option<String> = transaction
        .query_row(
            "SELECT source_root_id FROM artifacts WHERE id = ?1",
            [&artifact_id],
            |row| row.get(0),
        )
        .optional()?;
    if existing_root
        .as_ref()
        .is_some_and(|existing| existing != root_id)
    {
        return Err(bookmark_scope_conflict());
    }
    if existing_root.is_none() {
        transaction.execute(
            "INSERT INTO artifacts(
                id, source_root_id, title, media_type, state, created_at, last_seen_at
             ) VALUES (?1, ?2, ?3, 'text/x-bookmark', 'active', ?4, ?4)",
            params![artifact_id, root_id, entry.title, now],
        )?;
    } else {
        transaction.execute(
            "UPDATE artifacts SET title = ?1, state = 'active', last_seen_at = ?2 WHERE id = ?3",
            params![entry.title, now, artifact_id],
        )?;
    }
    transaction.execute(
        "INSERT INTO artifact_locators(
            id, artifact_id, kind, locator, active, first_seen_at, last_seen_at
         ) VALUES (?1, ?2, 'url', ?3, 1, ?4, ?4)
         ON CONFLICT(kind, locator) DO UPDATE SET artifact_id = excluded.artifact_id,
           active = 1, last_seen_at = excluded.last_seen_at",
        params![Uuid::new_v4().to_string(), artifact_id, entry.url, now],
    )?;
    upsert_bookmark_artifact(
        transaction,
        root_id,
        source_uri,
        &artifact_id,
        entry,
        entry_hash,
        now,
    )?;
    Ok(artifact_id)
}

fn upsert_bookmark_artifact(
    transaction: &Transaction<'_>,
    root_id: &str,
    source_uri: &str,
    artifact_id: &str,
    entry: &BookmarkEntry,
    entry_hash: &str,
    now: &str,
) -> Result<()> {
    let same_root: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM artifacts a
            JOIN source_roots r ON r.id = a.source_root_id AND r.enabled = 1
            WHERE a.id = ?1 AND a.source_root_id = ?2)",
        params![artifact_id, root_id],
        |row| row.get(0),
    )?;
    if !same_root {
        return Err(bookmark_scope_conflict());
    }
    let passage_text = bookmark_passage(entry);
    let content_hash = entry_hash;
    let extraction_metadata = serde_json::json!({
        "bookmark": {
            "folder_path": entry.folder_path,
            "added_at": entry.added_at,
            "modified_at": entry.modified_at,
            "source_export": source_uri,
        },
        "remote_fetch": false,
    });
    let active: Option<(String, String, String)> = transaction
        .query_row(
            "SELECT v.id, v.content_hash, v.extractor_version
             FROM artifacts a JOIN artifact_versions v ON v.id = a.active_version_id
             WHERE a.id = ?1",
            [artifact_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    if active.as_ref().is_some_and(|(_, hash, version)| {
        hash == content_hash && version == BOOKMARK_EXTRACTOR_VERSION
    }) {
        transaction.execute(
            "UPDATE artifacts SET title = ?1, state = 'active', last_seen_at = ?2
             WHERE id = ?3 AND source_root_id = ?4",
            params![entry.title, now, artifact_id, root_id],
        )?;
        return Ok(());
    }
    transaction.execute(
        "UPDATE artifact_versions SET status = 'superseded'
         WHERE artifact_id = ?1 AND status = 'ready'",
        [artifact_id],
    )?;
    let existing_version: Option<String> = transaction
        .query_row(
            "SELECT id FROM artifact_versions
             WHERE artifact_id = ?1 AND content_hash = ?2 AND extractor_id = ?3
               AND extractor_version = ?4",
            params![
                artifact_id,
                content_hash,
                BOOKMARK_EXTRACTOR_ID,
                BOOKMARK_EXTRACTOR_VERSION
            ],
            |row| row.get(0),
        )
        .optional()?;
    let version_id = if let Some(version_id) = existing_version {
        transaction.execute(
            "UPDATE artifact_versions
             SET status = 'ready', extraction_metadata_json = ?1
             WHERE id = ?2",
            params![serde_json::to_string(&extraction_metadata)?, version_id],
        )?;
        version_id
    } else {
        let version_id = Uuid::new_v4().to_string();
        transaction.execute(
            "INSERT INTO artifact_versions(
                id, artifact_id, content_hash, hash_algorithm, byte_size, source_modified_ns,
                extractor_id, extractor_version, parse_warnings_json, page_count,
                extraction_metadata_json, status, created_at
             ) VALUES (?1, ?2, ?3, 'blake3', ?4, NULL, ?5, ?6, '[]', NULL, ?7, 'ready', ?8)",
            params![
                version_id,
                artifact_id,
                content_hash,
                sql_i64(passage_text.len() as u64, "bookmark passage size")?,
                BOOKMARK_EXTRACTOR_ID,
                BOOKMARK_EXTRACTOR_VERSION,
                serde_json::to_string(&extraction_metadata)?,
                now
            ],
        )?;
        let char_end = passage_text.chars().count() as u64;
        let line_end = passage_text.lines().count().max(1) as u64;
        insert_passages(
            transaction,
            &version_id,
            &[PassageDraft {
                ordinal: 0,
                text: passage_text.clone(),
                text_hash: format!("blake3:{}", blake3::hash(passage_text.as_bytes()).to_hex()),
                anchor: EvidenceAnchor::Text {
                    char_start: 0,
                    char_end,
                    line_start: 1,
                    line_end,
                },
            }],
            now,
        )?;
        version_id
    };
    transaction.execute(
        "UPDATE artifacts SET active_version_id = ?1, title = ?2, media_type = 'text/x-bookmark',
             state = 'active', last_seen_at = ?3 WHERE id = ?4",
        params![version_id, entry.title, now, artifact_id],
    )?;
    Ok(())
}

fn bookmark_scope_conflict() -> LoomError {
    LoomError::InvalidPath("bookmark URL identity belongs to another selected export; re-select that export or explicitly purge its scope before importing the same URL from a different export".into())
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
        let (char_start, char_end, line_start, line_end) = match passage.anchor {
            EvidenceAnchor::Text {
                char_start,
                char_end,
                line_start,
                line_end,
            }
            | EvidenceAnchor::PdfPage {
                char_start,
                char_end,
                line_start,
                line_end,
                ..
            }
            | EvidenceAnchor::ImageRegion {
                char_start,
                char_end,
                line_start,
                line_end,
                ..
            } => (char_start, char_end, line_start, line_end),
        };
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

fn count_where(connection: &Connection, table: &str, predicate: &str) -> Result<u64> {
    let sql = format!("SELECT COUNT(*) FROM {table} WHERE {predicate}");
    let value: i64 = connection.query_row(&sql, [], |row| row.get(0))?;
    Ok(value.max(0) as u64)
}

fn storage_sql_bytes(connection: &Connection, query: &str) -> Result<u64> {
    let value: i64 = connection.query_row(query, [], |row| row.get(0))?;
    Ok(value.max(0) as u64)
}

fn regular_file_size(path: &Path) -> Result<u64> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(io_error(path, error)),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Ok(0);
    }
    Ok(metadata.len())
}

fn directory_size(path: &Path) -> Result<(u64, u64)> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok((0, 0)),
        Err(error) => return Err(io_error(path, error)),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Ok((0, 0));
    }
    let mut bytes = 0_u64;
    let mut files = 0_u64;
    for entry in fs::read_dir(path).map_err(|error| io_error(path, error))? {
        let entry = entry.map_err(|error| io_error(path, error))?;
        let child = entry.path();
        let child_metadata =
            fs::symlink_metadata(&child).map_err(|error| io_error(&child, error))?;
        if child_metadata.file_type().is_symlink() {
            continue;
        }
        if child_metadata.is_dir() {
            let (child_bytes, child_files) = directory_size(&child)?;
            bytes = bytes.saturating_add(child_bytes);
            files = files.saturating_add(child_files);
        } else if child_metadata.is_file() {
            bytes = bytes.saturating_add(child_metadata.len());
            files = files.saturating_add(1);
        }
    }
    Ok((bytes, files))
}

fn remove_regular_file(path: &Path, report: &mut DeletionReport) -> Result<()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(io_error(path, error)),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Ok(());
    }
    fs::remove_file(path).map_err(|error| io_error(path, error))?;
    report.files_deleted = report.files_deleted.saturating_add(1);
    report.bytes_deleted = report.bytes_deleted.saturating_add(metadata.len());
    report.paths.push(path.to_string_lossy().into_owned());
    Ok(())
}

fn remove_directory_contents(path: &Path, report: &mut DeletionReport) -> Result<()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(io_error(path, error)),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(path).map_err(|error| io_error(path, error))? {
        let entry = entry.map_err(|error| io_error(path, error))?;
        let child = entry.path();
        let child_metadata =
            fs::symlink_metadata(&child).map_err(|error| io_error(&child, error))?;
        if child_metadata.file_type().is_symlink() {
            continue;
        }
        if child_metadata.is_dir() {
            remove_directory_contents(&child, report)?;
            let _ = fs::remove_dir(&child);
        } else {
            remove_regular_file(&child, report)?;
        }
    }
    Ok(())
}

fn checkpoint_and_vacuum(connection: &Connection) -> Result<()> {
    connection.execute_batch(
        "PRAGMA wal_checkpoint(TRUNCATE);
         VACUUM;
         PRAGMA wal_checkpoint(TRUNCATE);",
    )?;
    Ok(())
}

fn normalize_timestamp(value: &str) -> Result<String> {
    DateTime::parse_from_rfc3339(value)
        .map(|timestamp| timestamp.with_timezone(&Utc).to_rfc3339())
        .map_err(|error| LoomError::InvalidPath(format!("timestamp must be RFC3339: {error}")))
}

fn artifact_ids_for_root(transaction: &Transaction<'_>, locator: &str) -> Result<Vec<String>> {
    let root_id: Option<String> = transaction
        .query_row(
            "SELECT id FROM source_roots WHERE locator = ?1",
            [locator],
            |row| row.get(0),
        )
        .optional()?;
    let Some(root_id) = root_id else {
        return Ok(Vec::new());
    };
    let mut statement = transaction
        .prepare("SELECT id FROM artifacts WHERE source_root_id = ?1 ORDER BY created_at, id")?;
    let ids = statement
        .query_map([root_id], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(ids)
}

fn delete_artifact_transaction(
    transaction: &Transaction<'_>,
    artifact_id: &str,
) -> Result<DeletionReport> {
    let exists: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM artifacts WHERE id = ?1)",
        [artifact_id],
        |row| row.get(0),
    )?;
    if !exists {
        return Err(LoomError::ArtifactNotFound(artifact_id.to_string()));
    }
    crate::jobs::purge_file_targets(transaction, None, Some(artifact_id), false)?;
    let versions_deleted: i64 = transaction.query_row(
        "SELECT COUNT(*) FROM artifact_versions WHERE artifact_id = ?1",
        [artifact_id],
        |row| row.get(0),
    )?;
    let passages_deleted: i64 = transaction.query_row(
        "SELECT COUNT(*) FROM passages WHERE artifact_version_id IN
            (SELECT id FROM artifact_versions WHERE artifact_id = ?1)",
        [artifact_id],
        |row| row.get(0),
    )?;
    let relationships_deleted: i64 = transaction.query_row(
        "SELECT COUNT(*) FROM relationships
         WHERE source_artifact_id = ?1 OR target_artifact_id = ?1",
        [artifact_id],
        |row| row.get(0),
    )?;
    let bookmark_records_deleted: i64 = transaction.query_row(
        "SELECT COUNT(*) FROM bookmark_records WHERE artifact_id = ?1",
        [artifact_id],
        |row| row.get(0),
    )?;
    transaction.execute("DELETE FROM artifacts WHERE id = ?1", [artifact_id])?;
    Ok(DeletionReport {
        selector: format!("artifact:{artifact_id}"),
        artifacts_deleted: 1,
        versions_deleted: versions_deleted.max(0) as u64,
        passages_deleted: passages_deleted.max(0) as u64,
        relationships_deleted: relationships_deleted.max(0) as u64,
        bookmark_records_deleted: bookmark_records_deleted.max(0) as u64,
        ..DeletionReport::default()
    })
}

fn merge_deletion_reports(target: &mut DeletionReport, source: DeletionReport) {
    target.artifacts_deleted = target
        .artifacts_deleted
        .saturating_add(source.artifacts_deleted);
    target.versions_deleted = target
        .versions_deleted
        .saturating_add(source.versions_deleted);
    target.passages_deleted = target
        .passages_deleted
        .saturating_add(source.passages_deleted);
    target.relationships_deleted = target
        .relationships_deleted
        .saturating_add(source.relationships_deleted);
    target.bookmark_records_deleted = target
        .bookmark_records_deleted
        .saturating_add(source.bookmark_records_deleted);
    target.files_deleted = target.files_deleted.saturating_add(source.files_deleted);
    target.bytes_deleted = target.bytes_deleted.saturating_add(source.bytes_deleted);
    target.paths.extend(source.paths);
}

fn purge_ocr_records_transaction(transaction: &Transaction<'_>) -> Result<OcrPurgeReport> {
    crate::jobs::purge_file_targets(transaction, None, None, true)?;
    let artifacts_affected: i64 = transaction.query_row(
        "SELECT COUNT(DISTINCT artifact_id) FROM artifact_versions WHERE extractor_id = ?1",
        [IMAGE_OCR_EXTRACTOR_ID],
        |row| row.get(0),
    )?;
    let versions_deleted: i64 = transaction.query_row(
        "SELECT COUNT(*) FROM artifact_versions WHERE extractor_id = ?1",
        [IMAGE_OCR_EXTRACTOR_ID],
        |row| row.get(0),
    )?;
    let passages_deleted: i64 = transaction.query_row(
        "SELECT COUNT(*) FROM passages WHERE artifact_version_id IN
            (SELECT id FROM artifact_versions WHERE extractor_id = ?1)",
        [IMAGE_OCR_EXTRACTOR_ID],
        |row| row.get(0),
    )?;
    transaction.execute(
        "DELETE FROM artifact_versions WHERE extractor_id = ?1",
        [IMAGE_OCR_EXTRACTOR_ID],
    )?;
    Ok(OcrPurgeReport {
        artifacts_affected: artifacts_affected.max(0) as u64,
        versions_deleted: versions_deleted.max(0) as u64,
        passages_deleted: passages_deleted.max(0) as u64,
    })
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct OcrPolicy {
    enabled: bool,
    revision: String,
}

impl OcrPolicy {
    fn load(connection: &Connection) -> Result<Self> {
        let (enabled, revision): (Option<String>, Option<String>) = connection.query_row(
            "SELECT (SELECT value FROM schema_meta WHERE key = 'ocr_enabled'),
                (SELECT value FROM schema_meta WHERE key = 'ocr_policy_revision')",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let invalid =
            || LoomError::OcrUnavailable("persisted OCR policy is missing or invalid".into());
        let enabled = match enabled.as_deref() {
            Some("1") => true,
            Some("0") => false,
            _ => return Err(invalid()),
        };
        let revision = revision
            .filter(|value| Uuid::parse_str(value).is_ok())
            .ok_or_else(invalid)?;
        Ok(Self { enabled, revision })
    }

    fn verify_current(&self, connection: &Connection) -> Result<()> {
        let current = Self::load(connection)?;
        if self.enabled != current.enabled || self.revision != current.revision {
            return Err(LoomError::OcrPolicyChanged);
        }
        Ok(())
    }
}

pub(crate) fn rotate_ocr_policy_revision(connection: &Connection) -> Result<()> {
    connection.execute(
        "INSERT INTO schema_meta(key, value) VALUES ('ocr_policy_revision', ?1)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [Uuid::new_v4().to_string()],
    )?;
    Ok(())
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

    #[cfg(unix)]
    #[test]
    fn discovery_fingerprint_distinguishes_native_paths_and_bounds() {
        use std::{ffi::OsString, os::unix::ffi::OsStringExt, path::PathBuf};
        let first = PathBuf::from(OsString::from_vec(b"/x\x80.md".to_vec()));
        let second = PathBuf::from(OsString::from_vec(b"/x\x81.md".to_vec()));
        assert_eq!(first.to_string_lossy(), second.to_string_lossy());
        let first_hash = super::discovery_fingerprint(std::slice::from_ref(&first), None, 10);
        assert_ne!(
            first_hash,
            super::discovery_fingerprint(&[second], None, 10)
        );
        assert_ne!(first_hash, super::discovery_fingerprint(&[first], None, 11));
    }

    // Deterministic prepared-provider boundary fixture. Native Vision quality is covered by
    // tests/image_ocr.rs; these tests control exactly when a prepared result reaches SQLite.
    fn prepared_ocr_document(source: &std::path::Path) -> ingest::StableDocument {
        let bytes = fs::read(source).unwrap();
        let raw_hash = format!("blake3:{}", blake3::hash(&bytes));
        let properties = crate::ocr::inspect_image(&bytes).unwrap();
        let text = "syntheticocrpolicy marker".to_owned();
        ingest::StableDocument {
            raw_hash: raw_hash.clone(),
            byte_size: bytes.len() as u64,
            modified_ns: None,
            normalized_text: text.clone(),
            media_type: "image/png",
            pdf_pages: None,
            page_count: None,
            parse_warnings: vec![],
            image_regions: Some(vec![crate::ocr::ImageOcrRegion {
                text: text.clone(),
                confidence_milli: 900,
                bounds: crate::ocr::ImagePixelBounds {
                    x: 0,
                    y: 0,
                    width: 10,
                    height: 10,
                },
                char_start: 0,
                char_end: text.chars().count() as u64,
                line_start: 1,
                line_end: 1,
                image_width: properties.width,
                image_height: properties.height,
                orientation: 1,
                scale_milli: 1_000,
            }]),
            extraction_metadata: serde_json::json!({
                "provider_id": "deterministic.test", "image_hash": raw_hash,
                "image_width": properties.width, "image_height": properties.height,
            }),
        }
    }

    fn prepared_ocr_must_not_undo_policy(action: &str) {
        let directory = tempdir().unwrap();
        let source = directory.path().join("policy.png");
        let original = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/ocr-golden.png"
        ));
        fs::write(&source, original).unwrap();
        let database = directory.path().join("policy.sqlite3");
        let worker = Library::open(&database).unwrap();
        let controller = Library::open(&database).unwrap();
        let source = source.canonicalize().unwrap();
        let mut authorization =
            super::ensure_source_root(&mut worker.lock().unwrap(), source.to_str().unwrap(), false)
                .unwrap();
        authorization.ocr_policy = Some(super::OcrPolicy::load(&worker.lock().unwrap()).unwrap());
        worker
            .index_document_with_extractor(
                &authorization,
                &source,
                prepared_ocr_document(&source),
                crate::ocr::IMAGE_OCR_EXTRACTOR_ID,
                crate::ocr::IMAGE_OCR_EXTRACTOR_VERSION,
            )
            .unwrap();
        let job = worker
            .start_index_job(&authorization, source.to_str().unwrap(), "preparedocr", 1)
            .unwrap();
        let prepared = prepared_ocr_document(&source);
        match action {
            "disable" => {
                controller.set_ocr_enabled(false).unwrap();
            }
            "disable_reenable" => {
                controller.set_ocr_enabled(false).unwrap();
                controller.set_ocr_enabled(true).unwrap();
            }
            "purge" => {
                controller.purge_ocr_records().unwrap();
            }
            "reassert_enabled" => {
                controller.set_ocr_enabled(true).unwrap();
            }
            _ => panic!("unknown fixture action"),
        }
        let before = controller.export_portable().unwrap().tables;
        let checkpoint = controller.index_checkpoint(&source).unwrap();
        let health = serde_json::to_value(controller.fts_health().unwrap()).unwrap();
        let result = worker.index_document_with_extractor_and_checkpoint(
            &authorization,
            &source,
            prepared,
            crate::ocr::IMAGE_OCR_EXTRACTOR_ID,
            crate::ocr::IMAGE_OCR_EXTRACTOR_VERSION,
            Some((&job.job_id, 1)),
        );
        assert!(
            result.is_err(),
            "old prepared OCR was accepted after {action}: {result:?}"
        );
        assert_eq!(controller.export_portable().unwrap().tables, before);
        assert_eq!(controller.index_checkpoint(&source).unwrap(), checkpoint);
        for operation in [
            worker.advance_index_job(&authorization, &job.job_id, 1),
            worker.mark_locator_missing_and_advance(
                &authorization,
                source.to_str().unwrap(),
                &job.job_id,
                1,
            ),
            worker.interrupt_index_job(&authorization, &job.job_id, "stale OCR"),
            worker.fail_index_job(&authorization, &job.job_id, "stale OCR"),
            worker.complete_index_job(&authorization, &job.job_id, None),
        ] {
            assert!(matches!(operation, Err(LoomError::OcrPolicyChanged)));
        }
        assert_eq!(controller.export_portable().unwrap().tables, before);
        assert_eq!(controller.index_checkpoint(&source).unwrap(), checkpoint);
        assert_eq!(
            serde_json::to_value(controller.fts_health().unwrap()).unwrap(),
            health
        );
        assert_eq!(fs::read(&source).unwrap(), original);
        controller.set_ocr_enabled(true).unwrap();
        let mut fresh = worker
            .approved_root_authorization(source.to_str().unwrap())
            .unwrap();
        fresh.ocr_policy = Some(super::OcrPolicy::load(&worker.lock().unwrap()).unwrap());
        let recovered = worker
            .index_document_with_extractor(
                &fresh,
                &source,
                prepared_ocr_document(&source),
                crate::ocr::IMAGE_OCR_EXTRACTOR_ID,
                crate::ocr::IMAGE_OCR_EXTRACTOR_VERSION,
            )
            .unwrap();
        assert_eq!(recovered, action != "reassert_enabled");
        assert_eq!(worker.ocr_status().unwrap().derived_versions, 1);
    }

    #[test]
    fn queued_prepared_ocr_obeys_revision_and_purge_fences_before_any_canonical_write() {
        use super::{
            CanonicalFileSnapshot, IndexFileTarget, PreparedFileJob, PreparedIndexDocument,
        };
        for action in ["reassert", "disable", "purge"] {
            let directory = tempdir().unwrap();
            let source = directory.path().join("queued-policy.png");
            let original = include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../tests/fixtures/ocr-golden.png"
            ));
            fs::write(&source, original).unwrap();
            let library = Library::open(directory.path().join("queued-policy.sqlite3")).unwrap();
            library.set_ocr_enabled(false).unwrap();
            library.index_path(&source).unwrap();
            library.set_ocr_enabled(true).unwrap();
            let queued = library
                .enqueue_index_file(&source, "queued-policy", crate::JobPriority::Normal)
                .unwrap();
            let mut worker = library.acquire_job_worker().unwrap();
            let claim = worker.claim_for_test().unwrap().unwrap();
            let json: String = library
                .lock()
                .unwrap()
                .query_row(
                    "SELECT target_json FROM background_jobs WHERE id=?1",
                    [&queued.id],
                    |row| row.get(0),
                )
                .unwrap();
            let target = IndexFileTarget::parse(&json).unwrap();
            let snapshot: Option<CanonicalFileSnapshot> =
                super::canonical_file_snapshot(&library.lock().unwrap(), &target.locator).unwrap();
            let prepared = PreparedFileJob {
                target,
                target_json: json,
                snapshot,
                extraction_metrics: Default::default(),
                document: PreparedIndexDocument::new(
                    &source,
                    prepared_ocr_document(&source),
                    LibraryLimits::default(),
                    crate::ocr::IMAGE_OCR_EXTRACTOR_ID,
                    crate::ocr::IMAGE_OCR_EXTRACTOR_VERSION,
                )
                .unwrap(),
            };
            match action {
                "reassert" => {
                    library.set_ocr_enabled(true).unwrap();
                }
                "disable" => {
                    library.set_ocr_enabled(false).unwrap();
                }
                "purge" => {
                    library.purge_ocr_records().unwrap();
                }
                _ => unreachable!(),
            }
            let before = library.export_portable().unwrap().digest;
            assert!(
                library.publish_file_job(&claim, &prepared).is_err(),
                "published after {action}"
            );
            assert_eq!(library.export_portable().unwrap().digest, before);
            assert_eq!(library.ocr_status().unwrap().derived_versions, 0);
            assert_eq!(fs::read(&source).unwrap(), original);
        }
    }

    #[test]
    fn prepared_ocr_cannot_undo_disable_from_another_connection() {
        prepared_ocr_must_not_undo_policy("disable");
    }

    #[test]
    fn prepared_ocr_cannot_inherit_a_later_reenable() {
        prepared_ocr_must_not_undo_policy("disable_reenable");
    }

    #[test]
    fn prepared_ocr_cannot_undo_a_purge_while_enabled() {
        prepared_ocr_must_not_undo_policy("purge");
    }

    #[test]
    fn prepared_ocr_cannot_inherit_an_explicit_policy_reassertion() {
        prepared_ocr_must_not_undo_policy("reassert_enabled");
    }

    #[test]
    fn ocr_results_require_a_captured_enabled_policy_before_canonical_writes() {
        let directory = tempdir().unwrap();
        let source = directory.path().join("policy.png");
        fs::write(
            &source,
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../tests/fixtures/ocr-golden.png"
            )),
        )
        .unwrap();
        let library = Library::open_in_memory().unwrap();
        let mut authorization = super::ensure_source_root(
            &mut library.lock().unwrap(),
            source.to_str().unwrap(),
            false,
        )
        .unwrap();
        let before = library.export_portable().unwrap().tables;
        let result = library.index_document_with_extractor(
            &authorization,
            &source,
            prepared_ocr_document(&source),
            crate::ocr::IMAGE_OCR_EXTRACTOR_ID,
            crate::ocr::IMAGE_OCR_EXTRACTOR_VERSION,
        );
        assert!(matches!(result, Err(LoomError::OcrUnavailable(_))));
        assert_eq!(library.export_portable().unwrap().tables, before);
        library.set_ocr_enabled(false).unwrap();
        authorization.ocr_policy = Some(super::OcrPolicy::load(&library.lock().unwrap()).unwrap());
        let result = library.index_document_with_extractor(
            &authorization,
            &source,
            prepared_ocr_document(&source),
            crate::ocr::IMAGE_OCR_EXTRACTOR_ID,
            crate::ocr::IMAGE_OCR_EXTRACTOR_VERSION,
        );
        assert!(matches!(result, Err(LoomError::OcrDisabled)));
        assert_eq!(library.export_portable().unwrap().tables, before);
    }

    #[test]
    fn changed_ocr_policy_restarts_a_scan_instead_of_resuming_past_disabled_images() {
        for change in ["reenable", "purge"] {
            let directory = tempdir().unwrap();
            let root = directory.path().join("selected");
            fs::create_dir(&root).unwrap();
            fs::write(
                root.join("a.png"),
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../tests/fixtures/ocr-golden.png"
                )),
            )
            .unwrap();
            fs::write(root.join("z.md"), "pending text").unwrap();
            let database = directory.path().join("policy.sqlite3");
            let worker = Library::open(&database).unwrap();
            let controller = Library::open(&database).unwrap();
            controller.set_ocr_enabled(false).unwrap();
            assert!(matches!(
                worker.index_path_with_fault(&root, Some(1)),
                Err(LoomError::IndexInterrupted(_))
            ));
            let previous = worker.index_checkpoint(&root).unwrap().unwrap();
            assert_eq!(previous.next_unit, 1);
            if change == "reenable" {
                controller.set_ocr_enabled(true).unwrap();
            } else {
                controller.purge_ocr_records().unwrap();
            }
            assert!(matches!(
                worker.index_path_with_fault(&root, Some(0)),
                Err(LoomError::IndexInterrupted(_))
            ));
            let restarted = worker.index_checkpoint(&root).unwrap().unwrap();
            assert_eq!(restarted.next_unit, 0);
            assert_eq!(worker.stats().unwrap().versions, 0);
        }
    }

    #[test]
    fn ocr_policy_restore_is_transactional_and_visible_to_other_open_handles() {
        let exported = Library::open_in_memory().unwrap();
        exported.set_ocr_enabled(false).unwrap();
        let mut archive = exported.export_portable().unwrap();
        assert!(!archive.settings.contains_key("ocr_policy_revision"));
        let directory = tempdir().unwrap();
        let database = directory.path().join("restored.sqlite3");
        let library = Library::open(&database).unwrap();
        let observer = Library::open(&database).unwrap();
        let before = super::OcrPolicy::load(&observer.lock().unwrap()).unwrap();
        archive
            .tables
            .get_mut("passages")
            .unwrap()
            .columns
            .push("unsupported_column".into());
        archive.seal().unwrap();
        assert!(matches!(
            library.import_portable(&archive),
            Err(LoomError::PortableExport(_))
        ));
        assert_eq!(
            super::OcrPolicy::load(&observer.lock().unwrap()).unwrap(),
            before
        );
        archive.tables.get_mut("passages").unwrap().columns.pop();
        archive.seal().unwrap();
        library.import_portable(&archive).unwrap();
        let restored = super::OcrPolicy::load(&observer.lock().unwrap()).unwrap();
        assert_ne!(restored.revision, before.revision);
        assert!(!observer.ocr_status().unwrap().enabled);
        library.purge_ocr_records().unwrap();
        assert_ne!(
            super::OcrPolicy::load(&observer.lock().unwrap())
                .unwrap()
                .revision,
            restored.revision
        );
    }

    #[test]
    fn existing_schema_ten_initializes_only_the_operational_ocr_revision() {
        let directory = tempdir().unwrap();
        let database = directory.path().join("old-ten.sqlite3");
        let source = directory.path().join("selected.md");
        fs::write(&source, "preserved migration text").unwrap();
        let library = Library::open(&database).unwrap();
        library.index_path(&source).unwrap();
        library.set_ocr_enabled(false).unwrap();
        let before = library.export_portable().unwrap();
        library
            .lock()
            .unwrap()
            .execute(
                "DELETE FROM schema_meta WHERE key = 'ocr_policy_revision'",
                [],
            )
            .unwrap();
        drop(library);
        let reopened = Library::open(&database).unwrap();
        let after = reopened.export_portable().unwrap();
        assert_eq!(after.library_schema_version, 10);
        assert_eq!(after.tables, before.tables);
        assert_eq!(after.settings, before.settings);
        assert_eq!(after.digest, before.digest);
        assert!(!reopened.ocr_status().unwrap().enabled);
    }

    #[test]
    fn ocr_status_observes_policy_changes_from_another_connection() {
        let directory = tempdir().unwrap();
        let database = directory.path().join("policy.sqlite3");
        let worker = Library::open(&database).unwrap();
        let controller = Library::open(&database).unwrap();
        controller.set_ocr_enabled(false).unwrap();
        assert!(!worker.ocr_status().unwrap().enabled);
        controller.set_ocr_enabled(true).unwrap();
        assert!(worker.ocr_status().unwrap().enabled);
    }

    #[test]
    fn malformed_ocr_policy_fails_closed_without_blocking_text() {
        let directory = tempdir().unwrap();
        let source = directory.path().join("policy.md");
        fs::write(&source, "textdoesnotneedocr").unwrap();
        let library = Library::open_in_memory().unwrap();
        library
            .lock()
            .unwrap()
            .execute(
                "UPDATE schema_meta SET value = 'yes' WHERE key = 'ocr_enabled'",
                [],
            )
            .unwrap();
        assert!(library.ocr_status().is_err());
        assert_eq!(library.index_path(&source).unwrap().indexed, 1);
    }

    #[test]
    fn malformed_or_missing_ocr_revision_refuses_images_but_not_text() {
        for revision in [Some("not-a-revision"), Some(""), None] {
            let directory = tempdir().unwrap();
            let source = directory.path().join("policy.png");
            let text = directory.path().join("policy.md");
            fs::write(
                &source,
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../tests/fixtures/ocr-golden.png"
                )),
            )
            .unwrap();
            fs::write(&text, "plaintextindependent").unwrap();
            let library = Library::open_in_memory().unwrap();
            let connection = library.lock().unwrap();
            match revision {
                Some(value) => {
                    connection
                        .execute(
                            "UPDATE schema_meta SET value = ?1 WHERE key = 'ocr_policy_revision'",
                            [value],
                        )
                        .unwrap();
                }
                None => {
                    connection
                        .execute(
                            "DELETE FROM schema_meta WHERE key = 'ocr_policy_revision'",
                            [],
                        )
                        .unwrap();
                }
            }
            drop(connection);
            assert!(matches!(
                library.ocr_status(),
                Err(LoomError::OcrUnavailable(_))
            ));
            assert!(matches!(
                library.index_path(&source),
                Err(LoomError::OcrUnavailable(_))
            ));
            assert_eq!(library.stats().unwrap().versions, 0);
            assert_eq!(library.index_path(&text).unwrap().indexed, 1);
        }
    }

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
        assert_eq!(schema_version, "10");

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
            "bookmark_imports",
            "bookmark_records",
            "bookmark_import_items",
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
            "bookmark_imports",
            "bookmark_records",
            "bookmark_import_items",
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
        for column in ["parse_warnings_json", "page_count"] {
            let exists: bool = connection
                .query_row(
                    "SELECT EXISTS(
                        SELECT 1 FROM pragma_table_info('artifact_versions') WHERE name = ?1
                    )",
                    [column],
                    |row| row.get(0),
                )
                .unwrap();
            assert!(
                exists,
                "migration did not create artifact_versions.{column}"
            );
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
        assert_eq!(schema_version, "10");
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
        let authorization = library
            .approved_root_authorization(canonical_source.to_str().unwrap())
            .unwrap();
        let document =
            ingest::read_stable(&canonical_source, &canonical_source, 8 * 1024 * 1024).unwrap();

        assert!(library
            .index_document_with_extractor(
                &authorization,
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
    #[cfg(unix)]
    fn a_denied_single_file_root_cannot_commit_prepared_work_or_checkpoint_updates() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempdir().unwrap();
        let source = directory.path().join("denied.md");
        fs::write(&source, "deniedrootmarker").unwrap();
        let library = Library::open_in_memory().unwrap();
        library.index_path(&source).unwrap();
        let source = source.canonicalize().unwrap();
        let authorization = library
            .approved_root_authorization(source.to_str().unwrap())
            .unwrap();
        let job = library
            .start_index_job(&authorization, source.to_str().unwrap(), "denied", 1)
            .unwrap();
        let prepared = ingest::read_stable(&source, &source, 8 * 1024 * 1024).unwrap();
        let before = library.export_portable().unwrap().tables;
        let checkpoint = library.index_checkpoint(&source).unwrap();
        fs::set_permissions(&source, fs::Permissions::from_mode(0o0)).unwrap();
        assert_eq!(
            library.source_roots().unwrap()[0].status,
            crate::SourceRootStatus::Denied
        );
        assert!(matches!(
            library.index_document_with_extractor(
                &authorization,
                &source,
                prepared,
                "loom.text",
                "changed"
            ),
            Err(LoomError::SourceRevoked(_))
        ));
        for operation in [
            library.advance_index_job(&authorization, &job.job_id, 1),
            library.mark_locator_missing_and_advance(
                &authorization,
                source.to_str().unwrap(),
                &job.job_id,
                1,
            ),
            library.interrupt_index_job(&authorization, &job.job_id, "denied"),
            library.fail_index_job(&authorization, &job.job_id, "denied"),
            library.complete_index_job(&authorization, &job.job_id, None),
        ] {
            assert!(matches!(operation, Err(LoomError::SourceRevoked(_))));
        }
        assert!(library.index_path(&source).is_err());
        assert_eq!(library.export_portable().unwrap().tables, before);
        assert_eq!(library.index_checkpoint(&source).unwrap(), checkpoint);
        fs::set_permissions(&source, fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(library.index_path(&source).unwrap().unchanged, 1);
    }

    #[test]
    fn file_directory_replacements_cannot_reuse_selected_consent() {
        for was_directory in [false, true] {
            let directory = tempdir().unwrap();
            let root = directory.path().join("selected.md");
            let source = if was_directory {
                fs::create_dir(&root).unwrap();
                root.join("original.md")
            } else {
                root.clone()
            };
            fs::write(&source, "oldshapemarker").unwrap();
            let library = Library::open_in_memory().unwrap();
            library.index_path(&root).unwrap();
            let root = root.canonicalize().unwrap();
            let source = source.canonicalize().unwrap();
            let authorization = library
                .approved_root_authorization(root.to_str().unwrap())
                .unwrap();
            let prepared = ingest::read_stable(&source, &root, 8 * 1024 * 1024).unwrap();
            let before = library.export_portable().unwrap().tables;
            fs::remove_file(&source).unwrap();
            if was_directory {
                fs::remove_dir(&root).unwrap();
                fs::write(&root, "replacementshapemarker").unwrap();
            } else {
                fs::create_dir(&root).unwrap();
                fs::write(root.join("replacement.md"), "replacementshapemarker").unwrap();
            }
            assert!(matches!(
                library.index_document_with_extractor(
                    &authorization,
                    &source,
                    prepared,
                    "loom.text",
                    "changed"
                ),
                Err(LoomError::SourceRevoked(_))
            ));
            assert!(library.index_path(&root).is_err());
            assert!(matches!(
                library.index_path_with_options(
                    &root,
                    &Default::default(),
                    None,
                    None,
                    None,
                    Some(authorization)
                ),
                Err(LoomError::SourceRevoked(_))
            ));
            assert_eq!(library.export_portable().unwrap().tables, before);
            library.purge_root(root.to_str().unwrap()).unwrap();
            let replacement = library.index_path(&root).unwrap();
            assert_eq!(replacement.indexed, 1);
            assert_eq!(
                library.source_roots().unwrap()[0].kind,
                if was_directory { "file" } else { "directory" }
            );
        }
    }

    #[test]
    fn bookmark_import_refuses_a_replaced_directory_scope_until_explicit_reset() {
        let directory = tempdir().unwrap();
        let root = directory.path().join("selected.html");
        fs::create_dir(&root).unwrap();
        let old = root.join("old.md");
        fs::write(&old, "oldbookmarkscope").unwrap();
        let library = Library::open_in_memory().unwrap();
        library.index_path(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let before = library.export_portable().unwrap().tables;
        fs::remove_file(old).unwrap();
        fs::remove_dir(&root).unwrap();
        fs::write(
            &root,
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/bookmarks/chrome.html"
            )),
        )
        .unwrap();
        assert!(library.import_bookmarks(&root).is_err());
        assert_eq!(library.export_portable().unwrap().tables, before);
        library.purge_root(root.to_str().unwrap()).unwrap();
        let imported = library.import_bookmarks(&root).unwrap();
        assert_eq!(imported.imported, 1);
        assert!(library.retry_bookmark_import(&imported.import_id).is_ok());
        super::validate_bookmark_scope_consistency(&library.lock().unwrap()).unwrap();
    }

    #[test]
    fn a_pending_bookmark_retry_cannot_inherit_later_source_consent() {
        for reselected in [false, true] {
            let directory = tempdir().unwrap();
            let source = directory.path().join("bookmarks.html");
            let database = directory.path().join("library.sqlite3");
            fs::write(
                &source,
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/tests/fixtures/bookmarks/chrome.html"
                )),
            )
            .unwrap();
            let worker = Library::open(&database).unwrap();
            let import = worker.import_bookmarks(&source).unwrap();
            let authorization = worker
                .approved_root_authorization(&import.source_uri)
                .unwrap();
            let controller = Library::open(&database).unwrap();
            controller.revoke_source_root(&import.source_uri).unwrap();
            if reselected {
                controller.import_bookmarks(&source).unwrap();
            }
            assert!(matches!(
                worker.import_bookmarks_with_authorization(&source, Some(authorization)),
                Err(LoomError::SourceRevoked(_))
            ));
            assert_eq!(controller.source_roots().unwrap()[0].enabled, reselected);
        }
    }

    #[test]
    fn completed_jobs_refuse_late_checkpoint_and_terminal_updates() {
        let directory = tempdir().unwrap();
        let source = directory.path().join("completed.md");
        fs::write(&source, "completedcheckpointcontent").unwrap();
        let library = Library::open_in_memory().unwrap();
        library.index_path(&source).unwrap();
        let source = source.canonicalize().unwrap();
        let authorization = library
            .approved_root_authorization(source.to_str().unwrap())
            .unwrap();
        let before = library.index_checkpoint(&source).unwrap().unwrap();
        for operation in [
            library.advance_index_job(&authorization, &before.job_id, 0),
            library.interrupt_index_job(&authorization, &before.job_id, "late"),
            library.fail_index_job(&authorization, &before.job_id, "late"),
            library.complete_index_job(&authorization, &before.job_id, None),
        ] {
            assert!(matches!(operation, Err(LoomError::IndexJobStale(_))));
        }
        assert_eq!(library.index_checkpoint(&source).unwrap().unwrap(), before);
    }

    #[test]
    fn source_authorization_cannot_mutate_another_roots_job_or_artifact() {
        let directory = tempdir().unwrap();
        let first = directory.path().join("first.md");
        let second = directory.path().join("second.md");
        fs::write(&first, "firstcheckpointcontent").unwrap();
        fs::write(&second, "secondcheckpointcontent").unwrap();
        let library = Library::open_in_memory().unwrap();
        library.index_path(&first).unwrap();
        library.index_path(&second).unwrap();
        let first = first.canonicalize().unwrap();
        let second = second.canonicalize().unwrap();
        let authorization = library
            .approved_root_authorization(first.to_str().unwrap())
            .unwrap();
        let other = library
            .approved_root_authorization(second.to_str().unwrap())
            .unwrap();
        let job = library
            .start_index_job(&other, second.to_str().unwrap(), "other", 1)
            .unwrap();
        let before = library.index_checkpoint(&second).unwrap().unwrap();
        let operations = [
            library.advance_index_job(&authorization, &job.job_id, 1),
            library.interrupt_index_job(&authorization, &job.job_id, "wrong-root"),
            library.fail_index_job(&authorization, &job.job_id, "wrong-root"),
            library.complete_index_job(&authorization, &job.job_id, None),
            library.mark_locator_missing_and_advance(
                &authorization,
                first.to_str().unwrap(),
                &job.job_id,
                1,
            ),
        ];
        for operation in operations {
            assert!(operation.is_err());
        }
        assert_eq!(library.index_checkpoint(&second).unwrap().unwrap(), before);
        let prepared = ingest::read_stable(&first, &first, 8 * 1024 * 1024).unwrap();
        assert!(library
            .index_document_with_extractor_and_checkpoint(
                &authorization,
                &first,
                prepared,
                "loom.text",
                "new-version",
                Some((&job.job_id, 1))
            )
            .is_err());
        assert_eq!(
            library.inspect_source(&first).unwrap().extractor_version,
            "0.1.0"
        );
        assert_eq!(
            library
                .search(&SearchRequest {
                    text: "firstcheckpointcontent".into(),
                    limit: 10
                })
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn extracted_document_cannot_reactivate_a_revoked_root_from_another_connection() {
        let directory = tempdir().unwrap();
        let source = directory.path().join("private.md");
        let database = directory.path().join("loom.sqlite3");
        fs::write(&source, "revokedprivatecontent must stay hidden").unwrap();
        let worker = Library::open(&database).unwrap();
        worker.index_path(&source).unwrap();
        let source = source.canonicalize().unwrap();
        let authorization = worker
            .approved_root_authorization(source.to_str().unwrap())
            .unwrap();
        let prepared = ingest::read_stable(&source, &source, 8 * 1024 * 1024).unwrap();
        let controller = Library::open(&database).unwrap();
        controller
            .revoke_source_root(source.to_str().unwrap())
            .unwrap();

        let result = worker.index_document_with_extractor(
            &authorization,
            &source,
            prepared,
            "loom.text",
            "0.2.0",
        );
        assert!(
            matches!(result, Err(LoomError::SourceRevoked(_))),
            "a revoked extraction must not commit"
        );
        assert!(controller
            .search(&SearchRequest {
                text: "revokedprivatecontent".into(),
                limit: 10
            })
            .unwrap()
            .is_empty());
        assert!(!controller.source_roots().unwrap()[0].enabled);
    }

    #[test]
    fn old_extraction_cannot_replace_a_reselected_sources_new_version() {
        let directory = tempdir().unwrap();
        let source = directory.path().join("private.md");
        let database = directory.path().join("loom.sqlite3");
        fs::write(&source, "oldprivatecontent before revocation").unwrap();
        let worker = Library::open(&database).unwrap();
        worker.index_path(&source).unwrap();
        let source = source.canonicalize().unwrap();
        let authorization = worker
            .approved_root_authorization(source.to_str().unwrap())
            .unwrap();
        let prepared = ingest::read_stable(&source, &source, 8 * 1024 * 1024).unwrap();
        let controller = Library::open(&database).unwrap();
        controller
            .revoke_source_root(source.to_str().unwrap())
            .unwrap();
        fs::write(&source, "newprivatecontent after explicit re-selection").unwrap();
        controller.index_path(&source).unwrap();
        let current = controller.inspect_source(&source).unwrap();

        let result = worker.index_document_with_extractor(
            &authorization,
            &source,
            prepared,
            "loom.text",
            "0.1.0",
        );
        assert!(
            matches!(result, Err(LoomError::SourceRevoked(_))),
            "old consent must not authorize a new selection"
        );
        assert_eq!(
            controller.inspect_source(&source).unwrap().content_hash,
            current.content_hash
        );
        assert!(controller
            .search(&SearchRequest {
                text: "oldprivatecontent".into(),
                limit: 10
            })
            .unwrap()
            .is_empty());
    }

    #[test]
    fn stale_workers_cannot_hide_reselected_evidence_or_mutate_its_checkpoint() {
        let directory = tempdir().unwrap();
        let root = directory.path().join("selected");
        fs::create_dir(&root).unwrap();
        let source = root.join("private.md");
        fs::write(&source, "freshprivatecontent after re-selection").unwrap();
        let database = directory.path().join("loom.sqlite3");
        let worker = Library::open(&database).unwrap();
        worker.index_path(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let source = source.canonicalize().unwrap();
        let authorization = worker
            .approved_root_authorization(root.to_str().unwrap())
            .unwrap();
        let checkpoint = worker.index_checkpoint(&root).unwrap().unwrap();
        let controller = Library::open(&database).unwrap();
        controller
            .revoke_source_root(root.to_str().unwrap())
            .unwrap();
        controller.index_path(&root).unwrap();
        let fresh = controller.index_checkpoint(&root).unwrap().unwrap();

        let operations = [
            worker.mark_locator_missing_and_advance(
                &authorization,
                source.to_str().unwrap(),
                &checkpoint.job_id,
                0,
            ),
            worker.reconcile_directory(&authorization, &Default::default()),
            worker.advance_index_job(&authorization, &checkpoint.job_id, 0),
            worker.interrupt_index_job(&authorization, &checkpoint.job_id, "stale cancellation"),
            worker.fail_index_job(&authorization, &checkpoint.job_id, "stale failure"),
            worker.complete_index_job(&authorization, &checkpoint.job_id, Some("stale completion")),
        ];
        for operation in operations {
            assert!(matches!(operation, Err(LoomError::SourceRevoked(_))));
        }
        assert!(matches!(
            worker.start_index_job(&authorization, root.to_str().unwrap(), "stale", 1),
            Err(LoomError::SourceRevoked(_))
        ));
        assert_eq!(controller.index_checkpoint(&root).unwrap().unwrap(), fresh);
        assert_eq!(
            controller
                .search(&SearchRequest {
                    text: "freshprivatecontent".into(),
                    limit: 10
                })
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn pending_observer_scan_cannot_reenable_a_revoked_root() {
        let directory = tempdir().unwrap();
        let root = directory.path().join("selected");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("private.md"), "observerprivatecontent").unwrap();
        let database = directory.path().join("loom.sqlite3");
        let observer = Library::open(&database).unwrap();
        observer.index_path(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let authorization = observer
            .approved_root_authorization(root.to_str().unwrap())
            .unwrap();
        let controller = Library::open(&database).unwrap();
        controller
            .revoke_source_root(root.to_str().unwrap())
            .unwrap();
        let result = observer.index_path_with_options(
            &root,
            &Default::default(),
            None,
            None,
            None,
            Some(authorization.clone()),
        );
        assert!(matches!(result, Err(LoomError::SourceRevoked(_))));
        assert!(!controller.source_roots().unwrap()[0].enabled);
        controller.index_path(&root).unwrap();
        let result = observer.index_path_with_options(
            &root,
            &Default::default(),
            None,
            None,
            None,
            Some(authorization),
        );
        assert!(matches!(result, Err(LoomError::SourceRevoked(_))));
        assert!(controller.source_roots().unwrap()[0].enabled);
    }

    #[test]
    fn disabled_roots_are_excluded_even_if_artifact_state_is_stale() {
        let directory = tempdir().unwrap();
        let source = directory.path().join("private.md");
        let database = directory.path().join("loom.sqlite3");
        fs::write(&source, "disabledprivatecontent").unwrap();
        let library = Library::open(&database).unwrap();
        library.index_path(&source).unwrap();
        let query = SearchRequest {
            text: "disabledprivatecontent".into(),
            limit: 10,
        };
        let hit = library.search(&query).unwrap().remove(0);
        library.semantic_rebuild().unwrap();
        let connection = rusqlite::Connection::open(&database).unwrap();
        // Model an older/unfenced writer or inconsistent artifact projection. Enabled consent
        // is authoritative even when the artifact's diagnostic state has not been reconciled.
        connection
            .execute("UPDATE source_roots SET enabled = 0", [])
            .unwrap();
        assert!(library.search(&query).unwrap().is_empty());
        assert!(library.inspect_source(&source).is_err());
        assert!(library
            .resolve_verified_artifact_path(&hit.artifact_id, &hit.version_id, &hit.content_hash)
            .is_err());
        assert!(library
            .resolve_verified_evidence(&crate::ResolveEvidenceRequest {
                artifact_id: hit.artifact_id,
                version_id: hit.version_id,
                passage_id: hit.passage_id,
                content_hash: hit.content_hash,
            })
            .is_err());
        assert!(!library.semantic_status().unwrap().healthy);
        assert!(library
            .semantic_search("disabledprivatecontent", 10)
            .is_err());
        assert_eq!(library.semantic_rebuild().unwrap().rebuilt_passages, 0);
        assert!(library.semantic_status().unwrap().healthy);
        assert!(library
            .semantic_search("disabledprivatecontent", 10)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn restore_of_the_same_root_ids_does_not_reauthorize_pre_restore_workers() {
        let directory = tempdir().unwrap();
        let source = directory.path().join("private.md");
        let database = directory.path().join("loom.sqlite3");
        fs::write(&source, "restoredprivatecontent").unwrap();
        let worker = Library::open(&database).unwrap();
        worker.index_path(&source).unwrap();
        let source = source.canonicalize().unwrap();
        let authorization = worker
            .approved_root_authorization(source.to_str().unwrap())
            .unwrap();
        let prepared = ingest::read_stable(&source, &source, 8 * 1024 * 1024).unwrap();
        let controller = Library::open(&database).unwrap();
        let export = controller.export_portable().unwrap();
        controller.purge_root(source.to_str().unwrap()).unwrap();
        controller.import_portable(&export).unwrap();

        let result = worker.index_document_with_extractor(
            &authorization,
            &source,
            prepared,
            "loom.text",
            "0.2.0",
        );
        assert!(
            matches!(result, Err(LoomError::SourceRevoked(_))),
            "restoring identical root IDs/generations must not revive an old worker"
        );
        assert_eq!(controller.export_portable().unwrap().tables, export.tables);
        assert_eq!(controller.index_path(&source).unwrap().unchanged, 1);
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
        } = anchor
        else {
            panic!("text fixture unexpectedly returned a PDF page anchor")
        };
        source
            .chars()
            .skip(*char_start as usize)
            .take((*char_end - *char_start) as usize)
            .collect()
    }
}
