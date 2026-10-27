fn main() {
    // Declaring the app's commands makes Tauri generate one permission per
    // command (allow-<name>), so a command is reachable from the webview only
    // when capabilities/main.json grants it.
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "system_drive",
            "list_drives",
            "scan_dir",
            "cancel_scan",
            "get_plan",
            "clean",
            "purge_recycled",
            "get_config",
            "save_config",
            "pick_folder",
        ]),
    ))
    .expect("tauri build script failed");
}
