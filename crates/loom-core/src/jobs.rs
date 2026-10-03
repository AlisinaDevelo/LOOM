//! Operational, non-portable background work with typed, source-fenced adapters.
use std::{
    fs::{File, OpenOptions},
    path::Path,
};

use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{error::io_error, Library, LoomError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Queued,
    Running,
    Retryable,
    Failed,
    Cancelled,
    Completed,
}

impl JobState {
    fn parse(value: &str) -> Result<Self> {
        match value {
            "queued" => Ok(Self::Queued),
            "running" => Ok(Self::Running),
            "retryable" => Ok(Self::Retryable),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            "completed" => Ok(Self::Completed),
            _ => Err(LoomError::JobQueue("invalid persisted job state".into())),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobPriority {
    Low,
    Normal,
    High,
}

impl JobPriority {
    fn number(self) -> i64 {
        match self {
            Self::Low => 0,
            Self::Normal => 1,
            Self::High => 2,
        }
    }
    fn parse(value: i64) -> Result<Self> {
        match value {
            0 => Ok(Self::Low),
            1 => Ok(Self::Normal),
            2 => Ok(Self::High),
            _ => Err(LoomError::JobQueue("invalid persisted job priority".into())),
        }
    }
}

/// Admission bounds apply transactionally across connections. Terminal keys remain deduplicable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobQueuePolicy {
    pub max_pending: u32,
    pub max_records: u32,
    pub max_attempts: u32,
    pub priority_burst: u32,
    pub retry_delay_seconds: u32,
}

impl Default for JobQueuePolicy {
    fn default() -> Self {
        Self {
            max_pending: 128,
            max_records: 4096,
            max_attempts: 3,
            priority_burst: 4,
            retry_delay_seconds: 1,
        }
    }
}

impl JobQueuePolicy {
    fn validate(self) -> Result<Self> {
        if !(1..=128).contains(&self.max_pending)
            || !(self.max_pending..=4096).contains(&self.max_records)
            || !(1..=8).contains(&self.max_attempts)
            || !(1..=8).contains(&self.priority_burst)
            || !(1..=3600).contains(&self.retry_delay_seconds)
        {
            return Err(LoomError::JobQueue(
                "policy exceeds supported bounds".into(),
            ));
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackgroundJob {
    pub id: String,
    pub idempotency_key: String,
    pub operation: String,
    pub target_locator: Option<String>,
    pub state: JobState,
    pub priority: JobPriority,
    pub attempts: u32,
    pub max_attempts: u32,
    pub cancel_requested: bool,
    pub ready_at_ms: i64,
    pub last_error: Option<String>,
    pub result: Option<serde_json::Value>,
}

const RUNTIME_SCHEMA: &str = "CREATE TABLE IF NOT EXISTS background_job_runtime(
            slot INTEGER PRIMARY KEY CHECK(slot = 1),
            epoch INTEGER NOT NULL CHECK(epoch >= 0),
            next_sequence INTEGER NOT NULL CHECK(next_sequence >= 1),
            priority_streak INTEGER NOT NULL CHECK(priority_streak BETWEEN 0 AND 8),
            policy_json TEXT NOT NULL
         ) STRICT;
         CREATE TABLE IF NOT EXISTS background_jobs(
            id TEXT PRIMARY KEY,
            idempotency_key TEXT NOT NULL UNIQUE CHECK(length(idempotency_key) BETWEEN 1 AND 128),
            operation TEXT NOT NULL CHECK(operation = 'fts_repair'),
            priority INTEGER NOT NULL CHECK(priority BETWEEN 0 AND 2),
            state TEXT NOT NULL CHECK(state IN ('queued','running','retryable','failed','cancelled','completed')),
            attempts INTEGER NOT NULL CHECK(attempts BETWEEN 0 AND 8),
            max_attempts INTEGER NOT NULL CHECK(max_attempts BETWEEN 1 AND 8),
            sequence INTEGER NOT NULL UNIQUE CHECK(sequence > 0),
            ready_at_ms INTEGER NOT NULL,
            epoch INTEGER,
            claim_token TEXT,
            cancel_requested INTEGER NOT NULL DEFAULT 0 CHECK(cancel_requested IN (0,1)),
            last_error TEXT CHECK(length(CAST(last_error AS BLOB)) <= 4096),
            result_json TEXT CHECK(length(CAST(result_json AS BLOB)) <= 65536 AND json_valid(result_json)),
            CHECK((state = 'running' AND epoch IS NOT NULL AND claim_token IS NOT NULL)
                OR (state != 'running' AND epoch IS NULL AND claim_token IS NULL))
         ) STRICT;
         CREATE INDEX IF NOT EXISTS background_jobs_ready ON background_jobs(state, ready_at_ms, sequence);";

fn current_runtime_schema() -> String {
    RUNTIME_SCHEMA
        .replace("CHECK(operation = 'fts_repair')", "CHECK(operation IN ('fts_repair','index_file'))")
        .replace(
            "CHECK((state = 'running'",
            "target_json TEXT CHECK(length(CAST(target_json AS BLOB)) <= 16384 AND json_valid(target_json)),
            CHECK((operation = 'fts_repair' AND target_json IS NULL)
                OR (operation = 'index_file' AND target_json IS NOT NULL)),
            CHECK((state = 'running'",
        )
}

pub(crate) fn ensure_schema(connection: &Connection) -> Result<()> {
    let transaction =
        rusqlite::Transaction::new_unchecked(connection, TransactionBehavior::Immediate)?;
    let version: Option<String> = transaction
        .query_row(
            "SELECT value FROM schema_meta WHERE key = 'background_job_schema_version'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    if version.is_some() {
        // Unsupported/corrupt runtime does not prevent opening canonical evidence.
        return Ok(());
    }
    let existing: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name = 'background_jobs')",
        [],
        |row| row.get(0),
    )?;
    if existing {
        // Existing runtimes migrate only through an explicit kernel-owned upgrade.
        return Ok(());
    }
    transaction.execute_batch(&current_runtime_schema())?;
    transaction.execute(
        "INSERT INTO background_job_runtime VALUES (1, 0, 1, 0, ?1) ON CONFLICT(slot) DO NOTHING",
        [serde_json::to_string(&JobQueuePolicy::default())?],
    )?;
    transaction.execute(
        "INSERT INTO schema_meta(key,value) VALUES ('background_job_schema_version','4')",
        [],
    )?;
    transaction.commit()?;
    Ok(())
}

/// Called only inside the worker owner's acquisition transaction. Unknown layouts and
/// invalid copied rows abort the whole upgrade, including epoch rotation and recovery.
fn upgrade_runtime(connection: &Connection) -> Result<()> {
    let version: Option<String> = connection
        .query_row(
            "SELECT value FROM schema_meta WHERE key = 'background_job_schema_version'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    if version.as_deref() == Some("4") {
        return validate_schema(connection);
    }
    let recognized = match version.as_deref() {
        Some("3") => validate_definitions(connection, &current_runtime_schema()),
        Some("2") => validate_definitions(connection, RUNTIME_SCHEMA),
        None => validate_definitions(connection, RUNTIME_SCHEMA)
            .or_else(|_| validate_definitions(connection, &legacy_runtime_schema())),
        _ => Err(LoomError::JobQueue("unsupported runtime upgrade".into())),
    };
    recognized?;
    let policy = load_policy(connection)?;
    let count: u32 =
        connection.query_row("SELECT COUNT(*) FROM background_jobs", [], |row| row.get(0))?;
    if count > policy.max_records {
        return Err(LoomError::JobQueue(
            "runtime upgrade exceeds its retained-record budget".into(),
        ));
    }
    connection.execute_batch(
        "ALTER TABLE background_jobs RENAME TO background_jobs_previous;
        DROP INDEX background_jobs_ready;",
    )?;
    connection.execute_batch(&current_runtime_schema())?;
    let copy = if version.as_deref() == Some("3") {
        "INSERT INTO background_jobs SELECT * FROM background_jobs_previous"
    } else {
        "INSERT INTO background_jobs SELECT *, NULL FROM background_jobs_previous"
    };
    // Reapply STRICT/CHECK constraints to every bounded retained row, including v3.
    connection.execute(copy, [])?;
    connection.execute("DROP TABLE background_jobs_previous", [])?;
    connection.execute(
        "INSERT INTO schema_meta(key,value) VALUES ('background_job_schema_version','4')
         ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        [],
    )?;
    validate_schema(connection)
}

fn legacy_runtime_schema() -> String {
    RUNTIME_SCHEMA
        .replace("length(CAST(last_error AS BLOB))", "length(last_error)")
        .replace(
            "length(CAST(result_json AS BLOB)) <= 65536 AND json_valid(result_json)",
            "length(result_json) <= 65536",
        )
}

fn validate_definitions(connection: &Connection, schema: &str) -> Result<()> {
    fn normalized(sql: &str) -> String {
        sql.split_whitespace()
            .collect::<String>()
            .to_ascii_lowercase()
            .replace("ifnotexists", "")
    }
    for definition in schema.split(';').filter(|sql| !sql.trim().is_empty()) {
        let name = if definition.contains("CREATE INDEX") {
            "background_jobs_ready"
        } else if definition.contains("background_job_runtime(") {
            "background_job_runtime"
        } else {
            "background_jobs"
        };
        let actual: Option<String> = connection
            .query_row(
                "SELECT sql FROM sqlite_master WHERE name = ?1",
                [name],
                |row| row.get(0),
            )
            .optional()?;
        if actual.as_deref().map(normalized) != Some(normalized(definition)) {
            return Err(LoomError::JobQueue(format!(
                "missing or unsupported runtime schema: {name}"
            )));
        }
    }
    Ok(())
}

/// Runtime version and definitions are independent of portable canonical schema.
fn validate_runtime_layout(connection: &Connection) -> Result<()> {
    let version: Option<String> = connection
        .query_row(
            "SELECT value FROM schema_meta WHERE key = 'background_job_schema_version'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    if version.as_deref() != Some("4") {
        return Err(LoomError::JobQueue(
            "unsupported or unmigrated runtime schema; explicitly run upgrade-job-runtime".into(),
        ));
    }
    validate_definitions(connection, &current_runtime_schema())?;
    Ok(())
}

pub(crate) fn validate_schema(connection: &Connection) -> Result<()> {
    validate_runtime_layout(connection)?;
    load_policy(connection)?;
    Ok(())
}

fn load_policy(connection: &Connection) -> Result<JobQueuePolicy> {
    let value: String = connection.query_row(
        "SELECT policy_json FROM background_job_runtime WHERE slot = 1",
        [],
        |row| row.get(0),
    )?;
    serde_json::from_str::<JobQueuePolicy>(&value)
        .map_err(|_| LoomError::JobQueue("invalid persisted queue policy".into()))?
        .validate()
}

fn next_sequence(connection: &Connection) -> Result<i64> {
    connection
        .query_row(
            "UPDATE background_job_runtime SET next_sequence = next_sequence + 1
         WHERE slot = 1 AND next_sequence < 9223372036854775807 RETURNING next_sequence - 1",
            [],
            |row| row.get(0),
        )
        .optional()?
        .ok_or_else(|| LoomError::JobQueue("queue sequence exhausted".into()))
}

pub(crate) fn reset_for_restore(connection: &Connection) -> Result<()> {
    advance_epoch(connection)?;
    connection.execute("DELETE FROM background_jobs", [])?;
    connection.execute(
        "UPDATE background_job_runtime SET priority_streak = 0 WHERE slot = 1",
        [],
    )?;
    Ok(())
}

fn purge_runtime_recovery_error() -> LoomError {
    LoomError::JobQueue(
        "deletion refused: no data was deleted because the durable job runtime is unsupported or malformed; use a compatible LOOM release or run the owned upgrade-job-runtime migration before retrying".into(),
    )
}

fn validate_purge_runtime_row(connection: &Connection) -> Result<()> {
    // Validate structural state, not scheduling policy: corrupt policy must not block deletion.
    let state: Option<(i64, i64, i64)> = connection
        .query_row(
            "SELECT epoch, next_sequence, priority_streak FROM background_job_runtime
             WHERE slot=1 AND (SELECT COUNT(*) FROM
                (SELECT slot FROM background_job_runtime LIMIT 2))=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    if state.is_some_and(|(epoch, sequence, streak)| {
        epoch >= 0 && sequence >= 1 && (0..=8).contains(&streak)
    }) {
        Ok(())
    } else {
        Err(purge_runtime_recovery_error())
    }
}

/// Purge must also remove bounded operational locators/diagnostics and invalidate running claims.
/// Known older runtimes contain no file targets; never silently migrate them during deletion.
pub(crate) fn purge_file_targets(
    connection: &Connection,
    locator: Option<&str>,
    artifact_id: Option<&str>,
    images: bool,
) -> Result<()> {
    let version: Option<String> = connection
        .query_row(
            "SELECT value FROM schema_meta WHERE key='background_job_schema_version'",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(|_| purge_runtime_recovery_error())?;
    if version.as_deref() == Some("3") {
        return Err(purge_runtime_recovery_error());
    }
    if version.as_deref() != Some("4") {
        if version.as_deref() == Some("2") {
            validate_definitions(connection, RUNTIME_SCHEMA)
                .map_err(|_| purge_runtime_recovery_error())?;
        } else if version.is_none() {
            validate_definitions(connection, &legacy_runtime_schema())
                .or_else(|_| validate_definitions(connection, RUNTIME_SCHEMA))
                .map_err(|_| purge_runtime_recovery_error())?;
        } else {
            return Err(purge_runtime_recovery_error());
        }
        return Ok(());
    }
    validate_runtime_layout(connection).map_err(|_| purge_runtime_recovery_error())?;
    validate_purge_runtime_row(connection).map_err(|_| purge_runtime_recovery_error())?;
    connection.execute(
        "DELETE FROM background_jobs WHERE operation='index_file' AND (
            (?1 IS NOT NULL AND (
                json_extract(target_json,'$.locator') = ?1
                OR json_extract(target_json,'$.authorization.root_id') IN
                    (SELECT id FROM source_roots WHERE locator=?1)))
            OR (?2 IS NOT NULL AND (
                json_extract(target_json,'$.locator') IN
                    (SELECT locator FROM artifact_locators WHERE artifact_id=?2 AND kind='file')
                OR json_extract(target_json,'$.artifact_id') = ?2))
            OR (?3=1 AND json_extract(target_json,'$.media_type') LIKE 'image/%'))",
        params![locator, artifact_id, images],
    )?;
    Ok(())
}

fn advance_epoch(connection: &Connection) -> Result<i64> {
    connection
        .query_row(
            "UPDATE background_job_runtime SET epoch = epoch + 1
         WHERE slot = 1 AND epoch < 9223372036854775807 RETURNING epoch",
            [],
            |row| row.get(0),
        )
        .optional()?
        .ok_or_else(|| LoomError::JobQueue("worker epoch exhausted".into()))
}

fn get_job(connection: &Connection, id: &str) -> Result<BackgroundJob> {
    let raw = connection.query_row(
        "SELECT id, idempotency_key, operation, state, priority, attempts, max_attempts,
            cancel_requested, ready_at_ms, last_error, result_json, target_json FROM background_jobs WHERE id = ?1",
        [id], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?,
            row.get::<_, String>(2)?, row.get::<_, String>(3)?, row.get::<_, i64>(4)?,
            row.get::<_, u32>(5)?, row.get::<_, u32>(6)?, row.get::<_, bool>(7)?,
            row.get::<_, i64>(8)?, row.get::<_, Option<String>>(9)?, row.get::<_, Option<String>>(10)?,
            row.get::<_, Option<String>>(11)?)),
    ).optional()?.ok_or_else(|| LoomError::JobQueue(format!("job not found: {id}")))?;
    Ok(BackgroundJob {
        id: raw.0,
        idempotency_key: raw.1,
        operation: raw.2,
        // Invalid typed payloads must remain inspectable after a failed dispatch.
        target_locator: raw.11.as_deref().and_then(|json| {
            serde_json::from_str::<crate::store::IndexFileTarget>(json)
                .ok()
                .map(|target| target.locator)
        }),
        state: JobState::parse(&raw.3)?,
        priority: JobPriority::parse(raw.4)?,
        attempts: raw.5,
        max_attempts: raw.6,
        cancel_requested: raw.7,
        ready_at_ms: raw.8,
        last_error: raw.9,
        result: raw
            .10
            .map(|value| serde_json::from_str(&value))
            .transpose()?,
    })
}

impl Library {
    fn queue_connection(&self) -> Result<std::sync::MutexGuard<'_, Connection>> {
        let connection = self.lock()?;
        validate_runtime_layout(&connection)?;
        Ok(connection)
    }

    pub fn job_queue_policy(&self) -> Result<JobQueuePolicy> {
        load_policy(&*self.queue_connection()?)
    }

    /// Policy changes require no pending work and cannot lower the retained-record bound below use.
    pub fn set_job_queue_policy(&self, policy: JobQueuePolicy) -> Result<()> {
        let policy = policy.validate()?;
        let mut connection = self.queue_connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (records, pending): (u32, u32) = transaction.query_row(
            "SELECT COUNT(*), COALESCE(SUM(state IN ('queued','running','retryable')),0) FROM background_jobs",
            [], |row| Ok((row.get(0)?, row.get(1)?)))?;
        if pending > 0 || records > policy.max_records {
            return Err(LoomError::JobQueue(
                "policy cannot change while work is pending or exceeds the new bound".into(),
            ));
        }
        transaction.execute(
            "UPDATE background_job_runtime SET policy_json = ?1 WHERE slot = 1",
            [serde_json::to_string(&policy)?],
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// Identical key/priority returns the original row, even when terminal. Conflicts never rewrite it.
    pub fn enqueue_fts_repair(&self, key: &str, priority: JobPriority) -> Result<BackgroundJob> {
        validate_key(key)?;
        let mut connection = self.queue_connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let job = admit(&transaction, key, priority, "fts_repair", None)?;
        transaction.commit()?;
        Ok(job)
    }

    /// Queue one already-approved regular file. Admission never selects a new source or extracts it.
    pub fn enqueue_index_file(
        &self,
        path: impl AsRef<Path>,
        key: &str,
        priority: JobPriority,
    ) -> Result<BackgroundJob> {
        validate_key(key)?;
        let locator = self.queue_file_locator(path.as_ref())?;
        let mut connection = self.queue_connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let target = crate::store::IndexFileTarget::capture(&transaction, &locator)?;
        let json = serde_json::to_string(&target)?;
        let job = admit(&transaction, key, priority, "index_file", Some(&json))?;
        transaction.commit()?;
        Ok(job)
    }

    pub fn background_job(&self, id: &str) -> Result<BackgroundJob> {
        get_job(&*self.queue_connection()?, id)
    }

    pub fn background_jobs(&self, limit: u32) -> Result<Vec<BackgroundJob>> {
        let mut connection = self.queue_connection()?;
        let transaction = connection.transaction()?;
        let ids = {
            let mut statement = transaction
                .prepare("SELECT id FROM background_jobs ORDER BY sequence DESC LIMIT ?1")?;
            let rows = statement
                .query_map([limit.clamp(1, 128)], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows
        };
        ids.iter().map(|id| get_job(&transaction, id)).collect()
    }

    /// Running cancellation is durable and acknowledged at the worker's next transaction boundary.
    pub fn cancel_background_job(&self, id: &str) -> Result<BackgroundJob> {
        let mut connection = self.queue_connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        get_job(&transaction, id)?;
        transaction.execute(
            "UPDATE background_jobs SET cancel_requested = 1,
                state = CASE WHEN state = 'running' THEN state ELSE 'cancelled' END
             WHERE id = ?1 AND state IN ('queued','running','retryable')",
            [id],
        )?;
        let job = get_job(&transaction, id)?;
        transaction.commit()?;
        Ok(job)
    }

    /// Explicitly forgets a terminal diagnostic record and its deduplication key. Never evicts work.
    pub fn forget_background_job(&self, id: &str) -> Result<BackgroundJob> {
        let mut connection = self.queue_connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let job = get_job(&transaction, id)?;
        if !matches!(
            job.state,
            JobState::Completed | JobState::Failed | JobState::Cancelled
        ) {
            return Err(LoomError::JobQueue(
                "only terminal jobs can be forgotten".into(),
            ));
        }
        let changed = transaction.execute(
            "DELETE FROM background_jobs WHERE id = ?1 AND state IN ('completed','failed','cancelled')", [id])?;
        if changed != 1 {
            return Err(LoomError::JobQueue(
                "terminal record changed before removal".into(),
            ));
        }
        transaction.commit()?;
        Ok(job)
    }

    pub fn acquire_job_worker(&self) -> Result<JobWorker> {
        self.open_job_worker()
    }

    /// Upgrade only operational queue data while holding exclusive worker ownership.
    pub fn upgrade_job_runtime(&self) -> Result<()> {
        JobWorker::upgrade_runtime(self.job_database_path()?)
    }
}

impl JobWorker {
    #[cfg(test)]
    pub(crate) fn claim_for_test(&mut self) -> Result<Option<JobClaim>> {
        self.claim_at(Utc::now().timestamp_millis())
    }
    /// Acquires ownership before opening SQLite; never creates or migrates the library.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_with_limits(path, crate::LibraryLimits::default())
    }

    pub(crate) fn open_with_limits(
        path: impl AsRef<Path>,
        limits: crate::LibraryLimits,
    ) -> Result<Self> {
        Self::open_with_mode(path, limits, false)
    }

    /// Explicitly migrates a recognized operational layout under exclusive worker ownership.
    /// Canonical schema must already be current; unknown layouts and invalid rows roll back.
    pub fn upgrade_runtime(path: impl AsRef<Path>) -> Result<()> {
        Self::open_with_mode(path, crate::LibraryLimits::default(), true).map(drop)
    }

    fn open_with_mode(
        path: impl AsRef<Path>,
        limits: crate::LibraryLimits,
        upgrade: bool,
    ) -> Result<Self> {
        let path = path.as_ref();
        let database = path.canonicalize().map_err(|error| io_error(path, error))?;
        #[cfg(unix)]
        let identity = crate::store::database_file_identity(&database)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if std::fs::metadata(&database)
                .map_err(|error| io_error(&database, error))?
                .nlink()
                != 1
            {
                return Err(LoomError::JobQueue(
                    "hard-linked database aliases are not supported by the worker".into(),
                ));
            }
        }
        let mut name = database
            .file_name()
            .ok_or_else(|| LoomError::JobQueue("database has no filename".into()))?
            .to_os_string();
        name.push(".worker.lock");
        let lock_path = database.with_file_name(name);
        let ownership = open_worker_lock(&lock_path)?;
        fs2::FileExt::try_lock_exclusive(&ownership).map_err(|error| {
            if error.kind() == std::io::ErrorKind::WouldBlock {
                LoomError::JobWorkerBusy
            } else {
                io_error(&lock_path, error)
            }
        })?;
        #[cfg(unix)]
        if crate::store::database_file_identity(&database)? != identity {
            return Err(LoomError::JobQueue(
                "database identity changed during acquisition".into(),
            ));
        }
        let library = Library::open_existing_job_library(&database, limits, !upgrade)?;
        #[cfg(unix)]
        if crate::store::database_file_identity(&database)? != identity {
            return Err(LoomError::JobQueue(
                "database identity changed during opening".into(),
            ));
        }
        let epoch = {
            let mut connection = library.lock()?;
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            if upgrade {
                upgrade_runtime(&transaction)?;
            }
            validate_schema(&transaction)?;
            load_policy(&transaction)?;
            let epoch = advance_epoch(&transaction)?;
            // No timeout takeover. Only a holder of the kernel lock can recover abandoned work.
            transaction.execute(
                "UPDATE background_jobs SET state = CASE WHEN cancel_requested = 1 THEN 'cancelled'
                    WHEN attempts >= max_attempts THEN 'failed' ELSE 'retryable' END,
                    epoch = NULL, claim_token = NULL, ready_at_ms = ?1,
                    last_error = 'worker stopped before completion' WHERE state = 'running'",
                [Utc::now().timestamp_millis()],
            )?;
            transaction.commit()?;
            epoch
        };
        Ok(JobWorker {
            library,
            epoch,
            _ownership: ownership,
            extractor: None,
        })
    }
}

fn validate_key(key: &str) -> Result<()> {
    if key.is_empty()
        || key.len() > 128
        || !key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.:".contains(&byte))
    {
        return Err(LoomError::JobQueue(
            "idempotency key must be 1–128 ASCII identifier bytes".into(),
        ));
    }
    Ok(())
}

fn admit(
    connection: &Connection,
    key: &str,
    priority: JobPriority,
    operation: &str,
    target: Option<&str>,
) -> Result<BackgroundJob> {
    let existing: Option<(String, Option<String>)> = connection
        .query_row(
            "SELECT id, target_json FROM background_jobs WHERE idempotency_key = ?1",
            [key],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    if let Some((id, existing_target)) = existing {
        let job = get_job(connection, &id)?;
        let target_matches = existing_target.as_deref() == target || {
            match (operation, existing_target.as_deref(), target) {
                ("index_file", Some(original), Some(current)) => {
                    crate::store::IndexFileTarget::parse(current)?.same_completed_request(
                        &crate::store::IndexFileTarget::parse(original)?,
                        &job,
                    )
                }
                _ => false,
            }
        };
        if job.operation != operation || job.priority != priority || !target_matches {
            return Err(LoomError::JobQueue(
                "idempotency key has conflicting input".into(),
            ));
        }
        return Ok(job);
    }
    let policy = load_policy(connection)?;
    let (records, pending): (u32, u32) = connection.query_row(
        "SELECT COUNT(*), COALESCE(SUM(state IN ('queued','running','retryable')),0) FROM background_jobs",
        [], |row| Ok((row.get(0)?, row.get(1)?)))?;
    if records >= policy.max_records || pending >= policy.max_pending {
        return Err(LoomError::JobQueue(
            "admission capacity reached; existing jobs were not evicted".into(),
        ));
    }
    let sequence = next_sequence(connection)?;
    let id = Uuid::new_v4().to_string();
    connection.execute(
        "INSERT INTO background_jobs(id,idempotency_key,operation,priority,state,attempts,
            max_attempts,sequence,ready_at_ms,target_json) VALUES (?1,?2,?3,?4,'queued',0,?5,?6,?7,?8)",
        params![id, key, operation, priority.number(), policy.max_attempts, sequence,
            Utc::now().timestamp_millis(), target])?;
    get_job(connection, &id)
}

fn open_worker_lock(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    let file = options.open(path).map_err(|error| io_error(path, error))?;
    if !file
        .metadata()
        .map_err(|error| io_error(path, error))?
        .is_file()
        || std::fs::symlink_metadata(path)
            .map_err(|error| io_error(path, error))?
            .file_type()
            .is_symlink()
    {
        return Err(LoomError::JobQueue(
            "worker lock must be a regular non-symlink file".into(),
        ));
    }
    Ok(file)
}

/// Not cloneable: one owned descriptor and mutable runner per persistent library.
pub struct JobWorker {
    library: Library,
    epoch: i64,
    _ownership: File,
    extractor: Option<loom_extraction::ExtractionSupervisor>,
}

pub(crate) struct JobClaim {
    id: String,
    epoch: i64,
    token: String,
}

impl JobClaim {
    pub(crate) fn verify_operation(
        &self,
        connection: &Connection,
        operation: &str,
        target: Option<&str>,
    ) -> Result<()> {
        self.verify(connection)?;
        let valid: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM background_jobs WHERE id=?1 AND operation=?2 AND target_json IS ?3)",
            params![self.id, operation, target], |row| row.get(0))?;
        if !valid {
            return Err(LoomError::JobClaimStale(self.id.clone()));
        }
        Ok(())
    }

    pub(crate) fn verify(&self, connection: &Connection) -> Result<()> {
        let valid: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM background_jobs j, background_job_runtime r
             WHERE j.id = ?1 AND j.state = 'running' AND j.epoch = ?2 AND j.claim_token = ?3
                AND j.cancel_requested = 0 AND r.slot = 1 AND r.epoch = ?2)",
            params![self.id, self.epoch, self.token],
            |row| row.get(0),
        )?;
        if !valid {
            return Err(LoomError::JobClaimStale(self.id.clone()));
        }
        Ok(())
    }

    pub(crate) fn complete(&self, connection: &Connection, result: &str) -> Result<BackgroundJob> {
        self.verify(connection)?;
        if result.len() > 65_536 {
            return Err(LoomError::JobQueue("job result exceeds 64 KiB".into()));
        }
        let changed = connection.execute(
            "UPDATE background_jobs SET state = 'completed', epoch = NULL, claim_token = NULL,
                result_json = ?1, last_error = NULL WHERE id = ?2 AND state = 'running'
                AND epoch = ?3 AND claim_token = ?4 AND cancel_requested = 0",
            params![result, self.id, self.epoch, self.token],
        )?;
        if changed != 1 {
            return Err(LoomError::JobClaimStale(self.id.clone()));
        }
        get_job(connection, &self.id)
    }
}

impl JobWorker {
    /// Explicit trusted executable configuration for hosts that stage the helper elsewhere.
    /// This is runtime wiring, not persisted source consent; there is no PATH/env fallback.
    pub fn with_extractor_path(mut self, path: impl AsRef<Path>) -> Result<Self> {
        self.extractor = Some(loom_extraction::ExtractionSupervisor::new(path)?);
        Ok(self)
    }

    fn claim_at(&mut self, now: i64) -> Result<Option<JobClaim>> {
        let mut connection = self.library.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (epoch, streak): (i64, u32) = transaction.query_row(
            "SELECT epoch, priority_streak FROM background_job_runtime WHERE slot = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if epoch != self.epoch {
            return Err(LoomError::JobClaimStale("worker epoch".into()));
        }
        let running: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM background_jobs WHERE state = 'running')",
            [],
            |row| row.get(0),
        )?;
        if running {
            return Err(LoomError::JobQueue(
                "finish the running claim before claiming another job".into(),
            ));
        }
        let policy = load_policy(&transaction)?;
        let force_oldest = streak >= policy.priority_burst;
        let query = if force_oldest {
            "SELECT id, priority FROM background_jobs WHERE state IN ('queued','retryable')
                AND ready_at_ms <= ?1 AND cancel_requested = 0 ORDER BY sequence LIMIT 1"
        } else {
            "SELECT id, priority FROM background_jobs WHERE state IN ('queued','retryable')
                AND ready_at_ms <= ?1 AND cancel_requested = 0 ORDER BY priority DESC, sequence LIMIT 1"
        };
        let next: Option<(String, i64)> = transaction
            .query_row(query, [now], |row| Ok((row.get(0)?, row.get(1)?)))
            .optional()?;
        let Some((id, priority)) = next else {
            return Ok(None);
        };
        let token = Uuid::new_v4().to_string();
        let changed = transaction.execute(
            "UPDATE background_jobs SET state = 'running', attempts = attempts + 1,
                epoch = ?1, claim_token = ?2 WHERE id = ?3 AND state IN ('queued','retryable')
                AND attempts < max_attempts AND cancel_requested = 0",
            params![epoch, token, id],
        )?;
        if changed != 1 {
            return Err(LoomError::JobQueue(
                "persisted retry exhausted its attempt budget".into(),
            ));
        }
        transaction.execute(
            "UPDATE background_job_runtime SET priority_streak = ?1 WHERE slot = 1",
            [if priority == 2 && !force_oldest {
                streak + 1
            } else {
                0
            }],
        )?;
        transaction.commit()?;
        Ok(Some(JobClaim { id, epoch, token }))
    }

    #[cfg(test)]
    fn settle_failure(
        &mut self,
        claim: &JobClaim,
        retryable: bool,
        reason: &str,
        now: i64,
    ) -> Result<BackgroundJob> {
        self.settle_outcome(claim, retryable, false, reason, now)
    }

    fn settle_outcome(
        &mut self,
        claim: &JobClaim,
        retryable: bool,
        cancelled: bool,
        reason: &str,
        now: i64,
    ) -> Result<BackgroundJob> {
        let mut connection = self.library.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let valid: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM background_jobs j, background_job_runtime r
             WHERE j.id = ?1 AND j.state = 'running' AND j.epoch = ?2 AND j.claim_token = ?3
                AND r.slot = 1 AND r.epoch = ?2)",
            params![claim.id, claim.epoch, claim.token],
            |row| row.get(0),
        )?;
        if !valid {
            return Err(LoomError::JobClaimStale(claim.id.clone()));
        }
        let policy = load_policy(&transaction)?;
        let sequence = next_sequence(&transaction)?;
        let mut reason = reason.to_owned();
        if reason.len() > 4096 {
            let mut boundary = 4096;
            while !reason.is_char_boundary(boundary) {
                boundary -= 1;
            }
            reason.truncate(boundary);
        }
        let due = now
            .checked_add(i64::from(policy.retry_delay_seconds) * 1000)
            .ok_or_else(|| LoomError::JobQueue("retry deadline overflow".into()))?;
        transaction.execute(
            "UPDATE background_jobs SET state = CASE WHEN cancel_requested = 1 OR ?6 = 1 THEN 'cancelled'
                WHEN ?1 = 1 AND attempts < max_attempts THEN 'retryable' ELSE 'failed' END,
                epoch = NULL, claim_token = NULL, last_error = ?2, ready_at_ms = ?3, sequence = ?4
             WHERE id = ?5",
            params![retryable, reason, due, sequence, claim.id, cancelled],
        )?;
        let settled = get_job(&transaction, &claim.id)?;
        transaction.commit()?;
        Ok(settled)
    }

    /// Runs at most one due typed unit. Publication and completion share a fenced transaction.
    pub fn run_next(&mut self) -> Result<Option<BackgroundJob>> {
        self.run_next_inner(|_| {})
    }

    fn run_next_inner(
        &mut self,
        after_settlement: impl FnOnce(&BackgroundJob),
    ) -> Result<Option<BackgroundJob>> {
        let Some(claim) = self.claim_at(Utc::now().timestamp_millis())? else {
            return Ok(None);
        };
        let execution = (|| {
            let (operation, target): (String, Option<String>) = {
                let connection = self.library.lock()?;
                claim.verify(&connection)?;
                connection.query_row(
                    "SELECT operation, target_json FROM background_jobs WHERE id=?1",
                    [&claim.id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )?
            };
            match (operation.as_str(), target.as_deref()) {
                ("fts_repair", None) => self.library.job_repair_fts(&claim),
                ("index_file", Some(json)) => {
                    self.library
                        .job_index_file(&claim, json, self.extractor.as_ref())
                }
                _ => Err(LoomError::JobQueue("invalid operation or target".into())),
            }
        })();
        let settled = match execution {
            Ok(completed) => completed,
            Err(error) => self.settle_outcome(
                &claim,
                is_retryable(&error),
                matches!(
                    error,
                    LoomError::SourceRevoked(_)
                        | LoomError::OcrPolicyChanged
                        | LoomError::OcrDisabled
                ),
                &error.to_string(),
                Utc::now().timestamp_millis(),
            )?,
        };
        after_settlement(&settled);
        Ok(Some(settled))
    }
}

fn is_retryable(error: &LoomError) -> bool {
    match error {
        LoomError::SourceChanged(_) => true,
        LoomError::Extraction(
            loom_extraction::ExtractionError::Launch
            | loom_extraction::ExtractionError::ChildCrashed,
        ) => true,
        LoomError::Database(rusqlite::Error::SqliteFailure(code, _)) => matches!(
            code.code,
            rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
        ),
        LoomError::Io { source, .. } => matches!(
            source.kind(),
            std::io::ErrorKind::Interrupted
                | std::io::ErrorKind::WouldBlock
                | std::io::ErrorKind::TimedOut
        ),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        process::{Child, Command, Stdio},
        sync::{Arc, Barrier},
        time::{Duration, Instant},
    };
    use tempfile::{tempdir, TempDir};

    #[test]
    fn fresh_queue_requires_supervised_extraction_runtime() {
        let (_directory, library) = fixture();
        let connection = library.lock().unwrap();
        let version: String = connection
            .query_row(
                "SELECT value FROM schema_meta WHERE key='background_job_schema_version'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(version, "4", "old workers must refuse before claiming work");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn persistent_retrieval_during_native_queued_ocr() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let (directory, library) = fixture();
        let corpus = directory.path().join("corpus");
        fs::create_dir(&corpus).unwrap();
        for index in 0..256 {
            fs::write(
                corpus.join(format!("source-{index:04}.md")),
                format!("Background responsiveness source marker {index:04}.\n"),
            )
            .unwrap();
        }
        library.set_ocr_enabled(false).unwrap();
        assert_eq!(library.index_path(&corpus).unwrap().indexed, 256);
        let mut images = Vec::new();
        for index in 0..24 {
            let image = corpus.join(format!("cropped-{index:02}.png"));
            fs::write(
                &image,
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../benchmarks/retrieval/v1/corpus/screenshot/ocr-cropped.png"
                )),
            )
            .unwrap();
            assert_eq!(library.index_path(&image).unwrap().skipped, 1);
            images.push(image);
        }
        library.set_ocr_enabled(true).unwrap();
        let ids = images
            .iter()
            .enumerate()
            .map(|(index, image)| {
                library
                    .enqueue_index_file(image, &format!("response-{index}"), JobPriority::Normal)
                    .unwrap()
                    .id
            })
            .collect::<Vec<_>>();
        let request = crate::SearchRequest {
            text: "\"Background responsiveness source marker\"".into(),
            limit: 5,
        };
        let tuples = |hits: Vec<crate::SearchHit>| {
            hits.into_iter()
                .map(|hit| {
                    (
                        hit.artifact_id,
                        hit.version_id,
                        hit.passage_id,
                        hit.content_hash,
                        hit.anchor,
                    )
                })
                .collect::<Vec<_>>()
        };
        let expected = tuples(library.search(&request).unwrap());
        assert_eq!(expected.len(), 5);
        // One interactive Library stays open throughout. No CLI startup,
        // canonical migration or FTS rebuilding occurs inside any measurement.
        let retrieval = || {
            let start = Instant::now();
            let hits = library.search(&request).unwrap();
            let elapsed = start.elapsed().as_secs_f64() * 1000.0;
            assert_eq!(tuples(hits), expected);
            elapsed
        };
        let baseline = (0..50).map(|_| retrieval()).collect::<Vec<_>>();
        let observations = Connection::open_with_flags(
            directory.path().join("queue.sqlite3"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        observations
            .busy_timeout(Duration::from_millis(100))
            .unwrap();
        let running_claim = || {
            observations.query_row(
                "SELECT id,epoch,claim_token FROM background_jobs WHERE state='running' LIMIT 1",
                [], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, String>(2)?)),
            ).optional().unwrap()
        };
        assert!(running_claim().is_none());
        let stop = AtomicBool::new(false);
        let cleanup_library =
            Library::open_for_jobs(directory.path().join("queue.sqlite3")).unwrap();
        cleanup_library
            .lock()
            .unwrap()
            .busy_timeout(Duration::ZERO)
            .unwrap();
        struct StopOnDrop<'a> {
            stop: &'a AtomicBool,
            library: &'a Library,
            ids: &'a [String],
        }
        impl Drop for StopOnDrop<'_> {
            fn drop(&mut self) {
                self.stop.store(true, Ordering::Release);
                let start = Instant::now();
                for id in self.ids {
                    loop {
                        match self.library.cancel_background_job(id) {
                            Err(error)
                                if is_retryable(&error)
                                    && start.elapsed() < Duration::from_millis(500) =>
                            {
                                std::thread::sleep(Duration::from_millis(5));
                            }
                            _ => break,
                        }
                    }
                }
            }
        }
        let mut worker = test_worker(&library).unwrap();
        let (overlapping, jobs) = std::thread::scope(|scope| {
            let thread = scope.spawn(|| {
                let mut jobs = Vec::new();
                while !stop.load(Ordering::Acquire) {
                    match worker.run_next().unwrap() {
                        Some(job) => {
                            println!("cohort job {}: {:?}", jobs.len() + 1, job.state);
                            jobs.push(job);
                        }
                        None => break,
                    }
                }
                jobs
            });
            // Drop inside the scope before join, including measurement panic:
            // revoke pending/running work so a failed test cannot strand helpers.
            let _cleanup = StopOnDrop {
                stop: &stop,
                library: &cleanup_library,
                ids: &ids,
            };
            let start = Instant::now();
            let mut last_progress = Instant::now();
            let mut overlapping = Vec::new();
            while !thread.is_finished() {
                assert!(
                    start.elapsed() < Duration::from_secs(120),
                    "OCR cohort deadline"
                );
                let before = running_claim();
                let elapsed = retrieval();
                let after = running_claim();
                if last_progress.elapsed() > Duration::from_secs(10) {
                    println!(
                        "cohort elapsed {:?}; running {:?}; overlap {}",
                        start.elapsed(),
                        after,
                        overlapping.len()
                    );
                    last_progress = Instant::now();
                }
                if before.is_some() && before == after {
                    overlapping.push(elapsed);
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            (overlapping, thread.join().unwrap())
        });
        assert_eq!(jobs.len(), 24);
        for (job, id) in jobs.iter().zip(&ids) {
            assert_eq!(&job.id, id);
            assert_eq!(job.state, JobState::Completed, "{job:?}");
            assert_eq!(job.attempts, 1);
        }
        let summary = |samples: &[f64]| {
            assert!(!samples.is_empty());
            let mut ordered = samples.to_vec();
            ordered.sort_by(f64::total_cmp);
            serde_json::json!({ "samples": ordered.len(),
                "median_ms": (ordered[(ordered.len()-1)/2] + ordered[ordered.len()/2]) / 2.0,
                "p95_ms": ordered[(ordered.len()*95).div_ceil(100)-1], "max_ms": ordered.last().unwrap() })
        };
        let report = serde_json::json!({
            "scope": "persistent Library lexical search; 256 synthetic text files; 24 explicitly selected cropped OCR images",
            "baseline": summary(&baseline), "same_claim_overlap": summary(&overlapping),
            "completed_ocr_jobs": jobs.len(),
            "extraction": jobs.iter().map(|job| job.result.as_ref().unwrap()["extraction"].clone()).collect::<Vec<_>>(),
            "smoke_gate": { "minimum_same_claim_samples": 20, "p95_ms_ceiling": 25 },
            "not_proven": ["desktop responsiveness", "100k corpus responsiveness", "hard peak RSS", "Vision service memory"]
        });
        println!("queued persistent retrieval: {report}");
        if let Some(path) = std::env::var_os("LOOM_TEST_RESPONSE_REPORT") {
            fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
        }
        assert!(
            overlapping.len() >= 20,
            "insufficient verified overlap samples"
        );
        assert!(
            report["same_claim_overlap"]["p95_ms"].as_f64().unwrap() < 25.0,
            "{report}"
        );
        assert!(!library
            .search(&crate::SearchRequest {
                text: "LOOM OCR marker".into(),
                limit: 5
            })
            .unwrap()
            .is_empty());
    }

    fn fixture() -> (TempDir, Library) {
        let directory = tempdir().unwrap();
        let library = Library::open(directory.path().join("queue.sqlite3")).unwrap();
        (directory, library)
    }

    // Unit race/state fixtures use the Cargo-managed test helper. The
    // extractor_helper integration test makes Cargo build it in this target
    // directory/profile; never search PATH or fall back to an in-process
    // provider.
    fn test_extractor_path() -> std::path::PathBuf {
        let helper_name = format!("loom-core-test-extractor{}", std::env::consts::EXE_SUFFIX);
        let path = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join(helper_name);
        assert!(
            path.is_file(),
            "Cargo did not build the core test extractor at {}; run cargo test -p loom-core without --lib",
            path.display()
        );
        path
    }

    fn test_extractor() -> loom_extraction::ExtractionSupervisor {
        loom_extraction::ExtractionSupervisor::new(test_extractor_path()).unwrap()
    }

    fn test_worker(library: &Library) -> Result<JobWorker> {
        library
            .acquire_job_worker()?
            .with_extractor_path(test_extractor_path())
    }

    fn reacquire_after_deliberate_test_drop(library: &Library) -> Result<JobWorker> {
        after_deliberate_test_drop(|| test_worker(library))
    }

    fn after_deliberate_test_drop<T>(mut operation: impl FnMut() -> Result<T>) -> Result<T> {
        // Other test threads can fork with a transient copy of the CLOEXEC lock
        // descriptor. This applies only after a known owner release; production
        // acquisition and all live-contention assertions remain immediate.
        let start = Instant::now();
        loop {
            match operation() {
                Err(LoomError::JobWorkerBusy) if start.elapsed() < Duration::from_millis(500) => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                result => return result,
            }
        }
    }

    #[cfg(unix)]
    fn fault_extractor(mode: &str) -> (TempDir, std::path::PathBuf, std::path::PathBuf) {
        let directory = tempdir().unwrap();
        let pid = directory.path().join("pid");
        let source = directory.path().join("fault.rs");
        let executable = directory.path().join("fault");
        fs::write(
            &source,
            format!(
                "const PID_FILE: &str = {:?};\nconst MODE: &str = {mode:?};\nconst HELPER: &str = {:?};\n{}",
                pid.to_str().unwrap(),
                test_extractor_path().to_str().unwrap(),
                include_str!("../../loom-extraction/tests/support/fault.rs")
            ),
        )
        .unwrap();
        let output = Command::new("rustc")
            .arg("--edition=2021")
            .arg(source)
            .arg("-o")
            .arg(&executable)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(Command::new(&executable)
            .arg("--warmup")
            .status()
            .unwrap()
            .success());
        assert!(!pid.exists(), "warmup must not count as fault entry");
        (directory, executable, pid)
    }

    #[cfg(unix)]
    fn wait_for_fixture(pid: &Path) {
        let start = Instant::now();
        while !pid.exists() {
            assert!(
                start.elapsed() < Duration::from_secs(3),
                "child never entered main"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[cfg(unix)]
    fn assert_fixture_reaped(pid: &Path) {
        let pid = fs::read_to_string(pid).unwrap();
        assert!(
            Command::new("/bin/ps")
                .args(["-p", &pid, "-o", "pid="])
                .output()
                .unwrap()
                .stdout
                .is_empty(),
            "extractor remains alive or zombie"
        );
    }

    #[cfg(unix)]
    #[test]
    fn live_extraction_fences_cancel_revoke_purge_restore_target_and_foreground_cas() {
        for action in [
            "cancel",
            "revoke",
            "purge",
            "root-purge",
            "restore",
            "target",
            "cas",
        ] {
            let (_directory, library, source) = file_fixture();
            let restore = library.export_portable().unwrap();
            fs::write(&source, "Pending private-synthetic refresh.").unwrap();
            let job = library
                .enqueue_index_file(&source, "live-fence", JobPriority::Normal)
                .unwrap();
            let (_fault_dir, executable, pid) = fault_extractor("hang");
            let mut worker = library
                .acquire_job_worker()
                .unwrap()
                .with_extractor_path(executable)
                .unwrap();
            let started = Instant::now();
            let thread = std::thread::spawn(move || worker.run_next());
            wait_for_fixture(&pid);
            assert_eq!(
                library.background_job(&job.id).unwrap().state,
                JobState::Running
            );
            match action {
                "cancel" => {
                    library.cancel_background_job(&job.id).unwrap();
                }
                "revoke" => {
                    library
                        .revoke_source_root(source.to_str().unwrap())
                        .unwrap();
                }
                "purge" => {
                    library
                        .purge_artifact(&file_identity(&library, &source).0)
                        .unwrap();
                }
                "root-purge" => {
                    library.purge_root(source.to_str().unwrap()).unwrap();
                }
                "restore" => {
                    // Fixture-only empty-library precondition; preserve operational rows so
                    // import/epoch invalidation is tested, not public purge cleanup alone.
                    library
                        .lock()
                        .unwrap()
                        .execute_batch("DELETE FROM artifacts; DELETE FROM source_roots;")
                        .unwrap();
                    library.import_portable(&restore).unwrap();
                }
                "target" => {
                    library.lock().unwrap().execute("UPDATE background_jobs SET target_json=json_set(target_json,'$.locator','/wrong.md') WHERE id=?1", [&job.id]).unwrap();
                }
                "cas" => {
                    library.index_path(&source).unwrap();
                }
                _ => unreachable!(),
            }
            let after_controller = library.export_portable().unwrap().digest;
            let outcome = thread.join().unwrap();
            assert!(
                started.elapsed() < Duration::from_secs(3),
                "{action}: probe did not interrupt promptly"
            );
            assert_fixture_reaped(&pid);
            assert_eq!(
                library.export_portable().unwrap().digest,
                after_controller,
                "{action}: stale child published"
            );
            match action {
                "purge" | "root-purge" => assert!(outcome.is_err(), "{action}: {outcome:?}"),
                "restore" => {
                    // The monitor may see the empty precondition before import commits.
                    assert!(
                        outcome.is_err() || outcome.unwrap().unwrap().state == JobState::Cancelled
                    );
                    assert!(library.background_jobs(128).unwrap().is_empty());
                }
                "cancel" | "revoke" => {
                    assert_eq!(outcome.unwrap().unwrap().state, JobState::Cancelled)
                }
                "target" => assert_eq!(outcome.unwrap().unwrap().state, JobState::Failed),
                "cas" => assert_eq!(outcome.unwrap().unwrap().state, JobState::Retryable),
                _ => unreachable!(),
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn provider_unavailability_is_failure_not_consent_cancellation_and_does_not_publish() {
        let (_directory, library, source) = file_fixture();
        let before = library.export_portable().unwrap().digest;
        fs::write(
            &source,
            "Never publish this private-synthetic provider failure.",
        )
        .unwrap();
        library
            .enqueue_index_file(&source, "provider-unavailable", JobPriority::Normal)
            .unwrap();
        let (_fault_dir, executable, pid) = fault_extractor("unavailable");
        let failed = library
            .acquire_job_worker()
            .unwrap()
            .with_extractor_path(executable)
            .unwrap()
            .run_next()
            .unwrap()
            .unwrap();
        assert_eq!(failed.state, JobState::Failed);
        assert_eq!(failed.attempts, 1);
        assert_eq!(
            failed.last_error.as_deref(),
            Some("OCR is unavailable: local provider is unavailable")
        );
        assert_eq!(library.export_portable().unwrap().digest, before);
        assert_fixture_reaped(&pid);
        assert!(!is_retryable(&LoomError::OcrUnavailable(
            "unavailable".into()
        )));
        for error in [
            loom_extraction::ExtractionError::WallTime,
            loom_extraction::ExtractionError::CpuTime,
            loom_extraction::ExtractionError::Memory,
            loom_extraction::ExtractionError::OutputLimit,
        ] {
            assert!(!is_retryable(&LoomError::Extraction(error)));
        }
    }

    #[cfg(unix)]
    #[test]
    fn live_image_policy_rotation_and_purge_stop_blocked_provider_input() {
        for action in ["reassert", "disable", "purge"] {
            let (directory, library) = fixture();
            let source = directory.path().join("policy.png");
            fs::write(
                &source,
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../tests/fixtures/ocr-golden.png"
                )),
            )
            .unwrap();
            library.set_ocr_enabled(false).unwrap();
            library.index_path(&source).unwrap();
            library.set_ocr_enabled(true).unwrap();
            let job = library
                .enqueue_index_file(&source, "live-ocr-policy", JobPriority::Normal)
                .unwrap();
            let (_fault_dir, executable, pid) = fault_extractor("hang");
            let mut worker = library
                .acquire_job_worker()
                .unwrap()
                .with_extractor_path(executable)
                .unwrap();
            let thread = std::thread::spawn(move || worker.run_next());
            wait_for_fixture(&pid);
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
            let after = library.export_portable().unwrap().digest;
            let outcome = thread.join().unwrap();
            assert_fixture_reaped(&pid);
            assert_eq!(library.export_portable().unwrap().digest, after);
            assert_eq!(library.stats().unwrap().artifacts, 0);
            if matches!(action, "purge" | "disable") {
                assert!(outcome.is_err());
                assert!(library.background_job(&job.id).is_err());
            } else {
                assert_eq!(outcome.unwrap().unwrap().state, JobState::Cancelled);
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn actual_helper_crashes_retry_only_to_budget_and_memory_overrun_fails_without_publication() {
        for mode in ["crash", "memory", "oversize-output"] {
            let (_directory, library, source) = file_fixture();
            let before = library.export_portable().unwrap().digest;
            fs::write(&source, "Never publish faulting provider evidence.").unwrap();
            library
                .enqueue_index_file(&source, "fault-budget", JobPriority::Normal)
                .unwrap();
            let (_fault_dir, executable, pid) = fault_extractor(mode);
            let mut worker = library
                .acquire_job_worker()
                .unwrap()
                .with_extractor_path(executable)
                .unwrap();
            for attempt in 1..=if mode == "crash" { 3 } else { 1 } {
                let settled = worker.run_next().unwrap().unwrap();
                assert_eq!(settled.attempts, attempt);
                assert_eq!(
                    settled.state,
                    if mode == "crash" && attempt < 3 {
                        JobState::Retryable
                    } else {
                        JobState::Failed
                    },
                    "{mode}: {settled:?}"
                );
                assert_fixture_reaped(&pid);
                assert_eq!(library.export_portable().unwrap().digest, before);
                if mode == "memory" {
                    assert!(settled
                        .last_error
                        .as_deref()
                        .unwrap()
                        .contains("sampled memory"));
                } else if mode == "oversize-output" {
                    assert!(settled
                        .last_error
                        .as_deref()
                        .unwrap()
                        .contains("output limit"));
                }
                if settled.state == JobState::Retryable {
                    assert!(
                        worker.run_next().unwrap().is_none(),
                        "persisted retry delay was bypassed"
                    );
                    std::thread::sleep(Duration::from_millis(1100));
                }
            }
            assert!(worker.run_next().unwrap().is_none());
        }
    }

    #[test]
    fn v3_runtime_upgrade_preserves_typed_work_and_refuses_live_or_invalid_layouts() {
        let (_directory, library, source) = file_fixture();
        let queued = library
            .enqueue_index_file(&source, "v3-file", JobPriority::Low)
            .unwrap();
        let before = library.export_portable().unwrap().digest;
        let worker = test_worker(&library).unwrap();
        library
            .lock()
            .unwrap()
            .execute(
                "UPDATE schema_meta SET value='3' WHERE key='background_job_schema_version'",
                [],
            )
            .unwrap();
        assert!(matches!(
            library.upgrade_job_runtime(),
            Err(LoomError::JobWorkerBusy)
        ));
        drop(worker);
        assert!(after_deliberate_test_drop(|| library.acquire_job_worker()).is_err());
        after_deliberate_test_drop(|| library.upgrade_job_runtime()).unwrap();
        assert_eq!(library.background_job(&queued.id).unwrap(), queued);
        library.upgrade_job_runtime().unwrap();
        assert_eq!(library.background_job(&queued.id).unwrap(), queued);
        assert_eq!(library.export_portable().unwrap().digest, before);
        library.lock().unwrap().execute_batch("UPDATE schema_meta SET value='3' WHERE key='background_job_schema_version'; PRAGMA ignore_check_constraints=ON;").unwrap();
        library
            .lock()
            .unwrap()
            .execute(
                "UPDATE background_jobs SET last_error=?1 WHERE id=?2",
                params!["x".repeat(4097), queued.id],
            )
            .unwrap();
        library
            .lock()
            .unwrap()
            .execute_batch("PRAGMA ignore_check_constraints=OFF;")
            .unwrap();
        assert!(library.upgrade_job_runtime().is_err());
        let connection = library.lock().unwrap();
        assert_eq!(
            connection
                .query_row(
                    "SELECT value FROM schema_meta WHERE key='background_job_schema_version'",
                    [],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
            "3"
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT length(last_error) FROM background_jobs WHERE id=?1",
                    [&queued.id],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            4097
        );
    }

    fn now() -> i64 {
        Utc::now().timestamp_millis() + 60_000
    }

    fn complete(worker: &JobWorker, claim: &JobClaim) {
        let mut connection = worker.library.lock().unwrap();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        claim.complete(&transaction, "{}").unwrap();
        transaction.commit().unwrap();
    }

    fn file_fixture() -> (TempDir, Library, std::path::PathBuf) {
        let (directory, library) = fixture();
        let source = directory.path().join("approved.md");
        fs::write(&source, "Original exact evidence marker.").unwrap();
        library.index_path(&source).unwrap();
        (directory, library, source.canonicalize().unwrap())
    }

    fn claim_file(worker: &mut JobWorker) -> (JobClaim, String) {
        let claim = worker.claim_at(now()).unwrap().unwrap();
        let json = worker
            .library
            .lock()
            .unwrap()
            .query_row(
                "SELECT target_json FROM background_jobs WHERE id=?1",
                [&claim.id],
                |row| row.get(0),
            )
            .unwrap();
        (claim, json)
    }

    fn file_identity(library: &Library, source: &Path) -> (String, String) {
        library
            .lock()
            .unwrap()
            .query_row(
                "SELECT a.id, a.active_version_id FROM artifacts a JOIN artifact_locators l
             ON l.artifact_id=a.id WHERE l.kind='file' AND l.locator=?1",
                [source.to_str().unwrap()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap()
    }

    #[test]
    fn approved_file_admission_is_bounded_atomic_and_never_grants_scope() {
        let (directory, library, source) = file_fixture();
        let before = library.export_portable().unwrap().digest;
        let unknown = directory.path().join("unapproved.md");
        fs::write(&unknown, "private unapproved marker").unwrap();
        assert!(matches!(
            library.enqueue_index_file(&unknown, "unapproved", JobPriority::Normal),
            Err(LoomError::SourceRevoked(_))
        ));
        assert!(library
            .enqueue_index_file(directory.path(), "directory", JobPriority::Normal)
            .is_err());
        #[cfg(unix)]
        {
            let alias = directory.path().join("alias.md");
            std::os::unix::fs::symlink(&source, &alias).unwrap();
            assert!(library
                .enqueue_index_file(alias, "alias", JobPriority::Normal)
                .is_err());
        }
        let other = Library::open(directory.path().join("queue.sqlite3")).unwrap();
        let other_source = source.clone();
        let barrier = Arc::new(Barrier::new(2));
        let second_barrier = Arc::clone(&barrier);
        let task = std::thread::spawn(move || {
            second_barrier.wait();
            other
                .enqueue_index_file(other_source, "one-file", JobPriority::Normal)
                .unwrap()
        });
        barrier.wait();
        let first = library
            .enqueue_index_file(&source, "one-file", JobPriority::Normal)
            .unwrap();
        assert_eq!(first, task.join().unwrap());
        assert_eq!(first.target_locator.as_deref(), source.to_str());
        assert_eq!(library.background_jobs(128).unwrap().len(), 1);
        assert!(library
            .enqueue_fts_repair("one-file", JobPriority::Normal)
            .is_err());
        assert!(library
            .enqueue_index_file(&source, "one-file", JobPriority::High)
            .is_err());
        assert_eq!(library.export_portable().unwrap().digest, before);
    }

    #[test]
    fn real_file_adapter_has_one_parent_authority_and_stable_artifact_identity() {
        let (_directory, library, source) = file_fixture();
        let before = file_identity(&library, &source);
        let checkpoints: i64 = library
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM index_jobs", [], |row| row.get(0))
            .unwrap();
        fs::write(&source, "Refreshed queued evidence marker.").unwrap();
        let queued = library
            .enqueue_index_file(&source, "refresh", JobPriority::Normal)
            .unwrap();
        let mut worker = test_worker(&library).unwrap();
        let completed = worker.run_next().unwrap().unwrap();
        assert_eq!(completed.id, queued.id);
        assert_eq!(completed.state, JobState::Completed);
        assert_eq!(completed.result.as_ref().unwrap()["indexed"], true);
        let after = file_identity(&library, &source);
        assert_eq!(before.0, after.0);
        assert_ne!(before.1, after.1);
        assert_eq!(
            library
                .lock()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM index_jobs", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            checkpoints
        );
        assert_eq!(
            library
                .enqueue_index_file(&source, "refresh", JobPriority::Normal)
                .unwrap(),
            completed
        );
        library
            .enqueue_index_file(&source, "unchanged", JobPriority::Normal)
            .unwrap();
        assert_eq!(
            worker.run_next().unwrap().unwrap().result.unwrap()["indexed"],
            false
        );
        assert_eq!(file_identity(&library, &source).1, after.1);
        assert_eq!(
            fs::read_to_string(source).unwrap(),
            "Refreshed queued evidence marker."
        );
    }

    #[test]
    fn file_publication_and_parent_completion_roll_back_together() {
        let (_directory, library, source) = file_fixture();
        let before = library.export_portable().unwrap().digest;
        fs::write(&source, "Atomic publication marker.").unwrap();
        library
            .enqueue_index_file(&source, "atomic", JobPriority::Normal)
            .unwrap();
        let mut worker = test_worker(&library).unwrap();
        let (claim, json) = claim_file(&mut worker);
        let prepared = worker
            .library
            .prepare_file_job(&claim, &json, Some(&test_extractor()))
            .unwrap();
        library.lock().unwrap().execute_batch("CREATE TRIGGER fail_job_completion BEFORE UPDATE OF state ON background_jobs WHEN NEW.state='completed' BEGIN SELECT RAISE(ABORT,'fixture rollback'); END;").unwrap();
        assert!(worker.library.publish_file_job(&claim, &prepared).is_err());
        assert_eq!(library.export_portable().unwrap().digest, before);
        assert_eq!(
            library.background_job(&claim.id).unwrap().state,
            JobState::Running
        );
        library
            .lock()
            .unwrap()
            .execute_batch("DROP TRIGGER fail_job_completion;")
            .unwrap();
        assert_eq!(
            worker
                .library
                .publish_file_job(&claim, &prepared)
                .unwrap()
                .state,
            JobState::Completed
        );
        assert_ne!(library.export_portable().unwrap().digest, before);
    }

    #[test]
    fn prepared_file_cancellation_cannot_publish_and_is_durably_acknowledged() {
        let (_directory, library, source) = file_fixture();
        fs::write(&source, "Cancelled prepared evidence.").unwrap();
        let before = library.export_portable().unwrap().digest;
        let job = library
            .enqueue_index_file(&source, "cancel-prepared", JobPriority::Normal)
            .unwrap();
        let mut worker = test_worker(&library).unwrap();
        let (claim, json) = claim_file(&mut worker);
        let prepared = worker
            .library
            .prepare_file_job(&claim, &json, Some(&test_extractor()))
            .unwrap();
        library.cancel_background_job(&job.id).unwrap();
        assert!(matches!(
            worker.library.publish_file_job(&claim, &prepared),
            Err(LoomError::JobClaimStale(_))
        ));
        let settled = worker
            .settle_failure(&claim, true, "cancelled fixture", now())
            .unwrap();
        assert_eq!(settled.state, JobState::Cancelled);
        assert_eq!(library.export_portable().unwrap().digest, before);
    }

    #[test]
    fn reselected_file_does_not_reauthorize_an_admitted_capability() {
        let (_directory, library, source) = file_fixture();
        let job = library
            .enqueue_index_file(&source, "old-capability", JobPriority::Normal)
            .unwrap();
        library
            .revoke_source_root(source.to_str().unwrap())
            .unwrap();
        fs::write(&source, "New selected source evidence.").unwrap();
        library.index_path(&source).unwrap();
        let before = library.export_portable().unwrap().digest;
        assert!(library
            .enqueue_index_file(&source, "old-capability", JobPriority::Normal)
            .is_err());
        let mut worker = test_worker(&library).unwrap();
        let cancelled = worker.run_next().unwrap().unwrap();
        assert_eq!(cancelled.id, job.id);
        assert_eq!(cancelled.state, JobState::Cancelled);
        assert_eq!(library.export_portable().unwrap().digest, before);
        library
            .enqueue_index_file(&source, "new-capability", JobPriority::Normal)
            .unwrap();
        assert_eq!(
            worker.run_next().unwrap().unwrap().state,
            JobState::Completed
        );
    }

    #[test]
    fn source_hash_and_foreground_cas_both_fence_prepared_file_publication() {
        let (_directory, library, source) = file_fixture();
        fs::write(&source, "Prepared older source.").unwrap();
        library
            .enqueue_index_file(&source, "stale-source", JobPriority::Normal)
            .unwrap();
        let mut worker = test_worker(&library).unwrap();
        let (claim, json) = claim_file(&mut worker);
        let prepared = worker
            .library
            .prepare_file_job(&claim, &json, Some(&test_extractor()))
            .unwrap();
        let before = library.export_portable().unwrap().digest;
        fs::write(&source, "Foreground newer source.").unwrap();
        assert!(matches!(
            worker.library.publish_file_job(&claim, &prepared),
            Err(LoomError::SourceChanged(_))
        ));
        assert_eq!(library.export_portable().unwrap().digest, before);
        library.index_path(&source).unwrap();
        let foreground = library.export_portable().unwrap().digest;
        // Restore the earlier bytes so this refusal comes from canonical CAS, not the hash fence.
        fs::write(&source, "Prepared older source.").unwrap();
        assert!(matches!(
            worker.library.publish_file_job(&claim, &prepared),
            Err(LoomError::SourceChanged(_))
        ));
        assert_eq!(library.export_portable().unwrap().digest, foreground);
        assert!(is_retryable(&LoomError::SourceChanged("fixture".into())));
        assert_eq!(
            worker
                .settle_failure(&claim, true, "CAS changed", now())
                .unwrap()
                .state,
            JobState::Retryable
        );
    }

    #[test]
    fn file_hash_change_while_waiting_for_sqlite_writer_cannot_publish() {
        let (directory, library, source) = file_fixture();
        fs::write(&source, "Prepared before lock contention.").unwrap();
        library
            .enqueue_index_file(&source, "writer-contention", JobPriority::Normal)
            .unwrap();
        let mut worker = test_worker(&library).unwrap();
        let (claim, json) = claim_file(&mut worker);
        let prepared = worker
            .library
            .prepare_file_job(&claim, &json, Some(&test_extractor()))
            .unwrap();
        let before = library.export_portable().unwrap().digest;
        let database = directory.path().join("queue.sqlite3");
        let changed_source = source.clone();
        let barrier = Arc::new(Barrier::new(2));
        let writer_barrier = Arc::clone(&barrier);
        let writer = std::thread::spawn(move || {
            let mut connection = Connection::open(database).unwrap();
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .unwrap();
            writer_barrier.wait();
            std::thread::sleep(Duration::from_millis(150));
            fs::write(changed_source, "Changed while waiting for writer.").unwrap();
            transaction.rollback().unwrap();
        });
        barrier.wait();
        let result = worker.library.publish_file_job(&claim, &prepared);
        writer.join().unwrap();
        assert!(matches!(result, Err(LoomError::SourceChanged(_))));
        assert_eq!(library.export_portable().unwrap().digest, before);
    }

    #[test]
    fn typed_payload_tampering_cannot_redirect_a_prepared_result() {
        let (_directory, library, source) = file_fixture();
        fs::write(&source, "Prepared authentic result.").unwrap();
        library
            .enqueue_index_file(&source, "typed", JobPriority::Normal)
            .unwrap();
        let mut worker = test_worker(&library).unwrap();
        let (claim, json) = claim_file(&mut worker);
        let prepared = worker
            .library
            .prepare_file_job(&claim, &json, Some(&test_extractor()))
            .unwrap();
        let before = library.export_portable().unwrap().digest;
        let mut changed: serde_json::Value = serde_json::from_str(&json).unwrap();
        changed["locator"] = serde_json::json!("/not-an-approved-source.md");
        library
            .lock()
            .unwrap()
            .execute(
                "UPDATE background_jobs SET target_json=?1 WHERE id=?2",
                params![changed.to_string(), claim.id],
            )
            .unwrap();
        assert!(matches!(
            worker.library.publish_file_job(&claim, &prepared),
            Err(LoomError::JobClaimStale(_))
        ));
        assert_eq!(library.export_portable().unwrap().digest, before);
        library
            .lock()
            .unwrap()
            .execute(
                "UPDATE background_jobs SET target_json='{}' WHERE id=?1",
                [&claim.id],
            )
            .unwrap();
        drop(worker);
        let failed = reacquire_after_deliberate_test_drop(&library)
            .unwrap()
            .run_next()
            .unwrap()
            .unwrap();
        assert_eq!(failed.state, JobState::Failed);
        assert!(failed.target_locator.is_none());
        assert_eq!(library.export_portable().unwrap().digest, before);
    }

    #[test]
    fn purge_removes_target_diagnostics_and_prepared_claims_cannot_resurrect_evidence() {
        let (_directory, library, source) = file_fixture();
        fs::write(&source, "Purge prepared result.").unwrap();
        let job = library
            .enqueue_index_file(&source, "purge-prepared", JobPriority::Normal)
            .unwrap();
        let mut worker = test_worker(&library).unwrap();
        let (claim, json) = claim_file(&mut worker);
        let prepared = worker
            .library
            .prepare_file_job(&claim, &json, Some(&test_extractor()))
            .unwrap();
        let artifact = file_identity(&library, &source).0;
        library.purge_artifact(&artifact).unwrap();
        assert!(library.background_job(&job.id).is_err());
        assert!(matches!(
            worker.library.publish_file_job(&claim, &prepared),
            Err(LoomError::JobClaimStale(_))
        ));
        assert_eq!(library.stats().unwrap().artifacts, 0);
        // A selected file root may have no artifact (e.g. extraction failed). Root purge still removes work.
        library
            .enqueue_index_file(&source, "root-only", JobPriority::Normal)
            .unwrap();
        library.purge_root(source.to_str().unwrap()).unwrap();
        assert!(library.background_jobs(128).unwrap().is_empty());
        assert!(library.source_roots().unwrap().is_empty());
    }

    #[test]
    fn worker_recovery_reextracts_file_under_new_claim_and_rejects_old_prepared_work() {
        let (_directory, library, source) = file_fixture();
        fs::write(&source, "Recovery exact source.").unwrap();
        let job = library
            .enqueue_index_file(&source, "recover-file", JobPriority::Normal)
            .unwrap();
        let mut old = test_worker(&library).unwrap();
        let (claim, json) = claim_file(&mut old);
        let prepared = old
            .library
            .prepare_file_job(&claim, &json, Some(&test_extractor()))
            .unwrap();
        drop(old);
        let mut new = reacquire_after_deliberate_test_drop(&library).unwrap();
        assert!(matches!(
            new.library.publish_file_job(&claim, &prepared),
            Err(LoomError::JobClaimStale(_))
        ));
        let completed = new.run_next().unwrap().unwrap();
        assert_eq!(completed.id, job.id);
        assert_eq!(completed.attempts, 2);
        assert_eq!(completed.state, JobState::Completed);
    }

    #[test]
    fn excessive_extracted_text_fails_without_hiding_or_deleting_original_evidence() {
        let (_directory, library, source) = file_fixture();
        let before = library.export_portable().unwrap().digest;
        fs::write(&source, "x".repeat(2 * 1024 * 1024 + 1)).unwrap();
        library
            .enqueue_index_file(&source, "output-limit", JobPriority::Normal)
            .unwrap();
        let failed = test_worker(&library).unwrap().run_next().unwrap().unwrap();
        assert_eq!(failed.state, JobState::Failed);
        assert!(failed.last_error.unwrap().contains("output limit"));
        assert_eq!(library.export_portable().unwrap().digest, before);
        fs::write(&source, "x".repeat(8 * 1024 * 1024 + 1)).unwrap();
        assert!(library
            .enqueue_index_file(&source, "input-limit", JobPriority::Normal)
            .is_err());
        assert_eq!(library.background_jobs(128).unwrap().len(), 1);
    }

    #[test]
    fn artifact_purge_does_not_remove_other_file_targets_in_a_shared_canonical_root() {
        let (directory, library, first) = file_fixture();
        let second = directory.path().join("second.md");
        fs::write(&second, "Second exact source.").unwrap();
        library.index_path(&second).unwrap();
        let second = second.canonicalize().unwrap();
        let first_job = library
            .enqueue_index_file(&first, "keep-other-target", JobPriority::Normal)
            .unwrap();
        let second_job = library
            .enqueue_index_file(&second, "purge-only-target", JobPriority::Normal)
            .unwrap();
        let first_artifact = file_identity(&library, &first).0;
        let second_artifact = file_identity(&library, &second).0;
        // Canonical roots can contain multiple artifacts (e.g. directory/import records). Build
        // the FK-valid shared-root shape to prove deletion is locator-scoped, not root-wide.
        library.lock().unwrap().execute(
            "UPDATE artifacts SET source_root_id=(SELECT source_root_id FROM artifacts WHERE id=?1) WHERE id=?2",
            params![first_artifact, second_artifact]).unwrap();
        library.purge_artifact(&second_artifact).unwrap();
        assert!(library.background_job(&second_job.id).is_err());
        assert_eq!(library.background_job(&first_job.id).unwrap(), first_job);
        assert_eq!(library.stats().unwrap().artifacts, 1);
    }

    #[test]
    fn artifact_and_ocr_purges_preserve_unrelated_malformed_target_diagnostics() {
        let (_directory, library, source) = file_fixture();
        let original = fs::read(&source).unwrap();
        let malformed = library
            .enqueue_index_file(&source, "malformed-target-diagnostics", JobPriority::Normal)
            .unwrap();
        library
            .lock()
            .unwrap()
            .execute(
                "UPDATE background_jobs SET target_json='{}' WHERE id=?1",
                [&malformed.id],
            )
            .unwrap();
        let malformed = library.background_job(&malformed.id).unwrap();
        assert!(malformed.target_locator.is_none());
        let matching = library
            .enqueue_index_file(&source, "purge-matching-target", JobPriority::Normal)
            .unwrap();
        library
            .purge_artifact(&file_identity(&library, &source).0)
            .unwrap();
        assert!(library.background_job(&matching.id).is_err());
        assert_eq!(library.background_job(&malformed.id).unwrap(), malformed);
        library.purge_ocr_records().unwrap();
        assert_eq!(library.background_job(&malformed.id).unwrap(), malformed);
        let failed = test_worker(&library).unwrap().run_next().unwrap().unwrap();
        assert_eq!(failed.id, malformed.id);
        assert_eq!(failed.state, JobState::Failed);
        assert!(failed.target_locator.is_none());
        assert_eq!(library.stats().unwrap().artifacts, 0);
        assert_eq!(fs::read(source).unwrap(), original);
    }

    #[test]
    fn missing_admission_artifact_identity_cannot_refresh_existing_evidence() {
        let (_directory, library, source) = file_fixture();
        let before = library.export_portable().unwrap().digest;
        let job = library
            .enqueue_index_file(&source, "missing-admission-identity", JobPriority::Normal)
            .unwrap();
        library
            .lock()
            .unwrap()
            .execute(
                "UPDATE background_jobs SET target_json=json_remove(target_json,'$.artifact_id') WHERE id=?1",
                [&job.id],
            )
            .unwrap();
        let failed = test_worker(&library).unwrap().run_next().unwrap().unwrap();
        assert_eq!(failed.id, job.id);
        assert_eq!(failed.state, JobState::Failed);
        assert_eq!(library.export_portable().unwrap().digest, before);
    }

    #[test]
    fn missing_admission_identity_after_deletion_cannot_recreate_evidence() {
        let (_directory, library, source) = file_fixture();
        let job = library
            .enqueue_index_file(&source, "deleted-missing-identity", JobPriority::Normal)
            .unwrap();
        let artifact = file_identity(&library, &source).0;
        {
            let connection = library.lock().unwrap();
            connection
                .execute(
                    "UPDATE background_jobs SET target_json=json_remove(target_json,'$.artifact_id') WHERE id=?1",
                    [&job.id],
                )
                .unwrap();
            // Simulate a canonical-only old binary's deletion without erasing its queue record.
            connection
                .execute("DELETE FROM artifacts WHERE id=?1", [&artifact])
                .unwrap();
        }
        let before = library.export_portable().unwrap().digest;
        let failed = test_worker(&library).unwrap().run_next().unwrap().unwrap();
        assert_eq!(failed.id, job.id);
        assert_eq!(failed.state, JobState::Failed);
        assert_eq!(library.export_portable().unwrap().digest, before);
        assert_eq!(library.stats().unwrap().artifacts, 0);
    }

    #[test]
    fn invalid_scheduling_policy_does_not_block_known_layout_deletion() {
        let (_directory, library, source) = file_fixture();
        library
            .enqueue_index_file(&source, "delete-despite-policy", JobPriority::Normal)
            .unwrap();
        let artifact = file_identity(&library, &source).0;
        library
            .lock()
            .unwrap()
            .execute(
                "UPDATE background_job_runtime SET policy_json='invalid-policy' WHERE slot=1",
                [],
            )
            .unwrap();
        library.purge_artifact(&artifact).unwrap();
        assert_eq!(library.stats().unwrap().artifacts, 0);
        let retained: i64 = library
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM background_jobs", [], |row| row.get(0))
            .unwrap();
        assert_eq!(retained, 0);
    }

    #[test]
    fn exhausted_runtime_counters_do_not_block_known_layout_deletion() {
        let (_directory, library, source) = file_fixture();
        library
            .enqueue_index_file(&source, "delete-exhausted-runtime", JobPriority::Normal)
            .unwrap();
        let artifact = file_identity(&library, &source).0;
        library
            .lock()
            .unwrap()
            .execute(
                "UPDATE background_job_runtime SET epoch=?1, next_sequence=?1, priority_streak=8 WHERE slot=1",
                [i64::MAX],
            )
            .unwrap();
        library.purge_artifact(&artifact).unwrap();
        assert_eq!(library.stats().unwrap().artifacts, 0);
        let retained: i64 = library
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM background_jobs", [], |row| row.get(0))
            .unwrap();
        assert_eq!(retained, 0);
    }

    fn assert_purge_runtime_recovery(error: LoomError, raw_fixture_marker: &str) {
        let reason = match error {
            LoomError::JobQueue(reason) => reason,
            other => panic!("expected a bounded queue diagnostic, got {other:?}"),
        };
        assert!(reason.contains("no data was deleted"), "{reason}");
        assert!(reason.contains("compatible LOOM release"), "{reason}");
        assert!(reason.contains("upgrade-job-runtime"), "{reason}");
        assert!(
            !reason.contains(raw_fixture_marker),
            "diagnostic must not reflect fixture details: {reason}"
        );
    }

    fn raw_job(library: &Library, id: &str) -> BackgroundJob {
        let connection = library.lock().unwrap();
        get_job(&connection, id).unwrap()
    }

    fn corrupt_runtime_row(library: &Library, fixture: &str) {
        let sql = match fixture {
            "missing-row" => "DELETE FROM background_job_runtime",
            "invalid-epoch" => "UPDATE background_job_runtime SET epoch=-1",
            "invalid-sequence" => "UPDATE background_job_runtime SET next_sequence=0",
            "invalid-streak" => "UPDATE background_job_runtime SET priority_streak=9",
            "extra-row" => "INSERT INTO background_job_runtime SELECT 2, epoch, next_sequence, priority_streak, policy_json FROM background_job_runtime WHERE slot=1",
            _ => unreachable!(),
        };
        let connection = library.lock().unwrap();
        // Fixture-only corruption keeps the exact STRICT schema definition intact.
        connection
            .execute_batch("PRAGMA ignore_check_constraints=ON")
            .unwrap();
        connection.execute(sql, []).unwrap();
        connection
            .execute_batch("PRAGMA ignore_check_constraints=OFF")
            .unwrap();
    }

    fn raw_runtime_rows(library: &Library) -> Vec<(i64, i64, i64, i64, String)> {
        let connection = library.lock().unwrap();
        let mut statement = connection
            .prepare("SELECT slot, epoch, next_sequence, priority_streak, policy_json FROM background_job_runtime ORDER BY slot")
            .unwrap();
        statement
            .query_map([], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    }

    #[test]
    fn unsupported_or_malformed_runtime_blocks_purge_without_partial_deletion() {
        for fixture in [
            "future",
            "v3",
            "malformed",
            "missing-row",
            "invalid-epoch",
            "invalid-sequence",
            "invalid-streak",
            "extra-row",
        ] {
            let (_directory, library, source) = file_fixture();
            let job = library
                .enqueue_index_file(&source, "retain-runtime-fixture", JobPriority::Normal)
                .unwrap();
            let artifact = file_identity(&library, &source).0;
            let before_digest = library.export_portable().unwrap().digest;
            let before_roots = library.source_roots().unwrap();
            let before_job = raw_job(&library, &job.id);
            let before_payload: String = library
                .lock()
                .unwrap()
                .query_row(
                    "SELECT target_json FROM background_jobs WHERE id=?1",
                    [&job.id],
                    |row| row.get(0),
                )
                .unwrap();

            let raw_marker = match fixture {
                "future" => {
                    library
                        .lock()
                        .unwrap()
                        .execute(
                            "UPDATE schema_meta SET value='future-runtime-marker' WHERE key='background_job_schema_version'",
                            [],
                        )
                        .unwrap();
                    "future-runtime-marker"
                }
                "v3" => {
                    library
                        .lock()
                        .unwrap()
                        .execute(
                            "UPDATE schema_meta SET value='3' WHERE key='background_job_schema_version'",
                            [],
                        )
                        .unwrap();
                    "3"
                }
                "malformed" => {
                    library
                        .lock()
                        .unwrap()
                        .execute(
                            "ALTER TABLE background_jobs ADD COLUMN unexpected_fixture TEXT",
                            [],
                        )
                        .unwrap();
                    "unexpected_fixture"
                }
                _ => {
                    corrupt_runtime_row(&library, fixture);
                    fixture
                }
            };
            let before_runtime = raw_runtime_rows(&library);

            assert_purge_runtime_recovery(
                library.purge_artifact(&artifact).unwrap_err(),
                raw_marker,
            );
            assert_eq!(library.export_portable().unwrap().digest, before_digest);
            assert_eq!(library.source_roots().unwrap(), before_roots);
            assert_eq!(raw_job(&library, &job.id), before_job);
            assert_eq!(raw_runtime_rows(&library), before_runtime);
            assert_eq!(
                library
                    .lock()
                    .unwrap()
                    .query_row(
                        "SELECT target_json FROM background_jobs WHERE id=?1",
                        [&job.id],
                        |row| row.get::<_, String>(0),
                    )
                    .unwrap(),
                before_payload
            );

            assert_purge_runtime_recovery(
                library.purge_root(source.to_str().unwrap()).unwrap_err(),
                raw_marker,
            );
            assert_eq!(library.export_portable().unwrap().digest, before_digest);
            assert_eq!(library.source_roots().unwrap(), before_roots);
            assert_eq!(raw_job(&library, &job.id), before_job);
            assert_eq!(raw_runtime_rows(&library), before_runtime);
        }
    }

    fn insert_synthetic_ocr_derivative(library: &Library, source: &Path) {
        let (artifact_id, _) = file_identity(library, source);
        let version_id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        let mut connection = library.lock().unwrap();
        let transaction = connection.transaction().unwrap();
        transaction
            .execute(
                "INSERT INTO artifact_versions(
                    id, artifact_id, content_hash, hash_algorithm, byte_size, source_modified_ns,
                    extractor_id, extractor_version, parse_warnings_json, page_count,
                    extraction_metadata_json, status, created_at
                 ) VALUES (?1, ?2, ?3, 'blake3', 12, NULL, ?4, ?5, '[]', NULL, '{}', 'ready', ?6)",
                params![
                    version_id,
                    artifact_id,
                    "blake3:synthetic-ocr-purge",
                    crate::ocr::IMAGE_OCR_EXTRACTOR_ID,
                    crate::ocr::IMAGE_OCR_EXTRACTOR_VERSION,
                    now,
                ],
            )
            .unwrap();
        transaction
            .execute(
                "INSERT INTO passages(
                    id, artifact_version_id, ordinal, text, text_hash, locator_json,
                    char_start, char_end, line_start, line_end, created_at
                 ) VALUES (?1, ?2, 0, 'synthetic OCR derivative', 'blake3:synthetic-ocr-passage', '{\"kind\":\"text\",\"char_start\":0,\"char_end\":23,\"line_start\":1,\"line_end\":1}', 0, 23, 1, 1, ?3)",
                params![Uuid::new_v4().to_string(), version_id, now],
            )
            .unwrap();
        transaction.commit().unwrap();
    }

    fn synthetic_ocr_rows(library: &Library) -> Vec<(String, String, String)> {
        let connection = library.lock().unwrap();
        let mut statement = connection
            .prepare(
                "SELECT id, content_hash, extractor_version FROM artifact_versions
                 WHERE extractor_id=?1 ORDER BY id",
            )
            .unwrap();
        statement
            .query_map([crate::ocr::IMAGE_OCR_EXTRACTOR_ID], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    }

    #[test]
    fn unsupported_runtime_blocks_ocr_policy_and_derived_purges_without_changes() {
        for fixture in [
            "future",
            "v3",
            "malformed",
            "missing-row",
            "invalid-epoch",
            "invalid-sequence",
            "invalid-streak",
            "extra-row",
        ] {
            for disable in [false, true] {
                let (_directory, library, source) = file_fixture();
                insert_synthetic_ocr_derivative(&library, &source);
                let job = library
                    .enqueue_index_file(&source, "retain-ocr-runtime", JobPriority::Normal)
                    .unwrap();
                let before_digest = library.export_portable().unwrap().digest;
                let before_roots = library.source_roots().unwrap();
                let before_ocr = library.ocr_status().unwrap();
                let before_ocr_rows = synthetic_ocr_rows(&library);
                let before_ocr_revision: String = library
                    .lock()
                    .unwrap()
                    .query_row(
                        "SELECT value FROM schema_meta WHERE key='ocr_policy_revision'",
                        [],
                        |row| row.get(0),
                    )
                    .unwrap();
                let before_job = raw_job(&library, &job.id);
                let before_payload: String = library
                    .lock()
                    .unwrap()
                    .query_row(
                        "SELECT target_json FROM background_jobs WHERE id=?1",
                        [&job.id],
                        |row| row.get(0),
                    )
                    .unwrap();

                let raw_marker = match fixture {
                    "future" => {
                        library
                            .lock()
                            .unwrap()
                            .execute(
                                "UPDATE schema_meta SET value='future-ocr-runtime-marker' WHERE key='background_job_schema_version'",
                                [],
                            )
                            .unwrap();
                        "future-ocr-runtime-marker"
                    }
                    "v3" => {
                        library
                            .lock()
                            .unwrap()
                            .execute(
                                "UPDATE schema_meta SET value='3' WHERE key='background_job_schema_version'",
                                [],
                            )
                            .unwrap();
                        "3"
                    }
                    "malformed" => {
                        library
                            .lock()
                            .unwrap()
                            .execute(
                                "ALTER TABLE background_jobs ADD COLUMN unexpected_ocr_fixture TEXT",
                                [],
                            )
                            .unwrap();
                        "unexpected_ocr_fixture"
                    }
                    _ => {
                        corrupt_runtime_row(&library, fixture);
                        fixture
                    }
                };
                let before_runtime = raw_runtime_rows(&library);

                let result = if disable {
                    library.set_ocr_enabled(false).map(|_| ())
                } else {
                    library.purge_ocr_records().map(|_| ())
                };
                assert_purge_runtime_recovery(result.unwrap_err(), raw_marker);
                assert_eq!(library.export_portable().unwrap().digest, before_digest);
                assert_eq!(library.source_roots().unwrap(), before_roots);
                assert_eq!(library.ocr_status().unwrap(), before_ocr);
                assert_eq!(synthetic_ocr_rows(&library), before_ocr_rows);
                assert_eq!(
                    library
                        .lock()
                        .unwrap()
                        .query_row(
                            "SELECT value FROM schema_meta WHERE key='ocr_policy_revision'",
                            [],
                            |row| row.get::<_, String>(0),
                        )
                        .unwrap(),
                    before_ocr_revision
                );
                assert_eq!(raw_job(&library, &job.id), before_job);
                assert_eq!(raw_runtime_rows(&library), before_runtime);
                assert_eq!(
                    library
                        .lock()
                        .unwrap()
                        .query_row(
                            "SELECT target_json FROM background_jobs WHERE id=?1",
                            [&job.id],
                            |row| row.get::<_, String>(0),
                        )
                        .unwrap(),
                    before_payload
                );
            }
        }
    }

    #[test]
    fn admission_artifact_identity_survives_old_binary_purge_without_resurrection() {
        let (_directory, library, source) = file_fixture();
        let original = fs::read(&source).unwrap();
        let artifact = file_identity(&library, &source).0;
        library
            .enqueue_index_file(&source, "pre-old-purge", JobPriority::Normal)
            .unwrap();
        // Older canonical-only tooling cannot remove a typed v3 operational target.
        library
            .lock()
            .unwrap()
            .execute("DELETE FROM artifacts WHERE id=?1", [&artifact])
            .unwrap();
        assert_eq!(library.background_jobs(128).unwrap().len(), 1);
        let mut worker = test_worker(&library).unwrap();
        let cancelled = worker.run_next().unwrap().unwrap();
        drop(worker);
        assert_eq!(cancelled.state, JobState::Cancelled);
        assert_eq!(library.stats().unwrap().artifacts, 0);
        assert_eq!(fs::read(&source).unwrap(), original);
        // Explicit new admission after deletion may create an artifact under the still-valid scope.
        library
            .enqueue_index_file(&source, "post-old-purge", JobPriority::Normal)
            .unwrap();
        let completed = reacquire_after_deliberate_test_drop(&library)
            .unwrap()
            .run_next()
            .unwrap()
            .unwrap();
        assert_eq!(completed.state, JobState::Completed);
        assert_eq!(
            library
                .enqueue_index_file(&source, "post-old-purge", JobPriority::Normal)
                .unwrap(),
            completed
        );
        assert_ne!(file_identity(&library, &source).0, artifact);
    }

    #[test]
    fn restore_invalidates_prepared_file_work_and_completed_return_survives_forgetting() {
        let (_directory, library, source) = file_fixture();
        let archive = library.export_portable().unwrap();
        fs::write(&source, "Pre-restore prepared source.").unwrap();
        library
            .enqueue_index_file(&source, "restore-file", JobPriority::Normal)
            .unwrap();
        let mut worker = test_worker(&library).unwrap();
        let (claim, json) = claim_file(&mut worker);
        let prepared = worker
            .library
            .prepare_file_job(&claim, &json, Some(&test_extractor()))
            .unwrap();
        // Restore accepts only an empty canonical library. Preserve operational state here so
        // this fixture isolates restore fencing rather than the separately tested purge fence.
        library
            .lock()
            .unwrap()
            .execute_batch("DELETE FROM artifacts; DELETE FROM source_roots;")
            .unwrap();
        library.import_portable(&archive).unwrap();
        assert!(matches!(
            worker.library.publish_file_job(&claim, &prepared),
            Err(LoomError::JobClaimStale(_))
        ));
        assert!(library.background_jobs(128).unwrap().is_empty());
        drop(worker);
        let queued = library
            .enqueue_index_file(&source, "post-restore-file", JobPriority::Normal)
            .unwrap();
        let completed = reacquire_after_deliberate_test_drop(&library)
            .unwrap()
            .run_next_inner(|completed| {
                library.forget_background_job(&completed.id).unwrap();
            })
            .unwrap()
            .unwrap();
        assert_eq!(completed.id, queued.id);
        assert_eq!(completed.state, JobState::Completed);
        assert!(completed.result.is_some());
        assert!(library.background_job(&completed.id).is_err());
    }

    #[test]
    fn file_and_maintenance_jobs_share_one_admission_budget() {
        let (_directory, library, source) = file_fixture();
        library
            .set_job_queue_policy(JobQueuePolicy {
                max_pending: 1,
                max_records: 2,
                ..Default::default()
            })
            .unwrap();
        let first = library
            .enqueue_index_file(&source, "budget-file", JobPriority::Normal)
            .unwrap();
        assert!(library
            .enqueue_fts_repair("budget-fts", JobPriority::Normal)
            .is_err());
        library.cancel_background_job(&first.id).unwrap();
        library
            .enqueue_fts_repair("budget-fts", JobPriority::Normal)
            .unwrap();
        assert!(library
            .enqueue_index_file(&source, "another-file", JobPriority::Normal)
            .is_err());
        assert_eq!(library.background_jobs(128).unwrap().len(), 2);
    }

    #[test]
    fn image_admission_requires_enabled_policy_and_policy_rotation_cancels_before_extraction() {
        let (directory, library) = fixture();
        let image = directory.path().join("intentionally-invalid.png");
        fs::write(&image, "not image bytes").unwrap();
        library.set_ocr_enabled(false).unwrap();
        library.index_path(&image).unwrap(); // explicit selection with OCR off
        assert!(matches!(
            library.enqueue_index_file(&image, "ocr-off", JobPriority::Normal),
            Err(LoomError::OcrDisabled)
        ));
        library.set_ocr_enabled(true).unwrap();
        library
            .enqueue_index_file(&image, "old-ocr-policy", JobPriority::Normal)
            .unwrap();
        library.set_ocr_enabled(true).unwrap(); // same boolean, new explicit consent revision
        let cancelled = test_worker(&library).unwrap().run_next().unwrap().unwrap();
        assert_eq!(cancelled.state, JobState::Cancelled);
        assert!(cancelled.last_error.unwrap().contains("policy changed"));
        library
            .enqueue_index_file(&image, "purged-ocr-policy", JobPriority::Normal)
            .unwrap();
        library.purge_ocr_records().unwrap();
        assert!(library.background_jobs(128).unwrap().is_empty());
    }

    #[test]
    fn idempotent_admission_is_atomic_across_connections_and_rejects_input_conflicts() {
        let (directory, library) = fixture();
        let database = directory.path().join("queue.sqlite3");
        let other = Library::open(&database).unwrap();
        let barrier = Arc::new(Barrier::new(2));
        let second_barrier = Arc::clone(&barrier);
        let task = std::thread::spawn(move || {
            second_barrier.wait();
            other
                .enqueue_fts_repair("duplicate-key", JobPriority::Normal)
                .unwrap()
        });
        barrier.wait();
        let first = library
            .enqueue_fts_repair("duplicate-key", JobPriority::Normal)
            .unwrap();
        assert_eq!(first.id, task.join().unwrap().id);
        assert_eq!(library.background_jobs(128).unwrap().len(), 1);
        assert!(library
            .enqueue_fts_repair("duplicate-key", JobPriority::High)
            .is_err());
        for key in ["", "a b", "a\n", "../outside", "非ASCII"] {
            assert!(library
                .enqueue_fts_repair(key, JobPriority::Normal)
                .is_err());
        }
        assert_eq!(library.background_jobs(128).unwrap(), vec![first]);
    }

    #[test]
    fn pending_and_terminal_record_bounds_preserve_existing_idempotency_keys() {
        let (_directory, library) = fixture();
        let policy = JobQueuePolicy {
            max_pending: 2,
            max_records: 3,
            ..JobQueuePolicy::default()
        };
        library.set_job_queue_policy(policy).unwrap();
        let first = library
            .enqueue_fts_repair("one", JobPriority::Normal)
            .unwrap();
        let second = library
            .enqueue_fts_repair("two", JobPriority::Normal)
            .unwrap();
        assert!(library
            .enqueue_fts_repair("three", JobPriority::Normal)
            .is_err());
        assert!(library.set_job_queue_policy(policy).is_err());
        library.cancel_background_job(&first.id).unwrap();
        let third = library
            .enqueue_fts_repair("three", JobPriority::Normal)
            .unwrap();
        library.cancel_background_job(&second.id).unwrap();
        library.cancel_background_job(&third.id).unwrap();
        assert!(library
            .enqueue_fts_repair("four", JobPriority::Normal)
            .is_err());
        assert_eq!(
            library
                .enqueue_fts_repair("one", JobPriority::Normal)
                .unwrap()
                .id,
            first.id
        );
        assert!(library
            .set_job_queue_policy(JobQueuePolicy {
                max_records: 2,
                ..policy
            })
            .is_err());
        library.set_job_queue_policy(policy).unwrap();
        assert_eq!(
            library.forget_background_job(&second.id).unwrap().state,
            JobState::Cancelled
        );
        let fourth = library
            .enqueue_fts_repair("four", JobPriority::Normal)
            .unwrap();
        assert!(library.forget_background_job(&fourth.id).is_err());
    }

    #[test]
    fn real_fts_adapter_publishes_repair_and_completion_atomically_without_rebuilding_on_acquire() {
        let (directory, library) = fixture();
        let source = directory.path().join("selected.md");
        fs::write(&source, "bounded queue evidence").unwrap();
        library.index_path(&source).unwrap();
        let canonical = library.export_portable().unwrap();
        library
            .lock()
            .unwrap()
            .execute("DELETE FROM passages_fts", [])
            .unwrap();
        assert!(!library.fts_health().unwrap().healthy);
        let job = library
            .enqueue_fts_repair("repair", JobPriority::Normal)
            .unwrap();
        assert_eq!(job.state, JobState::Queued);
        let mut worker = test_worker(&library).unwrap();
        assert!(
            !library.fts_health().unwrap().healthy,
            "worker acquisition rebuilt the derivative"
        );
        let finished = worker.run_next().unwrap().unwrap();
        assert_eq!(finished.id, job.id);
        assert_eq!(finished.state, JobState::Completed);
        assert_eq!(finished.attempts, 1);
        assert_eq!(finished.result.as_ref().unwrap()["after"]["healthy"], true);
        assert!(library.fts_health().unwrap().healthy);
        assert_eq!(library.export_portable().unwrap().digest, canonical.digest);
        assert_eq!(
            fs::read_to_string(source).unwrap(),
            "bounded queue evidence"
        );
        assert!(worker.run_next().unwrap().is_none());
        assert_eq!(
            library
                .enqueue_fts_repair("repair", JobPriority::Normal)
                .unwrap(),
            finished
        );
        assert_eq!(library.cancel_background_job(&job.id).unwrap(), finished);
    }

    #[test]
    fn completed_and_failed_results_survive_terminal_forgetting_before_return() {
        let (_directory, library) = fixture();
        let job = library
            .enqueue_fts_repair("forget-after-publish", JobPriority::Normal)
            .unwrap();
        let mut worker = test_worker(&library).unwrap();
        let finished = worker
            .run_next_inner(|snapshot| {
                assert_eq!(snapshot.state, JobState::Completed);
                library.forget_background_job(&snapshot.id).unwrap();
            })
            .unwrap()
            .unwrap();
        assert_eq!(finished.id, job.id);
        assert!(finished.result.unwrap()["after"]["healthy"]
            .as_bool()
            .unwrap());
        assert!(library.background_job(&job.id).is_err());

        let failed = library
            .enqueue_fts_repair("forget-after-failure", JobPriority::Normal)
            .unwrap();
        library
            .lock()
            .unwrap()
            .execute("DROP TABLE passages_fts", [])
            .unwrap();
        let snapshot = worker
            .run_next_inner(|snapshot| {
                assert_eq!(snapshot.state, JobState::Failed);
                library.forget_background_job(&snapshot.id).unwrap();
            })
            .unwrap()
            .unwrap();
        assert_eq!(snapshot.id, failed.id);
        assert!(snapshot.last_error.is_some());
        assert!(library.background_job(&failed.id).is_err());
    }

    fn downgrade_runtime_for_fixture(library: &Library) {
        downgrade_runtime_layout(library, false);
    }

    fn downgrade_runtime_layout(library: &Library, versioned: bool) {
        let mut connection = library.lock().unwrap();
        let transaction = connection.transaction().unwrap();
        transaction.execute_batch("ALTER TABLE background_jobs RENAME TO current_jobs; DROP INDEX background_jobs_ready;").unwrap();
        let schema = if versioned {
            RUNTIME_SCHEMA.to_owned()
        } else {
            legacy_runtime_schema()
        };
        transaction.execute_batch(&schema).unwrap();
        transaction
            .execute(
                "INSERT INTO background_jobs SELECT id,idempotency_key,operation,priority,state,
                attempts,max_attempts,sequence,ready_at_ms,epoch,claim_token,cancel_requested,
                last_error,result_json FROM current_jobs",
                [],
            )
            .unwrap();
        transaction.execute("DROP TABLE current_jobs", []).unwrap();
        transaction
            .execute(
                "DELETE FROM schema_meta WHERE key = 'background_job_schema_version'",
                [],
            )
            .unwrap();
        transaction.commit().unwrap();
        drop(connection);
        if versioned {
            library.lock().unwrap().execute("INSERT INTO schema_meta(key,value) VALUES ('background_job_schema_version','2')", []).unwrap();
        }
    }

    #[test]
    fn versioned_v2_upgrade_is_explicit_preserves_records_and_can_be_repeated() {
        let (directory, library) = fixture();
        let job = library
            .enqueue_fts_repair("v2-preserved", JobPriority::Normal)
            .unwrap();
        let canonical = library.export_portable().unwrap().digest;
        downgrade_runtime_layout(&library, true);
        let reader = Library::open(directory.path().join("queue.sqlite3")).unwrap();
        assert_eq!(
            reader
                .lock()
                .unwrap()
                .query_row(
                    "SELECT value FROM schema_meta WHERE key='background_job_schema_version'",
                    [],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
            "2"
        );
        assert!(reader.background_jobs(128).is_err());
        reader.upgrade_job_runtime().unwrap();
        assert_eq!(reader.background_job(&job.id).unwrap(), job);
        reader.upgrade_job_runtime().unwrap();
        assert_eq!(reader.background_job(&job.id).unwrap(), job);
        assert_eq!(reader.export_portable().unwrap().digest, canonical);
    }

    #[test]
    fn runtime_upgrade_preserves_policy_epoch_order_and_all_job_states_then_restore_fences_them() {
        let (directory, library) = fixture();
        library
            .set_job_queue_policy(JobQueuePolicy {
                max_pending: 4,
                max_records: 8,
                ..Default::default()
            })
            .unwrap();
        let mut worker = test_worker(&library).unwrap();
        library
            .enqueue_fts_repair("legacy-completed", JobPriority::High)
            .unwrap();
        worker.run_next().unwrap().unwrap();
        library
            .enqueue_fts_repair("legacy-queued", JobPriority::Normal)
            .unwrap();
        library
            .enqueue_fts_repair("legacy-running", JobPriority::High)
            .unwrap();
        let claim = worker.claim_at(now()).unwrap().unwrap();
        let rows = library.background_jobs(128).unwrap();
        let policy = library.job_queue_policy().unwrap();
        let runtime: (i64, i64, i64) = library
            .lock()
            .unwrap()
            .query_row(
                "SELECT epoch,next_sequence,priority_streak FROM background_job_runtime",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        let archive = library.export_portable().unwrap();
        downgrade_runtime_for_fixture(&library);
        assert!(Library::open_for_jobs(directory.path().join("queue.sqlite3")).is_err());
        let migrated = Library::open(directory.path().join("queue.sqlite3")).unwrap();
        assert!(validate_schema(&migrated.lock().unwrap()).is_err());
        assert!(matches!(
            migrated.upgrade_job_runtime(),
            Err(LoomError::JobWorkerBusy)
        ));
        claim.verify(&worker.library.lock().unwrap()).unwrap();
        drop(worker);
        after_deliberate_test_drop(|| migrated.upgrade_job_runtime()).unwrap();
        validate_schema(&migrated.lock().unwrap()).unwrap();
        let upgraded_rows = migrated.background_jobs(128).unwrap();
        for row in rows {
            let upgraded = upgraded_rows
                .iter()
                .find(|upgraded| upgraded.id == row.id)
                .unwrap();
            if row.state == JobState::Running {
                assert_eq!(upgraded.state, JobState::Retryable);
                assert_eq!(upgraded.attempts, row.attempts);
            } else {
                assert_eq!(upgraded, &row);
            }
        }
        assert_eq!(migrated.job_queue_policy().unwrap(), policy);
        assert_eq!(
            migrated
                .lock()
                .unwrap()
                .query_row(
                    "SELECT epoch,next_sequence,priority_streak FROM background_job_runtime",
                    [],
                    |row| Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?
                    )),
                )
                .unwrap(),
            (runtime.0 + 1, runtime.1, runtime.2)
        );
        assert_eq!(migrated.export_portable().unwrap().digest, archive.digest);
        assert!(!archive
            .settings
            .contains_key("background_job_schema_version"));
        assert!(claim.verify(&migrated.lock().unwrap()).is_err());
        let mut worker = test_worker(&migrated).unwrap();
        let claim = worker.claim_at(now()).unwrap().unwrap();
        migrated.import_portable(&archive).unwrap();
        assert!(migrated.background_jobs(128).unwrap().is_empty());
        assert!(claim.verify(&worker.library.lock().unwrap()).is_err());
        validate_schema(&migrated.lock().unwrap()).unwrap();
    }

    #[test]
    fn invalid_legacy_diagnostics_roll_back_runtime_upgrade_without_dropping_jobs() {
        let (directory, library) = fixture();
        let job = library
            .enqueue_fts_repair("retain-invalid-legacy", JobPriority::Normal)
            .unwrap();
        downgrade_runtime_for_fixture(&library);
        library
            .lock()
            .unwrap()
            .execute(
                "UPDATE background_jobs SET result_json = 'invalid-json' WHERE id = ?1",
                [&job.id],
            )
            .unwrap();
        let opened = Library::open(directory.path().join("queue.sqlite3")).unwrap();
        assert!(opened.stats().is_ok());
        assert!(opened.upgrade_job_runtime().is_err());
        assert!(Library::open_for_jobs(directory.path().join("queue.sqlite3")).is_err());
        let connection = opened.lock().unwrap();
        assert_eq!(
            connection
                .query_row(
                    "SELECT result_json FROM background_jobs WHERE id = ?1",
                    [&job.id],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
            "invalid-json"
        );
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM background_jobs", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert!(!connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name = 'background_jobs_v2')",
                [],
                |row| row.get::<_, bool>(0)
            )
            .unwrap());
        drop(connection);
        assert!(opened
            .enqueue_fts_repair("must-not-admit", JobPriority::Normal)
            .is_err());
        assert!(opened.background_job(&job.id).is_err());
        assert!(opened.background_jobs(128).is_err());
        assert!(opened.cancel_background_job(&job.id).is_err());
        assert!(opened.forget_background_job(&job.id).is_err());
        assert!(opened.job_queue_policy().is_err());
        assert!(opened
            .set_job_queue_policy(JobQueuePolicy::default())
            .is_err());
        assert!(opened.stats().is_ok());
    }

    #[test]
    fn queue_open_refuses_uninitialized_or_malformed_runtime_without_migrating() {
        let (directory, library) = fixture();
        let missing = directory.path().join("missing.sqlite3");
        assert!(Library::open_for_jobs(&missing).is_err());
        assert!(JobWorker::open(&missing).is_err());
        assert!(!missing.exists());
        let database = library.job_database_path().unwrap();
        library
            .lock()
            .unwrap()
            .execute("ALTER TABLE background_jobs ADD COLUMN unexpected TEXT", [])
            .unwrap();
        assert!(matches!(
            Library::open_for_jobs(&database),
            Err(LoomError::JobQueue(_))
        ));
        assert!(matches!(
            JobWorker::open(&database),
            Err(LoomError::JobQueue(_))
        ));
        assert!(
            library.stats().is_ok(),
            "operational corruption must not hide canonical evidence"
        );
    }

    #[test]
    fn persisted_result_rejects_malformed_json_and_unicode_byte_overflow() {
        let (_directory, library) = fixture();
        let job = library
            .enqueue_fts_repair("invalid-result", JobPriority::Normal)
            .unwrap();
        let connection = library.lock().unwrap();
        for json in [
            "not-json".to_owned(),
            serde_json::to_string(&"🔥".repeat(20_000)).unwrap(),
        ] {
            assert!(connection
                .execute(
                    "UPDATE background_jobs SET result_json = ?1 WHERE id = ?2",
                    params![json, job.id]
                )
                .is_err());
        }
        assert!(connection
            .execute(
                "UPDATE background_jobs SET last_error = ?1 WHERE id = ?2",
                params!["🔥".repeat(4096), job.id]
            )
            .is_err());
    }

    #[test]
    fn retry_deadline_attempt_exhaustion_and_every_state_are_observable() {
        let (_directory, library) = fixture();
        let job = library
            .enqueue_fts_repair("retry", JobPriority::Normal)
            .unwrap();
        let mut worker = test_worker(&library).unwrap();
        let mut clock = now();
        for attempt in 1..=3 {
            let claim = worker.claim_at(clock).unwrap().unwrap();
            let running = library.background_job(&job.id).unwrap();
            assert_eq!(running.state, JobState::Running);
            assert_eq!(running.attempts, attempt);
            assert!(worker.claim_at(clock).is_err());
            worker
                .settle_failure(&claim, true, "retry fixture", clock)
                .unwrap();
            let state = library.background_job(&job.id).unwrap().state;
            assert_eq!(
                state,
                if attempt == 3 {
                    JobState::Failed
                } else {
                    JobState::Retryable
                }
            );
            assert!(claim.verify(&library.lock().unwrap()).is_err());
            assert!(worker.claim_at(clock + 999).unwrap().is_none());
            clock += 1000;
        }
        assert!(worker.claim_at(clock).unwrap().is_none());
        assert_eq!(library.background_job(&job.id).unwrap().attempts, 3);
    }

    #[test]
    fn cancellation_before_claim_and_before_publish_cannot_modify_the_derivative() {
        let (_directory, library) = fixture();
        let queued = library
            .enqueue_fts_repair("cancel-queued", JobPriority::Normal)
            .unwrap();
        assert_eq!(
            library.cancel_background_job(&queued.id).unwrap().state,
            JobState::Cancelled
        );
        let running = library
            .enqueue_fts_repair("cancel-running", JobPriority::Normal)
            .unwrap();
        let mut worker = test_worker(&library).unwrap();
        let claim = worker.claim_at(now()).unwrap().unwrap();
        assert_eq!(claim.id, running.id);
        assert!(library.forget_background_job(&running.id).is_err());
        let cancelled = library.cancel_background_job(&running.id).unwrap();
        assert_eq!(cancelled.state, JobState::Running);
        assert!(cancelled.cancel_requested);
        let before = library.fts_health().unwrap();
        assert!(matches!(
            worker.library.job_repair_fts(&claim),
            Err(LoomError::JobClaimStale(_))
        ));
        assert_eq!(library.fts_health().unwrap(), before);
        worker
            .settle_failure(&claim, true, "cancel fixture", now())
            .unwrap();
        assert_eq!(
            library.background_job(&running.id).unwrap().state,
            JobState::Cancelled
        );
        assert!(worker.run_next().unwrap().is_none());
    }

    #[test]
    fn priority_bursts_cannot_starve_an_older_low_priority_job() {
        let (_directory, library) = fixture();
        let low = library
            .enqueue_fts_repair("older-low", JobPriority::Low)
            .unwrap();
        for index in 0..4 {
            let mut worker = if index == 0 {
                test_worker(&library)
            } else {
                reacquire_after_deliberate_test_drop(&library)
            }
            .expect("released test owner remained locked after bounded exec window");
            library
                .enqueue_fts_repair(&format!("high-{index}"), JobPriority::High)
                .unwrap();
            let claim = worker.claim_at(now()).unwrap().unwrap();
            assert_ne!(claim.id, low.id);
            complete(&worker, &claim);
            drop(worker);
        }
        library
            .enqueue_fts_repair("more-high", JobPriority::High)
            .unwrap();
        let mut worker = reacquire_after_deliberate_test_drop(&library)
            .expect("released test owner remained locked after bounded exec window");
        let claim = worker.claim_at(now()).unwrap().unwrap();
        assert_eq!(claim.id, low.id);
        complete(&worker, &claim);
        assert_eq!(
            worker.run_next().unwrap().unwrap().priority,
            JobPriority::High
        );
    }

    #[test]
    fn durable_epoch_and_claim_token_reject_old_completion_after_recovery() {
        let (_directory, library) = fixture();
        let job = library
            .enqueue_fts_repair("crashed", JobPriority::Normal)
            .unwrap();
        let mut old = test_worker(&library).unwrap();
        let claim = old.claim_at(now()).unwrap().unwrap();
        assert!(matches!(
            test_worker(&library),
            Err(LoomError::JobWorkerBusy)
        ));
        drop(old);
        let mut new = reacquire_after_deliberate_test_drop(&library).unwrap();
        assert_eq!(
            library.background_job(&job.id).unwrap().state,
            JobState::Retryable
        );
        let replacement = new.claim_at(now()).unwrap().unwrap();
        assert_ne!(replacement.epoch, claim.epoch);
        assert_ne!(replacement.token, claim.token);
        assert!(matches!(
            new.library.job_repair_fts(&claim),
            Err(LoomError::JobClaimStale(_))
        ));
        assert_eq!(
            library.background_job(&job.id).unwrap().state,
            JobState::Running
        );
        new.library.job_repair_fts(&replacement).unwrap();
        assert_eq!(library.background_job(&job.id).unwrap().attempts, 2);
    }

    #[test]
    fn runtime_is_not_exported_restore_invalidates_workers_and_invalid_restore_rolls_back() {
        let (_directory, library) = fixture();
        let mut archive = library.export_portable().unwrap();
        library
            .enqueue_fts_repair("not-portable", JobPriority::Normal)
            .unwrap();
        assert_eq!(library.export_portable().unwrap().digest, archive.digest);
        assert!(!archive.tables.contains_key("background_jobs"));
        let mut worker = test_worker(&library).unwrap();
        let claim = worker.claim_at(now()).unwrap().unwrap();
        archive
            .tables
            .get_mut("passages")
            .unwrap()
            .columns
            .push("invalid-column".into());
        archive.seal().unwrap();
        assert!(library.import_portable(&archive).is_err());
        claim.verify(&library.lock().unwrap()).unwrap();
        archive.tables.get_mut("passages").unwrap().columns.pop();
        archive.seal().unwrap();
        library.import_portable(&archive).unwrap();
        assert!(library.background_jobs(128).unwrap().is_empty());
        assert!(matches!(
            worker.library.job_repair_fts(&claim),
            Err(LoomError::JobClaimStale(_))
        ));
        assert!(matches!(
            worker.run_next(),
            Err(LoomError::JobClaimStale(_))
        ));
    }

    #[test]
    fn worker_does_not_share_the_interactive_connection_mutex() {
        let (_directory, library) = fixture();
        library
            .enqueue_fts_repair("independent-connection", JobPriority::Normal)
            .unwrap();
        let mut worker = test_worker(&library).unwrap();
        let interactive_guard = library.lock().unwrap();
        let (send, receive) = std::sync::mpsc::channel();
        let task = std::thread::spawn(move || {
            send.send(worker.run_next()).unwrap();
        });
        assert_eq!(
            receive
                .recv_timeout(Duration::from_secs(3))
                .unwrap()
                .unwrap()
                .unwrap()
                .state,
            JobState::Completed
        );
        drop(interactive_guard);
        task.join().unwrap();
    }

    #[test]
    fn bounded_diagnostics_and_invalid_policy_fail_without_unbounded_persistence() {
        let (directory, library) = fixture();
        let source = directory.path().join("source.md");
        fs::write(
            &source,
            "operational policy does not hide canonical evidence",
        )
        .unwrap();
        library.index_path(&source).unwrap();
        let job = library
            .enqueue_fts_repair("diagnostic", JobPriority::Normal)
            .unwrap();
        let mut worker = test_worker(&library).unwrap();
        let claim = worker.claim_at(now()).unwrap().unwrap();
        worker
            .settle_failure(&claim, false, &"🔥".repeat(4096), now())
            .unwrap();
        assert_eq!(
            library
                .background_job(&job.id)
                .unwrap()
                .last_error
                .unwrap()
                .len(),
            4096
        );
        library
            .lock()
            .unwrap()
            .execute("UPDATE background_job_runtime SET policy_json = '{}'", [])
            .unwrap();
        assert!(library
            .enqueue_fts_repair("new", JobPriority::Normal)
            .is_err());
        assert!(worker.run_next().is_err());
        assert_eq!(library.background_jobs(128).unwrap().len(), 1);
        let reader = Library::open(directory.path().join("queue.sqlite3")).unwrap();
        assert_eq!(
            reader
                .search(&crate::SearchRequest {
                    text: "operational policy".into(),
                    limit: 5
                })
                .unwrap()
                .len(),
            1
        );
        assert!(reader
            .enqueue_fts_repair("still-invalid", JobPriority::Normal)
            .is_err());
    }

    #[cfg(unix)]
    #[test]
    fn worker_lock_refuses_symlinks_without_modifying_the_target() {
        let (directory, library) = fixture();
        let outside = directory.path().join("untouched.txt");
        fs::write(&outside, "must stay unchanged").unwrap();
        std::os::unix::fs::symlink(&outside, directory.path().join("queue.sqlite3.worker.lock"))
            .unwrap();
        assert!(test_worker(&library).is_err());
        assert_eq!(fs::read_to_string(outside).unwrap(), "must stay unchanged");
    }

    #[cfg(unix)]
    #[test]
    fn canonical_symlink_database_aliases_share_ownership_and_hard_links_are_refused() {
        let (directory, library) = fixture();
        let alias = directory.path().join("alias.sqlite3");
        std::os::unix::fs::symlink(directory.path().join("queue.sqlite3"), &alias).unwrap();
        let alias_library = Library::open(&alias).unwrap();
        let different = Library::open(directory.path().join("different.sqlite3")).unwrap();
        fs::remove_file(&alias).unwrap();
        std::os::unix::fs::symlink(different.job_database_path().unwrap(), &alias).unwrap();
        let worker = test_worker(&library).unwrap();
        assert!(matches!(
            test_worker(&alias_library),
            Err(LoomError::JobWorkerBusy)
        ));
        drop(worker);
        fs::hard_link(
            directory.path().join("queue.sqlite3"),
            directory.path().join("hard.sqlite3"),
        )
        .unwrap();
        assert!(
            matches!(test_worker(&library), Err(LoomError::JobQueue(reason)) if reason.contains("hard-linked"))
        );
        assert!(
            matches!(Library::open_for_jobs(directory.path().join("hard.sqlite3")), Err(LoomError::JobQueue(reason)) if reason.contains("hard-linked"))
        );
    }

    #[cfg(unix)]
    #[test]
    fn worker_refuses_database_file_replacement_since_original_open() {
        let (directory, library) = fixture();
        let path = library.job_database_path().unwrap();
        assert!(path.is_absolute());
        fs::rename(&path, directory.path().join("original.sqlite3")).unwrap();
        fs::write(&path, "replacement must not be opened").unwrap();
        assert!(
            matches!(test_worker(&library), Err(LoomError::JobQueue(reason)) if reason.contains("identity changed"))
        );
        assert_eq!(
            fs::read_to_string(path).unwrap(),
            "replacement must not be opened"
        );
    }

    #[test]
    fn worker_rejects_a_changed_schema_and_retry_classification_is_not_generic() {
        let (_directory, library) = fixture();
        library
            .lock()
            .unwrap()
            .execute(
                "UPDATE schema_meta SET value = '99' WHERE key = 'schema_version'",
                [],
            )
            .unwrap();
        assert!(matches!(test_worker(&library), Err(LoomError::JobQueue(_))));
        for code in [rusqlite::ffi::SQLITE_BUSY, rusqlite::ffi::SQLITE_LOCKED] {
            assert!(is_retryable(&LoomError::Database(
                rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(code), None)
            )));
        }
        for code in [
            rusqlite::ffi::SQLITE_CORRUPT,
            rusqlite::ffi::SQLITE_CONSTRAINT,
        ] {
            assert!(!is_retryable(&LoomError::Database(
                rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(code), None)
            )));
        }
        assert!(!is_retryable(&LoomError::JobQueue("invalid input".into())));
    }

    struct ChildGuard(Child);
    impl Drop for ChildGuard {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    #[test]
    fn process_lock_child() {
        let Some(database) = std::env::var_os("LOOM_TEST_JOB_LOCK_DATABASE") else {
            return;
        };
        let library = Library::open_for_jobs(database).unwrap();
        let mut worker = test_worker(&library).unwrap();
        let claim = worker.claim_at(now()).unwrap().unwrap();
        let target: Option<String> = worker
            .library
            .lock()
            .unwrap()
            .query_row(
                "SELECT target_json FROM background_jobs WHERE id=?1",
                [&claim.id],
                |row| row.get(0),
            )
            .unwrap();
        let _prepared = target.as_deref().map(|json| {
            worker
                .library
                .prepare_file_job(&claim, json, Some(&test_extractor()))
                .unwrap()
        });
        fs::write(
            std::env::var_os("LOOM_TEST_JOB_LOCK_READY").unwrap(),
            "ready",
        )
        .unwrap();
        loop {
            std::thread::park();
        }
    }

    #[test]
    fn actual_process_death_releases_the_lock_and_recovers_running_work() {
        let (directory, library, source) = file_fixture();
        for operation in ["fts_repair", "index_file"] {
            fs::write(
                &source,
                format!("Killed-process {operation} recovery marker."),
            )
            .unwrap();
            let job = if operation == "fts_repair" {
                library
                    .enqueue_fts_repair("process-crash", JobPriority::Normal)
                    .unwrap()
            } else {
                library
                    .enqueue_index_file(&source, "process-file-crash", JobPriority::Normal)
                    .unwrap()
            };
            let canonical = library.export_portable().unwrap().digest;
            let ready = directory.path().join(format!("child-ready-{operation}"));
            let child = Command::new(std::env::current_exe().unwrap())
                .args(["jobs::tests::process_lock_child", "--exact", "--nocapture"])
                .env(
                    "LOOM_TEST_JOB_LOCK_DATABASE",
                    directory.path().join("queue.sqlite3"),
                )
                .env("LOOM_TEST_JOB_LOCK_READY", &ready)
                .stdout(Stdio::null())
                .spawn()
                .unwrap();
            let mut child = ChildGuard(child);
            let deadline = Instant::now() + Duration::from_secs(10);
            while !ready.exists() && Instant::now() < deadline {
                assert!(
                    child.0.try_wait().unwrap().is_none(),
                    "child exited before locking"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
            assert!(ready.exists(), "child did not acquire the worker lock");
            assert!(matches!(
                test_worker(&library),
                Err(LoomError::JobWorkerBusy)
            ));
            assert_eq!(
                library.background_job(&job.id).unwrap().state,
                JobState::Running
            );
            assert_eq!(library.export_portable().unwrap().digest, canonical);
            child.0.kill().unwrap();
            child.0.wait().unwrap();
            let mut recovered = test_worker(&library).unwrap();
            let completed = recovered.run_next().unwrap().unwrap();
            assert_eq!(completed.state, JobState::Completed);
            assert_eq!(completed.id, job.id);
            assert_eq!(completed.attempts, 2);
        }
    }
}
