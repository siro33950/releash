use super::*;
use crate::adaptor::gateway::failure_records::FailureRecordStore;
use futures_util::StreamExt;
use std::sync::atomic::Ordering;

const KEY_TARGET: &str = "launch";

fn start(endpoint: &ClientConnectionDto) -> (DesktopClient, Arc<FailureRecordStore>) {
    let store = Arc::new(FailureRecordStore::default());
    let client = DesktopClient::start(
        super::client(endpoint).unwrap(),
        super::stream_client(endpoint).unwrap(),
        FailureKey::new("daemon_liveness", KEY_TARGET),
        Arc::new(FailureRecordingUsecase::new(store.clone(), None)),
        Arc::new(RetryLimiter::deterministic()),
    );
    (client, store)
}

async fn wait_for_disconnect(client: &DesktopClient, limit: std::time::Duration) {
    tokio::time::timeout(limit, async {
        while client.connected() {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
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

fn ready_event() -> rpc::StateSubscriptionEvent {
    to_rpc(&wire::StateSubscriptionEvent {
        event: Some(wire::state_subscription_event::Event::Ready(wire::Unit {})),
        ..Default::default()
    })
    .unwrap()
}

async fn stream_server(
    events: Vec<rpc::StateSubscriptionEvent>,
    hold: bool,
) -> (ClientConnectionDto, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = ClientConnectionDto {
        url: format!("http://{}", listener.local_addr().unwrap()),
        token: "client".into(),
    };
    let mut frames: Vec<Vec<u8>> = events.into_iter().map(envelope).collect();
    if !hold {
        frames.push(vec![2, 0, 0, 0, 2, b'{', b'}']);
    }
    let router = axum::Router::new().route(
        "/releash.client.v1.ClientService/OpenStateStream",
        axum::routing::post(move || {
            let frames = frames.clone();
            async move {
                let body = futures_util::stream::iter(
                    frames.into_iter().map(Ok::<_, std::convert::Infallible>),
                );
                let body: std::pin::Pin<
                    Box<
                        dyn futures_util::Stream<Item = Result<Vec<u8>, std::convert::Infallible>>
                            + Send,
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
    );
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (endpoint, server)
}

#[tokio::test]
async fn test_生存確認_接続拒否が3回続いたときだけ接続を失い分類を記録する() {
    // Given
    let endpoint = ClientConnectionDto {
        url: "http://127.0.0.1:1".into(),
        token: "client".into(),
    };
    let (client, store) = start(&endpoint);
    // When
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    // Then
    assert!(client.connected());
    assert_eq!(client.failure(), None);
    let records = store.records(KEY_TARGET);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].record.count, 1);
    // When
    wait_for_disconnect(&client, std::time::Duration::from_secs(10)).await;
    // Then
    let failure = client.failure().unwrap();
    assert_eq!(
        failure.kind,
        Failure::Technical(TechnicalFailureNature::Transient)
    );
    let records = store.records(KEY_TARGET);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].record.count, 3);
    assert_eq!(records[0].record.operation, "daemon_liveness");
    assert_eq!(records[0].record.kind, failure.kind);
    assert!(records[0].record.active);
}

#[tokio::test]
async fn test_生存確認_streamが届く間は接続を維持し記録しない() {
    // Given
    let (endpoint, server) = stream_server(vec![ready_event()], true).await;
    let (client, store) = start(&endpoint);
    // When
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    // Then
    assert!(client.connected());
    assert_eq!(client.failure(), None);
    assert!(store.records(KEY_TARGET).is_empty());
    server.abort();
}

#[tokio::test]
async fn test_生存確認_streamの終了が3回続くと一時的な失敗として接続を失う() {
    // Given
    let (endpoint, server) = stream_server(vec![], false).await;
    let (client, store) = start(&endpoint);
    // When
    wait_for_disconnect(&client, std::time::Duration::from_secs(10)).await;
    // Then
    let failure = client.failure().unwrap();
    assert_eq!(
        failure.kind,
        Failure::Technical(TechnicalFailureNature::Transient)
    );
    assert_eq!(failure.message, "State stream ended");
    let records = store.records(KEY_TARGET);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].record.count, 3);
    server.abort();
}

#[derive(Default)]
struct RecordedCalls(parking_lot::Mutex<Vec<&'static str>>);

impl crate::domain::failure::FailureRecordRepository for RecordedCalls {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn record_observed(
        &self,
        _key: &FailureKey,
        _failure: WorkFailure,
        _requires_attention: bool,
    ) -> bool {
        self.0.lock().push("observed");
        false
    }
    fn record_resolved(&self, _key: &FailureKey) -> bool {
        self.0.lock().push("resolved");
        false
    }
}

#[tokio::test]
async fn test_生存確認_失敗の後に届けば連続失敗が消え記録を解消する() {
    // Given
    let (endpoint, server) = stream_server(vec![ready_event()], false).await;
    let calls = Arc::new(RecordedCalls::default());
    let client = DesktopClient::start(
        super::client(&endpoint).unwrap(),
        super::stream_client(&endpoint).unwrap(),
        FailureKey::new("daemon_liveness", KEY_TARGET),
        Arc::new(FailureRecordingUsecase::new(calls.clone(), None)),
        Arc::new(RetryLimiter::deterministic()),
    );
    // When
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    // Then
    assert!(client.connected());
    assert_eq!(client.failure(), None);
    let calls = calls.0.lock().clone();
    assert!(calls.len() >= 4, "{calls:?}");
    assert_eq!(calls[0], "observed");
    assert_eq!(calls[1], "resolved");
    assert_eq!(calls[2], "observed");
    server.abort();
}

#[tokio::test(start_paused = true)]
async fn test_生存確認_応答が無い期限超過が3回続くと接続を失う() {
    // Given
    let (endpoint, server) = stream_server(vec![], true).await;
    let (client, store) = start(&endpoint);
    // When
    wait_for_disconnect(&client, std::time::Duration::from_secs(120)).await;
    // Then
    let failure = client.failure().unwrap();
    assert_eq!(
        failure.kind,
        Failure::Technical(TechnicalFailureNature::TimedOut)
    );
    let records = store.records(KEY_TARGET);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].record.count, 3);
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
        (ErrorCode::Canceled, TechnicalFailureNature::Cancelled),
        (ErrorCode::Internal, TechnicalFailureNature::Other),
        (ErrorCode::Unauthenticated, TechnicalFailureNature::Other),
    ] {
        let failure = liveness_failure(connectrpc::ConnectError::new(code, "reason"));
        assert_eq!(failure.kind, Failure::Technical(nature));
        assert_eq!(
            failure.message,
            connectrpc::ConnectError::new(code, "reason").to_string()
        );
    }
}

async fn error_server(
    error: connectrpc::ConnectError,
) -> (
    ClientConnectionDto,
    tokio::task::JoinHandle<()>,
    Arc<std::sync::atomic::AtomicUsize>,
) {
    let requests = Arc::new(std::sync::atomic::AtomicUsize::new(0));
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
            kind: connectrpc::ErrorCode::Internal,
            detail,
        }))
        .await;
        let (client, _) = start(&endpoint);
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
