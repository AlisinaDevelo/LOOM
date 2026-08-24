fn main() {
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "index_selected_folder",
            "cancel_indexing",
            "reconcile_approved_roots",
            "list_source_roots",
            "revoke_source_root",
            "search",
            "library_stats",
            "open_artifact",
        ]),
    ))
    .expect("failed to build LOOM's Tauri manifest");
}
