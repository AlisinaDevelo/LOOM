use std::fs;

use loom_core::{Library, SearchRequest, SourceRootStatus};
use tempfile::tempdir;

#[test]
fn persisted_root_reopens_and_reconciles_only_the_selected_locator() {
    let root = tempdir().unwrap();
    let database = tempdir().unwrap();
    let original = root.path().join("original.md");
    fs::write(&original, "persisted root original marker").unwrap();

    let library = Library::open(database.path().join("library.sqlite3")).unwrap();
    assert_eq!(library.index_path(root.path()).unwrap().indexed, 1);
    let locator = root
        .path()
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert_eq!(
        library.source_roots().unwrap(),
        vec![loom_core::SourceRootInfo {
            locator: locator.clone(),
            kind: "directory".into(),
            enabled: true,
            read_only: true,
            status: SourceRootStatus::Available,
        }]
    );
    drop(library);

    fs::write(
        root.path().join("after-relaunch.md"),
        "relaunch reconciliation marker",
    )
    .unwrap();
    let reopened = Library::open(database.path().join("library.sqlite3")).unwrap();
    let report = reopened.reconcile_approved_roots().unwrap();
    assert_eq!(report.roots_scanned, 1);
    assert_eq!(report.roots_failed, 0);
    assert!(report.failures.is_empty());
    assert_eq!(
        search(&reopened, "relaunch reconciliation marker"),
        1,
        "relaunch must rescan only the persisted root"
    );
    assert_eq!(reopened.source_roots().unwrap()[0].locator, locator);
}

#[test]
fn revocation_hides_existing_evidence_until_explicit_reselection() {
    let root = tempdir().unwrap();
    let database = tempdir().unwrap();
    fs::write(root.path().join("private.md"), "revocation private marker").unwrap();
    let library = Library::open(database.path().join("library.sqlite3")).unwrap();
    library.index_path(root.path()).unwrap();
    let locator = root
        .path()
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .into_owned();

    let revoked = library.revoke_source_root(&locator).unwrap();
    assert!(!revoked.enabled);
    assert!(revoked.read_only);
    assert_eq!(revoked.status, SourceRootStatus::Revoked);
    assert_eq!(search(&library, "revocation private marker"), 0);
    assert_eq!(library.reconcile_approved_roots().unwrap().roots_scanned, 0);

    fs::write(root.path().join("after-revoke.md"), "reselection marker").unwrap();
    assert_eq!(search(&library, "reselection marker"), 0);

    // The only re-enabling path is an explicit user-selected path passed back through indexing.
    library.index_path(root.path()).unwrap();
    let roots = library.source_roots().unwrap();
    assert_eq!(roots.len(), 1);
    assert!(roots[0].enabled);
    assert_eq!(roots[0].status, SourceRootStatus::Available);
    assert_eq!(search(&library, "reselection marker"), 1);
    assert_eq!(search(&library, "revocation private marker"), 1);
}

#[test]
fn moved_root_reports_missing_and_requires_explicit_reselection() {
    let parent = tempdir().unwrap();
    let database = tempdir().unwrap();
    let original_root = parent.path().join("selected");
    let moved_root = parent.path().join("moved");
    fs::create_dir(&original_root).unwrap();
    fs::write(original_root.join("source.md"), "moved root marker").unwrap();

    let library = Library::open(database.path().join("library.sqlite3")).unwrap();
    library.index_path(&original_root).unwrap();
    let original_locator = original_root
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    fs::rename(&original_root, &moved_root).unwrap();

    let missing = library.source_roots().unwrap();
    assert_eq!(missing[0].status, SourceRootStatus::Missing);
    let report = library.reconcile_approved_roots().unwrap();
    assert_eq!(report.roots_scanned, 1);
    assert_eq!(report.roots_failed, 1);
    assert_eq!(report.failures.len(), 1);
    assert!(report.failures[0].source.contains("selected"));

    library.revoke_source_root(&original_locator).unwrap();
    assert_eq!(search(&library, "moved root marker"), 0);
    library.index_path(&moved_root).unwrap();
    let roots = library.source_roots().unwrap();
    assert_eq!(roots.iter().filter(|root| root.enabled).count(), 1);
    assert_eq!(
        roots
            .iter()
            .filter(|root| root.status == SourceRootStatus::Missing)
            .count(),
        0
    );
    assert_eq!(search(&library, "moved root marker"), 1);
}

#[cfg(unix)]
#[test]
fn denied_root_is_visible_and_reconcile_fails_without_fallback() {
    use std::os::unix::fs::PermissionsExt;

    let root = tempdir().unwrap();
    let database = tempdir().unwrap();
    fs::write(root.path().join("denied.md"), "denied root marker").unwrap();
    let library = Library::open(database.path().join("library.sqlite3")).unwrap();
    library.index_path(root.path()).unwrap();

    let mut permissions = fs::metadata(root.path()).unwrap().permissions();
    permissions.set_mode(0o000);
    fs::set_permissions(root.path(), permissions).unwrap();
    let status = library.source_roots().unwrap()[0].status.clone();
    let report = library.reconcile_approved_roots().unwrap();

    let mut restored = fs::metadata(root.path()).unwrap().permissions();
    restored.set_mode(0o700);
    fs::set_permissions(root.path(), restored).unwrap();

    assert_eq!(status, SourceRootStatus::Denied);
    assert_eq!(report.roots_scanned, 1);
    assert_eq!(report.roots_failed, 1);
    assert!(report.failures[0].reason.contains("Denied"));
    assert_eq!(search(&library, "not in a fallback"), 0);
}

#[cfg(unix)]
#[test]
fn symlink_replacement_is_unsafe_and_never_followed_on_reconcile() {
    use std::os::unix::fs::symlink;

    let parent = tempdir().unwrap();
    let database = tempdir().unwrap();
    let selected = parent.path().join("selected");
    let outside = parent.path().join("outside");
    fs::create_dir(&selected).unwrap();
    fs::create_dir(&outside).unwrap();
    fs::write(selected.join("inside.md"), "inside root marker").unwrap();
    fs::write(outside.join("secret.md"), "outside symlink marker").unwrap();

    let library = Library::open(database.path().join("library.sqlite3")).unwrap();
    library.index_path(&selected).unwrap();
    fs::remove_dir_all(&selected).unwrap();
    symlink(&outside, &selected).unwrap();

    assert_eq!(
        library.source_roots().unwrap()[0].status,
        SourceRootStatus::Unsafe
    );
    let report = library.reconcile_approved_roots().unwrap();
    assert_eq!(report.roots_failed, 1);
    assert!(report.failures[0].reason.contains("Unsafe"));
    assert_eq!(search(&library, "outside symlink marker"), 0);
}

fn search(library: &Library, query: &str) -> usize {
    library
        .search(&SearchRequest {
            text: query.into(),
            limit: 10,
        })
        .unwrap()
        .len()
}
