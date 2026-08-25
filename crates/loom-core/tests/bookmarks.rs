use std::fs;

use loom_core::{parse_bookmark_export, Library};
use tempfile::tempdir;

const CHROME_EXPORT: &str = include_str!("fixtures/bookmarks/chrome.html");
const FIREFOX_EXPORT: &str = include_str!("fixtures/bookmarks/firefox.html");

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

#[test]
fn import_rejects_symlinks_and_size_limits_before_parsing() {
    let directory = tempdir().unwrap();
    let export = directory.path().join("Bookmarks.html");
    fs::write(&export, CHROME_EXPORT).unwrap();
    let link = directory.path().join("Bookmarks-link.html");
    std::os::unix::fs::symlink(&export, &link).unwrap();
    let library = Library::open_in_memory().unwrap();
    assert!(library.import_bookmarks(&link).is_err());

    let limited = Library::open_with_limits(
        directory.path().join("limited.sqlite"),
        loom_core::LibraryLimits {
            max_file_bytes: 8,
            ..loom_core::LibraryLimits::default()
        },
    )
    .unwrap();
    assert!(limited.import_bookmarks(&export).is_err());
}
