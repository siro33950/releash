use releash_lib::test_support::integration::fixtures::fixtures_adaptor_controller_api_client_dispatch as dispatch;
use releash_lib::test_support::integration::wire::state_payload::Value;

use releash_lib::test_support::integration::subscriptions::StateSubscriptionUsecase;
use releash_lib::test_support::integration::subscriptions::SubscriptionTarget;
use releash_lib::test_support::integration::transport::router;
use releash_lib::test_support::integration::transport::rpc;
use releash_lib::test_support::integration::transport::to_rpc;
use releash_lib::test_support::integration::transport::to_wire;
use releash_lib::test_support::integration::transport::ClientCommandDispatch;
use releash_lib::test_support::integration::wire;
use releash_lib::test_support::integration::wire::state_subscription_event::Event;
use std::sync::Arc;
use std::time::Duration;

use connectrpc::client::ClientConfig;
use connectrpc::client::HttpClient;
use prost::Message;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

async fn serve(
    dispatch: ClientCommandDispatch,
) -> (
    rpc::ClientServiceClient<HttpClient>,
    tokio::task::JoinHandle<()>,
) {
    let router = router(
        Some(
            releash_lib::test_support::integration::transport::client_api_deps(Arc::new(dispatch)),
        ),
        releash_lib::test_support::integration::daemon::default_timeout(),
    )
    .layer(axum::middleware::from_fn_with_state(
        releash_lib::test_support::integration::transport::ClientBearerToken::from(
            Arc::<str>::from("client"),
        ),
        releash_lib::test_support::integration::transport::require_client,
    ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let config = ClientConfig::new(
        format!("http://{}", listener.local_addr().unwrap())
            .parse()
            .unwrap(),
    )
    .with_default_header("authorization", "Bearer client")
    .with_default_header("origin", "tauri://localhost");
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    (
        rpc::ClientServiceClient::new(HttpClient::plaintext(), config),
        task,
    )
}

#[tokio::test]
pub async fn test_connect_生成clientのunaryで結果と構造化エラーを返す() {
    // Given
    let mut dispatch = dispatch();
    dispatch.register_domain(
        &["add_repo_path"],
        Box::new(|_| {
            Box::pin(async {
                Ok(wire::command_result::Command::AddRepoPath(
                    wire::ResultBool { value: Some(true) },
                ))
            })
        }),
    );
    dispatch.register_domain(
        &["build_diff_file_tree"],
        Box::new(|command| {
            Box::pin(async move {
                let wire::command_request::Command::BuildDiffFileTree(args) = command else {
                    unreachable!()
                };
                releash_lib::test_support::integration::transport::required(
                    args.entries,
                    "entries",
                )?;
                Ok(wire::command_result::Command::BuildDiffFileTree(
                    Default::default(),
                ))
            })
        }),
    );
    let (client, server) = serve(dispatch).await;
    // When / Then
    assert_eq!(
        client
            .add_repo_path(rpc::AddRepoPathRequest::default())
            .await
            .unwrap()
            .into_owned()
            .value,
        Some(true)
    );
    let error = client
        .build_diff_file_tree(rpc::BuildDiffFileTreeRequest::default())
        .await
        .unwrap_err();
    assert_eq!(error.code, connectrpc::ErrorCode::InvalidArgument);
    assert_eq!(error.details[0].type_url, "releash.client.v1.CommandError");
    server.abort();
}

#[tokio::test]
pub async fn test_応答未到達_副作用は完了するが照会と再送は行わず現在状態を取得する() {
    // Given
    let effects = Arc::new(AtomicUsize::new(0));
    let started = Arc::new(tokio::sync::Notify::new());
    let finish = Arc::new(tokio::sync::Notify::new());
    let mut dispatch = dispatch();
    let (changed, began, done) = (effects.clone(), started.clone(), finish.clone());
    dispatch.register_domain(
        &["update_crash_reporting"],
        Box::new(move |_| {
            let (changed, began, done) = (changed.clone(), began.clone(), done.clone());
            Box::pin(async move {
                began.notify_one();
                done.notified().await;
                changed.fetch_add(1, Ordering::SeqCst);
                Ok(wire::command_result::Command::UpdateCrashReporting(
                    wire::Unit {},
                ))
            })
        }),
    );
    let count = effects.clone();
    dispatch.register_domain(
        &["add_repo_path"],
        Box::new(move |_| {
            let count = count.clone();
            Box::pin(async move {
                Ok(wire::command_result::Command::AddRepoPath(
                    wire::ResultBool {
                        value: Some(count.load(Ordering::SeqCst) != 0),
                    },
                ))
            })
        }),
    );
    let (client, server) = serve(dispatch).await;
    // When
    let request = client.update_crash_reporting_with_options(
        rpc::UpdateCrashReportingRequest {
            enabled: Some(true),
            ..Default::default()
        },
        connectrpc::client::CallOptions::default()
            .with_timeout(std::time::Duration::from_millis(50)),
    );
    let (_, result) = tokio::join!(started.notified(), request);
    assert!(result.is_err());
    finish.notify_one();
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while effects.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    // Then
    assert_eq!(
        client
            .add_repo_path(rpc::AddRepoPathRequest::default())
            .await
            .unwrap()
            .into_owned()
            .value,
        Some(true)
    );
    assert_eq!(effects.load(Ordering::SeqCst), 1);
    server.abort();
}

#[tokio::test]
pub async fn test_connect_変更前と同じ16mibまで要求を受理する() {
    // Given
    let received = Arc::new(AtomicUsize::new(0));
    let size = received.clone();
    let mut dispatch = dispatch();
    dispatch.register_domain(
        &["update_external_editor"],
        Box::new(move |command| {
            let size = size.clone();
            Box::pin(async move {
                let wire::command_request::Command::UpdateExternalEditor(args) = command else {
                    unreachable!()
                };
                size.store(args.editor.unwrap().len(), Ordering::SeqCst);
                Ok(wire::command_result::Command::UpdateExternalEditor(
                    wire::Unit {},
                ))
            })
        }),
    );
    let (client, server) = serve(dispatch).await;
    // When / Then
    let accepted = 16 * 1024 * 1024 - 5;
    client
        .update_external_editor(rpc::UpdateExternalEditorRequest {
            editor: Some("x".repeat(accepted)),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(received.load(Ordering::SeqCst), accepted);
    assert_eq!(
        client
            .update_external_editor(rpc::UpdateExternalEditorRequest {
                editor: Some("x".repeat(16 * 1024 * 1024)),
                ..Default::default()
            })
            .await
            .unwrap_err()
            .code,
        connectrpc::ErrorCode::ResourceExhausted
    );
    assert_eq!(received.load(Ordering::SeqCst), accepted);
    server.abort();
}

#[tokio::test]
pub async fn test_状態購読stream_全段の枠が埋まっていてもイベントを受け取り席を使わない() {
    // Given
    let subscriptions =
        releash_lib::test_support::integration::subscriptions::StateSubscriptionUsecase::new(
            vec![],
            releash_lib::test_support::integration::subscriptions::read_driver(),
        );
    let deps =
        releash_lib::test_support::integration::transport::client_api_deps(Arc::new(dispatch()))
            .with_state_subscriptions(subscriptions.deps());
    let _permits =
        ["interactive", "workflow", "default"].map(|level| deps.priority_limits().fill(level));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let config = ClientConfig::new(
        format!("http://{}", listener.local_addr().unwrap())
            .parse()
            .unwrap(),
    );
    let server = tokio::spawn({
        let deps = deps.clone();
        async move {
            axum::serve(
                listener,
                router(
                    Some(deps),
                    releash_lib::test_support::integration::daemon::default_timeout(),
                ),
            )
            .await
            .unwrap()
        }
    });
    let client = rpc::ClientServiceClient::new(HttpClient::plaintext(), config);

    // When
    let mut stream = client
        .open_state_stream(rpc::OpenStateStreamRequest {
            client_id: "priority-bypass".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    let event: wire::StateSubscriptionEvent = to_wire(
        &stream
            .message::<rpc::StateSubscriptionEvent>()
            .await
            .unwrap()
            .unwrap()
            .to_owned_message(),
    )
    .unwrap();

    // Then
    assert!(matches!(
        event.event,
        Some(wire::state_subscription_event::Event::Ready(_))
    ));
    for level in ["interactive", "workflow", "default"] {
        assert_eq!(deps.priority_limits().available(level), 0);
    }
    drop(stream);
    server.abort();
}

#[tokio::test]
pub async fn test_状態購読_購読idを入口で128バイトまで受け付ける() {
    // Given
    let subscriptions =
        releash_lib::test_support::integration::subscriptions::StateSubscriptionUsecase::new(
            vec![],
            releash_lib::test_support::integration::subscriptions::read_driver(),
        );
    let deps =
        releash_lib::test_support::integration::transport::client_api_deps(Arc::new(dispatch()))
            .with_state_subscriptions(subscriptions.deps());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let config = ClientConfig::new(
        format!("http://{}", listener.local_addr().unwrap())
            .parse()
            .unwrap(),
    );
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            router(
                Some(deps),
                releash_lib::test_support::integration::daemon::default_timeout(),
            ),
        )
        .await
        .unwrap();
    });
    let client = rpc::ClientServiceClient::new(HttpClient::plaintext(), config);
    // When
    for id in [String::new(), "x".repeat(129), "あ".repeat(43)] {
        let mut stream = client
            .open_state_stream(rpc::OpenStateStreamRequest {
                client_id: id,
                ..Default::default()
            })
            .await
            .unwrap();
        // Then
        assert_eq!(
            stream
                .message::<rpc::StateSubscriptionEvent>()
                .await
                .unwrap_err()
                .code,
            connectrpc::ErrorCode::InvalidArgument
        );
    }
    // When
    let mut stream = client
        .open_state_stream(rpc::OpenStateStreamRequest {
            client_id: "x".repeat(128),
            ..Default::default()
        })
        .await
        .unwrap();
    // Then
    assert!(stream
        .message::<rpc::StateSubscriptionEvent>()
        .await
        .unwrap()
        .is_some());
    server.abort();
}

#[tokio::test]
pub async fn test_状態購読_connectで初期状態と変更と再開を配信する() {
    use releash_lib::test_support::integration::subscriptions::StateSubscriptionUsecase;
    use releash_lib::test_support::integration::subscriptions::SubscriptionTarget;
    use wire::state_subscription_event::Event;
    // Given
    let subscriptions = StateSubscriptionUsecase::new(
        vec!["/repo".into()],
        releash_lib::test_support::integration::subscriptions::read_driver(),
    );
    let deps =
        releash_lib::test_support::integration::transport::client_api_deps(Arc::new(dispatch()))
            .with_state_subscriptions(subscriptions.deps());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let config = ClientConfig::new(
        format!("http://{}", listener.local_addr().unwrap())
            .parse()
            .unwrap(),
    );
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            router(
                Some(deps),
                releash_lib::test_support::integration::daemon::default_timeout(),
            ),
        )
        .await
        .unwrap();
    });
    let client = rpc::ClientServiceClient::new(HttpClient::plaintext(), config);
    let mut stream = client
        .open_state_stream(rpc::OpenStateStreamRequest {
            client_id: "state-test".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    let ready: wire::StateSubscriptionEvent = to_wire(
        &stream
            .message::<rpc::StateSubscriptionEvent>()
            .await
            .unwrap()
            .unwrap()
            .to_owned_message(),
    )
    .unwrap();
    assert!(matches!(ready.event, Some(Event::Ready(_))));
    // When
    let request = wire::StartStateSubscriptionRequest {
        client_id: "state-test".into(),
        target: SubscriptionTarget::RepositoryPaths.to_string(),
        args: vec![],
        version: None,
        subscription_id: "state-subscription".into(),
    };
    client
        .start_state_subscription(to_rpc::<rpc::StartStateSubscriptionRequest>(&request).unwrap())
        .await
        .unwrap();
    assert_eq!(
        client
            .start_state_subscription(
                to_rpc::<rpc::StartStateSubscriptionRequest>(&request).unwrap()
            )
            .await
            .unwrap_err()
            .code,
        connectrpc::ErrorCode::AlreadyExists
    );
    // Then
    let initial: wire::StateSubscriptionEvent = to_wire(
        &stream
            .message::<rpc::StateSubscriptionEvent>()
            .await
            .unwrap()
            .unwrap()
            .to_owned_message(),
    )
    .unwrap();
    assert!(matches!(initial.event, Some(Event::Snapshot(_))));
    let bookmark: wire::StateSubscriptionEvent = to_wire(
        &stream
            .message::<rpc::StateSubscriptionEvent>()
            .await
            .unwrap()
            .unwrap()
            .to_owned_message(),
    )
    .unwrap();
    assert!(matches!(bookmark.event, Some(Event::Bookmark(_))));
    subscriptions.test_set_repository_paths(vec!["/next".into()]);
    subscriptions.notify(
        releash_lib::test_support::integration::subscriptions::StateChangeSource::Repositories,
    );
    let changed: wire::StateSubscriptionEvent = to_wire(
        &stream
            .message::<rpc::StateSubscriptionEvent>()
            .await
            .unwrap()
            .unwrap()
            .to_owned_message(),
    )
    .unwrap();
    assert!(matches!(changed.event, Some(Event::Change(_))));
    let changed_version = changed.version.unwrap();
    assert_eq!(changed_version.sequence, 1);
    client
        .stop_state_subscription(rpc::StopStateSubscriptionRequest {
            subscription_id: "state-subscription".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    let error = client
        .start_state_subscription(rpc::StartStateSubscriptionRequest {
            client_id: "state-test".into(),
            target: "missing".into(),
            subscription_id: "missing".into(),
            ..Default::default()
        })
        .await
        .unwrap_err();
    assert_eq!(error.code, connectrpc::ErrorCode::NotFound);
    client
        .start_state_subscription(
            to_rpc::<rpc::StartStateSubscriptionRequest>(&wire::StartStateSubscriptionRequest {
                version: initial.version,
                ..request
            })
            .unwrap(),
        )
        .await
        .unwrap();
    let resumed: wire::StateSubscriptionEvent = to_wire(
        &stream
            .message::<rpc::StateSubscriptionEvent>()
            .await
            .unwrap()
            .unwrap()
            .to_owned_message(),
    )
    .unwrap();
    assert_eq!(resumed.version, Some(changed_version));
    let Some(Event::Change(payload)) = resumed.event else {
        panic!("repository paths change after resubscription");
    };
    assert_eq!(
        payload.payload.and_then(|payload| payload.value),
        Some(wire::state_payload::Value::RepositoryPaths(
            wire::Liststring {
                items: vec!["/next".into()],
            }
        ))
    );
    drop(stream);
    server.abort();
}

#[tokio::test]
pub async fn test_単発rpc_client切断で処理が終了する() {
    use tokio::io::AsyncWriteExt;
    // Given
    let stopped = tokio_util::sync::CancellationToken::new();
    let started = tokio_util::sync::CancellationToken::new();
    let signal = stopped.clone();
    let start_signal = started.clone();
    let mut dispatch = dispatch();
    dispatch.register_domain(
        &["update_external_editor"],
        Box::new(move |_| {
            let guard = signal.clone().drop_guard();
            start_signal.cancel();
            Box::pin(async move {
                let _guard = guard;
                std::future::pending().await
            })
        }),
    );
    let deps =
        releash_lib::test_support::integration::transport::client_api_deps(Arc::new(dispatch));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let router = router(
        Some(deps.clone()),
        releash_lib::test_support::integration::daemon::default_timeout(),
    );
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let mut connection = tokio::net::TcpStream::connect(address).await.unwrap();
    connection.write_all(b"POST /releash.client.v1.ClientService/UpdateExternalEditor HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nConnect-Protocol-Version: 1\r\nContent-Length: 2\r\n\r\n{}").await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(2), started.cancelled())
        .await
        .unwrap();
    // When
    drop(connection);
    // Then
    let result = tokio::time::timeout(std::time::Duration::from_secs(2), stopped.cancelled()).await;
    server.abort();
    result.unwrap();
    assert_eq!(deps.priority_limits().available("default"), 41);
}

#[tokio::test]
pub async fn test_terminal購読_connectの後段配線と差分再開と流量停止中の応答を保証する() {
    use releash_lib::test_support::integration::platform::TerminalSurfaceEventHub;
    use releash_lib::test_support::integration::terminal::TerminalSurface;
    use releash_lib::test_support::integration::terminal::TerminalSurfaceEventSink;
    use releash_lib::test_support::integration::terminal::TerminalSurfaceOutputControl;
    use releash_lib::test_support::integration::terminal::TerminalSurfaceOutputEvent;

    use releash_lib::test_support::integration::terminal::TerminalSurfaceOwner;
    use releash_lib::test_support::integration::workspace::WorkspaceIdentity;

    use releash_lib::test_support::integration::terminal::FakePtyGateway;
    use releash_lib::test_support::integration::terminal::TerminalSurfaceApplication;

    use wire::state_payload::Value;

    use wire::terminal_event::Item;

    // Given
    let first = TerminalSurface::new(
        1,
        TerminalSurfaceOwner::workspace(WorkspaceIdentity::new("/first")).unwrap(),
        None,
    );
    let second = TerminalSurface::new(
        2,
        TerminalSurfaceOwner::workspace(WorkspaceIdentity::new("/second")).unwrap(),
        None,
    );
    let mut gateway = FakePtyGateway::new();
    gateway.additional_surfaces = vec![first.clone(), second.clone()];
    let gateway = Arc::new(gateway);
    let hub = Arc::new(TerminalSurfaceEventHub::with_flags(256, true));
    hub.initialize(
        releash_lib::test_support::integration::subscriptions::registration(
            &first.session_key,
            "/first",
            None,
            1,
            0,
        ),
    )
    .unwrap();
    hub.initialize(
        releash_lib::test_support::integration::subscriptions::registration(
            &second.session_key,
            "/second",
            None,
            2,
            0,
        ),
    )
    .unwrap();
    let terminal = Arc::new(TerminalSurfaceApplication::new(
        std::sync::Arc::new(releash_lib::test_support::integration::telemetry::TelemetryGateway),
        gateway.clone(),
        Arc::new(releash_lib::test_support::integration::terminal::TerminalSurfaceEventSourceGateway::new(hub.event_sender())),
        hub.clone(),
    ));
    let (app, _, _) =
        crate::adaptor_controller_client_workflow_mod::tests::make_read_only_app_with_terminal(
            terminal.clone(),
        );
    let mut dependencies = app.client;
    dependencies.workflow_runtime_usecase = Some(Arc::new(
        releash_lib::test_support::integration::workflow::WorkflowRuntimeUsecase::new(
            Arc::new(
                crate::adaptor_controller_api_mod::test_support::RecordingRuntimeGateway::default(),
            ),
            Arc::new(releash_lib::test_support::integration::workflow::NoopArchiveRepository),
        ),
    ));
    let mut dispatch = dispatch();
    dispatch.register_dependencies(&dependencies);
    let subscriptions = StateSubscriptionUsecase::new(
        vec!["/repo".into()],
        releash_lib::test_support::integration::subscriptions::read_driver(),
    );
    let subscriptions = subscriptions.with_terminal(terminal);
    let deps =
        releash_lib::test_support::integration::transport::client_api_deps(Arc::new(dispatch))
            .with_state_subscriptions(subscriptions.deps());
    assert_eq!(*gateway.list_summaries_calls.lock(), 0);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let config = ClientConfig::new(
        format!("http://{}", listener.local_addr().unwrap())
            .parse()
            .unwrap(),
    );
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            router(
                Some(deps),
                releash_lib::test_support::integration::daemon::default_timeout(),
            ),
        )
        .await
        .unwrap();
    });
    let client = rpc::ClientServiceClient::new(HttpClient::plaintext(), config);
    let mut stream = client
        .open_state_stream(rpc::OpenStateStreamRequest {
            client_id: "terminal-client".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    macro_rules! next_event {
        () => {{
            let message = tokio::time::timeout(
                Duration::from_secs(2),
                stream.message::<rpc::StateSubscriptionEvent>(),
            )
            .await
            .expect("state stream deadline")
            .unwrap()
            .unwrap()
            .to_owned_message();
            let event: wire::StateSubscriptionEvent = to_wire(&message).unwrap();
            event
        }};
    }
    assert!(matches!(next_event!().event, Some(Event::Ready(_))));

    // When / Then: terminal is wired after subscriptions, as in build_router.
    let mut initial_version = None;
    for path in ["/first", "/second"] {
        client
            .start_state_subscription(rpc::StartStateSubscriptionRequest {
                client_id: "terminal-client".into(),
                target: "terminal".into(),
                args: vec![path.into()],
                subscription_id: format!("input-{path}"),
                ..Default::default()
            })
            .await
            .unwrap();
        let initial = next_event!();
        assert_eq!(initial.subscription_id, format!("input-{path}"));
        assert_eq!(initial.version.as_ref().unwrap().sequence, 0);
        let Some(Event::Snapshot(payload)) = initial.event else {
            panic!("terminal snapshot");
        };
        let Some(Value::Terminal(event)) = payload.value else {
            panic!("terminal payload");
        };
        let Some(Item::Snapshot(snapshot)) = event.item else {
            panic!("initial terminal state");
        };
        assert_eq!(snapshot.processed_report_units, 5000);
        assert_eq!(snapshot.sequence, 0);
        if path == "/first" {
            initial_version = initial.version;
        }
        assert!(matches!(next_event!().event, Some(Event::Bookmark(_))));
    }
    client
        .start_state_subscription(rpc::StartStateSubscriptionRequest {
            client_id: "terminal-client".into(),
            target: "repository-paths".into(),
            subscription_id: "repositories".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    let repository = next_event!();
    assert_eq!(repository.subscription_id, "repositories");
    assert!(matches!(repository.event, Some(Event::Snapshot(_))));
    assert!(matches!(next_event!().event, Some(Event::Bookmark(_))));

    // When: unprocessed UTF-16 output exceeds the high watermark.
    let data: Arc<str> = "🙂".repeat(50_001).into();
    hub.publish(TerminalSurfaceOutputEvent::Output {
        session_key: first.session_key.clone(),
        data: data.clone(),
        sequence: 1,
    });
    let output = next_event!();
    assert_eq!(output.version.as_ref().unwrap().sequence, 1);
    let Some(Event::Change(change)) = output.event else {
        panic!("output delta");
    };
    assert!(change.delta);
    let Some(Value::Terminal(event)) = change.payload.unwrap().value else {
        panic!("terminal payload");
    };
    assert!(
        matches!(event.item, Some(Item::Output(value)) if value.data == data.as_ref() && value.sequence == 1)
    );
    struct ReleaseSource(Arc<TerminalSurfaceEventHub>, String);
    impl Drop for ReleaseSource {
        fn drop(&mut self) {
            self.0.release_output(&self.1);
        }
    }
    let _release_source = ReleaseSource(hub.clone(), first.session_key.clone());
    let mut paused = tokio::task::spawn_blocking({
        let hub = hub.clone();
        let key = first.session_key.clone();
        move || hub.wait_output(&key)
    });
    assert!(tokio::time::timeout(Duration::from_millis(50), &mut paused)
        .await
        .is_err());

    // Then: another terminal, another RPC, and resize of the paused terminal progress.
    hub.publish(TerminalSurfaceOutputEvent::Output {
        session_key: second.session_key.clone(),
        data: "still running".into(),
        sequence: 1,
    });
    let other = next_event!();
    assert_eq!(other.subscription_id, "input-/second");
    let Some(Event::Change(change)) = other.event else {
        panic!("other terminal delta");
    };
    let Some(Value::Terminal(event)) = change.payload.unwrap().value else {
        panic!("terminal payload");
    };
    assert!(matches!(event.item, Some(Item::Output(value)) if value.data == "still running"));
    tokio::time::timeout(
        Duration::from_secs(2),
        client.get_server_info(rpc::Unit::default()),
    )
    .await
    .expect("other RPC deadline")
    .unwrap();
    let request = wire::command_request_from_value("resize_terminal_surface", serde_json::json!({"owner": {"kind": "workspace", "workspacePath": "/first"}, "rows": 30, "cols": 120})).unwrap();
    let wire::command_request::Command::ResizeTerminalSurface(request) = request.command.unwrap()
    else {
        unreachable!()
    };
    tokio::time::timeout(
        Duration::from_secs(2),
        client.resize_terminal_surface(
            to_rpc::<rpc::ResizeTerminalSurfaceRequest>(&request).unwrap(),
        ),
    )
    .await
    .expect("paused terminal resize deadline")
    .unwrap();
    assert_eq!(
        *gateway.resizes.lock(),
        vec![(first.session_key.clone(), 30, 120)]
    );
    assert!(!paused.is_finished());

    // When / Then: only valid processed units release the source below the low watermark.
    let request = |units| rpc::ReportTerminalProcessedRequest {
        subscription_id: "input-/first".into(),
        units,
        ..Default::default()
    };
    assert_eq!(
        client
            .report_terminal_processed(request(1))
            .await
            .unwrap_err()
            .code,
        connectrpc::ErrorCode::InvalidArgument
    );
    for _ in 0..19 {
        client
            .report_terminal_processed(request(5000))
            .await
            .unwrap();
    }
    assert!(!paused.is_finished());
    client
        .report_terminal_processed(request(5000))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), paused)
        .await
        .expect("source resume deadline")
        .unwrap();

    // When / Then: resume the terminal's output number through the same wire contract.
    client
        .stop_state_subscription(rpc::StopStateSubscriptionRequest {
            subscription_id: "input-/first".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    let resume = wire::StartStateSubscriptionRequest {
        client_id: "terminal-client".into(),
        target: "terminal".into(),
        args: vec!["/first".into()],
        version: initial_version,
        subscription_id: "resumed-input".into(),
    };
    client
        .start_state_subscription(to_rpc::<rpc::StartStateSubscriptionRequest>(&resume).unwrap())
        .await
        .unwrap();
    let resumed = next_event!();
    assert_eq!(resumed.version.unwrap().sequence, 1);
    assert!(matches!(resumed.event, Some(Event::Change(change)) if change.delta));
    let mut bookmark = None;
    for _ in 0..3 {
        let event = next_event!();
        if event.subscription_id == "resumed-input" {
            bookmark = Some(event);
            break;
        }
    }
    let bookmark = bookmark.expect("resumed terminal bookmark");
    assert_eq!(bookmark.version.unwrap().sequence, 1);
    assert!(matches!(bookmark.event, Some(Event::Bookmark(_))));
    drop(stream);
    server.abort();
}

#[tokio::test]
pub async fn test_notion購読_正規化した対象を共有し識別子ごとに値と失敗を届ける() {
    use releash_lib::test_support::integration::subscriptions::{
        StateReadError, StateSubscriptionRead, StateValue,
    };

    use releash_lib::test_support::integration::subscriptions::StateSubscriptionOutput;

    struct Reads(AtomicUsize);
    #[async_trait::async_trait]
    impl StateSubscriptionRead for Reads {
        async fn read(&self, _: &SubscriptionTarget) -> Result<StateValue, StateReadError> {
            Ok(StateValue::NotionTasks(
                releash_lib::test_support::integration::platform::Fetched {
                    value: Some(
                        releash_lib::test_support::integration::platform::NotionTaskPage {
                            tasks: vec![],
                            has_more: true,
                            next_cursor: None,
                        },
                    ),
                    error: None,
                },
            ))
        }
        async fn refresh_external(&self, _: &SubscriptionTarget) -> Result<(), StateReadError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        fn repositories(&self) -> Vec<String> {
            vec![]
        }
    }
    // Given
    let reads = Arc::new(Reads(AtomicUsize::new(0)));
    let subscriptions =
        releash_lib::test_support::integration::subscriptions::StateSubscriptionUsecase::new(
            vec![],
            releash_lib::test_support::integration::subscriptions::read_driver(),
        )
        .with_reads(reads.clone(), None, vec![], String::new());
    let presenter = Arc::new(subscriptions.test_presenter().unwrap().clone());
    let deps =
        releash_lib::test_support::integration::transport::client_api_deps(Arc::new(dispatch()))
            .with_state_subscriptions(releash_lib::test_support::integration::subscriptions::deps(
                subscriptions.clone(),
                presenter.clone(),
            ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let config = ClientConfig::new(
        format!("http://{}", listener.local_addr().unwrap())
            .parse()
            .unwrap(),
    );
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            router(
                Some(deps),
                releash_lib::test_support::integration::daemon::default_timeout(),
            ),
        )
        .await
        .unwrap();
    });
    let client = rpc::ClientServiceClient::new(HttpClient::plaintext(), config);
    let mut stream = client
        .open_state_stream(rpc::OpenStateStreamRequest {
            client_id: "notion-client".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    macro_rules! next_event {
        () => {{
            let message = tokio::time::timeout(
                std::time::Duration::from_secs(2),
                stream.message::<rpc::StateSubscriptionEvent>(),
            )
            .await
            .unwrap()
            .unwrap()
            .unwrap()
            .to_owned_message();
            let event: wire::StateSubscriptionEvent = to_wire(&message).unwrap();
            event
        }};
    }
    assert!(matches!(next_event!().event, Some(Event::Ready(_))));
    let a = vec![
        "/repo".to_string(),
        "20".into(),
        r#"labels={"Tags":["z","a","a"],"Status":["Todo"]}"#.into(),
    ];
    let b = vec![
        "/repo".to_string(),
        "20".into(),
        r#"labels={"Status":["Todo"],"Tags":["a","z"]}"#.into(),
    ];
    let target = SubscriptionTarget::from_parts(
        "notion-tasks",
        &a.iter().map(String::as_str).collect::<Vec<_>>(),
    )
    .unwrap();
    // When / Then
    let invalid = client
        .start_state_subscription(rpc::StartStateSubscriptionRequest {
            client_id: "notion-client".into(),
            target: "notion-tasks".into(),
            args: vec!["/repo".into(), "20".into(), "labels=invalid".into()],
            subscription_id: "invalid".into(),
            ..Default::default()
        })
        .await
        .unwrap_err();
    assert_eq!(invalid.code, connectrpc::ErrorCode::InvalidArgument);
    assert_eq!(reads.0.load(Ordering::SeqCst), 0);
    for (index, args) in [&a, &b].into_iter().enumerate() {
        client
            .start_state_subscription(rpc::StartStateSubscriptionRequest {
                client_id: "notion-client".into(),
                target: "notion-tasks".into(),
                args: args.clone(),
                subscription_id: format!("notion-{index}"),
                ..Default::default()
            })
            .await
            .unwrap();
        let mut received = std::collections::HashSet::new();
        while received.is_empty() {
            let event = next_event!();
            if let Some(Event::Snapshot(payload)) = event.event {
                assert_eq!(event.subscription_id, format!("notion-{index}"));
                let Some(Value::NotionTasks(snapshot)) = payload.value else {
                    panic!("Notion tasks expected")
                };
                assert_eq!(snapshot.page.unwrap().has_more, Some(true));
                received.insert(event.subscription_id);
            }
        }
        assert!(received.contains(&format!("notion-{index}")));
    }
    assert_eq!(reads.0.load(Ordering::SeqCst), 1);
    assert_eq!(subscriptions.test_worker_count(), 1);
    presenter
        .publish(
            &target,
            StateValue::NotionTasks(releash_lib::test_support::integration::platform::Fetched {
                value: Some(releash_lib::test_support::integration::platform::NotionTaskPage {
                    tasks: vec![],
                    has_more: true,
                    next_cursor: None,
                }),
                error: Some(releash_lib::test_support::integration::platform::NotionUsecaseError::ConfigNotFound),
            }),
            None,
        )
        .unwrap();
    let mut failures = std::collections::HashSet::new();
    while failures.len() < 2 {
        let event = next_event!();
        if let Some(Event::Change(change)) = event.event {
            let Some(Value::NotionTasks(snapshot)) = change.payload.unwrap().value else {
                panic!("Notion failure expected")
            };
            assert!(snapshot.page.is_some());
            assert_eq!(snapshot.read_error.unwrap().config_missing, Some(true));
            failures.insert(event.subscription_id);
        }
    }
    assert_eq!(
        failures,
        std::collections::HashSet::from(["notion-0".into(), "notion-1".into()])
    );
    for index in 0..2 {
        client
            .stop_state_subscription(rpc::StopStateSubscriptionRequest {
                subscription_id: format!("notion-{index}"),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(subscriptions.test_worker_count(), 1 - index);
    }
    drop(stream);
    server.abort();
}

#[tokio::test]
pub async fn test_connect受付_停止後の新規streamを拒否し既存streamと重複停止を維持する() {
    // Given
    let daemon =
        releash_lib::test_support::integration::daemon::DaemonUsecase::test_with_repository(
            releash_lib::test_support::integration::daemon::serving(),
        );
    let (sender, mut exit) = tokio::sync::mpsc::channel(1);
    let directory = tempfile::tempdir().unwrap();
    let mut dependencies =
        releash_lib::test_support::integration::transport::build_client_dependencies(
            directory.path().into(),
        );
    dependencies.daemon = daemon.clone();
    dependencies.process_port = sender;
    let mut dispatch = ClientCommandDispatch::new(daemon.clone());
    dispatch.register_dependencies(&dependencies);
    let subscriptions =
        releash_lib::test_support::integration::subscriptions::StateSubscriptionUsecase::new(
            vec![],
            releash_lib::test_support::integration::subscriptions::read_driver(),
        );
    let deps =
        releash_lib::test_support::integration::transport::client_api_deps(Arc::new(dispatch))
            .with_state_subscriptions(subscriptions.deps());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let config = ClientConfig::new(
        format!("http://{}", listener.local_addr().unwrap())
            .parse()
            .unwrap(),
    );
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            router(
                Some(deps),
                releash_lib::test_support::integration::daemon::default_timeout(),
            ),
        )
        .await
        .unwrap();
    });
    let client = rpc::ClientServiceClient::new(HttpClient::plaintext(), config);
    let mut stream = client
        .open_state_stream(rpc::OpenStateStreamRequest {
            client_id: "existing".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    let ready = stream
        .message::<rpc::StateSubscriptionEvent>()
        .await
        .unwrap()
        .unwrap()
        .to_owned_message();
    assert!(matches!(
        to_wire::<wire::StateSubscriptionEvent>(&ready)
            .unwrap()
            .event,
        Some(wire::state_subscription_event::Event::Ready(_))
    ));
    client
        .start_state_subscription(rpc::StartStateSubscriptionRequest {
            client_id: "existing".into(),
            subscription_id: "existing-paths".into(),
            target: "repository-paths".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    for expected in ["snapshot", "bookmark"] {
        let event = stream
            .message::<rpc::StateSubscriptionEvent>()
            .await
            .unwrap()
            .unwrap()
            .to_owned_message();
        let event = to_wire::<wire::StateSubscriptionEvent>(&event).unwrap();
        assert!(matches!(
            (expected, event.event),
            (
                "snapshot",
                Some(wire::state_subscription_event::Event::Snapshot(_))
            ) | (
                "bookmark",
                Some(wire::state_subscription_event::Event::Bookmark(_))
            )
        ));
    }
    let quit = |code| {
        let wire::command_request::Command::RequestApplicationQuit(request) =
            wire::command_request_from_value(
                "request_application_quit",
                serde_json::json!({"request": {"intent": {"type": "restart", "code": code}}}),
            )
            .unwrap()
            .command
            .unwrap()
        else {
            unreachable!()
        };
        to_rpc::<rpc::RequestApplicationQuitRequest>(&request).unwrap()
    };
    // When
    client.request_application_quit(quit(23)).await.unwrap();
    assert_eq!(exit.try_recv().unwrap(), 23);
    exit.close();
    // Then
    client.request_application_quit(quit(99)).await.unwrap();
    assert_eq!(exit.len(), 0);
    let info = client
        .get_server_info(rpc::Unit::default())
        .await
        .unwrap()
        .into_owned();
    assert_eq!(info.serving_status, rpc::ServingStatus::Stopping);
    subscriptions.test_set_repository_paths(vec!["/after-stop".into()]);
    subscriptions.notify(
        releash_lib::test_support::integration::subscriptions::StateChangeSource::Repositories,
    );
    let event = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        stream.message::<rpc::StateSubscriptionEvent>(),
    )
    .await
    .unwrap()
    .unwrap()
    .unwrap()
    .to_owned_message();
    let event = to_wire::<wire::StateSubscriptionEvent>(&event).unwrap();
    assert_eq!(event.subscription_id, "existing-paths");
    let Some(wire::state_subscription_event::Event::Change(change)) = event.event else {
        panic!("post-stop change expected");
    };
    assert_eq!(
        change.payload.unwrap().value,
        Some(wire::state_payload::Value::RepositoryPaths(
            wire::Liststring {
                items: vec!["/after-stop".into()]
            }
        ))
    );
    let mut rejected = client
        .open_state_stream(rpc::OpenStateStreamRequest {
            client_id: "new".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    let error = rejected
        .message::<rpc::StateSubscriptionEvent>()
        .await
        .unwrap_err();
    assert_eq!(error.code, connectrpc::ErrorCode::FailedPrecondition);
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD_NO_PAD
        .decode(error.details[0].value.as_ref().unwrap())
        .unwrap();
    let detail = wire::CommandError::decode(bytes.as_slice()).unwrap();
    assert!(
        matches!(detail.variant, Some(wire::command_error::Variant::Coded(value)) if value.code.as_deref() == Some("APPLICATION_UNAVAILABLE"))
    );
    drop(stream);
    server.abort();
}
