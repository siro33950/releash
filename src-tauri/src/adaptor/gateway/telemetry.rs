use crate::usecase::telemetry::TelemetryPort;
use crate::usecase::telemetry::{
    TerminalInputSample, TerminalLaunch, TerminalLaunchSample, TerminalPerformanceSwitches,
};
use std::time::Duration;

pub(crate) struct TelemetryGateway;
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
    fn terminal_performance_switches(&self) -> TerminalPerformanceSwitches {
        crate::infrastructure::performance_switches::terminal_performance_switches().into()
    }
    fn performance_real_app_mode(&self) -> bool {
        crate::infrastructure::performance_switches::performance_real_app_mode()
    }
    fn start_terminal_launch_collection(&self) {
        crate::infrastructure::telemetry::metrics::start_terminal_launch_sample_collection();
    }
    fn take_terminal_launch_samples(&self) -> Vec<TerminalLaunchSample> {
        crate::infrastructure::telemetry::metrics::take_terminal_launch_samples()
            .into_iter()
            .map(Into::into)
            .collect()
    }
    fn start_terminal_input_collection(&self) {
        crate::infrastructure::telemetry::metrics::start_terminal_input_sample_collection();
    }
    fn take_terminal_input_samples(&self) -> Vec<TerminalInputSample> {
        crate::infrastructure::telemetry::metrics::take_terminal_input_samples()
            .into_iter()
            .map(Into::into)
            .collect()
    }
    fn record_terminal_launch(&self, phase: TerminalLaunch, duration: Duration) {
        crate::infrastructure::telemetry::metrics::record_terminal_launch(phase.into(), duration);
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
    fn start_terminal_input_trace(
        &self,
        attachment_id: &str,
        sequence: u64,
        client_started_at_unix_ms: f64,
    ) {
        metrics::start_terminal_input_trace(attachment_id, sequence, client_started_at_unix_ms);
    }
    fn record_terminal_input_admission(&self, attachment_id: &str, sequence: u64) {
        metrics::record_terminal_input_admission(attachment_id, sequence);
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
            TerminalLaunch::FirstXtermParsed => Self::FirstXtermParsed,
            TerminalLaunch::FirstPaint => Self::FirstPaint,
        }
    }
}
impl From<metrics::TerminalLaunchSample> for crate::usecase::telemetry::TerminalLaunchSample {
    fn from(value: metrics::TerminalLaunchSample) -> Self {
        Self {
            phase: value.phase,
            duration_ms: value.duration_ms,
        }
    }
}
impl From<metrics::TerminalInputSample> for crate::usecase::telemetry::TerminalInputSample {
    fn from(value: metrics::TerminalInputSample) -> Self {
        Self {
            sequence: value.sequence,
            on_data_to_command_ingress_ms: value.on_data_to_command_ingress_ms,
            command_ingress_to_admission_ms: value.command_ingress_to_admission_ms,
            admission_to_writer_enqueue_ms: value.admission_to_writer_enqueue_ms,
            writer_enqueue_to_output_read_ms: value.writer_enqueue_to_output_read_ms,
            output_read_to_model_apply_ms: value.output_read_to_model_apply_ms,
            model_apply_to_event_publish_ms: value.model_apply_to_event_publish_ms,
            event_published_at_unix_ms: value.event_published_at_unix_ms,
        }
    }
}
impl From<crate::infrastructure::performance_switches::TerminalPerformanceSwitches>
    for crate::usecase::telemetry::TerminalPerformanceSwitches
{
    fn from(
        value: crate::infrastructure::performance_switches::TerminalPerformanceSwitches,
    ) -> Self {
        Self {
            disable_output_flow_control: value.disable_output_flow_control,
            disable_terminal_journal: value.disable_terminal_journal,
            disable_renderer_write_serialization: value.disable_renderer_write_serialization,
            disable_webgl_renderer: value.disable_webgl_renderer,
        }
    }
}

#[cfg(test)]
#[path = "telemetry_test.rs"]
mod telemetry_tests;
