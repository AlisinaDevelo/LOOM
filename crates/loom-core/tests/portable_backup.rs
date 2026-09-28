use std::fs;

use loom_core::{
    BackupOptions, Library, PortableExport, RelationshipInput, RelationshipKind,
    RelationshipOrigin, SearchRequest,
};
use serde_json::{json, Value};
use tempfile::{tempdir, TempDir};

const CHROME_EXPORT: &str = include_str!("fixtures/bookmarks/chrome.html");
const PASSWORD: &[u8] = b"correct horse battery staple";
const FAST: BackupOptions = BackupOptions {
    kdf_memory_kib: 64,
    kdf_iterations: 1,
    kdf_parallelism: 1,
};

/// A library with text, Markdown, bookmarks, a typed relationship, and non-default settings.
fn populated() -> (TempDir, Library) {
    let directory = tempdir().unwrap();
    let notes = directory.path().join("notes");
    fs::create_dir(&notes).unwrap();
    fs::write(
        notes.join("alpha.md"),
        "# Alpha\n\nportable retry anomaly note",
    )
    .unwrap();
    fs::write(notes.join("beta.txt"), "beta evidence for the backup drill").unwrap();
    let bookmarks = directory.path().join("Bookmarks.html");
    fs::write(&bookmarks, CHROME_EXPORT).unwrap();

    let library = Library::open(directory.path().join("library.sqlite3")).unwrap();
    library.index_path(&notes).unwrap();
    library.import_bookmarks(&bookmarks).unwrap();
    library.set_retention_days(Some(90)).unwrap();
    library.set_ocr_enabled(false).unwrap();
    let hit = |text: &str| {
        library
            .search(&SearchRequest {
                text: text.into(),
                limit: 1,
            })
            .unwrap()
            .remove(0)
    };
    let alpha = hit("retry anomaly");
    let beta = hit("backup drill");
    library
        .add_relationship(&RelationshipInput {
            source_artifact_id: alpha.artifact_id,
            target_artifact_id: beta.artifact_id,
            kind: RelationshipKind::Related,
            origin: RelationshipOrigin::Inferred,
            evidence_passage_id: Some(alpha.passage_id),
            confidence: Some(0.625),
            method: "portable-test".into(),
            metadata: json!({"reason": "fixture"}),
        })
        .unwrap();
    (directory, library)
}

fn comparable(mut export: PortableExport) -> PortableExport {
    export.exported_at = String::new();
    export
}

#[test]
fn export_import_round_trip_preserves_every_canonical_row_and_setting() {
    let (_directory, library) = populated();
    let export = library.export_portable().unwrap();
    assert_eq!(export.library_schema_version, 8);
    assert_eq!(export.settings["retention_days"], "90");
    assert_eq!(export.settings["ocr_enabled"], "0");
    for table in [
        "source_roots",
        "artifacts",
        "artifact_versions",
        "passages",
        "relationships",
        "bookmark_records",
    ] {
        assert!(!export.tables[table].rows.is_empty(), "{table} is empty");
    }
    let serialized = serde_json::to_string(&export).unwrap();
    assert!(!serialized.contains("passages_fts"));
    assert!(!serialized.contains("semantic"));

    let restored = Library::open_in_memory().unwrap();
    let report = restored.import_portable(&export).unwrap();
    assert!(report.fts_healthy);
    assert_eq!(
        report.rows.values().sum::<u64>(),
        export.row_count(),
        "{report:?}"
    );
    assert_eq!(
        comparable(restored.export_portable().unwrap()),
        comparable(export)
    );
    assert_eq!(restored.stats().unwrap(), library.stats().unwrap());
    assert_eq!(restored.retention_policy().unwrap().days, Some(90));
    assert!(!restored.ocr_status().unwrap().enabled);
    let hits = restored
        .search(&SearchRequest {
            text: "retry anomaly".into(),
            limit: 5,
        })
        .unwrap();
    assert_eq!(hits.len(), 1);
}

#[test]
fn imports_exports_from_both_supported_schema_revisions() {
    let (_directory, library) = populated();

    // Schema 6 predates the bookmark tables; its exports carry none of them.
    let mut v6 = library.export_portable().unwrap();
    v6.library_schema_version = 6;
    for table in [
        "bookmark_import_failures",
        "bookmark_import_items",
        "bookmark_records",
        "bookmark_imports",
    ] {
        v6.tables.remove(table);
    }
    // Bookmark artifacts belong to the bookmark source root; drop them to form a coherent v6 set.
    let bookmark_roots = v6.tables["source_roots"]
        .rows
        .iter()
        .filter(|row| {
            row[2]
                .as_str()
                .is_some_and(|locator| locator.ends_with(".html"))
        })
        .map(|row| row[0].clone())
        .collect::<Vec<Value>>();
    let artifacts = v6.tables.get_mut("artifacts").unwrap();
    let bookmark_artifacts = artifacts
        .rows
        .iter()
        .filter(|row| bookmark_roots.contains(&row[1]))
        .map(|row| row[0].clone())
        .collect::<Vec<Value>>();
    artifacts
        .rows
        .retain(|row| !bookmark_roots.contains(&row[1]));
    let versions = v6.tables.get_mut("artifact_versions").unwrap();
    let bookmark_versions = versions
        .rows
        .iter()
        .filter(|row| bookmark_artifacts.contains(&row[1]))
        .map(|row| row[0].clone())
        .collect::<Vec<Value>>();
    versions
        .rows
        .retain(|row| !bookmark_artifacts.contains(&row[1]));
    v6.tables
        .get_mut("artifact_locators")
        .unwrap()
        .rows
        .retain(|row| !bookmark_artifacts.contains(&row[1]));
    v6.tables
        .get_mut("passages")
        .unwrap()
        .rows
        .retain(|row| !bookmark_versions.contains(&row[1]));
    v6.tables
        .get_mut("source_roots")
        .unwrap()
        .rows
        .retain(|row| !bookmark_roots.contains(&row[0]));
    v6.seal().unwrap();

    let restored = Library::open_in_memory().unwrap();
    let report = restored.import_portable(&v6).unwrap();
    assert_eq!(report.source_schema_version, 6);
    assert_eq!(report.rows["bookmark_records"], 0);
    assert_eq!(
        restored
            .search(&SearchRequest {
                text: "backup drill".into(),
                limit: 5,
            })
            .unwrap()
            .len(),
        1
    );

    // Schema 7 exports predate the connector metadata columns and the failures table.
    let mut v7 = library.export_portable().unwrap();
    v7.library_schema_version = 7;
    v7.tables.remove("bookmark_import_failures");
    let imports = v7.tables.get_mut("bookmark_imports").unwrap();
    let keep = imports
        .columns
        .iter()
        .map(|column| {
            ![
                "source_application",
                "export_version",
                "permissions_json",
                "skipped_fields_json",
                "status",
            ]
            .contains(&column.as_str())
        })
        .collect::<Vec<_>>();
    imports.columns = imports
        .columns
        .iter()
        .zip(&keep)
        .filter(|(_, keep)| **keep)
        .map(|(column, _)| column.clone())
        .collect();
    for row in &mut imports.rows {
        *row = row
            .iter()
            .zip(&keep)
            .filter(|(_, keep)| **keep)
            .map(|(value, _)| value.clone())
            .collect();
    }
    v7.seal().unwrap();
    let from_v7 = Library::open_in_memory().unwrap();
    assert_eq!(
        from_v7.import_portable(&v7).unwrap().source_schema_version,
        7
    );
    let upgraded = from_v7.export_portable().unwrap();
    let status = upgraded.tables["bookmark_imports"]
        .columns
        .iter()
        .position(|column| column == "status")
        .unwrap();
    assert!(upgraded.tables["bookmark_imports"]
        .rows
        .iter()
        .all(|row| row[status] == "complete"));

    let v8 = library.export_portable().unwrap();
    let current = Library::open_in_memory().unwrap();
    assert_eq!(
        current.import_portable(&v8).unwrap().source_schema_version,
        8
    );

    let mut unsupported = library.export_portable().unwrap();
    unsupported.library_schema_version = 5;
    unsupported.seal().unwrap();
    assert!(Library::open_in_memory()
        .unwrap()
        .import_portable(&unsupported)
        .is_err());
}

fn assert_rejected_and_empty(export: &PortableExport, why: &str) {
    let target = Library::open_in_memory().unwrap();
    assert!(
        target.import_portable(export).is_err(),
        "{why} was accepted"
    );
    let stats = target.stats().unwrap();
    assert_eq!(
        (stats.source_roots, stats.artifacts, stats.passages),
        (0, 0, 0),
        "{why} left rows behind"
    );
}

#[test]
fn corrupted_and_hostile_exports_fail_closed_without_partial_rows() {
    let (_directory, library) = populated();
    let export = library.export_portable().unwrap();

    let mut unsealed = export.clone();
    unsealed.tables.get_mut("passages").unwrap().rows[0][3] = json!("edited text");
    assert_rejected_and_empty(&unsealed, "digest mismatch");

    let mut dangling = export.clone();
    dangling.tables.get_mut("passages").unwrap().rows[0][1] = json!("missing-version");
    dangling.seal().unwrap();
    assert_rejected_and_empty(&dangling, "foreign-key violation");

    let mut unknown_column = export.clone();
    let passages = unknown_column.tables.get_mut("passages").unwrap();
    passages
        .columns
        .push("injected\"; DROP TABLE artifacts; --".into());
    for row in &mut passages.rows {
        row.push(Value::Null);
    }
    unknown_column.seal().unwrap();
    assert_rejected_and_empty(&unknown_column, "unknown column");

    let mut unknown_table = export.clone();
    unknown_table.tables.insert(
        "schema_meta".into(),
        unknown_table.tables["artifacts"].clone(),
    );
    unknown_table.seal().unwrap();
    assert_rejected_and_empty(&unknown_table, "unknown table");

    let mut missing_required = export.clone();
    let passages = missing_required.tables.get_mut("passages").unwrap();
    let text = passages.columns.iter().position(|c| c == "text").unwrap();
    passages.columns.remove(text);
    for row in &mut passages.rows {
        row.remove(text);
    }
    missing_required.seal().unwrap();
    assert_rejected_and_empty(&missing_required, "missing required column");

    let mut bad_setting = export.clone();
    bad_setting
        .settings
        .insert("retention_days".into(), "0".into());
    bad_setting.seal().unwrap();
    assert_rejected_and_empty(&bad_setting, "invalid setting");

    let mut wrong_format = export.clone();
    wrong_format.format_version = 2;
    assert_rejected_and_empty(&wrong_format, "future format");

    let (_other_directory, occupied) = populated();
    assert!(occupied.import_portable(&export).is_err());
}

#[test]
fn encrypted_backup_restores_into_a_new_library() {
    let (directory, library) = populated();
    let backup = directory.path().join("library.loombak");
    let report = library
        .write_encrypted_backup(&backup, PASSWORD, FAST)
        .unwrap();
    assert!(report.rows > 0);
    let bytes = fs::read(&backup).unwrap();
    assert!(bytes.starts_with(b"LOOMBAK1"));
    for plaintext in [&b"retry anomaly"[..], b"backup drill", b"example.test"] {
        assert!(
            !bytes
                .windows(plaintext.len())
                .any(|window| window == plaintext),
            "plaintext leaked into the backup"
        );
    }
    assert!(library
        .write_encrypted_backup(&backup, PASSWORD, FAST)
        .is_err());

    let restored_path = directory.path().join("restored").join("library.sqlite3");
    let restored = Library::restore_encrypted_backup(&backup, PASSWORD, &restored_path).unwrap();
    assert!(restored.import.fts_healthy);
    let reopened = Library::open(&restored_path).unwrap();
    assert_eq!(
        comparable(reopened.export_portable().unwrap()),
        comparable(library.export_portable().unwrap())
    );
    assert!(Library::restore_encrypted_backup(&backup, PASSWORD, &restored_path).is_err());
    let leftovers = fs::read_dir(restored_path.parent().unwrap())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains("loom-restore"))
        .count();
    assert_eq!(leftovers, 0);
}

#[test]
fn tampered_truncated_or_wrong_password_backups_never_create_a_library() {
    let (directory, library) = populated();
    let backup = directory.path().join("library.loombak");
    library
        .write_encrypted_backup(&backup, PASSWORD, FAST)
        .unwrap();
    let bytes = fs::read(&backup).unwrap();
    let restore_dir = directory.path().join("restore");

    let cases: Vec<(&str, Vec<u8>, &[u8])> = vec![
        (
            "wrong password",
            bytes.clone(),
            b"wrong horse battery staple",
        ),
        ("truncated", bytes[..bytes.len() - 20].to_vec(), PASSWORD),
        (
            "flipped ciphertext",
            {
                let mut tampered = bytes.clone();
                let last = tampered.len() - 30;
                tampered[last] ^= 0x80;
                tampered
            },
            PASSWORD,
        ),
        (
            "flipped header",
            {
                let mut tampered = bytes.clone();
                tampered[20] ^= 0x01;
                tampered
            },
            PASSWORD,
        ),
    ];
    for (name, tampered, password) in cases {
        let path = directory
            .path()
            .join(format!("{}.loombak", name.replace(' ', "-")));
        fs::write(&path, tampered).unwrap();
        let database = restore_dir.join(format!("{}.sqlite3", name.replace(' ', "-")));
        assert!(
            Library::restore_encrypted_backup(&path, password, &database).is_err(),
            "{name} restored"
        );
        assert!(!database.exists(), "{name} created a database");
    }
    if restore_dir.exists() {
        assert_eq!(fs::read_dir(&restore_dir).unwrap().count(), 0);
    }
}
