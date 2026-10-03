use std::fs;

use loom_core::{
    BackupOptions, Library, PortableExport, RelationshipInput, RelationshipKind,
    RelationshipOrigin, SearchRequest,
};
use serde_json::{json, Value};
use tempfile::{tempdir, TempDir};

const CHROME_EXPORT: &str = include_str!("fixtures/bookmarks/chrome.html");
const PASSWORD: &[u8] = b"correct horse battery staple";
/// The cheapest cost the public API accepts (the OWASP Argon2id floor).
const FAST: BackupOptions = BackupOptions {
    kdf_memory_kib: loom_core::MIN_KDF_MEMORY_KIB,
    kdf_iterations: loom_core::MIN_KDF_ITERATIONS,
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
fn scope_inconsistent_bookmark_exports_are_refused_transactionally() {
    for mismatch in [
        "import_root",
        "import_locator",
        "record_artifact",
        "import_item",
        "failure_resolution",
    ] {
        let (directory, library) = populated();
        let extra = directory.path().join("second-bookmarks.html");
        fs::write(
            &extra,
            CHROME_EXPORT.replace("https://", "https://second.example.test/"),
        )
        .unwrap();
        library.import_bookmarks(&extra).unwrap();
        let mut export = library.export_portable().unwrap();
        let column = |table: &str, name: &str| {
            export.tables[table]
                .columns
                .iter()
                .position(|value| value == name)
                .unwrap()
        };
        match mismatch {
            "import_root" => {
                let root_id = column("source_roots", "id");
                let kind = column("source_roots", "kind");
                let wrong_root = export.tables["source_roots"]
                    .rows
                    .iter()
                    .find(|row| row[kind] == json!("directory"))
                    .unwrap()[root_id]
                    .clone();
                let root_column = column("bookmark_imports", "source_root_id");
                export.tables.get_mut("bookmark_imports").unwrap().rows[0][root_column] =
                    wrong_root;
            }
            "record_artifact" => {
                let artifact_id = column("artifacts", "id");
                let media = column("artifacts", "media_type");
                let wrong_artifact = export.tables["artifacts"]
                    .rows
                    .iter()
                    .find(|row| row[media] == json!("text/markdown"))
                    .unwrap()[artifact_id]
                    .clone();
                let artifact_column = column("bookmark_records", "artifact_id");
                export.tables.get_mut("bookmark_records").unwrap().rows[0][artifact_column] =
                    wrong_artifact;
            }
            "import_locator" => {
                let locator = column("bookmark_imports", "source_locator");
                export.tables.get_mut("bookmark_imports").unwrap().rows[0][locator] =
                    json!("/unselected/export.html");
            }
            "failure_resolution" => {
                let import_id = column("bookmark_imports", "id");
                let first = export.tables["bookmark_imports"].rows[0][import_id].clone();
                let second = export.tables["bookmark_imports"].rows[1][import_id].clone();
                let failures = export.tables.get_mut("bookmark_import_failures").unwrap();
                let row = failures
                    .columns
                    .iter()
                    .map(|name| match name.as_str() {
                        "import_id" => first.clone(),
                        "resolved_by_import_id" => second.clone(),
                        "ordinal" | "byte_offset" => json!(0),
                        "state" => json!("resolved"),
                        "code" => json!("synthetic_scope_conflict"),
                        "detail" => json!("synthetic fixture"),
                        "created_at" => json!("2026-10-03T00:00:00Z"),
                        _ => panic!("unexpected failure column"),
                    })
                    .collect();
                failures.rows.push(row);
            }
            _ => {
                let import_id = column("bookmark_imports", "id");
                let wrong_import = export.tables["bookmark_imports"].rows[1][import_id].clone();
                let item_import = column("bookmark_import_items", "import_id");
                let item_bookmark = column("bookmark_import_items", "bookmark_id");
                let first_import = export.tables["bookmark_imports"].rows[0][import_id].clone();
                let items = export.tables.get_mut("bookmark_import_items").unwrap();
                let original_bookmark = items
                    .rows
                    .iter()
                    .find(|row| row[item_import] == first_import)
                    .unwrap()[item_bookmark]
                    .clone();
                let row = items
                    .rows
                    .iter_mut()
                    .find(|row| row[item_import] == wrong_import)
                    .unwrap();
                row[item_bookmark] = original_bookmark;
            }
        }
        export.seal().unwrap();
        let target = Library::open_in_memory().unwrap();
        let before = target.export_portable().unwrap();
        assert!(
            target.import_portable(&export).is_err(),
            "accepted {mismatch}"
        );
        assert_eq!(
            comparable(target.export_portable().unwrap()),
            comparable(before)
        );
    }
}

#[test]
fn export_import_round_trip_preserves_every_canonical_row_and_setting() {
    let (_directory, library) = populated();
    let export = library.export_portable().unwrap();
    assert_eq!(export.library_schema_version, 10);
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
fn schema_v9_export_without_generation_imports_with_safe_default() {
    let (_directory, library) = populated();
    let mut export = library.export_portable().unwrap();
    export.library_schema_version = 9;
    let roots = export.tables.get_mut("source_roots").unwrap();
    let column = roots
        .columns
        .iter()
        .position(|column| column == "scope_generation")
        .unwrap();
    roots.columns.remove(column);
    for row in &mut roots.rows {
        row.remove(column);
    }
    export.seal().unwrap();
    let restored = Library::open_in_memory().unwrap();
    assert!(restored.import_portable(&export).unwrap().fts_healthy);
    let current = restored.export_portable().unwrap();
    let roots = &current.tables["source_roots"];
    let column = roots
        .columns
        .iter()
        .position(|column| column == "scope_generation")
        .unwrap();
    assert!(roots.rows.iter().all(|row| row[column] == json!(0)));
    assert_eq!(restored.stats().unwrap(), library.stats().unwrap());
}

#[test]
fn portable_restore_preserves_revoked_generations_and_never_reenables_them() {
    let (_directory, library) = populated();
    let root = library
        .source_roots()
        .unwrap()
        .into_iter()
        .find(|root| root.kind == "directory")
        .unwrap();
    library.revoke_source_root(&root.locator).unwrap();
    let export = library.export_portable().unwrap();
    let restored = Library::open_in_memory().unwrap();
    restored.import_portable(&export).unwrap();
    assert_eq!(
        comparable(restored.export_portable().unwrap()),
        comparable(export)
    );
    assert!(restored
        .search(&SearchRequest {
            text: "retry anomaly".into(),
            limit: 5
        })
        .unwrap()
        .is_empty());
    assert!(restored
        .source_roots()
        .unwrap()
        .iter()
        .any(|source| source.locator == root.locator && !source.enabled));
    assert_eq!(
        restored.reconcile_approved_roots().unwrap().roots_scanned,
        1
    );
}

#[test]
fn imports_exports_from_supported_schema_revisions() {
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

    // Schema 8 exports predate the relationship compaction table.
    let mut v8 = library.export_portable().unwrap();
    v8.library_schema_version = 8;
    v8.tables.remove("relationship_compactions");
    v8.seal().unwrap();
    let from_v8 = Library::open_in_memory().unwrap();
    assert_eq!(
        from_v8.import_portable(&v8).unwrap().source_schema_version,
        8
    );

    let latest = library.export_portable().unwrap();
    let current = Library::open_in_memory().unwrap();
    assert_eq!(
        current
            .import_portable(&latest)
            .unwrap()
            .source_schema_version,
        10
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

#[test]
fn weak_key_derivation_options_are_refused_when_writing() {
    let (directory, library) = populated();
    for weak in [
        BackupOptions {
            kdf_memory_kib: 64,
            ..FAST
        },
        BackupOptions {
            kdf_iterations: 1,
            ..FAST
        },
    ] {
        let path = directory.path().join("weak.loombak");
        assert!(library
            .write_encrypted_backup(&path, PASSWORD, weak)
            .is_err());
        assert!(!path.exists());
    }
}

#[cfg(unix)]
#[test]
fn restore_sweeps_stale_staging_and_rejects_non_regular_inputs() {
    use std::os::unix::fs::PermissionsExt;

    let (directory, library) = populated();
    let backup = directory.path().join("library.loombak");
    library
        .write_encrypted_backup(&backup, PASSWORD, FAST)
        .unwrap();
    let restore_dir = directory.path().join("restore");
    fs::create_dir(&restore_dir).unwrap();
    let stale = restore_dir.join(".loom-restore-stale");
    fs::create_dir(&stale).unwrap();
    fs::write(stale.join("library.sqlite3"), b"leftover plaintext").unwrap();
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(7200);
    fs::File::open(&stale).unwrap().set_modified(old).unwrap();

    Library::restore_encrypted_backup(&backup, PASSWORD, restore_dir.join("library.sqlite3"))
        .unwrap();
    let names = fs::read_dir(&restore_dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        vec!["library.sqlite3"],
        "staging left behind: {names:?}"
    );
    let reopened = Library::open(restore_dir.join("library.sqlite3")).unwrap();
    assert!(reopened.stats().unwrap().passages > 0);

    let not_a_file = directory.path().join("folder.loombak");
    fs::create_dir(&not_a_file).unwrap();
    assert!(Library::restore_encrypted_backup(
        &not_a_file,
        PASSWORD,
        restore_dir.join("b.sqlite3")
    )
    .is_err());
    assert!(fs::metadata(&restore_dir).unwrap().permissions().mode() & 0o777 != 0);
}

#[test]
fn integers_beyond_sqlite_range_are_rejected_on_import() {
    let (_directory, library) = populated();
    let mut export = library.export_portable().unwrap();
    let versions = export.tables.get_mut("artifact_versions").unwrap();
    let byte_size = versions
        .columns
        .iter()
        .position(|column| column == "byte_size")
        .unwrap();
    versions.rows[0][byte_size] = json!(u64::MAX);
    export.seal().unwrap();
    let target = Library::open_in_memory().unwrap();
    assert!(target.import_portable(&export).is_err());
    assert_eq!(target.stats().unwrap().artifacts, 0);
}
