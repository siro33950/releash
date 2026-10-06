use super::*;

use connectrpc::client::{ClientConfig, HttpClient};
use prost::Message;
use std::sync::atomic::{AtomicUsize, Ordering};

fn dispatch() -> ClientCommandDispatch {
    ClientCommandDispatch::new(crate::usecase::daemon::DaemonUsecase::test_with_repository(
        crate::adaptor::gateway::daemon::serving(),
    ))
}

fn unary_request(method: &str, body: &str) -> axum::http::Request<axum::body::Body> {
    axum::http::Request::post(format!("/releash.client.v1.ClientService/{method}"))
        .header("content-type", "application/json")
        .header("connect-protocol-version", "1")
        .body(axum::body::Body::from(body.to_string()))
        .unwrap()
}

async fn serve(
    dispatch: ClientCommandDispatch,
) -> (
    rpc::ClientServiceClient<HttpClient>,
    tokio::task::JoinHandle<()>,
) {
    let router = router(
        Some(crate::test_support::client_api_deps(Arc::new(dispatch))),
        crate::adaptor::controller::daemon::default_timeout(),
    )
    .layer(axum::middleware::from_fn_with_state(
        crate::infrastructure::local_api::ClientBearerToken::from(Arc::<str>::from("client")),
        super::super::auth::require_client,
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
                crate::adaptor::controller::client::required(args.entries, "entries")?;
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
pub async fn test_サーバ情報取得_全段の枠が埋まっていても受理し枠を使わない() {
    use axum::{
        body::{to_bytes, Body},
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;
    // Given
    let deps = crate::test_support::client_api_deps(Arc::new(dispatch()));
    let _permits =
        ["interactive", "workflow", "default"].map(|level| deps.priority_limits().fill(level));
    let router = router(
        Some(deps.clone()),
        crate::adaptor::controller::daemon::default_timeout(),
    );
    let request = || {
        Request::post("/releash.client.v1.ClientService/GetServerInfo")
            .header("content-type", "application/json")
            .header("connect-protocol-version", "1")
            .body(Body::from("{}"))
            .unwrap()
    };
    // When
    let response = router.clone().oneshot(request()).await.unwrap();
    // Then
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["release"], env!("CARGO_PKG_VERSION"));
    for level in ["interactive", "workflow", "default"] {
        assert_eq!(deps.priority_limits().available(level), 0);
    }
}

#[tokio::test]
pub async fn test_状態購読stream_全段の枠が埋まっていてもイベントを受け取り席を使わない() {
    // Given
    let subscriptions = crate::usecase::state_subscription::StateSubscriptionUsecase::new(
        vec![],
        crate::test_support::state_subscription::read_driver(),
    );
    let deps = crate::test_support::client_api_deps(Arc::new(dispatch()))
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
                    crate::adaptor::controller::daemon::default_timeout(),
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
    let subscriptions = crate::usecase::state_subscription::StateSubscriptionUsecase::new(
        vec![],
        crate::test_support::state_subscription::read_driver(),
    );
    let deps = crate::test_support::client_api_deps(Arc::new(dispatch()))
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
                crate::adaptor::controller::daemon::default_timeout(),
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
    use crate::usecase::state_subscription::{StateSubscriptionUsecase, SubscriptionTarget};
    use wire::state_subscription_event::Event;
    // Given
    let subscriptions = StateSubscriptionUsecase::new(
        vec!["/repo".into()],
        crate::test_support::state_subscription::read_driver(),
    );
    let deps = crate::test_support::client_api_deps(Arc::new(dispatch()))
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
                crate::adaptor::controller::daemon::default_timeout(),
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
    subscriptions.notify(crate::usecase::state_subscription::StateChangeSource::Repositories);
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

async fn assert_request_deadline(timeout: Option<&str>, seconds: u64) {
    use axum::{
        body::{to_bytes, Body},
        http::Request,
    };
    use tower::ServiceExt;
    // Given
    let stopped = tokio_util::sync::CancellationToken::new();
    let signal = stopped.clone();
    let mut dispatch = dispatch();
    dispatch.register_domain(
        &["update_external_editor"],
        Box::new(move |_| {
            let guard = signal.clone().drop_guard();
            Box::pin(async move {
                let _guard = guard;
                std::future::pending().await
            })
        }),
    );
    let deps = crate::test_support::client_api_deps(Arc::new(dispatch));
    let mut request = Request::post("/releash.client.v1.ClientService/UpdateExternalEditor")
        .header("content-type", "application/json")
        .header("connect-protocol-version", "1");
    if let Some(timeout) = timeout {
        request = request.header("connect-timeout-ms", timeout);
    }
    let call = router(
        Some(deps.clone()),
        crate::adaptor::controller::daemon::default_timeout(),
    )
    .oneshot(request.body(Body::from("{}")).unwrap());
    tokio::pin!(call);
    assert!(futures_util::poll!(&mut call).is_pending());
    tokio::task::yield_now().await;
    assert_eq!(deps.priority_limits().available("default"), 40);
    // When
    tokio::time::advance(std::time::Duration::from_secs(seconds - 1)).await;
    assert!(futures_util::poll!(&mut call).is_pending());
    tokio::time::advance(std::time::Duration::from_secs(1)).await;
    let response = call.await.unwrap();
    // Then
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let error: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(error["code"], "deadline_exceeded");
    stopped.cancelled().await;
    assert_eq!(deps.priority_limits().available("default"), 41);
}

#[tokio::test(start_paused = true)]
pub async fn test_単発rpc_期限なしは120秒で処理を止め枠を解放する() {
    assert_request_deadline(None, 120).await;
}

#[tokio::test(start_paused = true)]
pub async fn test_単発rpc_clientの短い期限で処理を止め枠を解放する() {
    assert_request_deadline(Some("1000"), 1).await;
}

#[tokio::test(start_paused = true)]
pub async fn test_単発rpc_clientの長い期限を短縮しない() {
    assert_request_deadline(Some("180000"), 180).await;
}

#[tokio::test]
pub async fn test_単発rpc_呼び出し破棄でasync処理を止め枠を解放する() {
    use tower::ServiceExt;
    // Given
    let stopped = tokio_util::sync::CancellationToken::new();
    let signal = stopped.clone();
    let mut dispatch = dispatch();
    dispatch.register_domain(
        &["update_external_editor"],
        Box::new(move |_| {
            let guard = signal.clone().drop_guard();
            Box::pin(async move {
                let _guard = guard;
                std::future::pending().await
            })
        }),
    );
    let deps = crate::test_support::client_api_deps(Arc::new(dispatch));
    let mut call = Box::pin(
        router(
            Some(deps.clone()),
            crate::adaptor::controller::daemon::default_timeout(),
        )
        .oneshot(unary_request(
            "UpdateExternalEditor",
            r#"{"editor":"code"}"#,
        )),
    );
    assert!(futures_util::poll!(&mut call).is_pending());
    tokio::task::yield_now().await;
    assert_eq!(deps.priority_limits().available("default"), 40);
    // When
    drop(call);
    // Then
    assert_eq!(deps.priority_limits().available("default"), 41);
    tokio::time::timeout(std::time::Duration::from_secs(1), stopped.cancelled())
        .await
        .unwrap();
}

#[tokio::test]
pub async fn test_単発rpc_取り消しはcancelledでpanicはinternalに分類する() {
    // Given
    let task = tokio::spawn(std::future::pending::<()>());
    // When
    task.abort();
    let error = task_error(task.await.unwrap_err());
    // Then
    assert_eq!(error.code, connectrpc::ErrorCode::Canceled);
    let task = tokio::spawn(async { panic!("test panic") });
    assert_eq!(
        task_error(task.await.unwrap_err()).code,
        connectrpc::ErrorCode::Internal
    );
    let token = tokio_util::sync::CancellationToken::new();
    token.cancel();
    let mut dispatch = dispatch();
    dispatch.register_domain(
        &["update_external_editor"],
        Box::new(|_| Box::pin(std::future::pending())),
    );
    let error = run_command(
        &token,
        dispatch.dispatch_admitted(wire::command_request::Command::UpdateExternalEditor(
            Default::default(),
        )),
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, connectrpc::ErrorCode::Canceled);
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
    let deps = crate::test_support::client_api_deps(Arc::new(dispatch));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let router = router(
        Some(deps.clone()),
        crate::adaptor::controller::daemon::default_timeout(),
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
pub async fn test_単発rpc_変更処理の取り消しでhandlerとworktreeの枠を解放する() {
    use crate::usecase::repository_usecase::WorktreeExecutionArchiver;
    // Given
    let runtime = Arc::new(crate::usecase::workflow::WorkflowRuntimeUsecase::new(
        Arc::new(super::super::test_support::RecordingRuntimeGateway::default()),
        Arc::new(crate::usecase::workflow::NoopArchiveRepository),
    ));
    let started = Arc::new(tokio::sync::Notify::new());
    let start_signal = started.clone();
    let stopped = tokio_util::sync::CancellationToken::new();
    let signal = stopped.clone();
    let mut dispatch = dispatch().with_worktree_mutations(runtime.clone());
    dispatch.register_domain(
        &["git_stage"],
        Box::new(move |_| {
            start_signal.notify_one();
            let guard = signal.clone().drop_guard();
            Box::pin(async move {
                let _guard = guard;
                std::future::pending().await
            })
        }),
    );
    let cancellation = tokio_util::sync::CancellationToken::new();
    let mut call = Box::pin(run_command(
        &cancellation,
        dispatch.dispatch_admitted(wire::command_request::Command::GitStage(
            wire::GitStageRequest {
                repo_path: Some("/repo".into()),
                ..Default::default()
            },
        )),
    ));
    tokio::select! {
        _ = started.notified() => {},
        result = &mut call => panic!("handler ended before cancellation: {result:?}"),
    }
    let deletion = runtime.begin_worktree_deletion("/repo");
    tokio::pin!(deletion);
    assert!(futures_util::poll!(&mut deletion).is_pending());
    // When
    cancellation.cancel();
    assert_eq!(
        call.await.unwrap_err().code,
        connectrpc::ErrorCode::Canceled
    );
    // Then
    assert!(stopped.is_cancelled());
    tokio::time::timeout(std::time::Duration::from_secs(1), deletion)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test(start_paused = true)]
pub async fn test_状態購読_既定期限後もbookmarkが届く() {
    use axum::{body::Body, http::Request};
    use futures_util::StreamExt;
    use tower::ServiceExt;
    // Given
    let subscriptions = crate::usecase::state_subscription::StateSubscriptionUsecase::new(
        Vec::new(),
        crate::test_support::state_subscription::read_driver(),
    );
    let deps = crate::test_support::client_api_deps(Arc::new(dispatch()))
        .with_state_subscriptions(subscriptions.deps());
    let payload = br#"{"clientId":"deadline-test"}"#;
    let mut bytes = vec![0];
    bytes.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    bytes.extend_from_slice(payload);
    let response = router(
        Some(deps),
        crate::adaptor::controller::daemon::default_timeout(),
    )
    .oneshot(
        Request::post("/releash.client.v1.ClientService/OpenStateStream")
            .header("content-type", "application/connect+json")
            .header("connect-protocol-version", "1")
            .body(Body::from(bytes))
            .unwrap(),
    )
    .await
    .unwrap();
    assert!(response.status().is_success());
    let mut body = response.into_body().into_data_stream();
    assert!(body.next().await.unwrap().is_ok());
    subscriptions
        .deps()
        .start_subscription(
            "deadline-test",
            &crate::usecase::state_subscription::SubscriptionTarget::parse("repository-paths")
                .unwrap(),
            &format!("{}:{}", "deadline-test", "repository-paths"),
            None,
        )
        .await
        .unwrap();
    assert!(body.next().await.unwrap().is_ok());
    // When / Then
    for _ in 0..13 {
        tokio::time::advance(std::time::Duration::from_secs(10)).await;
        let frame = body.next().await.unwrap().unwrap();
        assert_eq!(frame[0], 0);
        assert!(std::str::from_utf8(&frame[5..])
            .unwrap()
            .contains("bookmark"));
    }
}

#[tokio::test]
pub async fn test_状態購読操作_上限時は拒否し枠解放後は受理する() {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;
    // Given
    let subscriptions = crate::usecase::state_subscription::StateSubscriptionUsecase::new(
        Vec::new(),
        crate::test_support::state_subscription::read_driver(),
    );
    let _stream = subscriptions.open("limited".into()).unwrap();
    let deps = crate::test_support::client_api_deps(Arc::new(dispatch()))
        .with_state_subscriptions(subscriptions.deps());
    let router = router(
        Some(deps.clone()),
        crate::adaptor::controller::daemon::default_timeout(),
    );
    for method in ["StartStateSubscription", "StopStateSubscription"] {
        let permits = deps.priority_limits().fill("interactive");
        let request = || {
            Request::post(format!("/releash.client.v1.ClientService/{method}"))
                .header("content-type", "application/json")
                .header("connect-protocol-version", "1")
                .body(Body::from(
                    r#"{"clientId":"limited","target":"repository-paths","subscriptionId":"limited-subscription"}"#,
                ))
                .unwrap()
        };
        // When / Then
        assert_eq!(
            router.clone().oneshot(request()).await.unwrap().status(),
            StatusCode::TOO_MANY_REQUESTS
        );
        drop(permits);
        assert_eq!(
            router.clone().oneshot(request()).await.unwrap().status(),
            StatusCode::OK
        );
        assert_eq!(deps.priority_limits().available("interactive"), 11);
    }
}

async fn assert_cancelled_blocking_mutation(deadline: bool, repository: bool) {
    use crate::usecase::repository_usecase::WorktreeExecutionArchiver;
    use axum::{
        body::{to_bytes, Body},
        http::Request,
    };
    use tower::ServiceExt;
    // Given
    let runtime = Arc::new(crate::usecase::workflow::WorkflowRuntimeUsecase::new(
        Arc::new(super::super::test_support::RecordingRuntimeGateway::default()),
        Arc::new(crate::usecase::workflow::NoopArchiveRepository),
    ));
    let started = Arc::new(tokio::sync::Notify::new());
    let signal = started.clone();
    let (finish, receiver) = std::sync::mpsc::channel();
    let receiver = Arc::new(std::sync::Mutex::new(receiver));
    let stopped = tokio_util::sync::CancellationToken::new();
    let stop_signal = stopped.clone();
    let mut dispatch = dispatch().with_worktree_mutations(runtime.clone());
    dispatch.register_domain(
        &["git_stage"],
        Box::new(move |_| {
            let signal = signal.clone();
            let receiver = receiver.clone();
            let guard = stop_signal.clone().drop_guard();
            Box::pin(async move {
                let _guard = guard;
                let operation = move || {
                    signal.notify_one();
                    receiver.lock().unwrap().recv().unwrap();
                };
                let result = if repository {
                    crate::adaptor::controller::client::repository::run_blocking(move || {
                        operation();
                        Ok(())
                    })
                    .await
                } else {
                    crate::adaptor::controller::client::code::run_blocking(move || {
                        operation();
                        Ok(())
                    })
                    .await
                };
                result.map_err(wire::CommandFailure::from)?;
                Ok(wire::command_result::Command::GitStage(wire::Unit {}))
            })
        }),
    );
    let deps = crate::test_support::client_api_deps(Arc::new(dispatch));
    let request = Request::post("/releash.client.v1.ClientService/GitStage")
        .header("content-type", "application/json")
        .header("connect-protocol-version", "1")
        .header("connect-timeout-ms", "1000")
        .body(Body::from(r#"{"repoPath":"/repo","paths":[]}"#))
        .unwrap();
    let mut call = Box::pin(
        router(
            Some(deps.clone()),
            crate::adaptor::controller::daemon::default_timeout(),
        )
        .oneshot(request),
    );
    assert!(futures_util::poll!(&mut call).is_pending());
    started.notified().await;
    let mut deletion = Box::pin(runtime.begin_worktree_deletion("/repo"));
    assert!(futures_util::poll!(&mut deletion).is_pending());
    // When
    if deadline {
        let response = call.await.unwrap();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let error: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(error["code"], "deadline_exceeded");
    } else {
        drop(call);
    }
    stopped.cancelled().await;
    // Then
    assert_eq!(deps.priority_limits().available("default"), 41);
    assert!(futures_util::poll!(&mut deletion).is_pending());
    assert!(runtime.begin_worktree_mutation("/repo").is_err());
    finish.send(()).unwrap();
    let guard = deletion.await.unwrap();
    assert!(runtime.begin_worktree_mutation("/repo").is_err());
    drop(guard);
    assert!(runtime.begin_worktree_mutation("/repo").is_ok());
}

#[tokio::test]
pub async fn test_変更rpc_中断後も同期処理の完了まで削除と変更を拒否する() {
    for repository in [false, true] {
        assert_cancelled_blocking_mutation(false, repository).await;
    }
}

#[tokio::test]
pub async fn test_変更rpc_期限切れ後も同期処理の完了まで削除と変更を拒否する() {
    for repository in [false, true] {
        assert_cancelled_blocking_mutation(true, repository).await;
    }
}

#[tokio::test]
pub async fn test_単発rpc_期限と呼出破棄が同期処理の内側まで届く() {
    use crate::common::operation_context::OperationStopped;
    use std::time::{Duration, Instant};
    for expire in [false, true] {
        // Given
        let (started, mut ready) = tokio::sync::mpsc::unbounded_channel();
        let (stopped, mut stopped_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut dispatch = dispatch();
        dispatch.register_domain(
            &["update_external_editor"],
            Box::new(move |_| {
                let started = started.clone();
                let stopped = stopped.clone();
                Box::pin(async move {
                    crate::common::operation_context::spawn_blocking(move || {
                        started.send(()).unwrap();
                        let error = crate::common::operation_context::sleep(
                            &crate::common::operation_context::current(),
                            Duration::from_secs(30),
                        )
                        .unwrap_err();
                        stopped.send(error).unwrap();
                        Err(crate::adaptor::presenter::error::AppError::from_failure(
                            crate::domain::failure::TechnicalFailure::from(error),
                        )
                        .into())
                    })
                    .await
                    .unwrap()
                })
            }),
        );
        let deps = crate::test_support::client_api_deps(Arc::new(dispatch));
        let mut call = Box::pin(deps.execute(
            expire.then(|| Instant::now() + Duration::from_millis(100)),
            wire::command_request::Command::UpdateExternalEditor(Default::default()),
        ));
        // When
        tokio::select! { _ = ready.recv() => {}, result = &mut call => panic!("call ended before starting: {result:?}") }
        if expire {
            assert_eq!(
                call.await.unwrap_err().code,
                connectrpc::ErrorCode::DeadlineExceeded
            );
        } else {
            drop(call);
        }
        // Then
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), stopped_rx.recv())
                .await
                .unwrap()
                .unwrap(),
            if expire {
                OperationStopped::Expired
            } else {
                OperationStopped::Cancelled
            }
        );
    }
}

#[tokio::test]
pub async fn test_terminal購読_connectの後段配線と差分再開と流量停止中の応答を保証する() {
    use crate::adaptor::presenter::terminal_event_hub::TerminalSurfaceEventHub;
    use crate::domain::terminal_surface::entities::TerminalSurface;
    use crate::usecase::terminal_surface::output::TerminalSurfaceEventSink;
    use crate::usecase::terminal_surface::output::TerminalSurfaceOutputControl;
    use crate::usecase::terminal_surface::output::TerminalSurfaceOutputEvent;

    use crate::domain::terminal_surface::TerminalSurfaceOwner;
    use crate::domain::workspace_tree::WorkspaceIdentity;
    use crate::usecase::state_subscription::StateSubscriptionUsecase;
    use crate::usecase::terminal_surface::application::TerminalSurfaceApplication;
    use crate::usecase::terminal_surface::test_helpers_io::FakePtyGateway;
    use std::time::Duration;
    use wire::{state_payload::Value, state_subscription_event::Event, terminal_event::Item};

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
    hub.initialize(crate::test_support::state_subscription::registration(
        &first.session_key,
        "/first",
        None,
        1,
        0,
    ))
    .unwrap();
    hub.initialize(crate::test_support::state_subscription::registration(
        &second.session_key,
        "/second",
        None,
        2,
        0,
    ))
    .unwrap();
    let terminal = Arc::new(TerminalSurfaceApplication::new(
        std::sync::Arc::new(crate::adaptor::gateway::telemetry::TelemetryGateway),
        gateway.clone(),
        Arc::new(crate::adaptor::gateway::terminal_surface::event_source::TerminalSurfaceEventSourceGateway::new(hub.event_sender())),
        hub.clone(),
    ));
    let (app, _, _) =
        crate::adaptor::controller::client::workflow::tests::make_read_only_app_with_terminal(
            terminal.clone(),
        );
    let mut dependencies = app.client;
    dependencies.workflow_runtime_usecase = Some(Arc::new(
        crate::usecase::workflow::WorkflowRuntimeUsecase::new(
            Arc::new(super::super::test_support::RecordingRuntimeGateway::default()),
            Arc::new(crate::usecase::workflow::NoopArchiveRepository),
        ),
    ));
    let mut dispatch = dispatch();
    dispatch.register_dependencies(&dependencies);
    let subscriptions = StateSubscriptionUsecase::new(
        vec!["/repo".into()],
        crate::test_support::state_subscription::read_driver(),
    );
    let subscriptions = subscriptions.with_terminal(terminal);
    let deps = crate::test_support::client_api_deps(Arc::new(dispatch))
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
                crate::adaptor::controller::daemon::default_timeout(),
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

async fn run_command(
    cancellation: &tokio_util::sync::CancellationToken,
    future: impl std::future::Future<
        Output = Result<wire::command_result::Command, wire::CommandFailure>,
    >,
) -> Result<wire::command_result::Command, connectrpc::ConnectError> {
    let context = crate::common::operation_context::OperationContext::new(
        None,
        Arc::new(cancellation.clone()),
    );
    crate::common::operation_context::wait(&context, future)
        .await
        .map_err(|error| {
            crate::adaptor::presenter::connect::classified_error(
                crate::adaptor::presenter::error::AppError::from_failure(
                    crate::domain::failure::TechnicalFailure::from(error),
                ),
            )
        })?
        .map_err(command_error)
}

#[tokio::test]
pub async fn test_共通入口_期限切れを変換し成功と内部失敗を保持する() {
    // Given / When
    let expired = ingress(Some(std::time::Instant::now()), async {
        Ok::<(), connectrpc::ConnectError>(())
    })
    .await
    .unwrap_err();
    // Then
    assert_eq!(expired.code, connectrpc::ErrorCode::DeadlineExceeded);
    assert_eq!(
        ingress(None, async { Ok::<_, connectrpc::ConnectError>(42) })
            .await
            .unwrap(),
        42
    );
    let error = ingress(None, async {
        Err::<(), _>(crate::adaptor::presenter::connect::classified_error(
            crate::adaptor::presenter::error::AppError::new("unavailable")
                .with_status(connectrpc::ErrorCode::Unavailable),
        ))
    })
    .await
    .unwrap_err();
    assert_eq!(error.code, connectrpc::ErrorCode::Unavailable);
}

fn pending_editor_dispatch() -> (ClientCommandDispatch, Arc<tokio::sync::Notify>) {
    let release = Arc::new(tokio::sync::Notify::new());
    let signal = release.clone();
    let mut dispatch = dispatch();
    dispatch.register_domain(
        &["update_external_editor"],
        Box::new(move |_| {
            let signal = signal.clone();
            Box::pin(async move {
                signal.notified().await;
                Ok(wire::command_result::Command::UpdateExternalEditor(
                    wire::Unit {},
                ))
            })
        }),
    );
    (dispatch, release)
}

#[tokio::test]
pub async fn test_流量制御_全段の枠が埋まっていても_report_terminal_processedを受理する() {
    use axum::http::StatusCode;
    use tower::ServiceExt;
    // Given
    let subscriptions = crate::usecase::state_subscription::StateSubscriptionUsecase::new(
        Vec::new(),
        crate::test_support::state_subscription::read_driver(),
    );
    let _stream = subscriptions.open("limited".into()).unwrap();
    let presenter = subscriptions.test_presenter().unwrap().clone();
    let units = crate::adaptor::presenter::terminal_subscription::TerminalSubscriptionPresenter::report_units();
    let deps = crate::test_support::client_api_deps(Arc::new(dispatch())).with_state_subscriptions(
        crate::test_support::state_subscription::deps(subscriptions, Arc::new(presenter)),
    );
    let _permits =
        ["interactive", "workflow", "default"].map(|level| deps.priority_limits().fill(level));
    // When
    let response = router(
        Some(deps),
        crate::adaptor::controller::daemon::default_timeout(),
    )
    .oneshot(unary_request(
        "ReportTerminalProcessed",
        &format!(r#"{{"subscriptionId":"session","units":{units}}}"#),
    ))
    .await
    .unwrap();
    // Then
    assert_ne!(response.status(), StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
pub async fn test_優先度_defaultが埋まっていてもinteractiveの呼び出しを受理する() {
    use axum::http::StatusCode;
    use tower::ServiceExt;
    // Given
    let subscriptions = crate::usecase::state_subscription::StateSubscriptionUsecase::new(
        Vec::new(),
        crate::test_support::state_subscription::read_driver(),
    );
    let _stream = subscriptions.open("limited".into()).unwrap();
    let deps = crate::test_support::client_api_deps(Arc::new(dispatch()))
        .with_state_subscriptions(subscriptions.deps());
    let _permits = deps.priority_limits().fill("default");
    let router = router(
        Some(deps.clone()),
        crate::adaptor::controller::daemon::default_timeout(),
    );
    // When
    let rejected = router
        .clone()
        .oneshot(unary_request(
            "UpdateExternalEditor",
            r#"{"editor":"code"}"#,
        ))
        .await
        .unwrap();
    let accepted = router
        .oneshot(unary_request(
            "StartStateSubscription",
            r#"{"clientId":"limited","target":"repository-paths","subscriptionId":"limited-subscription"}"#,
        ))
        .await
        .unwrap();
    // Then
    assert_eq!(rejected.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(accepted.status(), StatusCode::OK);
    assert_eq!(deps.priority_limits().available("interactive"), 11);
}

#[tokio::test]
pub async fn test_拒否_待ち行列が溢れても読まれない失敗は記録しない() {
    use axum::http::StatusCode;
    use tower::ServiceExt;
    // Given
    let deps = crate::test_support::client_api_deps(Arc::new(dispatch()));
    let permits = deps.priority_limits().fill("default");
    // When
    let router = router(
        Some(deps),
        crate::adaptor::controller::daemon::default_timeout(),
    );
    let response = router
        .clone()
        .oneshot(unary_request(
            "UpdateExternalEditor",
            r#"{"editor":"code"}"#,
        ))
        .await
        .unwrap();
    // Then
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    drop(permits);
    let accepted = router
        .oneshot(unary_request(
            "UpdateExternalEditor",
            r#"{"editor":"code"}"#,
        ))
        .await
        .unwrap();
    assert_ne!(accepted.status(), StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
pub async fn test_拒否_枠の対象外の呼び出しでも読まれない失敗は記録しない() {
    use axum::http::StatusCode;
    use tower::ServiceExt;
    // Given
    let deps = crate::test_support::client_api_deps(Arc::new(dispatch()));
    let permits = deps.priority_limits().fill("default");
    let router = router(
        Some(deps),
        crate::adaptor::controller::daemon::default_timeout(),
    );
    assert_eq!(
        router
            .clone()
            .oneshot(unary_request(
                "UpdateExternalEditor",
                r#"{"editor":"code"}"#,
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::TOO_MANY_REQUESTS
    );

    // When / Then
    for method in ["GetServerInfo", "ReportTerminalProcessed"] {
        let response = router
            .clone()
            .oneshot(unary_request(method, "{}"))
            .await
            .unwrap();
        assert_ne!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    }
    drop(permits);
    let response = router
        .oneshot(unary_request(
            "UpdateExternalEditor",
            r#"{"editor":"code"}"#,
        ))
        .await
        .unwrap();
    assert_ne!(response.status(), StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
pub async fn test_待ち行列_席が空くまで待ってから受理する() {
    use axum::http::StatusCode;
    use tower::ServiceExt;
    // Given
    let (dispatch, release) = pending_editor_dispatch();
    let deps = crate::test_support::client_api_deps(Arc::new(dispatch));
    let seats = deps
        .priority_limits()
        .seats("default")
        .try_acquire_many_owned(41)
        .unwrap();
    let mut call = Box::pin(
        router(
            Some(deps.clone()),
            crate::adaptor::controller::daemon::default_timeout(),
        )
        .oneshot(unary_request(
            "UpdateExternalEditor",
            r#"{"editor":"code"}"#,
        )),
    );
    assert!(futures_util::poll!(&mut call).is_pending());
    tokio::task::yield_now().await;
    assert_eq!(deps.priority_limits().queue_length("default"), 49);
    // When
    drop(seats);
    release.notify_one();
    let response = call.await.unwrap();
    // Then
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(deps.priority_limits().queue_length("default"), 50);
    assert_eq!(deps.priority_limits().available("default"), 41);
}

#[tokio::test]
pub async fn test_notion購読_正規化した対象を共有し識別子ごとに値と失敗を届ける() {
    use crate::usecase::state_subscription::{
        StateReadError, StateSubscriptionOutput, StateSubscriptionRead, StateValue,
        SubscriptionTarget,
    };
    use wire::{state_payload::Value, state_subscription_event::Event};
    struct Reads(AtomicUsize);
    #[async_trait::async_trait]
    impl StateSubscriptionRead for Reads {
        async fn read(&self, _: &SubscriptionTarget) -> Result<StateValue, StateReadError> {
            Ok(StateValue::NotionTasks(crate::usecase::fetched::Fetched {
                value: Some(crate::domain::notion::NotionTaskPage {
                    tasks: vec![],
                    has_more: true,
                    next_cursor: None,
                }),
                error: None,
            }))
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
    let subscriptions = crate::usecase::state_subscription::StateSubscriptionUsecase::new(
        vec![],
        crate::test_support::state_subscription::read_driver(),
    )
    .with_reads(reads.clone(), None, vec![], String::new());
    let presenter = Arc::new(subscriptions.test_presenter().unwrap().clone());
    let deps = crate::test_support::client_api_deps(Arc::new(dispatch())).with_state_subscriptions(
        crate::test_support::state_subscription::deps(subscriptions.clone(), presenter.clone()),
    );
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
                crate::adaptor::controller::daemon::default_timeout(),
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
            StateValue::NotionTasks(crate::usecase::fetched::Fetched {
                value: Some(crate::domain::notion::NotionTaskPage {
                    tasks: vec![],
                    has_more: true,
                    next_cursor: None,
                }),
                error: Some(crate::usecase::notion::error::NotionUsecaseError::ConfigNotFound),
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

const NOTION_REQUEST_A: &str = r#"{"subscriptionId":"notion-a","clientId":"client","target":"notion-tasks","args":["/repo","20","labels={\"Tags\":[\"a\"]}"]}"#;
const NOTION_REQUEST_B: &str = r#"{"subscriptionId":"notion-b","clientId":"client","target":"notion-tasks","args":["/repo","20","labels={\"Tags\":[\"a\",\"a\"]}"]}"#;

fn notion_cancellation_fixture() -> (Router, StateSubscriptionDeps) {
    use crate::usecase::state_subscription::{
        StateReadError, StateSubscriptionRead, StateValue, SubscriptionTarget,
    };
    struct Reads;
    #[async_trait::async_trait]
    impl StateSubscriptionRead for Reads {
        async fn read(&self, target: &SubscriptionTarget) -> Result<StateValue, StateReadError> {
            match target {
                SubscriptionTarget::NotionTasks(_) => {
                    Ok(StateValue::NotionTasks(crate::usecase::fetched::Fetched {
                        value: Some(crate::domain::notion::NotionTaskPage {
                            tasks: vec![],
                            has_more: false,
                            next_cursor: None,
                        }),
                        error: None,
                    }))
                }
                _ => std::future::pending().await,
            }
        }
        fn repositories(&self) -> Vec<String> {
            vec![]
        }
    }
    let subscriptions = crate::usecase::state_subscription::StateSubscriptionUsecase::new(
        vec![],
        crate::test_support::state_subscription::read_driver(),
    )
    .with_reads(Arc::new(Reads), None, vec![], String::new())
    .deps();
    let app = router(
        Some(
            crate::test_support::client_api_deps(Arc::new(dispatch()))
                .with_state_subscriptions(subscriptions.clone()),
        ),
        crate::adaptor::controller::daemon::default_timeout(),
    );
    (app, subscriptions)
}

#[tokio::test]
pub async fn test_notion購読_初回開始の中断で新規要求を残さない() {
    use crate::usecase::state_subscription::SubscriptionTarget;
    use tower::ServiceExt;
    // Given
    let (app, subscriptions) = notion_cancellation_fixture();
    let _stream = subscriptions.open_stream("client".into()).unwrap();
    let blocked = SubscriptionTarget::BranchBase("/blocked".into(), "main".into());
    let mut blocker =
        Box::pin(subscriptions.start_subscription("client", &blocked, "blocked", None));
    assert!(futures_util::poll!(&mut blocker).is_pending());
    let mut call = Box::pin(app.oneshot(unary_request("StartStateSubscription", NOTION_REQUEST_A)));
    // When
    assert!(futures_util::poll!(&mut call).is_pending());
    drop(call);
    drop(blocker);
    let registration = subscriptions
        .test_presenter()
        .delivery("notion-a")
        .map(|(client, target, _)| (client, target));
    // Then
    assert!(registration.is_none());
    assert_eq!(subscriptions.test_usecase().test_worker_count(), 0);
}

#[tokio::test(start_paused = true)]
pub async fn test_notion購読_別名開始の期限切れ後に既存要求を停止するとworkerを解放する() {
    use crate::usecase::state_subscription::SubscriptionTarget;
    use tower::ServiceExt;
    // Given
    let (app, subscriptions) = notion_cancellation_fixture();
    let _stream = subscriptions.open_stream("client".into()).unwrap();
    app.clone()
        .oneshot(unary_request("StartStateSubscription", NOTION_REQUEST_A))
        .await
        .unwrap();
    let blocked = SubscriptionTarget::BranchBase("/blocked".into(), "main".into());
    let mut blocker =
        Box::pin(subscriptions.start_subscription("client", &blocked, "blocked", None));
    assert!(futures_util::poll!(&mut blocker).is_pending());
    let mut request = unary_request("StartStateSubscription", NOTION_REQUEST_B);
    request
        .headers_mut()
        .insert("connect-timeout-ms", "1000".parse().unwrap());
    let mut call = Box::pin(app.clone().oneshot(request));
    // When
    assert!(futures_util::poll!(&mut call).is_pending());
    tokio::time::advance(std::time::Duration::from_secs(1)).await;
    let expired = call.await.unwrap();
    let body = axum::body::to_bytes(expired.into_body(), usize::MAX)
        .await
        .unwrap();
    let error: serde_json::Value = serde_json::from_slice(&body).unwrap();
    drop(blocker);
    let stopped = app
        .oneshot(unary_request(
            "StopStateSubscription",
            r#"{"subscriptionId":"notion-a"}"#,
        ))
        .await
        .unwrap();
    // Then
    assert_eq!(error["code"], "deadline_exceeded");
    assert!(stopped.status().is_success());
    assert_eq!(subscriptions.test_usecase().test_worker_count(), 0);
}

#[tokio::test]
pub async fn test_notion購読_重複開始の中断で既存要求を消さない() {
    use crate::usecase::state_subscription::SubscriptionTarget;
    use tower::ServiceExt;
    // Given
    let (app, subscriptions) = notion_cancellation_fixture();
    let _stream = subscriptions.open_stream("client".into()).unwrap();
    app.clone()
        .oneshot(unary_request("StartStateSubscription", NOTION_REQUEST_A))
        .await
        .unwrap();
    app.clone()
        .oneshot(unary_request("StartStateSubscription", NOTION_REQUEST_B))
        .await
        .unwrap();
    let blocked = SubscriptionTarget::BranchBase("/blocked".into(), "main".into());
    let mut blocker =
        Box::pin(subscriptions.start_subscription("client", &blocked, "blocked", None));
    assert!(futures_util::poll!(&mut blocker).is_pending());
    let call = Box::pin(
        app.clone()
            .oneshot(unary_request("StartStateSubscription", NOTION_REQUEST_A)),
    );
    // When
    let response = call.await.unwrap();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let error: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(error["code"], "already_exists");
    drop(blocker);
    let stopped = app
        .oneshot(unary_request(
            "StopStateSubscription",
            r#"{"subscriptionId":"notion-b"}"#,
        ))
        .await
        .unwrap();
    // Then
    assert!(stopped.status().is_success());
    assert_eq!(subscriptions.test_usecase().test_worker_count(), 1);
}

#[tokio::test]
pub async fn test_notion購読_最後の停止の中断で要求と購読を保持する() {
    use crate::usecase::state_subscription::SubscriptionTarget;
    use tower::ServiceExt;
    // Given
    let (app, subscriptions) = notion_cancellation_fixture();
    let _stream = subscriptions.open_stream("client".into()).unwrap();
    app.clone()
        .oneshot(unary_request("StartStateSubscription", NOTION_REQUEST_A))
        .await
        .unwrap();
    let blocked = SubscriptionTarget::BranchBase("/blocked".into(), "main".into());
    let mut blocker =
        Box::pin(subscriptions.start_subscription("client", &blocked, "blocked", None));
    assert!(futures_util::poll!(&mut blocker).is_pending());
    let target = SubscriptionTarget::from_parts(
        "notion-tasks",
        &["/repo", "20", r#"labels={"Tags":["a"]}"#],
    )
    .unwrap();
    let mut call = Box::pin(app.clone().oneshot(unary_request(
        "StopStateSubscription",
        r#"{"subscriptionId":"notion-a"}"#,
    )));
    // When
    assert!(futures_util::poll!(&mut call).is_pending());
    drop(call);
    let registration = subscriptions
        .test_presenter()
        .delivery("notion-a")
        .map(|(client, target, _)| (client, target));
    let workers = subscriptions.test_usecase().test_worker_count();
    drop(blocker);
    let stopped = app
        .oneshot(unary_request(
            "StopStateSubscription",
            r#"{"subscriptionId":"notion-a"}"#,
        ))
        .await
        .unwrap();
    // Then
    assert_eq!(registration, Some(("client".into(), target.to_string())));
    assert_eq!(workers, 1);
    assert!(stopped.status().is_success());
    assert_eq!(subscriptions.test_usecase().test_worker_count(), 0);
}

#[tokio::test]
pub async fn test_購読識別子_入口で形と全clientの重複を検査し未知の停止は成功する() {
    use crate::usecase::state_subscription::SubscriptionTarget;
    use tower::ServiceExt;
    // Given
    let (app, subscriptions) = notion_cancellation_fixture();
    let first = subscriptions.open_stream("client".into()).unwrap();
    let _second = subscriptions.open_stream("other".into()).unwrap();
    let request = |client: &str, id: &str| {
        serde_json::json!({ "clientId": client, "subscriptionId": id, "target": "notion-tasks", "args": ["/repo", "20"] }).to_string()
    };
    // When / Then
    for id in [String::new(), "x".repeat(129), "あ".repeat(43)] {
        let response = app
            .clone()
            .oneshot(unary_request(
                "StartStateSubscription",
                &request("client", &id),
            ))
            .await
            .unwrap();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let error: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(error["code"], "invalid_argument");
    }
    for id in ["x".repeat(128), "あ".repeat(42)] {
        assert!(app
            .clone()
            .oneshot(unary_request(
                "StartStateSubscription",
                &request("client", &id)
            ))
            .await
            .unwrap()
            .status()
            .is_success());
        for client in ["client", "other"] {
            let response = app
                .clone()
                .oneshot(unary_request(
                    "StartStateSubscription",
                    &request(client, &id),
                ))
                .await
                .unwrap();
            let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            let error: serde_json::Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(error["code"], "already_exists");
        }
        let stop = serde_json::json!({"subscriptionId": id}).to_string();
        for _ in 0..2 {
            assert!(app
                .clone()
                .oneshot(unary_request("StopStateSubscription", &stop))
                .await
                .unwrap()
                .status()
                .is_success());
        }
        assert!(app
            .clone()
            .oneshot(unary_request(
                "StartStateSubscription",
                &request("other", &id)
            ))
            .await
            .unwrap()
            .status()
            .is_success());
        subscriptions.stop_subscription(&id).await.unwrap();
    }
    let target = SubscriptionTarget::from_parts("notion-tasks", &["/repo", "20"]).unwrap();
    subscriptions
        .start_subscription("client", &target, "x", None)
        .await
        .unwrap();
    assert!(matches!(
        subscriptions
            .terminal_processed("x", 5000)
            .unwrap_err()
            .source,
        crate::usecase::state_subscription::StateReadFailure::TerminalSubscriptionEnded
    ));
    drop(first);
    subscriptions.stop_subscription("x").await.unwrap();
    subscriptions
        .stop_subscription("never-started")
        .await
        .unwrap();
    subscriptions
        .start_subscription("other", &target, "x", None)
        .await
        .unwrap();
}

#[tokio::test]
pub async fn test_購読停止_待機中に再利用された識別子の新しい登録を解除しない() {
    // Given
    use crate::usecase::state_subscription::SubscriptionTarget;
    use futures_util::{poll, StreamExt};
    let (_app, subscriptions) = notion_cancellation_fixture();
    let mut first = Box::pin(subscriptions.stream("first".into()).unwrap());
    let mut second = Box::pin(subscriptions.stream("second".into()).unwrap());
    first.next().await.unwrap();
    second.next().await.unwrap();
    let target = SubscriptionTarget::from_parts("notion-tasks", &["/repo", "20"]).unwrap();
    subscriptions
        .start_subscription("first", &target, "x", None)
        .await
        .unwrap();
    let blocked = SubscriptionTarget::BranchBase("/blocked".into(), "main".into());
    let mut blocker =
        Box::pin(subscriptions.start_subscription("second", &blocked, "blocked", None));
    assert!(poll!(&mut blocker).is_pending());
    // When
    let stop = subscriptions.stop_subscription("x");
    tokio::pin!(stop);
    assert!(poll!(&mut stop).is_pending());
    drop(first);
    assert!(subscriptions
        .test_presenter()
        .delivery("x")
        .map(|(client, target, _)| (client, target))
        .is_none());
    let delivery = subscriptions
        .test_presenter()
        .reserve_delivery("second", "x", &target.to_string(), None)
        .unwrap();
    drop(blocker);
    let (stopped, started) = tokio::join!(
        stop,
        subscriptions
            .test_usecase()
            .start_subscription("second", &target, &delivery),
    );
    stopped.unwrap();
    started.unwrap();
    // Then
    assert_eq!(
        subscriptions
            .test_presenter()
            .delivery("x")
            .map(|(client, target, _)| (client, target)),
        Some(("second".into(), target.to_string()))
    );
    assert!(subscriptions
        .test_usecase()
        .active_targets()
        .contains(&target));
}

#[tokio::test]
pub async fn test_購読停止_解放後の同じclientの再登録へ古い後始末が作用しない() {
    // Given
    use crate::usecase::state_subscription::{StateSubscriptionDelivery, SubscriptionTarget};
    let (_app, subscriptions) = notion_cancellation_fixture();
    let _stream = subscriptions.open_stream("client".into()).unwrap();
    let target = SubscriptionTarget::from_parts("notion-tasks", &["/repo", "20"]).unwrap();
    subscriptions
        .start_subscription("client", &target, "x", None)
        .await
        .unwrap();
    let (_, _, old) = subscriptions.test_presenter().delivery("x").unwrap();
    subscriptions.stop_subscription("x").await.unwrap();
    subscriptions
        .start_subscription("client", &target, "x", None)
        .await
        .unwrap();
    // When
    old.finish(&subscriptions.test_usecase().active_targets())
        .unwrap();
    subscriptions
        .test_usecase()
        .stop_subscription("client", &target, &old)
        .await
        .unwrap();
    // Then
    assert_eq!(
        subscriptions
            .test_presenter()
            .delivery("x")
            .map(|(client, target, _)| (client, target)),
        Some(("client".into(), target.to_string()))
    );
    assert!(subscriptions
        .test_usecase()
        .active_targets()
        .contains(&target));
    subscriptions.stop_subscription("x").await.unwrap();
    assert!(subscriptions.test_usecase().active_targets().is_empty());
}

#[tokio::test]
pub async fn test_terminal購読識別子_入口で空と超過を拒み上限と空白を受け付ける() {
    use tower::ServiceExt;
    // Given
    let (terminal, _, _, _) =
        crate::test_support::state_subscription::terminal_application_fixture();
    let subscriptions =
        crate::test_support::state_subscription::test_subscriptions().with_terminal(terminal);
    let deps = subscriptions.deps();
    let _stream = deps.stream("client".into()).unwrap();
    let app = router(
        Some(
            crate::test_support::client_api_deps(Arc::new(dispatch()))
                .with_state_subscriptions(deps),
        ),
        crate::adaptor::controller::daemon::default_timeout(),
    );
    // When / Then
    for id in [String::new(), "x".repeat(129), "あ".repeat(43)] {
        let body = serde_json::json!({"clientId":"client", "subscriptionId":id,"target":"terminal","args":["/repo"]}).to_string();
        let response = app
            .clone()
            .oneshot(unary_request("StartStateSubscription", &body))
            .await
            .unwrap();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let error: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(error["code"], "invalid_argument");
    }
    for id in ["x".repeat(128), "あ".repeat(42), " ".into()] {
        let body = serde_json::json!({"clientId":"client", "subscriptionId":id,"target":"terminal","args":["/repo"]}).to_string();
        let response = app
            .clone()
            .oneshot(unary_request("StartStateSubscription", &body))
            .await
            .unwrap();
        assert!(response.status().is_success());
    }
}

#[tokio::test]
pub async fn test_connect受付_停止後の新規streamを拒否し既存streamと重複停止を維持する() {
    // Given
    let daemon = crate::usecase::daemon::DaemonUsecase::test_with_repository(
        crate::adaptor::gateway::daemon::serving(),
    );
    let (sender, mut exit) = tokio::sync::mpsc::channel(1);
    let directory = tempfile::tempdir().unwrap();
    let mut dependencies =
        crate::acceptance_test_support::build_client_dependencies(directory.path().into());
    dependencies.daemon = daemon.clone();
    dependencies.process_port = sender;
    let mut dispatch = ClientCommandDispatch::new(daemon.clone());
    dispatch.register_dependencies(&dependencies);
    let subscriptions = crate::usecase::state_subscription::StateSubscriptionUsecase::new(
        vec![],
        crate::test_support::state_subscription::read_driver(),
    );
    let deps = crate::test_support::client_api_deps(Arc::new(dispatch))
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
                crate::adaptor::controller::daemon::default_timeout(),
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
    subscriptions.notify(crate::usecase::state_subscription::StateChangeSource::Repositories);
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
