pub(crate) mod commands;

pub(super) const COMMAND_NAMES: &[&str] = &[
    "report_frontend_error",
    "report_mounted_xterm_count",
    "report_usage_event",
];

pub(crate) fn register(router: &mut super::CommandRouter) {
    router.register_domain(COMMAND_NAMES, Box::new(invoke_handler()));
}

pub(crate) fn invoke_handler(
) -> impl Fn(tauri::ipc::Invoke<tauri::Wry>) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        commands::report_frontend_error,
        commands::report_mounted_xterm_count,
        commands::report_usage_event,
    ]
}
