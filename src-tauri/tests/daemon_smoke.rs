#![cfg(not(feature = "desktop"))]

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use prost::Message;
use serde_json::Value;
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message as Frame};

pub mod wire {
    include!(concat!(env!("OUT_DIR"), "/releash.client.v1.rs"));
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;
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

async fn receive(socket: &mut Socket) -> wire::envelope::Body {
    loop {
        let frame = tokio::time::timeout(Duration::from_secs(10), socket.next())
            .await
            .unwrap()
            .expect("socket closed")
            .unwrap();
        if let Frame::Binary(bytes) = frame {
            let body = wire::Envelope::decode(bytes).unwrap().body.unwrap();
            if !matches!(body, wire::envelope::Body::Push(_)) {
                return body;
            }
        }
    }
}

async fn send(socket: &mut Socket, body: wire::envelope::Body) {
    socket
        .send(Frame::Binary(
            wire::Envelope { body: Some(body) }.encode_to_vec().into(),
        ))
        .await
        .unwrap();
}

async fn connect(discovery: &Value) -> (Socket, String) {
    let mut request = format!("ws://127.0.0.1:{}/v1/client", discovery["port"])
        .into_client_request()
        .unwrap();
    request.headers_mut().insert(
        "sec-websocket-protocol",
        format!("releash-bearer.{}", discovery["token"].as_str().unwrap())
            .parse()
            .unwrap(),
    );
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    send(
        &mut socket,
        wire::envelope::Body::Hello(wire::ClientHello::default()),
    )
    .await;
    let wire::envelope::Body::Hello(hello) = receive(&mut socket).await else {
        panic!("hello")
    };
    assert!(!hello.launch_id.is_empty());
    assert_eq!(hello.release, env!("CARGO_PKG_VERSION"));
    (socket, hello.instance_id)
}

async fn request(
    socket: &mut Socket,
    id: &str,
    command: wire::command_request::Command,
) -> wire::command_result::Command {
    send(
        socket,
        wire::envelope::Body::Request(Box::new(wire::CommandRequest {
            request_id: id.into(),
            command: Some(command),
            ..Default::default()
        })),
    )
    .await;
    let wire::envelope::Body::Response(response) = receive(socket).await else {
        panic!("response")
    };
    assert_eq!(response.request_id, id);
    let Some(wire::command_response::Outcome::Result(result)) = response.outcome else {
        panic!("command failed: {response:?}")
    };
    result.command.unwrap()
}

async fn quit(daemon: &mut Daemon, socket: &mut Socket, restart: bool) {
    send(
        socket,
        wire::envelope::Body::Request(Box::new(wire::CommandRequest {
            request_id: "quit".into(),
            command: Some(wire::command_request::Command::RequestApplicationQuit(
                wire::RequestApplicationQuitRequest {
                    request: Some(wire::ApplicationQuitRequestDtoV1 {
                        request_id: Some(uuid::Uuid::new_v4().to_string()),
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
            )),
            ..Default::default()
        })),
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
    send(
        &mut socket,
        wire::envelope::Body::OperationQuery(wire::OperationQuery {
            request_id: "save".into(),
            instance_id: instance,
            sent: true,
            request: Some(wire::CommandRequest {
                request_id: "save".into(),
                command: Some(C::UpdateExternalEditor(wire::UpdateExternalEditorRequest {
                    editor: Some("must-not-replay".into()),
                })),
                ..Default::default()
            }),
            ..Default::default()
        }),
    )
    .await;
    let wire::envelope::Body::OperationStatus(status) = receive(&mut socket).await else {
        panic!("operation status")
    };
    assert_eq!(status.state, "unknown");
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
async fn test_daemon本番配線_repository変更が同じwsへpushされる() {
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
    send(
        &mut socket,
        wire::envelope::Body::Request(Box::new(wire::CommandRequest {
            request_id: "add-repo".into(),
            command: Some(wire::command_request::Command::AddRepoPath(
                wire::AddRepoPathRequest {
                    path: Some(repository.clone()),
                },
            )),
            ..Default::default()
        })),
    )
    .await;
    // Then
    let mut response_received = false;
    let mut push_received = false;
    tokio::time::timeout(Duration::from_secs(10), async {
        while !response_received || !push_received {
            let Frame::Binary(bytes) = socket.next().await.unwrap().unwrap() else {
                continue;
            };
            match wire::Envelope::decode(bytes).unwrap().body.unwrap() {
                wire::envelope::Body::Response(response) => {
                    assert_eq!(response.request_id, "add-repo");
                    let Some(wire::command_response::Outcome::Result(result)) = response.outcome
                    else {
                        panic!("command failed")
                    };
                    let Some(wire::command_result::Command::AddRepoPath(added)) = result.command
                    else {
                        panic!("add repo result")
                    };
                    assert_eq!(added.value, Some(true));
                    response_received = true;
                }
                wire::envelope::Body::Push(wire::Push {
                    event: Some(wire::push::Event::RepoPathsChanged(paths)),
                }) => {
                    assert_eq!(paths.items, vec![repository.clone()]);
                    push_received = true;
                }
                _ => {}
            }
        }
    })
    .await
    .expect("production composition must connect notifier and websocket to the same sink");
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
        loop {
            let Frame::Binary(bytes) = socket.next().await.unwrap().unwrap() else {
                continue;
            };
            if let Some(wire::envelope::Body::Push(wire::Push { event: Some(event) })) =
                wire::Envelope::decode(bytes).unwrap().body
            {
                if matches(&event) {
                    break;
                }
            }
        }
    })
    .await
    .expect("production notifier must reach the websocket sink");
}

async fn request_with_push(
    socket: &mut Socket,
    id: &str,
    command: wire::command_request::Command,
    matches: impl Fn(&wire::push::Event) -> bool,
) -> wire::command_result::Command {
    send(
        socket,
        wire::envelope::Body::Request(Box::new(wire::CommandRequest {
            request_id: id.into(),
            command: Some(command),
            ..Default::default()
        })),
    )
    .await;
    tokio::time::timeout(Duration::from_secs(15), async {
        let mut result = None;
        let mut pushed = false;
        loop {
            let Frame::Binary(bytes) = socket.next().await.unwrap().unwrap() else {
                continue;
            };
            match wire::Envelope::decode(bytes).unwrap().body.unwrap() {
                wire::envelope::Body::Response(response) => {
                    assert_eq!(response.request_id, id);
                    let Some(wire::command_response::Outcome::Result(value)) = response.outcome
                    else {
                        panic!("{response:?}")
                    };
                    result = value.command;
                }
                wire::envelope::Body::Push(wire::Push { event: Some(event) }) => {
                    pushed |= matches(&event)
                }
                _ => {}
            }
            if pushed && result.is_some() {
                return result.unwrap();
            }
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!("production command and its notifier must share the websocket sink: {id}")
    })
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
        worktree_path: Some(worktree.into()), node_id: Some(node_id), name: Some("push verification".into()),
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
    let wire::command_result::Command::StartWatching(watch) = request(
        &mut socket,
        "files",
        C::StartWatching(wire::StartWatchingRequest {
            path: Some(files.to_str().unwrap().into()),
        }),
    )
    .await
    else {
        panic!("file watch")
    };
    let watched_file = files.join("changed.txt");
    std::fs::write(&watched_file, "changed").unwrap();
    expect_push(&mut socket, |event| matches!(event, E::FileChange(value) if value.watcher_id == watch.value && value.path.as_deref() == watched_file.to_str())).await;
    // When / Then: repository state notifier, including all of its state push types
    request(
        &mut socket,
        "git-watch",
        C::StartGitDirWatching(wire::StartGitDirWatchingRequest {
            repo_path: Some(worktree.into()),
        }),
    )
    .await;
    repository
        .branch(
            "pushed-branch",
            &repository.find_commit(commit).unwrap(),
            false,
        )
        .unwrap();
    let mut seen = [false; 3];
    tokio::time::timeout(Duration::from_secs(10), async {
        while !seen.iter().all(|seen| *seen) {
            let Frame::Binary(bytes) = socket.next().await.unwrap().unwrap() else {
                continue;
            };
            if let Some(wire::envelope::Body::Push(wire::Push { event: Some(event) })) =
                wire::Envelope::decode(bytes).unwrap().body
            {
                match event {
                    E::RepositorySnapshotChanged(value)
                        if value.worktree_path.as_deref() == Some(worktree) =>
                    {
                        seen[0] = true
                    }
                    E::GitStatusChanged(value) if value.repo_path.as_deref() == Some(worktree) => {
                        seen[1] = true
                    }
                    E::BranchListSync(_) => seen[2] = true,
                    _ => {}
                }
            }
        }
    })
    .await
    .expect("repository state notifier must send snapshot, git status, and branch pushes");
    quit(&mut daemon, &mut socket, false).await;
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
