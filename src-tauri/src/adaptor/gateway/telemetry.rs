use crate::usecase::telemetry::TelemetryPort;

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
}
