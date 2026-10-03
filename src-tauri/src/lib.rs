#[cfg(target_os = "macos")]
use std::process::Command;
use std::{
    fs,
    io::Cursor,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use blake3::Hash;
use chrono::Utc;
use loom_core::{
    ArtifactVersionHistory, BookmarkImportReport, CaptureBounds, CaptureContext, CaptureMode,
    CapturePurgeReport, CaptureReport, DeletionReport, IndexCancellationToken, IndexReport,
    Library, LibraryStats, ObservationReport, OcrPurgeReport, OcrStatus, OpenArtifactRequest,
    RelationshipView, ResolveEvidenceRequest, RetentionPolicy, RetentionReport, SearchHit,
    SearchRequest, SourceRootInfo, StorageInspection,
};
use serde::{Deserialize, Serialize};
use tauri::{Manager, State};
use tauri_plugin_dialog::DialogExt;

mod capture_policy;

use capture_policy::LoadedPolicy;

struct AppState {
    library: Arc<Library>,
    active_index: Mutex<Option<IndexCancellationToken>>,
    capture_root: PathBuf,
    capture_policy: Mutex<LoadedPolicy>,
}

type CommandResult<T> = std::result::Result<T, String>;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CapturePolicyStatus {
    paused: bool,
    excluded_apps: Vec<String>,
    capture_root: String,
    policy_error: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct CaptureRequest {
    mode: CaptureMode,
    #[serde(default = "default_display_scale")]
    display_scale_milli: u32,
    #[serde(default)]
    bounds: CaptureBounds,
    #[serde(default)]
    app_name: Option<String>,
    #[serde(default)]
    window_title: Option<String>,
}

const fn default_display_scale() -> u32 {
    1_000
}

#[tauri::command]
async fn index_selected_folder(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> CommandResult<Option<IndexReport>> {
    let selected = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_title("Choose a folder for LOOM to index")
            .blocking_pick_folder()
    })
    .await
    .map_err(|error| format!("folder picker stopped: {error}"))?;
    let Some(selected) = selected else {
        return Ok(None);
    };
    let path = selected
        .into_path()
        .map_err(|error| format!("could not read selected folder: {error}"))?;
    let cancellation = IndexCancellationToken::new();
    {
        let mut active = state
            .active_index
            .lock()
            .map_err(|_| "index state lock is unavailable".to_string())?;
        if active.is_some() {
            return Err("an indexing run is already active".into());
        }
        *active = Some(cancellation.clone());
    }
    let library = Arc::clone(&state.library);
    let result = tauri::async_runtime::spawn_blocking(move || {
        library.index_path_with_cancellation(path, &cancellation)
    })
    .await;
    state
        .active_index
        .lock()
        .map_err(|_| "index state lock is unavailable".to_string())?
        .take();
    result
        .map_err(|error| format!("index worker stopped: {error}"))?
        .map(Some)
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn import_bookmarks(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> CommandResult<Option<BookmarkImportReport>> {
    let selected = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_title("Choose a Chrome or Firefox bookmark export")
            .add_filter("Bookmark HTML", &["html", "htm"])
            .blocking_pick_file()
    })
    .await
    .map_err(|error| format!("bookmark picker stopped: {error}"))?;
    let Some(selected) = selected else {
        return Ok(None);
    };
    let path = selected
        .into_path()
        .map_err(|error| format!("could not read selected bookmark export: {error}"))?;
    let library = Arc::clone(&state.library);
    tauri::async_runtime::spawn_blocking(move || library.import_bookmarks(path))
        .await
        .map_err(|error| format!("bookmark import worker stopped: {error}"))?
        .map(Some)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn cancel_indexing(state: State<'_, AppState>) -> CommandResult<bool> {
    let active = state
        .active_index
        .lock()
        .map_err(|_| "index state lock is unavailable".to_string())?;
    if let Some(token) = active.as_ref() {
        token.cancel();
        Ok(true)
    } else {
        Ok(false)
    }
}

#[tauri::command]
fn capture_status(state: State<'_, AppState>) -> CommandResult<CapturePolicyStatus> {
    let policy = state
        .capture_policy
        .lock()
        .map_err(|_| "capture policy lock is unavailable".to_string())?;
    Ok(capture_policy_status(&policy, &state.capture_root))
}

#[tauri::command]
fn set_capture_paused(
    state: State<'_, AppState>,
    paused: bool,
) -> CommandResult<CapturePolicyStatus> {
    let mut policy = state
        .capture_policy
        .lock()
        .map_err(|_| "capture policy lock is unavailable".to_string())?;
    policy.set_paused(&capture_policy::policy_path(&state.capture_root), paused)?;
    Ok(capture_policy_status(&policy, &state.capture_root))
}

#[tauri::command]
fn set_capture_exclusions(
    state: State<'_, AppState>,
    excluded_apps: Vec<String>,
) -> CommandResult<CapturePolicyStatus> {
    let mut policy = state
        .capture_policy
        .lock()
        .map_err(|_| "capture policy lock is unavailable".to_string())?;
    policy.set_exclusions(
        &capture_policy::policy_path(&state.capture_root),
        excluded_apps,
    )?;
    Ok(capture_policy_status(&policy, &state.capture_root))
}

#[tauri::command]
async fn capture_intentional(
    state: State<'_, AppState>,
    request: CaptureRequest,
) -> CommandResult<CaptureReport> {
    let context = capture_context(&request);
    {
        let loaded = state
            .capture_policy
            .lock()
            .map_err(|_| "capture policy lock is unavailable".to_string())?;
        let policy = &loaded.policy;
        if loaded.error.is_some() || policy.paused {
            return Ok(skipped_capture_report("paused", context));
        }
        let app_name = request
            .app_name
            .as_deref()
            .map(|app| app.trim().to_ascii_lowercase());
        if app_name
            .as_deref()
            .is_some_and(|app| policy.excluded_apps.iter().any(|excluded| excluded == app))
        {
            return Ok(skipped_capture_report("excluded_app", context));
        }
    }
    let root = state.capture_root.clone();
    let library = Arc::clone(&state.library);
    tauri::async_runtime::spawn_blocking(move || capture_native_image(&root, &library, &request))
        .await
        .map_err(|error| format!("capture worker stopped: {error}"))?
}

#[tauri::command]
fn purge_captures(state: State<'_, AppState>) -> CommandResult<CapturePurgeReport> {
    let root = &state.capture_root;
    let mut report = CapturePurgeReport::default();
    if !root.exists() {
        return Ok(report);
    }
    for entry in fs::read_dir(root).map_err(|error| format!("could not list captures: {error}"))? {
        let path = entry
            .map_err(|error| format!("could not read capture entry: {error}"))?
            .path();
        if path.extension().and_then(|value| value.to_str()) != Some("png") {
            continue;
        }
        let locator = path
            .canonicalize()
            .map_err(|error| format!("could not resolve capture: {error}"))?
            .to_string_lossy()
            .into_owned();
        let deleted = state
            .library
            .purge_source_root(&locator)
            .map_err(|error| error.to_string())?;
        report.artifacts_deleted += deleted.artifacts_deleted;
        report.versions_deleted += deleted.versions_deleted;
        report.passages_deleted += deleted.passages_deleted;
        fs::remove_file(&path).map_err(|error| format!("could not purge capture: {error}"))?;
    }
    Ok(report)
}

#[tauri::command]
async fn reconcile_approved_roots(state: State<'_, AppState>) -> CommandResult<ObservationReport> {
    let library = Arc::clone(&state.library);
    tauri::async_runtime::spawn_blocking(move || library.reconcile_approved_roots())
        .await
        .map_err(|error| format!("observation worker stopped: {error}"))?
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn list_source_roots(state: State<'_, AppState>) -> CommandResult<Vec<SourceRootInfo>> {
    state
        .library
        .source_roots()
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn list_relationships(
    state: State<'_, AppState>,
    artifact_id: String,
) -> CommandResult<Vec<RelationshipView>> {
    state
        .library
        .list_relationships(&artifact_id, 50)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn revoke_source_root(
    state: State<'_, AppState>,
    locator: String,
) -> CommandResult<SourceRootInfo> {
    state
        .library
        .revoke_source_root(&locator)
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn search(
    state: State<'_, AppState>,
    request: SearchRequest,
) -> CommandResult<Vec<SearchHit>> {
    let library = Arc::clone(&state.library);
    tauri::async_runtime::spawn_blocking(move || library.search(&request))
        .await
        .map_err(|error| format!("search worker stopped: {error}"))?
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn library_stats(state: State<'_, AppState>) -> CommandResult<LibraryStats> {
    state.library.stats().map_err(|error| error.to_string())
}

#[tauri::command]
fn storage_inspection(state: State<'_, AppState>) -> CommandResult<StorageInspection> {
    state
        .library
        .inspect_storage()
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn purge_artifact(
    state: State<'_, AppState>,
    artifact_id: String,
) -> CommandResult<DeletionReport> {
    state
        .library
        .purge_artifact(&artifact_id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn purge_root(state: State<'_, AppState>, locator: String) -> CommandResult<DeletionReport> {
    state
        .library
        .purge_root(&locator)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn retention_status(state: State<'_, AppState>) -> CommandResult<RetentionPolicy> {
    state
        .library
        .retention_policy()
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn set_retention_days(
    state: State<'_, AppState>,
    days: Option<u32>,
) -> CommandResult<RetentionPolicy> {
    state
        .library
        .set_retention_days(days)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn apply_retention(state: State<'_, AppState>) -> CommandResult<RetentionReport> {
    state
        .library
        .apply_retention()
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn purge_disposable_storage(state: State<'_, AppState>) -> CommandResult<DeletionReport> {
    state
        .library
        .purge_disposable_storage()
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn ocr_status(state: State<'_, AppState>) -> CommandResult<OcrStatus> {
    state
        .library
        .ocr_status()
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn set_ocr_enabled(state: State<'_, AppState>, enabled: bool) -> CommandResult<OcrPurgeReport> {
    state
        .library
        .set_ocr_enabled(enabled)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn purge_ocr_records(state: State<'_, AppState>) -> CommandResult<OcrPurgeReport> {
    state
        .library
        .purge_ocr_records()
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn open_artifact(
    state: State<'_, AppState>,
    request: OpenArtifactRequest,
) -> CommandResult<()> {
    let library = Arc::clone(&state.library);
    tauri::async_runtime::spawn_blocking(move || {
        let path = library
            .resolve_verified_artifact_path(
                &request.artifact_id,
                &request.version_id,
                &request.content_hash,
            )
            .map_err(|error| error.to_string())?;
        opener::open(path).map_err(|error| format!("could not open original source: {error}"))
    })
    .await
    .map_err(|error| format!("source opener stopped: {error}"))?
}

#[tauri::command]
async fn resolve_evidence(
    state: State<'_, AppState>,
    request: ResolveEvidenceRequest,
) -> CommandResult<loom_core::EvidenceView> {
    let library = Arc::clone(&state.library);
    tauri::async_runtime::spawn_blocking(move || library.resolve_verified_evidence(&request))
        .await
        .map_err(|error| format!("evidence resolver stopped: {error}"))?
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn artifact_version_history(
    state: State<'_, AppState>,
    artifact_id: String,
) -> CommandResult<ArtifactVersionHistory> {
    let library = Arc::clone(&state.library);
    tauri::async_runtime::spawn_blocking(move || library.artifact_version_history(&artifact_id, 20))
        .await
        .map_err(|error| format!("version inspector stopped: {error}"))?
        .map_err(|error| error.to_string())
}

fn capture_policy_status(loaded: &LoadedPolicy, capture_root: &Path) -> CapturePolicyStatus {
    CapturePolicyStatus {
        paused: loaded.policy.paused,
        excluded_apps: loaded.policy.excluded_apps.clone(),
        capture_root: capture_root.to_string_lossy().into_owned(),
        policy_error: loaded.error.clone(),
    }
}

fn capture_context(request: &CaptureRequest) -> CaptureContext {
    CaptureContext {
        mode: request.mode.clone(),
        captured_at: Utc::now().to_rfc3339(),
        display_scale_milli: request.display_scale_milli.clamp(500, 4_000),
        bounds: request.bounds,
        app_name: request.app_name.clone(),
        window_title: request.window_title.clone(),
        source: "macOS screencapture".into(),
    }
}

fn skipped_capture_report(status: &str, context: CaptureContext) -> CaptureReport {
    CaptureReport {
        status: status.into(),
        source_uri: String::new(),
        content_hash: String::new(),
        byte_size: 0,
        duplicate: false,
        context,
    }
}

struct CommittedCapture {
    destination: PathBuf,
    content_hash: String,
    byte_size: u64,
    width: u32,
    height: u32,
    duplicate: bool,
}

/// Validates screencapture output and moves it into content-addressed capture storage.
///
/// Nothing is committed until the bytes are read, non-empty, and decode as an image; every failure
/// before commit removes the temporary file, so capture storage never holds unvalidated pixels.
fn commit_capture(capture_root: &Path, temporary: &Path) -> CommandResult<CommittedCapture> {
    let validated = fs::read(temporary)
        .map_err(|error| format!("capture output could not be read: {error}"))
        .and_then(|bytes| {
            if bytes.is_empty() {
                return Err("capture_cancelled_or_denied: no pixels were returned".to_string());
            }
            let (width, height) = image::ImageReader::new(Cursor::new(&bytes))
                .with_guessed_format()
                .map_err(|error| format!("capture image format is invalid: {error}"))?
                .into_dimensions()
                .map_err(|error| format!("capture image dimensions are invalid: {error}"))?;
            Ok((bytes, width, height))
        });
    let (bytes, width, height) = match validated {
        Ok(validated) => validated,
        Err(error) => {
            let _ = fs::remove_file(temporary);
            return Err(error);
        }
    };
    let digest: Hash = blake3::hash(&bytes);
    let destination = capture_root.join(format!("{digest}.png"));
    let duplicate = destination.exists();
    if duplicate {
        fs::remove_file(temporary)
            .map_err(|error| format!("could not discard duplicate capture: {error}"))?;
    } else if let Err(error) = fs::rename(temporary, &destination) {
        let _ = fs::remove_file(temporary);
        return Err(format!("could not commit capture bytes: {error}"));
    }
    Ok(CommittedCapture {
        destination,
        content_hash: format!("blake3:{digest}"),
        byte_size: bytes.len() as u64,
        width,
        height,
        duplicate,
    })
}

/// Removes a capture this command just committed when it could not be indexed, together with any
/// rows the failed attempt left, so no unindexed pixels remain. A pre-existing duplicate belongs to
/// an earlier capture and is kept.
fn discard_new_capture(library: &Library, destination: &Path, duplicate: bool) {
    if duplicate {
        return;
    }
    if let Ok(locator) = destination.canonicalize() {
        let _ = library.purge_source_root(&locator.to_string_lossy());
    }
    let _ = fs::remove_file(destination);
}

fn capture_native_image(
    capture_root: &Path,
    library: &Library,
    request: &CaptureRequest,
) -> CommandResult<CaptureReport> {
    if !library
        .ocr_status()
        .map_err(|error| error.to_string())?
        .enabled
    {
        return Err(loom_core::LoomError::OcrDisabled.to_string());
    }
    fs::create_dir_all(capture_root)
        .map_err(|error| format!("capture storage is unavailable: {error}"))?;
    let temporary = capture_root.join(format!(".loom-capture-{}.png", uuid::Uuid::new_v4()));
    let status = run_native_capture(&request.mode, &temporary)?;
    if !status.success() {
        let _ = fs::remove_file(&temporary);
        return Err(format!(
            "capture_cancelled_or_denied: macOS Screen Recording permission may be denied; grant LOOM access in System Settings → Privacy & Security → Screen Recording, then retry (exit {})",
            status.code().map_or_else(|| "unknown".into(), |code| code.to_string())
        ));
    }
    let CommittedCapture {
        destination,
        content_hash,
        byte_size,
        width,
        height,
        duplicate,
    } = commit_capture(capture_root, &temporary)?;
    let mut context = capture_context(request);
    context.bounds.width = width;
    context.bounds.height = height;
    let index = match library.index_captured_image(&destination, &context) {
        Ok(index) => index,
        Err(error) => {
            discard_new_capture(library, &destination, duplicate);
            return Err(error.to_string());
        }
    };
    if let Some(failure) = index.failures.first() {
        discard_new_capture(library, &destination, duplicate);
        return Err(format!("capture could not be indexed: {}", failure.reason));
    }
    if index.skipped > 0
        || index.cancelled > 0
        || index.indexed.saturating_add(index.unchanged) != 1
    {
        discard_new_capture(library, &destination, duplicate);
        return Err("capture could not be indexed: one exact image result is required".into());
    }
    Ok(CaptureReport {
        status: if duplicate || index.unchanged > 0 {
            "duplicate".into()
        } else {
            "captured".into()
        },
        source_uri: destination.to_string_lossy().into_owned(),
        content_hash,
        byte_size,
        duplicate: duplicate || index.unchanged > 0,
        context,
    })
}

fn run_native_capture(
    mode: &CaptureMode,
    output: &Path,
) -> CommandResult<std::process::ExitStatus> {
    #[cfg(target_os = "macos")]
    {
        Command::new("/usr/sbin/screencapture")
            .args(native_capture_arguments(mode, output))
            .status()
            .map_err(|error| format!("capture helper could not start: {error}"))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (mode, output);
        Err("capture is only supported on macOS".into())
    }
}

#[cfg(any(target_os = "macos", test))]
fn native_capture_arguments(mode: &CaptureMode, output: &Path) -> Vec<String> {
    let mut arguments = vec!["-x".into(), "-t".into(), "png".into()];
    match mode {
        CaptureMode::Screen => {}
        CaptureMode::Window => arguments.extend(["-i".into(), "-W".into()]),
        CaptureMode::Region => arguments.push("-i".into()),
    }
    arguments.push(output.to_string_lossy().into_owned());
    arguments
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let data_directory = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_directory)?;
            let capture_root = data_directory.join("captures");
            fs::create_dir_all(&capture_root)?;
            let library = Library::open(data_directory.join("library.sqlite3"))
                .map_err(|error| std::io::Error::other(error.to_string()))?;
            app.manage(AppState {
                library: Arc::new(library),
                active_index: Mutex::new(None),
                capture_policy: Mutex::new(capture_policy::load(&capture_policy::policy_path(
                    &capture_root,
                ))),
                capture_root,
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            index_selected_folder,
            import_bookmarks,
            cancel_indexing,
            capture_status,
            set_capture_paused,
            set_capture_exclusions,
            capture_intentional,
            purge_captures,
            reconcile_approved_roots,
            list_source_roots,
            list_relationships,
            revoke_source_root,
            search,
            library_stats,
            storage_inspection,
            purge_artifact,
            purge_root,
            retention_status,
            set_retention_days,
            apply_retention,
            purge_disposable_storage,
            ocr_status,
            set_ocr_enabled,
            purge_ocr_records,
            open_artifact,
            resolve_evidence,
            artifact_version_history
        ])
        .run(tauri::generate_context!())
        .expect("error while running LOOM");
}

#[cfg(test)]
mod tests {
    #[test]
    fn disabled_ocr_refuses_native_capture_before_creating_files_or_running_picker() {
        let directory = tempfile::tempdir().unwrap();
        let capture_root = directory.path().join("captures");
        let library = loom_core::Library::open_in_memory().unwrap();
        library.set_ocr_enabled(false).unwrap();
        let request = serde_json::from_value(serde_json::json!({"mode": "region"})).unwrap();
        assert_eq!(
            super::capture_native_image(&capture_root, &library, &request).unwrap_err(),
            "image OCR is disabled"
        );
        assert!(!capture_root.exists());
        assert!(library.source_roots().unwrap().is_empty());
    }

    #[test]
    fn version_history_permission_defines_the_registered_command() {
        let permission = include_str!("../permissions/autogenerated/artifact_version_history.toml");
        assert!(permission.contains("identifier = \"allow-artifact-version-history\""));
        assert!(permission.contains("commands.allow = [\"artifact_version_history\"]"));
        assert!(permission.contains("commands.deny = [\"artifact_version_history\"]"));
    }

    use std::{fs, path::Path};

    use loom_core::Library;

    use super::CaptureMode;

    const CAPABILITIES: &str = include_str!("../capabilities/default.json");
    const TAURI_CONFIG: &str = include_str!("../tauri.conf.json");

    #[test]
    fn desktop_contract_stays_local_and_command_scoped() {
        for permission in [
            "allow-fs-",
            "allow-shell-",
            "allow-http-",
            "allow-process-",
            "allow-notification-",
        ] {
            assert!(
                !CAPABILITIES.contains(permission),
                "unexpected broad permission namespace: {permission}"
            );
        }
        for command in [
            "allow-index-selected-folder",
            "allow-cancel-indexing",
            "allow-capture-status",
            "allow-set-capture-paused",
            "allow-set-capture-exclusions",
            "allow-capture-intentional",
            "allow-purge-captures",
            "allow-reconcile-approved-roots",
            "allow-list-source-roots",
            "allow-revoke-source-root",
            "allow-search",
            "allow-library-stats",
            "allow-storage-inspection",
            "allow-purge-artifact",
            "allow-purge-root",
            "allow-retention-status",
            "allow-set-retention-days",
            "allow-apply-retention",
            "allow-purge-disposable-storage",
            "allow-ocr-status",
            "allow-set-ocr-enabled",
            "allow-purge-ocr-records",
            "allow-open-artifact",
            "allow-resolve-evidence",
            "allow-artifact-version-history",
        ] {
            assert!(
                CAPABILITIES.contains(command),
                "missing command permission: {command}"
            );
        }
        assert!(TAURI_CONFIG.contains("\"connect-src\": \"ipc: http://ipc.localhost\""));
        assert!(!TAURI_CONFIG.contains("\"connect-src\": \"ipc: http://ipc.localhost https:"));
        assert!(TAURI_CONFIG.contains("\"frontendDist\": \"../dist\""));
    }

    fn png_files(root: &Path) -> Vec<String> {
        let mut names = fs::read_dir(root)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".png"))
            .collect::<Vec<_>>();
        names.sort();
        names
    }

    fn write_png(path: &Path) {
        image::RgbImage::from_pixel(3, 2, image::Rgb([10, 20, 30]))
            .save_with_format(path, image::ImageFormat::Png)
            .unwrap();
    }

    #[test]
    fn invalid_or_empty_capture_output_is_never_committed() {
        let root = tempfile::tempdir().unwrap();
        for bytes in [&b"not an image"[..], &b""[..]] {
            let temporary = root.path().join(".loom-capture-test.png");
            fs::write(&temporary, bytes).unwrap();
            assert!(super::commit_capture(root.path(), &temporary).is_err());
            assert!(png_files(root.path()).is_empty());
        }
        let missing = root.path().join(".loom-capture-missing.png");
        assert!(super::commit_capture(root.path(), &missing).is_err());
    }

    #[test]
    fn valid_capture_commits_once_and_duplicates_keep_the_original() {
        let root = tempfile::tempdir().unwrap();
        let first = root.path().join(".loom-capture-first.png");
        write_png(&first);
        let committed = super::commit_capture(root.path(), &first).unwrap();
        assert!(!committed.duplicate);
        assert_eq!((committed.width, committed.height), (3, 2));
        assert!(committed.destination.is_file());
        assert!(!first.exists());
        let expected = committed
            .content_hash
            .strip_prefix("blake3:")
            .unwrap()
            .to_string()
            + ".png";
        assert_eq!(png_files(root.path()), vec![expected.clone()]);

        let second = root.path().join(".loom-capture-second.png");
        write_png(&second);
        let duplicate = super::commit_capture(root.path(), &second).unwrap();
        assert!(duplicate.duplicate);
        assert_eq!(duplicate.destination, committed.destination);
        assert_eq!(png_files(root.path()), vec![expected]);

        let library = Library::open_in_memory().unwrap();
        super::discard_new_capture(&library, &duplicate.destination, true);
        assert!(committed.destination.is_file());
        super::discard_new_capture(&library, &committed.destination, false);
        assert!(png_files(root.path()).is_empty());
    }

    #[test]
    fn capture_policy_and_modes_are_explicit_and_bounded() {
        assert_eq!(
            super::capture_policy::normalize_exclusions(vec![
                " Safari ".into(),
                "safari".into(),
                "".into()
            ]),
            vec!["safari"]
        );
        assert_eq!(
            super::native_capture_arguments(&CaptureMode::Screen, Path::new("/tmp/a.png")),
            vec!["-x", "-t", "png", "/tmp/a.png"]
        );
        assert_eq!(
            super::native_capture_arguments(&CaptureMode::Window, Path::new("/tmp/a.png")),
            vec!["-x", "-t", "png", "-i", "-W", "/tmp/a.png"]
        );
        assert_eq!(
            super::native_capture_arguments(&CaptureMode::Region, Path::new("/tmp/a.png")),
            vec!["-x", "-t", "png", "-i", "/tmp/a.png"]
        );
    }
}
