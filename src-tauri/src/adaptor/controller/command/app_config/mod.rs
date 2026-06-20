pub(crate) mod commands;

const COMMAND_NAMES: &[&str] = &[
    "get_server_config",
    "update_server_port",
    "regenerate_token",
    "update_telemetry_enabled",
    "get_remote_config",
    "update_remote_config",
    "get_workflow_config",
    "update_workflow_config",
    "get_app_settings",
    "update_app_settings",
    "update_last_server_context",
    "get_crash_reporting_enabled",
    "update_crash_reporting",
    "get_mcp_config",
    "update_mcp_config",
    "regenerate_mcp_token",
];

pub(crate) fn register(router: &mut super::CommandRouter) {
    router.register_domain(COMMAND_NAMES, Box::new(invoke_handler()));
}

pub(crate) fn invoke_handler(
) -> impl Fn(tauri::ipc::Invoke<tauri::Wry>) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        commands::get_server_config,
        commands::update_server_port,
        commands::regenerate_token,
        commands::update_telemetry_enabled,
        commands::get_remote_config,
        commands::update_remote_config,
        commands::get_workflow_config,
        commands::update_workflow_config,
        commands::get_app_settings,
        commands::update_app_settings,
        commands::update_last_server_context,
        commands::get_crash_reporting_enabled,
        commands::update_crash_reporting,
        commands::get_mcp_config,
        commands::update_mcp_config,
        commands::regenerate_mcp_token,
    ]
}
