#![cfg(all(debug_assertions, unix))]

use releash_desktop::test_support as host;
use releashd::test_support::client_api_acceptance as client_api;
use serde_json::{json, Value};
use std::{
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};
type App = tauri::App<tauri::test::MockRuntime>;
type Window = tauri::WebviewWindow<tauri::test::MockRuntime>;

async fn wait_phase(app: &App, phase: &str) {
    tokio::time::timeout(Duration::from_secs(35), async {
        loop {
            let status = host::desktop_connection_status(app.handle());
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
    client: client_api::NativeClient,
    daemon_id: String,
}
impl Renderer {
    async fn attach(app: &App) -> Self {
        let window = tauri::WebviewWindowBuilder::new(app, "main", Default::default())
            .build()
            .unwrap();
        let endpoint = host::desktop_client_endpoint(app.handle()).await;
        let client = client_api::connect_client(&endpoint);
        let hello = client
            .get_server_info(client_api::rpc::Unit::default())
            .await
            .unwrap()
            .into_owned();
        assert_eq!(hello.release, env!("CARGO_PKG_VERSION"));
        Self {
            window,
            client,
            daemon_id: hello.daemon_id,
        }
    }
    async fn request(&mut self, command: &str, args: Value) -> Value {
        client_api::request_client(&self.client, command, args)
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
        let snapshot = client_api::read_state(&self.client, "workspaces")
            .await
            .unwrap();
        let repos = Value::Array(
            snapshot["repositories"]
                .as_array()
                .unwrap()
                .iter()
                .map(|repo| repo["path"].clone())
                .collect(),
        );
        let settings = client_api::read_state(&self.client, "desktop-settings")
            .await
            .unwrap();
        assert_eq!(repos, expected_repos);
        assert!(settings["performanceTelemetry"].is_boolean());
        assert_eq!(
            host::desktop_connection_status(app.handle())["phase"],
            "ready"
        );
        self.request("update_external_editor", json!({"editor":"vim"}))
            .await;
        wait_phase(app, "ready").await;
        self.request("update_external_editor", json!({"editor":"vim"}))
            .await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_画面更新_実workflowとサーバを停止せず更新後の画面が同じサーバへ接続する() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let root_path = directory.path().canonicalize().unwrap();
    let root = root_path.as_path();
    git2::Repository::init(root).unwrap();
    std::env::set_current_dir(root).unwrap();
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
    let backend = support::backend_executable();
    let app = host::desktop_connection_app(tauri::test::mock_builder(), root, &backend);
    wait_phase(&app, "ready").await;
    let old = discovery(root);
    let mut renderer = Renderer::attach(&app).await;
    renderer.restore(&app, json!([root])).await;
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
    assert!(!facts_before
        .iter()
        .any(|(_, event, _)| event == "process_exited"));
    let pid = old["pid"].as_i64().unwrap() as i32;
    let old_daemon_id = renderer.daemon_id.clone();
    let next_binary = root.join("updated-backend");
    let steps = Arc::new(Mutex::new(Vec::new()));
    // When
    host::apply_desktop_update(
        app.handle(),
        Arc::new({
            let backend = backend.clone();
            let execution_id = execution_id.clone();
            let facts_before = facts_before.clone();
            let root = root.to_path_buf();
            let next_binary = next_binary.clone();
            let steps = steps.clone();
            move |stage| {
                if stage == "download" {
                    assert_eq!(unsafe { libc::kill(pid, 0) }, 0);
                }
                if stage == "install" {
                    assert_eq!(
                        unsafe { libc::kill(pid, 0) },
                        0,
                        "server must survive installation"
                    );
                    assert_eq!(
                        unsafe { libc::kill(command_pid, 0) },
                        0,
                        "workflow command must survive installation"
                    );
                    assert_eq!(workflow_facts(&root, &execution_id), facts_before);
                    std::fs::copy(&backend, &next_binary).unwrap();
                }
                steps.lock().unwrap().push(stage.to_string());
                Ok(())
            }
        }),
    )
    .await
    .unwrap();
    // Then
    assert_eq!(*steps.lock().unwrap(), ["download", "install", "restart"]);
    assert_eq!(unsafe { libc::kill(pid, 0) }, 0);
    assert_eq!(unsafe { libc::kill(command_pid, 0) }, 0);
    renderer.window.destroy().unwrap();
    drop(renderer);
    drop(app);
    let next = host::desktop_connection_app(tauri::test::mock_builder(), root, &next_binary);
    wait_phase(&next, "ready").await;
    let mut renderer = Renderer::attach(&next).await;
    let current = discovery(root);
    assert_eq!(current, old);
    assert_eq!(renderer.daemon_id, old_daemon_id);
    assert_eq!(unsafe { libc::kill(command_pid, 0) }, 0);
    assert_eq!(workflow_facts(root, &execution_id), facts_before);
    renderer.restore(&next, json!([root])).await;
    assert_eq!(discovery(root)["pid"], current["pid"]);
    host::stop_desktop_daemon(next.handle()).await;
    assert!(!root.join("client-api.json").exists());
}

mod support;
