mod pty;

use pty::{kill_pty, resize_pty, spawn_pty, write_pty, PtyManager};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(PtyManager::default())
        .invoke_handler(tauri::generate_handler![
            spawn_pty, write_pty, resize_pty, kill_pty
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
