use std::sync::OnceLock;

const DISABLE_OUTPUT_FLOW_CONTROL_ENV: &str = "RELEASH_PERF_DISABLE_OUTPUT_FLOW_CONTROL";
const DISABLE_TERMINAL_JOURNAL_ENV: &str = "RELEASH_PERF_DISABLE_TERMINAL_JOURNAL";
const DISABLE_RENDERER_WRITE_SERIALIZATION_ENV: &str =
    "RELEASH_PERF_DISABLE_RENDERER_WRITE_SERIALIZATION";
const DISABLE_WEBGL_RENDERER_ENV: &str = "RELEASH_PERF_DISABLE_WEBGL_RENDERER";
const DISABLE_TERMINAL_WEBSOCKET_ENV: &str = "RELEASH_PERF_DISABLE_TERMINAL_WEBSOCKET";

/// Performance A/B kill-switches. Every switch defaults to `false` so that a
/// process launched without any `RELEASH_PERF_DISABLE_*` env behaves exactly
/// like the current production path.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TerminalPerformanceSwitches {
    pub disable_output_flow_control: bool,
    pub disable_terminal_journal: bool,
    pub disable_renderer_write_serialization: bool,
    pub disable_webgl_renderer: bool,
    pub disable_terminal_websocket: bool,
}

impl TerminalPerformanceSwitches {
    pub fn from_env_reader(read: impl Fn(&str) -> Option<String>) -> Self {
        Self {
            disable_output_flow_control: is_enabled(read(DISABLE_OUTPUT_FLOW_CONTROL_ENV)),
            disable_terminal_journal: is_enabled(read(DISABLE_TERMINAL_JOURNAL_ENV)),
            disable_renderer_write_serialization: is_enabled(read(
                DISABLE_RENDERER_WRITE_SERIALIZATION_ENV,
            )),
            disable_webgl_renderer: is_enabled(read(DISABLE_WEBGL_RENDERER_ENV)),
            disable_terminal_websocket: is_enabled(read(DISABLE_TERMINAL_WEBSOCKET_ENV)),
        }
    }
}

fn is_enabled(value: Option<String>) -> bool {
    value.is_some_and(|value| {
        let value = value.trim();
        value == "1" || value.eq_ignore_ascii_case("true")
    })
}

pub fn terminal_performance_switches() -> TerminalPerformanceSwitches {
    static SWITCHES: OnceLock<TerminalPerformanceSwitches> = OnceLock::new();
    *SWITCHES.get_or_init(|| {
        TerminalPerformanceSwitches::from_env_reader(|name| std::env::var(name).ok())
    })
}

const REAL_APP_MODE_ENV: &str = "RELEASH_PERF_REAL_APP";

/// When enabled, a performance build mounts the real WorkbenchApp UI instead
/// of the dedicated TerminalPerformanceScreen so measurements run against the
/// production UI tree.
pub fn performance_real_app_mode() -> bool {
    static MODE: OnceLock<bool> = OnceLock::new();
    *MODE.get_or_init(|| is_enabled(std::env::var(REAL_APP_MODE_ENV).ok()))
}

#[cfg(test)]
#[path = "performance_switches_test.rs"]
mod performance_switches_tests;
