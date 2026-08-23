fn main() {
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "index_selected_folder",
            "search",
            "library_stats",
            "open_artifact",
        ]),
    ))
    .expect("failed to build LOOM's Tauri manifest");
}
