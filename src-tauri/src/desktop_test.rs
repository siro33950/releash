use crate::infrastructure::platform::window_lifecycle::{
    NORMAL_WINDOW_LABEL, STARTUP_FAILURE_WINDOW_LABEL,
};
#[test]
fn b071_pre_admission_window_grants_no_plugin_ipc_capability() {
    let config: serde_json::Value =
        serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
    assert_eq!(config["app"]["windows"][0]["label"], NORMAL_WINDOW_LABEL);
    assert_eq!(config["app"]["windows"][0]["create"], false);

    fn capability_files(root: &std::path::Path, output: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(root).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                capability_files(&path, output);
            } else if path.extension().and_then(|value| value.to_str()) == Some("json") {
                output.push(path);
            }
        }
    }

    let mut automatically_loaded = Vec::new();
    capability_files(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("capabilities"),
        &mut automatically_loaded,
    );
    assert!(!automatically_loaded.is_empty());
    let mut startup_failure_capability_seen = false;
    let mut normal_capability = None;
    for path in automatically_loaded {
        let capability: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let windows = capability["windows"]
            .as_array()
            .expect("capability windows");
        let permissions = capability["permissions"]
            .as_array()
            .expect("capability permissions");
        if !permissions.is_empty() {
            assert_eq!(
                capability["windows"],
                serde_json::json!([NORMAL_WINDOW_LABEL]),
                "a plugin-capable capability can target only the post-admission window: {}",
                path.display()
            );
        }
        if windows
            .iter()
            .any(|window| window == STARTUP_FAILURE_WINDOW_LABEL)
        {
            startup_failure_capability_seen = true;
            assert_eq!(
                capability["permissions"],
                serde_json::json!([]),
                "startup-failure window received plugin IPC permissions from {}",
                path.display()
            );
        }
        if windows.iter().any(|window| window == NORMAL_WINDOW_LABEL) {
            normal_capability = Some(capability);
        }
    }
    assert!(startup_failure_capability_seen);

    let normal = normal_capability.expect("normal workbench capability");
    assert_eq!(normal["windows"], serde_json::json!(["main"]));
    let permissions = normal["permissions"]
        .as_array()
        .expect("normal workbench capability permissions");
    for plugin in ["fs:default"] {
        assert!(
            permissions.iter().any(|permission| permission == plugin),
            "Ready-only capability lost {plugin}"
        );
    }
}

#[test]
fn test_desktop設定_daemonの設定だけを保持して閉じる操作へ渡す() {
    use crate::infrastructure::platform::window_lifecycle::WindowPreferencesState;
    use crate::usecase::app_config::query_service::DesktopSettingsDto;
    use tauri::Manager;
    // Given
    let _telemetry = crate::infrastructure::telemetry::metrics::lock_test_telemetry();
    let _crash = crate::infrastructure::telemetry::crash::tests::TEST_LOCK
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
    };
    // When
    super::apply_desktop_settings(app.handle(), settings);
    // Then
    let preferences = app.state::<WindowPreferencesState>().read();
    assert!(!preferences.close_to_tray);
    // When
    settings.close_to_tray = true;
    super::apply_desktop_settings(app.handle(), settings);
    // Then
    assert!(app.state::<WindowPreferencesState>().read().close_to_tray);
}

#[test]
fn test_desktop設定_クラッシュ送信の無効化と再有効化を再起動なしで反映する() {
    use crate::infrastructure::telemetry::crash::tests::{install_test_exporter, TEST_LOCK};
    use crate::usecase::app_config::query_service::DesktopSettingsDto;
    // Given
    let _telemetry = crate::infrastructure::telemetry::metrics::lock_test_telemetry();
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
    };
    // When / Then
    super::apply_desktop_settings(app.handle(), settings);
    assert!(std::panic::catch_unwind(|| panic!("disabled desktop panic")).is_err());
    provider.force_flush().unwrap();
    assert!(exporter.get_emitted_logs().unwrap().is_empty());
    settings.crash_reporting = true;
    super::apply_desktop_settings(app.handle(), settings);
    assert!(std::panic::catch_unwind(|| panic!("enabled desktop panic")).is_err());
    provider.force_flush().unwrap();
    assert_eq!(exporter.get_emitted_logs().unwrap().len(), 1);
    crate::infrastructure::telemetry::crash::reset_for_tests();
}

#[test]
fn test_desktop観測_初回windowとappの起動時間を設定に従って記録する() {
    use crate::infrastructure::telemetry::metrics as telemetry;
    // Given
    let _guard = telemetry::lock_test_telemetry();
    telemetry::reset_test_metrics();
    telemetry::set_startup_origin(std::time::Instant::now());
    telemetry::set_performance_configured(true);
    telemetry::set_performance_enabled(true);
    // When
    super::record_window_ready();
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
    super::record_window_ready();
    assert_eq!(telemetry::test_metric_records().len(), count);
    telemetry::reset_test_metrics();
}
