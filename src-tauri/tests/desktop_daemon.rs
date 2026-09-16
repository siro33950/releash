#![cfg(all(debug_assertions, feature = "desktop"))]

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

fn discovery(directory: &Path, filename: &str) -> Value {
    serde_json::from_slice(&std::fs::read(directory.join(filename)).unwrap()).unwrap()
}

async fn start_daemon(directory: &Path) -> (tokio::process::Child, Value) {
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_releash-backend"))
        .arg("--internal-daemon")
        .arg(directory)
        .env_remove("RELEASH_DATA_DIR")
        .env("SHELL", "/bin/sh")
        .env("XDG_CONFIG_HOME", directory.join("config"))
        .env("HOME", directory)
        .env("CLAUDE_CONFIG_DIR", directory.join(".claude"))
        .env("CODEX_HOME", directory.join(".codex"))
        .current_dir(directory)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let current = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            assert!(child.try_wait().unwrap().is_none(), "daemon exited");
            if let Ok(bytes) = std::fs::read(directory.join("client-api.json")) {
                if let Ok(current) = serde_json::from_slice::<Value>(&bytes) {
                    if current["pid"].as_u64() == child.id().map(u64::from) {
                        break current;
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("daemon discovery deadline");
    (child, current)
}

#[tokio::test(flavor = "multi_thread")]
async fn test_desktop接続_discoveryとtauri経由で外部daemonの初回接続と再起動から復旧する() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("releash.toml"),
        "[app]\nclose_to_tray = false\nstart_minimized = true\n",
    )
    .unwrap();
    let (mut daemon, first) = start_daemon(directory.path()).await;
    let app = releash_lib::client_api_acceptance::desktop_connection_app(
        tauri::test::mock_builder(),
        directory.path(),
    );
    let window = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .unwrap();
    assert_eq!(discovery(directory.path(), "client-api.json"), first);
    releash_lib::client_api_acceptance::initialize_desktop_settings(app.handle()).await;
    assert_eq!(
        releash_lib::client_api_acceptance::desktop_window_preferences(app.handle()),
        (false, true)
    );
    let mut client = tokio::process::Command::new("node")
        .arg("tests/helpers/desktop-daemon.mjs")
        .current_dir(Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut input = client.stdin.take().unwrap();
    let mut output = BufReader::new(client.stdout.take().unwrap()).lines();
    let mut endpoints = Vec::new();
    let mut restarted = false;
    // When
    tokio::time::timeout(Duration::from_secs(60), async {
        while let Some(line) = output.next_line().await.unwrap() {
            let request: Value = serde_json::from_str(&line).expect("client bridge request");
            let result = match request["command"].as_str().unwrap() {
                "get_client_endpoint" => {
                    let endpoint = tauri::test::get_ipc_response(
                        &window,
                        tauri::webview::InvokeRequest {
                            cmd: "get_client_endpoint".into(),
                            callback: tauri::ipc::CallbackFn(0),
                            error: tauri::ipc::CallbackFn(1),
                            url: "tauri://localhost".parse().unwrap(),
                            body: tauri::ipc::InvokeBody::default(),
                            headers: Default::default(),
                            invoke_key: tauri::test::INVOKE_KEY.to_string(),
                        },
                    )
                    .unwrap()
                    .deserialize::<Value>()
                    .unwrap();
                    let current = discovery(directory.path(), "client-api.json");
                    let master = discovery(directory.path(), "local-api.json");
                    assert_eq!(current["pid"].as_u64(), daemon.id().map(u64::from));
                    assert_ne!(current["token"], master["token"]);
                    assert_eq!(
                        endpoint,
                        json!({
                            "url": format!("ws://127.0.0.1:{}/v1/client", current["port"]),
                            "authSubprotocol": format!("releash-bearer.{}", current["token"].as_str().unwrap())
                        })
                    );
                    endpoints.push(endpoint.clone());
                    endpoint
                }
                "apply_desktop_settings" => {
                    tauri::test::get_ipc_response(&window, tauri::webview::InvokeRequest {
                        cmd: "apply_desktop_settings".into(),
                        callback: tauri::ipc::CallbackFn(0), error: tauri::ipc::CallbackFn(1),
                        url: "tauri://localhost".parse().unwrap(),
                        body: tauri::ipc::InvokeBody::Json(request["args"].clone()),
                        headers: Default::default(), invoke_key: tauri::test::INVOKE_KEY.to_string(),
                    }).unwrap();
                    let settings = &request["args"]["settings"];
                    assert_eq!(releash_lib::client_api_acceptance::desktop_window_preferences(app.handle()),
                        (settings["closeToTray"].as_bool().unwrap(), settings["startMinimized"].as_bool().unwrap()));
                    Value::Null
                }
                "damage_settings" => {
                    for content in [Some("[app]\nclose_to_tray = true\n"), Some("[invalid"), None] {
                        let path = directory.path().join("releash.toml");
                        match content {
                            Some(content) => std::fs::write(&path, content).unwrap(),
                            None => std::fs::remove_file(&path).unwrap(),
                        }
                        releash_lib::client_api_acceptance::initialize_desktop_settings(app.handle()).await;
                        assert_eq!(releash_lib::client_api_acceptance::desktop_window_preferences(app.handle()), (false, true));
                    }
                    std::fs::create_dir(directory.path().join("releash.toml")).unwrap();
                    releash_lib::client_api_acceptance::initialize_desktop_settings(app.handle()).await;
                    assert_eq!(releash_lib::client_api_acceptance::desktop_window_preferences(app.handle()), (false, true));
                    std::fs::remove_dir(directory.path().join("releash.toml")).unwrap();
                    Value::Null
                }
                "restart" => {
                    assert!(!restarted);
                    daemon.kill().await.unwrap();
                    let (next, current) = start_daemon(directory.path()).await;
                    assert_ne!(first["instance_id"], current["instance_id"]);
                    assert_ne!(first["token"], current["token"]);
                    daemon = next;
                    restarted = true;
                    Value::Null
                }
                command => panic!("unexpected client bridge command: {command}"),
            };
            let response = json!({"id": request["id"], "result": result});
            input
                .write_all(format!("{response}\n").as_bytes())
                .await
                .unwrap();
        }
        assert!(client.wait().await.unwrap().success(), "desktop client failed");
    })
    .await
    .expect("desktop recovery deadline");
    // Then
    assert!(restarted);
    assert!(endpoints.len() >= 2);
    assert_ne!(
        endpoints.first().unwrap()["authSubprotocol"],
        endpoints.last().unwrap()["authSubprotocol"]
    );
    assert!(daemon.try_wait().unwrap().is_none());
    window.destroy().unwrap();
    daemon.kill().await.unwrap();
}
