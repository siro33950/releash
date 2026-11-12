#![cfg(unix)]

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use serde_json::Value;

use releashd::test_support::integration::client_protocol::{rpc, wire};
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
struct Daemon(Child);
impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn start(directory: &Path) -> (Daemon, Value) {
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
        .stdin(Stdio::null())
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

async fn connect(discovery: &Value) -> Socket {
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
    client.get_server_info(rpc::Unit::default()).await.unwrap();
    Socket { client }
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

use std::os::unix::fs::PermissionsExt;
use wire::command_request::Command as C;

struct Host {
    daemon: Daemon,
    socket: Socket,
    discovery: Value,
    root: tempfile::TempDir,
    worktree: String,
}

impl Host {
    async fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().canonicalize().unwrap();
        let repository = git2::Repository::init(path.join("repository")).unwrap();
        let signature = git2::Signature::now("acceptance", "test@example.test").unwrap();
        let tree = repository.index().unwrap().write_tree().unwrap();
        repository
            .commit(
                Some("HEAD"),
                &signature,
                &signature,
                "initial",
                &repository.find_tree(tree).unwrap(),
                &[],
            )
            .unwrap();
        let worktree = path.join("repository").to_str().unwrap().to_owned();
        let cli = Path::new(env!("CARGO_BIN_EXE_releashd")).with_file_name("releash");
        let fixture = path.join("provider");
        let quote = |path: &Path| format!("'{}'", path.to_str().unwrap().replace('\'', "'\"'\"'"));
        std::fs::write(&fixture, format!(r#"#!/bin/sh
if [ "$1" = --version ]; then echo 'claude 1.0.0'; exit; fi
control={root}/"$RELEASH_SESSION_ID"
hook() {{
  printf '{{"session_id":"provider-%s","transcript_path":"provider://fixture/%s","hook_event_name":"%s"}}' "$RELEASH_SESSION_ID" "$RELEASH_SESSION_ID" "$1" | {cli} hook receive --provider claude
}}
if [ ! -f {root}/without-conversation ]; then hook SessionStart || exit 90; fi
echo $$ > "$control.ready"
while :; do
  if [ -f "$control.command" ]; then
    command=$(cat "$control.command")
    rm "$control.command"
    case "$command" in
      stop) hook Stop ;;
      exit) exit 0 ;;
      crash) exit 23 ;;
    esac
  fi
  sleep 0.02
done
"#, root = quote(&path), cli = quote(&cli))).unwrap();
        std::fs::set_permissions(&fixture, std::fs::Permissions::from_mode(0o755)).unwrap();
        let (daemon, discovery) = start(&path);
        let socket = connect(&discovery).await;
        let host = Self {
            daemon,
            socket,
            discovery,
            root,
            worktree,
        };
        host.command(C::AddRepoPath(wire::AddRepoPathRequest {
            path: Some(host.worktree.clone()),
        }))
        .await;
        host.command(C::UpdateProviderExecutable(
            wire::UpdateProviderExecutableRequest {
                provider: Some("claude".into()),
                executable: Some(fixture.to_str().unwrap().into()),
            },
        ))
        .await;
        host
    }

    async fn command(&self, command: C) -> wire::command_result::Command {
        call(&self.socket.client, command).await.unwrap()
    }

    async fn standalone(&self) -> String {
        let result = self
            .command(C::CreateAgentSession(wire::CreateAgentSessionRequest {
                workspace_identity: Some(self.worktree.clone()),
                worktree_path: Some(self.worktree.clone()),
                provider: Some("claude".into()),
                rows: Some(24),
                cols: Some(80),
                caller_request_id: Some(uuid::Uuid::new_v4().to_string()),
            }))
            .await;
        let wire::command_result::Command::CreateAgentSession(selected) = result else {
            panic!("session selection")
        };
        let id = selected.agent_session_id.unwrap();
        self.ready(&id, None).await;
        id
    }

    async fn ready(&self, session: &str, previous: Option<&str>) -> String {
        tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                if let Ok(pid) =
                    std::fs::read_to_string(self.root.path().join(format!("{session}.ready")))
                {
                    if previous != Some(pid.as_str()) {
                        return pid;
                    }
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("provider process ready")
    }

    fn provider(&self, session: &str, command: &str) {
        std::fs::write(self.root.path().join(format!("{session}.command")), command).unwrap();
    }

    async fn layout(&self) -> wire::WorkspaceStateDto {
        let mut stream = subscribe_state(
            &self.socket,
            "workspace-state",
            vec![self.worktree.clone(), self.worktree.clone()],
        )
        .await;
        let value = expect_state(
            &mut stream,
            "workspace layout",
            |v| matches!(v, wire::state_payload::Value::WorkspaceState(s) if s.value.is_some()),
        )
        .await;
        let wire::state_payload::Value::WorkspaceState(state) = value else {
            unreachable!()
        };
        state.value.unwrap()
    }

    async fn tabs(&self, session: &str, count: usize) {
        let mut stream = subscribe_state(
            &self.socket,
            "workspace-state",
            vec![self.worktree.clone(), self.worktree.clone()],
        )
        .await;
        expect_state(&mut stream, &format!("{session}: {count} tabs"), |value| {
            matches!(value, wire::state_payload::Value::WorkspaceState(state) if state.value.as_ref().is_some_and(|state| tab_count(state.pane_layout.as_ref(), session) == count))
        }).await;
    }

    async fn assert_single_tab_remains(&self, session: &str) {
        let mut states = subscribe_state(
            &self.socket,
            "workspace-state",
            vec![self.worktree.clone(), self.worktree.clone()],
        )
        .await;
        let observation = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let value = states
                    .next()
                    .await
                    .expect("workspace subscription remains open");
                let wire::state_payload::Value::WorkspaceState(state) = value else {
                    panic!("workspace state")
                };
                assert_eq!(
                    tab_count(
                        state.value.as_ref().and_then(|s| s.pane_layout.as_ref()),
                        session
                    ),
                    1
                );
            }
        })
        .await;
        assert!(
            observation.is_err(),
            "observe the reopened tab for the full interval"
        );
    }

    async fn availability(&self, session: &str, expected: Option<i32>) {
        let mut stream = subscribe_state(&self.socket, "session-tab", vec![session.into()]).await;
        expect_state(&mut stream, "session availability", |value| {
            matches!(value, wire::state_payload::Value::SessionTab(tab) if tab.session.as_ref().and_then(|s| s.availability) == expected)
        }).await;
    }

    async fn reopen(&self, session: &str, mut previous: wire::WorkspaceStateDto) {
        previous.etag = self.layout().await.etag;
        self.command(C::SaveWorkspaceState(wire::SaveWorkspaceStateRequest {
            worktree_name: Some(self.worktree.clone()),
            state: Some(previous),
        }))
        .await;
        self.command(C::OpenAgentSession(wire::OpenAgentSessionRequest {
            agent_session_id: Some(session.into()),
            rows: Some(24),
            cols: Some(80),
            caller_request_id: Some(uuid::Uuid::new_v4().to_string()),
        }))
        .await;
    }

    async fn resume(&self, node: &str) {
        self.command(C::ResumeWorkspaceSessionNode(
            wire::ResumeWorkspaceSessionNodeRequest {
                worktree_path: Some(self.worktree.clone()),
                node_id: Some(node.into()),
            },
        ))
        .await;
    }

    async fn archive(&self, execution: &str) {
        self.command(C::ArchiveWorkspaceWorkflowExecution(
            wire::ArchiveWorkspaceWorkflowExecutionRequest {
                worktree_path: Some(self.worktree.clone()),
                execution_id: Some(execution.into()),
            },
        ))
        .await;
    }

    async fn workflow(&self, nodes: &str) -> String {
        let workflows = if cfg!(target_os = "macos") {
            self.root
                .path()
                .join("Library/Application Support/releash/workflows")
        } else {
            self.root.path().join("config/releash/workflows")
        };
        std::fs::create_dir_all(workflows.join("instructions")).unwrap();
        std::fs::write(
            workflows.join("instructions/tabs.md"),
            "Exercise session tabs.",
        )
        .unwrap();
        std::fs::write(workflows.join("tabs.yml"), format!("name: tabs\ndescription: Session tab acceptance\nschemas:\n  result: {{type: object, properties: {{passed: {{type: boolean}}}}, required: [passed]}}\nnodes:\n{nodes}")).unwrap();
        let result = self
            .command(C::StartWorkflow(wire::StartWorkflowRequest {
                workflow_name: Some("tabs".into()),
                worktree_path: Some(self.worktree.clone()),
                ..Default::default()
            }))
            .await;
        let wire::command_result::Command::StartWorkflow(id) = result else {
            panic!("workflow")
        };
        id.value.unwrap()
    }

    async fn node(&self, execution: &str, name: &str) -> (String, String) {
        let mut stream =
            subscribe_state(&self.socket, "workflow-execution", vec![execution.into()]).await;
        let find = |value: &wire::state_payload::Value| {
            let wire::state_payload::Value::WorkflowExecution(state) = value else {
                return None;
            };
            state
                .value
                .as_ref()?
                .node_executions
                .as_ref()?
                .items
                .iter()
                .find(|n| n.node_name.as_deref() == Some(name) && n.session_id.is_some())
                .map(|n| (n.id.clone().unwrap(), n.session_id.clone().unwrap()))
        };
        let state = expect_state(&mut stream, "workflow session", |value| {
            find(value).is_some()
        })
        .await;
        let result = find(&state).unwrap();
        self.ready(&result.1, None).await;
        result
    }

    async fn submit(&self, node: &str, passed: bool) {
        self.command(C::WorkflowSubmitOutput(wire::WorkflowSubmitOutputRequest {
            node_execution_id: Some(node.into()),
            artifact: Some(wire::WorkflowSubmitArtifactInput {
                contract: Some("result".into()),
                value: Some(wire::WorkflowValue {
                    variant: Some(wire::workflow_value::Variant::ObjectValue(
                        wire::WorkflowValueObject {
                            entries: [(
                                "passed".into(),
                                wire::WorkflowValue {
                                    variant: Some(wire::workflow_value::Variant::BooleanValue(
                                        wire::ResultBool {
                                            value: Some(passed),
                                        },
                                    )),
                                },
                            )]
                            .into_iter()
                            .collect(),
                        },
                    )),
                }),
            }),
        }))
        .await;
    }

    async fn finish(mut self) {
        quit(&mut self.daemon, &mut self.socket).await;
    }
}

fn tab_count(layout: Option<&wire::PaneLayout>, session: &str) -> usize {
    match layout.and_then(|layout| layout.node.as_ref()) {
        Some(wire::pane_layout::Node::Pane(pane)) => pane
            .tabs
            .iter()
            .filter(|tab| tab.agent_session_id.as_deref() == Some(session))
            .count(),
        Some(wire::pane_layout::Node::Split(split)) => {
            tab_count(split.first.as_deref(), session) + tab_count(split.second.as_deref(), session)
        }
        None => 0,
    }
}

// B-003, B-004, B-011
#[tokio::test]
async fn test_会話タブ_provider終了と消失で閉じ再接続でも開かない() {
    for exit in ["exit", "crash", "without-conversation"] {
        // Given
        let mut host = Host::new().await;
        if exit == "without-conversation" {
            std::fs::write(host.root.path().join(exit), "").unwrap();
        }
        let session = host.standalone().await;
        host.tabs(&session, 1).await;
        // When
        host.provider(
            &session,
            if exit == "without-conversation" {
                "exit"
            } else {
                exit
            },
        );
        host.availability(
            &session,
            if exit == "without-conversation" {
                None
            } else {
                Some(wire::AgentSessionAvailability::Paused as i32)
            },
        )
        .await;
        // Then
        host.tabs(&session, 0).await;
        if exit == "without-conversation" {
            let mut workspaces = subscribe_workspaces(&host.socket).await;
            expect_workspace(&mut workspaces, "session removed from Node pane", |snapshot| {
                find_worktree(snapshot, &host.worktree)
                    .and_then(|tree| tree.snapshot.as_ref())
                    .and_then(|tree| tree.nodes.as_ref())
                    .is_some_and(|nodes| !nodes.items.iter().any(|item| {
                        matches!(&item.variant, Some(wire::workspace_tree_item::Variant::Node(node)) if node.session_id.as_deref() == Some(&session))
                    }))
            }).await;
        }
        // When: Swift.app の再接続と同じく、新しい client で配置を購読する。
        host.socket = connect(&host.discovery).await;
        // Then
        host.tabs(&session, 0).await;
        host.finish().await;
    }
}

// B-007, B-008, B-010
#[tokio::test]
async fn test_会話タブ_停止中の明示表示を保ちresumeで一つ開く() {
    for reopen in [false, true] {
        // Given
        let host = Host::new().await;
        let session = host.standalone().await;
        host.tabs(&session, 1).await;
        let layout = host.layout().await;
        let pid = host.ready(&session, None).await;
        host.provider(&session, "exit");
        host.availability(
            &session,
            Some(wire::AgentSessionAvailability::Paused as i32),
        )
        .await;
        host.tabs(&session, 0).await;
        if reopen {
            // When
            host.reopen(&session, layout).await;
            // Then
            host.tabs(&session, 1).await;
            host.availability(
                &session,
                Some(wire::AgentSessionAvailability::Paused as i32),
            )
            .await;
            host.assert_single_tab_remains(&session).await;
        }
        // When
        host.resume(&session).await;
        host.ready(&session, Some(&pid)).await;
        // Then
        host.availability(&session, Some(wire::AgentSessionAvailability::Open as i32))
            .await;
        host.tabs(&session, 1).await;
        if reopen {
            host.assert_single_tab_remains(&session).await;
        }
        host.finish().await;
    }
}

// B-001, B-002, B-005, B-006, B-011
#[tokio::test]
async fn test_会話タブ_workflow完了とabortとarchiveで閉じる() {
    for action in ["complete", "abort", "archive", "reopen-archive"] {
        // Given
        let mut host = Host::new().await;
        let archive = action.ends_with("archive");
        let nodes = if archive {
            "  main: {fanout: {children: [first, second]}}\n  first:\n    session: {provider: claude, facets: {instruction: tabs}}\n    artifact: result\n  second:\n    session: {provider: claude, facets: {instruction: tabs}}\n    artifact: result\n"
        } else {
            "  main:\n    session: {provider: claude, facets: {instruction: tabs}}\n    artifact: result\n"
        };
        let execution = host.workflow(nodes).await;
        let (node, session) = host
            .node(&execution, if archive { "first" } else { "main" })
            .await;
        let second = if archive {
            let (_, id) = host.node(&execution, "second").await;
            host.tabs(&id, 1).await;
            Some(id)
        } else {
            None
        };
        host.tabs(&session, 1).await;
        let layout = host.layout().await;
        if action == "reopen-archive" {
            host.provider(&session, "exit");
            host.availability(
                &session,
                Some(wire::AgentSessionAvailability::Paused as i32),
            )
            .await;
            host.tabs(&session, 0).await;
            host.reopen(&session, layout).await;
            host.tabs(&session, 1).await;
        }
        // When
        match action {
            "complete" => {
                host.submit(&node, true).await;
                host.provider(&session, "stop");
            }
            "abort" => {
                host.command(C::AbortWorkflow(wire::AbortWorkflowRequest {
                    execution_id: Some(execution.clone()),
                }))
                .await;
            }
            _ => host.archive(&execution).await,
        }
        // Then
        if !archive {
            let mut execution_state =
                subscribe_state(&host.socket, "workflow-execution", vec![execution.clone()]).await;
            let expected = if action == "complete" {
                wire::execution_status_view::Value::Completed
            } else {
                wire::execution_status_view::Value::Aborted
            } as i32;
            expect_state(&mut execution_state, "execution ended", |state| {
            matches!(state, wire::state_payload::Value::WorkflowExecution(state) if state.value.as_ref().and_then(|view| view.status.as_ref()).and_then(|status| status.value) == Some(expected))
        }).await;
            host.availability(
                &session,
                Some(wire::AgentSessionAvailability::Paused as i32),
            )
            .await;
        }
        host.tabs(&session, 0).await;
        if let Some(second) = &second {
            host.tabs(second, 0).await;
        }
        // When
        host.socket = connect(&host.discovery).await;
        // Then
        host.tabs(&session, 0).await;
        if let Some(second) = &second {
            host.tabs(second, 0).await;
        }
        host.finish().await;
    }
}

// B-009
#[tokio::test]
async fn test_会話タブ_delegateの結果で再起動した親sessionを開く() {
    // Given
    let host = Host::new().await;
    let execution = host.workflow("  main:\n    session: {provider: claude, facets: {instruction: tabs}}\n    artifact: result\n    completion:\n      delegate: {child: check, when: child.passed, max_iterations: 2}\n  check:\n    session: {provider: claude, facets: {instruction: tabs}}\n    artifact: result\n").await;
    let (parent, session) = host.node(&execution, "main").await;
    host.tabs(&session, 1).await;
    let pid = host.ready(&session, None).await;
    host.submit(&parent, false).await;
    host.provider(&session, "stop");
    let (child, child_session) = host.node(&execution, "check").await;
    host.provider(&session, "exit");
    host.availability(
        &session,
        Some(wire::AgentSessionAvailability::Paused as i32),
    )
    .await;
    host.tabs(&session, 0).await;
    // When
    host.submit(&child, false).await;
    host.provider(&child_session, "stop");
    host.ready(&session, Some(&pid)).await;
    // Then
    host.availability(&session, Some(wire::AgentSessionAvailability::Open as i32))
        .await;
    host.tabs(&session, 1).await;
    host.finish().await;
}
