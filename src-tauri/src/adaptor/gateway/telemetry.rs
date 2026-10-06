use crate::usecase::telemetry::TelemetryPort;
use crate::usecase::telemetry::TerminalLaunch;

pub struct TelemetryGateway;
impl TelemetryPort for TelemetryGateway {
    fn report_frontend_error(&self, error_type: &str, message: &str, stack: Option<&str>) {
        crate::infrastructure::telemetry::crash::report_frontend_error(error_type, message, stack);
    }
    fn set_mounted_xterm_count(&self, count: u64) {
        crate::infrastructure::telemetry::metrics::set_mounted_xterm_count(count);
    }
    fn record_usage_event(&self, name: &str) {
        crate::infrastructure::telemetry::metrics::record_usage_event(name);
    }
    fn set_performance_enabled(&self, enabled: bool) {
        use crate::infrastructure::telemetry::config;
        crate::infrastructure::telemetry::metrics::set_performance_enabled(
            config::telemetry_active(
                config::BuildType::current(),
                config::endpoint(),
                config::license_key(),
                enabled,
            ),
        );
    }
    fn set_crash_reporting_enabled(&self, enabled: bool) {
        crate::infrastructure::telemetry::crash::set_crash_reporting_enabled(enabled);
    }
}

use crate::infrastructure::telemetry::metrics;
use crate::usecase::telemetry::{PerformanceOutput, TerminalLaunchCompletion};

impl TerminalLaunchCompletion for metrics::TerminalLaunchPhaseTimer {
    fn finish(self: Box<Self>) {
        (*self).finish();
    }
}
impl PerformanceOutput for TelemetryGateway {
    fn start_terminal_launch_phase(
        &self,
        phase: TerminalLaunch,
    ) -> Box<dyn TerminalLaunchCompletion> {
        Box::new(metrics::start_terminal_launch_phase(phase.into()))
    }
}
impl From<TerminalLaunch> for metrics::TerminalLaunch {
    fn from(phase: TerminalLaunch) -> Self {
        match phase {
            TerminalLaunch::AvailabilityAndLock => Self::AvailabilityAndLock,
            TerminalLaunch::DurableCreateCommit => Self::DurableCreateCommit,
            TerminalLaunch::LaunchFileMaterialize => Self::LaunchFileMaterialize,
            TerminalLaunch::CheckpointLookup => Self::CheckpointLookup,
            TerminalLaunch::OutputReaderReady => Self::OutputReaderReady,
        }
    }
}
