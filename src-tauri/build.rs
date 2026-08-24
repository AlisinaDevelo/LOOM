fn main() {
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "index_selected_folder",
            "cancel_indexing",
            "capture_status",
            "set_capture_paused",
            "set_capture_exclusions",
            "capture_intentional",
            "purge_captures",
            "reconcile_approved_roots",
            "list_source_roots",
            "revoke_source_root",
            "search",
            "library_stats",
            "ocr_status",
            "set_ocr_enabled",
            "purge_ocr_records",
            "open_artifact",
            "resolve_evidence",
        ]),
    ))
    .expect("failed to build LOOM's Tauri manifest");
}
