use std::sync::Arc;

use loom_core::{
    IndexReport, Library, LibraryStats, OpenArtifactRequest, SearchHit, SearchRequest,
};
use tauri::{Manager, State};
use tauri_plugin_dialog::DialogExt;

struct AppState {
    library: Arc<Library>,
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
    let library = Arc::clone(&state.library);
    tauri::async_runtime::spawn_blocking(move || library.index_path(path))
        .await
        .map_err(|error| format!("index worker stopped: {error}"))?
        .map(Some)
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
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            index_selected_folder,
            search,
            library_stats,
            open_artifact
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
            "allow-search",
            "allow-library-stats",
            "allow-open-artifact",
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
