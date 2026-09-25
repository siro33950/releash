#![cfg(all(debug_assertions, feature = "desktop", unix))]

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
fn workflow_facts(path: &Path, execution_id: &str) -> Vec<(String, String, String)> {
    let db = rusqlite::Connection::open_with_flags(
        path.join("local-event-store.sqlite3"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let facts = db.prepare("SELECT node_execution_id, event_type, detail FROM node_events WHERE tree_id = ? ORDER BY seq")
        .unwrap().query_map([execution_id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap().collect::<Result<Vec<_>, _>>().unwrap();
    facts
}
struct Renderer {
    window: Window,
    client: host::NativeClient,
    launch: String,
}
impl Renderer {
    async fn attach(app: &App) -> Self {
        let window = tauri::WebviewWindowBuilder::new(app, "main", Default::default())
            .build()
            .unwrap();
        let endpoint = host::desktop_client_endpoint(app.handle(), "view".into()).await;
        let client = host::connect_client(&endpoint);
        let hello = client
            .get_server_info(host::rpc::Unit::default())
            .await
            .unwrap()
            .into_owned();
        assert_eq!(hello.release, env!("CARGO_PKG_VERSION"));
        Self {
            window,
            client,
            launch: hello.launch_id,
        }
    }
    async fn request(&mut self, command: &str, args: Value) -> Value {
        host::request_client(&self.client, command, args)
            .await
            .unwrap()
    }
    async fn restore(&mut self, app: &App, expected_repos: Value) {
        self.request("update_external_editor", json!({"editor":"vim"}))
            .await;
        assert!(self
            .request("refresh_workspaces", json!({}))
            .await
            .is_null());
        let snapshot = host::read_state(&self.client, "workspaces").await.unwrap();
        let repos = Value::Array(
            snapshot["repositories"]
                .as_array()
                .unwrap()
                .iter()
                .map(|repo| repo["path"].clone())
                .collect(),
        );
        let telemetry = self
            .request("get_performance_telemetry_enabled", json!({}))
            .await;
        assert_eq!(repos, expected_repos);
        assert!(telemetry.is_boolean());
        assert_eq!(
            host::desktop_supervision_status(app.handle())["phase"],
            "restoring"
        );
        self.request("update_external_editor", json!({"editor":"vim"}))
            .await;
        ipc(
            &self.window,
            "complete_desktop_restoration",
            json!({"launchId":self.launch,"attachmentId":"view", "generation":host::desktop_supervision_status(app.handle())["connectionGeneration"]}),
        )
        .unwrap();
        wait_phase(app, "ready").await;
        self.request("update_external_editor", json!({"editor":"vim"}))
            .await;
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
    let mut renderer = Renderer::attach(&app).await;
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
    std::fs::write(workflows.join("update-acceptance.yml"), format!("name: update-acceptance\ndescription: update acceptance\nnodes:\n  main:\n    command: echo $$ > '{}' && exec sleep 120\n", marker.display())).unwrap();
    let execution_id = renderer
        .request(
            "start_workflow",
            json!({"workflowName":"update-acceptance", "worktreePath":root}),
        )
        .await
        .as_str()
        .unwrap()
        .to_string();
    tokio::time::timeout(Duration::from_secs(10), async {
        while std::fs::read_to_string(&marker)
            .ok()
            .and_then(|pid| pid.trim().parse::<i32>().ok())
            .is_none()
        {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let command_pid: i32 = std::fs::read_to_string(&marker)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert_eq!(unsafe { libc::kill(command_pid, 0) }, 0);
    let facts_before = workflow_facts(root, &execution_id);
    let command_node_id = facts_before
        .iter()
        .find(|(_, event, _)| event == "command_spawned")
        .expect("the running command must have a durable spawn fact")
        .0
        .clone();
    assert!(!facts_before
        .iter()
        .any(|(_, event, _)| event == "process_exited"));
    let pid = old["pid"].as_i64().unwrap() as i32;
    let old_launch = renderer.launch.clone();
    let next_binary = root.join("updated-backend");
    let steps = Arc::new(Mutex::new(Vec::new()));
    // When
    host::apply_desktop_update(app.handle(), Arc::new({
        let execution_id = execution_id.clone();
        let root = root.to_path_buf(); let next_binary = next_binary.clone(); let steps = steps.clone();
        move |stage| {
            if stage == "download" { assert_eq!(unsafe { libc::kill(pid,0) },0); }
            if stage == "install" {
                assert_eq!(unsafe { libc::kill(pid,0) },-1,"old daemon must exit before installation");
                assert_eq!(unsafe { libc::kill(command_pid,0) },-1,"workflow command must stop before installation");
                assert_eq!(workflow_facts(&root, &execution_id), facts_before, "shutdown must not append workflow facts");
                let db = rusqlite::Connection::open_with_flags(root.join("local-event-store.sqlite3"),rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
                let retired: i64 = db.query_row("SELECT COUNT(*) FROM sqlite_master WHERE name IN ('shutdown_plans', 'shutdown_targets', 'caller_attempts')",[],|row| row.get(0)).unwrap();
                assert_eq!(retired,0,"shutdown must not persist procedure records");
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
    let startup_probe = "startup-completion-probe";
    {
        let db = rusqlite::Connection::open(root.join("local-event-store.sqlite3")).unwrap();
        assert_eq!(db.execute(
            "INSERT INTO node_events (tree_id, seq, node_execution_id, parent_id, node_name, kind, attempt, event_type, session_id, detail, timestamp)
             SELECT ?1, 1, ?1, NULL, node_name, kind, attempt, event_type, NULL,
                    json_set(detail, '$.root.definition', json('{}')), timestamp + 1
             FROM node_events WHERE tree_id = ?2 AND seq = 1",
            [startup_probe, &execution_id],
        ).unwrap(), 1);
    }
    let next = host::desktop_connection_app(tauri::test::mock_builder(), root, &next_binary);
    wait_phase(&next, "restoring").await;
    let mut renderer = Renderer::attach(&next).await;
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let failures = host::read_state(
                &renderer.client,
                &format!("failures:{}:{startup_probe}", startup_probe.len()),
            )
            .await
            .unwrap();
            if failures["items"].as_array().unwrap().iter().any(|record| {
                record["target"] == startup_probe
                    && record["operation"] == "workflow_recovery"
                    && record["classification"] == "StateRequired"
                    && record["requiresAttention"] == true
            }) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("startup must report the unreadable definition as requiring attention");
    assert!(!workflow_facts(root, startup_probe)
        .iter()
        .any(|(_, event, _)| event == "abort_requested"));
    assert!(!workflow_facts(root, &execution_id)
        .iter()
        .any(|(node, event, _)| node == &command_node_id && event == "process_exited"));
    let current = discovery(root);
    assert_ne!(current["pid"], old["pid"]);
    assert_ne!(current["instance_id"], old["instance_id"]);
    assert_ne!(renderer.launch, old_launch);
    assert_ne!(renderer.launch, old_launch);
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
    renderer
        .request("update_external_editor", json!({"editor":"vim"}))
        .await;
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
