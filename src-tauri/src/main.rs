// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // [05] CLI mode: 第一引数が `workflow` のとき file-direct な CLI を起動し、
    // Tauri アプリは起動しない。CLI 起動独立性境界（spec [05] CLI 起動独立性境界）。
    if std::env::args().nth(1).as_deref() == Some("workflow") {
        std::process::exit(releash_lib::cli::run());
    }
    releash_lib::run()
}
