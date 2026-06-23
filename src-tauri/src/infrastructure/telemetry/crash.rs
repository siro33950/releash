use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, Once, OnceLock};
use std::time::SystemTime;

use opentelemetry::logs::{AnyValue, LogRecord, Logger, LoggerProvider, Severity};
use opentelemetry_sdk::logs::SdkLoggerProvider;
use regex::Regex;

static CRASH_REPORTING_ENABLED: AtomicBool = AtomicBool::new(true);
static OTLP_CONFIGURED: AtomicBool = AtomicBool::new(false);
static PANIC_HOOK: Once = Once::new();
static LOGGER_PROVIDER: Mutex<Option<SdkLoggerProvider>> = Mutex::new(None);

pub(crate) fn init_crash_reporting(
    provider: Option<SdkLoggerProvider>,
    enabled: bool,
    configured: bool,
) {
    set_crash_reporting_enabled(enabled);
    OTLP_CONFIGURED.store(configured, Ordering::Relaxed);
    *LOGGER_PROVIDER.lock().unwrap_or_else(|e| e.into_inner()) = provider;
    install_panic_hook();
}

pub(crate) fn set_crash_reporting_enabled(enabled: bool) {
    CRASH_REPORTING_ENABLED.store(enabled, Ordering::Relaxed);
}

#[cfg(test)]
fn reset_for_tests() {
    CRASH_REPORTING_ENABLED.store(true, Ordering::Relaxed);
    OTLP_CONFIGURED.store(false, Ordering::Relaxed);
    *LOGGER_PROVIDER.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

pub(crate) fn report_frontend_error(error_type: &str, message: &str, stack: Option<&str>) {
    report_error("frontend", error_type, message, stack);
}

pub(crate) fn report_error(source: &str, error_type: &str, message: &str, stack: Option<&str>) {
    if !CRASH_REPORTING_ENABLED.load(Ordering::Relaxed) || !OTLP_CONFIGURED.load(Ordering::Relaxed)
    {
        return;
    }
    let provider = LOGGER_PROVIDER
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let Some(provider) = provider else {
        return;
    };

    let scrubbed_message = scrub_sensitive(message);
    let scrubbed_stack = stack.map(scrub_sensitive);
    let scrubbed_error_type = scrub_sensitive(error_type);
    let logger = provider.logger("releash.error");
    let mut record = logger.create_log_record();
    record.set_event_name("exception");
    record.set_timestamp(SystemTime::now());
    record.set_observed_timestamp(SystemTime::now());
    record.set_severity_number(Severity::Error);
    record.set_severity_text("ERROR");
    record.set_body(AnyValue::from(scrubbed_message.clone()));
    record.add_attribute("exception.source", source.to_string());
    record.add_attribute("exception.type", scrubbed_error_type);
    record.add_attribute("exception.message", scrubbed_message);
    if let Some(stack) = scrubbed_stack {
        record.add_attribute("exception.stacktrace", stack);
    }
    logger.emit(record);
}

fn install_panic_hook() {
    PANIC_HOOK.call_once(|| {
        let default_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |panic_info| {
            let message = panic_info
                .payload()
                .downcast_ref::<&str>()
                .map(|s| (*s).to_string())
                .or_else(|| panic_info.payload().downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "panic".to_string());
            let location = panic_info.location().map(|location| {
                format!(
                    "{}:{}:{}",
                    scrub_paths(location.file()),
                    location.line(),
                    location.column()
                )
            });
            let backtrace = std::backtrace::Backtrace::force_capture().to_string();
            let stack = match location {
                Some(location) => format!("{location}\n{}", scrub_paths(&backtrace)),
                None => scrub_paths(&backtrace),
            };
            report_error("rust", "panic", &message, Some(&stack));
            default_hook(panic_info);
        }));
    });
}

fn scrub_home_dir(path: &str) -> String {
    if let Some(home) = dirs::home_dir() {
        if let Some(home_str) = home.to_str() {
            return path.replacen(home_str, "~", 1);
        }
    }
    path.to_string()
}

fn scrub_paths(text: &str) -> String {
    static UNIX_PATH_RE: OnceLock<Regex> = OnceLock::new();
    static WINDOWS_PATH_RE: OnceLock<Regex> = OnceLock::new();

    let text = scrub_home_dir(text);
    let text = UNIX_PATH_RE
        .get_or_init(|| Regex::new(r#"(^|[\s'"(])(/[^\s'")]+)"#).unwrap())
        .replace_all(&text, "$1[path]")
        .into_owned();
    WINDOWS_PATH_RE
        .get_or_init(|| Regex::new(r#"(?i)\b[A-Z]:\\[^\s'")]+\\?[^\s'")]*"#).unwrap())
        .replace_all(&text, "[path]")
        .into_owned()
}

fn scrub_sensitive(text: &str) -> String {
    static URL_RE: OnceLock<Regex> = OnceLock::new();
    static AUTHORIZATION_RE: OnceLock<Regex> = OnceLock::new();
    static SECRET_RE: OnceLock<Regex> = OnceLock::new();

    let text = scrub_paths(text);
    let text = URL_RE
        .get_or_init(|| Regex::new(r#"(?i)\b(?:https?|wss?)://[^\s'")<>]+"#).unwrap())
        .replace_all(&text, "[redacted-url]")
        .into_owned();
    let text = AUTHORIZATION_RE
        .get_or_init(|| {
            Regex::new(r#"(?i)\b(authorization)(\s*[:=]\s*["']?)(?:bearer\s+)?[^\s"',;]+"#).unwrap()
        })
        .replace_all(&text, "$1$2[redacted]")
        .into_owned();
    SECRET_RE
        .get_or_init(|| {
            Regex::new(
                r#"(?i)\b(bearer|token|api[-_]?key|apikey|secret|password|passwd|credential)(\s*[:=\s]\s*["']?)[^\s"',;]+"#,
            )
            .unwrap()
        })
        .replace_all(&text, "$1$2[redacted]")
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry::logs::AnyValue;
    use opentelemetry::Key;
    use opentelemetry_sdk::logs::{InMemoryLogExporter, SdkLogRecord, SdkLoggerProvider};

    static TEST_LOCK: Mutex<()> = Mutex::new(());

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

    fn install_test_exporter(
        enabled: bool,
        configured: bool,
    ) -> (SdkLoggerProvider, InMemoryLogExporter) {
        let exporter = InMemoryLogExporter::default();
        let provider = SdkLoggerProvider::builder()
            .with_simple_exporter(exporter.clone())
            .build();
        init_crash_reporting(Some(provider.clone()), enabled, configured);
        (provider, exporter)
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
}
