//! Portable library export and import (roadmap `0305`).
//!
//! An export is a self-describing JSON document holding every canonical row plus user settings.
//! Derived state (the FTS5 projection, semantic vectors, index checkpoints) is never exported; it is
//! rebuilt from canonical rows on import. The format and its compatibility policy are documented in
//! `docs/PORTABILITY.md`.

use std::collections::BTreeMap;

use chrono::{SecondsFormat, Utc};
use rusqlite::{types::ValueRef, Connection};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    error::{LoomError, Result},
    store::Library,
};

pub const EXPORT_FORMAT: &str = "loom.portable-export";
pub const EXPORT_FORMAT_VERSION: u32 = 1;
/// Library schema versions whose exports this build can import.
pub const IMPORTABLE_SCHEMA_VERSIONS: &[i64] = &[6, 7, 8];

/// Canonical tables in foreign-key order, with the column order used for deterministic output.
const TABLES: &[(&str, &str)] = &[
    ("source_roots", "id"),
    ("artifacts", "id"),
    ("artifact_locators", "id"),
    ("artifact_versions", "id"),
    ("passages", "id"),
    ("relationships", "id"),
    ("bookmark_imports", "id"),
    ("bookmark_records", "id"),
    ("bookmark_import_items", "import_id, bookmark_id, ordinal"),
    ("bookmark_import_failures", "import_id, ordinal"),
];

/// User settings carried by an export. Everything else in `schema_meta` is library bookkeeping.
const SETTINGS: &[&str] = &["ocr_enabled", "retention_days"];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortableTable {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortableExport {
    pub format: String,
    pub format_version: u32,
    pub library_schema_version: i64,
    pub exported_at: String,
    pub settings: BTreeMap<String, String>,
    pub tables: BTreeMap<String, PortableTable>,
    /// `blake3:` digest over the schema version, settings, and tables. It detects accidental
    /// corruption of a plaintext export; encrypted backups add authenticated integrity.
    pub digest: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortableImportReport {
    pub source_schema_version: i64,
    pub rows: BTreeMap<String, u64>,
    pub settings_applied: Vec<String>,
    pub fts_healthy: bool,
}

#[derive(Serialize)]
struct DigestInput<'a> {
    library_schema_version: i64,
    settings: &'a BTreeMap<String, String>,
    tables: &'a BTreeMap<String, PortableTable>,
}

fn digest(
    library_schema_version: i64,
    settings: &BTreeMap<String, String>,
    tables: &BTreeMap<String, PortableTable>,
) -> Result<String> {
    let bytes = serde_json::to_vec(&DigestInput {
        library_schema_version,
        settings,
        tables,
    })?;
    Ok(format!("blake3:{}", blake3::hash(&bytes).to_hex()))
}

struct ColumnInfo {
    name: String,
    required: bool,
}

fn table_columns(connection: &Connection, table: &str) -> Result<Vec<ColumnInfo>> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info(\"{table}\")"))?;
    let columns = statement
        .query_map([], |row| {
            let not_null: bool = row.get(3)?;
            let default: Option<String> = row.get(4)?;
            let primary_key: i64 = row.get(5)?;
            Ok(ColumnInfo {
                name: row.get(1)?,
                required: (not_null && default.is_none()) || primary_key > 0,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(columns)
}

fn to_json(value: ValueRef<'_>, table: &str) -> Result<Value> {
    Ok(match value {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(integer) => Value::from(integer),
        ValueRef::Real(real) => serde_json::Number::from_f64(real)
            .map(Value::Number)
            .ok_or_else(|| LoomError::PortableExport(format!("non-finite number in {table}")))?,
        ValueRef::Text(text) => Value::String(
            std::str::from_utf8(text)
                .map_err(|_| LoomError::PortableExport(format!("non-UTF-8 text in {table}")))?
                .to_owned(),
        ),
        ValueRef::Blob(_) => {
            return Err(LoomError::PortableExport(format!(
                "binary values are not portable ({table})"
            )))
        }
    })
}

fn to_sql(value: &Value, table: &str) -> Result<rusqlite::types::Value> {
    use rusqlite::types::Value as Sql;
    Ok(match value {
        Value::Null => Sql::Null,
        Value::Number(number) => {
            if let Some(integer) = number.as_i64() {
                Sql::Integer(integer)
            } else if let Some(real) = number.as_f64() {
                Sql::Real(real)
            } else {
                return Err(LoomError::PortableExport(format!(
                    "number out of range in {table}"
                )));
            }
        }
        Value::String(text) => Sql::Text(text.clone()),
        _ => {
            return Err(LoomError::PortableExport(format!(
                "unsupported value type in {table}"
            )))
        }
    })
}

fn table_exists(connection: &Connection, table: &str) -> Result<bool> {
    Ok(connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
        [table],
        |row| row.get(0),
    )?)
}

fn validate_setting(key: &str, value: &str) -> Result<()> {
    let valid = match key {
        "ocr_enabled" => matches!(value, "0" | "1"),
        "retention_days" => value
            .parse::<u32>()
            .is_ok_and(|days| (1..=36_500).contains(&days)),
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(LoomError::PortableExport(format!(
            "invalid setting {key}={value}"
        )))
    }
}

impl PortableExport {
    /// Recomputes the digest after a deliberate edit, such as migration tooling that rewrites an
    /// export for an older schema. Imports always verify the digest.
    pub fn seal(&mut self) -> Result<()> {
        self.digest = digest(self.library_schema_version, &self.settings, &self.tables)?;
        Ok(())
    }

    /// Total canonical rows across all tables.
    pub fn row_count(&self) -> u64 {
        self.tables
            .values()
            .map(|table| table.rows.len() as u64)
            .sum()
    }
}

impl Library {
    /// Exports every canonical row and user setting from one consistent read snapshot.
    pub fn export_portable(&self) -> Result<PortableExport> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        let library_schema_version: i64 = transaction
            .query_row(
                "SELECT value FROM schema_meta WHERE key = 'schema_version'",
                [],
                |row| row.get::<_, String>(0),
            )?
            .parse()
            .map_err(|_| LoomError::UnsupportedSchemaVersion("unparseable".into()))?;

        let mut settings = BTreeMap::new();
        for key in SETTINGS {
            let value: Option<String> = transaction
                .query_row(
                    "SELECT value FROM schema_meta WHERE key = ?1",
                    [key],
                    |row| row.get(0),
                )
                .ok();
            if let Some(value) = value {
                settings.insert((*key).to_owned(), value);
            }
        }

        let mut tables = BTreeMap::new();
        for (table, order) in TABLES {
            let columns = table_columns(&transaction, table)?
                .into_iter()
                .map(|column| column.name)
                .collect::<Vec<_>>();
            let column_list = columns
                .iter()
                .map(|column| format!("\"{column}\""))
                .collect::<Vec<_>>()
                .join(", ");
            let mut statement = transaction.prepare(&format!(
                "SELECT {column_list} FROM \"{table}\" ORDER BY {order}"
            ))?;
            let mut query = statement.query([])?;
            let mut rows = Vec::new();
            while let Some(row) = query.next()? {
                let mut values = Vec::with_capacity(columns.len());
                for index in 0..columns.len() {
                    values.push(to_json(row.get_ref(index)?, table)?);
                }
                rows.push(values);
            }
            tables.insert((*table).to_owned(), PortableTable { columns, rows });
        }
        drop(transaction);

        let digest = digest(library_schema_version, &settings, &tables)?;
        Ok(PortableExport {
            format: EXPORT_FORMAT.into(),
            format_version: EXPORT_FORMAT_VERSION,
            library_schema_version,
            exported_at: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
            settings,
            tables,
            digest,
        })
    }

    /// Imports a portable export into this library, which must hold no canonical rows.
    ///
    /// Every table and column is checked against the live schema, all rows go in one transaction
    /// with deferred foreign keys, and SQLite's foreign-key and integrity checks must pass before it
    /// commits; any failure rolls the import back and leaves the library empty. The FTS5 projection
    /// is filled by the passage triggers and checked after commit.
    pub fn import_portable(&self, export: &PortableExport) -> Result<PortableImportReport> {
        if export.format != EXPORT_FORMAT || export.format_version != EXPORT_FORMAT_VERSION {
            return Err(LoomError::PortableExport(format!(
                "unsupported format {} v{}",
                export.format, export.format_version
            )));
        }
        if !IMPORTABLE_SCHEMA_VERSIONS.contains(&export.library_schema_version) {
            return Err(LoomError::PortableExport(format!(
                "exports from library schema {} are not importable by this build",
                export.library_schema_version
            )));
        }
        if digest(
            export.library_schema_version,
            &export.settings,
            &export.tables,
        )? != export.digest
        {
            return Err(LoomError::PortableExport("digest mismatch".into()));
        }
        for (key, value) in &export.settings {
            validate_setting(key, value)?;
        }

        let mut connection = self.lock()?;
        for (table, _) in TABLES {
            let rows: i64 =
                connection.query_row(&format!("SELECT COUNT(*) FROM \"{table}\""), [], |row| {
                    row.get(0)
                })?;
            if rows > 0 {
                return Err(LoomError::PortableExport(
                    "imports go into an empty library only".into(),
                ));
            }
        }

        let mut report = PortableImportReport {
            source_schema_version: export.library_schema_version,
            ..PortableImportReport::default()
        };
        let transaction = connection.transaction()?;
        transaction.execute_batch("PRAGMA defer_foreign_keys = ON;")?;
        for name in export.tables.keys() {
            if !TABLES.iter().any(|(table, _)| table == name) {
                return Err(LoomError::PortableExport(format!("unknown table {name}")));
            }
        }
        for (table, _) in TABLES {
            let Some(data) = export.tables.get(*table) else {
                report.rows.insert((*table).to_owned(), 0);
                continue;
            };
            if !table_exists(&transaction, table)? {
                return Err(LoomError::PortableExport(format!("unknown table {table}")));
            }
            let live = table_columns(&transaction, table)?;
            for column in &data.columns {
                if !live.iter().any(|live| &live.name == column) {
                    return Err(LoomError::PortableExport(format!(
                        "unknown column {table}.{column}"
                    )));
                }
            }
            if data
                .columns
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != data.columns.len()
            {
                return Err(LoomError::PortableExport(format!(
                    "duplicate column in {table}"
                )));
            }
            for column in &live {
                if column.required && !data.columns.contains(&column.name) {
                    return Err(LoomError::PortableExport(format!(
                        "missing required column {table}.{}",
                        column.name
                    )));
                }
            }
            let column_list = data
                .columns
                .iter()
                .map(|column| format!("\"{column}\""))
                .collect::<Vec<_>>()
                .join(", ");
            let placeholders = vec!["?"; data.columns.len()].join(", ");
            let mut statement = transaction.prepare(&format!(
                "INSERT INTO \"{table}\" ({column_list}) VALUES ({placeholders})"
            ))?;
            for row in &data.rows {
                if row.len() != data.columns.len() {
                    return Err(LoomError::PortableExport(format!(
                        "row width mismatch in {table}"
                    )));
                }
                let values = row
                    .iter()
                    .map(|value| to_sql(value, table))
                    .collect::<Result<Vec<_>>>()?;
                statement.execute(rusqlite::params_from_iter(values))?;
            }
            report
                .rows
                .insert((*table).to_owned(), data.rows.len() as u64);
        }
        for (key, value) in &export.settings {
            transaction.execute(
                "INSERT INTO schema_meta(key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                [key, value],
            )?;
            report.settings_applied.push(key.clone());
        }
        let violations: i64 =
            transaction.query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
                row.get(0)
            })?;
        if violations > 0 {
            return Err(LoomError::PortableExport(format!(
                "{violations} foreign-key violations"
            )));
        }
        let integrity: String =
            transaction.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
        if integrity != "ok" {
            return Err(LoomError::PortableExport(format!(
                "integrity check failed: {integrity}"
            )));
        }
        transaction.commit()?;
        drop(connection);

        if let Some(value) = export.settings.get("ocr_enabled") {
            self.set_ocr_enabled_cache(value == "1");
        }
        let health = self.fts_health()?;
        report.fts_healthy = health.healthy;
        if !health.healthy {
            return Err(LoomError::PortableExport(
                "imported passages failed the FTS health check".into(),
            ));
        }
        Ok(report)
    }
}
