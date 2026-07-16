pub(crate) mod action;
pub(crate) mod backend;
pub(crate) mod edit_preview;
pub(crate) mod image;
pub(crate) mod model;
pub(crate) mod paste;
pub(crate) mod permission;
pub(crate) mod session;
pub(crate) mod status;
pub(crate) mod stored_session;
pub(crate) mod suggestion;
pub(crate) mod tool_activity;

pub(super) const COMMAND_NAMES: &[&str] = &[
    "get_session_status",
    "get_workspace_status",
    "list_workspace_statuses",
    "list_session_statuses",
    "query_worktree_node_statuses",
    "sync_worktree_node_statuses",
    "list_agent_backends",
    "start_agent_session",
    "interrupt_agent_query",
    "cancel_agent_queued_turn",
    "build_agent_task_list_report",
    "close_agent_session",
    "set_agent_permission_mode",
    "set_agent_plan_mode",
    "set_agent_model",
    "set_session_backend",
    "present_agent_permission_request",
    "report_agent_permission_request_observed",
    "respond_agent_permission",
    "send_agent_message",
    "search_agent_sessions",
    "search_agent_session_messages",
    "init_agent_sessions",
    "scan_agent_skills",
    "build_agent_edited_multi_edit_tool_input",
    "build_agent_edited_multi_edit_tool_input_all",
    "build_agent_edited_tool_input",
    "build_agent_edit_preview",
    "build_agent_prompt_suggestion",
    "present_agent_tool_activity",
    "prepare_image_attachment",
    "prepare_image_attachments_from_paths",
    "prepare_pasted_text_block",
    "expand_pasted_text_blocks",
    "list_sessions",
    "get_session",
    "get_session_page",
    "plan_agent_chat_eviction",
    "get_session_attachment",
    "get_session_tool_output",
    "create_session",
    "create_workspace_session",
    "close_session",
    "restore_session",
    "list_closed_sessions",
    "archive_session",
    "archive_open_session",
    "fork_session",
    "set_session_title",
    "add_message",
    "update_session_agent_info",
];

pub(crate) fn register(router: &mut super::CommandRouter) {
    router.register_domain(COMMAND_NAMES, Box::new(invoke_handler()));
}

pub(crate) fn invoke_handler(
) -> impl Fn(tauri::ipc::Invoke<tauri::Wry>) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        status::get_session_status,
        status::get_workspace_status,
        status::list_workspace_statuses,
        status::list_session_statuses,
        status::query_worktree_node_statuses,
        status::sync_worktree_node_statuses,
        backend::list_agent_backends,
        session::start_agent_session,
        session::interrupt_agent_query,
        session::cancel_agent_queued_turn,
        session::build_agent_task_list_report,
        session::close_agent_session,
        model::set_agent_permission_mode,
        model::set_agent_plan_mode,
        model::set_agent_model,
        session::set_session_backend,
        permission::present_agent_permission_request,
        permission::report_agent_permission_request_observed,
        permission::respond_agent_permission,
        session::send_agent_message,
        session::search_agent_sessions,
        session::search_agent_session_messages,
        session::init_agent_sessions,
        action::scan_agent_skills,
        edit_preview::build_agent_edited_multi_edit_tool_input,
        edit_preview::build_agent_edited_multi_edit_tool_input_all,
        edit_preview::build_agent_edited_tool_input,
        edit_preview::build_agent_edit_preview,
        suggestion::build_agent_prompt_suggestion,
        tool_activity::present_agent_tool_activity,
        image::prepare_image_attachment,
        image::prepare_image_attachments_from_paths,
        paste::prepare_pasted_text_block,
        paste::expand_pasted_text_blocks,
        stored_session::list_sessions,
        session::get_session,
        session::get_session_page,
        session::plan_agent_chat_eviction,
        session::get_session_attachment,
        session::get_session_tool_output,
        stored_session::create_session,
        stored_session::create_workspace_session,
        stored_session::close_session,
        stored_session::restore_session,
        stored_session::list_closed_sessions,
        stored_session::archive_session,
        stored_session::archive_open_session,
        stored_session::fork_session,
        stored_session::set_session_title,
        stored_session::add_message,
        stored_session::update_session_agent_info,
    ]
}
