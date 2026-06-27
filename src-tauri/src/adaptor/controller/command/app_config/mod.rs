pub(crate) mod commands;

pub(super) const COMMAND_NAMES: &[&str] = &[
    "update_performance_telemetry",
    "get_workflow_config",
    "update_workflow_config",
    "get_app_settings",
    "update_app_settings",
    "get_crash_reporting_enabled",
    "get_performance_telemetry_enabled",
    "update_crash_reporting",
];

pub(crate) fn register(router: &mut super::CommandRouter) {
    router.register_domain(COMMAND_NAMES, Box::new(invoke_handler()));
}

pub(crate) fn invoke_handler(
) -> impl Fn(tauri::ipc::Invoke<tauri::Wry>) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        commands::update_performance_telemetry,
        commands::get_workflow_config,
        commands::update_workflow_config,
        commands::get_app_settings,
        commands::update_app_settings,
        commands::get_crash_reporting_enabled,
        commands::get_performance_telemetry_enabled,
        commands::update_crash_reporting,
    ]
}
