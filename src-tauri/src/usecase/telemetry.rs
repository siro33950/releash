use crate::other::performance_switches::TerminalPerformanceSwitches;
use crate::other::telemetry::{TerminalInputSample, TerminalLaunch, TerminalLaunchSample};
use std::time::Duration;

pub(crate) trait TelemetryPort {
    fn report_frontend_error(&self, error_type: &str, message: &str, stack: Option<&str>);
    fn set_mounted_xterm_count(&self, count: u64);
    fn record_usage_event(&self, name: &str);
    fn set_performance_enabled(&self, enabled: bool);
    fn set_crash_reporting_enabled(&self, enabled: bool);
    fn terminal_performance_switches(&self) -> TerminalPerformanceSwitches;
    fn performance_real_app_mode(&self) -> bool;
    fn start_terminal_launch_collection(&self);
    fn take_terminal_launch_samples(&self) -> Vec<TerminalLaunchSample>;
    fn start_terminal_input_collection(&self);
    fn take_terminal_input_samples(&self) -> Vec<TerminalInputSample>;
    fn record_terminal_launch(&self, phase: TerminalLaunch, duration: Duration);
}

pub(crate) struct TelemetryUsecase<'a> {
    port: &'a dyn TelemetryPort,
}
impl<'a> TelemetryUsecase<'a> {
    pub(crate) fn new(port: &'a dyn TelemetryPort) -> Self {
        Self { port }
    }
    pub(crate) fn report_frontend_error(
        &self,
        error_type: &str,
        message: &str,
        stack: Option<&str>,
    ) {
        self.port.report_frontend_error(error_type, message, stack);
    }
    pub(crate) fn set_mounted_xterm_count(&self, count: u64) {
        self.port.set_mounted_xterm_count(count);
    }
    pub(crate) fn record_usage_event(&self, name: &str) {
        self.port.record_usage_event(name);
    }
    pub(crate) fn update_performance_telemetry(
        &self,
        config: &crate::usecase::app_config::AppConfigUsecase,
        enabled: bool,
    ) -> Result<(), crate::usecase::app_config::error::UsecaseError> {
        config.update_performance_telemetry(enabled)?;
        self.port.set_performance_enabled(enabled);
        Ok(())
    }
    pub(crate) fn update_crash_reporting(
        &self,
        config: &crate::usecase::app_config::AppConfigUsecase,
        enabled: bool,
    ) -> Result<(), crate::usecase::app_config::error::UsecaseError> {
        config.update_crash_reporting(enabled)?;
        self.port.set_crash_reporting_enabled(enabled);
        Ok(())
    }
    pub(crate) fn terminal_performance_switches(&self) -> TerminalPerformanceSwitches {
        self.port.terminal_performance_switches()
    }
    pub(crate) fn performance_real_app_mode(&self) -> bool {
        self.port.performance_real_app_mode()
    }
    pub(crate) fn start_terminal_launch_collection(&self) {
        self.port.start_terminal_launch_collection();
    }
    pub(crate) fn take_terminal_launch_samples(&self) -> Vec<TerminalLaunchSample> {
        self.port.take_terminal_launch_samples()
    }
    pub(crate) fn start_terminal_input_collection(&self) {
        self.port.start_terminal_input_collection();
    }
    pub(crate) fn take_terminal_input_samples(&self) -> Vec<TerminalInputSample> {
        self.port.take_terminal_input_samples()
    }
    pub(crate) fn record_terminal_launch(&self, phase: TerminalLaunch, duration: Duration) {
        self.port.record_terminal_launch(phase, duration);
    }
}

#[cfg(test)]
#[path = "telemetry_test.rs"]
mod telemetry_tests;
