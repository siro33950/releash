use super::*;
use crate::infrastructure::push::PushSink;
use crate::usecase::application_startup::ApplicationStartupAuthority;
use connectrpc::client::{ClientConfig, HttpClient};
use prost::Message;
use std::sync::atomic::{AtomicUsize, Ordering};

fn dispatch() -> ClientCommandDispatch {
    ClientCommandDispatch::new(
        Arc::new(crate::adaptor::controller::wiring::build_repository_usecase()),
        Arc::new(ApplicationStartupAuthority::ready()),
    )
}

async fn serve(
    dispatch: ClientCommandDispatch,
    sink: Arc<PushSink>,
) -> (
    rpc::ClientServiceClient<HttpClient>,
    tokio::task::JoinHandle<()>,
) {
    serve_with_watcher(dispatch, sink, crate::client_api_acceptance::watcher()).await
}

async fn serve_with_watcher(
    dispatch: ClientCommandDispatch,
    sink: Arc<PushSink>,
    watcher: Arc<crate::usecase::watcher::WatcherUsecase>,
) -> (
    rpc::ClientServiceClient<HttpClient>,
    tokio::task::JoinHandle<()>,
) {
    let router = router(Some(ClientApiDeps::new(
        Arc::new(dispatch),
        ClientPushGateway::new(sink),
        watcher,
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
        &["get_cwd"],
        Box::new(|_| {
            Box::pin(async {
                Ok(wire::command_result::Command::GetCwd(wire::ResultString {
                    value: Some("/repo".into()),
                }))
            })
        }),
    );
    let (client, server) = serve(dispatch, Arc::new(PushSink::new())).await;
    // When / Then
    assert_eq!(
        client
            .get_cwd(rpc::GetCwdRequest::default())
            .await
            .unwrap()
            .into_owned()
            .value,
        Some("/repo".into())
    );
    let error = client
        .get_current_branch(rpc::GetCurrentBranchRequest::default())
        .await
        .unwrap_err();
    assert_eq!(error.code, connectrpc::ErrorCode::InvalidArgument);
    assert_eq!(error.details[0].type_url, "releash.client.v1.CommandError");
    server.abort();
}

#[tokio::test]
async fn test_push_server_streamは再同期通知の後にbackend変更を配信する() {
    // Given
    let sink = Arc::new(PushSink::new());
    let (client, server) = serve(dispatch(), sink.clone()).await;
    let mut stream = client
        .subscribe_push(rpc::SubscribePushRequest::default())
        .await
        .unwrap();
    // When / Then
    let initial = stream
        .message::<rpc::Push>()
        .await
        .unwrap()
        .unwrap()
        .to_owned_message();
    assert!(matches!(
        to_wire::<wire::Push>(&initial).unwrap().event,
        Some(wire::push::Event::Resync(_))
    ));
    crate::adaptor::gateway::push::BackendPush::ReviewCommentsChanged("/next").emit(&sink);
    let push = stream
        .message::<rpc::Push>()
        .await
        .unwrap()
        .unwrap()
        .to_owned_message();
    let Some(wire::push::Event::ReviewCommentsChanged(value)) =
        to_wire::<wire::Push>(&push).unwrap().event
    else {
        panic!("repo paths push");
    };
    assert_eq!(value.value.as_deref(), Some("/next"));
    drop(stream);
    server.abort();
}

#[tokio::test]
async fn test_push配信_符号化済みpayloadを保持しlagged後も配信する() {
    use buffa::view::HasMessageView;
    use connectrpc::{CodecFormat, Encodable};
    use futures_util::StreamExt;
    use rpc::ClientService;
    // Given
    let sink = Arc::new(PushSink::new());
    let deps = ClientApiDeps::new(
        Arc::new(dispatch()),
        ClientPushGateway::new(sink.clone()),
        crate::client_api_acceptance::watcher(),
    );
    let body = axum::body::Bytes::new();
    let view = rpc::SubscribePushRequest::decode_view(&body).unwrap();
    let mut stream = deps
        .subscribe_push(
            connectrpc::RequestContext::default(),
            connectrpc::ServiceRequest::from_parts(&view, &body),
        )
        .await
        .unwrap()
        .body;
    let resync = wire::Push {
        event: Some(wire::push::Event::Resync(wire::Unit {})),
    }
    .encode_to_vec();
    assert_eq!(
        stream
            .next()
            .await
            .unwrap()
            .unwrap()
            .encode(CodecFormat::Proto)
            .unwrap(),
        resync
    );
    let event = wire::Push {
        event: Some(wire::push::Event::ReviewCommentsChanged(
            wire::ResultString {
                value: Some("/next".into()),
            },
        )),
    };
    let mut bytes = resync.clone();
    bytes.extend(event.encode_to_vec());
    // When / Then
    sink.send(bytes.clone());
    let push = stream.next().await.unwrap().unwrap();
    assert_eq!(push.encode(CodecFormat::Proto).unwrap(), bytes);
    let json: serde_json::Value =
        serde_json::from_slice(&push.encode(CodecFormat::Json).unwrap()).unwrap();
    assert_eq!(
        json,
        serde_json::json!({"reviewCommentsChanged": {"value": "/next"}})
    );
    for _ in 0..65 {
        sink.send(bytes.clone());
    }
    assert_eq!(
        stream
            .next()
            .await
            .unwrap()
            .unwrap()
            .encode(CodecFormat::Proto)
            .unwrap(),
        resync
    );
    sink.send(bytes.clone());
    assert_eq!(
        stream
            .next()
            .await
            .unwrap()
            .unwrap()
            .encode(CodecFormat::Proto)
            .unwrap(),
        bytes
    );
    sink.send(vec![0xff]);
    assert_eq!(
        stream
            .next()
            .await
            .unwrap()
            .unwrap()
            .encode(CodecFormat::Json)
            .unwrap_err()
            .code,
        connectrpc::ErrorCode::Internal
    );
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
        &["get_performance_telemetry_enabled"],
        Box::new(move |_| {
            let count = count.clone();
            Box::pin(async move {
                Ok(
                    wire::command_result::Command::GetPerformanceTelemetryEnabled(
                        wire::ResultBool {
                            value: Some(count.load(Ordering::SeqCst) != 0),
                        },
                    ),
                )
            })
        }),
    );
    let (client, server) = serve(dispatch, Arc::new(PushSink::new())).await;
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
            .get_performance_telemetry_enabled(rpc::GetPerformanceTelemetryEnabledRequest::default())
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
async fn test_監視unary_pushの購読が所有し明示停止と切断で解放する() {
    // Given
    let files = Arc::new(crate::usecase::watcher::watcher_tests::SubscriptionFiles::default());
    let watcher = Arc::new(crate::usecase::watcher::WatcherUsecase::new(
        None,
        files.clone(),
    ));
    let (client, server) = serve_with_watcher(dispatch(), Arc::new(PushSink::new()), watcher).await;
    let mut push = client
        .subscribe_push(rpc::SubscribePushRequest {
            subscription_id: "watchers".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    push.message::<rpc::Push>().await.unwrap().unwrap();
    let mut duplicate = client
        .subscribe_push(rpc::SubscribePushRequest {
            subscription_id: "watchers".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(
        duplicate.message::<rpc::Push>().await.unwrap_err().code,
        connectrpc::ErrorCode::AlreadyExists
    );
    // When
    let watched = client
        .watch_files(rpc::WatchFilesRequest {
            subscription_id: "watchers".into(),
            request: rpc::StartWatchingRequest {
                path: Some("/repo".into()),
                ..Default::default()
            }
            .into(),
            ..Default::default()
        })
        .await
        .unwrap()
        .into_owned();
    assert_eq!(files.active.lock().unwrap().len(), 1);
    for _ in 1..64 {
        client
            .watch_files(rpc::WatchFilesRequest {
                subscription_id: "watchers".into(),
                request: rpc::StartWatchingRequest {
                    path: Some("/repo".into()),
                    ..Default::default()
                }
                .into(),
                ..Default::default()
            })
            .await
            .unwrap();
    }
    assert_eq!(
        client
            .watch_files(rpc::WatchFilesRequest {
                subscription_id: "watchers".into(),
                request: rpc::StartWatchingRequest {
                    path: Some("/repo".into()),
                    ..Default::default()
                }
                .into(),
                ..Default::default()
            })
            .await
            .unwrap_err()
            .code,
        connectrpc::ErrorCode::ResourceExhausted
    );
    assert_eq!(files.active.lock().unwrap().len(), 64);
    files.fail_stop.store(true, Ordering::SeqCst);
    let error = client
        .stop_watching(rpc::StopWatchingRequest {
            watcher_id: watched.value,
            ..Default::default()
        })
        .await
        .unwrap_err();
    assert_eq!(error.code, connectrpc::ErrorCode::Internal);
    assert_eq!(files.active.lock().unwrap().len(), 64);
    files.fail_stop.store(false, Ordering::SeqCst);
    client
        .stop_watching(rpc::StopWatchingRequest {
            watcher_id: watched.value,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(files.active.lock().unwrap().len(), 63);
    drop(push);
    // Then
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while !files.active.lock().unwrap().is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        client
            .watch_files(rpc::WatchFilesRequest {
                subscription_id: "watchers".into(),
                request: rpc::StartWatchingRequest {
                    path: Some("/repo".into()),
                    ..Default::default()
                }
                .into(),
                ..Default::default()
            })
            .await
            .unwrap_err()
            .code,
        connectrpc::ErrorCode::NotFound
    );
    assert_eq!(
        client
            .watch_files(rpc::WatchFilesRequest::default())
            .await
            .unwrap_err()
            .code,
        connectrpc::ErrorCode::InvalidArgument
    );
    assert_eq!(files.active.lock().unwrap().len(), 0);
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
    let (client, server) = serve(dispatch, Arc::new(PushSink::new())).await;
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
async fn test_git監視_購読に束縛した公開経路から停止結果を返す() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let repository = Arc::new(crate::usecase::repository_state::service::tests::watching_service());
    let watcher = Arc::new(crate::usecase::watcher::WatcherUsecase::new(
        Some(repository.clone()),
        Arc::new(crate::usecase::watcher::watcher_tests::SubscriptionFiles::default()),
    ));
    let (client, server) = serve_with_watcher(dispatch(), Arc::new(PushSink::new()), watcher).await;
    let request = || rpc::WatchGitDirectoryRequest {
        subscription_id: "git".into(),
        request: rpc::StartGitDirWatchingRequest {
            repo_path: Some(directory.path().to_str().unwrap().into()),
            ..Default::default()
        }
        .into(),
        ..Default::default()
    };
    // When / Then
    assert_eq!(
        client
            .watch_git_directory(request())
            .await
            .unwrap_err()
            .code,
        connectrpc::ErrorCode::NotFound
    );
    let mut push = client
        .subscribe_push(rpc::SubscribePushRequest {
            subscription_id: "git".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    push.message::<rpc::Push>().await.unwrap().unwrap();
    let id = client
        .watch_git_directory(request())
        .await
        .unwrap()
        .into_owned()
        .value
        .unwrap();
    client
        .stop_watching(rpc::StopWatchingRequest {
            watcher_id: Some(id),
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(!repository.stop_watching(id).unwrap());
    drop(push);
    server.abort();
}

#[test]
fn test_監視開始_生成serviceは購読付きの二操作だけを公開する() {
    // Given
    let descriptor = prost_reflect::DescriptorPool::decode(
        include_bytes!(concat!(env!("OUT_DIR"), "/client_descriptor.bin")).as_slice(),
    )
    .unwrap();
    let service = descriptor
        .get_service_by_name("releash.client.v1.ClientService")
        .unwrap();
    let methods = service
        .methods()
        .map(|method| method.name().to_owned())
        .collect::<Vec<_>>();
    // Then
    assert!(methods.iter().any(|method| method == "WatchFiles"));
    assert!(methods.iter().any(|method| method == "WatchGitDirectory"));
    assert!(!methods.iter().any(|method| method == "StartWatching"));
    assert!(!methods.iter().any(|method| method == "StartGitDirWatching"));
}

#[tokio::test]
async fn test_設定保存_対象三操作の成功時だけdesktop再適用を応答で指示する() {
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;
    // Given
    let mut dispatch = dispatch();
    for (names, result) in [
        (
            &["update_app_settings"][..],
            wire::command_result::Command::UpdateAppSettings(wire::Unit {}),
        ),
        (
            &["update_crash_reporting"][..],
            wire::command_result::Command::UpdateCrashReporting(wire::Unit {}),
        ),
        (
            &["update_performance_telemetry"][..],
            wire::command_result::Command::UpdatePerformanceTelemetry(wire::Unit {}),
        ),
        (
            &["update_external_editor"][..],
            wire::command_result::Command::UpdateExternalEditor(wire::Unit {}),
        ),
    ] {
        dispatch.register_domain(
            names,
            Box::new(move |_| {
                let result = result.clone();
                Box::pin(async move { Ok(result) })
            }),
        );
    }
    let router = router(Some(ClientApiDeps::new(
        Arc::new(dispatch),
        ClientPushGateway::new(Arc::new(PushSink::new())),
        crate::client_api_acceptance::watcher(),
    )));
    // When / Then
    for (method, changed, success) in [
        ("UpdateAppSettings", true, true),
        ("UpdateCrashReporting", true, true),
        ("UpdatePerformanceTelemetry", true, true),
        ("UpdateExternalEditor", false, true),
        ("GetCurrentBranch", false, false),
    ] {
        let response = router
            .clone()
            .oneshot(
                Request::post(format!("/releash.client.v1.ClientService/{method}"))
                    .header("content-type", "application/json")
                    .header("connect-protocol-version", "1")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status().is_success(), success);
        assert_eq!(
            response
                .headers()
                .get("releash-desktop-settings-changed")
                .map(|value| value.to_str().unwrap()),
            changed.then_some("true")
        );
    }
}

#[tokio::test]
async fn test_監視rpc_通常要求と同じ枠を取得し上限時はblocking前に拒否する() {
    use axum::{
        body::{to_bytes, Body},
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;
    // Given
    let files = Arc::new(crate::usecase::watcher::watcher_tests::SubscriptionFiles::default());
    let watcher = Arc::new(crate::usecase::watcher::WatcherUsecase::new(
        None,
        files.clone(),
    ));
    let subscription = watcher.subscribe("limited".into()).unwrap();
    let deps = ClientApiDeps::new(
        Arc::new(dispatch()),
        ClientPushGateway::new(Arc::new(PushSink::new())),
        watcher,
    );
    let permits = deps
        .request_limit
        .clone()
        .try_acquire_many_owned(64)
        .unwrap();
    // When / Then
    let router = router(Some(deps.clone()));
    for (method, request) in [
        ("WatchFiles", serde_json::json!({"path": "/repo"})),
        (
            "WatchGitDirectory",
            serde_json::json!({"repoPath": "/repo"}),
        ),
    ] {
        let body = serde_json::json!({"subscriptionId": "limited", "request": request});
        let response = router
            .clone()
            .oneshot(
                Request::post(format!("/releash.client.v1.ClientService/{method}"))
                    .header("content-type", "application/json")
                    .header("connect-protocol-version", "1")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let error: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(error["code"], "resource_exhausted");
        assert_eq!(error["message"], "Too many pending client commands");
        assert_eq!(error["details"].as_array().unwrap().len(), 1);
        assert_eq!(
            error["details"][0]["type"],
            "releash.client.v1.CommandError"
        );
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD_NO_PAD
            .decode(error["details"][0]["value"].as_str().unwrap())
            .unwrap();
        let detail = wire::CommandError::decode(bytes.as_slice()).unwrap();
        let Some(wire::command_error::Variant::Coded(detail)) = detail.variant else {
            panic!("coded error");
        };
        assert_eq!(detail.code.as_deref(), Some("CLIENT_REQUEST_LIMIT"));
        assert_eq!(
            detail.message.as_deref(),
            Some("Too many pending client commands")
        );
    }
    assert_eq!(files.next.load(Ordering::SeqCst), 0);
    assert_eq!(
        deps.execute(wire::command_request::Command::GetCwd(
            wire::GetCwdRequest {}
        ))
        .await
        .unwrap_err()
        .code,
        connectrpc::ErrorCode::ResourceExhausted
    );
    drop(permits);
    deps.watch("limited".into(), "/repo".into(), false)
        .await
        .unwrap();
    assert_eq!(files.next.load(Ordering::SeqCst), 1);
    assert_eq!(deps.request_limit.available_permits(), 64);
    assert!(deps
        .watch("limited".into(), "/missing".into(), false)
        .await
        .is_err());
    assert_eq!(deps.request_limit.available_permits(), 64);
    drop(subscription);
}

#[test]
fn test_進行中要求の上限_構造化エラーで拒否理由を返し枠解放後は受理する() {
    // Given
    let deps = ClientApiDeps::new(
        Arc::new(dispatch()),
        ClientPushGateway::new(Arc::new(PushSink::new())),
        crate::client_api_acceptance::watcher(),
    );
    let permits = (0..64)
        .map(|_| deps.request_permit().unwrap())
        .collect::<Vec<_>>();
    // When
    let error = deps.request_permit().unwrap_err();
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
        Some("Too many pending client commands")
    );
    drop(permits);
    assert!(deps.request_permit().is_ok());
}

#[tokio::test]
async fn test_サーバ情報取得_上限時は設定取得を拒否し成功と失敗で要求枠を解放する() {
    use crate::adaptor::gateway::app_config::config_models::{config_to_domain, ReleashConfig};
    use crate::domain::app_config::{
        repository::{ConfigRepository, ConfigUpdate},
        value_objects::AppConfigDocument,
        AppConfigError,
    };
    use axum::{
        body::{to_bytes, Body},
        http::{Request, StatusCode},
    };

    use tower::ServiceExt;

    struct Config {
        loads: AtomicUsize,
        failure: AtomicUsize,
        limit: Arc<tokio::sync::Semaphore>,
    }
    impl ConfigRepository for Config {
        fn load(&self) -> Result<AppConfigDocument, AppConfigError> {
            self.loads.fetch_add(1, Ordering::SeqCst);
            assert_eq!(self.limit.available_permits(), 0);
            match self.failure.load(Ordering::SeqCst) {
                1 => return Err(AppConfigError::Repository("load failed".into())),
                2 => return Err(AppConfigError::InvalidInput("invalid settings".into())),
                _ => {}
            }
            Ok(config_to_domain(&ReleashConfig::default()))
        }
        fn save(&self, _: AppConfigDocument) -> Result<(), AppConfigError> {
            unreachable!()
        }
        fn update(&self, _: ConfigUpdate) -> Result<(), AppConfigError> {
            unreachable!()
        }
    }
    // Given
    let deps = ClientApiDeps::new(
        Arc::new(dispatch()),
        ClientPushGateway::new(Arc::new(PushSink::new())),
        crate::client_api_acceptance::watcher(),
    );
    let config = Arc::new(Config {
        loads: AtomicUsize::new(0),
        failure: AtomicUsize::new(0),
        limit: deps.request_limit.clone(),
    });
    let deps = deps.with_desktop_settings(crate::usecase::app_config::AppConfigUsecase::new(
        config.clone(),
    ));
    let _permits = deps
        .request_limit
        .clone()
        .try_acquire_many_owned(63)
        .unwrap();
    let last_permit = deps.request_permit().unwrap();
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
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let error: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(error["code"], "resource_exhausted");
    assert_eq!(config.loads.load(Ordering::SeqCst), 0);
    drop(last_permit);

    // When / Then
    for (failure, expected_status, expected_loads) in [
        (0, StatusCode::OK, 1),
        (1, StatusCode::INTERNAL_SERVER_ERROR, 2),
        (2, StatusCode::BAD_REQUEST, 3),
    ] {
        config.failure.store(failure, Ordering::SeqCst);
        let response = router.clone().oneshot(request()).await.unwrap();
        assert_eq!(response.status(), expected_status);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        if failure == 1 {
            assert_eq!(body["message"], "load failed");
            assert_eq!(body["code"], "internal");
        } else if failure == 2 {
            assert_eq!(body["message"], "invalid settings");
            assert_eq!(body["code"], "invalid_argument");
        } else {
            assert!(body["desktopSettings"].is_object());
        }
        assert_eq!(config.loads.load(Ordering::SeqCst), expected_loads);
        assert_eq!(deps.request_limit.available_permits(), 1);
    }
}

#[tokio::test]
async fn test_監視rpc_要求中断後もblocking終了まで枠を保持する() {
    struct BlockingFiles {
        started: tokio::sync::Notify,
        finish: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
    }
    impl crate::domain::repository::file_watcher::FileWatchGateway for BlockingFiles {
        fn start(&self, _: &str) -> Result<u64, String> {
            self.started.notify_one();
            self.finish.lock().unwrap().recv().unwrap();
            Ok(1)
        }
        fn stop(&self, _: u64) -> Result<(), String> {
            Ok(())
        }
    }
    // Given
    let (finish, receiver) = std::sync::mpsc::channel();
    let files = Arc::new(BlockingFiles {
        started: tokio::sync::Notify::new(),
        finish: std::sync::Mutex::new(receiver),
    });
    let watcher = Arc::new(crate::usecase::watcher::WatcherUsecase::new(
        None,
        files.clone(),
    ));
    let subscription = watcher.subscribe("blocking".into()).unwrap();
    let deps = ClientApiDeps::new(
        Arc::new(dispatch()),
        ClientPushGateway::new(Arc::new(PushSink::new())),
        watcher,
    );
    let task_deps = deps.clone();
    let task = tokio::spawn(async move {
        task_deps
            .watch("blocking".into(), "/repo".into(), false)
            .await
    });
    files.started.notified().await;
    // When
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    // Then
    assert_eq!(deps.request_limit.available_permits(), 63);
    finish.send(()).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while deps.request_limit.available_permits() != 64 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    drop(subscription);
}

#[tokio::test]
async fn test_push配信_不正payloadの復号失敗はstreamのinternalエラーとして返す() {
    // Given
    let sink = Arc::new(PushSink::new());
    let (client, server) = serve(dispatch(), sink.clone()).await;
    let mut stream = client
        .subscribe_push(rpc::SubscribePushRequest::default())
        .await
        .unwrap();
    stream.message::<rpc::Push>().await.unwrap().unwrap();
    // When
    sink.send(vec![0xff]);
    // Then
    assert_eq!(
        stream.message::<rpc::Push>().await.unwrap_err().code,
        connectrpc::ErrorCode::Internal
    );
    server.abort();
}

#[tokio::test]
async fn test_push購読_idは128byteまで受理し超過を保持前に拒否する() {
    // Given
    let watcher = crate::client_api_acceptance::watcher();
    let (client, server) =
        serve_with_watcher(dispatch(), Arc::new(PushSink::new()), watcher.clone()).await;
    for id in ["x".repeat(129), "あ".repeat(43)] {
        // When
        let mut stream = client
            .subscribe_push(rpc::SubscribePushRequest {
                subscription_id: id.clone(),
                ..Default::default()
            })
            .await
            .unwrap();
        // Then
        assert_eq!(
            stream.message::<rpc::Push>().await.unwrap_err().code,
            connectrpc::ErrorCode::InvalidArgument
        );
        assert!(watcher.subscribe(id).is_ok());
    }
    for id in ["x".repeat(128), uuid::Uuid::new_v4().to_string()] {
        let mut stream = client
            .subscribe_push(rpc::SubscribePushRequest {
                subscription_id: id,
                ..Default::default()
            })
            .await
            .unwrap();
        assert!(stream.message::<rpc::Push>().await.unwrap().is_some());
    }
    server.abort();
}

#[tokio::test]
async fn test_状態購読_connectで初期状態と変更と再開を配信する() {
    use crate::usecase::state_subscription::{StateSubscriptionUsecase, REPO_PATHS};
    use wire::state_subscription_event::Event;
    // Given
    let subscriptions = StateSubscriptionUsecase::new(
        vec!["/repo".into()],
        Arc::new(crate::adaptor::gateway::subscription_timer::TokioSubscriptionTimer),
    );
    let deps = ClientApiDeps::new(
        Arc::new(dispatch()),
        ClientPushGateway::new(Arc::new(PushSink::new())),
        crate::client_api_acceptance::watcher(),
    )
    .with_state_subscriptions(subscriptions.clone());
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
        target: REPO_PATHS.into(),
        version: None,
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
    crate::domain::repository::RepoPathsNotifier::notify_changed(
        &crate::adaptor::gateway::repository::notify::RepoPathsNotifyGateway::new(
            subscriptions.publisher(),
        ),
        vec!["/next".into()],
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
    assert_eq!(changed.version.unwrap().sequence, 1);
    client
        .stop_state_subscription(rpc::StopStateSubscriptionRequest {
            client_id: "state-test".into(),
            target: REPO_PATHS.into(),
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
    assert!(matches!(resumed.event, Some(Event::Change(_))));
    drop(stream);
    server.abort();
}
