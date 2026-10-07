#![cfg(debug_assertions)]

use releash_desktop::test_support as host;
use releash_sdk::discovery::{process_start_time, LocalApiDiscovery};
use serde_json::json;
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use tauri::Manager;

static TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn invoke(
    window: &tauri::WebviewWindow<tauri::test::MockRuntime>,
    command: &str,
) -> Result<tauri::ipc::InvokeResponseBody, serde_json::Value> {
    tauri::test::get_ipc_response(
        window,
        tauri::webview::InvokeRequest {
            cmd: command.into(),
            callback: tauri::ipc::CallbackFn(0),
            error: tauri::ipc::CallbackFn(1),
            url: "tauri://localhost".parse().unwrap(),
            body: tauri::ipc::InvokeBody::Json(json!({})),
            headers: Default::default(),
            invoke_key: tauri::test::INVOKE_KEY.to_string(),
        },
    )
}

#[tokio::test]
async fn test_hidden起動_設定ファイルがあっても起動失敗では登録を触らず失敗窓を表示する() {
    // Given
    let _guard = TEST_LOCK.lock().await;
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("releash.toml"),
        "[app]\nauto_launch = true\nstart_minimized = true\n",
    )
    .unwrap();
    let path = directory.path().to_path_buf();
    // When
    let result = std::thread::spawn(move || {
        let app = host::desktop_connection_app(
            tauri::test::mock_builder(),
            &path,
            std::path::Path::new("/missing/releashd"),
        );
        let handle = app.handle().clone();
        tauri::async_runtime::spawn(async move {
            host::initialize_desktop(&handle, true).await;
        });
        app.run_return(|app, event| {
            if matches!(event, tauri::RunEvent::MainEventsCleared) {
                if let Some(failure) = app.get_webview_window("startup-failure") {
                    // Then
                    assert!(failure.is_visible().unwrap());
                    assert!(app.get_webview_window("main").is_none());
                    assert!(host::desktop_login_item_calls(app).is_empty());
                    assert_eq!(host::desktop_connection_status(app)["phase"], "failed");
                    failure.destroy().unwrap();
                }
            }
        })
    });
    tokio::task::spawn_blocking(move || result.join().unwrap())
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn test_古いサーバ入れ替え_停止失敗では起動せず停止完了後に設定を受信し通常窓へ復帰する() {
    // Given
    let _guard = TEST_LOCK.lock().await;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().canonicalize().unwrap();
    std::env::set_var("HOME", &path);
    std::env::set_var("XDG_CONFIG_HOME", path.join("config"));
    std::env::set_var("SHELL", "/bin/sh");
    let old = tokio::process::Command::new("/bin/sleep")
        .arg("120")
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let pid = old.id().unwrap();
    let old = Arc::new(tokio::sync::Mutex::new(old));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let discovery = LocalApiDiscovery {
        port: listener.local_addr().unwrap().port(),
        token: "old".into(),
        instance_id: "old-server".into(),
        pid,
        process_started_at: process_start_time(pid).unwrap(),
    };
    let discovery_path = path.join("client-api.json");
    std::fs::write(&discovery_path, serde_json::to_vec(&discovery).unwrap()).unwrap();
    let info = prost::Message::encode_to_vec(&releash_sdk::wire::ServerInfo {
        daemon_id: discovery.instance_id.clone(),
        pid,
        process_started_at: discovery.process_started_at,
        protocol: 0,
        release: "old-release".into(),
        ..Default::default()
    });
    let reject = Arc::new(AtomicBool::new(true));
    let rejecting = reject.clone();
    let router = axum::Router::new()
        .route(
            "/releash.client.v1.ClientService/GetServerInfo",
            axum::routing::post(move || {
                let info = info.clone();
                async move { ([("content-type", "application/proto")], info) }
            }),
        )
        .route(
            "/releash.client.v1.ClientService/StopDaemon",
            axum::routing::post(move || {
                let old = old.clone();
                let path = discovery_path.clone();
                let rejecting = rejecting.clone();
                async move {
                    if rejecting.load(Ordering::SeqCst) {
                        return (
                            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                            [("content-type", "application/json")],
                            "{\"code\":\"internal\",\"message\":\"stop refused\"}"
                                .as_bytes()
                                .to_vec(),
                        );
                    }
                    let mut process = old.lock().await;
                    process.kill().await.unwrap();
                    process.wait().await.unwrap();
                    std::fs::remove_file(path).unwrap();
                    (
                        axum::http::StatusCode::OK,
                        [("content-type", "application/proto")],
                        Vec::new(),
                    )
                }
            }),
        );
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let app = host::desktop_connection_app(
        tauri::test::mock_builder(),
        &path,
        &support::backend_executable(),
    );
    tokio::time::timeout(Duration::from_secs(3), async {
        while host::desktop_connection_status(app.handle())["phase"] != "failed" {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let failure = tauri::WebviewWindowBuilder::new(&app, "startup-failure", Default::default())
        .build()
        .unwrap();
    // When / Then
    assert!(invoke(&failure, "replace_daemon").is_err());
    assert_eq!(
        releash_sdk::discovery::read_optional(&path).unwrap(),
        Some(discovery)
    );
    assert_eq!(unsafe { libc::kill(pid as i32, 0) }, 0);
    assert!(app.get_webview_window("main").is_none());
    // When
    reject.store(false, Ordering::SeqCst);
    invoke(&failure, "replace_daemon").unwrap();
    // Then
    let current = releash_sdk::discovery::read_optional(&path)
        .unwrap()
        .unwrap();
    assert_ne!(current.pid, pid);
    assert_eq!(unsafe { libc::kill(pid as i32, 0) }, -1);
    assert_eq!(
        host::desktop_connection_status(app.handle())["phase"],
        "ready"
    );
    assert!(!host::desktop_login_item_calls(app.handle()).is_empty());
    let handle = app.handle().clone();
    let finished = Arc::new(AtomicBool::new(false));
    let observed = finished.clone();
    app.run_return(move |app, event| {
        if matches!(event, tauri::RunEvent::MainEventsCleared) {
            if let Some(main) = app.get_webview_window("main") {
                if !observed.swap(true, Ordering::SeqCst) {
                    assert!(main.is_visible().unwrap());
                    main.destroy().unwrap();
                }
            }
        }
    });
    assert!(finished.load(Ordering::SeqCst));
    host::stop_desktop_daemon(&handle).await;
    server.abort();
}

mod support;
