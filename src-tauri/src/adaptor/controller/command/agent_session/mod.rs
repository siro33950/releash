pub(crate) const COMMAND_NAMES: &[&str] = &[
    "list_available_agent_session_providers",
    "get_provider_availability",
    "refresh_provider_availability",
    "update_provider_executable",
    "reset_provider_executable",
    "create_agent_session",
    "resume_agent_session_history_candidate",
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
    router.register_domain(
        COMMAND_NAMES,
        Box::new(super::client::handle_registered_invoke),
    );
}
