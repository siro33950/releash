#[derive(Clone, Debug, PartialEq)]
pub struct TerminalLaunchSample {
    pub phase: &'static str,
    pub duration_ms: f64,
}
#[derive(Clone, Debug, PartialEq)]
pub struct TerminalInputSample {
    pub sequence: u64,
    pub on_data_to_command_ingress_ms: f64,
    pub command_ingress_to_admission_ms: f64,
    pub admission_to_writer_enqueue_ms: f64,
    pub writer_enqueue_to_output_read_ms: f64,
    pub output_read_to_model_apply_ms: f64,
    pub model_apply_to_event_publish_ms: f64,
    pub event_published_at_unix_ms: f64,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TerminalPerformanceSwitches {
    pub disable_output_flow_control: bool,
    pub disable_terminal_journal: bool,
    pub disable_renderer_write_serialization: bool,
    pub disable_webgl_renderer: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PerformanceSwitches {
    pub real_app_mode: bool,
    pub terminal: TerminalPerformanceSwitches,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalLaunch {
    AvailabilityAndLock,
    DurableCreateCommit,
    LaunchFileMaterialize,
    CheckpointLookup,
    OutputReaderReady,
    FirstXtermParsed,
    FirstPaint,
}
use std::time::Duration;

pub trait TelemetryPort {
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

pub struct TelemetryUsecase<'a> {
    port: &'a dyn TelemetryPort,
}
impl<'a> TelemetryUsecase<'a> {
    pub fn new(port: &'a dyn TelemetryPort) -> Self {
        Self { port }
    }
    pub fn report_frontend_error(&self, error_type: &str, message: &str, stack: Option<&str>) {
        self.port.report_frontend_error(error_type, message, stack);
    }
    pub fn set_mounted_xterm_count(&self, count: u64) {
        self.port.set_mounted_xterm_count(count);
    }
    pub fn record_usage_event(&self, name: &str) {
        self.port.record_usage_event(name);
    }
    pub fn update_performance_telemetry(
        &self,
        config: &crate::usecase::app_config::AppConfigUsecase,
        enabled: bool,
    ) -> Result<(), crate::usecase::app_config::error::UsecaseError> {
        config.update_performance_telemetry(enabled)?;
        self.port.set_performance_enabled(enabled);
        Ok(())
    }
    pub fn update_crash_reporting(
        &self,
        config: &crate::usecase::app_config::AppConfigUsecase,
        enabled: bool,
    ) -> Result<(), crate::usecase::app_config::error::UsecaseError> {
        config.update_crash_reporting(enabled)?;
        self.port.set_crash_reporting_enabled(enabled);
        Ok(())
    }
    pub fn terminal_performance_switches(&self) -> TerminalPerformanceSwitches {
        self.port.terminal_performance_switches()
    }
    pub fn performance_real_app_mode(&self) -> bool {
        self.port.performance_real_app_mode()
    }
    pub fn start_terminal_launch_collection(&self) {
        self.port.start_terminal_launch_collection();
    }
    pub fn take_terminal_launch_samples(&self) -> Vec<TerminalLaunchSample> {
        self.port.take_terminal_launch_samples()
    }
    pub fn start_terminal_input_collection(&self) {
        self.port.start_terminal_input_collection();
    }
    pub fn take_terminal_input_samples(&self) -> Vec<TerminalInputSample> {
        self.port.take_terminal_input_samples()
    }
    pub fn record_terminal_launch(&self, phase: TerminalLaunch, duration: Duration) {
        self.port.record_terminal_launch(phase, duration);
    }
}

#[cfg(test)]
#[path = "telemetry_test.rs"]
mod telemetry_tests;

pub trait TerminalLaunchCompletion: Send {
    fn finish(self: Box<Self>);
}
pub trait PerformanceOutput: Send + Sync {
    fn start_terminal_launch_phase(
        &self,
        phase: TerminalLaunch,
    ) -> Box<dyn TerminalLaunchCompletion>;
    fn start_terminal_input_trace(
        &self,
        attachment_id: &str,
        sequence: u64,
        client_started_at_unix_ms: f64,
    );
    fn record_terminal_input_admission(&self, attachment_id: &str, sequence: u64);
}
