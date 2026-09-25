use super::*;
use crate::infrastructure::push::PushSink;
use crate::usecase::application_startup::ApplicationStartupAuthority;
use connectrpc::client::{ClientConfig, HttpClient};
use prost::Message;
use std::sync::atomic::{AtomicUsize, Ordering};

fn dispatch() -> ClientCommandDispatch {
    ClientCommandDispatch::new(Arc::new(ApplicationStartupAuthority::ready()))
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
        &["get_external_editor"],
        Box::new(|_| {
            Box::pin(async {
                Ok(wire::command_result::Command::GetExternalEditor(
                    wire::ResultString {
                        value: Some("/repo".into()),
                    },
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
    let (client, server) = serve(dispatch, Arc::new(PushSink::new())).await;
    // When / Then
    assert_eq!(
        client
            .get_external_editor(rpc::GetExternalEditorRequest::default())
            .await
            .unwrap()
            .into_owned()
            .value,
        Some("/repo".into())
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
        ("BuildDiffFileTree", false, false),
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
        deps.execute(
            None,
            wire::command_request::Command::GetExternalEditor(wire::GetExternalEditorRequest {})
        )
        .await
        .unwrap_err()
        .code,
        connectrpc::ErrorCode::ResourceExhausted
    );
    drop(permits);
    deps.watch(None, "limited".into(), "/repo".into(), false)
        .await
        .unwrap();
    assert_eq!(files.next.load(Ordering::SeqCst), 1);
    assert_eq!(deps.request_limit.available_permits(), 64);
    assert!(deps
        .watch(None, "limited".into(), "/missing".into(), false)
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
async fn test_監視rpc_要求中断でblocking終了前に枠を解放する() {
    struct BlockingFiles {
        started: tokio::sync::Notify,
        stopped: tokio::sync::Notify,
        finish: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
    }
    impl crate::domain::repository::file_watcher::FileWatchGateway for BlockingFiles {
        fn release(&self, _: u64) {
            self.stopped.notify_one();
        }
        fn start(&self, _: &str) -> Result<u64, String> {
            self.started.notify_one();
            self.finish.lock().unwrap().recv().unwrap();
            Ok(1)
        }
        fn stop(&self, _: u64) -> Result<(), String> {
            Err("ordinary stop failed".into())
        }
    }
    // Given
    let (finish, receiver) = std::sync::mpsc::channel();
    let files = Arc::new(BlockingFiles {
        started: tokio::sync::Notify::new(),
        stopped: tokio::sync::Notify::new(),
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
            .watch(None, "blocking".into(), "/repo".into(), false)
            .await
    });
    files.started.notified().await;
    // When
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    // Then
    assert_eq!(deps.request_limit.available_permits(), 64);
    finish.send(()).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while deps.request_limit.available_permits() != 64 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(1), files.stopped.notified())
        .await
        .unwrap();
    drop(subscription);
}

#[tokio::test]
async fn test_監視停止rpc_要求中断でblocking終了前に枠を解放する() {
    struct BlockingFiles {
        started: tokio::sync::Notify,
        finish: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
    }
    impl crate::domain::repository::file_watcher::FileWatchGateway for BlockingFiles {
        fn release(&self, _: u64) {}
        fn start(&self, _: &str) -> Result<u64, String> {
            Ok(1)
        }
        fn stop(&self, _: u64) -> Result<(), String> {
            self.started.notify_one();
            self.finish.lock().unwrap().recv().unwrap();
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
    let deps = ClientApiDeps::new(
        Arc::new(dispatch()),
        ClientPushGateway::new(Arc::new(PushSink::new())),
        watcher,
    );
    let id = 1;
    let task_deps = deps.clone();
    let task = tokio::spawn(async move {
        task_deps
            .execute(
                None,
                wire::command_request::Command::StopWatching(wire::StopWatchingRequest {
                    watcher_id: Some(id),
                }),
            )
            .await
    });
    files.started.notified().await;
    // When
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    // Then
    assert_eq!(deps.request_limit.available_permits(), 64);
    assert!(deps.request_permit().is_ok());
    finish.send(()).unwrap();
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
        args: vec![],
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
        &["get_external_editor"],
        Box::new(move |_| {
            let guard = signal.clone().drop_guard();
            Box::pin(async move {
                let _guard = guard;
                std::future::pending().await
            })
        }),
    );
    let deps = ClientApiDeps::new(
        Arc::new(dispatch),
        ClientPushGateway::new(Arc::new(PushSink::new())),
        crate::client_api_acceptance::watcher(),
    );
    let mut request = Request::post("/releash.client.v1.ClientService/GetExternalEditor")
        .header("content-type", "application/json")
        .header("connect-protocol-version", "1");
    if let Some(timeout) = timeout {
        request = request.header("connect-timeout-ms", timeout);
    }
    let call = router(Some(deps.clone())).oneshot(request.body(Body::from("{}")).unwrap());
    tokio::pin!(call);
    assert!(futures_util::poll!(&mut call).is_pending());
    tokio::task::yield_now().await;
    assert_eq!(deps.request_limit.available_permits(), 63);
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
    assert_eq!(deps.request_limit.available_permits(), 64);
    assert!(deps.request_permit().is_ok());
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
    // Given
    let stopped = tokio_util::sync::CancellationToken::new();
    let signal = stopped.clone();
    let mut dispatch = dispatch();
    dispatch.register_domain(
        &["get_external_editor"],
        Box::new(move |_| {
            let guard = signal.clone().drop_guard();
            Box::pin(async move {
                let _guard = guard;
                std::future::pending().await
            })
        }),
    );
    let deps = ClientApiDeps::new(
        Arc::new(dispatch),
        ClientPushGateway::new(Arc::new(PushSink::new())),
        crate::client_api_acceptance::watcher(),
    );
    let mut call = Box::pin(deps.execute(
        None,
        wire::command_request::Command::GetExternalEditor(wire::GetExternalEditorRequest {}),
    ));
    assert!(futures_util::poll!(&mut call).is_pending());
    tokio::task::yield_now().await;
    // When
    drop(call);
    // Then
    assert_eq!(deps.request_limit.available_permits(), 64);
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
        &["get_external_editor"],
        Box::new(|_| Box::pin(std::future::pending())),
    );
    let error = run_command(
        &token,
        dispatch.dispatch_admitted(wire::command_request::Command::GetExternalEditor(
            wire::GetExternalEditorRequest {},
        )),
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, connectrpc::ErrorCode::Canceled);
}

#[tokio::test(start_paused = true)]
async fn test_購読stream_既定期限を過ぎても配信できる() {
    use axum::{body::Body, http::Request};
    use futures_util::StreamExt;
    use tower::ServiceExt;
    // Given
    let sink = Arc::new(PushSink::new());
    let deps = ClientApiDeps::new(
        Arc::new(dispatch()),
        ClientPushGateway::new(sink.clone()),
        crate::client_api_acceptance::watcher(),
    );
    let response = router(Some(deps))
        .oneshot(
            Request::post("/releash.client.v1.ClientService/SubscribePush")
                .header("content-type", "application/connect+json")
                .header("connect-protocol-version", "1")
                .body(Body::from(vec![0, 0, 0, 0, 2, b'{', b'}']))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response.status().is_success());
    let mut body = response.into_body().into_data_stream();
    assert!(body.next().await.unwrap().is_ok());
    // When
    tokio::time::advance(std::time::Duration::from_secs(121)).await;
    assert!(futures_util::poll!(body.next()).is_pending());
    crate::adaptor::gateway::push::BackendPush::ReviewCommentsChanged("/next".into()).emit(&sink);
    // Then
    let frame = body.next().await.unwrap().unwrap();
    assert_eq!(frame[0], 0);
    assert!(std::str::from_utf8(&frame[5..]).unwrap().contains("/next"));
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
        &["get_external_editor"],
        Box::new(move |_| {
            let guard = signal.clone().drop_guard();
            start_signal.cancel();
            Box::pin(async move {
                let _guard = guard;
                std::future::pending().await
            })
        }),
    );
    let deps = ClientApiDeps::new(
        Arc::new(dispatch),
        ClientPushGateway::new(Arc::new(PushSink::new())),
        crate::client_api_acceptance::watcher(),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let router = router(Some(deps.clone()));
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let mut connection = tokio::net::TcpStream::connect(address).await.unwrap();
    connection.write_all(b"POST /releash.client.v1.ClientService/GetExternalEditor HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nConnect-Protocol-Version: 1\r\nContent-Length: 2\r\n\r\n{}").await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(2), started.cancelled())
        .await
        .unwrap();
    // When
    drop(connection);
    // Then
    let result = tokio::time::timeout(std::time::Duration::from_secs(2), stopped.cancelled()).await;
    server.abort();
    result.unwrap();
    assert_eq!(deps.request_limit.available_permits(), 64);
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
    let deps = ClientApiDeps::new(
        Arc::new(dispatch()),
        ClientPushGateway::new(Arc::new(PushSink::new())),
        crate::client_api_acceptance::watcher(),
    )
    .with_state_subscriptions(subscriptions.clone());
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
    subscriptions
        .start("deadline-test", "repository-paths", None)
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
    let deps = ClientApiDeps::new(
        Arc::new(dispatch()),
        ClientPushGateway::new(Arc::new(PushSink::new())),
        crate::client_api_acceptance::watcher(),
    )
    .with_state_subscriptions(subscriptions);
    let router = router(Some(deps.clone()));
    for method in ["StartStateSubscription", "StopStateSubscription"] {
        let permits = (0..64)
            .map(|_| deps.request_permit().unwrap())
            .collect::<Vec<_>>();
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
        assert_eq!(deps.request_limit.available_permits(), 64);
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
    let deps = ClientApiDeps::new(
        Arc::new(dispatch),
        ClientPushGateway::new(Arc::new(PushSink::new())),
        crate::client_api_acceptance::watcher(),
    );
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
    assert_eq!(deps.request_limit.available_permits(), 64);
    assert!(deps.request_permit().is_ok());
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
    use crate::domain::operation_context::OperationStopped;
    use std::time::{Duration, Instant};
    for expire in [false, true] {
        // Given
        let (started, mut ready) = tokio::sync::mpsc::unbounded_channel();
        let (stopped, mut stopped_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut dispatch = dispatch();
        dispatch.register_domain(
            &["get_external_editor"],
            Box::new(move |_| {
                let started = started.clone();
                let stopped = stopped.clone();
                Box::pin(async move {
                    crate::other::operation_context::spawn_blocking(move || {
                        started.send(()).unwrap();
                        let error = crate::other::operation_context::sleep(
                            &crate::other::operation_context::current(),
                            Duration::from_secs(30),
                        )
                        .unwrap_err();
                        stopped.send(error).unwrap();
                        Err(crate::other::AppError::from_failure(error).into())
                    })
                    .await
                    .unwrap()
                })
            }),
        );
        let deps = ClientApiDeps::new(
            Arc::new(dispatch),
            ClientPushGateway::new(Arc::new(PushSink::new())),
            crate::client_api_acceptance::watcher(),
        );
        let mut call = Box::pin(deps.execute(
            expire.then(|| Instant::now() + Duration::from_millis(100)),
            wire::command_request::Command::GetExternalEditor(wire::GetExternalEditorRequest {}),
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
        assert_eq!(deps.request_limit.available_permits(), 64);
    }
}

#[tokio::test]
async fn test_監視rpc_期限と呼出破棄が同期処理の内側まで届く() {
    use crate::domain::operation_context::OperationStopped;
    use std::time::{Duration, Instant};
    struct ContextFiles {
        started: tokio::sync::mpsc::UnboundedSender<()>,
        stopped: tokio::sync::mpsc::UnboundedSender<OperationStopped>,
    }
    impl crate::domain::repository::file_watcher::FileWatchGateway for ContextFiles {
        fn release(&self, _: u64) {}
        fn start(&self, _: &str) -> Result<u64, String> {
            self.started.send(()).unwrap();
            let error = crate::other::operation_context::sleep(
                &crate::other::operation_context::current(),
                Duration::from_secs(30),
            )
            .unwrap_err();
            self.stopped.send(error).unwrap();
            Err(error.to_string())
        }
        fn stop(&self, _: u64) -> Result<(), String> {
            Ok(())
        }
    }
    for expire in [false, true] {
        let (started, mut ready) = tokio::sync::mpsc::unbounded_channel();
        let (stopped, mut stopped_rx) = tokio::sync::mpsc::unbounded_channel();
        let watcher = Arc::new(crate::usecase::watcher::WatcherUsecase::new(
            None,
            Arc::new(ContextFiles { started, stopped }),
        ));
        let subscription = watcher.subscribe("context".into()).unwrap();
        let deps = ClientApiDeps::new(
            Arc::new(dispatch()),
            ClientPushGateway::new(Arc::new(PushSink::new())),
            watcher,
        );
        let mut call = Box::pin(deps.watch(
            expire.then(|| Instant::now() + Duration::from_millis(100)),
            "context".into(),
            "/repo".into(),
            false,
        ));
        tokio::select! { _ = ready.recv() => {}, result = &mut call => panic!("call ended before starting: {result:?}") }
        if expire {
            assert_eq!(
                call.await.unwrap_err().code,
                connectrpc::ErrorCode::DeadlineExceeded
            );
        } else {
            drop(call);
        }
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
        assert_eq!(deps.request_limit.available_permits(), 64);
        drop(subscription);
    }
}

#[tokio::test]
async fn test_監視rpc_登録後の期限切れでidを返せない監視を解除する() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};
    struct Files {
        started: AtomicUsize,
        stopped: AtomicUsize,
    }
    impl crate::domain::repository::file_watcher::FileWatchGateway for Files {
        fn release(&self, id: u64) {
            assert_eq!(id, 42);
            self.stopped.fetch_add(1, Ordering::SeqCst);
        }
        fn start(&self, _: &str) -> Result<u64, String> {
            self.started.fetch_add(1, Ordering::SeqCst);
            let _ = crate::other::operation_context::sleep(
                &crate::other::operation_context::current(),
                Duration::from_secs(5),
            );
            Ok(42)
        }
        fn stop(&self, id: u64) -> Result<(), String> {
            assert_eq!(id, 42);
            Err("ordinary stop failed".into())
        }
    }
    // Given
    let files = Arc::new(Files {
        started: AtomicUsize::new(0),
        stopped: AtomicUsize::new(0),
    });
    let watcher = Arc::new(crate::usecase::watcher::WatcherUsecase::new(
        None,
        files.clone(),
    ));
    let subscription = watcher.subscribe("expires".into()).unwrap();
    let deps = ClientApiDeps::new(
        Arc::new(dispatch()),
        ClientPushGateway::new(Arc::new(PushSink::new())),
        watcher,
    );
    // When / Then
    let error = deps
        .watch(
            Some(Instant::now()),
            "expires".into(),
            "/repo".into(),
            false,
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, connectrpc::ErrorCode::DeadlineExceeded);
    assert_eq!(files.started.load(Ordering::SeqCst), 0);
    let error = deps
        .watch(
            Some(Instant::now() + Duration::from_millis(100)),
            "expires".into(),
            "/repo".into(),
            false,
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, connectrpc::ErrorCode::DeadlineExceeded);
    assert_eq!(files.started.load(Ordering::SeqCst), 1);
    assert_eq!(files.stopped.load(Ordering::SeqCst), 1);
    drop(subscription);
}
