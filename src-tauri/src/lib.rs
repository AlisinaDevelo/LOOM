use std::{
    fs,
    io::Cursor,
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Mutex},
};

use blake3::Hash;
use chrono::Utc;
use loom_core::{
    BookmarkImportReport, CaptureBounds, CaptureContext, CaptureMode, CapturePurgeReport,
    CaptureReport, IndexCancellationToken, IndexReport, Library, LibraryStats, ObservationReport,
    OcrPurgeReport, OcrStatus, OpenArtifactRequest, RelationshipView, ResolveEvidenceRequest,
    SearchHit, SearchRequest, SourceRootInfo,
};
use serde::{Deserialize, Serialize};
use tauri::{Manager, State};
use tauri_plugin_dialog::DialogExt;

struct AppState {
    library: Arc<Library>,
    active_index: Mutex<Option<IndexCancellationToken>>,
    capture_root: PathBuf,
    capture_policy: Mutex<CapturePolicy>,
}

type CommandResult<T> = std::result::Result<T, String>;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct CapturePolicy {
    paused: bool,
    excluded_apps: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CapturePolicyStatus {
    paused: bool,
    excluded_apps: Vec<String>,
    capture_root: String,
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
    policy.paused = paused;
    save_capture_policy(&state.capture_root, &policy)?;
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
    policy.excluded_apps = normalize_exclusions(excluded_apps);
    save_capture_policy(&state.capture_root, &policy)?;
    Ok(capture_policy_status(&policy, &state.capture_root))
}

#[tauri::command]
async fn capture_intentional(
    state: State<'_, AppState>,
    request: CaptureRequest,
) -> CommandResult<CaptureReport> {
    let context = capture_context(&request);
    {
        let policy = state
            .capture_policy
            .lock()
            .map_err(|_| "capture policy lock is unavailable".to_string())?;
        if policy.paused {
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

fn capture_policy_status(policy: &CapturePolicy, capture_root: &Path) -> CapturePolicyStatus {
    CapturePolicyStatus {
        paused: policy.paused,
        excluded_apps: policy.excluded_apps.clone(),
        capture_root: capture_root.to_string_lossy().into_owned(),
    }
}

fn normalize_exclusions(excluded_apps: Vec<String>) -> Vec<String> {
    let mut values = excluded_apps
        .into_iter()
        .map(|app| app.trim().to_ascii_lowercase())
        .filter(|app| !app.is_empty())
        .collect::<Vec<_>>();
    values.sort();
    values.dedup();
    values
}

fn capture_policy_path(capture_root: &Path) -> PathBuf {
    capture_root
        .parent()
        .unwrap_or(capture_root)
        .join("capture-policy.json")
}

fn load_capture_policy(capture_root: &Path) -> CapturePolicy {
    fs::read(capture_policy_path(capture_root))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn save_capture_policy(capture_root: &Path, policy: &CapturePolicy) -> CommandResult<()> {
    let path = capture_policy_path(capture_root);
    let bytes = serde_json::to_vec_pretty(policy).map_err(|error| error.to_string())?;
    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, bytes)
        .map_err(|error| format!("could not save capture policy: {error}"))?;
    fs::rename(&temporary, &path)
        .map_err(|error| format!("could not commit capture policy: {error}"))
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

fn capture_native_image(
    capture_root: &Path,
    library: &Library,
    request: &CaptureRequest,
) -> CommandResult<CaptureReport> {
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
    let bytes = fs::read(&temporary)
        .map_err(|error| format!("capture output could not be read: {error}"))?;
    if bytes.is_empty() {
        let _ = fs::remove_file(&temporary);
        return Err("capture_cancelled_or_denied: no pixels were returned".into());
    }
    let digest: Hash = blake3::hash(&bytes);
    let content_hash = format!("blake3:{digest}");
    let destination = capture_root.join(format!("{digest}.png"));
    let duplicate = destination.exists();
    if duplicate {
        fs::remove_file(&temporary)
            .map_err(|error| format!("could not discard duplicate capture: {error}"))?;
    } else {
        fs::rename(&temporary, &destination)
            .map_err(|error| format!("could not commit capture bytes: {error}"))?;
    }
    let (width, height) = image::ImageReader::new(Cursor::new(&bytes))
        .with_guessed_format()
        .map_err(|error| format!("capture image format is invalid: {error}"))?
        .into_dimensions()
        .map_err(|error| format!("capture image dimensions are invalid: {error}"))?;
    let mut context = capture_context(request);
    context.bounds.width = width;
    context.bounds.height = height;
    let index = library
        .index_captured_image(&destination, &context)
        .map_err(|error| error.to_string())?;
    if let Some(failure) = index.failures.first() {
        return Err(format!(
            "capture was stored but indexing failed: {}",
            failure.reason
        ));
    }
    Ok(CaptureReport {
        status: if duplicate || index.unchanged > 0 {
            "duplicate".into()
        } else {
            "captured".into()
        },
        source_uri: destination.to_string_lossy().into_owned(),
        content_hash,
        byte_size: bytes.len() as u64,
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
                capture_policy: Mutex::new(load_capture_policy(&capture_root)),
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
            ocr_status,
            set_ocr_enabled,
            purge_ocr_records,
            open_artifact,
            resolve_evidence
        ])
        .run(tauri::generate_context!())
        .expect("error while running LOOM");
}

#[cfg(test)]
mod tests {
    use std::path::Path;

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
            "allow-ocr-status",
            "allow-set-ocr-enabled",
            "allow-purge-ocr-records",
            "allow-open-artifact",
            "allow-resolve-evidence",
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

    #[test]
    fn capture_policy_and_modes_are_explicit_and_bounded() {
        assert_eq!(
            super::normalize_exclusions(vec![" Safari ".into(), "safari".into(), "".into()]),
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
