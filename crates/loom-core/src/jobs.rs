//! Operational, non-portable background work. This first adapter runs FTS repair only.
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
    if existing && validate_definitions(&transaction, RUNTIME_SCHEMA).is_err() {
        let legacy = legacy_runtime_schema();
        if validate_definitions(&transaction, &legacy).is_err() {
            return Ok(());
        }
        transaction.execute_batch(
            "ALTER TABLE background_jobs RENAME TO background_jobs_v1;
             DROP INDEX background_jobs_ready;",
        )?;
        transaction.execute_batch(RUNTIME_SCHEMA)?;
        // Invalid legacy diagnostics/results leave the old runtime intact and unavailable;
        // never truncate, silently drop jobs, or hide canonical evidence.
        if transaction
            .execute(
                "INSERT INTO background_jobs SELECT * FROM background_jobs_v1",
                [],
            )
            .is_err()
        {
            return Ok(());
        }
        transaction.execute("DROP TABLE background_jobs_v1", [])?;
    } else {
        transaction.execute_batch(RUNTIME_SCHEMA)?;
    }
    transaction.execute(
        "INSERT INTO background_job_runtime VALUES (1, 0, 1, 0, ?1) ON CONFLICT(slot) DO NOTHING",
        [serde_json::to_string(&JobQueuePolicy::default())?],
    )?;
    transaction.execute(
        "INSERT INTO schema_meta(key,value) VALUES ('background_job_schema_version','2')",
        [],
    )?;
    transaction.commit()?;
    Ok(())
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
    if version.as_deref() != Some("2") {
        return Err(LoomError::JobQueue(
            "unsupported or unmigrated runtime schema; explicitly initialize with Library::open"
                .into(),
        ));
    }
    validate_definitions(connection, RUNTIME_SCHEMA)?;
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
            cancel_requested, ready_at_ms, last_error, result_json FROM background_jobs WHERE id = ?1",
        [id], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?,
            row.get::<_, String>(2)?, row.get::<_, String>(3)?, row.get::<_, i64>(4)?,
            row.get::<_, u32>(5)?, row.get::<_, u32>(6)?, row.get::<_, bool>(7)?,
            row.get::<_, i64>(8)?, row.get::<_, Option<String>>(9)?, row.get::<_, Option<String>>(10)?)),
    ).optional()?.ok_or_else(|| LoomError::JobQueue(format!("job not found: {id}")))?;
    Ok(BackgroundJob {
        id: raw.0,
        idempotency_key: raw.1,
        operation: raw.2,
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
        let mut connection = self.queue_connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing: Option<String> = transaction
            .query_row(
                "SELECT id FROM background_jobs WHERE idempotency_key = ?1",
                [key],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(id) = existing {
            let job = get_job(&transaction, &id)?;
            if job.operation != "fts_repair" || job.priority != priority {
                return Err(LoomError::JobQueue(
                    "idempotency key has conflicting input".into(),
                ));
            }
            return Ok(job);
        }
        let policy = load_policy(&transaction)?;
        let (records, pending): (u32, u32) = transaction.query_row(
            "SELECT COUNT(*), COALESCE(SUM(state IN ('queued','running','retryable')),0) FROM background_jobs",
            [], |row| Ok((row.get(0)?, row.get(1)?)))?;
        if records >= policy.max_records || pending >= policy.max_pending {
            return Err(LoomError::JobQueue(
                "admission capacity reached; existing jobs were not evicted".into(),
            ));
        }
        let sequence = next_sequence(&transaction)?;
        let id = Uuid::new_v4().to_string();
        transaction.execute(
            "INSERT INTO background_jobs(id,idempotency_key,operation,priority,state,attempts,
                max_attempts,sequence,ready_at_ms) VALUES (?1,?2,'fts_repair',?3,'queued',0,?4,?5,?6)",
            params![id, key, priority.number(), policy.max_attempts, sequence, Utc::now().timestamp_millis()],
        )?;
        let job = get_job(&transaction, &id)?;
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
}

impl JobWorker {
    /// Acquires ownership before opening SQLite; never creates or migrates the library.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_with_limits(path, crate::LibraryLimits::default())
    }

    pub(crate) fn open_with_limits(
        path: impl AsRef<Path>,
        limits: crate::LibraryLimits,
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
        let library = Library::open_for_jobs_with_limits(&database, limits)?;
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
        })
    }
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
}

pub(crate) struct JobClaim {
    id: String,
    epoch: i64,
    token: String,
}

impl JobClaim {
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

    fn settle_failure(
        &mut self,
        claim: &JobClaim,
        retryable: bool,
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
            "UPDATE background_jobs SET state = CASE WHEN cancel_requested = 1 THEN 'cancelled'
                WHEN ?1 = 1 AND attempts < max_attempts THEN 'retryable' ELSE 'failed' END,
                epoch = NULL, claim_token = NULL, last_error = ?2, ready_at_ms = ?3, sequence = ?4
             WHERE id = ?5",
            params![retryable, reason, due, sequence, claim.id],
        )?;
        let settled = get_job(&transaction, &claim.id)?;
        transaction.commit()?;
        Ok(settled)
    }

    /// Runs at most one real FTS rebuild. Derivative publication and completion share a fenced txn.
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
        let settled = match self.library.job_repair_fts(&claim) {
            Ok(completed) => completed,
            Err(error) => self.settle_failure(
                &claim,
                is_retryable(&error),
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

    fn fixture() -> (TempDir, Library) {
        let directory = tempdir().unwrap();
        let library = Library::open(directory.path().join("queue.sqlite3")).unwrap();
        (directory, library)
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
        let mut worker = library.acquire_job_worker().unwrap();
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
        let mut worker = library.acquire_job_worker().unwrap();
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
        let mut connection = library.lock().unwrap();
        let transaction = connection.transaction().unwrap();
        transaction.execute_batch("ALTER TABLE background_jobs RENAME TO current_jobs; DROP INDEX background_jobs_ready;").unwrap();
        transaction.execute_batch(&legacy_runtime_schema()).unwrap();
        transaction
            .execute("INSERT INTO background_jobs SELECT * FROM current_jobs", [])
            .unwrap();
        transaction.execute("DROP TABLE current_jobs", []).unwrap();
        transaction
            .execute(
                "DELETE FROM schema_meta WHERE key = 'background_job_schema_version'",
                [],
            )
            .unwrap();
        transaction.commit().unwrap();
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
        let mut worker = library.acquire_job_worker().unwrap();
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
        validate_schema(&migrated.lock().unwrap()).unwrap();
        assert_eq!(migrated.background_jobs(128).unwrap(), rows);
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
            runtime
        );
        assert_eq!(migrated.export_portable().unwrap().digest, archive.digest);
        assert!(!archive
            .settings
            .contains_key("background_job_schema_version"));
        claim.verify(&worker.library.lock().unwrap()).unwrap();
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
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name = 'background_jobs_v1')",
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
        let mut worker = library.acquire_job_worker().unwrap();
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
        let mut worker = library.acquire_job_worker().unwrap();
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
            let mut worker = library.acquire_job_worker().unwrap();
            library
                .enqueue_fts_repair(&format!("high-{index}"), JobPriority::High)
                .unwrap();
            let claim = worker.claim_at(now()).unwrap().unwrap();
            assert_ne!(claim.id, low.id);
            complete(&worker, &claim);
        }
        library
            .enqueue_fts_repair("more-high", JobPriority::High)
            .unwrap();
        let mut worker = library.acquire_job_worker().unwrap();
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
        let mut old = library.acquire_job_worker().unwrap();
        let claim = old.claim_at(now()).unwrap().unwrap();
        assert!(matches!(
            library.acquire_job_worker(),
            Err(LoomError::JobWorkerBusy)
        ));
        drop(old);
        let mut new = library.acquire_job_worker().unwrap();
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
        let mut worker = library.acquire_job_worker().unwrap();
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
        let mut worker = library.acquire_job_worker().unwrap();
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
        let mut worker = library.acquire_job_worker().unwrap();
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
        assert!(library.acquire_job_worker().is_err());
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
        let worker = library.acquire_job_worker().unwrap();
        assert!(matches!(
            alias_library.acquire_job_worker(),
            Err(LoomError::JobWorkerBusy)
        ));
        drop(worker);
        fs::hard_link(
            directory.path().join("queue.sqlite3"),
            directory.path().join("hard.sqlite3"),
        )
        .unwrap();
        assert!(
            matches!(library.acquire_job_worker(), Err(LoomError::JobQueue(reason)) if reason.contains("hard-linked"))
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
            matches!(library.acquire_job_worker(), Err(LoomError::JobQueue(reason)) if reason.contains("identity changed"))
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
        assert!(matches!(
            library.acquire_job_worker(),
            Err(LoomError::JobQueue(_))
        ));
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
        let library = Library::open(database).unwrap();
        let mut worker = library.acquire_job_worker().unwrap();
        worker.claim_at(now()).unwrap().unwrap();
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
        let (directory, library) = fixture();
        let job = library
            .enqueue_fts_repair("process-crash", JobPriority::Normal)
            .unwrap();
        let ready = directory.path().join("child-ready");
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
            library.acquire_job_worker(),
            Err(LoomError::JobWorkerBusy)
        ));
        assert_eq!(
            library.background_job(&job.id).unwrap().state,
            JobState::Running
        );
        child.0.kill().unwrap();
        child.0.wait().unwrap();
        let mut recovered = library.acquire_job_worker().unwrap();
        let completed = recovered.run_next().unwrap().unwrap();
        assert_eq!(completed.state, JobState::Completed);
        assert_eq!(completed.id, job.id);
        assert_eq!(completed.attempts, 2);
    }
}
