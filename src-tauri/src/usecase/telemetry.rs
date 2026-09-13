pub(crate) trait TelemetryPort {
    fn report_frontend_error(&self, error_type: &str, message: &str, stack: Option<&str>);
    fn set_mounted_xterm_count(&self, count: u64);
    fn record_usage_event(&self, name: &str);
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
}

#[cfg(test)]
#[path = "telemetry_test.rs"]
mod telemetry_tests;
