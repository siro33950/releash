// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // [05] CLI mode: 引数が一つでもあれば CLI として起動し、Tauri アプリ本体は
    // 立ち上げない。未知サブコマンドや構文エラーは clap が help/error を出力して
    // 終了するため、誤ったコマンド入力で GUI 本体が二重起動することはない。
    // GUI を起動するのは「引数が一つもない」場合のみ（spec [05] CLI 起動独立性境界）。
    if std::env::args().len() > 1 {
        std::process::exit(releash_lib::cli::run());
    }
    releash_lib::run()
}
