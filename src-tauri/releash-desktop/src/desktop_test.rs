use super::*;

#[test]
fn test_desktop観測_初回windowとappの起動時間を設定に従って記録する() {
    use releashd::desktop_api::set_startup_origin;
    use releashd::desktop_api::test_support as telemetry;
    // Given
    let _guard = telemetry::lock_test_telemetry();
    telemetry::reset_test_metrics();
    set_startup_origin(std::time::Instant::now());
    telemetry::set_performance_configured(true);
    telemetry::set_performance_enabled(true);
    // When
    record_window_ready();
    // Then
    let records = telemetry::test_metric_records();
    let operations: Vec<_> = records
        .iter()
        .flat_map(|record| &record.attributes)
        .filter(|(key, _)| key == "releash.operation")
        .map(|(_, value)| value.as_str())
        .collect();
    assert!(operations.contains(&"startup.app"));
    assert!(operations.contains(&"startup.first_window_ready"));
    let count = records.len();
    telemetry::set_performance_enabled(false);
    record_window_ready();
    assert_eq!(telemetry::test_metric_records().len(), count);
    telemetry::reset_test_metrics();
}

use crate::infrastructure::platform::window_lifecycle::WindowPreferencesState;
#[test]
fn test_desktop設定_daemonの設定だけを保持して閉じる操作へ渡す() {
    use releashd::desktop_api::DesktopSettingsDto;
    use tauri::Manager;
    // Given
    let _telemetry = releashd::desktop_api::test_support::lock_test_telemetry();
    let _crash = releashd::desktop_api::test_support::TEST_LOCK
        .lock()
        .unwrap();
    let app = tauri::test::mock_builder()
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    let mut settings = DesktopSettingsDto {
        close_to_tray: false,
        start_minimized: true,
        crash_reporting: false,
        performance_telemetry: false,
        auto_launch: false,
    };
    // When
    apply_desktop_settings(app.handle(), settings);
    // Then
    let preferences = app.state::<WindowPreferencesState>().read();
    assert!(!preferences.close_to_tray);
    // When
    settings.close_to_tray = true;
    apply_desktop_settings(app.handle(), settings);
    // Then
    assert!(app.state::<WindowPreferencesState>().read().close_to_tray);
}

#[test]
fn test_desktop設定_クラッシュ送信の無効化と再有効化を再起動なしで反映する() {
    use releashd::desktop_api::test_support::{install_test_exporter, TEST_LOCK};
    use releashd::desktop_api::DesktopSettingsDto;
    // Given
    let _telemetry = releashd::desktop_api::test_support::lock_test_telemetry();
    let _guard = TEST_LOCK.lock().unwrap();
    let (provider, exporter) = install_test_exporter(true, true);
    let app = tauri::test::mock_builder()
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    let mut settings = DesktopSettingsDto {
        close_to_tray: true,
        start_minimized: false,
        crash_reporting: false,
        performance_telemetry: false,
        auto_launch: false,
    };
    // When / Then
    apply_desktop_settings(app.handle(), settings);
    assert!(std::panic::catch_unwind(|| panic!("disabled desktop panic")).is_err());
    provider.force_flush().unwrap();
    assert!(exporter.get_emitted_logs().unwrap().is_empty());
    settings.crash_reporting = true;
    apply_desktop_settings(app.handle(), settings);
    assert!(std::panic::catch_unwind(|| panic!("enabled desktop panic")).is_err());
    provider.force_flush().unwrap();
    assert_eq!(exporter.get_emitted_logs().unwrap().len(), 1);
    releashd::desktop_api::test_support::reset_for_tests();
}
