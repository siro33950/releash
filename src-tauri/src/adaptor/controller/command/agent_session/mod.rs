pub(crate) mod provider_tui;

pub(super) const COMMAND_NAMES: &[&str] = &[
    "list_available_agent_session_providers",
    "get_provider_availability",
    "refresh_provider_availability",
    "update_provider_executable",
    "reset_provider_executable",
    "create_agent_session",
    "resume_agent_session_history_candidate",
    "list_agent_sessions",
    "get_agent_session",
    "open_agent_session",
    "resume_agent_session",
    "archive_agent_session",
    "restore_agent_session",
    "delete_agent_session",
    "confirm_agent_session_archive_delete",
    "list_agent_session_history",
    "list_provider_hook_health_warnings",
];

pub(crate) fn register(router: &mut super::CommandRouter) {
    router.register_domain(COMMAND_NAMES, Box::new(agent_session_invoke_handler()));
}

pub(crate) fn agent_session_invoke_handler<R: tauri::Runtime>(
) -> impl Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        provider_tui::list_available_agent_session_providers,
        provider_tui::get_provider_availability,
        provider_tui::refresh_provider_availability,
        provider_tui::update_provider_executable,
        provider_tui::reset_provider_executable,
        provider_tui::create_agent_session,
        provider_tui::resume_agent_session_history_candidate,
        provider_tui::list_agent_sessions,
        provider_tui::get_agent_session,
        provider_tui::open_agent_session,
        provider_tui::resume_agent_session,
        provider_tui::archive_agent_session,
        provider_tui::restore_agent_session,
        provider_tui::delete_agent_session,
        provider_tui::confirm_agent_session_archive_delete,
        provider_tui::list_agent_session_history,
        provider_tui::list_provider_hook_health_warnings,
    ]
}
