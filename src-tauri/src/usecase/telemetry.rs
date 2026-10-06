#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalLaunch {
    AvailabilityAndLock,
    DurableCreateCommit,
    LaunchFileMaterialize,
    CheckpointLookup,
    OutputReaderReady,
}

pub trait TelemetryPort {
    fn report_frontend_error(&self, error_type: &str, message: &str, stack: Option<&str>);
    fn set_mounted_xterm_count(&self, count: u64);
    fn record_usage_event(&self, name: &str);
    fn set_performance_enabled(&self, enabled: bool);
    fn set_crash_reporting_enabled(&self, enabled: bool);
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
}
