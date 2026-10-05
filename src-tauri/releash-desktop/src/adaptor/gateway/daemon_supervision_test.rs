use super::*;
use releash_lib::desktop_api::test_support::Unit;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::{cell::Cell, time::Duration};

fn gateway(data_dir: PathBuf) -> DaemonProcessGateway {
    DaemonProcessGateway::new(PathBuf::new(), data_dir, Arc::new(RetryLimiter::new()))
}

#[test]
fn test_接続失敗_技術的分類を監督へ渡す() {
    // Given
    let failure = TechnicalFailure {
        nature: releash_lib::desktop_api::TechnicalFailureNature::TimedOut,
        message: "State stream was silent".into(),
    };
    // When
    let result = supervised_connection_failure(failure);
    // Then
    assert_eq!(
        result.stage,
        FailureStage::Connection(releash_lib::desktop_api::TechnicalFailureNature::TimedOut)
    );
    assert_eq!(result.reason, "State stream was silent");
}

#[tokio::test]
async fn test_daemon接続_認証と検証が完了した呼び出しで接続情報を返す() {
    use prost::Message;
    use releash_lib::desktop_api::test_support::LocalApiDiscoveryFile;
    use releash_lib::desktop_api::{process_start_time, LocalApiDiscovery};
    // Given
    let directory = tempfile::tempdir().unwrap();
    let gateway = gateway(directory.path().into());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let discovery = LocalApiDiscovery {
        port: listener.local_addr().unwrap().port(),
        token: "master-token".into(),
        instance_id: "instance".into(),
        pid: std::process::id(),
        process_started_at: process_start_time(std::process::id()).unwrap(),
    };
    LocalApiDiscoveryFile::create(directory.path(), discovery.clone()).unwrap();
    LocalApiDiscoveryFile::create_client(
        directory.path(),
        LocalApiDiscovery {
            token: "client-token".into(),
            ..discovery
        },
    )
    .unwrap();
    let router = axum::Router::new()
        .route(
            "/releash.client.v1.ClientService/GetServerInfo",
            axum::routing::post(|headers: axum::http::HeaderMap| async move {
                assert_eq!(headers["authorization"], "Bearer client-token");
                assert_eq!(headers["origin"], "tauri://localhost");
                let info = wire::ServerInfo {
                    launch_id: "launch".into(),
                    release: env!("CARGO_PKG_VERSION").into(),
                    ..Default::default()
                };
                (
                    [("content-type", "application/proto")],
                    info.encode_to_vec(),
                )
            }),
        )
        .route(
            "/releash.client.v1.ClientService/OpenStateStream",
            axum::routing::post(|| async move {
                use wire::state_subscription_event::Event;
                let settings = wire::DesktopSettings {
                    close_to_tray: Some(true),
                    start_minimized: Some(false),
                    crash_reporting: Some(false),
                    performance_telemetry: Some(false),
                    auto_launch: Some(true),
                };
                let events = [
                    wire::StateSubscriptionEvent {
                        event: Some(Event::Ready(Unit {})),
                        ..Default::default()
                    },
                    wire::StateSubscriptionEvent {
                        subscription_id: "settings".into(),
                        event: Some(Event::Snapshot(wire::StatePayload {
                            value: Some(wire::state_payload::Value::DesktopSettings(settings)),
                        })),
                        ..Default::default()
                    },
                ];
                let mut body = Vec::new();
                for event in events {
                    let bytes = event.encode_to_vec();
                    body.push(0);
                    body.extend((bytes.len() as u32).to_be_bytes());
                    body.extend(bytes);
                }
                body.push(2);
                body.extend(2u32.to_be_bytes());
                body.extend(b"{}");
                ([("content-type", "application/connect+proto")], body)
            }),
        )
        .route(
            "/releash.client.v1.ClientService/StartStateSubscription",
            axum::routing::post(|| async {
                (
                    [("content-type", "application/proto")],
                    Unit {}.encode_to_vec(),
                )
            }),
        );
    let peer = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    // When
    let started_at_ms = gateway.monotonic_ms();
    let connection = tokio::time::timeout(Duration::from_secs(2), gateway.connect_client("launch"))
        .await
        .unwrap();
    // Then
    let connection = connection
        .unwrap()
        .expect("connection must be reported immediately");
    assert_eq!(connection.launch_id, "launch");
    assert_eq!(connection.release, env!("CARGO_PKG_VERSION"));
    assert!(connection.settings.auto_launch);
    assert!((started_at_ms..=gateway.monotonic_ms()).contains(&connection.connected_at_ms));
    assert!(gateway.connected());
    let cached = gateway.connection().await.unwrap().unwrap();
    assert_eq!(cached.launch_id, connection.launch_id);
    assert_eq!(cached.connected_at_ms, connection.connected_at_ms);
    peer.abort();
}

#[tokio::test]
async fn test_初回設定待ち_購読開始失敗後も同じclientがprotoの待ちで再接続する() {
    use axum::response::IntoResponse;
    use futures_util::StreamExt;
    use prost::Message;
    use releash_lib::desktop_api::test_support::LocalApiDiscoveryFile;
    use releash_lib::desktop_api::{process_start_time, LocalApiDiscovery};
    use wire::state_subscription_event::Event;

    // Given
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let endpoint = releash_lib::desktop_api::ClientConnectionDto {
        url: format!("http://{}", listener.local_addr().unwrap()),
        token: "client".into(),
    };
    let frame = |event| {
        let bytes = wire::StateSubscriptionEvent {
            event: Some(event),
            ..Default::default()
        }
        .encode_to_vec();
        let mut frame = vec![0];
        frame.extend((bytes.len() as u32).to_be_bytes());
        frame.extend(bytes);
        frame
    };
    let ready = frame(Event::Ready(Unit {}));
    let snapshot = frame(Event::Snapshot(wire::StatePayload {
        value: Some(wire::state_payload::Value::DesktopSettings(
            wire::DesktopSettings {
                close_to_tray: Some(true),
                start_minimized: Some(false),
                crash_reporting: Some(false),
                performance_telemetry: Some(false),
                auto_launch: Some(true),
            },
        )),
    }));
    let opened = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let open_times = opened.clone();
    let starts = Arc::new(AtomicUsize::new(0));
    let start_count = starts.clone();
    let subscribed = Arc::new(tokio::sync::Notify::new());
    let stream_subscribed = subscribed.clone();
    let start_subscribed = subscribed.clone();
    let router = axum::Router::new()
        .route(
            "/releash.client.v1.ClientService/GetServerInfo",
            axum::routing::post(|| async {
                (
                    [("content-type", "application/proto")],
                    wire::ServerInfo {
                        launch_id: "launch".into(),
                        release: env!("CARGO_PKG_VERSION").into(),
                        ..Default::default()
                    }
                    .encode_to_vec(),
                )
            }),
        )
        .route(
            "/releash.client.v1.ClientService/OpenStateStream",
            axum::routing::post(move || {
                let opened = open_times.clone();
                let ready = ready.clone();
                let snapshot = snapshot.clone();
                let subscribed = stream_subscribed.clone();
                async move {
                    let attempt = {
                        let mut opened = opened.lock();
                        opened.push(tokio::time::Instant::now());
                        opened.len()
                    };
                    let ready = futures_util::stream::once(async {
                        Ok::<_, std::convert::Infallible>(ready)
                    });
                    let body: std::pin::Pin<
                        Box<
                            dyn futures_util::Stream<
                                    Item = Result<Vec<u8>, std::convert::Infallible>,
                                > + Send,
                        >,
                    > = if attempt == 3 {
                        Box::pin(
                            ready
                                .chain(futures_util::stream::once(async move {
                                    subscribed.notified().await;
                                    Ok(snapshot)
                                }))
                                .chain(futures_util::stream::pending()),
                        )
                    } else {
                        Box::pin(ready.chain(futures_util::stream::pending()))
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
                let starts = start_count.clone();
                let subscribed = start_subscribed.clone();
                async move {
                    if starts.fetch_add(1, Ordering::SeqCst) < 2 {
                        (
                            axum::http::StatusCode::SERVICE_UNAVAILABLE,
                            axum::Json(connectrpc::ConnectError::new(
                                connectrpc::ErrorCode::Unavailable,
                                "subscription failed",
                            )),
                        )
                            .into_response()
                    } else {
                        subscribed.notify_one();
                        (
                            [("content-type", "application/proto")],
                            Unit {}.encode_to_vec(),
                        )
                            .into_response()
                    }
                }
            }),
        );
    let peer = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let directory = tempfile::tempdir().unwrap();
    let discovery = LocalApiDiscovery {
        port,
        token: "master-token".into(),
        instance_id: "instance".into(),
        pid: std::process::id(),
        process_started_at: process_start_time(std::process::id()).unwrap(),
    };
    LocalApiDiscoveryFile::create(directory.path(), discovery.clone()).unwrap();
    LocalApiDiscoveryFile::create_client(
        directory.path(),
        LocalApiDiscovery {
            token: endpoint.token,
            ..discovery
        },
    )
    .unwrap();
    let gateway = DaemonProcessGateway::new(
        PathBuf::new(),
        directory.path().into(),
        Arc::new(RetryLimiter::deterministic()),
    );

    // When
    assert!(gateway.connect_client("launch").await.unwrap().is_none());
    assert!(gateway.pending.lock().is_some());
    let connection = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(connection) = gateway.connection().await.unwrap() {
                break connection;
            }
        }
    })
    .await
    .unwrap();

    // Then
    let opened = opened.lock();
    assert_eq!(opened.len(), 3);
    assert!(opened[1].duration_since(opened[0]) >= Duration::from_secs(1));
    assert!(opened[2].duration_since(opened[1]) >= Duration::from_millis(1_600));
    assert_eq!(starts.load(Ordering::SeqCst), 3);
    assert!(connection.settings.auto_launch);
    assert!(gateway.pending.lock().is_none());
    assert!(gateway.connected());
    peer.abort();
}

#[tokio::test]
async fn test_初回設定待ち_生存失敗が確定したら分類を保って監督へ渡す() {
    // Given
    let gateway = gateway(PathBuf::new());
    let endpoint = releash_lib::desktop_api::ClientConnectionDto {
        url: "http://127.0.0.1:1".into(),
        token: "client".into(),
    };
    let client = Arc::new(super::super::desktop_client::DesktopClient::start(
        super::super::desktop_client::client(&endpoint).unwrap(),
        super::super::desktop_client::stream_client(&endpoint).unwrap(),
        Arc::new(RetryLimiter::deterministic()),
    ));
    *gateway.pending.lock() = Some(PendingConnection {
        client,
        hello: wire::ServerInfo {
            launch_id: "launch".into(),
            release: env!("CARGO_PKG_VERSION").into(),
            ..Default::default()
        },
        endpoint,
    });

    // When
    let failure = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match gateway.connection().await {
                Err(failure) => break failure,
                Ok(None) => {}
                Ok(Some(_)) => panic!("no settings were delivered"),
            }
        }
    })
    .await
    .unwrap();

    // Then
    assert_eq!(
        failure.stage,
        FailureStage::Connection(releash_lib::desktop_api::TechnicalFailureNature::Transient)
    );
    assert!(gateway.pending.lock().is_none());
}

#[tokio::test(start_paused = true)]
async fn test_daemon停止_自然終了なら強制終了しない() {
    // Given
    let start = tokio::time::Instant::now();
    // When
    wait_for_termination(
        || std::future::ready(Ok(start.elapsed() >= Duration::from_secs(1))),
        || panic!("a stopped daemon must not be killed"),
    )
    .await
    .unwrap();
    // Then
    assert_eq!(start.elapsed(), Duration::from_secs(1));
}

#[tokio::test(start_paused = true)]
async fn test_daemon停止_五秒後に一度強制終了して終了確認まで待つ() {
    // Given
    let start = tokio::time::Instant::now();
    let killed = Cell::new(false);
    // When
    wait_for_termination(
        || std::future::ready(Ok(start.elapsed() >= Duration::from_secs(6))),
        || {
            assert_eq!(start.elapsed(), Duration::from_secs(5));
            assert!(!killed.replace(true));
            Ok(())
        },
    )
    .await
    .unwrap();
    // Then
    assert!(killed.get());
    assert_eq!(start.elapsed(), Duration::from_secs(6));
}

#[tokio::test(start_paused = true)]
async fn test_daemon停止_強制終了後も終了未確認なら十秒で失敗する() {
    // Given
    let start = tokio::time::Instant::now();
    let kills = Cell::new(0);
    // When
    let error = wait_for_termination(
        || std::future::ready(Ok(false)),
        || {
            kills.set(kills.get() + 1);
            Ok(())
        },
    )
    .await
    .unwrap_err();
    // Then
    assert_eq!(
        error,
        "Daemon exit could not be confirmed after termination."
    );
    assert_eq!(start.elapsed(), Duration::from_secs(10));
    assert_eq!(kills.get(), 1);
}

#[tokio::test(start_paused = true)]
async fn test_daemon停止_終了観測とkillのエラーを伝播する() {
    // Given / When / Then
    assert_eq!(
        wait_for_termination(
            || std::future::ready(Err("wait failed".into())),
            || panic!("must not kill after an observation error"),
        )
        .await
        .unwrap_err(),
        "wait failed"
    );
    assert_eq!(
        wait_for_termination(
            || std::future::ready(Ok(false)),
            || Err("kill failed".into()),
        )
        .await
        .unwrap_err(),
        "kill failed"
    );
}

#[tokio::test]
async fn test_daemon停止_子がない場合は即座に完了する() {
    // Given
    let gateway = gateway(PathBuf::new());
    // When / Then
    gateway.terminate_and_wait().await.unwrap();
}

#[test]
fn test_停止応答_acceptedを受理し異なる応答を拒否する() {
    // Given
    use wire::{application_quit_outcome_dto_v1 as outcome, command_result::Command};
    let response = |variant| {
        Command::RequestApplicationQuit(wire::ApplicationQuitOutcomeDtoV1 {
            variant: Some(variant),
        })
    };
    // When / Then
    shutdown_response(response(outcome::Variant::Accepted(Default::default()))).unwrap();
    assert!(shutdown_response(Command::RequestApplicationQuit(
        wire::ApplicationQuitOutcomeDtoV1 { variant: None },
    ))
    .is_err());
    assert_eq!(
        shutdown_response(Command::UpdateExternalEditor(Default::default())).unwrap_err(),
        "Unexpected shutdown response."
    );
}
