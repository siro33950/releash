pub(crate) mod action;
pub(crate) mod backend;
pub(crate) mod edit_preview;
pub(crate) mod image;
pub(crate) mod model;
pub(crate) mod notice;
pub(crate) mod paste;
pub(crate) mod permission;
pub(crate) mod provider_tui;
pub(crate) mod session;
pub(crate) mod status;
pub(crate) mod stored_session;
pub(crate) mod suggestion;
pub(crate) mod tool_activity;

use crate::other::error::AppError;
use crate::usecase::agent_session::runtime::usecase::AgentRuntimeError;

impl From<AgentRuntimeError> for AppError {
    fn from(error: AgentRuntimeError) -> Self {
        match error {
            AgentRuntimeError::StartupTimeout {
                retry_count,
                max_retries,
            } => Self::AgentStartupTimeout {
                retry_count,
                max_retries,
            },
            locked @ AgentRuntimeError::BackendSelectionLocked => {
                Self::Internal(locked.to_string())
            }
            lost @ AgentRuntimeError::BackendSessionLost { .. } => Self::Internal(lost.to_string()),
            deferred @ AgentRuntimeError::AcceptedEffectAdmissionDeferred => {
                Self::Internal(deferred.to_string())
            }
            failed @ AgentRuntimeError::AcceptedEffectAdmissionFailed { .. } => {
                Self::Internal(failed.to_string())
            }
            workflow_send @ AgentRuntimeError::WorkflowTurnSend(_) => {
                Self::Internal(workflow_send.to_string())
            }
            workspace_query @ AgentRuntimeError::WorkspaceQuery(_) => {
                Self::Internal(workspace_query.to_string())
            }
            AgentRuntimeError::Other(message) => Self::Internal(message),
        }
    }
}

pub(super) const LEGACY_COMMAND_NAMES: &[&str] = &[
    "get_session_status",
    "get_workspace_status",
    "list_workspace_statuses",
    "list_session_statuses",
    "query_worktree_node_statuses",
    "sync_worktree_node_statuses",
    "list_agent_backends",
    "stop_agent_session",
    "get_stop_operation",
    "request_session_lifecycle",
    "get_session_lifecycle_operation",
    "list_pending_agent_recovery",
    "get_pending_recovery_snapshot",
    "list_pending_agent_attempts",
    "acknowledge_agent_attempt",
    "resolve_pending_recovery_action",
    "get_recovery_action",
    "resume_agent_queue",
    "cancel_agent_queued_turn",
    "build_agent_task_list_report",
    "set_agent_permission_mode",
    "set_agent_plan_mode",
    "set_agent_model",
    "present_agent_permission_request",
    "report_agent_permission_request_observed",
    "respond_agent_permission",
    "get_agent_permission_response_operation",
    "send_agent_message",
    "get_agent_send_operation",
    "search_agent_sessions",
    "search_agent_session_messages",
    "init_agent_sessions",
    "get_agent_session_notice",
    "update_agent_session_notice",
    "list_agent_session_feedback",
    "dismiss_agent_session_feedback",
    "retry_agent_session_feedback",
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
    "get_agent_session_display_window",
    "get_session_page",
    "plan_agent_chat_eviction",
    "get_session_attachment",
    "get_session_tool_output",
    "create_session",
    "create_workspace_session",
    "restore_session",
    "list_closed_sessions",
    "fork_session",
    "set_session_title",
];

pub(super) const PROVIDER_TUI_COMMAND_NAMES: &[&str] = &[
    "list_available_provider_agent_session_providers",
    "create_provider_agent_session",
    "resume_provider_agent_session_history_candidate",
    "list_provider_agent_sessions",
    "get_provider_agent_session",
    "open_provider_agent_session",
    "resume_provider_agent_session",
    "archive_provider_agent_session",
    "restore_provider_agent_session",
    "delete_provider_agent_session",
    "confirm_provider_agent_session_archive_delete",
    "list_provider_agent_session_history",
    "list_provider_hook_health_warnings",
];

pub(crate) fn register(router: &mut super::CommandRouter) {
    register_legacy(router);
    register_provider_tui(router);
}

pub(super) fn register_legacy(router: &mut super::CommandRouter) {
    router.register_domain(LEGACY_COMMAND_NAMES, Box::new(invoke_handler()));
}

pub(super) fn register_provider_tui(router: &mut super::CommandRouter) {
    router.register_domain(
        PROVIDER_TUI_COMMAND_NAMES,
        Box::new(provider_tui_invoke_handler()),
    );
}

pub(crate) fn invoke_handler() -> impl Fn(tauri::ipc::Invoke) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        status::get_session_status,
        status::get_workspace_status,
        status::list_workspace_statuses,
        status::list_session_statuses,
        status::query_worktree_node_statuses,
        status::sync_worktree_node_statuses,
        backend::list_agent_backends,
        session::stop_agent_session,
        session::get_stop_operation,
        session::request_session_lifecycle,
        session::get_session_lifecycle_operation,
        session::list_pending_agent_recovery,
        session::get_pending_recovery_snapshot,
        session::list_pending_agent_attempts,
        session::acknowledge_agent_attempt,
        session::resolve_pending_recovery_action,
        session::get_recovery_action,
        session::resume_agent_queue,
        session::cancel_agent_queued_turn,
        session::build_agent_task_list_report,
        model::set_agent_permission_mode,
        model::set_agent_plan_mode,
        model::set_agent_model,
        permission::present_agent_permission_request,
        permission::report_agent_permission_request_observed,
        permission::respond_agent_permission,
        permission::get_agent_permission_response_operation,
        session::send_agent_message,
        session::get_agent_send_operation,
        session::search_agent_sessions,
        session::search_agent_session_messages,
        session::init_agent_sessions,
        notice::get_agent_session_notice,
        notice::update_agent_session_notice,
        notice::list_agent_session_feedback,
        notice::dismiss_agent_session_feedback,
        notice::retry_agent_session_feedback,
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
        session::get_agent_session_display_window,
        session::get_session_page,
        session::plan_agent_chat_eviction,
        session::get_session_attachment,
        session::get_session_tool_output,
        stored_session::create_session,
        stored_session::create_workspace_session,
        stored_session::restore_session,
        stored_session::list_closed_sessions,
        stored_session::fork_session,
        stored_session::set_session_title,
    ]
}

pub(crate) fn provider_tui_invoke_handler<R: tauri::Runtime>(
) -> impl Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        provider_tui::list_available_provider_agent_session_providers,
        provider_tui::create_provider_agent_session,
        provider_tui::resume_provider_agent_session_history_candidate,
        provider_tui::list_provider_agent_sessions,
        provider_tui::get_provider_agent_session,
        provider_tui::open_provider_agent_session,
        provider_tui::resume_provider_agent_session,
        provider_tui::archive_provider_agent_session,
        provider_tui::restore_provider_agent_session,
        provider_tui::delete_provider_agent_session,
        provider_tui::confirm_provider_agent_session_archive_delete,
        provider_tui::list_provider_agent_session_history,
        provider_tui::list_provider_hook_health_warnings,
    ]
}
