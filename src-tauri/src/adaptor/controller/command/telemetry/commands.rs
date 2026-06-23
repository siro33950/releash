use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FrontendErrorPayload {
    error_type: String,
    message: String,
    stack: Option<String>,
}

#[tauri::command]
pub(crate) fn report_frontend_error(payload: FrontendErrorPayload) {
    crate::infrastructure::telemetry::crash::report_frontend_error(
        &payload.error_type,
        &payload.message,
        payload.stack.as_deref(),
    );
}

#[tauri::command]
pub(crate) fn report_mounted_xterm_count(count: u64) {
    crate::other::telemetry::set_mounted_xterm_count(count);
}

#[tauri::command]
pub(crate) fn report_usage_event(name: String) {
    crate::other::telemetry::record_usage_event(&name);
}
