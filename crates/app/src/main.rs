//! Disk Cleaner GUI. The frontend lives in `ui/` (plain HTML/CSS/JS).
// No console window behind the GUI in release builds on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(commands::AppState::load())
        .invoke_handler(tauri::generate_handler![
            commands::system_drive,
            commands::list_drives,
            commands::scan_dir,
            commands::cancel_scan,
            commands::get_plan,
            commands::clean,
            commands::purge_recycled,
            commands::get_config,
            commands::save_config,
            commands::pick_folder,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Disk Cleaner");
}
