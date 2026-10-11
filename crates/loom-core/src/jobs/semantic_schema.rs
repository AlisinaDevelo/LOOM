//! Operational v6. Canonical schema and portable archives deliberately stay unchanged.
use super::*;

pub(super) const SEMANTIC_SCHEMA: &str = "CREATE TABLE IF NOT EXISTS background_semantic_builds(
    id TEXT PRIMARY KEY CHECK(length(id)=36),
    job_id TEXT UNIQUE REFERENCES background_jobs(id) ON DELETE CASCADE,
    target_json TEXT NOT NULL CHECK(length(CAST(target_json AS BLOB))<=16384 AND json_valid(target_json)),
    total_units INTEGER NOT NULL CHECK(total_units BETWEEN 0 AND 20000),
    next_unit INTEGER NOT NULL DEFAULT 0 CHECK(next_unit BETWEEN 0 AND total_units),
    source_bytes INTEGER NOT NULL CHECK(source_bytes BETWEEN 0 AND 67108864),
    vector_bytes INTEGER NOT NULL DEFAULT 0 CHECK(vector_bytes BETWEEN 0 AND 10240000)
) STRICT;
CREATE TABLE IF NOT EXISTS background_semantic_units(
    build_id TEXT NOT NULL REFERENCES background_semantic_builds(id) ON DELETE CASCADE,
    ordinal INTEGER NOT NULL CHECK(ordinal BETWEEN 0 AND 19999),
    passage_id TEXT NOT NULL CHECK(length(passage_id)=36),
    passage_hash TEXT NOT NULL CHECK(length(passage_hash)=71 AND substr(passage_hash,1,7)='blake3:'),
    artifact_id TEXT NOT NULL CHECK(length(artifact_id)=36),
    version_id TEXT NOT NULL CHECK(length(version_id)=36),
    root_id TEXT NOT NULL CHECK(length(root_id)=36),
    scope_generation INTEGER NOT NULL CHECK(scope_generation>=0),
    text_bytes INTEGER NOT NULL CHECK(text_bytes BETWEEN 0 AND 65536),
    unit_hash TEXT NOT NULL CHECK(length(unit_hash)=71 AND substr(unit_hash,1,7)='blake3:'),
    vector_blob BLOB CHECK(length(vector_blob)=512),
    vector_hash TEXT CHECK(length(vector_hash)=71 AND substr(vector_hash,1,7)='blake3:'),
    CHECK((vector_blob IS NULL AND vector_hash IS NULL) OR (vector_blob IS NOT NULL AND vector_hash IS NOT NULL)),
    PRIMARY KEY(build_id,ordinal),
    UNIQUE(build_id,passage_id)
) STRICT;
CREATE TABLE IF NOT EXISTS background_semantic_active(
    slot INTEGER PRIMARY KEY CHECK(slot=1),
    build_id TEXT NOT NULL UNIQUE REFERENCES background_semantic_builds(id) ON DELETE CASCADE
) STRICT;";

pub(super) fn schema_v6() -> String {
    RUNTIME_SCHEMA_V5.replace(
        "'index_file','index_directory'",
        "'index_file','index_directory','semantic_rebuild'",
    ) + SEMANTIC_SCHEMA
}

const CLEAR: &str = "DELETE FROM background_semantic_active;
    DELETE FROM background_jobs WHERE operation='semantic_rebuild';
    DELETE FROM background_semantic_builds;";

pub(super) fn fences() -> Vec<(&'static str, String)> {
    [
        ("background_semantic_passage_deleted", "AFTER DELETE ON passages", ""),
        ("background_semantic_passage_changed", "AFTER UPDATE ON passages", "WHEN NEW.text<>OLD.text OR NEW.text_hash<>OLD.text_hash OR NEW.artifact_version_id<>OLD.artifact_version_id"),
        ("background_semantic_source_changed", "AFTER UPDATE ON source_roots", "WHEN NEW.enabled<>OLD.enabled OR NEW.scope_generation<>OLD.scope_generation"),
        ("background_semantic_source_deleted", "AFTER DELETE ON source_roots", ""),
        ("background_semantic_artifact_changed", "AFTER UPDATE ON artifacts", "WHEN NEW.source_root_id<>OLD.source_root_id OR NEW.state<>OLD.state"),
        ("background_semantic_policy_changed", "AFTER UPDATE ON schema_meta", "WHEN OLD.key IN ('authorization_incarnation','ocr_enabled','ocr_policy_revision','source_selection_purge_revision') AND NEW.value<>OLD.value"),
        ("background_semantic_policy_inserted", "AFTER INSERT ON schema_meta", "WHEN NEW.key IN ('authorization_incarnation','ocr_enabled','ocr_policy_revision','source_selection_purge_revision')"),
        ("background_semantic_policy_deleted", "AFTER DELETE ON schema_meta", "WHEN OLD.key IN ('authorization_incarnation','ocr_enabled','ocr_policy_revision','source_selection_purge_revision')"),
        ("background_semantic_legacy_inserted", "AFTER INSERT ON semantic_index_meta", ""),
        ("background_semantic_legacy_changed", "AFTER UPDATE ON semantic_index_meta", ""),
        ("background_semantic_legacy_deleted", "AFTER DELETE ON semantic_index_meta", ""),
    ].into_iter().map(|(name,event,condition)| {
        // Sensitive source/capability changes invalidate both formats. Legacy-manifest triggers
        // clear only v6 state, so a foreground rebuild can publish its new manifest and recursive
        // triggers cannot recurse through a DELETE of their own table.
        let legacy = if name.starts_with("background_semantic_legacy_") {
            ""
        } else {
            "DELETE FROM semantic_embeddings; DELETE FROM semantic_index_meta;"
        };
        (name, format!("CREATE TRIGGER IF NOT EXISTS {name} {event} {condition} BEGIN {CLEAR} {legacy} END"))
    }).collect()
}

pub(super) fn create(connection: &Connection) -> Result<()> {
    connection.execute_batch(&schema_v6())?;
    for fence in [
        DIRECTORY_DELETE_FENCE,
        DIRECTORY_UPDATE_FENCE,
        DIRECTORY_ARTIFACT_FENCE,
    ] {
        connection.execute_batch(fence)?;
    }
    for (_, fence) in fences() {
        connection.execute_batch(&fence)?;
    }
    Ok(())
}

pub(super) fn validate(connection: &Connection) -> Result<()> {
    let schema = schema_v6();
    validate_definitions(connection, &schema)?;
    let mut names: Vec<String> = schema
        .split(';')
        .filter(|sql| !sql.trim().is_empty())
        .map(|sql| {
            sql.split_whitespace()
                .nth(5)
                .unwrap()
                .split('(')
                .next()
                .unwrap()
                .to_owned()
        })
        .collect();
    for (name, definition) in [
        (
            "background_directory_locator_deleted",
            DIRECTORY_DELETE_FENCE,
        ),
        (
            "background_directory_locator_changed",
            DIRECTORY_UPDATE_FENCE,
        ),
        (
            "background_directory_artifact_changed",
            DIRECTORY_ARTIFACT_FENCE,
        ),
    ] {
        validate_definition(connection, name, definition)?;
        names.push(name.to_owned());
    }
    for (name, definition) in fences() {
        validate_definition(connection, name, &definition)?;
        names.push(name.to_owned());
    }
    let mut statement = connection
        .prepare("SELECT name FROM sqlite_master WHERE lower(name) GLOB 'background_*'")?;
    for name in statement.query_map([], |row| row.get::<_, String>(0))? {
        if !names.contains(&name?) {
            return Err(LoomError::JobQueue(
                "unknown private runtime object; operation refused".into(),
            ));
        }
    }
    Ok(())
}

/// v5 is a released layout. Copy its complete queue and directory payload under the owned
/// writer transaction, reapplying checks. Rename children first and drop old indexes before
/// creating the new parent; otherwise SQLite would leave their FKs attached to the old table.
pub(super) fn migrate_v5(connection: &Connection) -> Result<()> {
    validate_runtime_v5(connection)?;
    let policy = load_policy(connection)?;
    let records: u32 =
        connection.query_row("SELECT COUNT(*) FROM background_jobs", [], |row| row.get(0))?;
    if records > policy.max_records {
        return Err(LoomError::JobQueue(
            "runtime upgrade exceeds its retained-record budget".into(),
        ));
    }
    crate::store::validate_directory_targets_for_purge(connection)?;
    // Released v5 has no capability fences. Its vectors cannot prove which OCR/consent/purge
    // incarnation built them, including any ABA before upgrade. Rebuild, never adopt them.
    connection
        .execute_batch("DELETE FROM semantic_embeddings; DELETE FROM semantic_index_meta;")?;
    connection.execute_batch("DROP TRIGGER background_directory_locator_deleted;
        DROP TRIGGER background_directory_locator_changed;
        DROP TRIGGER background_directory_artifact_changed;
        ALTER TABLE background_directory_units RENAME TO background_directory_units_previous;
        ALTER TABLE background_directory_manifests RENAME TO background_directory_manifests_previous;
        ALTER TABLE background_jobs RENAME TO background_jobs_previous;
        DROP INDEX background_jobs_ready;
        DROP INDEX background_directory_unit_locator;
        DROP INDEX background_directory_unit_artifact;")?;
    create(connection)?;
    connection.execute_batch("INSERT INTO background_jobs SELECT * FROM background_jobs_previous;
        INSERT INTO background_directory_manifests SELECT * FROM background_directory_manifests_previous;
        INSERT INTO background_directory_units SELECT * FROM background_directory_units_previous;
        DROP TABLE background_directory_units_previous;
        DROP TABLE background_directory_manifests_previous;
        DROP TABLE background_jobs_previous;")?;
    Ok(())
}

pub(crate) fn clear(connection: &Connection) -> Result<()> {
    connection.execute_batch(CLEAR)?;
    Ok(())
}
