use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use serde_json::Value;

use releashd::test_support::connect::rpc;
use releashd::test_support::wire;
fn to_wire<T: prost::Message + Default>(
    value: &impl buffa::Message,
) -> Result<T, connectrpc::ConnectError> {
    T::decode(buffa::Message::encode_to_vec(value).as_slice())
        .map_err(|e| connectrpc::ConnectError::internal(e.to_string()))
}
fn to_rpc<T: buffa::Message>(value: &impl prost::Message) -> Result<T, connectrpc::ConnectError> {
    T::decode_from_slice(&prost::Message::encode_to_vec(value))
        .map_err(|e| connectrpc::ConnectError::internal(e.to_string()))
}
include!(concat!(env!("OUT_DIR"), "/client_calls.rs"));
struct Socket {
    client: rpc::ClientServiceClient<connectrpc::client::HttpClient>,
}
type StateStream =
    std::pin::Pin<Box<dyn futures_util::Stream<Item = wire::state_payload::Value> + Send>>;
async fn read_state(socket: &Socket, target: &str) -> wire::state_payload::Value {
    subscribe_state(socket, target, vec![])
        .await
        .next()
        .await
        .unwrap()
}
async fn read_external_editor(socket: &Socket) -> String {
    let wire::state_payload::Value::ExternalEditor(editor) =
        read_state(socket, "external-editor").await
    else {
        panic!("external editor")
    };
    editor.selected.unwrap()
}
type WorkspaceStream =
    std::pin::Pin<Box<dyn futures_util::Stream<Item = wire::WorkspaceListSnapshot> + Send>>;
async fn subscribe_state(socket: &Socket, target: &str, args: Vec<String>) -> StateStream {
    let client_id = uuid::Uuid::new_v4().to_string();
    let mut stream = socket
        .client
        .open_state_stream(rpc::OpenStateStreamRequest {
            client_id: client_id.clone(),
            ..Default::default()
        })
        .await
        .unwrap();
    stream
        .message::<rpc::StateSubscriptionEvent>()
        .await
        .unwrap();
    socket
        .client
        .start_state_subscription(rpc::StartStateSubscriptionRequest {
            client_id,
            subscription_id: uuid::Uuid::new_v4().to_string(),
            target: target.into(),
            args,
            ..Default::default()
        })
        .await
        .unwrap();
    Box::pin(futures_util::stream::unfold(
        stream,
        |mut stream| async move {
            loop {
                let item = stream
                    .message::<rpc::StateSubscriptionEvent>()
                    .await
                    .unwrap()?
                    .to_owned_message();
                let event: wire::StateSubscriptionEvent = to_wire(&item).unwrap();
                let payload = match event.event {
                    Some(wire::state_subscription_event::Event::Snapshot(value)) => Some(value),
                    Some(wire::state_subscription_event::Event::Change(value)) => value.payload,
                    _ => None,
                };
                if let Some(wire::StatePayload { value: Some(value) }) = payload {
                    return Some((value, stream));
                }
            }
        },
    ))
}
async fn subscribe_workspaces(socket: &Socket) -> WorkspaceStream {
    Box::pin(
        subscribe_state(socket, "workspaces", vec![])
            .await
            .filter_map(|value| async move {
                match value {
                    wire::state_payload::Value::Workspaces(value) => Some(value),
                    _ => None,
                }
            }),
    )
}
async fn expect_state(
    stream: &mut StateStream,
    phase: &str,
    predicate: impl Fn(&wire::state_payload::Value) -> bool,
) -> wire::state_payload::Value {
    let mut last = None;
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let value = stream.next().await.unwrap();
            if predicate(&value) {
                return value;
            }
            last = Some(value);
        }
    })
    .await
    .unwrap_or_else(|_| panic!("state subscription did not reflect {phase}: {last:?}"))
}
async fn expect_workspace(
    stream: &mut WorkspaceStream,
    phase: &str,
    predicate: impl Fn(&wire::WorkspaceListSnapshot) -> bool,
) -> wire::WorkspaceListSnapshot {
    let mut last = None;
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let value = stream.next().await.unwrap();
            if predicate(&value) {
                return value;
            }
            last = Some(value);
        }
    })
    .await
    .unwrap_or_else(|_| panic!("workspace subscription did not reflect {phase}: {last:?}"))
}
fn find_worktree<'a>(
    value: &'a wire::WorkspaceListSnapshot,
    path: &str,
) -> Option<&'a wire::WorkspaceWorktreeList> {
    value
        .repositories
        .as_ref()
        .unwrap()
        .items
        .iter()
        .flat_map(|repo| &repo.worktrees.as_ref().unwrap().items)
        .find(|tree| tree.path.as_deref() == Some(path))
}
fn worktree_state<'a>(
    value: &'a wire::WorkspaceListSnapshot,
    path: &str,
) -> &'a wire::WorkspaceWorktreeList {
    find_worktree(value, path).unwrap()
}
/// 一覧の行のうち、条件を満たすものがあるか。
fn any_branch(
    value: &wire::WorkspaceListSnapshot,
    predicate: impl Fn(&wire::WorkspaceBranch) -> bool,
) -> bool {
    value
        .repositories
        .as_ref()
        .unwrap()
        .items
        .iter()
        .flat_map(|repo| &repo.branches.as_ref().unwrap().items)
        .any(predicate)
}
struct Daemon(Child);
impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn start(directory: &Path) -> (Daemon, Value) {
    start_with_parent(directory, false)
}
fn start_with_parent(directory: &Path, parent_pipe: bool) -> (Daemon, Value) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_releashd"));
    command
        .arg("--data-dir")
        .arg(directory)
        .env_remove("RELEASH_DATA_DIR")
        .env("SHELL", "/bin/sh")
        .env("XDG_CONFIG_HOME", directory.join("config"))
        .env("HOME", directory)
        .env("CLAUDE_CONFIG_DIR", directory.join(".claude"))
        .env("CODEX_HOME", directory.join(".codex"))
        .current_dir(directory)
        .stdin(if parent_pipe {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    wait_for_discovery(Daemon(command.spawn().unwrap()), directory)
}

fn wait_for_discovery(mut child: Daemon, directory: &Path) -> (Daemon, Value) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "daemon exited before discovery"
        );
        if let Ok(bytes) = std::fs::read(directory.join("client-api.json")) {
            if let Ok(discovery) = serde_json::from_slice::<Value>(&bytes) {
                if discovery["pid"].as_u64() == Some(child.0.id() as u64) {
                    return (child, discovery);
                }
            }
        }
        assert!(Instant::now() < deadline, "daemon discovery timed out");
        std::thread::sleep(Duration::from_millis(20));
    }
}

async fn connect(discovery: &Value) -> (Socket, String) {
    use connectrpc::client::{ClientConfig, HttpClient};
    let config = ClientConfig::new(
        format!("http://127.0.0.1:{}", discovery["port"])
            .parse()
            .unwrap(),
    )
    .with_default_header(
        "authorization",
        format!("Bearer {}", discovery["token"].as_str().unwrap()),
    );
    let client = rpc::ClientServiceClient::new(HttpClient::plaintext(), config);
    let info = client
        .get_server_info(rpc::Unit::default())
        .await
        .unwrap()
        .into_owned();
    assert_eq!(info.release, env!("CARGO_PKG_VERSION"));
    assert_eq!(info.daemon_id, discovery["daemon_id"].as_str().unwrap());
    assert!(discovery.get("instance_id").is_none());
    assert_eq!(info.pid as u64, discovery["pid"].as_u64().unwrap());
    assert_eq!(
        info.process_started_at,
        discovery["process_started_at"]
            .as_str()
            .unwrap()
            .parse::<u64>()
            .unwrap()
    );
    assert_eq!(info.protocol, 1);
    assert!(info.capabilities.is_empty());
    assert_eq!(info.serving_status, rpc::ServingStatus::Serving);
    (Socket { client }, info.daemon_id)
}

#[cfg(unix)]
#[tokio::test]
async fn test_daemon起動_data_dirの全解決経路で子プロセスへ解決済みpathを渡す() {
    for mode in ["--data-dir", "environment", "default"] {
        // Given
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        let environment = root.join("environment");
        let default_base = if cfg!(target_os = "macos") {
            root.join("Library/Application Support")
        } else {
            root.join("data")
        };
        let resolved = match mode {
            "default" => default_base.join(
                crate::infrastructure::platform::data_dir::default_data_dir_name_for_profile(
                    crate::infrastructure::platform::data_dir::BuildProfile::current(),
                ),
            ),
            "environment" => environment.clone(),
            _ => root.join("explicit"),
        };
        let mut command = Command::new(env!("CARGO_BIN_EXE_releashd"));
        command
            .env("HOME", &root)
            .env("XDG_DATA_HOME", &default_base)
            .env("XDG_CONFIG_HOME", root.join("config"))
            .env("CLAUDE_CONFIG_DIR", root.join(".claude"))
            .env("CODEX_HOME", root.join(".codex"))
            .env("SHELL", "/bin/sh")
            .current_dir(&root)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        if mode == "default" {
            command.env_remove("RELEASH_DATA_DIR");
        } else {
            command.env("RELEASH_DATA_DIR", &environment);
        }
        if mode == "--data-dir" {
            command.arg(mode).arg(&resolved);
        }
        // When
        let (mut daemon, discovery) =
            wait_for_discovery(Daemon(command.spawn().unwrap()), &resolved);
        let (mut socket, _) = connect(&discovery).await;
        request(
            &mut socket,
            "data-dir",
            wire::command_request::Command::GetOrSpawnTerminalSurface(
                wire::GetOrSpawnTerminalSurfaceRequest {
                    rows: Some(24),
                    cols: Some(80),
                    cwd: Some(root.to_string_lossy().into_owned()),
                    owner: Some(wire::TerminalSurfaceOwnerV1 {
                        variant: Some(wire::terminal_surface_owner_v1::Variant::Workspace(
                            wire::TerminalSurfaceOwnerV1Workspace {
                                workspace_path: Some(root.to_string_lossy().into_owned()),
                            },
                        )),
                    }),
                    startup_command: Some(
                        "printf '%s' \"$RELEASH_DATA_DIR\" > child-data-dir".into(),
                    ),
                    ..Default::default()
                },
            ),
        )
        .await;
        // Then
        let expected = resolved.to_string_lossy();
        let deadline = Instant::now() + Duration::from_secs(10);
        while std::fs::read_to_string(root.join("child-data-dir"))
            .ok()
            .as_deref()
            != Some(expected.as_ref())
        {
            assert!(Instant::now() < deadline, "{mode}: child data dir mismatch");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        quit(&mut daemon, &mut socket).await;
    }
}
async fn request(
    socket: &mut Socket,
    _id: &str,
    command: wire::command_request::Command,
) -> wire::command_result::Command {
    call(&socket.client, command).await.unwrap()
}

async fn quit(daemon: &mut Daemon, socket: &mut Socket) {
    let _ = call(
        &socket.client,
        wire::command_request::Command::StopDaemon(wire::StopDaemonRequest {}),
    )
    .await;
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(status) = daemon.0.try_wait().unwrap() {
            assert!(status.success(), "{status}");
            return;
        }
        assert!(
            Instant::now() < deadline,
            "daemon did not finish coordinated shutdown"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_headless単独起動_commandと永続化と再起動とexitを実processで確認する() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let help = Command::new(Path::new(env!("CARGO_BIN_EXE_releashd")).with_file_name("releash"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(help.status.success());
    assert!(!String::from_utf8(help.stdout).unwrap().contains("daemon"));
    let (mut daemon, discovery) = start(directory.path());
    assert_ne!(
        discovery["pid"].as_u64().unwrap(),
        u64::from(std::process::id())
    );
    let duplicate = Command::new(env!("CARGO_BIN_EXE_releashd"))
        .arg("--data-dir")
        .arg(directory.path())
        .env("HOME", directory.path())
        .env("XDG_CONFIG_HOME", directory.path().join("config"))
        .env_remove("RELEASH_DATA_DIR")
        .env("SHELL", "/bin/sh")
        .current_dir(directory.path())
        .output()
        .unwrap();
    assert_eq!(duplicate.status.code(), Some(1));
    let rejection = String::from_utf8(duplicate.stderr).unwrap();
    let failure = rejection
        .lines()
        .find(|line| line.starts_with("Local data is currently in use"))
        .expect("safe startup failure");
    let correlation = failure.rsplit_once('(').unwrap().1.trim_end_matches(')');
    uuid::Uuid::parse_str(correlation).unwrap();
    let local_log = std::fs::read_to_string(directory.path().join("logs/releash.log")).unwrap();
    assert!(
        local_log.contains(&format!("application startup admission failed: {failure}")),
        "{local_log}"
    );
    assert!(local_log.contains("daemon"));
    assert_eq!(
        serde_json::from_slice::<Value>(
            &std::fs::read(directory.path().join("client-api.json")).unwrap()
        )
        .unwrap(),
        discovery
    );
    let (mut socket, instance) = connect(&discovery).await;
    // When / Then
    use wire::command_request::Command as C;
    request(
        &mut socket,
        "save",
        C::UpdateExternalEditor(wire::UpdateExternalEditorRequest {
            editor: Some("daemon-smoke".into()),
        }),
    )
    .await;
    assert_eq!(read_external_editor(&socket).await, "daemon-smoke");
    let status = Command::new(Path::new(env!("CARGO_BIN_EXE_releashd")).with_file_name("releash"))
        .args(["workflow", "status", "550e8400-e29b-41d4-a716-446655440000"])
        .env("RELEASH_DATA_DIR", directory.path())
        .output()
        .unwrap();
    assert_eq!(status.status.code(), Some(1));
    let alias = if cfg!(debug_assertions) {
        "releash-dev"
    } else {
        "releash"
    };
    request(&mut socket, "terminal", C::GetOrSpawnTerminalSurface(wire::GetOrSpawnTerminalSurfaceRequest {
        rows: Some(24), cols: Some(80), cwd: Some(directory.path().to_string_lossy().into_owned()),
        owner: Some(wire::TerminalSurfaceOwnerV1 { variant: Some(wire::terminal_surface_owner_v1::Variant::Workspace(wire::TerminalSurfaceOwnerV1Workspace { workspace_path: Some(directory.path().to_string_lossy().into_owned()) })) }),
        startup_command: Some(format!(r#"printf '%s' "$RELEASH_DATA_DIR" > child-data-dir; command -v {alias} > child-alias; {alias} --help > child-cli; printf '%s' "$$" > child-pid"#)),
        ..Default::default()
    })).await;
    let deadline = Instant::now() + Duration::from_secs(10);
    while !directory.path().join("child-pid").exists() {
        assert!(
            Instant::now() < deadline,
            "terminal startup command did not finish"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(
        std::fs::read_to_string(directory.path().join("child-data-dir")).unwrap(),
        directory.path().to_string_lossy()
    );
    assert_eq!(
        std::fs::read_to_string(directory.path().join("child-alias"))
            .unwrap()
            .trim(),
        directory.path().join("bin").join(alias).to_string_lossy()
    );
    assert!(std::fs::read_to_string(directory.path().join("child-cli"))
        .unwrap()
        .contains("workflow"));
    let wrapper = std::fs::read_to_string(directory.path().join("bin").join(alias)).unwrap();
    assert!(wrapper.contains(
        &Path::new(env!("CARGO_BIN_EXE_releashd"))
            .with_file_name("releash")
            .to_string_lossy()
            .into_owned()
    ));
    let child_pid = std::fs::read_to_string(directory.path().join("child-pid"))
        .unwrap()
        .parse::<i32>()
        .unwrap();
    quit(&mut daemon, &mut socket).await;
    assert_eq!(
        unsafe { libc::kill(child_pid, 0) },
        -1,
        "terminal child survived coordinated shutdown"
    );
    assert!(!directory.path().join("client-api.json").exists());
    let (mut restarted, next_discovery) = start(directory.path());
    assert_ne!(discovery["daemon_id"], next_discovery["daemon_id"]);
    assert_ne!(discovery["token"], next_discovery["token"]);
    let (mut socket, next_instance) = connect(&next_discovery).await;
    assert_ne!(instance, next_instance);
    assert_eq!(read_external_editor(&socket).await, "daemon-smoke");
    assert_eq!(read_external_editor(&socket).await, "daemon-smoke");
    quit(&mut restarted, &mut socket).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_daemon起動_ログ作成失敗でも従来どおりstoreとapiを利用できる() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("logs"), "unavailable log directory").unwrap();
    // When
    let (mut daemon, discovery) = start(directory.path());
    let (mut socket, _) = connect(&discovery).await;
    let wire::state_payload::Value::DesktopSettings(settings) =
        read_state(&socket, "desktop-settings").await
    else {
        panic!("settings")
    };
    // Then
    assert_eq!(settings.close_to_tray, Some(true));
    quit(&mut daemon, &mut socket).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_daemon本番配線_repository一覧が購読へ配信される() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let (mut daemon, discovery) = start(directory.path());
    let (mut socket, _) = connect(&discovery).await;
    let repository = directory
        .path()
        .join("repository")
        .to_string_lossy()
        .into_owned();
    let mut states = socket
        .client
        .open_state_stream(rpc::OpenStateStreamRequest {
            client_id: "repositories".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    states
        .message::<rpc::StateSubscriptionEvent>()
        .await
        .unwrap()
        .unwrap();
    socket
        .client
        .start_state_subscription(rpc::StartStateSubscriptionRequest {
            client_id: "repositories".into(),
            subscription_id: "repositories".into(),
            target: "repository-paths".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    let initial: wire::StateSubscriptionEvent = to_wire(
        &states
            .message::<rpc::StateSubscriptionEvent>()
            .await
            .unwrap()
            .unwrap()
            .to_owned_message(),
    )
    .unwrap();
    assert!(matches!(
        initial.event,
        Some(wire::state_subscription_event::Event::Snapshot(_))
    ));
    states
        .message::<rpc::StateSubscriptionEvent>()
        .await
        .unwrap()
        .unwrap();
    // When
    let result = request(
        &mut socket,
        "add-repo",
        wire::command_request::Command::AddRepoPath(wire::AddRepoPathRequest {
            path: Some(repository.clone()),
        }),
    )
    .await;
    // Then
    let changed = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let event: wire::StateSubscriptionEvent = to_wire(
                &states
                    .message::<rpc::StateSubscriptionEvent>()
                    .await
                    .unwrap()
                    .unwrap()
                    .to_owned_message(),
            )
            .unwrap();
            if let Some(wire::state_subscription_event::Event::Change(change)) = event.event {
                break change;
            }
        }
    })
    .await
    .unwrap();
    assert!(!changed.delta);
    let Some(wire::state_payload::Value::RepositoryPaths(paths)) = changed.payload.unwrap().value
    else {
        panic!("repository paths");
    };
    assert_eq!(paths.items, [repository]);
    drop(states);
    let wire::command_result::Command::AddRepoPath(added) = result else {
        panic!("add repo result");
    };
    assert_eq!(added.value, Some(true));
    let workflows = if cfg!(target_os = "macos") {
        directory
            .path()
            .join("Library/Application Support/releash/workflows")
    } else {
        directory.path().join("config/releash/workflows")
    };
    assert!(workflows.join(".luarc.json").exists());
    assert!(workflows.join(".releash").is_dir());
    quit(&mut daemon, &mut socket).await;
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn test_daemon本番配線_状態を購読へ配信する() {
    use std::os::unix::fs::PermissionsExt;
    use wire::command_request::Command as C;
    // Given
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let repo_path = root.join("repository");
    let repository = git2::Repository::init(&repo_path).unwrap();
    let signature = git2::Signature::now("daemon smoke", "smoke@example.test").unwrap();
    let tree_id = repository.index().unwrap().write_tree().unwrap();
    let commit = repository
        .commit(
            Some("HEAD"),
            &signature,
            &signature,
            "initial",
            &repository.find_tree(tree_id).unwrap(),
            &[],
        )
        .unwrap();
    let worktree = repo_path.to_str().unwrap();
    let fixture = root.join("provider-fixture");
    std::fs::write(&fixture, "#!/bin/sh\nif [ \"$1\" = --version ]; then echo 'claude 1.0.0'; exit; fi\nwhile :; do sleep 3600 & wait $!; done\n").unwrap();
    std::fs::set_permissions(&fixture, std::fs::Permissions::from_mode(0o755)).unwrap();
    let (mut daemon, discovery) = start(&root);
    let (mut socket, _) = connect(&discovery).await;
    request(
        &mut socket,
        "configured-repository",
        C::AddRepoPath(wire::AddRepoPathRequest {
            path: Some(worktree.into()),
        }),
    )
    .await;
    // When / Then: AgentSession notifier
    request(
        &mut socket,
        "provider",
        C::UpdateProviderExecutable(wire::UpdateProviderExecutableRequest {
            provider: Some("claude".into()),
            executable: Some(fixture.to_str().unwrap().into()),
        }),
    )
    .await;
    let selected = request(
        &mut socket,
        "session",
        C::CreateAgentSession(wire::CreateAgentSessionRequest {
            workspace_identity: Some(worktree.into()),
            worktree_path: Some(worktree.into()),
            provider: Some("claude".into()),
            rows: Some(24),
            cols: Some(80),
            caller_request_id: Some(uuid::Uuid::new_v4().to_string()),
        }),
    )
    .await;
    let wire::command_result::Command::CreateAgentSession(selected) = selected else {
        panic!("session creation result");
    };
    let selected_node_id = selected.node_id.unwrap();
    assert_eq!(
        selected.agent_session_id.as_deref(),
        Some(selected_node_id.as_str())
    );
    // 購読の最初の値は取得中で届くことがある。走査と実行木の読み取りが終わった値を待つ。
    let mut states = subscribe_workspaces(&socket).await;
    let snapshot = expect_workspace(&mut states, "initial execution tree", |value| {
        find_worktree(value, worktree).is_some_and(|tree| tree.snapshot.is_some())
    })
    .await;
    let tree = worktree_state(&snapshot, worktree)
        .snapshot
        .as_ref()
        .unwrap();
    let node_id = tree
        .nodes
        .as_ref()
        .unwrap()
        .items
        .iter()
        .find_map(|item| match &item.variant {
            Some(wire::workspace_tree_item::Variant::Node(node)) => node.id.clone(),
            _ => None,
        })
        .expect("standalone session node");
    assert_eq!(node_id, selected_node_id);
    request(
        &mut socket,
        "rename-session",
        C::RenameWorkspaceSessionNode(wire::RenameWorkspaceSessionNodeRequest {
            worktree_path: Some(worktree.into()),
            node_id: Some(node_id.clone()),
            name: Some("subscription verification".into()),
        }),
    )
    .await;
    expect_workspace(&mut states, "session rename", |value| worktree_state(value, worktree).snapshot.as_ref().unwrap().nodes.as_ref().unwrap().items.iter().any(|item| matches!(&item.variant, Some(wire::workspace_tree_item::Variant::Node(node)) if node.id.as_deref() == Some(&node_id) && node.title.as_deref() == Some("subscription verification")))).await;
    // When / Then: workflow notifier
    let workflows = if cfg!(target_os = "macos") {
        root.join("Library/Application Support/releash/workflows")
    } else {
        root.join("config/releash/workflows")
    };
    std::fs::write(
        workflows.join("push-smoke.yml"),
        format!("name: push-smoke\ndescription: push smoke\nnodes:\n  main:\n    command: while [ ! -f '{}' ]; do sleep 0.01; done; printf done\n    completion:\n      require: approval\n", root.join("output-ready").to_str().unwrap().replace('\'', "'\\''")),
    )
    .unwrap();
    let diagnosed = request(
        &mut socket,
        "diagnose-directory",
        C::DiagnoseWorkflowDirectory(wire::DiagnoseWorkflowDirectoryRequest {
            dir: Some(workflows.to_str().unwrap().into()),
        }),
    )
    .await;
    let wire::command_result::Command::DiagnoseWorkflowDirectory(diagnosed) = diagnosed else {
        panic!("diagnostic report");
    };
    assert!(diagnosed.report.is_some());
    for (dir, expected) in [
        ("relative".into(), connectrpc::ErrorCode::InvalidArgument),
        (
            root.join("missing").to_str().unwrap().to_string(),
            connectrpc::ErrorCode::NotFound,
        ),
    ] {
        assert_eq!(
            call(
                &socket.client,
                C::DiagnoseWorkflowDirectory(wire::DiagnoseWorkflowDirectoryRequest {
                    dir: Some(dir)
                })
            )
            .await
            .unwrap_err()
            .code,
            expected
        );
    }
    let workflow_result = request(
        &mut socket,
        "workflow",
        C::StartWorkflow(wire::StartWorkflowRequest {
            workflow_name: Some("push-smoke".into()),
            worktree_path: Some(worktree.into()),
            ..Default::default()
        }),
    )
    .await;
    let wire::command_result::Command::StartWorkflow(workflow) = workflow_result else {
        panic!("workflow start result");
    };
    let execution_id = workflow.value.unwrap();
    expect_workspace(&mut states, "workflow start", |value| worktree_state(value, worktree).snapshot.as_ref().unwrap().nodes.as_ref().unwrap().items.iter().any(|item| matches!(&item.variant, Some(wire::workspace_tree_item::Variant::Node(node)) if node.id.as_deref() == Some(&execution_id) && node.title.as_deref() == Some("push-smoke")))).await;

    let mut execution =
        subscribe_state(&socket, "workflow-execution", vec![execution_id.clone()]).await;
    expect_state(&mut execution, "execution by id", |value| matches!(value, wire::state_payload::Value::WorkflowExecution(snapshot) if snapshot.value.is_some())).await;
    let mut output = subscribe_state(
        &socket,
        "workflow-output",
        vec![execution_id.clone(), "main".into()],
    )
    .await;
    expect_state(&mut output, "unsubmitted output", |value| matches!(value, wire::state_payload::Value::WorkflowOutput(snapshot) if snapshot.value.as_ref().is_some_and(|output| matches!(output.variant, Some(wire::workflow_output_view::Variant::NotSubmitted(_)))))).await;
    std::fs::write(root.join("output-ready"), "ready").unwrap();
    let mut missing = subscribe_state(
        &socket,
        "workflow-execution",
        vec![uuid::Uuid::new_v4().to_string()],
    )
    .await;
    expect_state(&mut missing, "missing execution", |value| matches!(value, wire::state_payload::Value::WorkflowExecution(snapshot) if snapshot.value.is_none())).await;
    let missing_id = uuid::Uuid::new_v4().to_string();
    let mut missing_output = subscribe_state(
        &socket,
        "workflow-output",
        vec![missing_id.clone(), "main".into()],
    )
    .await;
    expect_state(&mut missing_output, "missing output", |value| matches!(value, wire::state_payload::Value::WorkflowOutput(snapshot) if snapshot.value.is_none())).await;
    let mut missing_session =
        subscribe_state(&socket, "review-session-threads", vec![missing_id.clone()]).await;
    expect_state(&mut missing_session, "missing session", |value| matches!(value, wire::state_payload::Value::ReviewSessionThreads(snapshot) if snapshot.value.is_none())).await;
    let mut missing_thread = subscribe_state(
        &socket,
        "review-session-thread",
        vec![selected_node_id.clone(), missing_id.clone()],
    )
    .await;
    expect_state(&mut missing_thread, "missing thread", |value| matches!(value, wire::state_payload::Value::ReviewSessionThread(snapshot) if snapshot.value.is_none())).await;
    let mut missing_history = subscribe_state(
        &socket,
        "review-session-thread-history",
        vec![selected_node_id.clone(), missing_id],
    )
    .await;
    expect_state(&mut missing_history, "missing history", |value| matches!(value, wire::state_payload::Value::ReviewSessionThreadHistory(snapshot) if snapshot.value.is_none())).await;
    drop((
        missing_output,
        missing_session,
        missing_thread,
        missing_history,
    ));
    let mut session_threads = subscribe_state(
        &socket,
        "review-session-threads",
        vec![
            selected_node_id.clone(),
            "state=open".into(),
            "author=self".into(),
        ],
    )
    .await;
    expect_state(&mut session_threads, "empty session threads", |value| matches!(value, wire::state_payload::Value::ReviewSessionThreads(snapshot) if snapshot.value.as_ref().is_some_and(|list| list.items.is_empty()))).await;
    let created = request(
        &mut socket,
        "session-comment",
        C::CreateSessionReviewThread(wire::CreateSessionReviewThreadRequest {
            session_id: Some(selected_node_id.clone()),
            content: Some("session comment".into()),
            ..Default::default()
        }),
    )
    .await;
    let wire::command_result::Command::CreateSessionReviewThread(created) = created else {
        panic!("session thread");
    };
    let thread_id = created.thread.unwrap().id.unwrap();
    expect_state(&mut session_threads, "session thread created", |value| matches!(value, wire::state_payload::Value::ReviewSessionThreads(snapshot) if snapshot.value.as_ref().is_some_and(|list| list.items.iter().any(|thread| thread.id.as_deref() == Some(&thread_id))))).await;
    let mut worktree_threads = subscribe_state(
        &socket,
        "review-worktree-threads",
        vec![
            worktree.into(),
            "state=open".into(),
            format!("thread={thread_id}"),
        ],
    )
    .await;
    expect_state(&mut worktree_threads, "worktree threads share session store", |value| matches!(value, wire::state_payload::Value::ReviewThreads(list) if list.items.len() == 1 && list.items[0].id.as_deref() == Some(&thread_id))).await;
    drop(worktree_threads);
    let mut history = subscribe_state(
        &socket,
        "review-session-thread-history",
        vec![selected_node_id.clone(), thread_id.clone()],
    )
    .await;
    expect_state(&mut history, "created history", |value| matches!(value, wire::state_payload::Value::ReviewSessionThreadHistory(snapshot) if snapshot.value.as_ref().is_some_and(|list| list.items.len() == 1))).await;
    request(
        &mut socket,
        "append-session-comment",
        C::AppendSessionReviewComment(wire::AppendSessionReviewCommentRequest {
            session_id: Some(selected_node_id.clone()),
            thread_id: Some(thread_id.clone()),
            content: Some("follow up".into()),
        }),
    )
    .await;
    expect_state(&mut history, "appended history", |value| matches!(value, wire::state_payload::Value::ReviewSessionThreadHistory(snapshot) if snapshot.value.as_ref().is_some_and(|list| list.items.len() == 2))).await;
    request(
        &mut socket,
        "resolve-session-comment",
        C::ResolveSessionReviewThread(wire::ResolveSessionReviewThreadRequest {
            session_id: Some(selected_node_id.clone()),
            thread_id: Some(thread_id.clone()),
            outcome: Some("fixed".into()),
            summary: Some("addressed".into()),
        }),
    )
    .await;
    expect_state(&mut session_threads, "resolved thread excluded", |value| matches!(value, wire::state_payload::Value::ReviewSessionThreads(snapshot) if snapshot.value.as_ref().is_some_and(|list| list.items.is_empty()))).await;
    let mut thread = subscribe_state(
        &socket,
        "review-session-thread",
        vec![selected_node_id.clone(), thread_id.clone()],
    )
    .await;
    expect_state(&mut thread, "resolved thread by id", |value| matches!(value, wire::state_payload::Value::ReviewSessionThread(snapshot) if snapshot.value.as_ref().is_some_and(|thread| thread.state.as_ref().and_then(|state| state.value) == Some(wire::review_thread_state_dto::Value::Resolved as i32)))).await;
    request(
        &mut socket,
        "delete-test-session-thread",
        C::DeleteReviewThread(wire::DeleteReviewThreadRequest {
            worktree_name: Some(worktree.into()),
            thread_id: Some(thread_id),
        }),
    )
    .await;
    expect_state(&mut thread, "deleted thread", |value| matches!(value, wire::state_payload::Value::ReviewSessionThread(snapshot) if snapshot.value.is_none())).await;
    expect_state(&mut history, "deleted thread history", |value| matches!(value, wire::state_payload::Value::ReviewSessionThreadHistory(snapshot) if snapshot.value.is_none())).await;
    drop((execution, output, missing, session_threads, history, thread));

    // When / Then: review threads reach the subscription after a comment command
    let mut threads = subscribe_state(&socket, "review-threads", vec!["repository".into()]).await;
    expect_state(&mut threads, "initial review threads", |value| matches!(value, wire::state_payload::Value::ReviewThreads(list) if list.items.is_empty())).await;
    request(
        &mut socket,
        "comment",
        C::CreateReviewThread(wire::CreateReviewThreadRequest {
            worktree_name: Some("repository".into()),
            content: Some("production comment".into()),
            ..Default::default()
        }),
    )
    .await;
    expect_state(&mut threads, "comment creation", |value| matches!(value, wire::state_payload::Value::ReviewThreads(list) if list.items.iter().any(|thread| thread.comments.as_ref().unwrap().items.iter().any(|comment| comment.content.as_deref() == Some("production comment"))))).await;
    // When / Then: the subscription owns the review-comments watch, without a comment command
    for entry in std::fs::read_dir(root.join("review-comments")).unwrap() {
        let path = entry.unwrap().path();
        if path.to_string_lossy().ends_with(".events.json") {
            std::fs::write(&path, "[]").unwrap();
        }
    }
    expect_state(&mut threads, "external comment edit", |value| matches!(value, wire::state_payload::Value::ReviewThreads(list) if list.items.is_empty())).await;
    // When / Then: the review subscription owns the worktree watch.
    let mut review = subscribe_state(
        &socket,
        "review-snapshot",
        vec![worktree.into(), "head".into()],
    )
    .await;
    review.next().await.unwrap();
    std::fs::write(repo_path.join("smoke.txt"), "smoke").unwrap();
    expect_state(&mut review, "review snapshot after file change", |value| matches!(value, wire::state_payload::Value::ReviewSnapshot(snapshot) if snapshot.changed_files.as_ref().unwrap().items.iter().any(|file| file.path.as_deref() == Some("smoke.txt")))).await;
    // Given: a branch merged into the default branch through a merge commit.
    let side = repository
        .commit(
            None,
            &signature,
            &signature,
            "side",
            &repository.find_tree(tree_id).unwrap(),
            &[&repository.find_commit(commit).unwrap()],
        )
        .unwrap();
    repository
        .branch(
            "pushed-branch",
            &repository.find_commit(side).unwrap(),
            false,
        )
        .unwrap();
    repository
        .commit(
            Some("HEAD"),
            &signature,
            &signature,
            "merge",
            &repository.find_tree(tree_id).unwrap(),
            &[
                &repository.find_commit(commit).unwrap(),
                &repository.find_commit(side).unwrap(),
            ],
        )
        .unwrap();
    // When / Then: the workspace subscription owns the repository watch.
    assert!(!repository.path().join("worktrees").exists());
    let created = request(
        &mut socket,
        "create-first-linked",
        C::CreateWorktree(wire::CreateWorktreeRequest {
            repo_path: Some(worktree.into()),
            branch: Some("pushed-branch".into()),
            create_branch: Some(false),
            base_branch: None,
        }),
    )
    .await;
    let wire::command_result::Command::CreateWorktree(created) = created else {
        panic!("worktree creation result")
    };
    let linked = created.value.unwrap();
    expect_workspace(&mut states, "first linked worktree creation", |value| {
        any_branch(value, |branch| {
            branch.name.as_deref() == Some("pushed-branch")
                && branch.worktree_path.as_deref() == Some(linked.as_str())
                && branch.is_merged == Some(true)
        })
    })
    .await;
    // When / Then: only Refresh rescans the base configuration, which Git watches ignore.
    for base in [Some("pushed-branch"), None] {
        let mut config = repository.config().unwrap();
        match base {
            Some(base) => config.set_str("releash.base", base).unwrap(),
            None => config.remove("releash.base").unwrap(),
        }
        let result = request(
            &mut socket,
            "refresh-workspaces",
            C::RefreshWorkspaces(wire::RefreshWorkspacesRequest::default()),
        )
        .await;
        assert!(matches!(
            result,
            wire::command_result::Command::RefreshWorkspaces(_)
        ));
        expect_workspace(&mut states, "manual workspace rescan", |value| {
            any_branch(value, |branch| {
                branch.name.as_deref() == Some("pushed-branch")
                    && branch.is_merged == Some(base.is_none())
            })
        })
        .await;
    }
    request(
        &mut socket,
        "remove-linked",
        C::RemoveWorktree(wire::RemoveWorktreeRequest {
            repo_path: Some(worktree.into()),
            worktree_path: Some(linked.clone()),
            force: Some(true),
        }),
    )
    .await;
    expect_workspace(&mut states, "linked worktree removal", |value| {
        !any_branch(value, |branch| {
            branch.worktree_path.as_deref() == Some(linked.as_str())
        })
    })
    .await;
    let external = root.join("external-worktree");
    let reference = repository
        .find_reference("refs/heads/pushed-branch")
        .unwrap();
    let mut options = git2::WorktreeAddOptions::new();
    options.reference(Some(&reference));
    repository
        .worktree("external", &external, Some(&options))
        .unwrap();
    let external = external
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    expect_workspace(&mut states, "external linked worktree creation", |value| {
        any_branch(value, |branch| {
            branch.worktree_path.as_deref() == Some(external.as_str())
        })
    })
    .await;
    let db = rusqlite::Connection::open_with_flags(
        root.join("local-event-store.sqlite3"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let sessions = || {
        db.prepare(
            "SELECT event_type, detail FROM node_events WHERE node_execution_id = ? ORDER BY seq",
        )
        .unwrap()
        .query_map([&node_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
    };
    let before_quit = sessions();
    assert!(before_quit
        .iter()
        .any(|(event, _)| event == "session_attached"));
    assert!(!before_quit
        .iter()
        .any(|(event, _)| event == "process_exited"));
    quit(&mut daemon, &mut socket).await;
    assert_eq!(
        sessions(),
        before_quit,
        "terminal shutdown must preserve AgentSession state"
    );
    // Given: an unfinished workflow and its legacy archive file survive daemon shutdown.
    let read_workflow_facts = || {
        db.prepare("SELECT event_type, detail FROM node_events WHERE tree_id = ? AND parent_id IS NULL ORDER BY seq")
            .unwrap()
            .query_map([&execution_id], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    };
    assert!(!read_workflow_facts()
        .iter()
        .any(|(event, _)| event == "abort_requested" || event == "archive_requested"));
    let legacy_path = root.join("workflow_execution_archives.json");
    std::fs::write(&legacy_path, serde_json::json!({"executions": {execution_id.clone(): {"archivedAt": 12.345678, "archiveReason": "manual"}}}).to_string()).unwrap();
    // When: the production daemon startup invokes archive migration.
    let (mut restarted, discovery) = start(&root);
    let (mut socket, _) = connect(&discovery).await;
    // Then
    assert!(!legacy_path.exists());
    let facts = read_workflow_facts();
    let aborted = facts
        .iter()
        .position(|(event, _)| event == "abort_requested")
        .unwrap();
    let archived = facts
        .iter()
        .position(|(event, _)| event == "archive_requested")
        .unwrap();
    assert!(aborted < archived);
    let archive: Value = serde_json::from_str(&facts[archived].1).unwrap();
    assert_eq!(archive["archivedAt"], 12.345678);
    assert_eq!(archive["reason"], "manual");
    let mut restarted_states = subscribe_workspaces(&socket).await;
    let snapshot = expect_workspace(&mut restarted_states, "restarted execution tree", |value| {
        find_worktree(value, worktree).is_some_and(|tree| tree.snapshot.is_some())
    })
    .await;
    let history = &worktree_state(&snapshot, worktree)
        .workflow_history
        .as_ref()
        .unwrap()
        .items;
    let archived = history
        .iter()
        .find(|item| item.execution_id.as_deref() == Some(&execution_id))
        .unwrap();
    assert_eq!(
        archived.status.as_ref().unwrap().value,
        Some(wire::workspace_history_status::Value::Aborted as i32)
    );
    quit(&mut restarted, &mut socket).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_親ui終了_stdinを閉じてもdaemonとterminalが動き続ける() {
    let directory = tempfile::tempdir().unwrap();
    let (mut daemon, discovery) = start_with_parent(directory.path(), true);
    let (mut socket, _) = connect(&discovery).await;
    request(
        &mut socket,
        "terminal-parent",
        wire::command_request::Command::GetOrSpawnTerminalSurface(
            wire::GetOrSpawnTerminalSurfaceRequest {
                rows: Some(24),
                cols: Some(80),
                cwd: Some(directory.path().to_string_lossy().into_owned()),
                owner: Some(wire::TerminalSurfaceOwnerV1 {
                    variant: Some(wire::terminal_surface_owner_v1::Variant::Workspace(
                        wire::TerminalSurfaceOwnerV1Workspace {
                            workspace_path: Some(directory.path().to_string_lossy().into_owned()),
                        },
                    )),
                }),
                startup_command: Some(
                    "sleep 100 & echo $! > grandchild-pid; printf '%s' \"$$\" > child-pid".into(),
                ),
                ..Default::default()
            },
        ),
    )
    .await;
    let deadline = Instant::now() + Duration::from_secs(10);
    while !directory.path().join("child-pid").exists() {
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let pids: Vec<i32> = ["child-pid", "grandchild-pid"]
        .iter()
        .map(|name| {
            std::fs::read_to_string(directory.path().join(name))
                .unwrap()
                .trim()
                .parse()
                .unwrap()
        })
        .collect();
    drop(daemon.0.stdin.take());
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(daemon.0.try_wait().unwrap().is_none());
    assert!(directory.path().join("client-api.json").exists());
    for pid in pids {
        assert_eq!(unsafe { libc::kill(pid, 0) }, 0);
    }
    socket
        .client
        .get_server_info(rpc::Unit::default())
        .await
        .unwrap();
    quit(&mut daemon, &mut socket).await;
    assert!(!directory.path().join("client-api.json").exists());
}

#[tokio::test]
async fn test_connect提出_worktreeなしで待機中nodeにartifactを記録する() {
    use std::os::unix::fs::PermissionsExt;
    use wire::command_request::Command as C;
    // Given
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let repo_path = root.join("repository");
    let repository = git2::Repository::init(&repo_path).unwrap();
    let signature = git2::Signature::now("daemon smoke", "smoke@example.test").unwrap();
    let tree_id = repository.index().unwrap().write_tree().unwrap();
    repository
        .commit(
            Some("HEAD"),
            &signature,
            &signature,
            "initial",
            &repository.find_tree(tree_id).unwrap(),
            &[],
        )
        .unwrap();
    let worktree = repo_path.to_str().unwrap();
    let fixture = root.join("provider-fixture");
    std::fs::write(&fixture, "#!/bin/sh\nif [ \"$1\" = --version ]; then echo 'claude 1.0.0'; exit; fi\nwhile :; do sleep 3600 & wait $!; done\n").unwrap();
    std::fs::set_permissions(&fixture, std::fs::Permissions::from_mode(0o755)).unwrap();
    let (mut daemon, discovery) = start(&root);
    let (mut socket, _) = connect(&discovery).await;
    request(
        &mut socket,
        "repository",
        C::AddRepoPath(wire::AddRepoPathRequest {
            path: Some(worktree.into()),
        }),
    )
    .await;
    request(
        &mut socket,
        "provider",
        C::UpdateProviderExecutable(wire::UpdateProviderExecutableRequest {
            provider: Some("claude".into()),
            executable: Some(fixture.to_str().unwrap().into()),
        }),
    )
    .await;
    let workflows = if cfg!(target_os = "macos") {
        root.join("Library/Application Support/releash/workflows")
    } else {
        root.join("config/releash/workflows")
    };
    let instructions = workflows.join("instructions");
    std::fs::create_dir_all(&instructions).unwrap();
    std::fs::write(
        instructions.join("submit-smoke.md"),
        "Submit the result artifact.",
    )
    .unwrap();
    std::fs::write(workflows.join("submit-smoke.yml"), "name: submit-smoke\ndescription: Connect artifact submission\nschemas:\n  result: {type: object, properties: {result: {type: string}}, required: [result]}\nnodes:\n  main:\n    session: {provider: claude, facets: {instruction: submit-smoke}}\n    artifact: result\n").unwrap();
    let started = request(
        &mut socket,
        "workflow",
        C::StartWorkflow(wire::StartWorkflowRequest {
            workflow_name: Some("submit-smoke".into()),
            worktree_path: Some(worktree.into()),
            ..Default::default()
        }),
    )
    .await;
    let wire::command_result::Command::StartWorkflow(started) = started else {
        panic!("workflow start");
    };
    let execution_id = started.value.unwrap();
    let mut execution =
        subscribe_state(&socket, "workflow-execution", vec![execution_id.clone()]).await;
    let state = expect_state(&mut execution, "artifact waiting session", |value| matches!(value, wire::state_payload::Value::WorkflowExecution(snapshot) if snapshot.value.as_ref().is_some_and(|view| view.node_executions.as_ref().is_some_and(|nodes| nodes.items.iter().any(|node| node.session_id.is_some() && node.has_artifact == Some(false)))))).await;
    let wire::state_payload::Value::WorkflowExecution(state) = state else {
        unreachable!()
    };
    let node_id = state
        .value
        .unwrap()
        .node_executions
        .unwrap()
        .items
        .into_iter()
        .find(|node| node.node_name.as_deref() == Some("main"))
        .unwrap()
        .id
        .unwrap();
    let value = wire::WorkflowValue {
        variant: Some(wire::workflow_value::Variant::ObjectValue(
            wire::WorkflowValueObject {
                entries: [(
                    "result".into(),
                    wire::WorkflowValue {
                        variant: Some(wire::workflow_value::Variant::StringValue(
                            wire::ResultString {
                                value: Some("accepted".into()),
                            },
                        )),
                    },
                )]
                .into_iter()
                .collect(),
            },
        )),
    };
    // When
    request(
        &mut socket,
        "submit",
        C::WorkflowSubmitOutput(wire::WorkflowSubmitOutputRequest {
            node_execution_id: Some(node_id.clone()),
            artifact: Some(wire::WorkflowSubmitArtifactInput {
                contract: Some("result".into()),
                value: Some(value.clone()),
            }),
        }),
    )
    .await;
    // Then
    expect_state(&mut execution, "artifact recorded on target node", |state| matches!(state, wire::state_payload::Value::WorkflowExecution(snapshot) if snapshot.value.as_ref().is_some_and(|view| view.node_executions.as_ref().is_some_and(|nodes| nodes.items.iter().any(|node| node.id.as_deref() == Some(&node_id) && node.has_artifact == Some(true) && node.artifact.as_ref().is_some_and(|artifact| artifact.contract.as_deref() == Some("result") && artifact.value.as_ref() == Some(&value))))))).await;
    let mut output = subscribe_state(
        &socket,
        "workflow-output",
        vec![execution_id, "main".into()],
    )
    .await;
    expect_state(&mut output, "submitted output", |state| matches!(state, wire::state_payload::Value::WorkflowOutput(snapshot) if snapshot.value.as_ref().is_some_and(|output| matches!(&output.variant, Some(wire::workflow_output_view::Variant::Submitted(output)) if output.contract.as_deref() == Some("result") && output.structured_output.as_ref() == Some(&value))))).await;
    drop((execution, output));
    quit(&mut daemon, &mut socket).await;
}

#[tokio::test]
async fn test_daemon状態購読_server_infoと一致し停止受理後にstoppingを配信する() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let (mut daemon, discovery) = start(directory.path());
    let (socket, _) = connect(&discovery).await;
    let response = socket
        .client
        .get_server_info(rpc::Unit::default())
        .await
        .unwrap();
    let info: wire::ServerInfo = to_wire(&response.into_owned()).unwrap();
    let eligibility = info.cli_installation.as_ref().unwrap();
    assert_eq!(
        eligibility.status,
        wire::CliInstallationStatus::Development as i32
    );
    let error = socket
        .client
        .install_cli(rpc::InstallCliRequest::default())
        .await
        .unwrap_err();
    assert_eq!(error.code, connectrpc::ErrorCode::FailedPrecondition);
    assert_eq!(error.message.as_deref(), Some(eligibility.reason.as_str()));
    let gui = directory
        .path()
        .join("AppTranslocation/id/Releash.app/Contents/MacOS/releash-desktop");
    std::fs::create_dir_all(gui.parent().unwrap()).unwrap();
    std::fs::write(&gui, "").unwrap();
    let response = socket
        .client
        .check_login_registration(rpc::CheckLoginRegistrationRequest {
            executable_path: gui.to_str().unwrap().into(),
            ..Default::default()
        })
        .await
        .unwrap();
    let registration: wire::LoginRegistrationResult = to_wire(&response.into_owned()).unwrap();
    assert_eq!(
        registration.status,
        if cfg!(target_os = "macos") {
            wire::LoginRegistrationStatus::Translocated
        } else {
            wire::LoginRegistrationStatus::Allowed
        } as i32
    );
    let response = socket
        .client
        .check_login_registration(rpc::CheckLoginRegistrationRequest {
            executable_path: std::env::current_exe().unwrap().to_str().unwrap().into(),
            ..Default::default()
        })
        .await
        .unwrap();
    let registration: wire::LoginRegistrationResult = to_wire(&response.into_owned()).unwrap();
    assert_eq!(
        registration.status,
        wire::LoginRegistrationStatus::Allowed as i32
    );
    let mut stream = subscribe_state(&socket, "daemon-info", vec![]).await;
    // When / Then
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), stream.next())
            .await
            .unwrap()
            .unwrap(),
        wire::state_payload::Value::DaemonInfo(info.clone())
    );
    assert_eq!(info.serving_status, wire::ServingStatus::Serving as i32);
    socket
        .client
        .stop_daemon(rpc::StopDaemonRequest::default())
        .await
        .unwrap();
    let mut stopping = info;
    stopping.serving_status = wire::ServingStatus::Stopping as i32;
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), stream.next())
            .await
            .unwrap()
            .unwrap(),
        wire::state_payload::Value::DaemonInfo(stopping)
    );
    drop(stream);
    let deadline = Instant::now() + Duration::from_secs(20);
    while daemon.0.try_wait().unwrap().is_none() {
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(!directory.path().join("client-api.json").exists());
}
