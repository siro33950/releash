#![cfg(all(debug_assertions, feature = "desktop"))]

use serde_json::json;

#[tokio::test]
async fn test_クライアントcommand_tauri経路を拒否する() {
    // Given
    let app =
        releash_lib::desktop_client_acceptance::command_rejection_app(tauri::test::mock_builder());
    let repo = tempfile::tempdir().unwrap();
    // When / Then
    let window = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .unwrap();
    let invoke = tauri::webview::InvokeRequest {
        cmd: "current-branch".into(),
        callback: tauri::ipc::CallbackFn(0),
        error: tauri::ipc::CallbackFn(1),
        url: "tauri://localhost".parse().unwrap(),
        body: tauri::ipc::InvokeBody::Json(json!({"repoPath": repo.path().to_str().unwrap()})),
        headers: Default::default(),
        invoke_key: tauri::test::INVOKE_KEY.into(),
    };
    assert!(tauri::test::get_ipc_response(&window, invoke).is_err());
}

#[tokio::test]
async fn test_terminal接続情報_削除済みcommandはtauri_invokeでエラーになる() {
    // Given
    let app =
        releash_lib::desktop_client_acceptance::command_rejection_app(tauri::test::mock_builder());
    let window = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .unwrap();
    // When
    let result = tauri::test::get_ipc_response(
        &window,
        tauri::webview::InvokeRequest {
            cmd: "get_terminal_stream_endpoint".into(),
            callback: tauri::ipc::CallbackFn(0),
            error: tauri::ipc::CallbackFn(1),
            url: "tauri://localhost".parse().unwrap(),
            body: tauri::ipc::InvokeBody::Json(json!({})),
            headers: Default::default(),
            invoke_key: tauri::test::INVOKE_KEY.into(),
        },
    );
    // Then
    assert_eq!(
        result.unwrap_err(),
        json!("Command get_terminal_stream_endpoint not found")
    );
}
