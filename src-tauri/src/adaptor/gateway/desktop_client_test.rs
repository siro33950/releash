use super::*;
use futures_util::StreamExt;
use std::sync::atomic::{AtomicUsize, Ordering};

fn start(endpoint: &ClientConnectionDto) -> DesktopClient {
    DesktopClient::start(
        super::client(endpoint).unwrap(),
        super::stream_client(endpoint).unwrap(),
        Arc::new(RetryLimiter::deterministic()),
    )
}

async fn wait_for_disconnect(client: &DesktopClient, limit: Duration) {
    tokio::time::timeout(limit, async {
        while client.connected() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
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
                        wire::Unit {}.encode_to_vec(),
                    )
                }
            }),
        );
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (endpoint, server, opens, starts)
}

#[test]
fn test_接続規則_protoの値を読む() {
    // Given / When / Then
    assert_eq!(POLICY.silence, Duration::from_secs(20));
    assert_eq!(POLICY.default_timeout, Duration::from_secs(120));
    assert_eq!(POLICY.reset_after, Duration::from_secs(120));
    assert_eq!(POLICY.backoff.delay(1, 1.0), Duration::from_secs(1));
    assert!((POLICY.backoff.delay(2, 1.0).as_secs_f64() - 1.6).abs() < 0.0001);
    assert_eq!(POLICY.backoff.delay(100, 1.0), Duration::from_secs(120));
    assert!((POLICY.jitter - 0.2).abs() < 0.0001);
    assert_eq!(POLICY.reconnect_codes, [14, 10, 8]);
}

#[tokio::test(start_paused = true)]
async fn test_接続規則_protoの待ちを共通の計算で適用する() {
    // Given
    let limiter = RetryLimiter::deterministic();
    let start = tokio::time::Instant::now();
    // When
    limiter
        .wait_with_spread(POLICY.backoff, 1, POLICY.jitter)
        .await
        .unwrap();
    // Then
    assert_eq!(start.elapsed(), Duration::from_secs(1));
    // When
    limiter
        .wait_with_spread(POLICY.backoff, 2, POLICY.jitter)
        .await
        .unwrap();
    // Then
    assert!(
        (start.elapsed().as_secs_f64() - 2.6).abs() < 0.002,
        "{:?}",
        start.elapsed()
    );
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
async fn test_生存確認_接続拒否が2回続くと切断する() {
    // Given
    let endpoint = ClientConnectionDto {
        url: "http://127.0.0.1:1".into(),
        token: "client".into(),
    };
    let client = start(&endpoint);
    // When
    tokio::time::sleep(Duration::from_millis(200)).await;
    // Then
    assert!(client.connected());
    wait_for_disconnect(&client, Duration::from_secs(5)).await;
    assert_eq!(
        client.failure().unwrap().nature,
        TechnicalFailureNature::Transient
    );
}

#[tokio::test]
async fn test_生存確認_再接続対象外の開始失敗も2回続けば切断する() {
    // Given
    let (endpoint, server, requests) = error_server(connectrpc::ConnectError::new(
        connectrpc::ErrorCode::InvalidArgument,
        "invalid stream request",
    ))
    .await;
    let client = start(&endpoint);
    // When
    wait_for_disconnect(&client, Duration::from_secs(5)).await;
    // Then
    assert_eq!(requests.load(Ordering::SeqCst), 2);
    assert_eq!(
        client.failure().unwrap().nature,
        TechnicalFailureNature::Other
    );
    server.abort();
}

#[tokio::test]
async fn test_生存確認_再接続対象外の受信失敗も2回続けば切断する() {
    // Given
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = ClientConnectionDto {
        url: format!("http://{}", listener.local_addr().unwrap()),
        token: "client".into(),
    };
    let opens = Arc::new(AtomicUsize::new(0));
    let requests = opens.clone();
    let router = axum::Router::new().route(
        "/releash.client.v1.ClientService/OpenStateStream",
        axum::routing::post(move || {
            let requests = requests.clone();
            async move {
                requests.fetch_add(1, Ordering::SeqCst);
                (
                    [("content-type", "application/connect+proto")],
                    axum::body::Body::empty(),
                )
            }
        }),
    );
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let client = start(&endpoint);
    // When
    wait_for_disconnect(&client, Duration::from_secs(5)).await;
    // Then
    assert_eq!(opens.load(Ordering::SeqCst), 2);
    assert!(client.failure().is_some());
    server.abort();
}

#[tokio::test]
async fn test_生存確認_stream終了が2回続くと切断する() {
    // Given
    let (endpoint, server, opens, _) = stream_server(vec![], false).await;
    let client = start(&endpoint);
    // When
    wait_for_disconnect(&client, Duration::from_secs(5)).await;
    // Then
    assert_eq!(opens.load(Ordering::SeqCst), 2);
    assert_eq!(client.failure().unwrap().message, "State stream ended");
    server.abort();
}

#[tokio::test(start_paused = true)]
async fn test_生存確認_無音が20秒を超えると再接続し2回目で切断する() {
    // Given
    let (endpoint, server, opens, _) = stream_server(vec![], true).await;
    let client = start(&endpoint);
    // When
    wait_for_disconnect(&client, Duration::from_secs(50)).await;
    // Then
    assert_eq!(opens.load(Ordering::SeqCst), 2);
    assert_eq!(
        client.failure().unwrap().nature,
        TechnicalFailureNature::TimedOut
    );
    server.abort();
}

#[tokio::test(start_paused = true)]
async fn test_生存確認_変更が続いてもbookmarkが無ければ無音と判定する() {
    // Given
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = ClientConnectionDto {
        url: format!("http://{}", listener.local_addr().unwrap()),
        token: "client".into(),
    };
    let router = axum::Router::new().route(
        "/releash.client.v1.ClientService/OpenStateStream",
        axum::routing::post(|| async {
            let body = futures_util::stream::unfold((), |_| async {
                tokio::time::sleep(Duration::from_secs(1)).await;
                Some((
                    Ok::<_, std::convert::Infallible>(envelope(event(
                        wire::state_subscription_event::Event::Change(wire::StateChange {
                            delta: false,
                            payload: None,
                        }),
                    ))),
                    (),
                ))
            });
            (
                [("content-type", "application/connect+proto")],
                axum::body::Body::from_stream(body),
            )
        }),
    );
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let client = start(&endpoint);
    // When
    wait_for_disconnect(&client, Duration::from_secs(50)).await;
    // Then
    assert_eq!(
        client.failure().unwrap().nature,
        TechnicalFailureNature::TimedOut
    );
    server.abort();
}

#[tokio::test]
async fn test_生存確認_bookmarkが届くと連続失敗を消す() {
    // Given
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = ClientConnectionDto {
        url: format!("http://{}", listener.local_addr().unwrap()),
        token: "client".into(),
    };
    let opens = Arc::new(AtomicUsize::new(0));
    let count = opens.clone();
    let router = axum::Router::new().route(
        "/releash.client.v1.ClientService/OpenStateStream",
        axum::routing::post(move || {
            let attempt = count.fetch_add(1, Ordering::SeqCst);
            async move {
                let mut frames = if attempt == 1 {
                    vec![envelope(event(
                        wire::state_subscription_event::Event::Bookmark(wire::Unit {}),
                    ))]
                } else {
                    vec![]
                };
                if attempt < 2 {
                    frames.push(vec![2, 0, 0, 0, 2, b'{', b'}']);
                }
                let body = futures_util::stream::iter(
                    frames.into_iter().map(Ok::<_, std::convert::Infallible>),
                );
                let body: std::pin::Pin<
                    Box<
                        dyn futures_util::Stream<Item = Result<Vec<u8>, std::convert::Infallible>>
                            + Send,
                    >,
                > = if attempt >= 2 {
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
    );
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let client = start(&endpoint);
    // When
    tokio::time::timeout(Duration::from_secs(6), async {
        while opens.load(Ordering::SeqCst) < 3 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    // Then
    assert!(client.connected());
    assert_eq!(client.failure(), None);
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
            envelope(event(wire::state_subscription_event::Event::Ready(
                wire::Unit {},
            ))),
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
    assert!(received.close_to_tray);
    assert_eq!(opens.load(Ordering::SeqCst), 1);
    assert_eq!(starts.load(Ordering::SeqCst), 1);
    assert!(client.connected());
    server.abort();
}

#[test]
fn test_購読開始_重複したreadyで開始要求が増えない() {
    // Given
    let client = super::client(&ClientConnectionDto {
        url: "http://127.0.0.1:1".into(),
        token: "client".into(),
    })
    .unwrap();
    let mut subscription = SettingsSubscription::default();
    subscription.request_if_needed(&client, "client-id");
    let requested = subscription.has_pending();
    subscription.pending = None;
    // When
    subscription.request_if_needed(&client, "client-id");
    // Then
    assert!(requested);
    assert!(!subscription.has_pending());
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
                            wire::state_subscription_event::Event::Ready(wire::Unit {}),
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
    assert!(client.connected());
    assert_eq!(opens.load(Ordering::SeqCst), 3);
    assert_eq!(client.failure(), None);
    server.abort();
}

#[tokio::test(start_paused = true)]
async fn test_購読開始_再接続対象外の失敗でも生存監視を続ける() {
    // Given
    let (endpoint, server, opens, starts) = subscription_error_server(
        connectrpc::ErrorCode::InvalidArgument,
        axum::http::StatusCode::BAD_REQUEST,
    )
    .await;
    let client = start(&endpoint);
    // When
    wait_for_disconnect(&client, Duration::from_secs(50)).await;
    // Then
    assert_eq!(starts.load(Ordering::SeqCst), 2);
    assert_eq!(opens.load(Ordering::SeqCst), 2);
    assert_eq!(
        client.failure().unwrap().nature,
        TechnicalFailureNature::TimedOut
    );
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
    assert!(client.connected());
    server.abort();
}

#[tokio::test(start_paused = true)]
async fn test_設定変換失敗_生存監視を続ける() {
    // Given
    let (endpoint, server, opens, _) = stream_server(
        vec![envelope(event(
            wire::state_subscription_event::Event::Snapshot(wire::StatePayload {
                value: Some(wire::state_payload::Value::DesktopSettings(
                    wire::DesktopSettings::default(),
                )),
            }),
        ))],
        true,
    )
    .await;
    let client = start(&endpoint);
    // When
    wait_for_disconnect(&client, Duration::from_secs(50)).await;
    // Then
    assert_eq!(opens.load(Ordering::SeqCst), 2);
    assert_eq!(client.current_settings(), None);
    assert_eq!(
        client.failure().unwrap().nature,
        TechnicalFailureNature::TimedOut
    );
    server.abort();
}

#[tokio::test(start_paused = true)]
async fn test_購読開始_応答待ちでも無音でつなぎ直す() {
    // Given
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = ClientConnectionDto {
        url: format!("http://{}", listener.local_addr().unwrap()),
        token: "client".into(),
    };
    let opens = Arc::new(AtomicUsize::new(0));
    let stream_count = opens.clone();
    let starts = Arc::new(AtomicUsize::new(0));
    let start_count = starts.clone();
    let router = axum::Router::new()
        .route(
            "/releash.client.v1.ClientService/OpenStateStream",
            axum::routing::post(move || {
                let stream_count = stream_count.clone();
                async move {
                    stream_count.fetch_add(1, Ordering::SeqCst);
                    let body = futures_util::stream::once(async {
                        Ok::<_, std::convert::Infallible>(envelope(event(
                            wire::state_subscription_event::Event::Ready(wire::Unit {}),
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
                let start_count = start_count.clone();
                async move {
                    start_count.fetch_add(1, Ordering::SeqCst);
                    std::future::pending::<axum::http::StatusCode>().await
                }
            }),
        );
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let client = start(&endpoint);
    // When
    wait_for_disconnect(&client, Duration::from_secs(50)).await;
    // Then
    assert_eq!(opens.load(Ordering::SeqCst), 2);
    assert_eq!(starts.load(Ordering::SeqCst), 2);
    assert_eq!(
        client.failure().unwrap().nature,
        TechnicalFailureNature::TimedOut
    );
    server.abort();
}

#[tokio::test(start_paused = true)]
async fn test_長時間続いたstreamの後は初回の待ちへ戻る() {
    // Given
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = ClientConnectionDto {
        url: format!("http://{}", listener.local_addr().unwrap()),
        token: "client".into(),
    };
    let opened = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let attempts = opened.clone();
    let router = axum::Router::new().route(
        "/releash.client.v1.ClientService/OpenStateStream",
        axum::routing::post(move || {
            let attempt = {
                let mut opened = attempts.lock();
                opened.push(tokio::time::Instant::now());
                opened.len()
            };
            async move {
                let body: std::pin::Pin<
                    Box<
                        dyn futures_util::Stream<Item = Result<Vec<u8>, std::convert::Infallible>>
                            + Send,
                    >,
                > = match attempt {
                    1 => Box::pin(futures_util::stream::iter([Ok(vec![
                        2, 0, 0, 0, 2, b'{', b'}',
                    ])])),
                    2 => Box::pin(
                        futures_util::stream::unfold(0, |count| async move {
                            if count >= 13 {
                                return None;
                            }
                            tokio::time::sleep(Duration::from_secs(10)).await;
                            Some((
                                Ok(envelope(event(
                                    wire::state_subscription_event::Event::Bookmark(wire::Unit {}),
                                ))),
                                count + 1,
                            ))
                        })
                        .chain(futures_util::stream::once(async {
                            Ok(vec![2, 0, 0, 0, 2, b'{', b'}'])
                        })),
                    ),
                    _ => Box::pin(futures_util::stream::pending()),
                };
                (
                    [("content-type", "application/connect+proto")],
                    axum::body::Body::from_stream(body),
                )
            }
        }),
    );
    let listener = axum::serve::ListenerExt::tap_io(listener, |stream| {
        stream.set_nodelay(true).unwrap();
    });
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let client = start(&endpoint);
    // When
    let connected = tokio::time::timeout(Duration::from_secs(135), async {
        while opened.lock().len() < 3 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    assert!(
        connected.is_ok(),
        "opens={:?}, connected={}, failure={:?}",
        *opened.lock(),
        client.connected(),
        client.failure()
    );
    // Then
    let opened = opened.lock();
    assert!(opened[1].duration_since(opened[0]) >= Duration::from_secs(1));
    assert!(opened[1].duration_since(opened[0]) < Duration::from_millis(1_200));
    assert!(opened[2].duration_since(opened[1]) >= Duration::from_secs(131));
    assert!(
        opened[2].duration_since(opened[1]) < Duration::from_millis(131_200),
        "{:?}",
        opened[2].duration_since(opened[1])
    );
    assert!(client.connected());
    server.abort();
}

#[test]
fn test_生存確認_connectの失敗を技術的な失敗の性質へ写す() {
    use connectrpc::ErrorCode;
    // Given / When / Then
    for (code, nature) in [
        (
            ErrorCode::DeadlineExceeded,
            TechnicalFailureNature::TimedOut,
        ),
        (ErrorCode::Unavailable, TechnicalFailureNature::Transient),
        (
            ErrorCode::ResourceExhausted,
            TechnicalFailureNature::Transient,
        ),
        (ErrorCode::Aborted, TechnicalFailureNature::Other),
        (ErrorCode::Unauthenticated, TechnicalFailureNature::Other),
        (ErrorCode::Canceled, TechnicalFailureNature::Cancelled),
        (ErrorCode::Internal, TechnicalFailureNature::Other),
    ] {
        let error = connectrpc::ConnectError::new(code, "reason");
        let message = error.to_string();
        let failure = liveness_failure(error);
        assert_eq!(failure.nature, nature);
        assert_eq!(failure.message, message);
    }
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
    use crate::adaptor::presenter::connect::command_error;
    // Given / When / Then
    for detail in [
        wire::CommandError::from(crate::adaptor::presenter::error::AppError::new(
            "設定を保存できません",
        )),
        crate::adaptor::presenter::error::AppError::coded(
            "LOGIN_ITEM_SAVE_FAILED",
            "ログイン項目を保存できません",
            connectrpc::ErrorCode::Internal,
        )
        .into(),
        wire::CommandError {
            variant: Some(wire::command_error::Variant::Application(Box::new(
                wire::ApplicationError {
                    r#type: Some("shutdown_failed".into()),
                    message: Some("実行中の処理を停止できません".into()),
                    ..Default::default()
                },
            ))),
        },
    ] {
        let expected = match detail.variant.as_ref().unwrap() {
            wire::command_error::Variant::Message(value) => value.value.as_ref().unwrap(),
            wire::command_error::Variant::Coded(value) => value.message.as_ref().unwrap(),
            wire::command_error::Variant::Application(value) => value.message.as_ref().unwrap(),
        }
        .clone();
        let (endpoint, server, _) = error_server(command_error(wire::CommandFailure {
            message: None,
            kind: connectrpc::ErrorCode::Internal,
            detail,
        }))
        .await;
        let client = start(&endpoint);
        for command in [
            wire::command_request::Command::RequestApplicationQuit(Default::default()),
            wire::command_request::Command::UpdateAppSettings(Default::default()),
            wire::command_request::Command::UpdateLoginItemPreference(Default::default()),
        ] {
            assert_eq!(client.request(command).await, Err(expected.clone()));
        }
        assert_eq!(server_info(&endpoint).await, Err(expected));
        server.abort();
    }
}

#[test]
fn test_ネイティブエラー_detailが無いか不正な場合は元の診断を保持する() {
    // Given / When / Then
    let error = connectrpc::ConnectError::unavailable("connection refused");
    let expected = error.to_string();
    assert_eq!(error_message(error.clone()), expected);
    for (type_url, value) in [
        ("releash.client.v1.CommandError", Some("invalid-base64")),
        ("releash.client.v1.CommandError", Some("AA")),
        ("releash.client.v1.CommandError", None),
        ("other.Error", Some("AA")),
    ] {
        assert_eq!(
            error_message(error.clone().with_detail(connectrpc::ErrorDetail {
                type_url: type_url.into(),
                value: value.map(Into::into),
                debug: None,
            })),
            expected
        );
    }
}
