use super::*;
use serde_json::json;
use std::time::Duration;
use tokio_tungstenite::{tungstenite::Message as WsMessage, MaybeTlsStream, WebSocketStream};

type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

async fn connect(
    deps: ClientApiDeps,
) -> (Socket, std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, router(Some(deps))).await.unwrap();
    });
    let (socket, _) = tokio_tungstenite::connect_async(format!("ws://{address}/v1/client"))
        .await
        .unwrap();
    (socket, address, server)
}

fn request(id: &str, command: &str, args: serde_json::Value) -> WsMessage {
    WsMessage::Binary(crate::client_api_acceptance::encode_client_request(id, command, args).into())
}

async fn receive(socket: &mut Socket) -> Body {
    let WsMessage::Binary(bytes) = tokio::time::timeout(Duration::from_secs(5), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap()
    else {
        panic!("binary frame");
    };
    Envelope::decode(bytes).unwrap().body.unwrap()
}

fn dispatch() -> ClientCommandDispatch {
    ClientCommandDispatch::new(
        Arc::new(crate::adaptor::controller::wiring::build_repository_usecase()),
        Arc::new(crate::usecase::application_startup::ApplicationStartupAuthority::ready()),
    )
}

#[derive(Default)]
struct Files(std::sync::Mutex<bool>);
impl crate::domain::repository::file_watcher::FileWatchGateway for Files {
    fn start(&self, _: &str) -> Result<u64, String> {
        *self.0.lock().unwrap() = true;
        Ok(42)
    }
    fn stop(&self, _: u64) -> Result<(), String> {
        *self.0.lock().unwrap() = false;
        Ok(())
    }
}

#[tokio::test]
async fn test_監視開始_応答未受領の切断だけがfileとgit監視を回収する() {
    use crate::usecase::watcher::WatcherUsecase;
    for git in [false, true] {
        for phase in ["running", "unreceived", "received"] {
            // Given
            let directory = tempfile::tempdir().unwrap();
            let repository =
                Arc::new(crate::usecase::repository_state::service::tests::watching_service());
            let files = Arc::new(Files::default());
            let watcher = Arc::new(WatcherUsecase::new(
                git.then(|| repository.clone()),
                files.clone(),
            ));
            let gate = Arc::new(tokio::sync::Semaphore::new(0));
            let started = Arc::new(tokio::sync::Notify::new());
            let stopped = Arc::new(tokio::sync::Notify::new());
            let watch_id = Arc::new(std::sync::Mutex::new(None));
            let mut dispatch = dispatch();
            let name = if git {
                "start_git_dir_watching"
            } else {
                "start_watching"
            };
            let path = directory.path().to_str().unwrap().to_owned();
            {
                let (watcher, gate, started, watch_id) = (
                    watcher.clone(),
                    gate.clone(),
                    started.clone(),
                    watch_id.clone(),
                );
                dispatch.register_domain(
                    if git {
                        &["start_git_dir_watching"]
                    } else {
                        &["start_watching"]
                    },
                    Box::new(move |_| {
                        let (watcher, gate, started, watch_id, path) = (
                            watcher.clone(),
                            gate.clone(),
                            started.clone(),
                            watch_id.clone(),
                            path.clone(),
                        );
                        Box::pin(async move {
                            started.notify_one();
                            let _gate = gate.acquire().await.unwrap();
                            let id = if git {
                                watcher.start_git_dir(&path)
                            } else {
                                watcher.start(&path)
                            }
                            .unwrap();
                            *watch_id.lock().unwrap() = Some(id);
                            let value = wire::ResultUint64 { value: Some(id) };
                            Ok(if git {
                                wire::command_result::Command::StartGitDirWatching(value)
                            } else {
                                wire::command_result::Command::StartWatching(value)
                            })
                        })
                    }),
                );
            }
            {
                let (watcher, stopped) = (watcher.clone(), stopped.clone());
                dispatch.register_domain(
                    &["stop_watching"],
                    Box::new(move |command| {
                        let (watcher, stopped) = (watcher.clone(), stopped.clone());
                        Box::pin(async move {
                            let wire::command_request::Command::StopWatching(args) = command else {
                                panic!("stop");
                            };
                            watcher.stop(args.watcher_id.unwrap()).unwrap();
                            stopped.notify_one();
                            Ok(wire::command_result::Command::StopWatching(wire::Unit {}))
                        })
                    }),
                );
            }
            let deps = ClientApiDeps::new(
                Arc::new(dispatch),
                ClientPushGateway::new(Arc::new(crate::infrastructure::push::PushSink::new())),
            );
            let slots = deps.connection_limit.clone();
            let (mut socket, _, server) = connect(deps).await;
            socket
                .send(request(
                    "watch",
                    name,
                    if git {
                        json!({"repoPath":directory.path()})
                    } else {
                        json!({"path":directory.path()})
                    },
                ))
                .await
                .unwrap();
            started.notified().await;
            // When
            if phase != "running" {
                gate.add_permits(1);
                assert!(matches!(receive(&mut socket).await, Body::Response(_)));
                if phase == "received" {
                    socket
                        .send(WsMessage::Binary(
                            Envelope {
                                body: Some(Body::RequestAck(wire::RequestAck {
                                    request_id: "watch".into(),
                                })),
                            }
                            .encode_to_vec()
                            .into(),
                        ))
                        .await
                        .unwrap();
                    socket
                        .send(request(
                            "barrier",
                            "get_current_branch",
                            json!({"repoPath":"/missing"}),
                        ))
                        .await
                        .unwrap();
                    assert!(matches!(receive(&mut socket).await, Body::Response(_)));
                }
            }
            socket.close(None).await.unwrap();
            tokio::time::timeout(Duration::from_secs(5), async {
                while slots.available_permits() != 16 {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            if phase == "running" {
                gate.add_permits(1);
            }
            // Then
            if phase != "received" {
                tokio::time::timeout(Duration::from_secs(5), stopped.notified())
                    .await
                    .unwrap();
            }
            let id = watch_id.lock().unwrap().unwrap();
            if git {
                assert_eq!(
                    repository.stop_watching(id).unwrap(),
                    phase == "received",
                    "{phase}"
                );
            } else {
                assert_eq!(*files.0.lock().unwrap(), phase == "received", "{phase}");
            }
            server.abort();
        }
    }
}

#[tokio::test]
async fn test_workspace保存_先行保存が遅れても受信順に永続化し再起動後に復元する() {
    assert_workspace_save_order("same").await;
}

#[tokio::test]
async fn test_workspace保存_再接続しても受理済み保存を継続し最後の保存を復元する() {
    assert_workspace_save_order("reconnect").await;
}

#[tokio::test]
async fn test_workspace保存_複数接続でも受信順に永続化し最後の保存を復元する() {
    assert_workspace_save_order("concurrent").await;
}

async fn assert_workspace_save_order(connection: &str) {
    use crate::adaptor::gateway::workspace_state::WorkspaceStateStore;
    use crate::domain::workspace_state::WorkspaceStateRepository;
    // Given
    let data = tempfile::tempdir().unwrap();
    let store = Arc::new(WorkspaceStateStore::new(data.path().to_owned()));
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let started = Arc::new(tokio::sync::Notify::new());
    let calls = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut dispatch = dispatch();
    {
        let (store, gate, started, calls) =
            (store.clone(), gate.clone(), started.clone(), calls.clone());
        dispatch.register_domain(
            &["save_workspace_state"],
            Box::new(move |command| {
                let (store, gate, started, calls) =
                    (store.clone(), gate.clone(), started.clone(), calls.clone());
                Box::pin(async move {
                    let wire::command_request::Command::SaveWorkspaceState(args) = command else {
                        panic!("save");
                    };
                    let state: crate::usecase::workspace_state::dto::WorkspaceStateDto =
                        args.state.unwrap().try_into().unwrap();
                    let tab = state.tabs.active_editor_path.clone();
                    calls.lock().unwrap().push(tab.clone());
                    if tab.as_deref() == Some("first") {
                        started.notify_one();
                        let _gate = gate.acquire().await.unwrap();
                    }
                    crate::usecase::workspace_state::usecase::save_workspace_state(
                        store.as_ref(),
                        &args.worktree_name.unwrap(),
                        state.into(),
                    )
                    .unwrap();
                    Ok(wire::command_result::Command::SaveWorkspaceState(
                        wire::Unit {},
                    ))
                })
            }),
        );
    }
    let deps = ClientApiDeps::new(
        Arc::new(dispatch),
        ClientPushGateway::new(Arc::new(crate::infrastructure::push::PushSink::new())),
    );
    let slots = deps.connection_limit.clone();
    let (mut socket, address, server) = connect(deps).await;
    for name in ["first", "middle", "last"] {
        std::fs::write(data.path().join(name), "").unwrap();
    }
    let state = |name| json!({"version":1,"tabs":{"editors":[{"path":name,"name":name}],"activeEditorPath":name},"layout":{"centerTab":"editor","activeView":"git","leftNavCollapsed":false,"rightCollapsed":false,"rightBottomCollapsed":false,"rightBottomActiveTab":"terminal","selectedDiffFile":name}});
    // When
    for name in ["first", "middle"] {
        socket
            .send(request(
                name,
                "save_workspace_state",
                json!({"worktreeName":"repo","state":state(name)}),
            ))
            .await
            .unwrap();
    }
    started.notified().await;
    socket
        .send(request(
            "barrier",
            "get_current_branch",
            json!({"repoPath":"/missing"}),
        ))
        .await
        .unwrap();
    let Body::Response(response) = receive(&mut socket).await else {
        panic!("response");
    };
    assert_eq!(
        response.request_id, "barrier",
        "saving must not block other commands"
    );
    assert_eq!(*calls.lock().unwrap(), [Some("first".into())]);
    let mut previous_socket = None;
    if connection != "same" {
        if connection == "reconnect" {
            socket.close(None).await.unwrap();
            tokio::time::timeout(Duration::from_secs(5), async {
                while slots.available_permits() != 16 {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
        }
        let (next, _) = tokio_tungstenite::connect_async(format!("ws://{address}/v1/client"))
            .await
            .unwrap();
        let previous = std::mem::replace(&mut socket, next);
        if connection == "concurrent" {
            previous_socket = Some(previous);
        }
    }
    socket
        .send(request(
            "last",
            "save_workspace_state",
            json!({"worktreeName":"repo","state":state("last")}),
        ))
        .await
        .unwrap();
    socket
        .send(request(
            "barrier",
            "get_current_branch",
            json!({"repoPath":"/missing"}),
        ))
        .await
        .unwrap();
    let Body::Response(response) = receive(&mut socket).await else {
        panic!("response");
    };
    assert_eq!(response.request_id, "barrier", "{connection}");
    assert_eq!(*calls.lock().unwrap(), [Some("first".into())]);
    gate.add_permits(1);
    let mut received = std::collections::HashSet::new();
    for _ in 0..if connection == "same" { 3 } else { 1 } {
        let Body::Response(response) = receive(&mut socket).await else {
            panic!("response");
        };
        assert!(matches!(
            response.outcome,
            Some(wire::command_response::Outcome::Result(_))
        ));
        received.insert(response.request_id);
    }
    if let Some(mut previous) = previous_socket {
        for _ in 0..2 {
            let Body::Response(response) = receive(&mut previous).await else {
                panic!("response");
            };
            assert!(matches!(
                response.outcome,
                Some(wire::command_response::Outcome::Result(_))
            ));
            received.insert(response.request_id);
        }
        previous.close(None).await.unwrap();
    }
    let expected = if connection == "reconnect" {
        vec!["last".into()]
    } else {
        vec!["first".into(), "middle".into(), "last".into()]
    };
    assert_eq!(received, expected.into_iter().collect());
    // Then
    assert_eq!(
        *calls.lock().unwrap(),
        [
            Some("first".into()),
            Some("middle".into()),
            Some("last".into())
        ]
    );
    assert_eq!(
        store
            .get("repo")
            .unwrap()
            .tabs
            .active_editor_path
            .as_deref(),
        Some("last")
    );
    let persisted: serde_json::Value = serde_json::from_slice(
        &std::fs::read(data.path().join("workspace_state/repo.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(persisted, state("last"));
    let restarted = WorkspaceStateStore::new(data.path().to_owned());
    assert_eq!(
        restarted
            .load("repo", data.path().to_str().unwrap())
            .unwrap()
            .tabs
            .active_editor_path
            .as_deref(),
        Some("last")
    );
    socket.close(None).await.unwrap();
    server.abort();
}
