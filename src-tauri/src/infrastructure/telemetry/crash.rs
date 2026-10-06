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

pub fn init_crash_reporting(provider: Option<SdkLoggerProvider>, enabled: bool, configured: bool) {
    set_crash_reporting_enabled(enabled);
    OTLP_CONFIGURED.store(configured, Ordering::Relaxed);
    *LOGGER_PROVIDER.lock().unwrap_or_else(|e| e.into_inner()) = provider;
    install_panic_hook();
}

pub(crate) fn set_crash_reporting_enabled(enabled: bool) {
    CRASH_REPORTING_ENABLED.store(enabled, Ordering::Relaxed);
}

#[cfg(any(test, feature = "test-support"))]
pub fn reset_for_tests() {
    CRASH_REPORTING_ENABLED.store(true, Ordering::Relaxed);
    OTLP_CONFIGURED.store(false, Ordering::Relaxed);
    *LOGGER_PROVIDER.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

pub(crate) fn report_frontend_error(error_type: &str, message: &str, stack: Option<&str>) {
    report_error("frontend", error_type, message, stack);
}

pub fn report_error(source: &str, error_type: &str, message: &str, stack: Option<&str>) {
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
#[path = "crash_test.rs"]
mod crash_tests;

#[cfg(any(test, feature = "test-support"))]
#[path = "crash_test_helpers.rs"]
pub(crate) mod crash_test_helpers;
