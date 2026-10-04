//! Durable directory capabilities, never interchangeable with exact-file capabilities.
use super::*;
use crate::jobs::{BackgroundJob, DirectoryProgress, JobClaim, JobState};

const MAX_DIRECTORY_UNITS: usize = 20_000;
const MAX_RETAINED_DIRECTORY_UNITS: u64 = 65_536;
const MAX_RETAINED_DIRECTORY_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceIdentity {
    device: u64,
    inode: u64,
    created: (u64, u32),
}

impl SourceIdentity {
    fn capture(path: &Path, directory: bool) -> Result<Self> {
        let metadata = fs::symlink_metadata(path).map_err(|error| namespace_error(path, error))?;
        Self::from_metadata(&metadata, path, directory)
    }

    fn from_metadata(metadata: &fs::Metadata, path: &Path, directory: bool) -> Result<Self> {
        if metadata.file_type().is_symlink()
            || (directory && !metadata.is_dir())
            || (!directory && !metadata.is_file())
        {
            return Err(LoomError::SourceChanged(path.display().to_string()));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            Ok(Self {
                device: metadata.dev(),
                inode: metadata.ino(),
                created: birth_time(metadata)?,
            })
        }
        #[cfg(not(unix))]
        Err(LoomError::UnsupportedSource(
            "durable directory jobs require Unix source identity".into(),
        ))
    }

    fn verify(&self, path: &Path, directory: bool) -> Result<()> {
        let metadata = fs::symlink_metadata(path).map_err(|error| namespace_error(path, error))?;
        self.verify_metadata(&metadata, path, directory)
    }

    fn verify_metadata(&self, metadata: &fs::Metadata, path: &Path, directory: bool) -> Result<()> {
        if Self::from_metadata(metadata, path, directory)? != *self {
            return Err(LoomError::SourceRevoked(path.display().to_string()));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct IndexDirectoryTarget {
    locator: String,
    authorization: SourceAuthorization,
    root_identity: SourceIdentity,
    root_change: (i64, i64),
    namespace_fingerprint: String,
    max_files: u32,
    total_units: u32,
}

impl IndexDirectoryTarget {
    fn parse(json: &str) -> Result<Self> {
        if json.len() > 16_384 {
            return Err(invalid("directory target exceeds 16 KiB"));
        }
        let target: Self =
            serde_json::from_str(json).map_err(|_| invalid("invalid typed directory target"))?;
        let auth = &target.authorization;
        let fingerprint = target.namespace_fingerprint.strip_prefix("blake3:");
        if !valid_locator(&target.locator)
            || auth.kind != "directory"
            || auth.generation < 0
            || Uuid::parse_str(&auth.root_id).is_err()
            || Uuid::parse_str(&auth.incarnation).is_err()
            || auth
                .ocr_policy
                .as_ref()
                .is_some_and(|policy| Uuid::parse_str(&policy.revision).is_err())
            || target.max_files == 0
            || target.max_files as usize > MAX_DIRECTORY_UNITS
            || target.total_units > target.max_files
            || !fingerprint.is_some_and(|value| {
                value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
            })
        {
            return Err(invalid("invalid directory capability shape"));
        }
        Ok(target)
    }

    fn verify(&self, connection: &Connection) -> Result<()> {
        self.authorization
            .verify_persisted_locator(connection, &self.locator)?;
        self.verify_root()?;
        if Path::new(&self.locator)
            .canonicalize()
            .map_err(|error| namespace_error(Path::new(&self.locator), error))?
            != Path::new(&self.locator)
        {
            return Err(LoomError::SourceRevoked(self.locator.clone()));
        }
        Ok(())
    }

    fn verify_child(&self, unit: &DirectoryUnit) -> Result<()> {
        unit.validate(self)?;
        self.verify_root()?;
        let path = Path::new(&unit.locator);
        unit.identity.verify(path, false)?;
        if path
            .canonicalize()
            .map_err(|error| namespace_error(path, error))?
            != path
        {
            return Err(LoomError::SourceRevoked(unit.locator.clone()));
        }
        Ok(())
    }

    fn verify_root(&self) -> Result<()> {
        let path = Path::new(&self.locator);
        let metadata = fs::symlink_metadata(path).map_err(|error| namespace_error(path, error))?;
        self.verify_root_metadata(&metadata)
    }

    fn verify_root_metadata(&self, metadata: &fs::Metadata) -> Result<()> {
        self.root_identity
            .verify_metadata(metadata, Path::new(&self.locator), true)?;
        if directory_change(metadata) != self.root_change {
            return Err(LoomError::SourceChanged(self.locator.clone()));
        }
        Ok(())
    }
}

fn birth_time(metadata: &fs::Metadata) -> Result<(u64, u32)> {
    let created = metadata.created().map_err(|_| {
        LoomError::UnsupportedSource(
            "durable directory jobs require filesystem birth-time identity".into(),
        )
    })?;
    let elapsed = created.duration_since(std::time::UNIX_EPOCH).map_err(|_| {
        LoomError::UnsupportedSource("source birth time precedes Unix epoch".into())
    })?;
    Ok((elapsed.as_secs(), elapsed.subsec_nanos()))
}

fn directory_change(_metadata: &fs::Metadata) -> (i64, i64) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        (_metadata.ctime(), _metadata.ctime_nsec())
    }
    #[cfg(not(unix))]
    (0, 0)
}

fn namespace_error(path: &Path, error: std::io::Error) -> LoomError {
    if matches!(
        error.kind(),
        std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
    ) || error.raw_os_error() == Some(libc::ELOOP)
    {
        LoomError::SourceChanged(path.display().to_string())
    } else {
        io_error(path, error)
    }
}

fn read_directory_bytes(
    target: &IndexDirectoryTarget,
    unit: &DirectoryUnit,
    max_bytes: u64,
) -> Result<ingest::StableBytes> {
    unit.validate(target)?;
    ingest::read_stable_bytes_fenced(
        Path::new(&unit.locator),
        Path::new(&target.locator),
        max_bytes,
        |root, file| {
            target.verify_root_metadata(root)?;
            unit.identity
                .verify_metadata(file, Path::new(&unit.locator), false)
        },
    )
    .map_err(|error| match error {
        LoomError::Io { path, source } => namespace_error(&path, source),
        other => other,
    })
}

fn invalid(message: &str) -> LoomError {
    LoomError::JobQueue(message.into())
}

fn valid_locator(locator: &str) -> bool {
    locator.len() <= 4096
        && !locator.chars().any(char::is_control)
        && Path::new(locator).is_absolute()
        && !Path::new(locator).components().any(|part| {
            matches!(
                part,
                std::path::Component::ParentDir | std::path::Component::CurDir
            )
        })
}

#[derive(PartialEq, Eq)]
struct DirectoryUnit {
    ordinal: u32,
    relative_path: String,
    locator: String,
    media_type: Option<String>,
    identity: SourceIdentity,
    artifact_id: Option<String>,
}

impl DirectoryUnit {
    fn validate(&self, target: &IndexDirectoryTarget) -> Result<()> {
        let relative = Path::new(&self.relative_path);
        if self.ordinal >= target.total_units
            || self.relative_path.is_empty()
            || self.relative_path.len() > 4096
            || self.relative_path.chars().any(char::is_control)
            || !relative
                .components()
                .all(|part| matches!(part, std::path::Component::Normal(_)))
            || !valid_locator(&self.locator)
            || Path::new(&target.locator).join(relative) != Path::new(&self.locator)
            || ingest::supported_media_type(Path::new(&self.locator)) != self.media_type.as_deref()
            || self
                .artifact_id
                .as_ref()
                .is_some_and(|id| Uuid::parse_str(id).is_err())
            || (self
                .media_type
                .as_deref()
                .is_some_and(|media| media.starts_with("image/"))
                && target.authorization.ocr_policy.is_none())
        {
            return Err(invalid("invalid directory manifest unit"));
        }
        Ok(())
    }

    fn verify_snapshot(
        &self,
        target: &IndexDirectoryTarget,
        snapshot: &Option<CanonicalFileSnapshot>,
    ) -> Result<()> {
        if snapshot.as_ref().map(|record| record.artifact_id.as_str())
            != self.artifact_id.as_deref()
            || snapshot.as_ref().is_some_and(|record| {
                record.root_id != target.authorization.root_id
                    || !record.locator_active
                    || record.state == "tombstoned"
            })
        {
            return Err(LoomError::SourceRevoked(
                target.authorization.root_id.clone(),
            ));
        }
        Ok(())
    }
}

pub(crate) struct DirectoryAdmission {
    target: IndexDirectoryTarget,
    paths: Vec<PathBuf>,
    identities: Vec<(u64, u64)>,
}

impl DirectoryAdmission {
    pub(crate) fn target_json(&self, connection: &Connection) -> Result<String> {
        self.target.verify(connection)?;
        let mut target = self.target.clone();
        if self.paths.iter().any(|path| {
            ingest::supported_media_type(path).is_some_and(|media| media.starts_with("image/"))
        }) {
            target.authorization.ocr_policy = Some(OcrPolicy::load(connection)?);
        }
        let json = serde_json::to_string(&target)?;
        IndexDirectoryTarget::parse(&json)?;
        Ok(json)
    }

    pub(crate) fn insert_manifest(
        &self,
        connection: &Connection,
        job: &BackgroundJob,
        existing: bool,
    ) -> Result<()> {
        if existing {
            if matches!(
                job.state,
                JobState::Queued | JobState::Running | JobState::Retryable
            ) && job.directory_progress.is_none()
            {
                return Err(invalid("pending directory job has no manifest"));
            }
            return Ok(());
        }
        let (retained_units, retained_bytes): (u64, u64) = connection.query_row(
            "SELECT COALESCE(SUM(total_units),0),COALESCE(SUM(encoded_bytes),0)
             FROM background_directory_manifests",
            [],
            |row| {
                Ok((
                    crate::jobs::row_bytes(row, 0)?,
                    crate::jobs::row_bytes(row, 1)?,
                ))
            },
        )?;
        enforce_manifest_capacity(
            retained_units,
            retained_bytes,
            u64::from(self.target.total_units),
            0,
        )?;
        let mut encoded_bytes = (job.id.len()
            + self.target.locator.len()
            + self.target.authorization.root_id.len()
            + self.target_json(connection)?.len()) as u64;
        enforce_manifest_capacity(
            retained_units,
            retained_bytes,
            u64::from(self.target.total_units),
            encoded_bytes,
        )?;
        connection.execute(
            "INSERT INTO background_directory_manifests(job_id,root_id,root_locator,encoded_bytes,total_units) VALUES (?1,?2,?3,?4,?5)",
            params![job.id, self.target.authorization.root_id, self.target.locator, sql_i64(encoded_bytes,"directory manifest bytes")?, self.target.total_units],
        )?;
        for (ordinal, (path, &(device, inode))) in
            self.paths.iter().zip(&self.identities).enumerate()
        {
            let locator = utf8_path(path)?;
            let relative = path
                .strip_prefix(&self.target.locator)
                .map_err(|_| invalid("child outside directory capability"))?;
            let relative_path = utf8_path(relative)?;
            if !valid_locator(&locator) || relative_path.chars().any(char::is_control) {
                return Err(invalid(
                    "queued directory requires exact bounded UTF-8 paths",
                ));
            }
            let snapshot = canonical_file_snapshot(connection, &locator)?;
            if snapshot.as_ref().is_some_and(|record| {
                record.root_id != self.target.authorization.root_id
                    || !record.locator_active
                    || record.state == "tombstoned"
            }) {
                return Err(LoomError::SourceRevoked(
                    self.target.authorization.root_id.clone(),
                ));
            }
            let identity = SourceIdentity::capture(path, false)?;
            if (identity.device, identity.inode) != (device, inode) {
                return Err(LoomError::SourceChanged(locator));
            }
            let identity_json = serde_json::to_string(&identity)?;
            let media = ingest::supported_media_type(path);
            encoded_bytes += (job.id.len()
                + 8
                + relative_path.len()
                + locator.len()
                + media.map_or(0, str::len)
                + identity_json.len()
                + snapshot
                    .as_ref()
                    .map_or(0, |record| record.artifact_id.len()))
                as u64;
            enforce_manifest_capacity(
                retained_units,
                retained_bytes,
                u64::from(self.target.total_units),
                encoded_bytes,
            )?;
            connection.execute("INSERT INTO background_directory_units(job_id,ordinal,relative_path,locator,media_type,identity_json,artifact_id)
                VALUES (?1,?2,?3,?4,?5,?6,?7)", params![job.id, ordinal as u32, relative_path, locator,
                media, identity_json, snapshot.map(|record| record.artifact_id)])?;
        }
        connection.execute(
            "UPDATE background_directory_manifests SET encoded_bytes=?1 WHERE job_id=?2",
            params![sql_i64(encoded_bytes, "directory manifest bytes")?, job.id],
        )?;
        self.target.verify(connection)?;
        Ok(())
    }
}

fn enforce_manifest_capacity(
    retained_units: u64,
    retained_bytes: u64,
    units: u64,
    bytes: u64,
) -> Result<()> {
    if retained_units.saturating_add(units) > MAX_RETAINED_DIRECTORY_UNITS
        || retained_bytes.saturating_add(bytes) > MAX_RETAINED_DIRECTORY_BYTES
    {
        return Err(invalid(
            "directory manifest capacity exceeded; settle or cancel pending directory jobs",
        ));
    }
    Ok(())
}

pub(super) fn validate_targets_for_purge(connection: &Connection) -> Result<()> {
    let mut statement = connection
        .prepare("SELECT id,target_json,state FROM background_jobs WHERE operation='index_directory' LIMIT 4097")?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    for (ordinal, row) in rows.enumerate() {
        if ordinal >= 4096 {
            return Err(invalid("directory diagnostic budget exceeded"));
        }
        let (id, json, state) = row?;
        let target = IndexDirectoryTarget::parse(&json)?;
        // Terminal diagnostics legitimately have no manifest; pending jobs must have one.
        let roots: Option<(String,String,u32)> = connection.query_row(
            "SELECT root_id,root_locator,total_units FROM background_directory_manifests WHERE job_id=?1", [&id],
            |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional()?;
        if let Some((root_id, locator, units)) = roots {
            if root_id != target.authorization.root_id
                || locator != target.locator
                || units != target.total_units
            {
                return Err(invalid("directory manifest capability mismatch"));
            }
        } else if matches!(state.as_str(), "queued" | "running" | "retryable") {
            return Err(invalid("pending directory job has no manifest"));
        }
    }
    Ok(())
}

pub(crate) struct PreparedDirectoryFile {
    target: IndexDirectoryTarget,
    target_json: String,
    unit: DirectoryUnit,
    snapshot: Option<CanonicalFileSnapshot>,
    document: Option<PreparedIndexDocument>,
    metrics: Option<loom_extraction::ExtractionMetrics>,
}

impl Library {
    pub(crate) fn prepare_directory_admission(
        &self,
        requested: &Path,
    ) -> Result<DirectoryAdmission> {
        let root = canonical_selected_root(requested)?;
        let root_identity = SourceIdentity::capture(&root, true)?;
        let root_change = directory_change(
            &fs::symlink_metadata(&root).map_err(|error| namespace_error(&root, error))?,
        );
        let locator = utf8_path(&root)?;
        if !valid_locator(&locator) {
            return Err(invalid("queued directory requires a bounded UTF-8 root"));
        }
        let authorization = {
            let connection = self.lock()?;
            connection.query_row("SELECT id,scope_generation,(SELECT value FROM schema_meta WHERE key='authorization_incarnation'),kind
                FROM source_roots WHERE locator=?1 AND kind='directory' AND enabled=1", [&locator], |row| Ok(SourceAuthorization {
                root_id: row.get(0)?, generation: row.get(1)?, incarnation: row.get(2)?, kind: row.get(3)?, ocr_policy: None,
            })).optional()?.ok_or_else(|| LoomError::SourceRevoked(locator.clone()))?
        };
        let max_files = self.limits.max_files_per_request.min(MAX_DIRECTORY_UNITS);
        if max_files == 0 {
            return Err(invalid("directory file limit must be positive"));
        }
        let namespace = crate::discovery::walk_snapshot(
            &root,
            crate::discovery::DiscoveryLimits::for_files(max_files),
            |_| Ok(()),
        )?;
        if namespace.file_identities.len() != namespace.files.len() {
            return Err(invalid("directory discovery has no exact file identities"));
        }
        // Reject non-UTF-8/control paths before any operational writes; never lossy-stringify them.
        for path in &namespace.files {
            if !valid_locator(&utf8_path(path)?) {
                return Err(invalid(
                    "queued directory requires exact bounded UTF-8 paths",
                ));
            }
        }
        root_identity.verify(&root, true)?;
        Ok(DirectoryAdmission {
            target: IndexDirectoryTarget {
                locator,
                authorization,
                root_identity,
                root_change,
                namespace_fingerprint: namespace.fingerprint,
                max_files: max_files as u32,
                total_units: namespace.files.len() as u32,
            },
            paths: namespace.files,
            identities: namespace.file_identities,
        })
    }

    pub(crate) fn job_index_directory(
        &self,
        claim: &JobClaim,
        json: &str,
        supervisor: Option<&loom_extraction::ExtractionSupervisor>,
    ) -> Result<BackgroundJob> {
        match self.prepare_directory_file(claim, json, supervisor)? {
            Some(prepared) => self.publish_directory_file(claim, &prepared),
            None => self.finish_directory_job(claim, json),
        }
    }

    pub(crate) fn prepare_directory_file(
        &self,
        claim: &JobClaim,
        json: &str,
        supervisor: Option<&loom_extraction::ExtractionSupervisor>,
    ) -> Result<Option<PreparedDirectoryFile>> {
        self.prepare_directory_file_before_read(claim, json, supervisor, || {})
    }

    fn prepare_directory_file_before_read(
        &self,
        claim: &JobClaim,
        json: &str,
        supervisor: Option<&loom_extraction::ExtractionSupervisor>,
        before_read: impl FnOnce(),
    ) -> Result<Option<PreparedDirectoryFile>> {
        let target = IndexDirectoryTarget::parse(json)?;
        let (unit, snapshot) = {
            let mut connection = self.lock()?;
            let transaction = connection.transaction()?;
            claim.verify_operation(&transaction, "index_directory", Some(json))?;
            target.verify(&transaction)?;
            let progress = directory_progress(&transaction, claim.id(), &target)?;
            if progress.next_unit == progress.total_units {
                return Ok(None);
            }
            let unit = load_unit(&transaction, claim.id(), progress.next_unit)?;
            target.verify_child(&unit)?;
            let snapshot = canonical_file_snapshot(&transaction, &unit.locator)?;
            unit.verify_snapshot(&target, &snapshot)?;
            transaction.commit()?;
            (unit, snapshot)
        };
        let skip = unit.media_type.is_none()
            || (unit
                .media_type
                .as_deref()
                .is_some_and(|media| media.starts_with("image/"))
                && !target
                    .authorization
                    .ocr_policy
                    .as_ref()
                    .is_some_and(|policy| policy.enabled));
        let (document, metrics) = if skip {
            (None, None)
        } else {
            before_read();
            let path = Path::new(&unit.locator);
            let media = ingest::supported_media_type(path)
                .ok_or_else(|| invalid("directory media type changed"))?;
            let (document, metrics) = self.prepare_queued_document(
                path,
                Path::new(&target.locator),
                media,
                supervisor,
                Some(read_directory_bytes(
                    &target,
                    &unit,
                    self.limits.max_file_bytes.min(8 * 1024 * 1024),
                )?),
                || self.probe_directory_file(claim, json, &target, &unit, &snapshot),
            )?;
            (Some(document), Some(metrics))
        };
        Ok(Some(PreparedDirectoryFile {
            target,
            target_json: json.to_owned(),
            unit,
            snapshot,
            document,
            metrics,
        }))
    }

    fn probe_directory_file(
        &self,
        claim: &JobClaim,
        json: &str,
        target: &IndexDirectoryTarget,
        unit: &DirectoryUnit,
        snapshot: &Option<CanonicalFileSnapshot>,
    ) -> Result<()> {
        // Never wait five seconds on a busy DB while the supervised child consumes resources.
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
            claim.verify_operation(&transaction, "index_directory", Some(json))?;
            target
                .authorization
                .verify_persisted_locator(&transaction, &target.locator)?;
            if directory_progress(&transaction, claim.id(), target)?.next_unit != unit.ordinal {
                return Err(invalid("directory cursor changed"));
            }
            if load_unit(&transaction, claim.id(), unit.ordinal)? != *unit {
                return Err(invalid("directory manifest unit changed"));
            }
            let current = canonical_file_snapshot(&transaction, &unit.locator)?;
            unit.verify_snapshot(target, &current)?;
            if &current != snapshot {
                return Err(LoomError::SourceChanged(unit.locator.clone()));
            }
            transaction.commit()?;
            Ok(())
        })();
        let restored = connection
            .busy_timeout(std::time::Duration::from_secs(5))
            .map_err(LoomError::from);
        result.and(restored)
    }

    pub(crate) fn publish_directory_file(
        &self,
        claim: &JobClaim,
        prepared: &PreparedDirectoryFile,
    ) -> Result<BackgroundJob> {
        let mut connection = self.lock()?;
        let transaction = source_write_transaction(&mut connection)?;
        claim.verify_operation(&transaction, "index_directory", Some(&prepared.target_json))?;
        let target = &prepared.target;
        let unit = &prepared.unit;
        target.verify(&transaction)?;
        target.verify_child(unit)?;
        if directory_progress(&transaction, claim.id(), target)?.next_unit != unit.ordinal {
            return Err(invalid("directory cursor changed"));
        }
        if load_unit(&transaction, claim.id(), unit.ordinal)? != *unit {
            return Err(invalid("directory manifest unit changed"));
        }
        let current = canonical_file_snapshot(&transaction, &unit.locator)?;
        unit.verify_snapshot(target, &current)?;
        if current != prepared.snapshot {
            return Err(LoomError::SourceChanged(unit.locator.clone()));
        }
        let (indexed, unchanged, skipped, bytes) = if let Some(document) = &prepared.document {
            let stable = read_directory_bytes(
                target,
                unit,
                self.limits.max_file_bytes.min(8 * 1024 * 1024),
            )?;
            if format!("blake3:{}", blake3::hash(&stable.bytes).to_hex())
                != document.document.raw_hash
            {
                return Err(LoomError::SourceChanged(unit.locator.clone()));
            }
            target.verify(&transaction)?;
            target.verify_child(unit)?;
            let indexed =
                Self::commit_index_document(&transaction, &target.authorization, document, None)?;
            (
                u32::from(indexed),
                u32::from(!indexed),
                0,
                document.document.byte_size,
            )
        } else {
            (0, 0, 1, 0)
        };
        let changed = transaction.execute(
            "UPDATE background_directory_manifests SET next_unit=next_unit+1,
            indexed=indexed+?1, unchanged=unchanged+?2, skipped=skipped+?3, bytes_read=bytes_read+?4, last_extraction_json=?7
            WHERE job_id=?5 AND next_unit=?6",
            params![
                indexed,
                unchanged,
                skipped,
                sql_i64(bytes, "directory byte progress")?,
                claim.id(),
                unit.ordinal,
                prepared.metrics.as_ref().map(serde_json::to_string).transpose()?
            ],
        )?;
        if changed != 1 {
            return Err(invalid("directory cursor changed"));
        }
        let yielded = claim.yield_progress(&transaction)?;
        transaction.commit()?;
        Ok(yielded)
    }

    fn finish_directory_job(&self, claim: &JobClaim, json: &str) -> Result<BackgroundJob> {
        let target = IndexDirectoryTarget::parse(json)?;
        let namespace = crate::discovery::walk_snapshot(
            Path::new(&target.locator),
            crate::discovery::DiscoveryLimits::for_files(target.max_files as usize),
            |_| {
                let connection = self.lock()?;
                claim.verify_operation(&connection, "index_directory", Some(json))?;
                target
                    .authorization
                    .verify_persisted_locator(&connection, &target.locator)
            },
        )?;
        if namespace.fingerprint != target.namespace_fingerprint
            || namespace.files.len() != target.total_units as usize
        {
            return Err(LoomError::SourceChanged(target.locator.clone()));
        }
        self.finish_directory_snapshot(claim, json, &target, namespace)
    }

    fn finish_directory_snapshot(
        &self,
        claim: &JobClaim,
        json: &str,
        target: &IndexDirectoryTarget,
        namespace: crate::discovery::NamespaceSnapshot,
    ) -> Result<BackgroundJob> {
        let seen: HashSet<String> = namespace
            .files
            .iter()
            .map(|path| utf8_path(path))
            .collect::<Result<_>>()?;
        let mut connection = self.lock()?;
        let transaction = source_write_transaction(&mut connection)?;
        claim.verify_operation(&transaction, "index_directory", Some(json))?;
        target.verify(&transaction)?;
        let progress = directory_progress(&transaction, claim.id(), target)?;
        if progress.next_unit != progress.total_units {
            return Err(invalid("directory is not fully settled"));
        }
        // Refuse oversized historical roots rather than turn the final claim into an unbounded
        // canonical sweep. A later adapter may slice this reconciliation too.
        let candidates: Vec<(String, String)> = {
            let mut statement = transaction.prepare(
                "SELECT a.id,l.locator FROM artifacts a INDEXED BY background_directory_artifact_root
                JOIN artifact_locators l INDEXED BY background_directory_artifact_locator ON l.artifact_id=a.id AND l.kind='file' AND l.active=1
                WHERE a.source_root_id=?1 AND a.state='active' LIMIT ?2",
            )?;
            let rows = statement.query_map(
                params![target.authorization.root_id, target.max_files + 1],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        if candidates.len() > target.max_files as usize {
            return Err(invalid(
                "directory reconciliation exceeds its configured file limit",
            ));
        }
        // Cross-check the complete retained manifest, including unsupported/OCR-disabled units.
        let count: u32 = transaction.query_row(
            "SELECT COUNT(*) FROM background_directory_units WHERE job_id=?1",
            [claim.id()],
            |row| row.get(0),
        )?;
        if count != target.total_units {
            return Err(invalid("incomplete directory manifest"));
        }
        for ordinal in 0..target.total_units {
            let unit = load_unit(&transaction, claim.id(), ordinal)?;
            unit.validate(target)?;
            if namespace.files[ordinal as usize] != Path::new(&unit.locator)
                || namespace.file_identities[ordinal as usize]
                    != (unit.identity.device, unit.identity.inode)
            {
                return Err(LoomError::SourceChanged(unit.locator));
            }
        }
        namespace.verify_directories(
            Path::new(&target.locator),
            crate::discovery::DiscoveryLimits::for_files(target.max_files as usize),
        )?;
        target.verify(&transaction)?;
        let mut missing = 0;
        for (artifact_id, locator) in candidates {
            if !seen.contains(&locator) {
                missing += transaction.execute("UPDATE artifacts SET state='missing' WHERE id=?1 AND source_root_id=?2 AND state='active'", params![artifact_id, target.authorization.root_id])?;
            }
        }
        let result = serde_json::to_string(
            &serde_json::json!({"indexed": progress.indexed, "unchanged": progress.unchanged,
            "skipped": progress.skipped, "bytes_read": progress.bytes_read, "total_units": progress.total_units,
            "missing": missing, "namespace_fingerprint": target.namespace_fingerprint}),
        )?;
        let completed = claim.complete(&transaction, &result)?;
        transaction.commit()?;
        Ok(completed)
    }
}

fn directory_progress(
    connection: &Connection,
    id: &str,
    target: &IndexDirectoryTarget,
) -> Result<DirectoryProgress> {
    let progress = connection
        .query_row(
            "SELECT total_units,next_unit,indexed,unchanged,skipped,bytes_read,last_extraction_json
        FROM background_directory_manifests WHERE job_id=?1 AND root_id=?2 AND root_locator=?3",
            params![id, target.authorization.root_id, target.locator],
            |row| {
                Ok(DirectoryProgress {
                    total_units: row.get(0)?,
                    next_unit: row.get(1)?,
                    indexed: row.get(2)?,
                    unchanged: row.get(3)?,
                    skipped: row.get(4)?,
                    bytes_read: crate::jobs::row_bytes(row, 5)?,
                    last_extraction: crate::jobs::row_metrics(row, 6)?,
                })
            },
        )
        .optional()?
        .ok_or_else(|| invalid("directory manifest is unavailable"))?;
    if progress.total_units != target.total_units
        || progress.next_unit > progress.total_units
        || u64::from(progress.indexed) + u64::from(progress.unchanged) + u64::from(progress.skipped)
            != u64::from(progress.next_unit)
        || progress.bytes_read > 167_772_160_000
    {
        return Err(invalid("invalid directory manifest progress"));
    }
    Ok(progress)
}

fn load_unit(connection: &Connection, id: &str, ordinal: u32) -> Result<DirectoryUnit> {
    let raw = connection
        .query_row(
            "SELECT relative_path,locator,media_type,identity_json,artifact_id
        FROM background_directory_units WHERE job_id=?1 AND ordinal=?2",
            params![id, ordinal],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            },
        )
        .optional()?
        .ok_or_else(|| invalid("directory manifest unit is unavailable"))?;
    Ok(DirectoryUnit {
        ordinal,
        relative_path: raw.0,
        locator: raw.1,
        media_type: raw.2,
        identity: serde_json::from_str(&raw.3)
            .map_err(|_| invalid("invalid directory file identity"))?,
        artifact_id: raw.4,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::JobPriority;
    use tempfile::{tempdir, TempDir};

    fn fixture() -> (TempDir, Library, PathBuf) {
        let temporary = tempdir().unwrap();
        let root = temporary.path().join("selected");
        fs::create_dir(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let library = Library::open(temporary.path().join("library.sqlite3")).unwrap();
        library.index_path(&root).unwrap();
        (temporary, library, root)
    }

    fn helper() -> loom_extraction::ExtractionSupervisor {
        let path = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join(format!(
                "loom-core-test-extractor{}",
                std::env::consts::EXE_SUFFIX
            ));
        loom_extraction::ExtractionSupervisor::new(path).unwrap()
    }

    fn claim(library: &Library) -> (crate::JobWorker, JobClaim, String) {
        let mut worker = library.acquire_job_worker().unwrap();
        let claim = worker.claim_for_test().unwrap().unwrap();
        let json = library
            .lock()
            .unwrap()
            .query_row(
                "SELECT target_json FROM background_jobs WHERE id=?1",
                [claim.id()],
                |row| row.get(0),
            )
            .unwrap();
        (worker, claim, json)
    }

    #[test]
    fn publication_cursor_and_nonterminal_yield_roll_back_together() {
        let (_temporary, library, root) = fixture();
        fs::write(root.join("a.md"), "Atomic directory publication marker").unwrap();
        let job = library
            .enqueue_index_directory(&root, "atomic", JobPriority::Normal)
            .unwrap();
        let (_worker, claim, json) = claim(&library);
        let prepared = library
            .prepare_directory_file(&claim, &json, Some(&helper()))
            .unwrap()
            .unwrap();
        let before = library.export_portable().unwrap().digest;
        library.lock().unwrap().execute_batch("CREATE TRIGGER refuse_cursor BEFORE UPDATE OF next_unit ON background_directory_manifests
            BEGIN SELECT RAISE(ABORT, 'injected cursor failure'); END;").unwrap();
        assert!(library.publish_directory_file(&claim, &prepared).is_err());
        assert_eq!(library.export_portable().unwrap().digest, before);
        let running = library.background_job(&job.id).unwrap();
        assert_eq!(running.state, JobState::Running);
        assert_eq!(running.directory_progress.unwrap().next_unit, 0);
        assert_eq!(running.attempts, 1);
        library
            .lock()
            .unwrap()
            .execute_batch("DROP TRIGGER refuse_cursor;")
            .unwrap();
        let queued = library.publish_directory_file(&claim, &prepared).unwrap();
        assert_eq!(queued.state, JobState::Queued);
        assert_eq!(queued.attempts, 0);
        assert_eq!(queued.directory_progress.unwrap().next_unit, 1);
        assert_eq!(library.stats().unwrap().artifacts, 1);
        assert!(library.publish_directory_file(&claim, &prepared).is_err());
        assert_eq!(library.stats().unwrap().versions, 1);
    }

    #[test]
    fn canonical_and_content_cas_refuse_stale_prepared_directory_results() {
        for mutation in ["content", "canonical", "unit", "revoke"] {
            let (_temporary, library, root) = fixture();
            let source = root.join("a.md");
            fs::write(&source, "Original directory marker").unwrap();
            let job = library
                .enqueue_index_directory(&root, "cas", JobPriority::Normal)
                .unwrap();
            let (_worker, claim, json) = claim(&library);
            let prepared = library
                .prepare_directory_file(&claim, &json, Some(&helper()))
                .unwrap()
                .unwrap();
            match mutation {
                "content" => fs::write(&source, "Changed source marker").unwrap(),
                "canonical" => {
                    library.index_path(&root).unwrap();
                }
                "unit" => {
                    library.lock().unwrap().execute("UPDATE background_directory_units SET relative_path='../outside.md' WHERE job_id=?1", [&job.id]).unwrap();
                }
                "revoke" => {
                    library.revoke_source_root(root.to_str().unwrap()).unwrap();
                }
                _ => unreachable!(),
            }
            let before = library.export_portable().unwrap().digest;
            assert!(
                library.publish_directory_file(&claim, &prepared).is_err(),
                "{mutation}"
            );
            assert_eq!(library.export_portable().unwrap().digest, before);
            assert_eq!(
                library
                    .background_job(&job.id)
                    .unwrap()
                    .directory_progress
                    .unwrap()
                    .next_unit,
                0
            );
        }
    }

    #[test]
    fn namespace_change_after_discovery_before_writer_prevents_missing_cleanup() {
        let (_temporary, library, root) = fixture();
        let source = root.join("old.md");
        fs::write(&source, "Retained missing source marker").unwrap();
        library.index_path(&root).unwrap();
        fs::remove_file(&source).unwrap();
        library
            .enqueue_index_directory(&root, "final-gap", JobPriority::Normal)
            .unwrap();
        let (_worker, claim, json) = claim(&library);
        let target = IndexDirectoryTarget::parse(&json).unwrap();
        let namespace = crate::discovery::walk_snapshot(
            &root,
            crate::discovery::DiscoveryLimits::for_files(target.max_files as usize),
            |_| Ok(()),
        )
        .unwrap();
        fs::create_dir(root.join("late-directory")).unwrap();
        let before = library.export_portable().unwrap().digest;
        assert!(matches!(
            library.finish_directory_snapshot(&claim, &json, &target, namespace),
            Err(LoomError::SourceChanged(_))
        ));
        assert_eq!(library.export_portable().unwrap().digest, before);
        assert_eq!(
            library.background_job(claim.id()).unwrap().state,
            JobState::Running
        );
    }

    #[test]
    fn replaced_root_after_preparation_cannot_supply_even_the_same_child_inode() {
        let (_temporary, library, root) = fixture();
        fs::write(root.join("a.md"), "Private original source marker").unwrap();
        library
            .enqueue_index_directory(&root, "read-gap", JobPriority::Normal)
            .unwrap();
        let (_worker, claim, json) = claim(&library);
        let before = library.export_portable().unwrap().digest;
        let moved = root.with_extension("original");
        let error =
            match library.prepare_directory_file_before_read(&claim, &json, Some(&helper()), || {
                fs::rename(&root, &moved).unwrap();
                fs::create_dir(&root).unwrap();
                // A shared child inode alone does not authorize a replacement root.
                fs::hard_link(moved.join("a.md"), root.join("a.md")).unwrap();
            }) {
                Err(error) => error,
                Ok(_) => panic!("replacement root reached extraction"),
            };
        assert!(matches!(error, LoomError::SourceRevoked(_)), "{error}");
        assert_eq!(library.export_portable().unwrap().digest, before);
        assert_eq!(library.stats().unwrap().artifacts, 0);
        assert_eq!(
            library
                .background_job(claim.id())
                .unwrap()
                .directory_progress
                .unwrap()
                .next_unit,
            0
        );
    }

    #[test]
    fn birth_time_and_root_change_fence_device_inode_aba() {
        let (_temporary, library, root) = fixture();
        let path = root.join("a.md");
        fs::write(&path, "Identity marker").unwrap();
        let metadata = fs::metadata(&path).unwrap();
        let mut identity = SourceIdentity::capture(&path, false).unwrap();
        // Simulate a recycled dev/inode with an earlier filesystem birth time.
        identity.created.0 -= 1;
        assert!(matches!(
            identity.verify_metadata(&metadata, &path, false),
            Err(LoomError::SourceRevoked(_))
        ));
        let mut target = library.prepare_directory_admission(&root).unwrap().target;
        // Directory metadata is checked too even when device/inode/birth time match.
        target.root_change.0 -= 1;
        assert!(matches!(
            target.verify_root(),
            Err(LoomError::SourceChanged(_))
        ));
    }

    #[test]
    fn aggregate_manifest_capacity_refusal_rolls_back_admission() {
        let (_temporary, library, root) = fixture();
        fs::write(root.join("a.md"), "Capacity marker").unwrap();
        let job = library
            .enqueue_index_directory(&root, "first", JobPriority::Normal)
            .unwrap();
        let before = library.export_portable().unwrap().digest;
        library
            .lock()
            .unwrap()
            .execute(
                "UPDATE background_directory_manifests SET encoded_bytes=67108864 WHERE job_id=?1",
                [&job.id],
            )
            .unwrap();
        let error = library
            .enqueue_index_directory(&root, "over-budget", JobPriority::Normal)
            .unwrap_err();
        assert!(error.to_string().contains("manifest capacity exceeded"));
        assert_eq!(library.background_jobs(128).unwrap().len(), 1);
        assert_eq!(library.export_portable().unwrap().digest, before);
        assert!(enforce_manifest_capacity(
            MAX_RETAINED_DIRECTORY_UNITS - 1,
            0,
            1,
            MAX_RETAINED_DIRECTORY_BYTES
        )
        .is_ok());
        assert!(enforce_manifest_capacity(MAX_RETAINED_DIRECTORY_UNITS, 0, 1, 0).is_err());
        assert!(enforce_manifest_capacity(0, MAX_RETAINED_DIRECTORY_BYTES, 0, 1).is_err());
    }

    #[test]
    fn malformed_empty_target_blocks_purge_until_explicit_diagnostic_removal() {
        let (_temporary, library, root) = fixture();
        let job = library
            .enqueue_index_directory(&root, "malformed", JobPriority::Normal)
            .unwrap();
        library
            .lock()
            .unwrap()
            .execute(
                "UPDATE background_jobs SET target_json='{}' WHERE id=?1",
                [&job.id],
            )
            .unwrap();
        let before = library.export_portable().unwrap().digest;
        assert!(library
            .purge_root(root.to_str().unwrap())
            .unwrap_err()
            .to_string()
            .contains("no data was deleted"));
        assert_eq!(library.export_portable().unwrap().digest, before);
        library.cancel_background_job(&job.id).unwrap();
        library.forget_background_job(&job.id).unwrap();
        library.purge_root(root.to_str().unwrap()).unwrap();
        assert!(library.source_roots().unwrap().is_empty());
        let retained: i64 = library
            .lock()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM background_directory_manifests",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(retained, 0);
    }

    #[test]
    fn tombstoned_artifact_cannot_be_reactivated_by_directory_admission() {
        let (_temporary, library, root) = fixture();
        fs::write(root.join("a.md"), "Tombstoned marker").unwrap();
        library.index_path(&root).unwrap();
        library
            .lock()
            .unwrap()
            .execute("UPDATE artifacts SET state='tombstoned'", [])
            .unwrap();
        let before = library.export_portable().unwrap().digest;
        assert!(matches!(
            library.enqueue_index_directory(&root, "tombstone", JobPriority::Normal),
            Err(LoomError::SourceRevoked(_))
        ));
        assert_eq!(library.export_portable().unwrap().digest, before);
        assert!(library.background_jobs(128).unwrap().is_empty());
    }
}
