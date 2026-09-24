#![cfg(not(feature = "desktop"))]

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use serde_json::Value;

pub mod wire {
    include!(concat!(env!("OUT_DIR"), "/releash.client.v1.rs"));
}

mod generated {
    include!(concat!(env!("OUT_DIR"), "/connect/mod.rs"));
}
use generated::releash::client::v1 as rpc;
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
    push: std::pin::Pin<Box<dyn futures_util::Stream<Item = wire::Push> + Send>>,
    subscription_id: String,
}
type WorkspaceStream =
    std::pin::Pin<Box<dyn futures_util::Stream<Item = wire::WorkspaceListSnapshotDto> + Send>>;
async fn subscribe_workspaces(socket: &Socket) -> WorkspaceStream {
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
            target: "workspaces".into(),
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
                if let Some(wire::StatePayload {
                    value: Some(wire::state_payload::Value::Workspaces(value)),
                }) = payload
                {
                    return Some((value, stream));
                }
            }
        },
    ))
}
async fn expect_workspace(
    stream: &mut WorkspaceStream,
    phase: &str,
    predicate: impl Fn(&wire::WorkspaceListSnapshotDto) -> bool,
) -> wire::WorkspaceListSnapshotDto {
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
fn worktree_state<'a>(
    value: &'a wire::WorkspaceListSnapshotDto,
    path: &str,
) -> &'a wire::WorkspaceWorktreeListDto {
    value
        .repositories
        .as_ref()
        .unwrap()
        .items
        .iter()
        .flat_map(|repo| &repo.worktrees.as_ref().unwrap().items)
        .find(|tree| tree.path.as_deref() == Some(path))
        .unwrap()
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
    let mut command = Command::new(env!("CARGO_BIN_EXE_releash-backend"));
    command
        .arg("--internal-daemon")
        .arg(directory)
        .env(
            "RELEASH_DATA_DIR",
            directory.join(if cfg!(target_os = "macos") {
                "Library/Application Support/com.releash.app.performance"
            } else {
                ".local/share/com.releash.app.performance"
            }),
        )
        .env("SHELL", "/bin/sh")
        .env("XDG_CONFIG_HOME", directory.join("config"))
        .env("HOME", directory)
        .env("CLAUDE_CONFIG_DIR", directory.join(".claude"))
        .env("CODEX_HOME", directory.join(".codex"))
        .current_dir(directory)
        .env("RELEASH_DAEMON_LAUNCH_ID", uuid::Uuid::new_v4().to_string())
        .stdin(if parent_pipe {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    if parent_pipe {
        command.env("RELEASH_DAEMON_PARENT_PIPE", "1");
    }
    let mut child = Daemon(command.spawn().unwrap());
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
    )
    .with_default_header("origin", "tauri://localhost");
    let client = rpc::ClientServiceClient::new(HttpClient::plaintext(), config);
    let info = client
        .get_server_info(rpc::Unit::default())
        .await
        .unwrap()
        .into_owned();
    assert!(!info.launch_id.is_empty());
    assert_eq!(info.release, env!("CARGO_PKG_VERSION"));
    let subscription_id = uuid::Uuid::new_v4().to_string();
    let mut stream = client
        .subscribe_push(rpc::SubscribePushRequest {
            subscription_id: subscription_id.clone(),
            ..Default::default()
        })
        .await
        .unwrap();
    let initial = stream
        .message::<rpc::Push>()
        .await
        .unwrap()
        .unwrap()
        .to_owned_message();
    assert!(matches!(initial.event, Some(rpc::push::Event::Resync(_))));
    let push = Box::pin(futures_util::stream::unfold(
        stream,
        |mut stream| async move {
            stream
                .message::<rpc::Push>()
                .await
                .unwrap()
                .map(|message| (to_wire(&message.to_owned_message()).unwrap(), stream))
        },
    ));
    (
        Socket {
            client,
            push,
            subscription_id,
        },
        info.launch_id,
    )
}
async fn request(
    socket: &mut Socket,
    _id: &str,
    command: wire::command_request::Command,
) -> wire::command_result::Command {
    call(&socket.client, command).await.unwrap()
}

async fn quit(daemon: &mut Daemon, socket: &mut Socket, restart: bool) {
    let _ = call(
        &socket.client,
        wire::command_request::Command::RequestApplicationQuit(
            wire::RequestApplicationQuitRequest {
                request: Some(wire::ApplicationQuitRequestDtoV1 {
                    intent: Some(wire::ApplicationQuitIntentDtoV1 {
                        variant: Some(if restart {
                            wire::application_quit_intent_dto_v1::Variant::Restart(
                                wire::ApplicationQuitIntentDtoV1Restart { code: Some(0) },
                            )
                        } else {
                            wire::application_quit_intent_dto_v1::Variant::Exit(
                                wire::ApplicationQuitIntentDtoV1Exit { code: Some(0) },
                            )
                        }),
                    }),
                }),
            },
        ),
    )
    .await;
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(status) = daemon.0.try_wait().unwrap() {
            assert!(status.success(), "{status}");
            let mut proof = String::new();
            std::io::Read::read_to_string(&mut daemon.0.stdout.take().unwrap(), &mut proof)
                .unwrap();
            assert!(proof
                .lines()
                .any(|line| line == "releash-shutdown-complete"));
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
    let help = Command::new(env!("CARGO_BIN_EXE_releash-backend"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(help.status.success());
    assert!(!String::from_utf8(help.stdout).unwrap().contains("daemon"));
    let (mut daemon, discovery) = start(directory.path());
    let master: Value =
        serde_json::from_slice(&std::fs::read(directory.path().join("local-api.json")).unwrap())
            .unwrap();
    assert_ne!(master["token"], discovery["token"]);
    assert_ne!(
        master["pid"].as_u64().unwrap(),
        u64::from(std::process::id())
    );
    let duplicate = Command::new(env!("CARGO_BIN_EXE_releash-backend"))
        .arg("--internal-daemon")
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
            &std::fs::read(directory.path().join("local-api.json")).unwrap()
        )
        .unwrap(),
        master
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
    let settings = request(
        &mut socket,
        "read",
        C::GetAppSettings(wire::GetAppSettingsRequest {}),
    )
    .await;
    let wire::command_result::Command::GetAppSettings(settings) = settings else {
        panic!("settings")
    };
    assert_eq!(settings.external_editor.as_deref(), Some("daemon-smoke"));
    let status = Command::new(env!("CARGO_BIN_EXE_releash-backend"))
        .args(["workflow", "status", "550e8400-e29b-41d4-a716-446655440000"])
        .env("RELEASH_DATA_DIR", directory.path())
        .output()
        .unwrap();
    assert_eq!(status.status.code(), Some(4));
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
    assert!(wrapper.contains(env!("CARGO_BIN_EXE_releash-backend")));
    let child_pid = std::fs::read_to_string(directory.path().join("child-pid"))
        .unwrap()
        .parse::<i32>()
        .unwrap();
    quit(&mut daemon, &mut socket, false).await;
    assert_eq!(
        unsafe { libc::kill(child_pid, 0) },
        -1,
        "terminal child survived coordinated shutdown"
    );
    assert!(!directory.path().join("local-api.json").exists());
    assert!(!directory.path().join("client-api.json").exists());
    let (mut restarted, next_discovery) = start(directory.path());
    assert_ne!(discovery["instance_id"], next_discovery["instance_id"]);
    assert_ne!(discovery["token"], next_discovery["token"]);
    let (mut socket, next_instance) = connect(&next_discovery).await;
    assert_ne!(instance, next_instance);
    let settings = request(
        &mut socket,
        "after-restart",
        C::GetAppSettings(wire::GetAppSettingsRequest {}),
    )
    .await;
    let wire::command_result::Command::GetAppSettings(settings) = settings else {
        panic!("settings")
    };
    assert_eq!(settings.external_editor.as_deref(), Some("daemon-smoke"));
    let settings = request(
        &mut socket,
        "no-replay",
        C::GetAppSettings(wire::GetAppSettingsRequest {}),
    )
    .await;
    let wire::command_result::Command::GetAppSettings(settings) = settings else {
        panic!("settings")
    };
    assert_eq!(settings.external_editor.as_deref(), Some("daemon-smoke"));
    quit(&mut restarted, &mut socket, true).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_daemon起動_ログ作成失敗でも従来どおりstoreとapiを利用できる() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("logs"), "unavailable log directory").unwrap();
    // When
    let (mut daemon, discovery) = start(directory.path());
    let (mut socket, _) = connect(&discovery).await;
    let settings = request(
        &mut socket,
        "log-free-read",
        wire::command_request::Command::GetAppSettings(wire::GetAppSettingsRequest {}),
    )
    .await;
    // Then
    let wire::command_result::Command::GetAppSettings(settings) = settings else {
        panic!("settings")
    };
    assert_eq!(settings.close_to_tray, Some(true));
    quit(&mut daemon, &mut socket, false).await;
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
    quit(&mut daemon, &mut socket, true).await;
}

async fn expect_push(socket: &mut Socket, matches: impl Fn(&wire::push::Event) -> bool) {
    tokio::time::timeout(Duration::from_secs(10), async {
        while let Some(push) = socket.push.next().await {
            if push.event.as_ref().is_some_and(&matches) {
                return;
            }
        }
        panic!("push stream closed");
    })
    .await
    .expect("production notifier must reach the Connect subscription");
}
async fn request_with_push(
    socket: &mut Socket,
    id: &str,
    command: wire::command_request::Command,
    matches: impl Fn(&wire::push::Event) -> bool,
) -> wire::command_result::Command {
    let result = request(socket, id, command).await;
    expect_push(socket, matches).await;
    result
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn test_daemon本番配線_状態を購読へ配信し残る通知をpushへ届ける() {
    use std::os::unix::fs::PermissionsExt;
    use wire::command_request::Command as C;
    use wire::push::Event as E;
    // Given
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let repo_path = root.join("repository");
    let repository = git2::Repository::init(&repo_path).unwrap();
    let signature = git2::Signature::now("daemon smoke", "smoke@example.test").unwrap();
    let tree = repository.index().unwrap().write_tree().unwrap();
    let commit = repository
        .commit(
            Some("HEAD"),
            &signature,
            &signature,
            "initial",
            &repository.find_tree(tree).unwrap(),
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
    request(
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
    let mut states = subscribe_workspaces(&socket).await;
    let snapshot = states.next().await.unwrap();
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
            Some(wire::workspace_tree_item_dto::Variant::Node(node)) => node.id.clone(),
            _ => None,
        })
        .expect("standalone session node");
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
    expect_workspace(&mut states, "session rename", |value| worktree_state(value, worktree).snapshot.as_ref().unwrap().nodes.as_ref().unwrap().items.iter().any(|item| matches!(&item.variant, Some(wire::workspace_tree_item_dto::Variant::Node(node)) if node.id.as_deref() == Some(&node_id) && node.title.as_deref() == Some("subscription verification")))).await;
    // When / Then: workflow notifier
    let workflows = if cfg!(target_os = "macos") {
        root.join("Library/Application Support/releash/workflows")
    } else {
        root.join("config/releash/workflows")
    };
    std::fs::write(
        workflows.join("push-smoke.yml"),
        "name: push-smoke\ndescription: push smoke\nnodes:\n  main:\n    command: printf done\n    completion:\n      require: approval\n",
    )
    .unwrap();
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
    expect_workspace(&mut states, "workflow start", |value| worktree_state(value, worktree).snapshot.as_ref().unwrap().nodes.as_ref().unwrap().items.iter().any(|item| matches!(&item.variant, Some(wire::workspace_tree_item_dto::Variant::Node(node)) if node.id.as_deref() == Some(&execution_id) && node.title.as_deref() == Some("push-smoke")))).await;

    // When / Then: comment command notifier (the watcher wildcard cannot satisfy this assertion)
    request_with_push(&mut socket, "comment", C::CreateReviewThread(wire::CreateReviewThreadRequest {
        worktree_name: Some("repository".into()), content: Some("production comment".into()), ..Default::default()
    }), |event| matches!(event, E::ReviewCommentsChanged(value) if value.value.as_deref() == Some("repository"))).await;
    // When / Then: review watcher, without a comment command
    std::fs::write(root.join("review-comments/external.events.json"), "[]").unwrap();
    expect_push(&mut socket, |event| matches!(event, E::ReviewCommentsChanged(value) if value.value.as_deref() == Some("*"))).await;
    // When / Then: fallback file watcher, outside a git repository
    let files = root.join("files");
    std::fs::create_dir(&files).unwrap();
    let watch = socket
        .client
        .watch_files(rpc::WatchFilesRequest {
            subscription_id: socket.subscription_id.clone(),
            request: rpc::StartWatchingRequest {
                path: Some(files.to_str().unwrap().into()),
                ..Default::default()
            }
            .into(),
            ..Default::default()
        })
        .await
        .unwrap()
        .into_owned();
    let watched_file = files.join("changed.txt");
    std::fs::write(&watched_file, "changed").unwrap();
    expect_push(&mut socket, |event| matches!(event, E::FileChange(value) if value.watcher_id == watch.value && value.path.as_deref() == watched_file.to_str())).await;
    // When / Then: the workspace subscription owns the repository watch.
    repository
        .branch(
            "pushed-branch",
            &repository.find_commit(commit).unwrap(),
            false,
        )
        .unwrap();
    expect_push(&mut socket, |event| matches!(event, E::GitStatusChanged(value) if value.repo_path.as_deref() == Some(worktree))).await;
    expect_workspace(&mut states, "branch creation", |value| {
        value
            .repositories
            .as_ref()
            .unwrap()
            .items
            .iter()
            .any(|repo| {
                repo.branches.as_ref().unwrap().items.iter().any(|branch| {
                    branch.branch.as_ref().unwrap().name.as_deref() == Some("pushed-branch")
                })
            })
    })
    .await;
    // When / Then: only Refresh rescans the upstream configuration, which Git watches ignore.
    let head = repository.head().unwrap().name().unwrap().to_owned();
    for has_upstream in [true, false] {
        let mut config = repository.config().unwrap();
        if has_upstream {
            config.set_str("branch.pushed-branch.remote", ".").unwrap();
            config.set_str("branch.pushed-branch.merge", &head).unwrap();
        } else {
            config.remove("branch.pushed-branch.remote").unwrap();
            config.remove("branch.pushed-branch.merge").unwrap();
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
            value
                .repositories
                .as_ref()
                .unwrap()
                .items
                .iter()
                .any(|repo| {
                    repo.branches.as_ref().unwrap().items.iter().any(|branch| {
                        branch.branch.as_ref().is_some_and(|branch| {
                            branch.name.as_deref() == Some("pushed-branch")
                                && branch.has_upstream == Some(has_upstream)
                        })
                    })
                })
        })
        .await;
    }
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
    let linked = created.path.unwrap();
    expect_workspace(&mut states, "first linked worktree creation", |value| {
        value
            .repositories
            .as_ref()
            .unwrap()
            .items
            .iter()
            .any(|repo| {
                repo.branches.as_ref().unwrap().items.iter().any(|branch| {
                    branch.branch.as_ref().unwrap().worktree_path.as_deref()
                        == Some(linked.as_str())
                })
            })
    })
    .await;
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
        value
            .repositories
            .as_ref()
            .unwrap()
            .items
            .iter()
            .all(|repo| {
                repo.branches.as_ref().unwrap().items.iter().all(|branch| {
                    branch.branch.as_ref().unwrap().worktree_path.as_deref()
                        != Some(linked.as_str())
                })
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
        value
            .repositories
            .as_ref()
            .unwrap()
            .items
            .iter()
            .any(|repo| {
                repo.branches.as_ref().unwrap().items.iter().any(|branch| {
                    branch.branch.as_ref().unwrap().worktree_path.as_deref()
                        == Some(external.as_str())
                })
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
    quit(&mut daemon, &mut socket, false).await;
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
    let snapshot = restarted_states.next().await.unwrap();
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
    quit(&mut restarted, &mut socket, false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_親ui終了_daemonとterminal子孫が一括停止なしで終了する() {
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
    loop {
        if let Some(status) = daemon.0.try_wait().unwrap() {
            assert!(!status.success());
            break;
        }
        assert!(Instant::now() < deadline, "daemon survived parent EOF");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    for pid in pids {
        while unsafe { libc::kill(pid, 0) } == 0 {
            assert!(
                Instant::now() < deadline,
                "daemon descendant survived: {pid}"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
    let mut output = String::new();
    std::io::Read::read_to_string(&mut daemon.0.stdout.take().unwrap(), &mut output).unwrap();
    assert!(!output.contains("releash-shutdown-complete"));
    let (mut restarted, discovery) = start(directory.path());
    let (mut socket, _) = connect(&discovery).await;
    quit(&mut restarted, &mut socket, false).await;
}
