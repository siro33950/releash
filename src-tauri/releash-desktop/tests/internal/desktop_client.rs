use futures_util::StreamExt;
use releash_desktop::test_support::integration::desktop_client::*;
use releashd::desktop_api::test_support::{to_rpc, Unit};
use releashd::desktop_api::{
    rpc, to_wire, wire, ClientConnectionDto, RetryLimiter, TechnicalFailureNature,
};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::{sync::Arc, time::Duration};

fn start(endpoint: &ClientConnectionDto) -> DesktopClient {
    DesktopClient::start(
        endpoint.clone(),
        client(endpoint).unwrap(),
        stream_client(endpoint).unwrap(),
        Arc::new(RetryLimiter::deterministic()),
    )
}

fn envelope(event: rpc::StateSubscriptionEvent) -> Vec<u8> {
    use prost::Message;
    let event: wire::StateSubscriptionEvent = to_wire(&event).unwrap();
    let payload = event.encode_to_vec();
    let mut bytes = vec![0];
    bytes.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    bytes.extend_from_slice(&payload);
    bytes
}

fn event(event: wire::state_subscription_event::Event) -> rpc::StateSubscriptionEvent {
    to_rpc(&wire::StateSubscriptionEvent {
        event: Some(event),
        ..Default::default()
    })
    .unwrap()
}

async fn stream_server(
    mut frames: Vec<Vec<u8>>,
    hold: bool,
) -> (
    ClientConnectionDto,
    tokio::task::JoinHandle<()>,
    Arc<AtomicUsize>,
    Arc<AtomicUsize>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = ClientConnectionDto {
        url: format!("http://{}", listener.local_addr().unwrap()),
        token: "client".into(),
    };
    if !hold {
        frames.push(vec![2, 0, 0, 0, 2, b'{', b'}']);
    }
    let opens = Arc::new(AtomicUsize::new(0));
    let requests = opens.clone();
    let starts = Arc::new(AtomicUsize::new(0));
    let subscriptions = starts.clone();
    let router = axum::Router::new()
        .route(
            "/releash.client.v1.ClientService/OpenStateStream",
            axum::routing::post(move || {
                let frames = frames.clone();
                let requests = requests.clone();
                async move {
                    requests.fetch_add(1, Ordering::SeqCst);
                    let body = futures_util::stream::iter(
                        frames.into_iter().map(Ok::<_, std::convert::Infallible>),
                    );
                    let body: std::pin::Pin<
                        Box<
                            dyn futures_util::Stream<
                                    Item = Result<Vec<u8>, std::convert::Infallible>,
                                > + Send,
                        >,
                    > = if hold {
                        Box::pin(body.chain(futures_util::stream::pending()))
                    } else {
                        Box::pin(body)
                    };
                    (
                        [("content-type", "application/connect+proto")],
                        axum::body::Body::from_stream(body),
                    )
                }
            }),
        )
        .route(
            "/releash.client.v1.ClientService/StartStateSubscription",
            axum::routing::post(move || {
                let subscriptions = subscriptions.clone();
                async move {
                    subscriptions.fetch_add(1, Ordering::SeqCst);
                    use prost::Message;
                    (
                        [("content-type", "application/proto")],
                        Unit {}.encode_to_vec(),
                    )
                }
            }),
        );
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (endpoint, server, opens, starts)
}

#[tokio::test(start_paused = true)]
async fn test_単発呼び出し_期限指定が無ければprotoの既定期限で切れる() {
    // Given
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = ClientConnectionDto {
        url: format!("http://{}", listener.local_addr().unwrap()),
        token: "client".into(),
    };
    let received = Arc::new(AtomicUsize::new(0));
    let requests = received.clone();
    let router = axum::Router::new().route(
        "/releash.client.v1.ClientService/GetServerInfo",
        axum::routing::post(move || {
            let requests = requests.clone();
            async move {
                requests.fetch_add(1, Ordering::SeqCst);
                std::future::pending::<axum::http::StatusCode>().await
            }
        }),
    );
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let call = tokio::spawn(async move {
        client(&endpoint)
            .unwrap()
            .get_server_info(rpc::Unit::default())
            .await
    });
    for _ in 0..1000 {
        if received.load(Ordering::SeqCst) > 0 {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert_eq!(received.load(Ordering::SeqCst), 1);
    // When
    tokio::time::advance(Duration::from_secs(119)).await;
    // Then
    assert!(!call.is_finished());
    // When
    tokio::time::advance(Duration::from_secs(1)).await;
    // Then
    let error = call.await.unwrap().unwrap_err();
    assert_eq!(error.code, connectrpc::ErrorCode::DeadlineExceeded);
    server.abort();
}

#[tokio::test]
async fn test_設定受信_生存確認と同じstreamで受け取る() {
    // Given
    let settings = wire::DesktopSettings {
        close_to_tray: Some(true),
        start_minimized: Some(false),
        crash_reporting: Some(false),
        performance_telemetry: Some(false),
        auto_launch: Some(true),
    };
    let (endpoint, server, opens, starts) = stream_server(
        vec![
            envelope(event(wire::state_subscription_event::Event::Ready(Unit {}))),
            envelope(event(wire::state_subscription_event::Event::Snapshot(
                wire::StatePayload {
                    value: Some(wire::state_payload::Value::DesktopSettings(settings)),
                },
            ))),
        ],
        true,
    )
    .await;
    let client = start(&endpoint);
    // When
    let received = tokio::time::timeout(Duration::from_secs(2), client.first_settings())
        .await
        .unwrap()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while starts.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    // Then
    assert_eq!(client.endpoint().url, endpoint.url);
    assert_eq!(client.endpoint().token, endpoint.token);
    assert!(received.close_to_tray);
    assert_eq!(opens.load(Ordering::SeqCst), 1);
    assert_eq!(starts.load(Ordering::SeqCst), 1);
    assert!(client.failure().is_none());
    server.abort();
}

async fn subscription_error_server(
    code: connectrpc::ErrorCode,
    status: axum::http::StatusCode,
) -> (
    ClientConnectionDto,
    tokio::task::JoinHandle<()>,
    Arc<AtomicUsize>,
    Arc<AtomicUsize>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = ClientConnectionDto {
        url: format!("http://{}", listener.local_addr().unwrap()),
        token: "client".into(),
    };
    let opens = Arc::new(AtomicUsize::new(0));
    let requests = opens.clone();
    let starts = Arc::new(AtomicUsize::new(0));
    let subscriptions = starts.clone();
    let router = axum::Router::new()
        .route(
            "/releash.client.v1.ClientService/OpenStateStream",
            axum::routing::post(move || {
                let requests = requests.clone();
                async move {
                    requests.fetch_add(1, Ordering::SeqCst);
                    let body = futures_util::stream::once(async {
                        Ok::<_, std::convert::Infallible>(envelope(event(
                            wire::state_subscription_event::Event::Ready(Unit {}),
                        )))
                    })
                    .chain(futures_util::stream::pending());
                    (
                        [("content-type", "application/connect+proto")],
                        axum::body::Body::from_stream(body),
                    )
                }
            }),
        )
        .route(
            "/releash.client.v1.ClientService/StartStateSubscription",
            axum::routing::post(move || {
                let subscriptions = subscriptions.clone();
                async move {
                    subscriptions.fetch_add(1, Ordering::SeqCst);
                    (
                        status,
                        axum::Json(connectrpc::ConnectError::new(code, "subscription failed")),
                    )
                }
            }),
        );
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (endpoint, server, opens, starts)
}

#[tokio::test]
async fn test_購読開始_再接続対象の失敗を生存失敗に数えない() {
    // Given
    let (endpoint, server, opens, starts) = subscription_error_server(
        connectrpc::ErrorCode::Unavailable,
        axum::http::StatusCode::SERVICE_UNAVAILABLE,
    )
    .await;
    let client = start(&endpoint);
    // When
    tokio::time::timeout(Duration::from_secs(6), async {
        while starts.load(Ordering::SeqCst) < 3 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    // Then
    assert!(client.failure().is_none());
    assert_eq!(opens.load(Ordering::SeqCst), 3);
    assert_eq!(client.failure(), None);
    server.abort();
}

#[tokio::test]
async fn test_購読開始_再接続対象外の失敗を初回の設定の失敗として返す() {
    // Given
    let (endpoint, server, _, _) = subscription_error_server(
        connectrpc::ErrorCode::InvalidArgument,
        axum::http::StatusCode::BAD_REQUEST,
    )
    .await;
    let client = start(&endpoint);
    // When
    let failure = tokio::time::timeout(Duration::from_secs(2), client.first_settings())
        .await
        .unwrap()
        .unwrap_err();
    // Then
    assert_eq!(failure.nature, TechnicalFailureNature::Other);
    assert!(failure.message.contains("subscription failed"));
    assert!(client.failure().is_none());
    server.abort();
}

async fn error_server(
    error: connectrpc::ConnectError,
) -> (
    ClientConnectionDto,
    tokio::task::JoinHandle<()>,
    Arc<AtomicUsize>,
) {
    let requests = Arc::new(AtomicUsize::new(0));
    let count = requests.clone();
    let router = axum::Router::new().fallback(axum::routing::post(move || {
        let error = error.clone();
        count.fetch_add(1, Ordering::SeqCst);
        async move { (axum::http::StatusCode::TOO_MANY_REQUESTS, axum::Json(error)) }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = ClientConnectionDto {
        url: format!("http://{}", listener.local_addr().unwrap()),
        token: "client".into(),
    };
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (endpoint, server, requests)
}

#[tokio::test]
async fn test_ネイティブ要求_停止とログイン項目の具体的な失敗理由を保持する() {
    use releashd::desktop_api::test_support::command_error;
    // Given / When / Then
    for detail in [
        wire::CommandError::from(releashd::desktop_api::test_support::AppError::new(
            "設定を保存できません",
        )),
        releashd::desktop_api::test_support::AppError::coded(
            "LOGIN_ITEM_SAVE_FAILED",
            "ログイン項目を保存できません",
            connectrpc::ErrorCode::Internal,
        )
        .into(),
    ] {
        let expected = match detail.variant.as_ref().unwrap() {
            wire::command_error::Variant::Message(value) => value.value.as_ref().unwrap(),
            wire::command_error::Variant::Coded(value) => value.message.as_ref().unwrap(),
        }
        .clone();
        let (endpoint, server, _) = error_server(command_error(detail.into())).await;
        let client = start(&endpoint);
        for command in [
            wire::command_request::Command::StopDaemon(Default::default()),
            wire::command_request::Command::UpdateAppSettings(Default::default()),
            wire::command_request::Command::UpdateLoginItemPreference(Default::default()),
        ] {
            assert_eq!(client.request(command).await, Err(expected.clone()));
        }
        assert_eq!(
            releash_desktop::test_support::integration::desktop_client::client(&endpoint)
                .unwrap()
                .get_server_info(rpc::Unit::default())
                .await
                .map(|_| ())
                .map_err(error_message),
            Err(expected)
        );
        server.abort();
    }
}
