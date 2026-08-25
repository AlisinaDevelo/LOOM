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
    #[cfg(unix)]
    std::os::unix::fs::symlink(
        &outside,
        directory.path().join("cache").join("outside-link"),
    )
    .unwrap();
    let journal = directory.path().join("library.sqlite3-journal");
    fs::write(&journal, "stale journal").unwrap();

    let report = library.purge_disposable_storage().unwrap();
    assert!(report.files_deleted > disposable_files.len() as u64);
    for path in disposable_files {
        assert!(
            !path.exists(),
            "disposable file remained: {}",
            path.display()
        );
    }
    assert!(!journal.exists());
    #[cfg(unix)]
    assert!(
        outside.exists(),
        "cleanup must not follow disposable symlinks"
    );
    assert!(
        source.exists(),
        "cleanup must not delete user-owned source bytes"
    );
    assert!(library.inspect_storage().unwrap().source_bytes > 0);
}
