pub(crate) mod commands;

const COMMAND_NAMES: &[&str] = &[
    "start_mcp_server",
    "stop_mcp_server",
    "get_mcp_server_status",
    "get_mcp_connection_info",
    "get_configured_agents",
    "remove_agent_mcp_config",
    "save_and_generate_mcp_configs",
    "save_mcp_agent_selection",
    "generate_agent_mcp_config",
    "preview_agent_mcp_config",
];

pub(crate) fn register(router: &mut super::CommandRouter) {
    router.register_domain(COMMAND_NAMES, Box::new(invoke_handler()));
}

pub(crate) fn invoke_handler(
) -> impl Fn(tauri::ipc::Invoke<tauri::Wry>) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        commands::start_mcp_server,
        commands::stop_mcp_server,
        commands::get_mcp_server_status,
        commands::get_mcp_connection_info,
        commands::get_configured_agents,
        commands::remove_agent_mcp_config,
        commands::save_and_generate_mcp_configs,
        commands::save_mcp_agent_selection,
        commands::generate_agent_mcp_config,
        commands::preview_agent_mcp_config,
    ]
}
