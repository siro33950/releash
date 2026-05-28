// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // [05] CLI mode: 引数があれば CLI として起動し、Tauri アプリ本体は立ち上げない。
    // 未知サブコマンドや構文エラーは clap が help/error を出力して終了するため、
    // 誤ったコマンド入力で GUI 本体が二重起動することはない。
    // GUI を起動するのは「引数が一つもない」場合、または GUI 専用フラグ
    // （`--hidden` 単独）の場合のみ（spec [05] CLI 起動独立性境界）。
    //
    // `--hidden` は `tauri_plugin_autostart` が自動起動時に付与するフラグで、
    // lib.rs 側の起動処理で参照されるため CLI 経路に流してはならない。
    let args: Vec<String> = std::env::args().skip(1).collect();
    let is_gui_only_flag = args.len() == 1 && args[0] == "--hidden";
    if !args.is_empty() && !is_gui_only_flag {
        std::process::exit(releash_lib::cli::run());
    }
    releash_lib::run()
}
