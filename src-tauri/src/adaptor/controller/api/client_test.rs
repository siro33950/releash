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
        for phase in ["running", "unreceived", "received", "released"] {
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
            let operations = deps.operations.clone();
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
                if phase == "received" || phase == "released" {
                    socket
                        .send(WsMessage::Binary(
                            Envelope {
                                body: Some(Body::RequestAck(wire::RequestAck {
                                    request_id: "watch".into(),
                                    release_watch: phase == "released",
                                    confirm_watch: false,
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
            assert_eq!(
                matches!(
                    operation_response("watch", operations.query("watch", &operations.instance_id))
                        .body,
                    Some(Body::Response(_))
                ),
                phase == "received"
            );
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

#[tokio::test]
async fn test_変更要求_切断前後の結果を同じ操作で取得し副作用を重複させない() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    // Given
    for completed_before_disconnect in [false, true] {
        let effects = Arc::new(AtomicUsize::new(0));
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        let started = Arc::new(tokio::sync::Notify::new());
        let mut dispatch = dispatch();
        let (effect, wait, start) = (effects.clone(), gate.clone(), started.clone());
        dispatch.register_domain(
            &["start_workflow"],
            Box::new(move |_| {
                let (effect, wait, start) = (effect.clone(), wait.clone(), start.clone());
                Box::pin(async move {
                    effect.fetch_add(1, Ordering::SeqCst);
                    start.notify_one();
                    let _permit = wait.acquire().await.unwrap();
                    Ok(wire::command_result::Command::StartWorkflow(
                        wire::ResultString {
                            value: Some("execution".into()),
                        },
                    ))
                })
            }),
        );
        let dispatch = Arc::new(dispatch);
        let push = ClientPushGateway::new(Arc::new(crate::infrastructure::push::PushSink::new()));
        let deps = ClientApiDeps::new(dispatch.clone(), push.clone());
        let operations = deps.operations.clone();
        let epoch = operations.instance_id.to_string();
        let (mut socket, address, server) = connect(deps).await;
        let mut command = wire::CommandRequest::from_value(
            "start_workflow",
            json!({"workflowName":"test","worktreePath":"/repo"}),
        )
        .unwrap();
        command.request_id = "original".into();
        command.instance_id = epoch.clone();
        let frame = Envelope {
            body: Some(Body::Request(Box::new(command.clone()))),
        };
        socket
            .send(WsMessage::Binary(frame.encode_to_vec().into()))
            .await
            .unwrap();
        started.notified().await;
        if completed_before_disconnect {
            gate.add_permits(1);
            tokio::time::timeout(Duration::from_secs(5), async {
                while !matches!(
                    operation_response("original", operations.query("original", &epoch)).body,
                    Some(Body::Response(_))
                ) {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
        }
        // When
        socket.close(None).await.unwrap();
        let (mut recovered, _) =
            tokio_tungstenite::connect_async(format!("ws://{address}/v1/client"))
                .await
                .unwrap();
        let mut retry = command.clone();
        retry.request_id = "retry".into();
        retry.predecessors.push(wire::OperationReference {
            request_id: "original".into(),
            command: "start_workflow".into(),
            uncertain: true,
            ..Default::default()
        });
        recovered
            .send(WsMessage::Binary(
                Envelope {
                    body: Some(Body::OperationQuery(wire::OperationQuery {
                        request_id: "retry".into(),
                        instance_id: epoch.clone(),
                        request: Some(retry),
                        ..Default::default()
                    })),
                }
                .encode_to_vec()
                .into(),
            ))
            .await
            .unwrap();
        let Body::OperationStatus(bound) = receive(&mut recovered).await else {
            panic!("retry binding");
        };
        assert_eq!(bound.request_id, "retry");
        assert_eq!(bound.state, "bound");
        assert_eq!(bound.operation_id, "original");
        let query = Envelope {
            body: Some(Body::OperationQuery(wire::OperationQuery {
                request_id: bound.operation_id,
                instance_id: epoch.clone(),
                request: Some(command.clone()),
                sent: true,
                expired: true,
                query_id: String::new(),
            })),
        };
        recovered
            .send(WsMessage::Binary(query.encode_to_vec().into()))
            .await
            .unwrap();
        if !completed_before_disconnect {
            assert!(
                matches!(receive(&mut recovered).await, Body::OperationStatus(status) if status.state == "pending")
            );
            let heartbeat = Envelope {
                body: Some(Body::Heartbeat(wire::Heartbeat {
                    nonce: "alive".into(),
                })),
            };
            recovered
                .send(WsMessage::Binary(heartbeat.encode_to_vec().into()))
                .await
                .unwrap();
            assert!(
                matches!(receive(&mut recovered).await, Body::Heartbeat(value) if value.nonce == "alive")
            );
            recovered
                .send(request(
                    "other",
                    "get_current_branch",
                    json!({"repoPath":"/missing"}),
                ))
                .await
                .unwrap();
            assert!(
                matches!(receive(&mut recovered).await, Body::Response(value) if value.request_id == "other")
            );
            gate.add_permits(1);
            tokio::time::timeout(Duration::from_secs(5), async {
                while !matches!(
                    operation_response("original", operations.query("original", &epoch)).body,
                    Some(Body::Response(_))
                ) {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            recovered
                .send(WsMessage::Binary(query.encode_to_vec().into()))
                .await
                .unwrap();
        }
        // Then
        let Body::Response(result) = receive(&mut recovered).await else {
            panic!("original result");
        };
        assert_eq!(result.request_id, "original");
        let Some(wire::command_response::Outcome::Result(value)) = &result.outcome else {
            panic!("completed operation: {result:?}");
        };
        assert_eq!(wire::from_value(value.clone()).unwrap(), json!("execution"));
        recovered
            .send(WsMessage::Binary(
                Envelope {
                    body: Some(Body::OperationQuery(wire::OperationQuery {
                        request_id: "original".into(),
                        instance_id: epoch.clone(),
                        ..Default::default()
                    })),
                }
                .encode_to_vec()
                .into(),
            ))
            .await
            .unwrap();
        assert_eq!(
            receive(&mut recovered).await,
            Body::Response(result.clone())
        );
        recovered
            .send(WsMessage::Binary(frame.encode_to_vec().into()))
            .await
            .unwrap();
        assert_eq!(receive(&mut recovered).await, Body::Response(result));
        assert_eq!(effects.load(Ordering::SeqCst), 1);
        recovered.close(None).await.unwrap();
        server.abort();
        let (mut restarted, _, server) = connect(ClientApiDeps::new(dispatch, push)).await;
        restarted
            .send(WsMessage::Binary(query.encode_to_vec().into()))
            .await
            .unwrap();
        assert!(
            matches!(receive(&mut restarted).await, Body::OperationStatus(status) if status.state == "unknown")
        );
        command.recover = true;
        restarted
            .send(WsMessage::Binary(
                Envelope {
                    body: Some(Body::Request(Box::new(command))),
                }
                .encode_to_vec()
                .into(),
            ))
            .await
            .unwrap();
        assert!(
            matches!(receive(&mut restarted).await, Body::OperationStatus(status) if status.state == "unknown")
        );
        assert_eq!(effects.load(Ordering::SeqCst), 1);
        restarted.close(None).await.unwrap();
        server.abort();
    }
}

#[tokio::test]
async fn test_レビュー画像_失効した参照のws応答は元の要求に対応するエラーを返す() {
    use tauri::Manager;
    // Given
    let (app, _data_dir, _store) =
        crate::adaptor::controller::client::workflow::tests::make_read_only_app();
    app.manage(Arc::new(
        crate::infrastructure::file_watcher::FileWatcherManager::default(),
    ));
    let mut dependencies = crate::desktop_test_support::build_client_dependencies(app.handle());
    dependencies.app_state.as_mut().unwrap().review_usecase = Arc::new(
        crate::usecase::review_usecase::tests_support::review_usecase_with_snapshot_version(8),
    );
    let mut dispatch = dispatch();
    dispatch.register_dependencies(&dependencies);
    let push = app
        .state::<Arc<crate::infrastructure::push::PushSink>>()
        .inner()
        .clone();
    let (mut socket, _, server) = connect(ClientApiDeps::new(
        Arc::new(dispatch),
        ClientPushGateway::new(push),
    ))
    .await;
    // When
    socket
        .send(request(
            "stale-blob",
            "get_review_blob",
            json!({"reference": "blob?worktree=%2Frepo&path=a.png&side=modified&section=changes&base=head&version=7"}),
        ))
        .await
        .unwrap();
    // Then
    let Body::Response(response) = receive(&mut socket).await else {
        panic!("blob response");
    };
    assert_eq!(response.request_id, "stale-blob");
    let Some(wire::command_response::Outcome::Error(error)) = response.outcome else {
        panic!("stale blob must fail: {response:?}");
    };
    assert_eq!(
        wire::from_value(error).unwrap(),
        json!("stale review blob version: requested 7, current 8")
    );
    socket.close(None).await.unwrap();
    server.abort();
}

#[tokio::test]
async fn test_レビュー画像_wsの画像参照から元のmimeとbytesを取得する() {
    use base64::Engine;
    use tauri::Manager;
    // Given
    let (app, _data_dir, _store) =
        crate::adaptor::controller::client::workflow::tests::make_read_only_app();
    app.manage(Arc::new(
        crate::infrastructure::file_watcher::FileWatcherManager::default(),
    ));
    let mut dispatch = dispatch();
    dispatch.register_dependencies(&crate::desktop_test_support::build_client_dependencies(
        app.handle(),
    ));
    let push = app
        .state::<Arc<crate::infrastructure::push::PushSink>>()
        .inner()
        .clone();
    let (mut socket, _, server) = connect(ClientApiDeps::new(
        Arc::new(dispatch),
        ClientPushGateway::new(push),
    ))
    .await;
    for (extension, mime, encoded) in [
        ("png", "image/png", "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aX1sAAAAASUVORK5CYII="),
        ("gif", "image/gif", "R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7"),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let worktree_path = directory.path().canonicalize().unwrap();
        let repo = git2::Repository::init(&worktree_path).unwrap();
        let tree_id = repo.index().unwrap().write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let signature = git2::Signature::now("test", "test@example.com").unwrap();
        repo.commit(Some("HEAD"), &signature, &signature, "initial", &tree, &[]).unwrap();
        let path = format!("画像.{extension}");
        let bytes = base64::engine::general_purpose::STANDARD.decode(encoded).unwrap();
        std::fs::write(directory.path().join(&path), &bytes).unwrap();
        // When
        socket.send(request("view", "get_review_file_view", json!({"input": {
            "worktreePath": worktree_path, "target": {"by":"path", "value":path},
            "section":"changes", "base":"head", "viewport":null, "snapshotVersion":null
        }}))).await.unwrap();
        let Body::Response(response) = receive(&mut socket).await else { panic!("view response"); };
        let Some(wire::command_response::Outcome::Result(result)) = response.outcome else { panic!("view: {response:?}"); };
        let view = wire::from_value(result).unwrap();
        assert_eq!(view["kind"], "image");
        assert_eq!(view["mime"], mime);
        assert!(view["originalUrl"].is_null());
        let reference = view["modifiedUrl"].as_str().unwrap();
        assert!(reference.starts_with("blob?"));
        socket.send(request("blob", "get_review_blob", json!({"reference": reference}))).await.unwrap();
        // Then
        let Body::Response(response) = receive(&mut socket).await else { panic!("blob response"); };
        assert_eq!(response.request_id, "blob");
        let Some(wire::command_response::Outcome::Result(result)) = response.outcome else { panic!("blob: {response:?}"); };
        let result = wire::from_value(result).unwrap();
        let data = result.as_str().unwrap().strip_prefix(&format!("data:{mime};base64,")).unwrap();
        assert_eq!(base64::engine::general_purpose::STANDARD.decode(data).unwrap(), bytes);
    }
    socket.close(None).await.unwrap();
    server.abort();
}

#[tokio::test]
async fn test_接続方針_helloはrustの生存確認と操作別期限を返す() {
    // Given
    let deps = ClientApiDeps::new(
        Arc::new(dispatch()),
        ClientPushGateway::new(Arc::new(crate::infrastructure::push::PushSink::new())),
    );
    let generation = deps.operations.instance_id.clone();
    let (mut socket, _, server) = connect(deps).await;
    // When
    socket
        .send(WsMessage::Binary(
            Envelope {
                body: Some(Body::Hello(wire::ClientHello::default())),
            }
            .encode_to_vec()
            .into(),
        ))
        .await
        .unwrap();
    // Then
    let Body::Hello(hello) = receive(&mut socket).await else {
        panic!("hello")
    };
    assert_eq!(hello.instance_id, generation.as_ref());
    assert_eq!(hello.heartbeat_interval_ms, policy::HEARTBEAT_INTERVAL_MS);
    assert_eq!(hello.heartbeat_timeout_ms, policy::HEARTBEAT_TIMEOUT_MS);
    assert_eq!(hello.connect_timeout_ms, policy::CONNECT_TIMEOUT_MS);
    assert_eq!(hello.reconnect_interval_ms, policy::RECONNECT_INTERVAL_MS);
    assert_eq!(hello.sleep_gap_ms, policy::SLEEP_GAP_MS);
    assert_eq!(hello.tick_interval_ms, policy::TICK_INTERVAL_MS);
    assert_eq!(hello.deadlines_ms.get("get_current_branch"), Some(&10_000));
    for (command, deadline) in hello.deadlines_ms {
        assert_eq!(deadline, policy::deadline_ms(&command));
    }
    server.abort();
}

#[tokio::test]
async fn test_冪等操作_同世代の復旧と異なるcommandの交差でも対象の最終状態と実行順を守る() {
    // Given
    let state = Arc::new(std::sync::Mutex::new((false, Vec::new())));
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let started = Arc::new(tokio::sync::Notify::new());
    let mut dispatch = dispatch();
    for names in [&["add_repo_path"][..], &["remove_repo_path"][..]] {
        let (target, wait, start) = (state.clone(), gate.clone(), started.clone());
        dispatch.register_domain(
            names,
            Box::new(move |command| {
                let (target, wait, start) = (target.clone(), wait.clone(), start.clone());
                Box::pin(async move {
                    let add = matches!(command, wire::command_request::Command::AddRepoPath(_));
                    if add {
                        start.notify_one();
                        let _permit = wait.acquire().await.unwrap();
                    }
                    let mut state = target.lock().unwrap();
                    let changed = state.0 != add;
                    state.0 = add;
                    state.1.push(if add { "add" } else { "remove" });
                    let result = wire::ResultBool {
                        value: Some(changed),
                    };
                    Ok(if add {
                        wire::command_result::Command::AddRepoPath(result)
                    } else {
                        wire::command_result::Command::RemoveRepoPath(result)
                    })
                })
            }),
        );
    }
    let push = ClientPushGateway::new(Arc::new(crate::infrastructure::push::PushSink::new()));
    let dispatch = Arc::new(dispatch);
    let old = ClientApiDeps::new(dispatch.clone(), push.clone());
    let old_generation = old.operations.instance_id.to_string();
    let new = old.clone();
    let new_generation = new.operations.instance_id.to_string();
    let (mut socket, _, server) = connect(new).await;
    let mut add =
        wire::CommandRequest::from_value("add_repo_path", json!({"path":"/repo"})).unwrap();
    add.request_id = "add".into();
    add.instance_id = old_generation.clone();
    add.recover = true;
    let mut remove =
        wire::CommandRequest::from_value("remove_repo_path", json!({"path":"/repo"})).unwrap();
    remove.request_id = "remove".into();
    remove.instance_id = new_generation;
    remove.predecessors.push(wire::OperationReference {
        request_id: "add".into(),
        command: "add_repo_path".into(),
        uncertain: true,
        ..Default::default()
    });
    let frame = |request: wire::CommandRequest| {
        WsMessage::Binary(
            Envelope {
                body: Some(Body::Request(Box::new(request))),
            }
            .encode_to_vec()
            .into(),
        )
    };
    // When: later removal arrives before the recovery of the original addition.
    socket.send(frame(remove.clone())).await.unwrap();
    assert!(
        matches!(receive(&mut socket).await, Body::OperationStatus(status) if status.state == "blocked")
    );
    socket.send(frame(add.clone())).await.unwrap();
    started.notified().await;
    socket.send(frame(add.clone())).await.unwrap();
    assert!(
        matches!(receive(&mut socket).await, Body::OperationStatus(status) if status.state == "pending")
    );
    socket.send(frame(remove.clone())).await.unwrap();
    assert!(
        matches!(receive(&mut socket).await, Body::OperationStatus(status) if status.state == "blocked")
    );
    gate.add_permits(1);
    let added = receive(&mut socket).await;
    assert!(matches!(&added, Body::Response(response) if response.request_id == "add"));
    socket.send(frame(add.clone())).await.unwrap();
    assert_eq!(receive(&mut socket).await, added);
    socket.send(frame(remove.clone())).await.unwrap();
    let removed = receive(&mut socket).await;
    assert!(matches!(&removed, Body::Response(response) if response.request_id == "remove"));
    socket.send(frame(remove)).await.unwrap();
    assert_eq!(receive(&mut socket).await, removed);
    socket.send(frame(add)).await.unwrap();
    assert_eq!(receive(&mut socket).await, added);
    // Then
    assert_eq!(*state.lock().unwrap(), (false, vec!["add", "remove"]));
    server.abort();
}

#[tokio::test]
async fn test_確定後続操作_削除を受領した後の別世代復旧で追加を再実行しない() {
    // Given
    let state = Arc::new(std::sync::Mutex::new((false, Vec::new())));
    let mut dispatch = dispatch();
    for names in [&["add_repo_path"][..], &["remove_repo_path"][..]] {
        let target = state.clone();
        dispatch.register_domain(
            names,
            Box::new(move |command| {
                let target = target.clone();
                Box::pin(async move {
                    let add = matches!(command, wire::command_request::Command::AddRepoPath(_));
                    let mut state = target.lock().unwrap();
                    state.0 = add;
                    state.1.push(if add { "add" } else { "remove" });
                    let value = wire::ResultBool { value: Some(true) };
                    Ok(if add {
                        wire::command_result::Command::AddRepoPath(value)
                    } else {
                        wire::command_result::Command::RemoveRepoPath(value)
                    })
                })
            }),
        );
    }
    let dispatch = Arc::new(dispatch);
    let push = ClientPushGateway::new(Arc::new(crate::infrastructure::push::PushSink::new()));
    let old = ClientApiDeps::new(dispatch.clone(), push.clone());
    let generation = old.operations.instance_id.to_string();
    let operations = old.operations.clone();
    let (mut socket, _, server) = connect(old).await;
    socket
        .send(request("add", "add_repo_path", json!({"path":"/repo"})))
        .await
        .unwrap();
    assert!(matches!(receive(&mut socket).await, Body::Response(_)));
    // The original response is unacknowledged; a later, distinct command is confirmed.
    socket
        .send(request(
            "remove",
            "remove_repo_path",
            json!({"path":"/repo"}),
        ))
        .await
        .unwrap();
    assert!(matches!(receive(&mut socket).await, Body::Response(_)));
    socket
        .send(WsMessage::Binary(
            Envelope {
                body: Some(Body::RequestAck(wire::RequestAck {
                    request_id: "remove".into(),
                    release_watch: false,
                    confirm_watch: false,
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
    receive(&mut socket).await;
    assert!(matches!(
        operations.query("remove", &generation),
        crate::domain::client_operation::registry::OperationDecision::Unknown
    ));
    assert!(matches!(
        operations.query("add", &generation),
        crate::domain::client_operation::registry::OperationDecision::Completed(_)
    ));
    socket.close(None).await.unwrap();
    server.abort();
    // When
    let (mut socket, _, server) = connect(ClientApiDeps::new(dispatch, push)).await;
    let mut original =
        wire::CommandRequest::from_value("add_repo_path", json!({"path":"/repo"})).unwrap();
    original.request_id = "add".into();
    original.instance_id = generation.clone();
    original.recover = true;
    let remove =
        wire::CommandRequest::from_value("remove_repo_path", json!({"path":"/repo"})).unwrap();
    let identity = crate::adaptor::controller::api::client_operation::identity(&remove).unwrap();
    original.successors.push(wire::OperationReference {
        request_id: "remove".into(),
        command: "remove_repo_path".into(),
        uncertain: false,
        fingerprint: identity.fingerprint.to_vec(),
        ordering_target: identity.target.unwrap().1.to_vec(),
    });
    for user_retry in [false, true] {
        original.user_retry = user_retry;
        socket
            .send(WsMessage::Binary(
                Envelope {
                    body: Some(Body::OperationQuery(wire::OperationQuery {
                        request_id: "add".into(),
                        instance_id: generation.clone(),
                        request: Some(original.clone()),
                        sent: true,
                        expired: false,
                        query_id: String::new(),
                    })),
                }
                .encode_to_vec()
                .into(),
            ))
            .await
            .unwrap();
        // Then
        assert!(
            matches!(receive(&mut socket).await, Body::OperationStatus(status) if status.state == "unknown")
        );
        socket
            .send(WsMessage::Binary(
                Envelope {
                    body: Some(Body::Request(Box::new(original.clone()))),
                }
                .encode_to_vec()
                .into(),
            ))
            .await
            .unwrap();
        assert!(
            matches!(receive(&mut socket).await, Body::OperationStatus(status) if status.state == "unknown")
        );
    }
    assert_eq!(*state.lock().unwrap(), (false, vec!["add", "remove"]));
    server.abort();
}

#[tokio::test]
async fn test_未受理の期限超過_ws照会と古いpacketは実行せず利用者再試行だけ一度実行する() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    // Given
    let effects = Arc::new(AtomicUsize::new(0));
    let mut dispatch = dispatch();
    let count = effects.clone();
    dispatch.register_domain(
        &["request_application_quit"],
        Box::new(move |_| {
            count.fetch_add(1, Ordering::SeqCst);
            Box::pin(async {
                Err(crate::other::AppError::coded("CONFIRMED", "confirmed result").into())
            })
        }),
    );
    let deps = ClientApiDeps::new(
        Arc::new(dispatch),
        ClientPushGateway::new(Arc::new(crate::infrastructure::push::PushSink::new())),
    );
    let (mut socket, _, server) = connect(deps).await;
    let mut original = wire::CommandRequest::from_value(
        "request_application_quit",
        json!({"request":{"request_id":"quit-original","intent":{"type":"exit","code":0}}}),
    )
    .unwrap();
    original.request_id = "quit".into();
    original.instance_id = "old-backend".into();
    original.recover = true;
    original.deadline_unix_ms = super::super::client_operation::now_ms() - 1;
    // When / Then
    socket
        .send(WsMessage::Binary(
            Envelope {
                body: Some(Body::OperationQuery(wire::OperationQuery {
                    request_id: "quit".into(),
                    instance_id: "old-backend".into(),
                    request: Some(original.clone()),
                    sent: true,
                    expired: false,
                    query_id: String::new(),
                })),
            }
            .encode_to_vec()
            .into(),
        ))
        .await
        .unwrap();
    assert!(
        matches!(receive(&mut socket).await, Body::OperationStatus(status) if status.state == "unknown")
    );
    socket
        .send(WsMessage::Binary(
            Envelope {
                body: Some(Body::Request(Box::new(original.clone()))),
            }
            .encode_to_vec()
            .into(),
        ))
        .await
        .unwrap();
    assert!(
        matches!(receive(&mut socket).await, Body::OperationStatus(status) if status.state == "unknown")
    );
    assert_eq!(effects.load(Ordering::SeqCst), 0);
    original.user_retry = true;
    original.deadline_unix_ms = super::super::client_operation::now_ms() + 120_000;
    for _ in 0..2 {
        socket
            .send(WsMessage::Binary(
                Envelope {
                    body: Some(Body::Request(Box::new(original.clone()))),
                }
                .encode_to_vec()
                .into(),
            ))
            .await
            .unwrap();
        assert!(
            matches!(receive(&mut socket).await, Body::Response(response) if response.request_id == "quit")
        );
    }
    assert_eq!(effects.load(Ordering::SeqCst), 1);
    server.abort();
}

#[tokio::test]
async fn test_監視の結果照会と遅延応答の交差_同じ監視を一度だけ開始して確定する() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    // Given
    let starts = Arc::new(AtomicUsize::new(0));
    let ready = Arc::new(tokio::sync::Semaphore::new(0));
    let mut dispatch = dispatch();
    let (count, wait) = (starts.clone(), ready.clone());
    dispatch.register_domain(
        &["start_watching"],
        Box::new(move |_| {
            count.fetch_add(1, Ordering::SeqCst);
            let wait = wait.clone();
            Box::pin(async move {
                let _permit = wait.acquire().await.unwrap();
                Ok(wire::command_result::Command::StartWatching(
                    wire::ResultUint64 { value: Some(42) },
                ))
            })
        }),
    );
    let deps = ClientApiDeps::new(
        Arc::new(dispatch),
        ClientPushGateway::new(Arc::new(crate::infrastructure::push::PushSink::new())),
    );
    let operations = deps.operations.clone();
    let generation = operations.instance_id.to_string();
    let (mut socket, _, server) = connect(deps).await;
    let mut original =
        wire::CommandRequest::from_value("start_watching", json!({"path":"/repo"})).unwrap();
    original.request_id = "watch".into();
    original.instance_id = generation.clone();
    socket
        .send(WsMessage::Binary(
            Envelope {
                body: Some(Body::Request(Box::new(original.clone()))),
            }
            .encode_to_vec()
            .into(),
        ))
        .await
        .unwrap();
    let query = || {
        WsMessage::Binary(
            Envelope {
                body: Some(Body::OperationQuery(wire::OperationQuery {
                    request_id: "watch".into(),
                    instance_id: generation.clone(),
                    request: Some(original.clone()),
                    sent: true,
                    expired: true,
                    query_id: String::new(),
                })),
            }
            .encode_to_vec()
            .into(),
        )
    };
    // When
    socket.send(query()).await.unwrap();
    assert!(
        matches!(receive(&mut socket).await, Body::OperationStatus(status) if status.state == "pending")
    );
    ready.add_permits(1);
    let original_result = receive(&mut socket).await;
    socket.send(query()).await.unwrap();
    let queried_result = receive(&mut socket).await;
    // Then
    assert_eq!(original_result, queried_result);
    for _ in 0..2 {
        socket
            .send(WsMessage::Binary(
                Envelope {
                    body: Some(Body::RequestAck(wire::RequestAck {
                        request_id: "watch".into(),
                        release_watch: false,
                        confirm_watch: false,
                    })),
                }
                .encode_to_vec()
                .into(),
            ))
            .await
            .unwrap();
    }
    socket.send(query()).await.unwrap();
    assert_eq!(receive(&mut socket).await, original_result);
    assert_eq!(starts.load(Ordering::SeqCst), 1);
    socket.close(None).await.unwrap();
    assert!(matches!(
        operations.query("watch", &generation),
        crate::domain::client_operation::registry::OperationDecision::Completed(_)
    ));
    server.abort();
}

#[tokio::test]
async fn test_監視の保持期限_遅延受領を拒否して新しい購読の実在を確認する() {
    use std::sync::atomic::{AtomicU64, Ordering};
    // Given
    let now = Arc::new(AtomicU64::new(0));
    let starts = Arc::new(AtomicU64::new(0));
    let stopped = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut dispatch = dispatch();
    let count = starts.clone();
    dispatch.register_domain(
        &["start_watching"],
        Box::new(move |_| {
            let id = count.fetch_add(1, Ordering::SeqCst) + 1;
            Box::pin(async move {
                Ok(wire::command_result::Command::StartWatching(
                    wire::ResultUint64 { value: Some(id) },
                ))
            })
        }),
    );
    let mut deps = ClientApiDeps::new(
        Arc::new(dispatch),
        ClientPushGateway::new(Arc::new(crate::infrastructure::push::PushSink::new())),
    );
    deps.operations = Arc::new(ClientOperationUsecase::new(
        "current".into(),
        {
            let now = now.clone();
            Arc::new(move || now.load(Ordering::SeqCst))
        },
        {
            let stopped = stopped.clone();
            Arc::new(move |id| {
                let stopped = stopped.clone();
                Box::pin(async move {
                    stopped.lock().unwrap().push(id);
                })
            })
        },
    ));
    let (mut socket, _, server) = connect(deps).await;
    socket
        .send(request("old", "start_watching", json!({"path":"/repo"})))
        .await
        .unwrap();
    assert!(matches!(receive(&mut socket).await, Body::Response(_)));
    // When
    now.store(policy::OPERATION_RETENTION_MS, Ordering::SeqCst);
    socket
        .send(WsMessage::Binary(
            Envelope {
                body: Some(Body::RequestAck(wire::RequestAck {
                    request_id: "old".into(),
                    release_watch: false,
                    confirm_watch: true,
                })),
            }
            .encode_to_vec()
            .into(),
        ))
        .await
        .unwrap();
    // Then
    assert!(
        matches!(receive(&mut socket).await, Body::OperationStatus(status) if status.state == "watch_released")
    );
    assert_eq!(*stopped.lock().unwrap(), [1]);
    let mut replacement =
        wire::CommandRequest::from_value("start_watching", json!({"path":"/repo"})).unwrap();
    let identity =
        crate::adaptor::controller::api::client_operation::identity(&replacement).unwrap();
    replacement.request_id = "replacement".into();
    replacement.instance_id = "current".into();
    replacement.predecessors.push(wire::OperationReference {
        request_id: "old".into(),
        command: "start_watching".into(),
        uncertain: true,
        fingerprint: identity.fingerprint.to_vec(),
        ..Default::default()
    });
    socket
        .send(WsMessage::Binary(
            Envelope {
                body: Some(Body::OperationQuery(wire::OperationQuery {
                    request_id: "replacement".into(),
                    instance_id: "current".into(),
                    request: Some(replacement.clone()),
                    query_id: "proposal".into(),
                    ..Default::default()
                })),
            }
            .encode_to_vec()
            .into(),
        ))
        .await
        .unwrap();
    assert!(
        matches!(receive(&mut socket).await, Body::OperationStatus(status) if status.state == "ready" && status.query_id == "proposal")
    );
    socket
        .send(WsMessage::Binary(
            Envelope {
                body: Some(Body::Request(Box::new(replacement))),
            }
            .encode_to_vec()
            .into(),
        ))
        .await
        .unwrap();
    assert!(matches!(receive(&mut socket).await, Body::Response(_)));
    socket
        .send(WsMessage::Binary(
            Envelope {
                body: Some(Body::RequestAck(wire::RequestAck {
                    request_id: "replacement".into(),
                    release_watch: false,
                    confirm_watch: true,
                })),
            }
            .encode_to_vec()
            .into(),
        ))
        .await
        .unwrap();
    assert!(
        matches!(receive(&mut socket).await, Body::OperationStatus(status) if status.state == "watch_active")
    );
    assert_eq!(starts.load(Ordering::SeqCst), 2);
    assert_eq!(*stopped.lock().unwrap(), [1]);
    server.abort();
}

#[tokio::test]
async fn test_別世代の冪等操作_後続操作がなくても自動照会と再試行でdispatchしない() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    // Given
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let mut dispatch = dispatch();
    dispatch.register_domain(
        &["add_repo_path"],
        Box::new(move |_| {
            count.fetch_add(1, Ordering::SeqCst);
            Box::pin(async {
                Ok(wire::command_result::Command::AddRepoPath(
                    wire::ResultBool { value: Some(true) },
                ))
            })
        }),
    );
    let deps = ClientApiDeps::new(
        Arc::new(dispatch),
        ClientPushGateway::new(Arc::new(crate::infrastructure::push::PushSink::new())),
    );
    let (mut socket, _, server) = connect(deps).await;
    let mut original =
        wire::CommandRequest::from_value("add_repo_path", json!({"path":"/repo"})).unwrap();
    original.request_id = "original".into();
    original.instance_id = "old".into();
    original.recover = true;
    // When / Then
    for user_retry in [false, true] {
        original.user_retry = user_retry;
        for body in [
            Body::OperationQuery(wire::OperationQuery {
                request_id: original.request_id.clone(),
                instance_id: "old".into(),
                sent: true,
                request: Some(original.clone()),
                ..Default::default()
            }),
            Body::Request(Box::new(original.clone())),
        ] {
            socket
                .send(WsMessage::Binary(
                    Envelope { body: Some(body) }.encode_to_vec().into(),
                ))
                .await
                .unwrap();
            assert!(
                matches!(receive(&mut socket).await, Body::OperationStatus(status) if status.state == "unknown" && status.request_id == "original")
            );
        }
    }
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    server.abort();
}
