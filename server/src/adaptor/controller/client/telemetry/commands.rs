pub(crate) fn report_frontend_error_shared(
    error_type: String,
    message: String,
    stack: Option<String>,
) {
    telemetry().report_frontend_error(&error_type, &message, stack.as_deref());
}

pub(crate) fn report_mounted_xterm_count_shared(count: u64) {
    telemetry().set_mounted_xterm_count(count);
}

pub(crate) fn report_usage_event_shared(name: String) {
    telemetry().record_usage_event(&name);
}

fn telemetry() -> crate::usecase::telemetry::TelemetryUsecase<'static> {
    crate::usecase::telemetry::TelemetryUsecase::new(
        &crate::adaptor::gateway::telemetry::TelemetryGateway,
    )
}
