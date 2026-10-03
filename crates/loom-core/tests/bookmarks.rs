use std::fs;

use loom_core::{parse_bookmark_export, Library};
use tempfile::tempdir;

const CHROME_EXPORT: &str = include_str!("fixtures/bookmarks/chrome.html");

fn active_artifact_count(library: &Library) -> usize {
    let export = library.export_portable().unwrap();
    let artifacts = &export.tables["artifacts"];
    let state = artifacts
        .columns
        .iter()
        .position(|column| column == "state")
        .unwrap();
    artifacts
        .rows
        .iter()
        .filter(|row| row[state].as_str() == Some("active"))
        .count()
}

#[test]
fn reselection_of_a_changed_export_restores_unchanged_bookmarks() {
    let directory = tempdir().unwrap();
    let export = directory.path().join("Bookmarks.html");
    fs::write(&export, CHROME_EXPORT).unwrap();
    let library = Library::open_in_memory().unwrap();
    let first = library.import_bookmarks(&export).unwrap();
    let before = library.list_bookmarks(10).unwrap();
    library.revoke_source_root(&first.source_uri).unwrap();
    fs::write(
        &export,
        format!("{CHROME_EXPORT}\n<!-- changed export header -->"),
    )
    .unwrap();
    let changed = library.import_bookmarks(&export).unwrap();
    assert_ne!(changed.import_id, first.import_id);
    assert_eq!(changed.unchanged, first.discovered);
    assert_eq!(active_artifact_count(&library), before.len());
    assert_eq!(
        library.list_bookmarks(10).unwrap()[0].artifact_id,
        before[0].artifact_id
    );
    assert_eq!(
        library
            .search(&loom_core::SearchRequest {
                text: "Rust SQLite".into(),
                limit: 10
            })
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn retry_refuses_a_disabled_root_even_if_legacy_status_is_complete() {
    let directory = tempdir().unwrap();
    let export = directory.path().join("Bookmarks.html");
    let database = directory.path().join("library.sqlite3");
    fs::write(&export, CHROME_EXPORT).unwrap();
    let library = Library::open(&database).unwrap();
    let first = library.import_bookmarks(&export).unwrap();
    library.revoke_source_root(&first.source_uri).unwrap();
    let connection = rusqlite::Connection::open(&database).unwrap();
    // Old connector schemas defaulted to complete even when the persisted root was disabled.
    connection
        .execute("UPDATE bookmark_imports SET status = 'complete'", [])
        .unwrap();
    assert!(library.retry_bookmark_import(&first.import_id).is_err());
    assert!(!library.source_roots().unwrap()[0].enabled);
    assert_eq!(active_artifact_count(&library), 0);
}

#[test]
fn identical_replay_refuses_scope_inconsistent_import_items() {
    let directory = tempdir().unwrap();
    let first = directory.path().join("first.html");
    let second = directory.path().join("second.html");
    let database = directory.path().join("library.sqlite3");
    fs::write(&first, CHROME_EXPORT).unwrap();
    fs::write(
        &second,
        CHROME_EXPORT.replace("https://", "https://second.example.test/"),
    )
    .unwrap();
    let library = Library::open(&database).unwrap();
    let first_import = library.import_bookmarks(&first).unwrap();
    let second_import = library.import_bookmarks(&second).unwrap();
    library
        .revoke_source_root(&second_import.source_uri)
        .unwrap();
    let connection = rusqlite::Connection::open(&database).unwrap();
    let second_bookmark: String = connection
        .query_row(
            "SELECT bookmark_id FROM bookmark_import_items WHERE import_id = ?1 LIMIT 1",
            [&second_import.import_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(connection.execute(
        "UPDATE bookmark_import_items SET bookmark_id = ?1 WHERE import_id = ?2 AND ordinal = 0",
        rusqlite::params![second_bookmark, first_import.import_id]).unwrap(), 1);
    let before = library.export_portable().unwrap().tables;
    assert!(library.import_bookmarks(&first).is_err());
    let after = library.export_portable().unwrap().tables;
    for table in [
        "artifacts",
        "artifact_versions",
        "bookmark_imports",
        "bookmark_import_items",
        "passages",
    ] {
        assert_eq!(before[table], after[table]);
    }
    assert_eq!(active_artifact_count(&library), 1);
}
const FIREFOX_EXPORT: &str = include_str!("fixtures/bookmarks/firefox.html");

#[test]
fn a_second_export_cannot_reactivate_or_adopt_another_scopes_bookmark() {
    for (revoked, changed_folder, changed_title) in [
        (false, false, false),
        (false, true, true),
        (true, false, false),
        (true, false, true),
        (true, true, true),
    ] {
        let directory = tempdir().unwrap();
        let first = directory.path().join("first.html");
        let second = directory.path().join("second.html");
        let database = directory.path().join("library.sqlite3");
        fs::write(
            &first,
            CHROME_EXPORT.replace("Rust &amp; SQLite", "firstprivatebookmark"),
        )
        .unwrap();
        let library = Library::open(&database).unwrap();
        library.import_bookmarks(&first).unwrap();
        let first_locator = first.canonicalize().unwrap().to_string_lossy().into_owned();
        if revoked {
            library.revoke_source_root(&first_locator).unwrap();
        }
        let before = library.export_portable().unwrap().tables;
        let mut content = fs::read_to_string(&first).unwrap();
        if changed_folder {
            content = content.replace("Engineering", "Other");
        }
        if changed_title {
            content = content.replace("firstprivatebookmark", "secondprivatebookmark");
        }
        content = content.replacen(
            "<DL><p>",
            "<DL><p>\n<DT><A HREF=\"https://example.test/unique-second\">secondonlybookmark</A>",
            1,
        );
        fs::write(&second, content).unwrap();

        // The second export imports its own entry; the shared URL stays with the first export and
        // is recorded as a per-record failure instead of being adopted or reactivated.
        let report = library.import_bookmarks(&second).unwrap();
        assert_eq!(report.imported, 1);
        assert_eq!(report.failed, 1, "the shared URL must not be adopted");
        assert!(report.failures[0].reason.contains("owned_by_other_export"));
        let after = library.export_portable().unwrap().tables;
        for table in [
            "artifacts",
            "artifact_versions",
            "bookmark_records",
            "bookmark_imports",
            "bookmark_import_items",
            "passages",
        ] {
            for row in &before[table].rows {
                assert!(
                    after[table].rows.contains(row),
                    "{table} row owned by the first export changed on a cross-scope import"
                );
            }
        }
        assert_eq!(
            library
                .search(&loom_core::SearchRequest {
                    text: "secondonlybookmark".into(),
                    limit: 10,
                })
                .unwrap()
                .len(),
            1
        );
        let connection = rusqlite::Connection::open(&database).unwrap();
        let active_under_revoked: i64 = connection
            .query_row(
                "SELECT count(*) FROM artifacts a JOIN source_roots r ON r.id = a.source_root_id
                WHERE r.enabled = 0 AND a.state = 'active'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(active_under_revoked, 0);
        let hits = library
            .search(&loom_core::SearchRequest {
                text: "firstprivatebookmark".into(),
                limit: 10,
            })
            .unwrap();
        assert_eq!(hits.len(), usize::from(!revoked));
        if revoked {
            library.import_bookmarks(&first).unwrap();
            assert_eq!(
                library
                    .search(&loom_core::SearchRequest {
                        text: "firstprivatebookmark".into(),
                        limit: 10
                    })
                    .unwrap()
                    .len(),
                1
            );
        }
    }
}

#[test]
fn chrome_and_firefox_exports_preserve_folder_title_url_and_timestamps() {
    let chrome = parse_bookmark_export(CHROME_EXPORT).unwrap();
    assert_eq!(chrome.format, "netscape_html");
    assert_eq!(chrome.bookmarks.len(), 1);
    assert_eq!(chrome.bookmarks[0].folder_path, "Engineering");
    assert_eq!(chrome.bookmarks[0].title, "Rust & SQLite");
    assert_eq!(chrome.bookmarks[0].url, "https://example.test/rust?x=1&y=2");
    assert_eq!(chrome.bookmarks[0].added_at.as_deref(), Some("1700000001"));
    assert_eq!(
        chrome.bookmarks[0].modified_at.as_deref(),
        Some("1700000002")
    );

    let firefox = parse_bookmark_export(FIREFOX_EXPORT).unwrap();
    assert_eq!(firefox.bookmarks.len(), 1);
    assert_eq!(firefox.bookmarks[0].folder_path, "Research / Local-first");
    assert_eq!(firefox.bookmarks[0].title, "Evidence \"first\"");
    assert_eq!(firefox.bookmarks[0].added_at.as_deref(), Some("1700000010"));
}

#[test]
fn repeated_import_is_idempotent_and_searchable_without_fetching_urls() {
    let directory = tempdir().unwrap();
    let export = directory.path().join("Bookmarks.html");
    fs::write(&export, CHROME_EXPORT).unwrap();
    let library = Library::open_in_memory().unwrap();

    let first = library.import_bookmarks(&export).unwrap();
    assert_eq!(first.discovered, 1);
    assert_eq!(first.imported, 1);
    assert_eq!(first.remote_fetches, 0);
    assert_eq!(library.list_bookmarks(10).unwrap().len(), 1);

    let second = library.import_bookmarks(&export).unwrap();
    assert_eq!(second.discovered, 1);
    assert_eq!(second.imported, 0);
    assert_eq!(second.unchanged, 1);
    assert_eq!(second.remote_fetches, 0);
    assert_eq!(library.stats().unwrap().artifacts, 1);
    let hit = library
        .search(&loom_core::SearchRequest {
            text: "\"Rust SQLite\"".into(),
            limit: 10,
        })
        .unwrap();
    assert_eq!(hit.len(), 1);
    assert_eq!(hit[0].source_uri, "https://example.test/rust?x=1&y=2");
    assert_eq!(hit[0].title, "Rust & SQLite");
}

#[test]
fn changed_exports_merge_metadata_and_report_duplicate_url_conflicts() {
    let directory = tempdir().unwrap();
    let export = directory.path().join("Bookmarks.html");
    fs::write(&export, CHROME_EXPORT).unwrap();
    let library = Library::open_in_memory().unwrap();
    library.import_bookmarks(&export).unwrap();

    let changed = CHROME_EXPORT.replace(
        "<DT><A HREF=\"https://example.test/rust?x=1&amp;y=2\" ADD_DATE=\"1700000001\" LAST_MODIFIED=\"1700000002\">Rust &amp; SQLite</A>",
        "<DT><H3>Later</H3><DL><p><DT><A HREF=\"https://example.test/rust?x=1&amp;y=2\" ADD_DATE=\"1700000003\">Renamed Rust</A></DL><p>",
    );
    fs::write(&export, changed).unwrap();
    let report = library.import_bookmarks(&export).unwrap();
    assert_eq!(report.imported, 1);
    assert_eq!(report.merged, 0);
    assert_eq!(report.conflicts, 1);
    assert_eq!(report.remote_fetches, 0);

    let records = library.list_bookmarks(10).unwrap();
    assert_eq!(records.len(), 2);
    assert!(records
        .iter()
        .any(|record| record.folder_path == "Engineering"));
    assert!(records
        .iter()
        .any(|record| record.folder_path == "Engineering / Later"));
    assert!(records.iter().all(|record| !record.import_id.is_empty()));
}

#[test]
fn changed_bookmark_timestamps_create_a_distinct_version_and_remain_visible() {
    let directory = tempdir().unwrap();
    let export = directory.path().join("Bookmarks.html");
    fs::write(&export, CHROME_EXPORT).unwrap();
    let library = Library::open_in_memory().unwrap();
    library.import_bookmarks(&export).unwrap();

    let changed = CHROME_EXPORT.replace("ADD_DATE=\"1700000001\"", "ADD_DATE=\"1700000003\"");
    fs::write(&export, changed).unwrap();
    let report = library.import_bookmarks(&export).unwrap();
    assert_eq!(report.merged, 1);
    assert_eq!(library.stats().unwrap().versions, 2);
    let record = library.list_bookmarks(1).unwrap().pop().unwrap();
    assert_eq!(record.added_at.as_deref(), Some("1700000003"));
}

#[test]
fn malformed_or_unsafe_bookmarks_fail_closed_before_writing_rows() {
    let directory = tempdir().unwrap();
    let export = directory.path().join("Bookmarks.html");
    fs::write(
        &export,
        "<DL><p><DT><A HREF=\"javascript:alert(1)\">unsafe</A></DL>",
    )
    .unwrap();
    let library = Library::open_in_memory().unwrap();
    assert!(library.import_bookmarks(&export).is_err());
    assert!(library.list_bookmarks(10).unwrap().is_empty());
}

#[test]
fn parser_rejects_malformed_exports_and_oversized_urls() {
    for malformed in [
        "<DL><p><DT><A HREF=\"https://example.test\">missing marker</A></DL>",
        "<!DOCTYPE NETSCAPE-Bookmark-file-1><DL><p><DT><A>missing href</A></DL>",
        "<!DOCTYPE NETSCAPE-Bookmark-file-1><DL><p><DT><A HREF=\"https://example.test\">unclosed",
    ] {
        assert!(parse_bookmark_export(malformed).is_err(), "{malformed}");
    }

    let oversized_url = format!(
        "<!DOCTYPE NETSCAPE-Bookmark-file-1><DL><p><DT><A HREF=\"https://example.test/{}\">too large</A></DL>",
        "x".repeat(8 * 1024)
    );
    assert!(parse_bookmark_export(&oversized_url).is_err());
}

fn limited_library(directory: &std::path::Path, max_file_bytes: u64) -> Library {
    Library::open_with_limits(
        directory.join("limited.sqlite"),
        loom_core::LibraryLimits {
            max_file_bytes,
            ..loom_core::LibraryLimits::default()
        },
    )
    .unwrap()
}

#[test]
fn import_rejects_size_limits_before_parsing() {
    let directory = tempdir().unwrap();
    let export = directory.path().join("Bookmarks.html");
    fs::write(&export, CHROME_EXPORT).unwrap();

    let limited = limited_library(directory.path(), 8);
    let error = limited.import_bookmarks(&export).unwrap_err().to_string();
    assert!(error.contains("8-byte limit"), "{error}");
}

#[test]
fn import_rejects_huge_sparse_file_without_reading_it() {
    // A 64 GiB sparse file cannot be read or allocated within the test's time and memory budget,
    // so a prompt, explicit limit error proves the bound is enforced before the read.
    let directory = tempdir().unwrap();
    let export = directory.path().join("Bookmarks.html");
    let file = fs::File::create(&export).unwrap();
    file.set_len(64 * 1024 * 1024 * 1024).unwrap();
    drop(file);

    let limited = limited_library(directory.path(), 1024);
    let started = std::time::Instant::now();
    let error = limited.import_bookmarks(&export).unwrap_err().to_string();
    assert!(error.contains("1024-byte limit"), "{error}");
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
}

#[test]
fn import_accepts_exact_limit_and_rejects_one_byte_over() {
    let directory = tempdir().unwrap();
    let export = directory.path().join("Bookmarks.html");
    fs::write(&export, CHROME_EXPORT).unwrap();
    let exact = CHROME_EXPORT.len() as u64;

    let at_limit = limited_library(directory.path(), exact);
    let report = at_limit.import_bookmarks(&export).unwrap();
    assert_eq!(report.discovered, 1);

    let mut over = CHROME_EXPORT.as_bytes().to_vec();
    over.push(b'\n');
    fs::write(&export, over).unwrap();
    let error = at_limit.import_bookmarks(&export).unwrap_err().to_string();
    assert!(error.contains("-byte limit"), "{error}");
}

#[test]
fn import_rejects_invalid_utf8_empty_and_directory_inputs() {
    let directory = tempdir().unwrap();
    let library = Library::open_in_memory().unwrap();

    let invalid = directory.path().join("invalid.html");
    fs::write(&invalid, [0xff, 0xfe, 0x00, 0xc3]).unwrap();
    let error = library.import_bookmarks(&invalid).unwrap_err().to_string();
    assert!(error.contains("not UTF-8"), "{error}");

    let empty = directory.path().join("empty.html");
    fs::write(&empty, b"").unwrap();
    assert!(library.import_bookmarks(&empty).is_err());

    let folder = directory.path().join("folder.html");
    fs::create_dir(&folder).unwrap();
    assert!(library.import_bookmarks(&folder).is_err());
}

#[cfg(unix)]
#[test]
fn import_rejects_symlinks_before_parsing() {
    use std::os::unix::fs::symlink;

    let directory = tempdir().unwrap();
    let export = directory.path().join("Bookmarks.html");
    fs::write(&export, CHROME_EXPORT).unwrap();
    let link = directory.path().join("Bookmarks-link.html");
    symlink(&export, &link).unwrap();
    let library = Library::open_in_memory().unwrap();
    assert!(library.import_bookmarks(&link).is_err());
}

const FIREFOX_REALISTIC: &str = include_str!("fixtures/bookmarks/firefox-realistic.html");
const MIXED_FAILURES: &str = include_str!("fixtures/bookmarks/mixed-failures.html");
const MIXED_FIXED: &str = include_str!("fixtures/bookmarks/mixed-fixed.html");

fn relationship_count(library: &Library) -> u64 {
    library
        .export_portable()
        .unwrap()
        .tables
        .get("relationships")
        .map_or(0, |table| table.rows.len() as u64)
}

#[test]
fn imports_record_source_application_version_permissions_and_skipped_fields() {
    let directory = tempdir().unwrap();
    let firefox = directory.path().join("firefox.html");
    let chrome = directory.path().join("chrome.html");
    fs::write(&firefox, FIREFOX_REALISTIC).unwrap();
    fs::write(&chrome, CHROME_EXPORT).unwrap();
    let library = Library::open_in_memory().unwrap();
    library.import_bookmarks(&firefox).unwrap();
    library.import_bookmarks(&chrome).unwrap();

    let imports = library.list_bookmark_imports(10).unwrap();
    let firefox_import = imports
        .iter()
        .find(|import| import.source_uri.ends_with("firefox.html"))
        .unwrap();
    assert_eq!(firefox_import.source_application, "firefox");
    assert_eq!(firefox_import.export_version, "netscape-bookmark-file-1");
    assert_eq!(firefox_import.permissions, vec!["read_selected_file"]);
    assert_eq!(
        firefox_import.skipped_fields,
        vec![
            "icon",
            "icon_uri",
            "last_charset",
            "personal_toolbar_folder",
            "shortcuturl",
            "tags"
        ]
    );
    assert_eq!(firefox_import.status, "complete");
    assert_eq!(firefox_import.items, 2);
    assert!(firefox_import.failures.is_empty());
    assert!(firefox_import.source_uri.starts_with('/'));

    let chrome_import = imports
        .iter()
        .find(|import| import.source_uri.ends_with("chrome.html"))
        .unwrap();
    assert_eq!(chrome_import.source_application, "netscape_compatible");
    assert_eq!(chrome_import.skipped_fields, Vec::<String>::new());
}

#[test]
fn replaying_the_same_export_keeps_artifact_identity_and_adds_no_relationships() {
    let directory = tempdir().unwrap();
    let export = directory.path().join("Bookmarks.html");
    fs::write(&export, FIREFOX_REALISTIC).unwrap();
    let library = Library::open_in_memory().unwrap();

    let first = library.import_bookmarks(&export).unwrap();
    let records = library.list_bookmarks(10).unwrap();
    let stats = library.stats().unwrap();
    let relationships = relationship_count(&library);

    let replay = library.import_bookmarks(&export).unwrap();
    assert_eq!(replay.import_id, first.import_id);
    assert_eq!(replay.unchanged, 2);
    assert_eq!(library.list_bookmarks(10).unwrap(), records);
    assert_eq!(library.stats().unwrap(), stats);
    assert_eq!(relationship_count(&library), relationships);
    assert_eq!(library.list_bookmark_imports(10).unwrap().len(), 1);

    // Replaying the same export into a second library yields the same records and hashes.
    let other = Library::open_in_memory().unwrap();
    other.import_bookmarks(&export).unwrap();
    let project = |library: &Library| {
        library
            .list_bookmarks(10)
            .unwrap()
            .into_iter()
            .map(|record| (record.url, record.folder_path, record.entry_hash))
            .collect::<Vec<_>>()
    };
    assert_eq!(project(&other), project(&library));
}

#[test]
fn malformed_records_become_inspectable_failures_without_blocking_good_records() {
    let directory = tempdir().unwrap();
    let export = directory.path().join("Bookmarks.html");
    fs::write(&export, MIXED_FAILURES).unwrap();
    let library = Library::open_in_memory().unwrap();

    let report = library.import_bookmarks(&export).unwrap();
    assert_eq!(report.imported, 2);
    assert_eq!(report.failed, 3);
    assert_eq!(report.remote_fetches, 0);

    let import = &library.list_bookmark_imports(10).unwrap()[0];
    assert_eq!(import.status, "partial");
    let codes = import
        .failures
        .iter()
        .map(|failure| {
            (
                failure.ordinal,
                failure.code.as_str(),
                failure.state.as_str(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        codes,
        vec![
            (1, "invalid_url", "pending"),
            (2, "missing_href", "pending"),
            (3, "empty_title", "pending")
        ]
    );
    let serialized = serde_json::to_string(&library.list_bookmark_imports(10).unwrap()).unwrap();
    assert!(!serialized.contains("secret-token-123"));
    assert!(!serialized.contains("javascript:"));
    let titles = library
        .list_bookmarks(10)
        .unwrap()
        .into_iter()
        .map(|record| record.title)
        .collect::<Vec<_>>();
    assert_eq!(titles, vec!["Good record one", "Good record two"]);

    // Retrying the unchanged export replays the same import and keeps the failures pending.
    let retry = library.retry_bookmark_import(&report.import_id).unwrap();
    assert_eq!(retry.import_id, report.import_id);
    assert_eq!(retry.failed, 3);

    // Fixing the export and retrying resolves every earlier failure and keeps existing identity.
    let before = library.list_bookmarks(10).unwrap();
    fs::write(&export, MIXED_FIXED).unwrap();
    let fixed = library.retry_bookmark_import(&report.import_id).unwrap();
    assert_ne!(fixed.import_id, report.import_id);
    assert_eq!(fixed.failed, 0);
    assert_eq!(fixed.unchanged, 2);
    let imports = library.list_bookmark_imports(10).unwrap();
    let original = imports
        .iter()
        .find(|import| import.import_id == report.import_id)
        .unwrap();
    assert!(original
        .failures
        .iter()
        .all(|failure| failure.state == "resolved"
            && failure.resolved_by_import_id.as_deref() == Some(fixed.import_id.as_str())));
    let after = library.list_bookmarks(10).unwrap();
    for record in &before {
        assert!(after
            .iter()
            .any(|kept| kept.id == record.id && kept.artifact_id == record.artifact_id));
    }
    assert_eq!(after.len(), 4);
}

#[test]
fn revoked_exports_stay_inspectable_and_retry_requires_reselection() {
    let directory = tempdir().unwrap();
    let export = directory.path().join("Bookmarks.html");
    fs::write(&export, MIXED_FAILURES).unwrap();
    let library = Library::open_in_memory().unwrap();
    let report = library.import_bookmarks(&export).unwrap();

    library.revoke_source_root(&report.source_uri).unwrap();
    let import = &library.list_bookmark_imports(10).unwrap()[0];
    assert_eq!(import.status, "revoked");
    assert_eq!(import.failures.len(), 3);
    assert!(library.retry_bookmark_import(&report.import_id).is_err());
    assert!(library
        .search(&loom_core::SearchRequest {
            text: "Good record".into(),
            limit: 5,
        })
        .unwrap()
        .is_empty());

    // Explicitly selecting the export again restores the same import and records.
    let reselected = library.import_bookmarks(&export).unwrap();
    assert_eq!(reselected.import_id, report.import_id);
    assert_eq!(
        library.list_bookmark_imports(10).unwrap()[0].status,
        "partial"
    );
    assert_eq!(
        library
            .search(&loom_core::SearchRequest {
                text: "Good record".into(),
                limit: 5,
            })
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn structurally_broken_or_empty_exports_still_fail_closed() {
    let directory = tempdir().unwrap();
    let library = Library::open_in_memory().unwrap();
    for (name, body) in [
        (
            "not-netscape.html",
            "<DL><p><DT><A HREF=\"https://example.test\">x</A></DL>",
        ),
        (
            "unterminated.html",
            "<!DOCTYPE NETSCAPE-Bookmark-file-1><DL><p><DT><A HREF=\"https://example.test",
        ),
        (
            "empty.html",
            "<!DOCTYPE NETSCAPE-Bookmark-file-1><DL><p></DL>",
        ),
    ] {
        let path = directory.path().join(name);
        fs::write(&path, body).unwrap();
        assert!(library.import_bookmarks(&path).is_err(), "{name}");
    }
    assert!(library.list_bookmark_imports(10).unwrap().is_empty());
}

const SHARED_A: &str = r#"<!DOCTYPE NETSCAPE-Bookmark-file-1>
<DL><p>
<DT><H3>Shared</H3>
<DL><p>
<DT><A HREF="https://example.test/shared" ADD_DATE="1700000001">Shared reference</A>
<DT><A HREF="https://example.test/only-a" ADD_DATE="1700000002">Only in export A</A>
</DL><p>
</DL><p>
"#;

const SHARED_B: &str = r#"<!DOCTYPE NETSCAPE-Bookmark-file-1>
<DL><p>
<DT><H3>Shared</H3>
<DL><p>
<DT><A HREF="https://example.test/only-b" ADD_DATE="1700000003">Only in export B</A>
<DT><A HREF="https://example.test/shared" ADD_DATE="1700000001">Shared reference</A>
</DL><p>
</DL><p>
"#;

#[test]
fn overlapping_exports_import_everything_except_the_entry_another_export_owns() {
    let directory = tempdir().unwrap();
    let a = directory.path().join("chrome.html");
    let b = directory.path().join("firefox.html");
    fs::write(&a, SHARED_A).unwrap();
    fs::write(&b, SHARED_B).unwrap();
    let library = Library::open(directory.path().join("library.sqlite3")).unwrap();

    let first = library.import_bookmarks(&a).unwrap();
    assert_eq!((first.imported, first.failed), (2, 0));

    // The second export shares one URL with the first: it imports its own entry and records the
    // overlap as a per-record failure instead of rolling back the whole import.
    let second = library.import_bookmarks(&b).unwrap();
    assert_eq!((second.imported, second.failed), (1, 1));
    assert!(second.failures[0].reason.contains("owned_by_other_export"));
    let imports = library.list_bookmark_imports(10).unwrap();
    let b_import = imports
        .iter()
        .find(|import| import.import_id == second.import_id)
        .unwrap();
    assert_eq!(b_import.status, "partial");
    assert_eq!(b_import.failures[0].code, "owned_by_other_export");
    assert_eq!(
        b_import.failures[0].ordinal, 1,
        "parser record numbering is kept"
    );
    let mut titles = library
        .list_bookmarks(10)
        .unwrap()
        .into_iter()
        .map(|record| record.title)
        .collect::<Vec<_>>();
    titles.sort();
    assert_eq!(
        titles,
        vec!["Only in export A", "Only in export B", "Shared reference"]
    );

    // Revoking export A leaves export B's own record searchable.
    library.revoke_source_root(&first.source_uri).unwrap();
    let hits = library
        .search(&loom_core::SearchRequest {
            text: "Only in export B".into(),
            limit: 5,
        })
        .unwrap();
    assert_eq!(hits.len(), 1);

    // Once export A is purged, re-selecting the unchanged export B imports the blocked entry into
    // the same import and resolves its failure.
    library.purge_root(&first.source_uri).unwrap();
    let retried = library.import_bookmarks(&b).unwrap();
    assert_eq!(retried.import_id, second.import_id);
    assert_eq!((retried.imported, retried.failed), (1, 0));
    let b_import = library
        .list_bookmark_imports(10)
        .unwrap()
        .into_iter()
        .find(|import| import.import_id == second.import_id)
        .unwrap();
    assert_eq!(b_import.status, "complete");
    assert_eq!(b_import.failures[0].state, "resolved");
    assert_eq!(
        library
            .search(&loom_core::SearchRequest {
                text: "Shared reference".into(),
                limit: 5,
            })
            .unwrap()
            .len(),
        1
    );
}
