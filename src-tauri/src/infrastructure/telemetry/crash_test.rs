use super::crash_test_helpers::{install_test_exporter, TEST_LOCK};
use super::*;
use opentelemetry::logs::AnyValue;
use opentelemetry::Key;

use opentelemetry_sdk::logs::SdkLogRecord;

fn any_value_to_string(value: &AnyValue) -> String {
    match value {
        AnyValue::String(value) => value.to_string(),
        _ => format!("{value:?}"),
    }
}

fn attr(record: &SdkLogRecord, key: &str) -> Option<String> {
    let key = Key::new(key.to_string());
    record
        .attributes_iter()
        .find(|(attr_key, _)| *attr_key == key)
        .map(|(_, value)| any_value_to_string(value))
}

fn body(record: &SdkLogRecord) -> Option<String> {
    record.body().map(any_value_to_string)
}

#[test]
fn test_rust_panic_hook_有効時にpanicをotlp_logへ送る() {
    // Given
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset_for_tests();
    let (provider, exporter) = install_test_exporter(true, true);
    // When
    assert!(std::panic::catch_unwind(|| panic!("desktop panic test")).is_err());
    provider.force_flush().unwrap();
    // Then
    let logs = exporter.get_emitted_logs().unwrap();
    assert_eq!(logs.len(), 1);
    assert_eq!(
        attr(&logs[0].record, "exception.source").as_deref(),
        Some("rust")
    );
    assert_eq!(
        attr(&logs[0].record, "exception.type").as_deref(),
        Some("panic")
    );
    assert_eq!(body(&logs[0].record).as_deref(), Some("desktop panic test"));
    reset_for_tests();
}

#[test]
fn scrub_home_dir_replaces_home_path() {
    if let Some(home) = dirs::home_dir() {
        let home_str = home.to_str().unwrap();
        let input = format!("{home_str}/projects/releash/src/main.rs");
        let result = scrub_home_dir(&input);
        assert_eq!(result, "~/projects/releash/src/main.rs");
    }
}

#[test]
fn scrub_home_dir_leaves_non_home_paths() {
    let input = "/usr/local/bin/releash";
    let result = scrub_home_dir(input);
    assert_eq!(result, "/usr/local/bin/releash");
}

#[test]
fn scrub_paths_replaces_absolute_unix_paths() {
    let input = "panic at /Volumes/work/releash/src/main.rs:10";
    let result = scrub_paths(input);
    assert_eq!(result, "panic at [path]");
}

#[test]
fn scrub_paths_replaces_windows_paths() {
    let input = r#"error at C:\Users\me\releash\src\main.rs"#;
    let result = scrub_paths(input);
    assert_eq!(result, "error at [path]");
}

#[test]
fn scrub_sensitive_replaces_urls_and_named_secrets_without_over_redaction() {
    let input = "failed https://hooks.slack.com/services/T000/B000/secret Authorization: Bearer xyz token=abc123 sha=0123456789abcdef0123456789abcdef01234567 uuid=550e8400-e29b-41d4-a716-446655440000";

    let result = scrub_sensitive(input);

    assert!(result.contains("[redacted-url]"));
    assert!(result.contains("Authorization: [redacted]"));
    assert!(result.contains("token=[redacted]"));
    assert!(!result.contains("hooks.slack.com"));
    assert!(!result.contains("Bearer xyz"));
    assert!(result.contains("0123456789abcdef0123456789abcdef01234567"));
    assert!(result.contains("550e8400-e29b-41d4-a716-446655440000"));
}

#[test]
fn report_error_emits_when_gate_is_enabled_and_scrubs_attributes() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset_for_tests();
    let (provider, exporter) = install_test_exporter(true, true);

    report_error(
        "rust",
        "panic https://example.com/type?token=abc",
        "panic at /Volumes/work/releash/src/main.rs token=secret",
        Some("stack C:\\Users\\me\\file.rs Authorization: Bearer xyz"),
    );
    provider.force_flush().unwrap();

    let logs = exporter.get_emitted_logs().unwrap();
    assert_eq!(logs.len(), 1);
    assert_eq!(
        body(&logs[0].record).as_deref(),
        Some("panic at [path] token=[redacted]")
    );
    assert_eq!(
        attr(&logs[0].record, "exception.type").as_deref(),
        Some("panic [redacted-url]")
    );
    assert_eq!(
        attr(&logs[0].record, "exception.stacktrace").as_deref(),
        Some("stack [path] Authorization: [redacted]")
    );
    reset_for_tests();
}

#[test]
fn report_frontend_error_emits_scrubbed_values() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset_for_tests();
    let (provider, exporter) = install_test_exporter(true, true);

    report_frontend_error(
        "react_error token=abc",
        "fetch failed ws://localhost/socket?api_key=abc",
        Some("component at /Users/me/project/App.tsx password=hunter2"),
    );
    provider.force_flush().unwrap();

    let logs = exporter.get_emitted_logs().unwrap();
    assert_eq!(logs.len(), 1);
    assert_eq!(
        attr(&logs[0].record, "exception.type").as_deref(),
        Some("react_error token=[redacted]")
    );
    assert_eq!(
        attr(&logs[0].record, "exception.message").as_deref(),
        Some("fetch failed [redacted-url]")
    );
    assert_eq!(
        attr(&logs[0].record, "exception.stacktrace").as_deref(),
        Some("component at [path] password=[redacted]")
    );
    reset_for_tests();
}

#[test]
fn report_error_skips_when_gate_is_disabled() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset_for_tests();
    let (provider, exporter) = install_test_exporter(false, true);

    report_error("rust", "panic", "boom", None);
    provider.force_flush().unwrap();

    assert!(exporter.get_emitted_logs().unwrap().is_empty());
    reset_for_tests();
}

#[test]
fn crash_reporting_gate_allows_runtime_reopt_in_only_when_configured() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset_for_tests();
    let (provider, exporter) = install_test_exporter(false, true);

    report_error("rust", "panic", "before opt-in", None);
    set_crash_reporting_enabled(true);
    report_error("rust", "panic", "after opt-in", None);
    provider.force_flush().unwrap();

    let logs = exporter.get_emitted_logs().unwrap();
    assert_eq!(logs.len(), 1);
    assert_eq!(body(&logs[0].record).as_deref(), Some("after opt-in"));
    reset_for_tests();

    let (provider, exporter) = install_test_exporter(true, false);
    report_error("rust", "panic", "unconfigured", None);
    provider.force_flush().unwrap();
    assert!(exporter.get_emitted_logs().unwrap().is_empty());
    reset_for_tests();
}

#[test]
fn crash_reporting_opt_out_stops_existing_configured_provider() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset_for_tests();
    let (provider, exporter) = install_test_exporter(true, true);

    report_error("rust", "panic", "before opt-out", None);
    set_crash_reporting_enabled(false);
    report_error("rust", "panic", "after opt-out", None);
    provider.force_flush().unwrap();

    let logs = exporter.get_emitted_logs().unwrap();
    assert_eq!(logs.len(), 1);
    assert_eq!(body(&logs[0].record).as_deref(), Some("before opt-out"));
    reset_for_tests();
}
