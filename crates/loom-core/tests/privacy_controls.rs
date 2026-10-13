use std::fs;

use chrono::Utc;
use loom_core::{Library, SearchRequest};
use tempfile::tempdir;

fn search(library: &Library, query: &str) -> Vec<loom_core::SearchHit> {
    library
        .search(&SearchRequest {
            text: query.into(),
            limit: 10,
        })
        .unwrap()
}

#[test]
fn storage_inspection_accounts_for_sources_and_known_disposable_files() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("library.sqlite3");
    let source = directory.path().join("notes.md");
    fs::write(&source, "storage inspector source marker").unwrap();
    let library = Library::open(&database).unwrap();
    library.index_path(&source).unwrap();

    for (name, content) in [
        ("cache/query.cache", "cache marker"),
        ("model-cache/model.bin", "model marker"),
        ("thumbnails/preview.png", "thumbnail marker"),
        ("ocr-scratch/page.txt", "ocr marker"),
        ("tmp-exports/export.json", "export marker"),
        ("logs/loom.log", "log marker"),
    ] {
        let path = directory.path().join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    let inspection = library.inspect_storage().unwrap();
    let canonical_source = source.canonicalize().unwrap();
    assert!(inspection.source_bytes >= "storage inspector source marker".len() as u64);
    assert!(inspection.disposable_bytes > 0);
    assert!(inspection.total_bytes >= inspection.source_bytes);
    assert!(inspection.entries.iter().any(
        |entry| entry.category == "source" && entry.path == canonical_source.to_string_lossy()
    ));
    for category in [
        "cache",
        "model_cache",
        "thumbnails",
        "ocr_scratch",
        "temporary_export",
        "log",
    ] {
        assert!(
            inspection
                .entries
                .iter()
                .any(|entry| entry.category == category),
            "missing storage category {category}"
        );
    }
    let wire = serde_json::to_value(&inspection).unwrap();
    for entry in wire["entries"].as_array().unwrap() {
        if matches!(
            entry["category"].as_str().unwrap(),
            "database"
                | "sqlite_sidecar"
                | "cache"
                | "model_cache"
                | "thumbnails"
                | "ocr_scratch"
                | "temporary_export"
                | "log"
        ) {
            assert_eq!(entry["exists"], true, "managed path was inspected");
        }
    }
}

#[test]
fn storage_source_history_keeps_version_sizes_and_original_projection() {
    // Synthetic text generated at runtime; LOOM retains observations, not original file copies.
    let sources = tempdir().unwrap();
    let database = tempdir().unwrap();
    let source = sources.path().join("history.md");
    let original_prefix = "originalstoragehistory source marker ";
    let original = format!(
        "{original_prefix}{}",
        "x".repeat(192 - original_prefix.len())
    );
    let revision_prefix = "revisedstoragehistory source marker ";
    let revision = format!(
        "{revision_prefix}{}",
        "y".repeat(60 - revision_prefix.len())
    );
    fs::write(&source, &original).unwrap();
    let library = Library::open(database.path().join("library.sqlite3")).unwrap();
    library.index_path(&source).unwrap();
    let before = search(&library, "originalstoragehistory").pop().unwrap();
    let original_observation = library.inspect_source(&source).unwrap();

    fs::write(&source, &revision).unwrap();
    library.index_path(&source).unwrap();
    assert_eq!(library.stats().unwrap().versions, 2);
    fs::write(&source, &original).unwrap();
    library.index_path(&source).unwrap();
    let after = search(&library, "originalstoragehistory").pop().unwrap();
    assert_eq!(after.artifact_id, before.artifact_id);
    assert_eq!(after.version_id, before.version_id);
    assert_eq!(after.passage_id, before.passage_id);
    assert_eq!(
        library.inspect_source(&source).unwrap(),
        original_observation
    );
    assert_eq!(library.stats().unwrap().versions, 2);
    let canonical_before = library.export_portable().unwrap().tables;

    let inspection = library.inspect_storage().unwrap();
    assert_eq!(inspection.source_bytes, 252);
    let sources = inspection
        .entries
        .iter()
        .filter(|entry| entry.category == "source")
        .collect::<Vec<_>>();
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0].bytes, 252);
    assert_eq!(sources[0].files, 1);
    assert_eq!(fs::metadata(&source).unwrap().len(), 192);
    assert_eq!(fs::read_to_string(&source).unwrap(), original);
    assert_eq!(library.export_portable().unwrap().tables, canonical_before);
    let wire = serde_json::to_value(sources[0]).unwrap();
    assert!(
        wire["exists"].is_null(),
        "history does not measure source availability"
    );
}

#[test]
fn storage_source_availability_is_unmeasured_for_existing_missing_and_revoked_locators() {
    let mut availability = Vec::new();
    for scenario in ["existing", "missing", "revoked"] {
        let sources = tempdir().unwrap();
        let database = tempdir().unwrap();
        let source = sources.path().join("selected.md");
        let original = "syntheticstorageavailability source marker";
        fs::write(&source, original).unwrap();
        let locator = source
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let library = Library::open(database.path().join("library.sqlite3")).unwrap();
        library.index_path(&source).unwrap();
        match scenario {
            "missing" => fs::remove_file(&source).unwrap(),
            "revoked" => {
                library.revoke_source_root(&locator).unwrap();
                assert!(search(&library, "syntheticstorageavailability").is_empty());
            }
            _ => {}
        }
        let before = library.export_portable().unwrap().tables;
        let inspection = library.inspect_storage().unwrap();
        assert_eq!(inspection.source_bytes, original.len() as u64);
        assert_eq!(library.export_portable().unwrap().tables, before);
        if scenario != "missing" {
            assert_eq!(fs::read_to_string(&source).unwrap(), original);
        }
        let source_entries = inspection
            .entries
            .iter()
            .filter(|entry| entry.category == "source")
            .collect::<Vec<_>>();
        assert_eq!(source_entries.len(), 1);
        assert_eq!(
            source_entries[0].source_uri.as_deref(),
            Some(locator.as_str())
        );
        let wire = serde_json::to_value(source_entries[0]).unwrap();
        availability.push((scenario, wire["exists"].clone()));
    }
    assert!(
        availability.iter().all(|(_, exists)| exists.is_null()),
        "source availability was invented: {availability:?}"
    );
}

#[cfg(unix)]
#[test]
fn storage_source_symlink_replacement_does_not_probe_or_admit_the_target() {
    let sources = tempdir().unwrap();
    let database = tempdir().unwrap();
    let source = sources.path().join("selected.md");
    let outside = sources.path().join("unselected.md");
    let original = "selectedstoragesource marker";
    let outside_bytes = "unselectedstoragetarget marker";
    fs::write(&source, original).unwrap();
    fs::write(&outside, outside_bytes).unwrap();
    let library = Library::open(database.path().join("library.sqlite3")).unwrap();
    library.index_path(&source).unwrap();
    fs::remove_file(&source).unwrap();
    std::os::unix::fs::symlink(&outside, &source).unwrap();
    let before = library.export_portable().unwrap().tables;

    let inspection = library.inspect_storage().unwrap();
    assert_eq!(inspection.source_bytes, original.len() as u64);
    assert_eq!(library.export_portable().unwrap().tables, before);
    assert_eq!(fs::read_to_string(&outside).unwrap(), outside_bytes);
    assert!(search(&library, "unselectedstoragetarget").is_empty());
    let source_entry = inspection
        .entries
        .iter()
        .find(|entry| entry.category == "source")
        .unwrap();
    let wire = serde_json::to_value(source_entry).unwrap();
    assert!(
        wire["exists"].is_null(),
        "symlink target availability was claimed"
    );
}

#[test]
fn storage_url_and_in_memory_estimates_do_not_claim_filesystem_availability() {
    let directory = tempdir().unwrap();
    let export = directory.path().join("bookmarks.html");
    let html = "<!DOCTYPE NETSCAPE-Bookmark-file-1>\n\
                <TITLE>Bookmarks</TITLE><H1>Bookmarks</H1>\n\
                <DL><p><DT><A HREF=\"https://example.invalid/storage\">Synthetic storage bookmark</A></DL><p>";
    fs::write(&export, html).unwrap();
    let library = Library::open_in_memory().unwrap();
    assert_eq!(library.import_bookmarks(&export).unwrap().remote_fetches, 0);
    let before = library.export_portable().unwrap().tables;
    let inspection = library.inspect_storage().unwrap();
    assert!(inspection.source_bytes > 0);
    assert!(inspection
        .entries
        .iter()
        .any(|entry| entry.source_uri.as_deref() == Some("https://example.invalid/storage")));
    assert_eq!(library.export_portable().unwrap().tables, before);
    assert_eq!(fs::read_to_string(&export).unwrap(), html);
    let wire = serde_json::to_value(inspection).unwrap();
    assert!(
        wire["entries"]
            .as_array()
            .unwrap()
            .iter()
            .all(|entry| entry["exists"].is_null()),
        "logical and URL entries do not measure filesystem presence"
    );
}

#[test]
fn purge_artifact_removes_evidence_and_survives_restart() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("library.sqlite3");
    let source = directory.path().join("private.md");
    fs::write(&source, "private artifact deletion marker").unwrap();
    let library = Library::open(&database).unwrap();
    library.index_path(&source).unwrap();
    let hit = search(&library, "private artifact deletion marker")
        .into_iter()
        .next()
        .unwrap();

    let report = library.purge_artifact(&hit.artifact_id).unwrap();
    assert_eq!(report.artifacts_deleted, 1);
    assert_eq!(report.versions_deleted, 1);
    assert!(report.passages_deleted >= 1);
    assert!(search(&library, "private artifact deletion marker").is_empty());
    assert!(library.fts_health().unwrap().healthy);
    drop(library);

    let reopened = Library::open(&database).unwrap();
    assert_eq!(reopened.stats().unwrap().artifacts, 0);
    assert!(search(&reopened, "private artifact deletion marker").is_empty());
    assert!(reopened.fts_health().unwrap().healthy);
    let database_bytes = fs::read(&database).unwrap();
    assert!(!database_bytes
        .windows("private artifact deletion marker".len())
        .any(|window| window == "private artifact deletion marker".as_bytes()));
}

#[test]
fn root_and_time_deletion_are_explicit_and_retention_is_deterministic() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("library.sqlite3");
    let source_root = directory.path().join("selected");
    fs::create_dir_all(&source_root).unwrap();
    fs::write(source_root.join("one.md"), "root deletion marker").unwrap();
    let library = Library::open(&database).unwrap();
    library.index_path(&source_root).unwrap();
    let locator = source_root
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .into_owned();

    let report = library.purge_root(&locator).unwrap();
    assert_eq!(report.artifacts_deleted, 1);
    assert_eq!(library.source_roots().unwrap().len(), 0);
    assert!(search(&library, "root deletion marker").is_empty());

    fs::write(source_root.join("retention.md"), "retention marker").unwrap();
    library.index_path(&source_root).unwrap();
    assert!(library.set_retention_days(Some(0)).is_err());
    assert!(library.set_retention_days(Some(36_501)).is_err());
    assert!(library.purge_before("not-a-timestamp").is_err());
    assert_eq!(library.set_retention_days(Some(1)).unwrap().days, Some(1));
    let evaluated_at = "2030-01-02T00:00:00Z";
    let retention = library.apply_retention_at(evaluated_at).unwrap();
    assert_eq!(retention.policy.days, Some(1));
    assert_eq!(
        retention.cutoff.as_deref(),
        Some("2030-01-01T00:00:00+00:00")
    );
    assert_eq!(retention.deletion.artifacts_deleted, 2);
    assert!(search(&library, "retention marker").is_empty());

    assert!(library.set_retention_days(None).unwrap().days.is_none());
    let disabled = library
        .apply_retention_at(&Utc::now().to_rfc3339())
        .unwrap();
    assert_eq!(disabled.deletion.artifacts_deleted, 0);
    drop(library);
    let reopened = Library::open(&database).unwrap();
    assert_eq!(reopened.stats().unwrap().artifacts, 0);
    assert!(search(&reopened, "root deletion marker").is_empty());
    assert!(search(&reopened, "retention marker").is_empty());
}

#[test]
fn disposable_cleanup_removes_only_known_local_derivatives() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("library.sqlite3");
    let source = directory.path().join("source.md");
    fs::write(&source, "user-owned source remains").unwrap();
    let library = Library::open(&database).unwrap();
    library.index_path(&source).unwrap();
    let disposable_files = [
        ("cache", "stale.bin"),
        ("model-cache", "model.bin"),
        ("thumbnails", "thumb.png"),
        ("ocr-scratch", "page.txt"),
        ("tmp-exports", "export.json"),
        ("logs", "loom.log"),
    ]
    .into_iter()
    .map(|(directory_name, file_name)| {
        let path = directory.path().join(directory_name).join(file_name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "stale derivative").unwrap();
        path
    })
    .collect::<Vec<_>>();
    let outside = directory.path().join("outside.txt");
    fs::write(&outside, "outside source remains").unwrap();
    let unowned_sidecar_like = directory.path().join("library.sqlite3-wal.backup");
    fs::write(&unowned_sidecar_like, "unowned sibling remains").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(
        &outside,
        directory.path().join("cache").join("outside-link"),
    )
    .unwrap();

    let sqlite_sidecars = library
        .inspect_storage()
        .unwrap()
        .entries
        .into_iter()
        .filter(|entry| entry.category == "sqlite_sidecar")
        .map(|entry| entry.path)
        .collect::<Vec<_>>();
    assert!(
        !sqlite_sidecars.is_empty(),
        "the regression fixture must exercise a live SQLite sidecar"
    );

    let report = library.purge_disposable_storage().unwrap();
    assert_eq!(report.files_deleted, disposable_files.len() as u64);
    assert!(
        sqlite_sidecars
            .iter()
            .all(|sidecar| !report.paths.contains(sidecar)),
        "SQLite-owned sidecars must not be reported as application-deleted"
    );
    for path in &disposable_files {
        assert!(
            !path.exists(),
            "disposable file remained: {}",
            path.display()
        );
    }
    #[cfg(unix)]
    assert!(
        outside.exists(),
        "cleanup must not follow disposable symlinks"
    );
    assert!(
        unowned_sidecar_like.exists(),
        "cleanup must not broaden to similarly named sibling files"
    );
    assert!(
        source.exists(),
        "cleanup must not delete user-owned source bytes"
    );
    assert!(library.inspect_storage().unwrap().source_bytes > 0);
    assert_eq!(search(&library, "user-owned source remains").len(), 1);
    assert!(library.fts_health().unwrap().healthy);

    let source_after_cleanup = directory.path().join("after-cleanup.md");
    fs::write(
        &source_after_cleanup,
        "post-cleanup writes remain searchable",
    )
    .unwrap();
    library.index_path(&source_after_cleanup).unwrap();
    assert_eq!(
        search(&library, "post-cleanup writes remain searchable").len(),
        1
    );
    drop(library);

    let reopened = Library::open(&database).unwrap();
    assert_eq!(reopened.stats().unwrap().artifacts, 2);
    assert_eq!(search(&reopened, "user-owned source remains").len(), 1);
    assert_eq!(
        search(&reopened, "post-cleanup writes remain searchable").len(),
        1
    );
    assert!(reopened.fts_health().unwrap().healthy);
}
