use releash::discovery::{process_start_time, LocalApiDiscovery};
use releash_desktop::test_support::integration::daemon_connection::{
    failure, message, DaemonConnectionUsecase, DaemonServiceGateway, RetryLimiter,
};
use std::sync::Arc;

#[tokio::test]
async fn test_接続_既存サーバのprotocolと同一性を確認し非互換なら版を返す() {
    // Given
    for (protocol, expected) in [(0, "サーバが古い"), (2, "画面が古い")] {
        let directory = tempfile::tempdir().unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let discovery = LocalApiDiscovery {
            port: listener.local_addr().unwrap().port().into(),
            token: "test".into(),
            daemon_id: "test-server".into(),
            pid: std::process::id(),
            process_started_at: process_start_time(std::process::id()).unwrap(),
            ..Default::default()
        };
        std::fs::write(
            directory.path().join("client-api.json"),
            serde_json::to_vec(&discovery).unwrap(),
        )
        .unwrap();
        let info = releash::wire::ServerInfo {
            daemon_id: discovery.daemon_id.clone(),
            pid: discovery.pid,
            process_started_at: discovery.process_started_at,
            protocol,
            release: "server-release".into(),
            ..Default::default()
        };
        let info = prost::Message::encode_to_vec(&info);
        let router = axum::Router::new().route(
            "/releash.client.v1.ClientService/GetServerInfo",
            axum::routing::post(move || {
                let info = info.clone();
                async move { ([("content-type", "application/proto")], info) }
            }),
        );
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let connection = Arc::new(DaemonServiceGateway::new(
            "/missing/releashd".into(),
            directory.path().into(),
            Arc::new(RetryLimiter::new()),
            releash_desktop::test_support::integration::daemon_connection::Deadline(
                releash::daemon::timeout("min_connect_timeout_ms"),
            ),
            tokio::sync::watch::channel(None).0,
        ));
        let usecase = DaemonConnectionUsecase::new(
            connection.clone(),
            connection.clone(),
            releash::descriptor::protocol(),
            env!("CARGO_PKG_VERSION").into(),
        );
        // When
        let error = message(usecase.connect().await.unwrap_err());
        // Then
        assert!(error.contains(expected), "{error}");
        assert!(error.contains("server-release"));
        assert!(error.contains(env!("CARGO_PKG_VERSION")));
        assert_eq!(
            releash::daemon::running(directory.path()).unwrap(),
            Some(discovery)
        );
        assert!(connection.client().is_err());
        server.abort();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_初回設定失敗_server_infoが成功しても接続先を渡さず設定適用や通常窓へ進まない() {
    use futures_util::StreamExt;
    use releash_desktop::test_support as host;
    use tauri::Manager;
    // Given
    for reject in [true, false] {
        let directory = tempfile::tempdir().unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let discovery = LocalApiDiscovery {
            port: listener.local_addr().unwrap().port().into(),
            token: "test".into(),
            daemon_id: "settings-failure".into(),
            pid: std::process::id(),
            process_started_at: process_start_time(std::process::id()).unwrap(),
            ..Default::default()
        };
        std::fs::write(
            directory.path().join("client-api.json"),
            serde_json::to_vec(&discovery).unwrap(),
        )
        .unwrap();
        let info = prost::Message::encode_to_vec(&releash::wire::ServerInfo {
            daemon_id: discovery.daemon_id,
            pid: discovery.pid,
            process_started_at: discovery.process_started_at,
            protocol: 1,
            release: "compatible".into(),
            ..Default::default()
        });
        let ready = prost::Message::encode_to_vec(&releash::wire::StateSubscriptionEvent {
            event: Some(releash::wire::state_subscription_event::Event::Ready(
                releash::wire::Unit {},
            )),
            ..Default::default()
        });
        let mut frame = vec![0];
        frame.extend_from_slice(&(ready.len() as u32).to_be_bytes());
        frame.extend_from_slice(&ready);
        let router = axum::Router::new()
            .route(
                "/releash.client.v1.ClientService/GetServerInfo",
                axum::routing::post(move || {
                    let info = info.clone();
                    async move { ([("content-type", "application/proto")], info) }
                }),
            )
            .route(
                "/releash.client.v1.ClientService/OpenStateStream",
                axum::routing::post(move || {
                    let frame = frame.clone();
                    async move {
                        let stream = futures_util::stream::once(async {
                            Ok::<_, std::convert::Infallible>(frame)
                        })
                        .chain(futures_util::stream::pending());
                        (
                            [("content-type", "application/connect+proto")],
                            axum::body::Body::from_stream(stream),
                        )
                    }
                }),
            )
            .route(
                "/releash.client.v1.ClientService/StartStateSubscription",
                axum::routing::post(move || async move {
                    if reject {
                        (
                            axum::http::StatusCode::FORBIDDEN,
                            [("content-type", "application/json")],
                            b"{\"code\":\"permission_denied\",\"message\":\"settings denied\"}"
                                .to_vec(),
                        )
                    } else {
                        (
                            axum::http::StatusCode::OK,
                            [("content-type", "application/proto")],
                            Vec::new(),
                        )
                    }
                }),
            );
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let (clients, updates) = tokio::sync::watch::channel(None);
        let gateway = DaemonServiceGateway::new(
            "/missing/releashd".into(),
            directory.path().into(),
            Arc::new(RetryLimiter::new()),
            releash_desktop::test_support::integration::daemon_connection::Deadline(
                std::time::Duration::from_millis(100),
            ),
            clients,
        );
        use releash_desktop::test_support::integration::daemon_connection::{
            DaemonEndpoint, DaemonService,
        };
        // When / Then
        assert!(gateway
            .connect(&DaemonEndpoint {
                url: format!("http://127.0.0.1:{}", discovery.port),
                token: discovery.token,
            })
            .await
            .is_err());
        assert!(gateway.client().is_err());
        assert!(updates.borrow().is_none());
        let app = host::desktop_connection_app(
            tauri::test::mock_builder(),
            directory.path(),
            std::path::Path::new("/missing/releashd"),
        );
        let window = tauri::WebviewWindowBuilder::new(&app, "startup-failure", Default::default())
            .build()
            .unwrap();
        // When
        let result = tauri::test::get_ipc_response(
            &window,
            tauri::webview::InvokeRequest {
                cmd: "get_client_endpoint".into(),
                callback: tauri::ipc::CallbackFn(0),
                error: tauri::ipc::CallbackFn(1),
                url: "tauri://localhost".parse().unwrap(),
                body: tauri::ipc::InvokeBody::Json(serde_json::json!({})),
                headers: Default::default(),
                invoke_key: tauri::test::INVOKE_KEY.to_string(),
            },
        );
        // Then
        let message = result.err().unwrap().to_string();
        assert!(
            message.contains(if reject {
                "settings denied"
            } else {
                "サーバの初回設定を受信できませんでした"
            }),
            "{message}"
        );
        assert_eq!(
            host::desktop_connection_status(app.handle())["phase"],
            "failed"
        );
        assert!(host::desktop_login_item_calls(app.handle()).is_empty());
        assert!(app.get_webview_window("main").is_none());
        server.abort();
    }
}

#[tokio::test]
async fn test_起動失敗_接続先を再要求しても終了状態とstderrを表示する() {
    use std::os::unix::fs::PermissionsExt;
    // Given
    let directory = tempfile::tempdir().unwrap();
    let executable = directory.path().join("releashd");
    std::fs::write(
        &executable,
        "#!/bin/sh\nprintf 'desktop-startup-marker\\n' >&2\nexit 7\n",
    )
    .unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
    let gateway = Arc::new(DaemonServiceGateway::new(
        executable,
        directory.path().into(),
        Arc::new(RetryLimiter::new()),
        releash_desktop::test_support::integration::daemon_connection::Deadline(
            releash::daemon::timeout("min_connect_timeout_ms"),
        ),
        tokio::sync::watch::channel(None).0,
    ));
    let connection = DaemonConnectionUsecase::new(
        gateway.clone(),
        gateway,
        releash::descriptor::protocol(),
        env!("CARGO_PKG_VERSION").into(),
    );
    // When
    assert!(connection.connect().await.is_err());
    assert!(connection.endpoint().await.is_err());
    let presented = failure(connection.failure().unwrap());
    // Then
    assert!(
        presented.message.contains("プロセスが終了しました"),
        "{}",
        presented.message
    );
    assert!(presented.message.contains('7'), "{}", presented.message);
    assert!(
        presented.message.contains("desktop-startup-marker"),
        "{}",
        presented.message
    );
}
