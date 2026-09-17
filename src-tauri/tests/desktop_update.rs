#![cfg(all(debug_assertions, feature = "desktop", unix))]

use prost::Message;
use releash_lib::client_api_acceptance as host;
use serde_json::{json, Value};
use std::{
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};
type App = tauri::App<tauri::test::MockRuntime>;
type Window = tauri::WebviewWindow<tauri::test::MockRuntime>;

fn ipc(window: &Window, command: &str, args: Value) -> Result<Value, Value> {
    tauri::test::get_ipc_response(
        window,
        tauri::webview::InvokeRequest {
            cmd: command.into(),
            callback: tauri::ipc::CallbackFn(0),
            error: tauri::ipc::CallbackFn(1),
            url: "tauri://localhost".parse().unwrap(),
            body: tauri::ipc::InvokeBody::Json(args),
            headers: Default::default(),
            invoke_key: tauri::test::INVOKE_KEY.to_string(),
        },
    )
    .map(|response| response.deserialize().unwrap())
}
async fn wait_phase(app: &App, phase: &str) {
    tokio::time::timeout(Duration::from_secs(35), async {
        loop {
            let status = host::desktop_supervision_status(app.handle());
            assert_ne!(status["phase"], "failed", "{status}");
            if status["phase"] == phase {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
}
fn discovery(path: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(path.join("client-api.json")).unwrap()).unwrap()
}
struct Renderer {
    window: Window,
    frames: tokio::sync::broadcast::Receiver<Option<Vec<u8>>>,
    launch: String,
    instance: String,
}
impl Renderer {
    fn attach(app: &App) -> Self {
        let window = tauri::WebviewWindowBuilder::new(app, "main", Default::default())
            .build()
            .unwrap();
        let (hello, frames) = host::attach_desktop_client(app.handle(), "view".into());
        let Some(host::envelope::Body::Hello(hello)) =
            host::Envelope::decode(hello.as_slice()).unwrap().body
        else {
            panic!("authenticated hello");
        };
        assert_eq!(hello.release, env!("CARGO_PKG_VERSION"));
        Self {
            window,
            frames,
            launch: hello.launch_id,
            instance: hello.instance_id,
        }
    }
    async fn request(&mut self, command: &str, args: Value) -> Value {
        let id = uuid::Uuid::new_v4().to_string();
        let mut frame =
            host::Envelope::decode(host::encode_client_request(&id, command, args).as_slice())
                .unwrap();
        if let Some(host::envelope::Body::Request(request)) = &mut frame.body {
            request.instance_id = self.instance.clone();
        }
        ipc(
            &self.window,
            "send_desktop_client_frame",
            json!({"launchId": self.launch, "bytes": frame.encode_to_vec()}),
        )
        .unwrap();
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let bytes = self.frames.recv().await.unwrap().unwrap();
                if let Some(host::envelope::Body::Response(response)) =
                    host::Envelope::decode(bytes.as_slice()).unwrap().body
                {
                    if response.request_id != id {
                        continue;
                    }
                    let value = match response.outcome.unwrap() {
                        host::command_response::Outcome::Result(value) => {
                            host::decode_client_value(value)
                        }
                        host::command_response::Outcome::Error(error) => {
                            panic!("{command}: {error:?}")
                        }
                    };
                    let ack = host::Envelope {
                        body: Some(host::envelope::Body::RequestAck(host::RequestAck {
                            request_id: id,
                            ..Default::default()
                        })),
                    };
                    ipc(
                        &self.window,
                        "send_desktop_client_frame",
                        json!({"launchId": self.launch, "bytes": ack.encode_to_vec()}),
                    )
                    .unwrap();
                    return value;
                }
            }
        })
        .await
        .unwrap()
    }
    async fn restore(&mut self, app: &App, expected_repos: Value) {
        assert!(ipc(
            &self.window,
            "admit_client_command",
            json!({"command":"update_external_editor"})
        )
        .is_err());
        let repos = self.request("get_repo_paths", json!({})).await;
        let telemetry = self
            .request("get_performance_telemetry_enabled", json!({}))
            .await;
        assert_eq!(repos, expected_repos);
        assert!(telemetry.is_boolean());
        assert_eq!(
            host::desktop_supervision_status(app.handle())["phase"],
            "restoring"
        );
        assert!(ipc(
            &self.window,
            "admit_client_command",
            json!({"command":"update_external_editor"})
        )
        .is_err());
        ipc(
            &self.window,
            "complete_desktop_restoration",
            json!({"launchId":self.launch,"attachmentId":"view", "generation":host::desktop_supervision_status(app.handle())["connectionGeneration"]}),
        )
        .unwrap();
        wait_phase(app, "ready").await;
        ipc(
            &self.window,
            "admit_client_command",
            json!({"command":"update_external_editor"}),
        )
        .unwrap();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_実workflow更新_一括停止と旧daemon終了から適用と新接続と状態反映まで順序を守る() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    git2::Repository::init(root).unwrap();
    for (key, value) in [
        ("HOME", root.to_path_buf()),
        ("XDG_CONFIG_HOME", root.join("config")),
        ("CLAUDE_CONFIG_DIR", root.join(".claude")),
        ("CODEX_HOME", root.join(".codex")),
    ] {
        std::env::set_var(key, value);
    }
    std::env::set_var("SHELL", "/bin/sh");
    std::env::remove_var("RELEASH_DATA_DIR");
    let backend = Path::new(env!("CARGO_BIN_EXE_releash-backend"));
    let app = host::desktop_connection_app(tauri::test::mock_builder(), root, backend);
    wait_phase(&app, "restoring").await;
    let old = discovery(root);
    let mut renderer = Renderer::attach(&app);
    renderer.restore(&app, json!([])).await;
    renderer
        .request("add_repo_path", json!({"path":root}))
        .await;
    let workflows = if cfg!(target_os = "macos") {
        root.join("Library/Application Support/releash/workflows")
    } else {
        root.join("config/releash/workflows")
    };
    let marker = root.join("workflow-running");
    std::fs::write(workflows.join("update-acceptance.yml"), format!("name: update-acceptance\ndescription: update acceptance\nnodes:\n  main:\n    command: touch '{}' && sleep 120\n", marker.display())).unwrap();
    renderer
        .request(
            "start_workflow",
            json!({"workflowName":"update-acceptance", "worktreePath":root}),
        )
        .await;
    tokio::time::timeout(Duration::from_secs(10), async {
        while !marker.exists() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let pid = old["pid"].as_i64().unwrap() as i32;
    let old_launch = renderer.launch.clone();
    let old_instance = renderer.instance.clone();
    let next_binary = root.join("updated-backend");
    let steps = Arc::new(Mutex::new(Vec::new()));
    // When
    host::apply_desktop_update(app.handle(), Arc::new({
        let root = root.to_path_buf(); let next_binary = next_binary.clone(); let steps = steps.clone();
        move |stage| {
            if stage == "download" { assert_eq!(unsafe { libc::kill(pid,0) },0); }
            if stage == "install" {
                assert_eq!(unsafe { libc::kill(pid,0) },-1,"old daemon must exit before installation");
                let db = rusqlite::Connection::open_with_flags(root.join("local-event-store.sqlite3"),rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
                let completed: i64 = db.query_row("SELECT COUNT(*) FROM shutdown_plans WHERE phase = 'completed'",[],|row| row.get(0)).unwrap();
                assert_eq!(completed,1,"ShutdownCoordinator must complete before installation");
                let targets: i64 = db.query_row("SELECT COUNT(*) FROM shutdown_targets WHERE detail LIKE '%workflow_execution%'",[],|row| row.get(0)).unwrap();
                assert!(targets>0,"running workflow must participate in shutdown");
                std::fs::copy(backend,&next_binary).unwrap();
            }
            steps.lock().unwrap().push(stage.to_string());
            Ok(())
        }
    })).await.unwrap();
    // Then
    assert_eq!(*steps.lock().unwrap(), ["download", "install", "restart"]);
    assert_eq!(
        host::desktop_supervision_status(app.handle())["phase"],
        "stopped"
    );
    let next = host::desktop_connection_app(tauri::test::mock_builder(), root, &next_binary);
    wait_phase(&next, "restoring").await;
    let current = discovery(root);
    assert_ne!(current["pid"], old["pid"]);
    assert_ne!(current["instance_id"], old["instance_id"]);
    let mut renderer = Renderer::attach(&next);
    assert_ne!(renderer.launch, old_launch);
    assert_ne!(renderer.instance, old_instance);
    ipc(
        &renderer.window,
        "validate_daemon_connection",
        json!({"launchId": renderer.launch, "release": env!("CARGO_PKG_VERSION")}),
    )
    .unwrap();
    assert!(ipc(
        &renderer.window,
        "validate_daemon_connection",
        json!({"launchId":old_launch,"release":env!("CARGO_PKG_VERSION")})
    )
    .is_err());
    assert!(ipc(
        &renderer.window,
        "validate_daemon_connection",
        json!({"launchId":renderer.launch,"release":"wrong-release"})
    )
    .is_err());
    let generation =
        host::desktop_supervision_status(next.handle())["connectionGeneration"].clone();
    ipc(
        &renderer.window,
        "fail_desktop_restoration",
        json!({"generation":generation, "reason":"Repositories: temporary read failure"}),
    )
    .unwrap();
    let failed = host::desktop_supervision_status(next.handle());
    assert_eq!(failed["phase"], "failed");
    assert_eq!(failed["stage"], "state_restoration");
    assert_eq!(failed["reason"], "Repositories: temporary read failure");
    assert!(ipc(
        &renderer.window,
        "admit_client_command",
        json!({"command":"update_external_editor"})
    )
    .is_err());
    ipc(&renderer.window, "retry_daemon", json!({})).unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while host::desktop_supervision_status(next.handle())["connectionGeneration"] == generation
        {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    wait_phase(&next, "restoring").await;
    assert!(ipc(
        &renderer.window,
        "complete_desktop_restoration",
        json!({"launchId":renderer.launch,"attachmentId":"view", "generation":generation})
    )
    .is_err());
    renderer.restore(&next, json!([root])).await;
    assert_eq!(discovery(root)["pid"], current["pid"]);
    host::stop_desktop_daemon(next.handle(), false);
    wait_phase(&next, "stopped").await;
}
