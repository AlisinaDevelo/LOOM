//! Bounded embedding quanta over canonical evidence, with one atomic generation pointer.
use super::*;
use crate::jobs::{BackgroundJob, JobClaim};

const MAX_UNITS: usize = 20_000;
const MAX_TEXT_BYTES: u64 = 64 * 1024;
const MAX_SOURCE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_RETAINED_UNITS: u64 = 65_536;

fn invalid(message: &str) -> LoomError {
    LoomError::JobQueue(message.into())
}
fn digest(bytes: &[u8]) -> String {
    format!("blake3:{}", blake3::hash(bytes).to_hex())
}
fn valid_digest(value: &str) -> bool {
    value.strip_prefix("blake3:").is_some_and(|value| {
        value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
    })
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Target {
    config: SemanticIndexConfig,
    incarnation: String,
    purge_revision: String,
    ocr_policy: OcrPolicy,
    source_digest: String,
    membership_digest: String,
    total_units: u32,
    source_bytes: u64,
}

impl Target {
    fn parse(json: &str) -> Result<Self> {
        if json.len() > 16384 {
            return Err(invalid("semantic target exceeds 16 KiB"));
        }
        let target: Self =
            serde_json::from_str(json).map_err(|_| invalid("invalid typed semantic target"))?;
        if target.config != SemanticIndexConfig::default()
            || Uuid::parse_str(&target.incarnation).is_err()
            || (!target.purge_revision.is_empty()
                && Uuid::parse_str(&target.purge_revision).is_err())
            || Uuid::parse_str(&target.ocr_policy.revision).is_err()
            || !valid_digest(&target.source_digest)
            || !valid_digest(&target.membership_digest)
            || target.total_units as usize > MAX_UNITS
            || target.source_bytes > MAX_SOURCE_BYTES
        {
            return Err(invalid("invalid semantic capability shape"));
        }
        Ok(target)
    }

    fn verify_policy(&self, connection: &Connection) -> Result<()> {
        let incarnation: String = connection.query_row(
            "SELECT value FROM schema_meta WHERE key='authorization_incarnation'",
            [],
            |row| row.get(0),
        )?;
        if incarnation != self.incarnation {
            return Err(LoomError::SourceRevoked(
                "semantic corpus incarnation".into(),
            ));
        }
        if purge_revision(connection)? != self.purge_revision {
            return Err(LoomError::SourceRevoked("semantic purge revision".into()));
        }
        self.ocr_policy.verify_current(connection)
    }

    fn report(&self) -> SemanticRebuildReport {
        SemanticRebuildReport {
            manifest: SemanticIndexManifest {
                config: self.config.clone(),
                source_digest: self.source_digest.clone(),
                canonical_passages: u64::from(self.total_units),
                indexed_passages: u64::from(self.total_units),
                vector_bytes: u64::from(self.total_units) * 512,
            },
            rebuilt_passages: u64::from(self.total_units),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Unit {
    passage_id: String,
    passage_hash: String,
    artifact_id: String,
    version_id: String,
    root_id: String,
    scope_generation: i64,
    text_bytes: u64,
}

impl Unit {
    fn hash(&self) -> Result<String> {
        Ok(digest(&serde_json::to_vec(self)?))
    }
    fn validate(&self) -> Result<()> {
        if [
            &self.passage_id,
            &self.artifact_id,
            &self.version_id,
            &self.root_id,
        ]
        .iter()
        .any(|id| Uuid::parse_str(id).is_err())
            || !valid_digest(&self.passage_hash)
            || self.scope_generation < 0
            || self.text_bytes > MAX_TEXT_BYTES
        {
            return Err(invalid("semantic unit exceeds supported bounds"));
        }
        Ok(())
    }
}

const MEMBERSHIP: &str =
    "SELECT p.id,p.text_hash,a.id,v.id,r.id,r.scope_generation,length(CAST(p.text AS BLOB))
    FROM passages p JOIN artifact_versions v ON v.id=p.artifact_version_id
    JOIN artifacts a ON a.id=v.artifact_id AND a.active_version_id=v.id
    JOIN source_roots r ON r.id=a.source_root_id AND r.enabled=1
    WHERE a.state='active' ORDER BY p.id LIMIT 20001";

fn unit_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Unit> {
    Ok(Unit {
        passage_id: row.get(0)?,
        passage_hash: row.get(1)?,
        artifact_id: row.get(2)?,
        version_id: row.get(3)?,
        root_id: row.get(4)?,
        scope_generation: row.get(5)?,
        text_bytes: crate::jobs::row_bytes(row, 6)?,
    })
}

fn capture(connection: &Connection) -> Result<(Target, Vec<Unit>)> {
    let mut statement = connection.prepare(MEMBERSHIP)?;
    let mut units = Vec::new();
    let mut membership = blake3::Hasher::new();
    let mut source = blake3::Hasher::new();
    let mut source_bytes = 0u64;
    for unit in statement.query_map([], unit_row)? {
        let unit = unit?;
        unit.validate()?;
        if units.len() >= MAX_UNITS {
            return Err(invalid("semantic corpus exceeds 20000 passages"));
        }
        source_bytes = source_bytes
            .checked_add(unit.text_bytes)
            .filter(|bytes| *bytes <= MAX_SOURCE_BYTES)
            .ok_or_else(|| invalid("semantic corpus exceeds 64 MiB input"))?;
        source.update(unit.passage_id.as_bytes());
        source.update(&[0]);
        source.update(unit.passage_hash.as_bytes());
        source.update(&[0]);
        membership.update(unit.hash()?.as_bytes());
        membership.update(&[0]);
        units.push(unit);
    }
    let incarnation = connection.query_row(
        "SELECT value FROM schema_meta WHERE key='authorization_incarnation'",
        [],
        |row| row.get(0),
    )?;
    let target = Target {
        config: SemanticIndexConfig::default(),
        incarnation,
        purge_revision: purge_revision(connection)?,
        ocr_policy: OcrPolicy::load(connection)?,
        source_digest: format!("blake3:{}", source.finalize().to_hex()),
        membership_digest: format!("blake3:{}", membership.finalize().to_hex()),
        total_units: units.len() as u32,
        source_bytes,
    };
    // Validate newly captured data against the same closed contract as persisted targets.
    Target::parse(&serde_json::to_string(&target)?)?;
    Ok((target, units))
}

fn purge_revision(connection: &Connection) -> Result<String> {
    Ok(connection.query_row("SELECT COALESCE((SELECT value FROM schema_meta WHERE key='source_selection_purge_revision'), '')", [], |row| row.get(0))?)
}

pub(super) fn check_corpus_bounds(connection: &Connection) -> Result<()> {
    // Metadata and byte lengths only; reject the corpus before foreground collects any text.
    capture(connection).map(|_| ())
}

pub(super) fn current_runtime(connection: &Connection) -> Result<bool> {
    let version: Option<String> = connection
        .query_row(
            "SELECT value FROM schema_meta WHERE key='background_job_schema_version'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    Ok(version.as_deref() == Some("6"))
}

pub(super) fn require_current_runtime(connection: &Connection) -> Result<()> {
    if !current_runtime(connection)? {
        return Err(LoomError::SemanticIndexUnavailable(
            "semantic use requires runtime v6; explicitly run upgrade-job-runtime".into(),
        ));
    }
    crate::jobs::validate_semantic_layout(connection)
}

pub(super) fn invalid_canonical_hashes(connection: &Connection) -> Result<u64> {
    // Stream bounded text, not an unbounded corpus allocation. Oversized passages/corpora are
    // incompatible too. Rehashing detects internally consistent but false stored hashes.
    let mut statement = connection.prepare(
        "SELECT p.text_hash,
        CASE WHEN length(CAST(p.text AS BLOB))<=65536 THEN p.text ELSE NULL END
        FROM passages p JOIN artifact_versions v ON v.id=p.artifact_version_id
        JOIN artifacts a ON a.id=v.artifact_id AND a.active_version_id=v.id
        JOIN source_roots r ON r.id=a.source_root_id AND r.enabled=1
        WHERE a.state='active' ORDER BY p.id LIMIT 20001",
    )?;
    let mut invalid = 0;
    let mut bytes = 0u64;
    for (ordinal, row) in statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })?
        .enumerate()
    {
        let (hash, text) = row?;
        if ordinal >= MAX_UNITS {
            return Ok(invalid + 1);
        }
        match text {
            Some(text) => {
                bytes += text.len() as u64;
                if digest(text.as_bytes()) != hash {
                    invalid += 1;
                }
                if bytes > MAX_SOURCE_BYTES {
                    return Ok(invalid + 1);
                }
            }
            None => invalid += 1,
        }
    }
    Ok(invalid)
}

pub(super) fn storage_bytes(connection: &Connection) -> Result<u64> {
    if !current_runtime(connection)? {
        return Ok(0);
    }
    require_current_runtime(connection)?;
    Ok(connection.query_row("SELECT
        (SELECT COALESCE(SUM(length(id)+length(COALESCE(job_id,''))+length(CAST(target_json AS BLOB))+32),0) FROM background_semantic_builds) +
        (SELECT COALESCE(SUM(length(build_id)+length(passage_id)+length(passage_hash)+length(artifact_id)+length(version_id)+length(root_id)+length(unit_hash)+length(COALESCE(vector_hash,''))+COALESCE(length(vector_blob),0)+24),0) FROM background_semantic_units) +
        (SELECT COALESCE(SUM(length(build_id)+8),0) FROM background_semantic_active)", [], |row| crate::jobs::row_bytes(row,0))?)
}

pub(crate) struct SemanticAdmission {
    target: Target,
    units: Vec<Unit>,
}
impl SemanticAdmission {
    pub(crate) fn target_json(&self) -> Result<String> {
        Ok(serde_json::to_string(&self.target)?)
    }
    pub(crate) fn insert(&self, connection: &Connection, job: &BackgroundJob) -> Result<()> {
        // Retire only unreferenced published generations. Pending jobs and the active projection
        // are never evicted to admit a request. All private rows have an aggregate finite bound.
        connection.execute("DELETE FROM background_semantic_builds WHERE job_id IS NULL AND id NOT IN (SELECT build_id FROM background_semantic_active)", [])?;
        let retained: u64 = connection.query_row(
            "SELECT COUNT(*) FROM background_semantic_units",
            [],
            |row| crate::jobs::row_bytes(row, 0),
        )?;
        if retained + self.units.len() as u64 > MAX_RETAINED_UNITS {
            return Err(invalid("semantic retained-unit capacity reached"));
        }
        let build = Uuid::new_v4().to_string();
        connection.execute("INSERT INTO background_semantic_builds(id,job_id,target_json,total_units,source_bytes) VALUES (?1,?2,?3,?4,?5)",
            params![build,job.id,self.target_json()?,self.target.total_units,sql_i64(self.target.source_bytes,"semantic source bytes")?])?;
        let mut insert=connection.prepare("INSERT INTO background_semantic_units(build_id,ordinal,passage_id,passage_hash,artifact_id,version_id,root_id,scope_generation,text_bytes,unit_hash) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)")?;
        for (ordinal, unit) in self.units.iter().enumerate() {
            insert.execute(params![
                build,
                ordinal as u32,
                unit.passage_id,
                unit.passage_hash,
                unit.artifact_id,
                unit.version_id,
                unit.root_id,
                unit.scope_generation,
                sql_i64(unit.text_bytes, "semantic passage bytes")?,
                unit.hash()?
            ])?;
        }
        Ok(())
    }
}

fn load_unit(
    connection: &Connection,
    build: &str,
    ordinal: u32,
) -> Result<(Unit, Option<Vec<u8>>, Option<String>)> {
    let (unit,hash,vector,vector_hash)=connection.query_row("SELECT passage_id,passage_hash,artifact_id,version_id,root_id,scope_generation,text_bytes,unit_hash,
        CASE WHEN length(vector_blob)=512 THEN vector_blob ELSE NULL END,vector_hash FROM background_semantic_units WHERE build_id=?1 AND ordinal=?2",
        params![build,ordinal],|row| Ok((unit_row(row)?,row.get::<_,String>(7)?,row.get::<_,Option<Vec<u8>>>(8)?,row.get::<_,Option<String>>(9)?)))?;
    unit.validate()?;
    if unit.hash()? != hash {
        return Err(invalid("semantic unit metadata checksum mismatch"));
    }
    Ok((unit, vector, vector_hash))
}

fn text_for_unit(connection: &Connection, unit: &Unit) -> Result<String> {
    // Check the stored byte length in SQL before allocating content. No file/OCR access here.
    let text: Option<String>=connection.query_row("SELECT p.text FROM passages p
        JOIN artifact_versions v ON v.id=p.artifact_version_id JOIN artifacts a ON a.id=v.artifact_id AND a.active_version_id=v.id
        JOIN source_roots r ON r.id=a.source_root_id AND r.enabled=1
        WHERE p.id=?1 AND p.text_hash=?2 AND a.id=?3 AND v.id=?4 AND r.id=?5 AND r.scope_generation=?6
        AND a.state='active' AND length(CAST(p.text AS BLOB))=?7 AND length(CAST(p.text AS BLOB))<=65536",
        params![unit.passage_id,unit.passage_hash,unit.artifact_id,unit.version_id,unit.root_id,unit.scope_generation,sql_i64(unit.text_bytes,"semantic passage bytes")?],|row| row.get(0)).optional()?;
    let text =
        text.ok_or_else(|| LoomError::SourceRevoked("semantic passage membership".into()))?;
    if digest(text.as_bytes()) != unit.passage_hash {
        return Err(invalid("canonical passage hash mismatch"));
    }
    Ok(text)
}

fn build_state(connection: &Connection, job: &str, json: &str) -> Result<(String, u32, u64)> {
    let (id,target,total,next,source_bytes,vector_bytes): (String,String,u32,u32,u64,u64)=connection.query_row(
        "SELECT id,target_json,total_units,next_unit,source_bytes,vector_bytes FROM background_semantic_builds WHERE job_id=?1", [job],
        |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,crate::jobs::row_bytes(row,4)?,crate::jobs::row_bytes(row,5)?)))?;
    let typed = Target::parse(json)?;
    if target != json
        || total != typed.total_units
        || source_bytes != typed.source_bytes
        || vector_bytes != u64::from(next) * 512
        || next > total
    {
        return Err(invalid("semantic manifest cursor or target mismatch"));
    }
    Ok((id, next, vector_bytes))
}

impl Library {
    pub(crate) fn prepare_semantic_admission(
        &self,
        connection: &Connection,
    ) -> Result<SemanticAdmission> {
        let (target, units) = capture(connection)?;
        Ok(SemanticAdmission { target, units })
    }

    pub(crate) fn job_semantic_rebuild(
        &self,
        claim: &JobClaim,
        json: &str,
    ) -> Result<BackgroundJob> {
        self.job_semantic_rebuild_with_hook(claim, json, || {})
    }

    pub(crate) fn job_semantic_rebuild_with_hook(
        &self,
        claim: &JobClaim,
        json: &str,
        before_commit: impl FnOnce(),
    ) -> Result<BackgroundJob> {
        let target = Target::parse(json)?;
        let (build, ordinal, unit, text) = {
            let mut connection = self.lock()?;
            let transaction = connection.transaction()?;
            claim.verify_operation(&transaction, "semantic_rebuild", Some(json))?;
            target.verify_policy(&transaction)?;
            let (build, ordinal, _) = build_state(&transaction, claim.id(), json)?;
            if ordinal == target.total_units {
                drop(transaction);
                drop(connection);
                return self.publish_semantic(claim, json, &target);
            }
            let (unit, vector, hash) = load_unit(&transaction, &build, ordinal)?;
            if vector.is_some() || hash.is_some() {
                return Err(invalid(
                    "semantic cursor points to an already embedded unit",
                ));
            }
            let text = text_for_unit(&transaction, &unit)?;
            (build, ordinal, unit, text)
        };
        // Provider work happens outside both the connection mutex and SQLite transaction.
        let encoded = encode_vector(&HashEmbeddingProvider::default().embed(&text));
        let vector_hash = digest(&encoded);
        before_commit();
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        claim.verify_operation(&transaction, "semantic_rebuild", Some(json))?;
        target.verify_policy(&transaction)?;
        if build_state(&transaction, claim.id(), json)?.0 != build
            || build_state(&transaction, claim.id(), json)?.1 != ordinal
            || load_unit(&transaction, &build, ordinal)?.0 != unit
            || text_for_unit(&transaction, &unit)? != text
        {
            return Err(invalid("semantic quantum changed before commit"));
        }
        if transaction.execute("UPDATE background_semantic_units SET vector_blob=?1,vector_hash=?2 WHERE build_id=?3 AND ordinal=?4 AND vector_blob IS NULL AND vector_hash IS NULL",params![encoded,vector_hash,build,ordinal])?!=1 {
            return Err(invalid("semantic unit compare-and-swap failed"));
        }
        if transaction.execute("UPDATE background_semantic_builds SET next_unit=next_unit+1,vector_bytes=vector_bytes+512 WHERE id=?1 AND next_unit=?2",params![build,ordinal])?!=1 {
            return Err(invalid("semantic cursor compare-and-swap failed"));
        }
        let job = claim.yield_progress(&transaction)?;
        transaction.commit()?;
        Ok(job)
    }

    fn publish_semantic(
        &self,
        claim: &JobClaim,
        json: &str,
        target: &Target,
    ) -> Result<BackgroundJob> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        claim.verify_operation(&transaction, "semantic_rebuild", Some(json))?;
        target.verify_policy(&transaction)?;
        let (build, next, vector_bytes) = build_state(&transaction, claim.id(), json)?;
        let (current, units) = capture(&transaction)?;
        if &current != target
            || next != target.total_units
            || vector_bytes != target.report().manifest.vector_bytes
        {
            return Err(LoomError::SourceRevoked(
                "semantic corpus changed before publication".into(),
            ));
        }
        let stored: u64 = transaction.query_row(
            "SELECT COUNT(*) FROM background_semantic_units WHERE build_id=?1",
            [&build],
            |row| crate::jobs::row_bytes(row, 0),
        )?;
        if stored != u64::from(target.total_units) {
            return Err(invalid("semantic staged membership count mismatch"));
        }
        for (ordinal, expected) in units.iter().enumerate() {
            let (unit, vector, hash) = load_unit(&transaction, &build, ordinal as u32)?;
            let vector = vector.ok_or_else(|| invalid("semantic vector is missing"))?;
            if &unit != expected
                || hash.as_deref() != Some(&digest(&vector))
                || decode_vector(&vector, target.config.dimension).is_none()
            {
                return Err(invalid(
                    "semantic staged vector or membership is incompatible",
                ));
            }
        }
        // O(1) pointer publication. Old unreferenced generations are bounded and pruned at
        // admission, not while swapping the index. Detach before job completion/forget cascades.
        if transaction.execute(
            "UPDATE background_semantic_builds SET job_id=NULL WHERE id=?1 AND job_id=?2",
            params![build, claim.id()],
        )? != 1
        {
            return Err(invalid("semantic generation detachment failed"));
        }
        if transaction.execute("INSERT INTO background_semantic_active(slot,build_id) VALUES (1,?1) ON CONFLICT(slot) DO UPDATE SET build_id=excluded.build_id",[build])?!=1 {
            return Err(invalid("semantic active-pointer publication failed"));
        }
        let job = claim.complete(&transaction, &serde_json::to_string(&target.report())?)?;
        transaction.commit()?;
        Ok(job)
    }
}

/// A released v5 library has no queued semantic work. New v6 foreground operations invalidate
/// every staged/published generation in the same writer transaction, including an empty drop.
pub(super) fn clear_foreground(connection: &Connection) -> Result<(u64, bool)> {
    let version: Option<String> = connection
        .query_row(
            "SELECT value FROM schema_meta WHERE key='background_job_schema_version'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    if version.as_deref() == Some("6") {
        crate::jobs::validate_semantic_layout(connection)?;
        let count = connection.query_row(
            "SELECT COUNT(*) FROM background_semantic_units WHERE vector_blob IS NOT NULL",
            [],
            |row| crate::jobs::row_bytes(row, 0),
        )?;
        let active: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM background_semantic_active)",
            [],
            |row| row.get(0),
        )?;
        crate::jobs::clear_semantic(connection)?;
        return Ok((count, active));
    }
    crate::jobs::validate_legacy_semantic_mutation(connection, version.as_deref())?;
    Ok((0, false))
}

pub(super) fn active_target(connection: &Connection) -> Result<Option<(String, String)>> {
    let version: Option<String> = connection
        .query_row(
            "SELECT value FROM schema_meta WHERE key='background_job_schema_version'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    if version.as_deref() != Some("6") {
        return Ok(None);
    }
    crate::jobs::validate_semantic_layout(connection)?;
    let active: Option<(String, Option<String>, Option<String>)> = connection.query_row(
        "SELECT a.build_id,b.target_json,b.job_id FROM background_semantic_active a LEFT JOIN background_semantic_builds b ON b.id=a.build_id WHERE a.slot=1",
        [], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional()?;
    match active {
        None => Ok(None),
        Some((build, Some(json), None)) if Uuid::parse_str(&build).is_ok() => {
            Ok(Some((build, json)))
        }
        Some(_) => Err(invalid(
            "active semantic pointer is missing or still job-owned",
        )),
    }
}

pub(super) fn status(connection: &Connection) -> Result<Option<SemanticIndexStatus>> {
    let Some((build, json)) = active_target(connection)? else {
        return Ok(None);
    };
    let target = Target::parse(&json)?;
    let (canonical_passages, canonical_digest) = canonical_semantic_source(connection)?;
    let (stored_units,indexed_passages,vector_bytes): (u64,u64,u64)=connection.query_row("SELECT COUNT(*),COUNT(vector_blob),COALESCE(SUM(length(vector_blob)),0) FROM background_semantic_units WHERE build_id=?1",[&build],|row| Ok((crate::jobs::row_bytes(row,0)?,crate::jobs::row_bytes(row,1)?,crate::jobs::row_bytes(row,2)?)))?;
    let (total,next,source_bytes,stored_vector_bytes): (u32,u32,u64,u64) = connection.query_row(
        "SELECT total_units,next_unit,source_bytes,vector_bytes FROM background_semantic_builds WHERE id=?1", [&build],
        |row| Ok((row.get(0)?,row.get(1)?,crate::jobs::row_bytes(row,2)?,crate::jobs::row_bytes(row,3)?)))?;
    let mut reasons = Vec::new();
    if target.verify_policy(connection).is_err() {
        reasons.push("source/OCR capability changed");
    }
    let captured = capture(connection);
    if !captured
        .as_ref()
        .is_ok_and(|(current, _)| current == &target)
    {
        reasons.push("canonical membership changed; rebuild required");
    }
    if canonical_passages != u64::from(target.total_units)
        || stored_units != canonical_passages
        || indexed_passages != canonical_passages
        || vector_bytes != indexed_passages * 512
        || total != target.total_units
        || next != total
        || source_bytes != target.source_bytes
        || stored_vector_bytes != vector_bytes
    {
        reasons.push("semantic vector count/bytes mismatch");
    }
    // Status and search share a read snapshot. Validate bounded staged bytes before reporting
    // healthy, including nonfinite vectors and corruption that preserved the blob length.
    for ordinal in 0..target.total_units {
        let (unit, vector, hash) = load_unit(connection, &build, ordinal)?;
        if vector.as_ref().is_none_or(|vector| {
            hash.as_deref() != Some(&digest(vector))
                || decode_vector(vector, target.config.dimension).is_none()
        }) || captured
            .as_ref()
            .ok()
            .and_then(|(_, units)| units.get(ordinal as usize))
            != Some(&unit)
        {
            reasons.push("semantic vector binding/bytes mismatch");
            break;
        }
    }
    Ok(Some(SemanticIndexStatus {
        healthy: reasons.is_empty(),
        canonical_passages,
        indexed_passages,
        canonical_digest,
        vector_bytes,
        manifest: Some(target.report().manifest),
        reason: (!reasons.is_empty()).then(|| reasons.join("; ")),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{JobPriority, JobState, JobWorker, SearchRequest};
    use tempfile::{tempdir, TempDir};

    fn fixture() -> (TempDir, Library, PathBuf) {
        let temporary = tempdir().unwrap();
        let root = temporary.path().join("selected");
        fs::create_dir(&root).unwrap();
        for name in ["a", "b"] {
            fs::write(
                root.join(format!("{name}.md")),
                format!("Synthetic semantic evidence {name}.\n"),
            )
            .unwrap();
        }
        let root = root.canonicalize().unwrap();
        let library = Library::open(temporary.path().join("library.sqlite3")).unwrap();
        library.index_path(&root).unwrap();
        library.semantic_rebuild().unwrap();
        (temporary, library, root)
    }

    fn worker(library: &Library) -> JobWorker {
        let started = std::time::Instant::now();
        loop {
            match library.acquire_job_worker() {
                Err(LoomError::JobWorkerBusy)
                    if started.elapsed() < std::time::Duration::from_millis(500) =>
                {
                    std::thread::sleep(std::time::Duration::from_millis(5))
                }
                result => return result.unwrap(),
            }
        }
    }

    fn claim_json(library: &Library, claim: &JobClaim) -> String {
        library
            .lock()
            .unwrap()
            .query_row(
                "SELECT target_json FROM background_jobs WHERE id=?1",
                [claim.id()],
                |row| row.get(0),
            )
            .unwrap()
    }

    fn counts(library: &Library) -> (u64, u64, u64) {
        library.lock().unwrap().query_row("SELECT (SELECT COUNT(*) FROM background_semantic_builds),
            (SELECT COUNT(*) FROM background_semantic_units),(SELECT COUNT(*) FROM background_semantic_active)",[],
            |row| Ok((crate::jobs::row_bytes(row,0)?,crate::jobs::row_bytes(row,1)?,crate::jobs::row_bytes(row,2)?))).unwrap()
    }

    #[test]
    fn queued_vectors_match_foreground_bytes_and_survive_forgetting_the_job() {
        let (_temporary, library, _root) = fixture();
        let baseline = library.semantic_rebuild().unwrap();
        let before = library.export_portable().unwrap().digest;
        let expected = library.semantic_search("evidence a", 10).unwrap();
        let job = library
            .enqueue_semantic_rebuild("vectors", JobPriority::Normal)
            .unwrap();
        for next in 1..=2 {
            let quantum = worker(&library).run_next().unwrap().unwrap();
            assert_eq!(quantum.state, JobState::Queued);
            assert_eq!(quantum.attempts, 0);
            assert_eq!(quantum.semantic_progress.unwrap().next_unit, next);
            assert!(library.semantic_status().unwrap().healthy);
            assert_eq!(library.semantic_search("evidence a", 10).unwrap(), expected);
        }
        let completed = worker(&library).run_next().unwrap().unwrap();
        assert_eq!(
            completed.result,
            Some(serde_json::to_value(baseline).unwrap())
        );
        assert!(completed.semantic_progress.is_none());
        assert_eq!(library.export_portable().unwrap().digest, before);
        let equal: bool=library.lock().unwrap().query_row("SELECT NOT EXISTS(SELECT 1 FROM background_semantic_units u
            JOIN background_semantic_active active ON active.build_id=u.build_id JOIN semantic_embeddings e ON e.passage_id=u.passage_id
            WHERE e.vector_blob<>u.vector_blob OR e.passage_hash<>u.passage_hash)",[],|row| row.get(0)).unwrap();
        assert!(equal);
        assert_eq!(
            library
                .enqueue_semantic_rebuild("vectors", JobPriority::Normal)
                .unwrap(),
            completed
        );
        library.forget_background_job(&job.id).unwrap();
        assert_eq!(counts(&library), (1, 2, 1));
        assert_eq!(library.semantic_search("evidence a", 10).unwrap(), expected);
        assert!(!library
            .export_portable()
            .unwrap()
            .tables
            .keys()
            .any(|name| name.starts_with("background_")));
    }

    #[test]
    fn compute_commit_fences_prevent_cancel_drop_scope_ocr_and_foreground_aba() {
        for action in [
            "cancel",
            "drop",
            "revoke-reselect",
            "ocr-reassert",
            "artifact-purge",
            "foreground",
            "legacy-meta",
            "restore",
        ] {
            let (temporary, library, root) = fixture();
            let controller = Library::open(temporary.path().join("library.sqlite3")).unwrap();
            let archive = library.export_portable().unwrap();
            let job = library
                .enqueue_semantic_rebuild("fence", JobPriority::Normal)
                .unwrap();
            let mut owner = worker(&library);
            let claim = owner.claim_for_test().unwrap().unwrap();
            let json = claim_json(&library, &claim);
            let outcome = library.job_semantic_rebuild_with_hook(&claim, &json, || match action {
                "cancel" => {
                    controller.cancel_background_job(&job.id).unwrap();
                }
                "drop" => {
                    controller.semantic_drop().unwrap();
                }
                "revoke-reselect" => {
                    controller
                        .revoke_source_root(root.to_str().unwrap())
                        .unwrap();
                    controller.index_path(&root).unwrap();
                }
                "ocr-reassert" => {
                    controller
                        .set_ocr_enabled(controller.ocr_status().unwrap().enabled)
                        .unwrap();
                }
                "artifact-purge" => {
                    let hit = controller
                        .search(&SearchRequest {
                            text: "evidence a".into(),
                            limit: 5,
                        })
                        .unwrap()
                        .remove(0);
                    controller.purge_artifact(&hit.artifact_id).unwrap();
                }
                "foreground" => {
                    controller.semantic_rebuild().unwrap();
                }
                "legacy-meta" => {
                    controller
                        .lock()
                        .unwrap()
                        .execute(
                            "UPDATE semantic_index_meta SET source_digest=source_digest",
                            [],
                        )
                        .unwrap();
                }
                "restore" => {
                    controller
                        .lock()
                        .unwrap()
                        .execute_batch("DELETE FROM artifacts;DELETE FROM source_roots;")
                        .unwrap();
                    controller.import_portable(&archive).unwrap();
                }
                _ => unreachable!(),
            });
            assert!(outcome.is_err(), "{action}: {outcome:?}");
            if action == "cancel" {
                // Live cancellation is acknowledged on owned recovery, not by pretending the
                // running transaction has already stopped.
                assert_eq!(
                    library.background_job(&job.id).unwrap().state,
                    JobState::Running
                );
                drop(owner);
                drop(worker(&library));
                assert_eq!(
                    library.background_job(&job.id).unwrap().state,
                    JobState::Cancelled
                );
            }
            assert_eq!(
                counts(&library),
                (0, 0, 0),
                "{action}: retained staged bytes"
            );
            if action == "drop" {
                assert!(library.semantic_status().unwrap().manifest.is_none());
            }
        }
    }

    #[test]
    fn final_pointer_and_completion_roll_back_together_on_write_failure() {
        let (_temporary, library, _root) = fixture();
        let old = library.semantic_search("evidence", 10).unwrap();
        let job = library
            .enqueue_semantic_rebuild("publish-fail", JobPriority::Normal)
            .unwrap();
        let mut owner = worker(&library);
        owner.run_next().unwrap();
        owner.run_next().unwrap();
        library.lock().unwrap().execute_batch("CREATE TRIGGER refuse_semantic_completion BEFORE UPDATE OF state ON background_jobs
            WHEN NEW.operation='semantic_rebuild' AND NEW.state='completed' BEGIN SELECT RAISE(ABORT,'injected publish failure'); END;").unwrap();
        let failed = owner.run_next().unwrap().unwrap();
        assert_eq!(failed.id, job.id);
        assert_eq!(failed.state, JobState::Failed);
        assert_eq!(counts(&library), (0, 0, 0));
        assert!(library.semantic_status().unwrap().healthy);
        assert_eq!(library.semantic_search("evidence", 10).unwrap(), old);
    }

    #[test]
    fn malformed_units_targets_vectors_and_cursors_fail_without_publication() {
        for damage in ["unit", "target", "cursor", "vector", "nan", "missing"] {
            let (_temporary, library, _root) = fixture();
            let before = library.export_portable().unwrap().digest;
            library
                .enqueue_semantic_rebuild("damage", JobPriority::Normal)
                .unwrap();
            let mut owner = worker(&library);
            owner.run_next().unwrap();
            owner.run_next().unwrap();
            let connection = library.lock().unwrap();
            match damage {
                "unit" => {
                    connection
                        .execute(
                            "UPDATE background_semantic_units SET unit_hash=?1 WHERE ordinal=0",
                            [digest(b"wrong")],
                        )
                        .unwrap();
                }
                "target" => {
                    connection.execute("UPDATE background_jobs SET target_json=json_set(target_json,'$.config.extra',true)",[]).unwrap();
                }
                "cursor" => {
                    connection
                        .execute("UPDATE background_semantic_builds SET vector_bytes=0", [])
                        .unwrap();
                }
                "vector" => {
                    connection.execute("UPDATE background_semantic_units SET vector_blob=zeroblob(512) WHERE ordinal=0",[]).unwrap();
                }
                "nan" => {
                    let vector = encode_vector(&vec![f32::NAN; 128]);
                    connection.execute("UPDATE background_semantic_units SET vector_blob=?1,vector_hash=?2 WHERE ordinal=0",params![vector,digest(&vector)]).unwrap();
                }
                "missing" => {
                    connection
                        .execute("DELETE FROM background_semantic_units WHERE ordinal=0", [])
                        .unwrap();
                }
                _ => unreachable!(),
            }
            drop(connection);
            let failed = owner.run_next().unwrap().unwrap();
            assert_eq!(failed.state, JobState::Failed, "{damage}: {failed:?}");
            assert_eq!(counts(&library), (0, 0, 0));
            assert!(library.semantic_status().unwrap().healthy);
            assert_eq!(library.export_portable().unwrap().digest, before);
        }
    }

    #[test]
    fn bounded_input_and_conflicting_admission_do_not_evict_work() {
        let (_temporary, library, root) = fixture();
        let admitted = library
            .enqueue_semantic_rebuild("same", JobPriority::Normal)
            .unwrap();
        assert!(library
            .enqueue_semantic_rebuild("same", JobPriority::High)
            .is_err());
        fs::write(root.join("c.md"), "New source changes membership").unwrap();
        library.index_path(&root).unwrap();
        assert!(library
            .enqueue_semantic_rebuild("same", JobPriority::Normal)
            .is_err());
        assert_eq!(library.background_job(&admitted.id).unwrap(), admitted);
        // A canonical text cell has no SQL size limit. Reject its metadata before reading it.
        let text = "x".repeat(MAX_TEXT_BYTES as usize + 1);
        library.lock().unwrap().execute("UPDATE passages SET text=?1,text_hash=?2 WHERE id=(SELECT id FROM passages ORDER BY id LIMIT 1)",params![text,digest(text.as_bytes())]).unwrap();
        assert!(library
            .enqueue_semantic_rebuild("oversized", JobPriority::Normal)
            .unwrap_err()
            .to_string()
            .contains("bounds"));
        assert!(
            library.background_jobs(128).unwrap().is_empty(),
            "passage mutation must invalidate previous staged work"
        );
    }

    #[test]
    fn empty_drop_cancels_work_and_purge_and_restore_leave_no_semantic_payload() {
        let (_temporary, library, root) = fixture();
        library.semantic_drop().unwrap();
        let job = library
            .enqueue_semantic_rebuild("empty-drop", JobPriority::Normal)
            .unwrap();
        library.semantic_drop().unwrap();
        assert!(library.background_job(&job.id).is_err());
        assert_eq!(counts(&library), (0, 0, 0));
        library
            .enqueue_semantic_rebuild("before-purge", JobPriority::Normal)
            .unwrap();
        worker(&library).run_next().unwrap();
        let archive = library.export_portable().unwrap();
        library.purge_root(root.to_str().unwrap()).unwrap();
        assert_eq!(counts(&library), (0, 0, 0));
        library.import_portable(&archive).unwrap();
        assert_eq!(counts(&library), (0, 0, 0));
        assert!(library.semantic_status().unwrap().manifest.is_none());
        assert_eq!(library.stats().unwrap().artifacts, 2);
    }

    #[test]
    fn active_corruption_is_not_silently_replaced_by_the_legacy_index() {
        let (_temporary, library, _root) = fixture();
        library
            .enqueue_semantic_rebuild("active", JobPriority::Normal)
            .unwrap();
        let mut owner = worker(&library);
        for _ in 0..3 {
            owner.run_next().unwrap();
        }
        let before = library.export_portable().unwrap().digest;
        library
            .lock()
            .unwrap()
            .execute(
                "UPDATE background_semantic_units SET vector_blob=zeroblob(512) WHERE ordinal=0",
                [],
            )
            .unwrap();
        assert!(!library.semantic_status().unwrap().healthy);
        assert!(library.semantic_search("evidence", 10).is_err());
        assert_eq!(
            library
                .search(&SearchRequest {
                    text: "evidence".into(),
                    limit: 10
                })
                .unwrap()
                .len(),
            2
        );
        assert_eq!(library.export_portable().unwrap().digest, before);
    }

    #[test]
    fn semantic_drop_allows_bad_scheduling_policy_but_refuses_unknown_layout() {
        let (_temporary, library, _root) = fixture();
        library
            .enqueue_semantic_rebuild("pending", JobPriority::Normal)
            .unwrap();
        library
            .lock()
            .unwrap()
            .execute(
                "UPDATE background_job_runtime SET policy_json='invalid'",
                [],
            )
            .unwrap();
        library.semantic_drop().unwrap();
        assert_eq!(counts(&library), (0, 0, 0));
        library
            .lock()
            .unwrap()
            .execute(
                "UPDATE background_job_runtime SET policy_json=?1",
                [serde_json::to_string(&crate::JobQueuePolicy::default()).unwrap()],
            )
            .unwrap();
        library
            .enqueue_semantic_rebuild("preserved", JobPriority::Normal)
            .unwrap();
        let before = library.export_portable().unwrap().digest;
        library
            .lock()
            .unwrap()
            .execute_batch("CREATE TABLE BACKGROUND_UNKNOWN(private_value TEXT);")
            .unwrap();
        assert!(library.semantic_drop().is_err());
        assert_eq!(counts(&library), (1, 2, 0));
        assert_eq!(library.export_portable().unwrap().digest, before);
    }

    fn insert_test_passages(
        connection: &Connection,
        version: &str,
        first: usize,
        count: usize,
        text: &str,
    ) {
        let hash = digest(text.as_bytes());
        let mut insert=connection.prepare("INSERT INTO passages(id,artifact_version_id,ordinal,text,text_hash,
            locator_json,char_start,char_end,line_start,line_end,created_at) VALUES (?1,?2,?3,?4,?5,'{}',0,?6,1,1,'synthetic-budget-fixture')").unwrap();
        for ordinal in first..first + count {
            insert
                .execute(params![
                    Uuid::new_v4().to_string(),
                    version,
                    ordinal as u32,
                    text,
                    hash,
                    text.len() as u32
                ])
                .unwrap();
        }
    }

    #[test]
    fn legacy_semantic_index_cannot_resurrect_across_consent_or_policy_aba() {
        for action in [
            "revoke-reselect",
            "ocr-reassert",
            "incarnation",
            "empty-purge",
        ] {
            let (temporary, library, root) = fixture();
            drop(worker(&library));
            let controller = Library::open(temporary.path().join("library.sqlite3")).unwrap();
            assert!(library.semantic_status().unwrap().healthy);
            match action {
                "revoke-reselect" => {
                    controller
                        .revoke_source_root(root.to_str().unwrap())
                        .unwrap();
                    controller.index_path(&root).unwrap();
                }
                "ocr-reassert" => {
                    controller
                        .set_ocr_enabled(controller.ocr_status().unwrap().enabled)
                        .unwrap();
                }
                "incarnation" => {
                    controller
                        .lock()
                        .unwrap()
                        .execute(
                            "UPDATE schema_meta SET value=?1 WHERE key='authorization_incarnation'",
                            [Uuid::new_v4().to_string()],
                        )
                        .unwrap();
                }
                "empty-purge" => {
                    controller
                        .purge_root(temporary.path().join("absent").to_str().unwrap())
                        .unwrap();
                }
                _ => unreachable!(),
            }
            assert!(!library.semantic_status().unwrap().healthy, "{action}");
            assert!(library.hybrid_search("evidence", 10).is_err(), "{action}");
            assert_eq!(
                library
                    .search(&SearchRequest {
                        text: "evidence".into(),
                        limit: 10
                    })
                    .unwrap()
                    .len(),
                2
            );
        }
    }

    #[test]
    fn legacy_semantic_status_rejects_nonfinite_vectors() {
        let (_temporary, library, _root) = fixture();
        let bytes: Vec<u8> = (0..128).flat_map(|_| f32::NAN.to_le_bytes()).collect();
        library
            .lock()
            .unwrap()
            .execute("UPDATE semantic_embeddings SET vector_blob=?1", [bytes])
            .unwrap();
        assert!(!library.semantic_status().unwrap().healthy);
        assert!(library.semantic_search("evidence", 10).is_err());
    }

    #[test]
    fn published_generation_rejects_extra_null_units_and_job_owned_pointer() {
        for action in ["extra-unit", "job-owned-pointer"] {
            let (_temporary, library, _root) = fixture();
            library
                .enqueue_semantic_rebuild("published", JobPriority::Normal)
                .unwrap();
            let mut owner = worker(&library);
            for _ in 0..3 {
                owner.run_next().unwrap();
            }
            assert!(library.semantic_status().unwrap().healthy);
            if action == "extra-unit" {
                let connection = library.lock().unwrap();
                let build: String = connection
                    .query_row(
                        "SELECT build_id FROM background_semantic_active",
                        [],
                        |row| row.get(0),
                    )
                    .unwrap();
                let (mut unit, _, _) = load_unit(&connection, &build, 0).unwrap();
                unit.passage_id = Uuid::new_v4().to_string();
                connection.execute("INSERT INTO background_semantic_units(build_id,ordinal,passage_id,passage_hash,artifact_id,version_id,root_id,scope_generation,text_bytes,unit_hash) VALUES (?1,2,?2,?3,?4,?5,?6,?7,?8,?9)",
                    params![build,unit.passage_id,unit.passage_hash,unit.artifact_id,unit.version_id,unit.root_id,unit.scope_generation,unit.text_bytes as i64,unit.hash().unwrap()]).unwrap();
            } else {
                let pending = library
                    .enqueue_semantic_rebuild("pending", JobPriority::Normal)
                    .unwrap();
                library.lock().unwrap().execute("UPDATE background_semantic_active SET build_id=(SELECT id FROM background_semantic_builds WHERE job_id=?1)", [&pending.id]).unwrap();
            }
            assert!(
                !library.semantic_status().is_ok_and(|status| status.healthy),
                "{action}"
            );
            assert!(library.semantic_search("evidence", 10).is_err(), "{action}");
        }
    }

    #[test]
    fn hybrid_branches_share_one_canonical_read_snapshot() {
        let (temporary, library, root) = fixture();
        let controller = Library::open(temporary.path().join("library.sqlite3")).unwrap();
        let before = library.hybrid_search("evidence", 10).unwrap();
        let hits = library
            .hybrid_search_with_hook("evidence", 10, || {
                controller
                    .revoke_source_root(root.to_str().unwrap())
                    .unwrap();
                fs::write(root.join("a.md"), "Replacement evidence a.\n").unwrap();
                controller.index_path(&root).unwrap();
                controller.semantic_rebuild().unwrap();
            })
            .unwrap();
        assert_eq!(hits, before);
        let next = library.hybrid_search("evidence", 10).unwrap();
        assert_ne!(next, before);
        assert_eq!(next.len(), 2);
    }

    #[test]
    fn foreground_rebuild_refuses_oversized_text_before_allocating_or_clearing() {
        let (_temporary, library, _root) = fixture();
        let text = "x".repeat(MAX_TEXT_BYTES as usize + 1);
        library
            .lock()
            .unwrap()
            .execute(
                "UPDATE passages SET text=?1,text_hash=?2 WHERE ordinal=0",
                params![text, digest(text.as_bytes())],
            )
            .unwrap();
        assert!(library.semantic_rebuild().is_err());
    }

    #[test]
    fn foreground_rebuild_refuses_a_false_canonical_passage_hash() {
        let (_temporary, library, _root) = fixture();
        library
            .lock()
            .unwrap()
            .execute(
                "UPDATE passages SET text_hash=?1 WHERE ordinal=0",
                [digest(b"not the passage text")],
            )
            .unwrap();
        let before = library.export_portable().unwrap().digest;
        assert!(library.semantic_rebuild().is_err());
        assert!(!library.semantic_status().unwrap().healthy);
        assert_eq!(library.export_portable().unwrap().digest, before);
    }

    #[test]
    fn legacy_status_rehashes_canonical_text_instead_of_trusting_consistent_false_hashes() {
        let (_temporary, library, _root) = fixture();
        let hash = digest(b"not the passage text");
        let connection = library.lock().unwrap();
        connection
            .execute_batch(
                "CREATE TEMP TABLE saved_vectors AS SELECT * FROM semantic_embeddings;
            CREATE TEMP TABLE saved_meta AS SELECT * FROM semantic_index_meta;",
            )
            .unwrap();
        connection
            .execute("UPDATE passages SET text_hash=?1", [&hash])
            .unwrap();
        connection
            .execute_batch(
                "INSERT INTO semantic_embeddings SELECT * FROM saved_vectors;
            INSERT INTO semantic_index_meta SELECT * FROM saved_meta;",
            )
            .unwrap();
        connection
            .execute("UPDATE semantic_embeddings SET passage_hash=?1", [hash])
            .unwrap();
        let (_, claimed_digest) = canonical_semantic_source(&connection).unwrap();
        connection
            .execute(
                "UPDATE semantic_index_meta SET source_digest=?1",
                [claimed_digest],
            )
            .unwrap();
        drop(connection);
        assert!(!library.semantic_status().unwrap().healthy);
        assert!(library.semantic_search("evidence", 10).is_err());
    }

    #[test]
    fn storage_inspection_accounts_for_pending_metadata_staged_and_published_vectors() {
        let (_temporary, library, _root) = fixture();
        let derived = || {
            library
                .inspect_storage()
                .unwrap()
                .entries
                .into_iter()
                .filter(|entry| entry.category == "derived_records")
                .map(|entry| entry.bytes)
                .sum::<u64>()
        };
        let before = derived();
        library
            .enqueue_semantic_rebuild("storage", JobPriority::Normal)
            .unwrap();
        let pending = derived();
        assert!(pending > before, "pending semantic metadata was omitted");
        let mut owner = worker(&library);
        owner.run_next().unwrap();
        assert_eq!(derived(), pending + 71 + 512);
        owner.run_next().unwrap();
        owner.run_next().unwrap();
        assert!(derived() >= before + 1024);
    }

    #[test]
    fn corpus_passage_and_combined_input_caps_accept_the_boundary_and_refuse_overflow() {
        for (kind, units, text) in [
            ("count", MAX_UNITS, "x".to_owned()),
            ("bytes", 1024, "x".repeat(MAX_TEXT_BYTES as usize)),
        ] {
            let (_temporary, library, _root) = fixture();
            let mut connection = library.lock().unwrap();
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .unwrap();
            let version: String = transaction
                .query_row(
                    "SELECT active_version_id FROM artifacts ORDER BY id LIMIT 1",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            transaction.execute("DELETE FROM passages", []).unwrap();
            insert_test_passages(&transaction, &version, 0, units, &text);
            let (target, captured) = capture(&transaction).unwrap();
            assert_eq!(target.total_units as usize, units);
            if kind == "bytes" {
                assert_eq!(target.source_bytes, MAX_SOURCE_BYTES);
            }
            drop(captured);
            insert_test_passages(&transaction, &version, units, 1, &text);
            assert!(capture(&transaction)
                .unwrap_err()
                .to_string()
                .contains(if kind == "count" { "20000" } else { "64 MiB" }));
            // Roll back the intentionally synthetic raw rows, never modify source files.
        }
    }

    #[test]
    fn retained_unit_cap_is_transactional_and_does_not_evict_pending_work() {
        let (_temporary, library, _root) = fixture();
        {
            let mut connection = library.lock().unwrap();
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .unwrap();
            let version: String = transaction
                .query_row(
                    "SELECT active_version_id FROM artifacts ORDER BY id LIMIT 1",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            transaction.execute("DELETE FROM passages", []).unwrap();
            insert_test_passages(&transaction, &version, 0, MAX_UNITS, "x");
            transaction.commit().unwrap();
        }
        let mut ids = Vec::new();
        for key in ["one", "two", "three"] {
            ids.push(
                library
                    .enqueue_semantic_rebuild(key, JobPriority::Normal)
                    .unwrap()
                    .id,
            );
        }
        assert_eq!(counts(&library), (3, 60000, 0));
        assert!(library
            .enqueue_semantic_rebuild("four", JobPriority::Normal)
            .unwrap_err()
            .to_string()
            .contains("retained-unit capacity"));
        assert_eq!(counts(&library), (3, 60000, 0));
        for id in &ids {
            assert_eq!(library.background_job(id).unwrap().state, JobState::Queued);
        }
        library.cancel_background_job(&ids[0]).unwrap();
        library
            .enqueue_semantic_rebuild("four", JobPriority::Normal)
            .unwrap();
        assert_eq!(counts(&library), (3, 60000, 0));
    }
}
