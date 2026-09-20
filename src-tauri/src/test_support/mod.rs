use std::ffi::{OsStr, OsString};
use std::path::Path;

pub(crate) mod git;

#[path = "../../tests/support/agent_tui_fixture.rs"]
pub(crate) mod agent_tui_fixture;

pub(crate) static TEST_ENV_LOCK: parking_lot::Mutex<()> = parking_lot::Mutex::new(());

pub(crate) struct EnvVarGuard {
    key: &'static str,
    previous: Option<OsString>,
}

impl EnvVarGuard {
    pub(crate) fn set_value(key: &'static str, value: &str) -> Self {
        Self::set_os(key, OsStr::new(value))
    }

    pub(crate) fn set_path(key: &'static str, value: &Path) -> Self {
        Self::set_os(key, value.as_os_str())
    }

    fn set_os(key: &'static str, value: &OsStr) -> Self {
        let previous = std::env::var_os(key);
        std::env::set_var(key, value);
        Self { key, previous }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match &self.previous {
            Some(value) => std::env::set_var(self.key, value),
            None => std::env::remove_var(self.key),
        }
    }
}

struct CapturingLogger {
    messages: std::sync::Mutex<Vec<String>>,
}

impl log::Log for CapturingLogger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= log::Level::Error
    }

    fn log(&self, record: &log::Record<'_>) {
        if self.enabled(record.metadata()) {
            self.messages
                .lock()
                .unwrap()
                .push(record.args().to_string());
        }
    }

    fn flush(&self) {}
}

static CAPTURING_LOGGER: CapturingLogger = CapturingLogger {
    messages: std::sync::Mutex::new(Vec::new()),
};
static CAPTURING_LOGGER_INIT: std::sync::Once = std::sync::Once::new();

pub(crate) fn install_capturing_logger() {
    CAPTURING_LOGGER_INIT.call_once(|| {
        log::set_logger(&CAPTURING_LOGGER).unwrap();
        log::set_max_level(log::LevelFilter::Trace);
    });
}

pub(crate) fn captured_error_messages() -> Vec<String> {
    CAPTURING_LOGGER.messages.lock().unwrap().clone()
}
