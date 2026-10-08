use super::*;

#[test]
fn test_telemetry初期化_並行する設定適用でも一度だけ初期化しguardを保持する() {
    use releashd::desktop_api::test_support as telemetry;
    use std::sync::{atomic::Ordering, Arc, Barrier};
    // Given
    let _telemetry = telemetry::lock_test_telemetry();
    let _crash = telemetry::TEST_LOCK.lock().unwrap();
    let app = tauri::test::mock_builder()
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    app.manage(WindowPreferencesState(parking_lot::RwLock::new(
        WindowPreferences {
            close_to_tray: true,
        },
    )));
    let runtime = Arc::new(DesktopRuntime::new(app.handle().clone()));
    let barrier = Arc::new(Barrier::new(8));
    // When
    std::thread::scope(|threads| {
        for _ in 0..8 {
            let runtime = runtime.clone();
            let barrier = barrier.clone();
            threads.spawn(move || {
                barrier.wait();
                runtime.apply_settings(
                    WindowPreferences {
                        close_to_tray: false,
                    },
                    false,
                    false,
                );
            });
        }
    });
    // Then
    assert_eq!(runtime.telemetry_initializations.load(Ordering::SeqCst), 1);
    assert!(runtime.telemetry.get().is_some());
    assert!(!app.state::<WindowPreferencesState>().read().close_to_tray);
    let (provider, exporter) = telemetry::install_test_exporter(true, true);
    // When / Then
    runtime.apply_settings(
        WindowPreferences {
            close_to_tray: false,
        },
        false,
        false,
    );
    releashd::desktop_api::TelemetryGateway.report_frontend_error("test", "disabled", None);
    provider.force_flush().unwrap();
    assert!(exporter.get_emitted_logs().unwrap().is_empty());
    // When
    runtime.apply_settings(
        WindowPreferences {
            close_to_tray: true,
        },
        true,
        true,
    );
    // Then
    assert_eq!(runtime.telemetry_initializations.load(Ordering::SeqCst), 1);
    assert!(runtime.telemetry.get().is_some());
    assert!(app.state::<WindowPreferencesState>().read().close_to_tray);
    releashd::desktop_api::TelemetryGateway.report_frontend_error("test", "enabled", None);
    provider.force_flush().unwrap();
    assert_eq!(exporter.get_emitted_logs().unwrap().len(), 1);
    telemetry::reset_for_tests();
}

#[tokio::test]
async fn test_確認応答_承諾と却下を返し応答口の喪失を失敗として返す() {
    // Given / When / Then
    for confirmed in [true, false] {
        assert_eq!(
            confirmation(|reply| reply(confirmed)).await.unwrap(),
            confirmed
        );
    }
    assert!(confirmation(drop).await.is_err());
}
