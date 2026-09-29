use super::*;

use crate::usecase::application_startup::ApplicationStartupAuthority;
use connectrpc::client::{ClientConfig, HttpClient};
use prost::Message;
use std::sync::atomic::{AtomicUsize, Ordering};

fn dispatch() -> ClientCommandDispatch {
    ClientCommandDispatch::new(Arc::new(ApplicationStartupAuthority::ready()))
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
    let router = router(Some(crate::test_support::client_api_deps(
        Arc::new(dispatch),
        None,
    )))
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
async fn test_connect_生成clientのunaryで結果と構造化エラーを返す() {
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
async fn test_応答未到達_副作用は完了するが照会と再送は行わず現在状態を取得する() {
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
async fn test_connect_変更前と同じ16mibまで要求を受理する() {
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

#[test]
fn test_拒否_構造化エラーで段と理由を返す() {
    // Given
    let rejection = crate::common::concurrency::Rejection {
        level: "default",
        reason: crate::common::concurrency::RejectReason::TimedOut,
    };
    // When
    let error = crate::adaptor::presenter::connect::request_rejected(&rejection);
    // Then
    assert_eq!(error.code, connectrpc::ErrorCode::ResourceExhausted);
    assert_eq!(error.details.len(), 1);
    assert_eq!(error.details[0].type_url, "releash.client.v1.CommandError");
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD_NO_PAD
        .decode(error.details[0].value.as_ref().unwrap())
        .unwrap();
    let detail = wire::CommandError::decode(bytes.as_slice()).unwrap();
    let Some(wire::command_error::Variant::Coded(detail)) = detail.variant else {
        panic!("coded error");
    };
    assert_eq!(detail.code.as_deref(), Some("CLIENT_REQUEST_LIMIT"));
    assert_eq!(
        detail.message.as_deref(),
        Some("default requests rejected: time-out")
    );
    assert_eq!(
        error.message.as_deref(),
        Some("default requests rejected: time-out")
    );
}

#[tokio::test]
async fn test_サーバ情報取得_全段の枠が埋まっていても受理し枠を使わない() {
    use axum::{
        body::{to_bytes, Body},
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;
    // Given
    let deps = crate::test_support::client_api_deps(Arc::new(dispatch()), None);
    let _permits =
        ["interactive", "workflow", "default"].map(|level| deps.priority.gate.limits().fill(level));
    let router = router(Some(deps.clone()));
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
        assert_eq!(deps.priority.gate.limits().available(level), 0);
    }
}

#[tokio::test]
async fn test_状態購読_購読idを入口で128バイトまで受け付ける() {
    // Given
    let subscriptions = crate::usecase::state_subscription::StateSubscriptionUsecase::new(
        vec![],
        Arc::new(crate::adaptor::gateway::subscription_timer::TokioSubscriptionTimer),
    );
    let deps = crate::test_support::client_api_deps(Arc::new(dispatch()), None)
        .with_state_subscriptions(StateSubscriptionDeps::new(
            subscriptions.clone(),
            Arc::new(subscriptions.test_presenter().unwrap().clone()),
        ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let config = ClientConfig::new(
        format!("http://{}", listener.local_addr().unwrap())
            .parse()
            .unwrap(),
    );
    let server = tokio::spawn(async move {
        axum::serve(listener, router(Some(deps))).await.unwrap();
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
async fn test_状態購読_connectで初期状態と変更と再開を配信する() {
    use crate::usecase::state_subscription::{StateSubscriptionUsecase, SubscriptionTarget};
    use wire::state_subscription_event::Event;
    // Given
    let subscriptions = StateSubscriptionUsecase::new(
        vec!["/repo".into()],
        Arc::new(crate::adaptor::gateway::subscription_timer::TokioSubscriptionTimer),
    );
    let deps = crate::test_support::client_api_deps(Arc::new(dispatch()), None)
        .with_state_subscriptions(StateSubscriptionDeps::new(
            subscriptions.clone(),
            Arc::new(subscriptions.test_presenter().unwrap().clone()),
        ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let config = ClientConfig::new(
        format!("http://{}", listener.local_addr().unwrap())
            .parse()
            .unwrap(),
    );
    let server = tokio::spawn(async move {
        axum::serve(listener, router(Some(deps))).await.unwrap();
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
        terminal_input_id: None,
    };
    client
        .start_state_subscription(to_rpc::<rpc::StartStateSubscriptionRequest>(&request).unwrap())
        .await
        .unwrap();
    client
        .start_state_subscription(to_rpc::<rpc::StartStateSubscriptionRequest>(&request).unwrap())
        .await
        .unwrap();
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
            client_id: "state-test".into(),
            target: SubscriptionTarget::RepositoryPaths.to_string(),
            ..Default::default()
        })
        .await
        .unwrap();
    let error = client
        .start_state_subscription(rpc::StartStateSubscriptionRequest {
            client_id: "state-test".into(),
            target: "missing".into(),
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
    let deps = crate::test_support::client_api_deps(Arc::new(dispatch), None);
    let mut request = Request::post("/releash.client.v1.ClientService/UpdateExternalEditor")
        .header("content-type", "application/json")
        .header("connect-protocol-version", "1");
    if let Some(timeout) = timeout {
        request = request.header("connect-timeout-ms", timeout);
    }
    let call = router(Some(deps.clone())).oneshot(request.body(Body::from("{}")).unwrap());
    tokio::pin!(call);
    assert!(futures_util::poll!(&mut call).is_pending());
    tokio::task::yield_now().await;
    assert_eq!(deps.priority.gate.limits().available("default"), 40);
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
    assert_eq!(deps.priority.gate.limits().available("default"), 41);
}

#[tokio::test(start_paused = true)]
async fn test_単発rpc_期限なしは120秒で処理を止め枠を解放する() {
    assert_request_deadline(None, 120).await;
}

#[tokio::test(start_paused = true)]
async fn test_単発rpc_clientの短い期限で処理を止め枠を解放する() {
    assert_request_deadline(Some("1000"), 1).await;
}

#[tokio::test(start_paused = true)]
async fn test_単発rpc_clientの長い期限を短縮しない() {
    assert_request_deadline(Some("180000"), 180).await;
}

#[tokio::test]
async fn test_単発rpc_呼び出し破棄でasync処理を止め枠を解放する() {
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
    let deps = crate::test_support::client_api_deps(Arc::new(dispatch), None);
    let mut call = Box::pin(router(Some(deps.clone())).oneshot(unary_request(
        "UpdateExternalEditor",
        r#"{"editor":"code"}"#,
    )));
    assert!(futures_util::poll!(&mut call).is_pending());
    tokio::task::yield_now().await;
    assert_eq!(deps.priority.gate.limits().available("default"), 40);
    // When
    drop(call);
    // Then
    assert_eq!(deps.priority.gate.limits().available("default"), 41);
    tokio::time::timeout(std::time::Duration::from_secs(1), stopped.cancelled())
        .await
        .unwrap();
}

#[tokio::test]
async fn test_単発rpc_取り消しはcancelledでpanicはinternalに分類する() {
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
async fn test_単発rpc_client切断で処理が終了する() {
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
    let deps = crate::test_support::client_api_deps(Arc::new(dispatch), None);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let router = router(Some(deps.clone()));
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
    assert_eq!(deps.priority.gate.limits().available("default"), 41);
}

#[tokio::test]
async fn test_単発rpc_変更処理の取り消しでhandlerとworktreeの枠を解放する() {
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
async fn test_状態購読_既定期限後もbookmarkが届く() {
    use axum::{body::Body, http::Request};
    use futures_util::StreamExt;
    use tower::ServiceExt;
    // Given
    let subscriptions = crate::usecase::state_subscription::StateSubscriptionUsecase::new(
        Vec::new(),
        Arc::new(crate::adaptor::gateway::subscription_timer::TokioSubscriptionTimer),
    );
    let deps = crate::test_support::client_api_deps(Arc::new(dispatch()), None)
        .with_state_subscriptions(StateSubscriptionDeps::new(
            subscriptions.clone(),
            Arc::new(subscriptions.test_presenter().unwrap().clone()),
        ));
    let payload = br#"{"clientId":"deadline-test"}"#;
    let mut bytes = vec![0];
    bytes.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    bytes.extend_from_slice(payload);
    let response = router(Some(deps))
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
    crate::test_support::state_subscription::start_read(
        &subscriptions,
        "deadline-test",
        "repository-paths",
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
async fn test_状態購読操作_上限時は拒否し枠解放後は受理する() {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;
    // Given
    let subscriptions = crate::usecase::state_subscription::StateSubscriptionUsecase::new(
        Vec::new(),
        Arc::new(crate::adaptor::gateway::subscription_timer::TokioSubscriptionTimer),
    );
    let _stream = subscriptions.open("limited".into()).unwrap();
    let deps = crate::test_support::client_api_deps(Arc::new(dispatch()), None)
        .with_state_subscriptions(StateSubscriptionDeps::new(
            subscriptions.clone(),
            Arc::new(subscriptions.test_presenter().unwrap().clone()),
        ));
    let router = router(Some(deps.clone()));
    for method in ["StartStateSubscription", "StopStateSubscription"] {
        let permits = deps.priority.gate.limits().fill("interactive");
        let request = || {
            Request::post(format!("/releash.client.v1.ClientService/{method}"))
                .header("content-type", "application/json")
                .header("connect-protocol-version", "1")
                .body(Body::from(
                    r#"{"clientId":"limited","target":"repository-paths"}"#,
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
        assert_eq!(deps.priority.gate.limits().available("interactive"), 11);
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
    let deps = crate::test_support::client_api_deps(Arc::new(dispatch), None);
    let request = Request::post("/releash.client.v1.ClientService/GitStage")
        .header("content-type", "application/json")
        .header("connect-protocol-version", "1")
        .header("connect-timeout-ms", "1000")
        .body(Body::from(r#"{"repoPath":"/repo","paths":[]}"#))
        .unwrap();
    let mut call = Box::pin(router(Some(deps.clone())).oneshot(request));
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
    assert_eq!(deps.priority.gate.limits().available("default"), 41);
    assert!(futures_util::poll!(&mut deletion).is_pending());
    assert!(runtime.begin_worktree_mutation("/repo").is_err());
    finish.send(()).unwrap();
    let guard = deletion.await.unwrap();
    assert!(runtime.begin_worktree_mutation("/repo").is_err());
    drop(guard);
    assert!(runtime.begin_worktree_mutation("/repo").is_ok());
}

#[tokio::test]
async fn test_変更rpc_中断後も同期処理の完了まで削除と変更を拒否する() {
    for repository in [false, true] {
        assert_cancelled_blocking_mutation(false, repository).await;
    }
}

#[tokio::test]
async fn test_変更rpc_期限切れ後も同期処理の完了まで削除と変更を拒否する() {
    for repository in [false, true] {
        assert_cancelled_blocking_mutation(true, repository).await;
    }
}

#[tokio::test]
async fn test_単発rpc_期限と呼出破棄が同期処理の内側まで届く() {
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
        let deps = crate::test_support::client_api_deps(Arc::new(dispatch), None);
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
async fn test_terminal購読_connectの後段配線と差分再開と流量停止中の応答を保証する() {
    use crate::adaptor::presenter::terminal_event_hub::TerminalSurfaceEventHub;
    use crate::domain::terminal_surface::entities::TerminalSurface;
    use crate::usecase::terminal_surface::output::TerminalSurfaceEventSink;
    use crate::usecase::terminal_surface::output::TerminalSurfaceOutputControl;
    use crate::usecase::terminal_surface::output::TerminalSurfaceOutputEvent;

    use crate::domain::terminal_surface::TerminalSurfaceOwner;
    use crate::domain::workspace_tree::WorkspaceIdentity;
    use crate::usecase::state_subscription::StateSubscriptionUsecase;
    use crate::usecase::terminal_surface::application::TerminalSurfaceApplication;
    use crate::usecase::terminal_surface::io_usecase::io_usecase_tests::FakePtyGateway;
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
    use tauri::Manager;
    app.manage(Arc::new(
        crate::infrastructure::file_watcher::FileWatcherManager::default(),
    ));
    let mut dependencies = crate::desktop_test_support::build_client_dependencies(app.handle());
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
        Arc::new(crate::adaptor::gateway::subscription_timer::TokioSubscriptionTimer),
    );
    subscriptions
        .test_presenter()
        .unwrap()
        .connect_terminal(&terminal)
        .unwrap();
    let subscriptions = subscriptions.with_terminal(terminal);
    let deps = crate::test_support::client_api_deps(Arc::new(dispatch), None)
        .with_state_subscriptions(StateSubscriptionDeps::new(
            subscriptions.clone(),
            Arc::new(subscriptions.test_presenter().unwrap().clone()),
        ));
    assert_eq!(*gateway.list_summaries_calls.lock(), 0);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let config = ClientConfig::new(
        format!("http://{}", listener.local_addr().unwrap())
            .parse()
            .unwrap(),
    );
    let server = tokio::spawn(async move {
        axum::serve(listener, router(Some(deps))).await.unwrap();
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
                terminal_input_id: (path == "/first").then(|| format!("input-{path}")).into(),
                ..Default::default()
            })
            .await
            .unwrap();
        let initial = next_event!();
        assert_eq!(initial.target, "terminal");
        assert_eq!(initial.args, vec![path]);
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
            ..Default::default()
        })
        .await
        .unwrap();
    let repository = next_event!();
    assert_eq!(repository.target, "repository-paths");
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
    assert_eq!(other.args, vec!["/second"]);
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
    let request = wire::CommandRequest::from_value("resize_terminal_surface", serde_json::json!({"owner": {"kind": "workspace", "workspacePath": "/first"}, "rows": 30, "cols": 120})).unwrap();
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
        client_id: "terminal-client".into(),
        args: vec!["/first".into()],
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
            client_id: "terminal-client".into(),
            target: "terminal".into(),
            args: vec!["/first".into()],
            ..Default::default()
        })
        .await
        .unwrap();
    let resume = wire::StartStateSubscriptionRequest {
        client_id: "terminal-client".into(),
        target: "terminal".into(),
        args: vec!["/first".into()],
        version: initial_version,
        terminal_input_id: Some("resumed-input".into()),
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
        if event.target == "terminal" && event.args.len() == 1 && event.args[0] == "/first" {
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
async fn test_共通入口_期限切れを変換し成功と内部失敗を保持する() {
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
#[test]
fn test_状態購読配線_usecaseとcontrollerが同じ出力実装を参照する() {
    // Given
    let presenter =
        Arc::new(crate::adaptor::presenter::state_subscription::StateSubscriptionPresenter::new());
    let usecase = crate::usecase::state_subscription::StateSubscriptionUsecase::new_with_output(
        presenter.clone(),
        Arc::new(crate::adaptor::gateway::subscription_timer::TokioSubscriptionTimer),
    );
    let deps = StateSubscriptionDeps::new(usecase, presenter);
    // When
    let output: Arc<dyn crate::usecase::state_subscription::StateSubscriptionOutput> =
        deps.presenter.clone();
    // Then
    assert!(Arc::ptr_eq(&deps.usecase.publisher(), &output));
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
async fn test_流量制御_全段の枠が埋まっていてもReportTerminalProcessedを受理する() {
    use axum::http::StatusCode;
    use tower::ServiceExt;
    // Given
    let subscriptions = crate::usecase::state_subscription::StateSubscriptionUsecase::new(
        Vec::new(),
        Arc::new(crate::adaptor::gateway::subscription_timer::TokioSubscriptionTimer),
    );
    let _stream = subscriptions.open("limited".into()).unwrap();
    let presenter = subscriptions.test_presenter().unwrap().clone();
    let units = presenter.terminal_report_units();
    let deps =
        crate::test_support::client_api_deps(Arc::new(dispatch()), None).with_state_subscriptions(
            StateSubscriptionDeps::new(subscriptions, Arc::new(presenter)),
        );
    let _permits =
        ["interactive", "workflow", "default"].map(|level| deps.priority.gate.limits().fill(level));
    // When
    let response = router(Some(deps))
        .oneshot(unary_request(
            "ReportTerminalProcessed",
            &format!(r#"{{"clientId":"limited","args":["session"],"units":{units}}}"#),
        ))
        .await
        .unwrap();
    // Then
    assert_ne!(response.status(), StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn test_優先度_defaultが埋まっていてもinteractiveの呼び出しを受理する() {
    use axum::http::StatusCode;
    use tower::ServiceExt;
    // Given
    let subscriptions = crate::usecase::state_subscription::StateSubscriptionUsecase::new(
        Vec::new(),
        Arc::new(crate::adaptor::gateway::subscription_timer::TokioSubscriptionTimer),
    );
    let _stream = subscriptions.open("limited".into()).unwrap();
    let deps = crate::test_support::client_api_deps(Arc::new(dispatch()), None)
        .with_state_subscriptions(StateSubscriptionDeps::new(
            subscriptions.clone(),
            Arc::new(subscriptions.test_presenter().unwrap().clone()),
        ));
    let _permits = deps.priority.gate.limits().fill("default");
    let router = router(Some(deps.clone()));
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
            r#"{"clientId":"limited","target":"repository-paths"}"#,
        ))
        .await
        .unwrap();
    // Then
    assert_eq!(rejected.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(accepted.status(), StatusCode::OK);
    assert_eq!(deps.priority.gate.limits().available("interactive"), 11);
}

#[tokio::test]
async fn test_拒否_待ち行列が溢れた拒否を記録し次の受理で解く() {
    use axum::http::StatusCode;
    use tower::ServiceExt;
    // Given
    let store = Arc::new(crate::adaptor::gateway::failure_records::FailureRecordStore::default());
    let deps = crate::test_support::client_api_deps(
        Arc::new(dispatch()),
        Some(Arc::new(
            crate::usecase::failure::FailureRecordingUsecase::new(store.clone(), None),
        )),
    );
    let permits = deps.priority.gate.limits().fill("default");
    // When
    let router = router(Some(deps));
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
    let records = store.records("daemon");
    assert_eq!(records.len(), 1);
    let record = &records[0].record;
    assert_eq!(record.operation, "client_request_limit");
    assert_eq!(
        record.kind,
        crate::usecase::failure::Failure::Technical(
            crate::domain::failure::TechnicalFailureNature::Transient
        )
    );
    assert_eq!(
        record.message,
        "/releash.client.v1.ClientService/UpdateExternalEditor: default requests rejected: queue_full"
    );
    assert!(!records[0].requires_attention);
    assert!(record.active);
    drop(permits);
    let accepted = router
        .oneshot(unary_request(
            "UpdateExternalEditor",
            r#"{"editor":"code"}"#,
        ))
        .await
        .unwrap();
    assert_ne!(accepted.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(!store.records("daemon")[0].record.active);
}

#[tokio::test]
async fn test_待ち行列_席が空くまで待ってから受理する() {
    use axum::http::StatusCode;
    use tower::ServiceExt;
    // Given
    let (dispatch, release) = pending_editor_dispatch();
    let deps = crate::test_support::client_api_deps(Arc::new(dispatch), None);
    let seats = deps
        .priority
        .gate
        .limits()
        .seats("default")
        .try_acquire_many_owned(41)
        .unwrap();
    let mut call = Box::pin(router(Some(deps.clone())).oneshot(unary_request(
        "UpdateExternalEditor",
        r#"{"editor":"code"}"#,
    )));
    assert!(futures_util::poll!(&mut call).is_pending());
    tokio::task::yield_now().await;
    assert_eq!(deps.priority.gate.limits().queue_length("default"), 49);
    // When
    drop(seats);
    release.notify_one();
    let response = call.await.unwrap();
    // Then
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(deps.priority.gate.limits().queue_length("default"), 50);
    assert_eq!(deps.priority.gate.limits().available("default"), 41);
}
