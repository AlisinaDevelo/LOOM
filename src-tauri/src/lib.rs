use std::sync::{Arc, Mutex};

use loom_core::{
    IndexCancellationToken, IndexReport, Library, LibraryStats, ObservationReport, OcrPurgeReport,
    OcrStatus, OpenArtifactRequest, ResolveEvidenceRequest, SearchHit, SearchRequest,
    SourceRootInfo,
};
use tauri::{Manager, State};
use tauri_plugin_dialog::DialogExt;

struct AppState {
    library: Arc<Library>,
    active_index: Mutex<Option<IndexCancellationToken>>,
}

type CommandResult<T> = std::result::Result<T, String>;

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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let data_directory = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_directory)?;
            let library = Library::open(data_directory.join("library.sqlite3"))
                .map_err(|error| std::io::Error::other(error.to_string()))?;
            app.manage(AppState {
                library: Arc::new(library),
                active_index: Mutex::new(None),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            index_selected_folder,
            cancel_indexing,
            reconcile_approved_roots,
            list_source_roots,
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
}
