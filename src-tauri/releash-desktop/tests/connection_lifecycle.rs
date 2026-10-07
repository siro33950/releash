#![cfg(debug_assertions)]

use releash_desktop::test_support as host;
use releash_sdk::discovery::{process_start_time, LocalApiDiscovery};
use serde_json::json;
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use tauri::Manager;

static TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn invoke(
    window: &tauri::WebviewWindow<tauri::test::MockRuntime>,
    command: &str,
) -> Result<tauri::ipc::InvokeResponseBody, serde_json::Value> {
    tauri::test::get_ipc_response(
        window,
        tauri::webview::InvokeRequest {
            cmd: command.into(),
            callback: tauri::ipc::CallbackFn(0),
            error: tauri::ipc::CallbackFn(1),
            url: "tauri://localhost".parse().unwrap(),
            body: tauri::ipc::InvokeBody::Json(json!({})),
            headers: Default::default(),
            invoke_key: tauri::test::INVOKE_KEY.to_string(),
        },
    )
}

#[tokio::test]
async fn test_hidden起動_設定ファイルがあっても起動失敗では登録を触らず失敗窓を表示する() {
    // Given
    let _guard = TEST_LOCK.lock().await;
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("releash.toml"),
        "[app]\nauto_launch = true\nstart_minimized = true\n",
    )
    .unwrap();
    let path = directory.path().to_path_buf();
    // When
    let result = std::thread::spawn(move || {
        let app = host::desktop_connection_app(
            tauri::test::mock_builder(),
            &path,
            std::path::Path::new("/missing/releashd"),
        );
        let handle = app.handle().clone();
        tauri::async_runtime::spawn(async move {
            host::initialize_desktop(&handle, true).await;
        });
        app.run_return(|app, event| {
            if matches!(event, tauri::RunEvent::MainEventsCleared) {
                if let Some(failure) = app.get_webview_window("startup-failure") {
                    // Then
                    assert!(failure.is_visible().unwrap());
                    assert!(app.get_webview_window("main").is_none());
                    assert!(host::desktop_login_item_calls(app).is_empty());
                    assert_eq!(host::desktop_connection_status(app)["phase"], "failed");
                    failure.destroy().unwrap();
                }
            }
        })
    });
    tokio::task::spawn_blocking(move || result.join().unwrap())
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn test_古いサーバ入れ替え_停止失敗では起動せず停止完了後に設定を受信し通常窓へ復帰する() {
    // Given
    let _guard = TEST_LOCK.lock().await;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().canonicalize().unwrap();
    std::env::set_var("HOME", &path);
    std::env::set_var("XDG_CONFIG_HOME", path.join("config"));
    std::env::set_var("SHELL", "/bin/sh");
    let old = tokio::process::Command::new("/bin/sleep")
        .arg("120")
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let pid = old.id().unwrap();
    let old = Arc::new(tokio::sync::Mutex::new(old));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let discovery = LocalApiDiscovery {
        port: listener.local_addr().unwrap().port(),
        token: "old".into(),
        instance_id: "old-server".into(),
        pid,
        process_started_at: process_start_time(pid).unwrap(),
    };
    let discovery_path = path.join("client-api.json");
    std::fs::write(&discovery_path, serde_json::to_vec(&discovery).unwrap()).unwrap();
    let info = prost::Message::encode_to_vec(&releash_sdk::wire::ServerInfo {
        daemon_id: discovery.instance_id.clone(),
        pid,
        process_started_at: discovery.process_started_at,
        protocol: 0,
        release: "old-release".into(),
        ..Default::default()
    });
    let reject = Arc::new(AtomicBool::new(true));
    let rejecting = reject.clone();
    let router = axum::Router::new()
        .route(
            "/releash.client.v1.ClientService/GetServerInfo",
            axum::routing::post(move || {
                let info = info.clone();
                async move { ([("content-type", "application/proto")], info) }
            }),
        )
        .route(
            "/releash.client.v1.ClientService/StopDaemon",
            axum::routing::post(move || {
                let old = old.clone();
                let path = discovery_path.clone();
                let rejecting = rejecting.clone();
                async move {
                    if rejecting.load(Ordering::SeqCst) {
                        return (
                            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                            [("content-type", "application/json")],
                            "{\"code\":\"internal\",\"message\":\"stop refused\"}"
                                .as_bytes()
                                .to_vec(),
                        );
                    }
                    let mut process = old.lock().await;
                    process.kill().await.unwrap();
                    process.wait().await.unwrap();
                    std::fs::remove_file(path).unwrap();
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
    let app = host::desktop_connection_app(
        tauri::test::mock_builder(),
        &path,
        &support::backend_executable(),
    );
    tokio::time::timeout(Duration::from_secs(3), async {
        while host::desktop_connection_status(app.handle())["phase"] != "failed" {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let failure = tauri::WebviewWindowBuilder::new(&app, "startup-failure", Default::default())
        .build()
        .unwrap();
    // When / Then
    assert!(invoke(&failure, "replace_daemon").is_err());
    assert_eq!(
        releash_sdk::discovery::read_optional(&path).unwrap(),
        Some(discovery)
    );
    assert_eq!(unsafe { libc::kill(pid as i32, 0) }, 0);
    assert!(app.get_webview_window("main").is_none());
    // When
    reject.store(false, Ordering::SeqCst);
    invoke(&failure, "replace_daemon").unwrap();
    // Then
    let current = releash_sdk::discovery::read_optional(&path)
        .unwrap()
        .unwrap();
    assert_ne!(current.pid, pid);
    assert_eq!(unsafe { libc::kill(pid as i32, 0) }, -1);
    assert_eq!(
        host::desktop_connection_status(app.handle())["phase"],
        "ready"
    );
    assert!(!host::desktop_login_item_calls(app.handle()).is_empty());
    let handle = app.handle().clone();
    let finished = Arc::new(AtomicBool::new(false));
    let observed = finished.clone();
    app.run_return(move |app, event| {
        if matches!(event, tauri::RunEvent::MainEventsCleared) {
            if let Some(main) = app.get_webview_window("main") {
                if !observed.swap(true, Ordering::SeqCst) {
                    assert!(main.is_visible().unwrap());
                    main.destroy().unwrap();
                }
            }
        }
    });
    assert!(finished.load(Ordering::SeqCst));
    host::stop_desktop_daemon(&handle).await;
    server.abort();
}

mod support;

#[tokio::test]
async fn test_初回設定適用_通知のobserverが無くても適用しhidden接続待機中の窓を置き換える() {
    use futures_util::StreamExt;
    use releash_desktop::test_support::integration::settings_observer::observe;
    use releashd::desktop_api::test_support as telemetry;
    // Given
    let _guard = TEST_LOCK.lock().await;
    let _telemetry = telemetry::lock_test_telemetry();
    let _crash = telemetry::TEST_LOCK.lock().unwrap();
    for hidden in [false, true] {
        telemetry::reset_test_metrics();
        releashd::desktop_api::set_startup_origin(std::time::Instant::now());
        let directory = tempfile::tempdir().unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let discovery = LocalApiDiscovery {
            port: listener.local_addr().unwrap().port(),
            token: "settings-test".into(),
            instance_id: "settings-test".into(),
            pid: std::process::id(),
            process_started_at: process_start_time(std::process::id()).unwrap(),
        };
        std::fs::write(
            directory.path().join("client-api.json"),
            serde_json::to_vec(&discovery).unwrap(),
        )
        .unwrap();
        let info = prost::Message::encode_to_vec(&releash_sdk::wire::ServerInfo {
            daemon_id: discovery.instance_id,
            pid: discovery.pid,
            process_started_at: discovery.process_started_at,
            protocol: 1,
            release: "settings-test".into(),
            ..Default::default()
        });
        let frames: Vec<_> = [
            releash_sdk::wire::state_subscription_event::Event::Ready(releash_sdk::wire::Unit {}),
            releash_sdk::wire::state_subscription_event::Event::Snapshot(
                releash_sdk::wire::StatePayload {
                    value: Some(releash_sdk::wire::state_payload::Value::DesktopSettings(
                        releash_sdk::wire::DesktopSettings {
                            close_to_tray: Some(false),
                            start_minimized: Some(true),
                            crash_reporting: Some(false),
                            performance_telemetry: Some(true),
                            auto_launch: Some(true),
                        },
                    )),
                },
            ),
        ]
        .into_iter()
        .map(|event| {
            let payload =
                prost::Message::encode_to_vec(&releash_sdk::wire::StateSubscriptionEvent {
                    event: Some(event),
                    ..Default::default()
                });
            let mut frame = vec![0];
            frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
            frame.extend_from_slice(&payload);
            frame
        })
        .collect();
        let release_settings = Arc::new(tokio::sync::Notify::new());
        let release = release_settings.clone();
        let (release_change, changed) = tokio::sync::watch::channel(());
        let payload = prost::Message::encode_to_vec(&releash_sdk::wire::StateSubscriptionEvent {
            event: Some(
                releash_sdk::wire::state_subscription_event::Event::Snapshot(
                    releash_sdk::wire::StatePayload {
                        value: Some(releash_sdk::wire::state_payload::Value::DesktopSettings(
                            releash_sdk::wire::DesktopSettings {
                                close_to_tray: Some(true),
                                start_minimized: Some(false),
                                crash_reporting: Some(false),
                                performance_telemetry: Some(true),
                                auto_launch: Some(false),
                            },
                        )),
                    },
                ),
            ),
            ..Default::default()
        });
        let mut changed_frame = vec![0];
        changed_frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        changed_frame.extend_from_slice(&payload);
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
                    let frames = frames.clone();
                    let release = release.clone();
                    let mut changed = changed.clone();
                    let changed_frame = changed_frame.clone();
                    async move {
                        let stream = futures_util::stream::once(async move {
                            release.notified().await;
                            Ok::<_, std::convert::Infallible>(frames.concat())
                        })
                        .chain(futures_util::stream::once(async move {
                            changed.changed().await.unwrap();
                            Ok::<_, std::convert::Infallible>(changed_frame)
                        }))
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
                axum::routing::post(|| async {
                    ([("content-type", "application/proto")], Vec::<u8>::new())
                }),
            );
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let (app, mut updates) = host::desktop_connection_app_parts(
            tauri::test::mock_builder(),
            directory.path(),
            std::path::Path::new("/missing/releashd"),
        );
        let handle = app.handle().clone();
        let mut initialized = Box::pin(host::initialize_desktop_with_settings_applied(
            &handle, hidden,
        ));
        // When / Then
        assert!(
            tokio::time::timeout(Duration::from_millis(30), &mut initialized)
                .await
                .is_err()
        );
        assert!(updates.borrow().is_none());
        assert!(app.get_webview_window("main").is_none());
        assert!(host::desktop_login_item_calls(app.handle()).is_empty());
        if hidden {
            host::show_desktop(app.handle()).unwrap();
            assert!(app.get_webview_window("startup-failure").is_some());
        }
        release_settings.notify_one();
        tokio::time::timeout(Duration::from_secs(3), &mut initialized)
            .await
            .unwrap();
        drop(initialized);
        let client = updates.borrow_and_update().clone().unwrap();
        assert!(!host::desktop_window_preferences(app.handle()));
        assert_eq!(
            host::desktop_login_item_calls(app.handle())
                .iter()
                .filter(|call| **call == "register")
                .count(),
            1
        );
        assert!(!telemetry::crash_reporting_enabled());
        assert_eq!(
            telemetry::performance_telemetry_active(),
            telemetry::performance_telemetry_configured()
        );
        // Given
        let (entered, entered_wait) = tokio::sync::oneshot::channel();
        let (resume, resume_wait) = tokio::sync::oneshot::channel();
        let handle = app.handle().clone();
        let serialized = tokio::spawn(async move {
            handle
                .state::<releash_desktop::test_support::integration::settings_observer::Serial>()
                .call(async {
                    entered.send(()).unwrap();
                    resume_wait.await.unwrap();
                })
                .await;
        });
        entered_wait.await.unwrap();
        let handle = app.handle().clone();
        let stale_client = client.clone();
        let mut stale = tokio::spawn(async move {
            host::apply_observed_desktop_settings(
                &handle,
                stale_client,
                releashd::desktop_api::DesktopSettingsDto {
                    close_to_tray: true,
                    start_minimized: false,
                    crash_reporting: true,
                    performance_telemetry: false,
                    auto_launch: false,
                },
            )
            .await;
        });
        assert!(tokio::time::timeout(Duration::from_millis(30), &mut stale)
            .await
            .is_err());
        use releash_desktop::test_support::integration::daemon_connection::DaemonService;
        let gateway = app.state::<Arc<
            releash_desktop::test_support::integration::daemon_connection::DaemonServiceGateway,
        >>();
        release_settings.notify_one();
        // When
        gateway
            .connect(
                &releash_desktop::test_support::integration::daemon_connection::DaemonEndpoint {
                    url: format!("http://127.0.0.1:{}", discovery.port),
                    token: discovery.token.clone(),
                },
            )
            .await
            .unwrap();
        resume.send(()).unwrap();
        serialized.await.unwrap();
        stale.await.unwrap();
        // Then
        assert!(!host::desktop_window_preferences(app.handle()));
        assert!(!telemetry::crash_reporting_enabled());
        assert!(!host::desktop_login_item_calls(app.handle()).contains(&"unregister"));
        drop(gateway);
        drop(client);
        let client = updates.borrow_and_update().clone().unwrap();
        release_change.send_replace(());
        let mut settings = client.settings_receiver();
        tokio::time::timeout(
            Duration::from_secs(3),
            settings.wait_for(|value| value.is_some_and(|settings| !settings.auto_launch)),
        )
        .await
        .unwrap()
        .unwrap();
        let handle = app.handle().clone();
        let observer = tokio::spawn(observe(
            updates,
            |client| (client.settings_receiver(), client.initial_settings()),
            move |client, settings| {
                let client = client.clone();
                let handle = handle.clone();
                async move {
                    host::apply_observed_desktop_settings(&handle, client, settings).await;
                }
            },
        ));
        tokio::time::timeout(Duration::from_secs(3), async {
            while !host::desktop_window_preferences(app.handle()) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            host::desktop_login_item_calls(app.handle())
                .iter()
                .filter(|call| **call == "unregister")
                .count(),
            1
        );
        assert_eq!(
            host::desktop_login_item_calls(app.handle())
                .iter()
                .filter(|call| **call == "register")
                .count(),
            1
        );
        let visible = Arc::new(AtomicBool::new(false));
        let observed = visible.clone();
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        app.run_return(move |app, event| {
            if matches!(event, tauri::RunEvent::MainEventsCleared) {
                assert!(
                    std::time::Instant::now() < deadline,
                    "通常窓への置き換えが完了しませんでした"
                );
                if let Some(main) = app.get_webview_window("main") {
                    if !observed.swap(true, Ordering::SeqCst) {
                        assert!(main.is_visible().unwrap());
                        main.destroy().unwrap();
                    }
                }
            }
        });
        assert!(visible.load(Ordering::SeqCst));
        // Then
        let records = telemetry::test_metric_records();
        let operations: Vec<_> = records
            .iter()
            .flat_map(|record| &record.attributes)
            .filter(|(key, _)| key == "releash.operation")
            .map(|(_, value)| value.as_str())
            .collect();
        if telemetry::performance_telemetry_configured() {
            assert!(operations.contains(&"startup.app"));
            assert!(operations.contains(&"startup.first_window_ready"));
        } else {
            assert!(operations.is_empty());
        }
        if !hidden {
            use releash_desktop::test_support::integration::daemon_connection::{
                DaemonEndpoint, DaemonService, DaemonServiceGateway, RetryLimiter,
            };
            let (clients, mut updates) = tokio::sync::watch::channel(None);
            let gateway = Arc::new(DaemonServiceGateway::new(
                "/missing/releashd".into(),
                directory.path().into(),
                Arc::new(RetryLimiter::new()),
                releash_desktop::test_support::integration::daemon_connection::Deadline(
                    releash_sdk::daemon::timeout("min_connect_timeout_ms"),
                ),
                clients,
            ));
            let endpoint = DaemonEndpoint {
                url: format!("http://127.0.0.1:{}", discovery.port),
                token: discovery.token,
            };
            // When / Then
            release_settings.notify_one();
            gateway.connect(&endpoint).await.unwrap();
            assert!(gateway.client().is_ok());
            assert!(updates.borrow_and_update().is_some());
        }
        observer.abort();
        drop(client);
        server.abort();
    }
    telemetry::reset_test_metrics();
    telemetry::reset_for_tests();
}
