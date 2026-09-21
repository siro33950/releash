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
async fn test_daemon本番配線_repository変更がconnectへpushされる() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let (mut daemon, discovery) = start(directory.path());
    let (mut socket, _) = connect(&discovery).await;
    let repository = directory
        .path()
        .join("repository")
        .to_string_lossy()
        .into_owned();
    // When
    let result = request_with_push(&mut socket,"add-repo",wire::command_request::Command::AddRepoPath(wire::AddRepoPathRequest {path:Some(repository.clone())}), |event| matches!(event,wire::push::Event::RepoPathsChanged(paths) if paths.items == [repository.clone()])).await;
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
async fn test_daemon本番配線_各通知元からwsへpushを届ける() {
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
    let wire::command_result::Command::ListWorkspaceWorktreeNodes(tree) = request(
        &mut socket,
        "session-tree",
        C::ListWorkspaceWorktreeNodes(wire::ListWorkspaceWorktreeNodesRequest {
            worktree_path: Some(worktree.into()),
        }),
    )
    .await
    else {
        panic!("workspace tree")
    };
    let node_id = tree
        .nodes
        .unwrap()
        .items
        .into_iter()
        .find_map(|item| match item.variant {
            Some(wire::workspace_tree_item_dto::Variant::Node(node)) => node.id,
            _ => None,
        })
        .expect("standalone session node");
    request_with_push(&mut socket, "rename-session", C::RenameWorkspaceSessionNode(wire::RenameWorkspaceSessionNodeRequest {
        worktree_path: Some(worktree.into()), node_id: Some(node_id.clone()), name: Some("push verification".into()),
    }), |event| matches!(event, E::AgentSessionChanged(value) if value.worktree_path.as_deref() == Some(worktree))).await;
    // When / Then: workflow notifier
    let workflows = if cfg!(target_os = "macos") {
        root.join("Library/Application Support/releash/workflows")
    } else {
        root.join("config/releash/workflows")
    };
    std::fs::write(
        workflows.join("push-smoke.yml"),
        "name: push-smoke\ndescription: push smoke\nnodes:\n  main:\n    command: printf done\n",
    )
    .unwrap();
    request_with_push(&mut socket, "workflow", C::StartWorkflow(wire::StartWorkflowRequest {
        workflow_name: Some("push-smoke".into()), worktree_path: Some(worktree.into()), ..Default::default()
    }), |event| matches!(event, E::WorkflowExecutionChanged(value) if value.worktree_path.as_deref() == Some(worktree))).await;
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
    // When / Then: repository state notifier, including all of its state push types
    socket
        .client
        .watch_git_directory(rpc::WatchGitDirectoryRequest {
            subscription_id: socket.subscription_id.clone(),
            request: rpc::StartGitDirWatchingRequest {
                repo_path: Some(worktree.into()),
                ..Default::default()
            }
            .into(),
            ..Default::default()
        })
        .await
        .unwrap();
    repository
        .branch(
            "pushed-branch",
            &repository.find_commit(commit).unwrap(),
            false,
        )
        .unwrap();
    let mut seen = [false; 2];
    tokio::time::timeout(Duration::from_secs(10), async {
        while !seen.iter().all(|seen| *seen) {
            if let Some(event) = socket.push.next().await.unwrap().event {
                match event {
                    E::GitStatusChanged(value) if value.repo_path.as_deref() == Some(worktree) => {
                        seen[0] = true
                    }
                    E::BranchListSync(_) => seen[1] = true,
                    _ => {}
                }
            }
        }
    })
    .await
    .expect("repository state notifier must send git status and branch pushes");
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
