use super::*;
use std::sync::atomic::Ordering;

#[tokio::test]
async fn test_接続監督_通信失敗は次の接続観測に反映する() {
    // Given
    let client = DesktopClient::start(
        super::client(&ClientConnectionDto {
            url: "http://127.0.0.1:1".into(),
            token: "client".into(),
        })
        .unwrap(),
    );
    // When
    tokio::time::timeout(std::time::Duration::from_secs(7), async {
        while client.connected() {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    // Then
    assert!(client.failure().is_some());
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
async fn test_接続監督_要求上限拒否が継続しても接続を維持する() {
    // Given
    let (endpoint, server, requests) = error_server(
        crate::adaptor::controller::api::protocol::connect::command_error_with_code(
            crate::other::AppError::coded(
                "CLIENT_REQUEST_LIMIT",
                "Too many pending client commands",
            )
            .into(),
            connectrpc::ErrorCode::ResourceExhausted,
        ),
    )
    .await;
    let client = DesktopClient::start(super::client(&endpoint).unwrap());
    // When / Then
    tokio::time::timeout(std::time::Duration::from_secs(45), async {
        while requests.load(Ordering::SeqCst) < 8 {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            assert!(client.connected());
            assert_eq!(client.failure(), None);
        }
    })
    .await
    .unwrap();
    assert!(client.connected());
    assert_eq!(client.failure(), None);
    server.abort();
}

#[tokio::test]
async fn test_ネイティブ要求_停止とログイン項目の具体的な失敗理由を保持する() {
    use crate::adaptor::controller::api::protocol::connect::command_error;
    // Given / When / Then
    for detail in [
        wire::CommandError::from("設定を保存できません".to_owned()),
        crate::other::AppError::coded("LOGIN_ITEM_SAVE_FAILED", "ログイン項目を保存できません")
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
        let (endpoint, server, _) = error_server(command_error(detail)).await;
        let client = DesktopClient::start(super::client(&endpoint).unwrap());
        for command in [
            wire::command_request::Command::RequestApplicationQuit(Default::default()),
            wire::command_request::Command::GetAppSettings(Default::default()),
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

#[tokio::test]
async fn test_接続監督_応答が無くなった場合は期限超過で接続を失う() {
    // Given
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = ClientConnectionDto {
        url: format!("http://{}", listener.local_addr().unwrap()),
        token: "client".into(),
    };
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new().fallback(axum::routing::post(|| async {
                std::future::pending::<axum::http::StatusCode>().await
            })),
        )
        .await
        .unwrap();
    });
    let client = DesktopClient::start(super::client(&endpoint).unwrap());
    // When
    tokio::time::timeout(std::time::Duration::from_secs(12), async {
        while client.connected() {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    // Then
    assert!(client.failure().is_some());
    server.abort();
}
