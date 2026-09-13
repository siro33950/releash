use crate::other::performance_switches::TerminalPerformanceSwitches;
use crate::other::telemetry::{TerminalInputSample, TerminalLaunch, TerminalLaunchSample};
use crate::usecase::telemetry::TelemetryPort;
use std::time::Duration;

pub(crate) struct TelemetryGateway;
impl TelemetryPort for TelemetryGateway {
    fn report_frontend_error(&self, error_type: &str, message: &str, stack: Option<&str>) {
        crate::infrastructure::telemetry::crash::report_frontend_error(error_type, message, stack);
    }
    fn set_mounted_xterm_count(&self, count: u64) {
        crate::other::telemetry::set_mounted_xterm_count(count);
    }
    fn record_usage_event(&self, name: &str) {
        crate::other::telemetry::record_usage_event(name);
    }
    fn set_performance_enabled(&self, enabled: bool) {
        use crate::infrastructure::telemetry::config;
        crate::other::telemetry::set_performance_enabled(config::telemetry_active(
            config::BuildType::current(),
            config::endpoint(),
            config::license_key(),
            enabled,
        ));
    }
    fn set_crash_reporting_enabled(&self, enabled: bool) {
        crate::infrastructure::telemetry::crash::set_crash_reporting_enabled(enabled);
    }
    fn terminal_performance_switches(&self) -> TerminalPerformanceSwitches {
        crate::other::performance_switches::terminal_performance_switches()
    }
    fn performance_real_app_mode(&self) -> bool {
        crate::other::performance_switches::performance_real_app_mode()
    }
    fn start_terminal_launch_collection(&self) {
        crate::other::telemetry::start_terminal_launch_sample_collection();
    }
    fn take_terminal_launch_samples(&self) -> Vec<TerminalLaunchSample> {
        crate::other::telemetry::take_terminal_launch_samples()
    }
    fn start_terminal_input_collection(&self) {
        crate::other::telemetry::start_terminal_input_sample_collection();
    }
    fn take_terminal_input_samples(&self) -> Vec<TerminalInputSample> {
        crate::other::telemetry::take_terminal_input_samples()
    }
    fn record_terminal_launch(&self, phase: TerminalLaunch, duration: Duration) {
        crate::other::telemetry::record_terminal_launch(phase, duration);
    }
}
