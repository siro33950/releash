#![cfg(all(debug_assertions, feature = "desktop"))]

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

fn discovery(directory: &Path, filename: &str) -> Value {
    serde_json::from_slice(&std::fs::read(directory.join(filename)).unwrap()).unwrap()
}

async fn wait_phase(app: &tauri::AppHandle<tauri::test::MockRuntime>, phase: &str) {
    tokio::time::timeout(Duration::from_secs(35), async {
        loop {
            let status = releash_lib::client_api_acceptance::desktop_supervision_status(app);
            if status["phase"] == phase {
                break;
            }
            if phase == "ready" {
                assert_ne!(status["phase"], "failed", "{status}");
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
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
    let app = releash_lib::client_api_acceptance::desktop_connection_app(
        tauri::test::mock_builder(),
        directory.path(),
        Path::new(env!("CARGO_BIN_EXE_releash-backend")),
    );
    wait_phase(app.handle(), "restoring").await;
    let first = discovery(directory.path(), "client-api.json");
    let window = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .unwrap();
    assert_eq!(discovery(directory.path(), "client-api.json"), first);
    releash_lib::client_api_acceptance::initialize_desktop_settings(app.handle()).await;
    assert_eq!(
        releash_lib::client_api_acceptance::desktop_window_preferences(app.handle()),
        false
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
    let mut tokens = Vec::new();
    let mut frames: Option<tokio::sync::broadcast::Receiver<Option<Vec<u8>>>> = None;
    let mut restarted = false;
    // When
    tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            let line = tokio::select! {
                line = output.next_line() => { let Some(line) = line.unwrap() else { break }; line }
                frame = async { match frames.as_mut() { Some(frames) => frames.recv().await, None => std::future::pending().await } } => {
                    let frame = frame.unwrap_or(None);
                    if frame.is_none() { frames = None; }
                    input.write_all(format!("{}\n", json!({"frame": frame})).as_bytes()).await.unwrap();
                    continue;
                }
            };
            let request: Value = serde_json::from_str(&line).expect("client bridge request");
            let result = match request["command"].as_str().unwrap() {
                "attach_desktop_client" => {
                    let (hello, receiver) = releash_lib::client_api_acceptance::attach_desktop_client(app.handle(), request["args"]["attachmentId"].as_str().unwrap().into());
                    frames = Some(receiver);
                    let current = discovery(directory.path(), "client-api.json");
                    assert_ne!(current["token"], discovery(directory.path(), "local-api.json")["token"]);
                    tokens.push(current["token"].clone());
                    json!(hello)
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
                        settings["closeToTray"].as_bool().unwrap());
                    Value::Null
                }
                command @ ("get_daemon_status" | "complete_desktop_restoration" | "validate_daemon_connection" | "admit_client_command" | "send_desktop_client_frame" | "detach_desktop_client" | "forget_client_operation" | "list_client_handoff") => {
                    tauri::test::get_ipc_response(&window, tauri::webview::InvokeRequest {
                        cmd: command.into(), callback: tauri::ipc::CallbackFn(0), error: tauri::ipc::CallbackFn(1), url: "tauri://localhost".parse().unwrap(),
                        body: tauri::ipc::InvokeBody::Json(request["args"].clone()), headers: Default::default(), invoke_key: tauri::test::INVOKE_KEY.to_string(),
                    }).unwrap().deserialize::<Value>().unwrap()
                }
                "damage_settings" => {
                    for content in [Some("[app]\nclose_to_tray = true\n"), Some("[invalid"), None] {
                        let path = directory.path().join("releash.toml");
                        match content {
                            Some(content) => std::fs::write(&path, content).unwrap(),
                            None => std::fs::remove_file(&path).unwrap(),
                        }
                        releash_lib::client_api_acceptance::initialize_desktop_settings(app.handle()).await;
                        assert_eq!(releash_lib::client_api_acceptance::desktop_window_preferences(app.handle()), false);
                    }
                    std::fs::create_dir(directory.path().join("releash.toml")).unwrap();
                    releash_lib::client_api_acceptance::initialize_desktop_settings(app.handle()).await;
                    assert_eq!(releash_lib::client_api_acceptance::desktop_window_preferences(app.handle()), false);
                    std::fs::remove_dir(directory.path().join("releash.toml")).unwrap();
                    Value::Null
                }
                "restart" => {
                    assert!(!restarted);
                    assert_eq!(unsafe { libc::kill(first["pid"].as_i64().unwrap() as i32, libc::SIGKILL) }, 0);
                    tokio::time::sleep(Duration::from_millis(250)).await;
                    wait_phase(app.handle(), "restoring").await;
                    let current = discovery(directory.path(), "client-api.json");
                    assert_ne!(first["instance_id"], current["instance_id"]);
                    assert_ne!(first["token"], current["token"]);
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
    assert!(tokens.len() >= 2);
    assert_ne!(tokens.first().unwrap(), tokens.last().unwrap());
    let old_pid = discovery(directory.path(), "client-api.json")["pid"]
        .as_i64()
        .unwrap() as i32;
    window.destroy().unwrap();
    assert_eq!(unsafe { libc::kill(old_pid, 0) }, 0);
    releash_lib::client_api_acceptance::stop_desktop_daemon(app.handle(), true);
    wait_phase(app.handle(), "stopped").await;
    assert_eq!(unsafe { libc::kill(old_pid, 0) }, -1);
    let next = releash_lib::client_api_acceptance::desktop_connection_app(
        tauri::test::mock_builder(),
        directory.path(),
        Path::new(env!("CARGO_BIN_EXE_releash-backend")),
    );
    wait_phase(next.handle(), "restoring").await;
    assert_ne!(
        discovery(directory.path(), "client-api.json")["pid"].as_i64(),
        Some(old_pid as i64)
    );
    assert_eq!(
        std::fs::read_dir(directory.path().join("desktop-client-operations"))
            .unwrap()
            .count(),
        1
    );
    let next_window = tauri::WebviewWindowBuilder::new(&next, "main", Default::default())
        .build()
        .unwrap();
    let mut restored = tokio::process::Command::new("node")
        .args(["tests/helpers/desktop-daemon.mjs", "--restored"])
        .current_dir(Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut input = restored.stdin.take().unwrap();
    let mut lines = BufReader::new(restored.stdout.take().unwrap()).lines();
    let mut frames: Option<tokio::sync::broadcast::Receiver<Option<Vec<u8>>>> = None;
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let line = tokio::select! {
                line = lines.next_line() => { let Some(line) = line.unwrap() else { break }; line }
                frame = async { match frames.as_mut() { Some(frames) => frames.recv().await, None => std::future::pending().await } } => {
                    let frame = frame.unwrap_or(None);
                    if frame.is_none() { frames = None; }
                    input.write_all(format!("{}\n", json!({"frame": frame})).as_bytes()).await.unwrap();
                    continue;
                }
            };
            let request: Value = serde_json::from_str(&line).unwrap();
            let result = if request["command"] == "attach_desktop_client" {
                let (hello, receiver) = releash_lib::client_api_acceptance::attach_desktop_client(next.handle(), request["args"]["attachmentId"].as_str().unwrap().into());
                frames = Some(receiver);
                json!(hello)
            } else { tauri::test::get_ipc_response(
                &next_window,
                tauri::webview::InvokeRequest {
                    cmd: request["command"].as_str().unwrap().into(),
                    callback: tauri::ipc::CallbackFn(0),
                    error: tauri::ipc::CallbackFn(1),
                    url: "tauri://localhost".parse().unwrap(),
                    body: tauri::ipc::InvokeBody::Json(request["args"].clone()),
                    headers: Default::default(),
                    invoke_key: tauri::test::INVOKE_KEY.to_string(),
                },
            )
            .unwrap()
            .deserialize::<Value>()
            .unwrap() };
            input
                .write_all(
                    format!("{}\n", json!({"id": request["id"], "result": result})).as_bytes(),
                )
                .await
                .unwrap();
        }
        assert!(restored.wait().await.unwrap().success());
    })
    .await
    .unwrap();
    releash_lib::client_api_acceptance::stop_desktop_daemon(next.handle(), false);
    wait_phase(next.handle(), "stopped").await;
    let failed_dir = directory.path().join("initialization-failure");
    std::fs::create_dir(&failed_dir).unwrap();
    std::fs::write(
        failed_dir.join("local-event-store.sqlite3"),
        "not a database",
    )
    .unwrap();
    for (executable, expected_stage, retries) in [
        (Path::new("/missing/releash-backend"), "spawn", 0),
        (
            Path::new(env!("CARGO_BIN_EXE_releash-backend")),
            "backend_initialization",
            3,
        ),
    ] {
        let failed = releash_lib::client_api_acceptance::desktop_connection_app(
            tauri::test::mock_builder(),
            &failed_dir,
            executable,
        );
        wait_phase(failed.handle(), "failed").await;
        let status =
            releash_lib::client_api_acceptance::desktop_supervision_status(failed.handle());
        assert_eq!(status["stage"], expected_stage);
        assert_eq!(status["retries"], retries);
        assert!(!status["reason"].as_str().unwrap().is_empty());
        releash_lib::client_api_acceptance::stop_desktop_daemon(failed.handle(), false);
        wait_phase(failed.handle(), "stopped").await;
    }
}
