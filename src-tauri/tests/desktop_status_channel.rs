#![cfg(all(debug_assertions, feature = "desktop"))]

use serde_json::{json, Value};
use std::{path::Path, time::Duration};
use tauri::ipc::{CallbackFn, InvokeBody, InvokeResponseBody};

fn ipc(window: &tauri::WebviewWindow<tauri::test::MockRuntime>, command: &str, args: Value) {
    tauri::test::get_ipc_response(
        window,
        tauri::webview::InvokeRequest {
            cmd: command.into(),
            callback: CallbackFn(0),
            error: CallbackFn(1),
            url: "tauri://localhost".parse().unwrap(),
            body: InvokeBody::Json(args),
            headers: Default::default(),
            invoke_key: tauri::test::INVOKE_KEY.into(),
        },
    )
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn test_シェル状態購読_tauriコマンドからchannelに初期値と変化が届き停止後は届かない() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    git2::Repository::init(directory.path()).unwrap();
    for (key, value) in [
        ("HOME", directory.path().to_path_buf()),
        ("XDG_CONFIG_HOME", directory.path().join("config")),
        ("CLAUDE_CONFIG_DIR", directory.path().join(".claude")),
        ("CODEX_HOME", directory.path().join(".codex")),
    ] {
        std::env::set_var(key, value);
    }
    std::env::set_var("SHELL", "/bin/sh");
    std::env::remove_var("RELEASH_DATA_DIR");
    let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    let (stopped_sender, mut stopped_receiver) = tokio::sync::mpsc::unbounded_channel();
    let builder = tauri::test::mock_builder().channel_interceptor(move |_, callback, _, body| {
        if callback == CallbackFn(7) || callback == CallbackFn(8) {
            let InvokeResponseBody::Json(body) = body else {
                panic!("expected JSON channel message")
            };
            let status = serde_json::from_str::<Value>(body).unwrap();
            if callback == CallbackFn(7) {
                sender.send(status).unwrap();
            } else {
                stopped_sender.send(status).unwrap();
            }
            true
        } else {
            false
        }
    });
    let app = releash_lib::client_api_acceptance::desktop_connection_app(
        builder,
        directory.path(),
        Path::new(env!("CARGO_BIN_EXE_releash-backend")),
    );
    let window = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(35), async {
        while releash_lib::client_api_acceptance::desktop_supervision_status(app.handle())["phase"]
            != "restoring"
        {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|error| {
        panic!(
            "{error}: {}",
            releash_lib::client_api_acceptance::desktop_supervision_status(app.handle())
        )
    });

    // When
    ipc(
        &window,
        "subscribe_daemon_status",
        json!({"id":"screen", "channel":"__CHANNEL__:7"}),
    );
    // Then
    let initial = tokio::time::timeout(Duration::from_secs(2), receiver.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(initial["phase"], "restoring");
    assert!(initial.get("connectionGeneration").is_some());
    assert!(initial.get("retryAvailable").is_some());

    // When
    ipc(
        &window,
        "subscribe_daemon_status",
        json!({"id":"stopped-screen", "channel":"__CHANNEL__:8"}),
    );
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), stopped_receiver.recv())
            .await
            .unwrap()
            .unwrap()["phase"],
        "restoring"
    );
    ipc(
        &window,
        "stop_daemon_status_subscription",
        json!({"id":"stopped-screen"}),
    );
    releash_lib::client_api_acceptance::stop_desktop_daemon(app.handle(), false);
    // Then
    let changed = tokio::time::timeout(Duration::from_secs(2), receiver.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(changed["phase"], "stopping");

    // Then
    assert!(stopped_receiver.try_recv().is_err());
    ipc(
        &window,
        "stop_daemon_status_subscription",
        json!({"id":"screen"}),
    );
}
