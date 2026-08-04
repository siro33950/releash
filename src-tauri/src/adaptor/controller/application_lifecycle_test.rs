use std::sync::{Arc, Mutex};

use super::shutdown_terminal_surface_before_local_api;

#[test]
fn test_通常終了テスト配置_追加したterminal_surface終了テストを別ファイルに置く() {
    let production_source = include_str!("application_lifecycle.rs");

    assert!(
        !production_source.contains("fn test_通常終了_terminal_surface保存後にlocal_apiを停止する")
    );
}

#[test]
fn test_通常終了_terminal_surface実行環境停止後にlocal_apiを停止する() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let terminal_calls = Arc::clone(&calls);
    let local_api_calls = Arc::clone(&calls);

    shutdown_terminal_surface_before_local_api(
        &move || {
            terminal_calls
                .lock()
                .unwrap()
                .push("terminal-stop-drain-flush");
            Ok(())
        },
        &move || local_api_calls.lock().unwrap().push("local-api-stop"),
    )
    .unwrap();

    assert_eq!(
        calls.lock().unwrap().as_slice(),
        &["terminal-stop-drain-flush", "local-api-stop"]
    );
}
