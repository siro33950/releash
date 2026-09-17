pub const HEARTBEAT_INTERVAL_MS: u64 = 5_000;
pub const HEARTBEAT_TIMEOUT_MS: u64 = 3_000;
pub const CONNECT_TIMEOUT_MS: u64 = 10_000;
pub const RECONNECT_INTERVAL_MS: u64 = 1_000;
pub const SLEEP_GAP_MS: u64 = 2_000;
pub const TICK_INTERVAL_MS: u64 = 1_000;
pub const OPERATION_RETENTION_MS: u64 = 300_000;
pub const MAX_UNACKNOWLEDGED_OPERATIONS: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Recovery {
    Read,
    Idempotent,
    CallerAttempt,
    AtMostOnce,
    Connection,
}

pub const READ_COMMANDS: &[&str] = &[
    "get_review_blob",
    "build_review_thread_handoff",
    "compute_hidden_ranges_from_content",
    "compute_markdown_diff_ranges",
    "compute_markdown_inline_chunks",
    "compute_markdown_split_rows",
    "compute_visible_markdown_blocks",
    "detect_editors",
    "diagnose_all_cmd",
    "fetch_issues",
    "fetch_notion_label_options",
    "get_agent_session",
    "get_app_settings",
    "get_application_quit_operation",
    "get_application_shutdown",
    "get_application_startup_outcome",
    "get_automation_config_dir",
    "get_branch_base",
    "get_cached_issues",
    "get_cached_pr_status",
    "get_current_branch",
    "get_cwd",
    "get_external_editor",
    "get_facet",
    "get_file_navigation",
    "get_language_from_path",
    "get_main_repo_path",
    "get_notion_config",
    "get_performance_real_app_mode",
    "get_performance_telemetry_enabled",
    "get_provider_availability",
    "get_releash_base",
    "get_repo_paths",
    "get_review_file_view",
    "get_review_snapshot",
    "get_shutdown_plan",
    "get_terminal_performance_switches",
    "get_terminal_surface",
    "get_workflow",
    "get_workflow_config",
    "get_workflow_execution_state",
    "get_workflow_source",
    "get_workspace_node_detail",
    "get_workspace_session_node_id",
    "get_workspace_tree_selection_reconciliation",
    "list_agent_session_history",
    "list_available_agent_session_providers",
    "list_branches",
    "list_branches_with_status",
    "list_branches_with_status_snapshot",
    "list_facet_summaries",
    "list_pending_application_attempts",
    "list_provider_hook_health_warnings",
    "list_review_threads",
    "list_workflows",
    "list_workspace_workflow_history",
    "list_workspace_worktree_nodes",
    "list_worktrees",
    "load_workspace_state",
    "query_notion_tasks",
    "render_facet_preview",
    "resolve_active_execution_by_worktree",
    "validate_notion_config",
    "get_crash_reporting_enabled",
    "get_file_at_ref",
    "get_staged_content",
    "get_binary_staged_content",
    "get_file_at_branch_base",
    "get_binary_file_at_branch_base",
    "get_binary_file_at_ref",
    "get_branch_diff_summary",
    "build_diff_file_tree",
    "get_head_diff_file_tree_snapshot",
    "compute_hidden_ranges",
    "get_relative_path",
    "get_review_thread",
    "get_review_thread_history",
    "fetch_pr_status",
    "get_default_branch",
    "get_git_status",
    "get_git_status_snapshot",
    "get_status_diff_stats",
    "get_status_diff_stats_snapshot",
    "get_git_log",
    "get_worktree_dirty_count",
    "get_repo_git_dir",
    "list_workflow_executions",
    "get_workflow_execution",
    "get_workflow_execution_log",
    "get_workflow_node_detail",
    "resolve_worktree_by_execution",
    "list_facets",
    "workflow_validate_output",
    "workflow_get_output",
];
pub const CALLER_ATTEMPT_COMMANDS: &[&str] = &[
    "create_agent_session",
    "open_agent_session",
    "resume_agent_session",
    "confirm_agent_session_archive_delete",
    "archive_agent_session",
    "delete_agent_session",
    "restore_agent_session",
    "resume_agent_session_history_candidate",
    "request_application_quit",
];
pub const CONNECTION_COMMANDS: &[&str] = &[
    "attach_terminal_surface",
    "detach_terminal_surface",
    "start_watching",
    "start_git_dir_watching",
];
pub const LONG_COMMANDS: &[&str] = &[
    "create_agent_session",
    "resume_agent_session",
    "open_agent_session",
    "resume_agent_session_history_candidate",
    "create_worktree",
    "remove_worktree",
    "start_workflow",
    "request_application_quit",
    "refresh_provider_availability",
    "update_provider_executable",
    "reset_provider_executable",
    "fetch_issues",
    "fetch_pr_status",
    "query_notion_tasks",
    "validate_notion_config",
];
pub const INPUT_COMMANDS: &[&str] = &["write_terminal_surface", "write_paths_to_terminal_surface"];

pub fn recovery(command: &str) -> Recovery {
    if READ_COMMANDS.contains(&command) {
        Recovery::Read
    } else if ordering_scope(command).is_some() {
        Recovery::Idempotent
    } else if CALLER_ATTEMPT_COMMANDS.contains(&command) {
        Recovery::CallerAttempt
    } else if CONNECTION_COMMANDS.contains(&command) {
        Recovery::Connection
    } else {
        Recovery::AtMostOnce
    }
}

pub fn replay_after_restart(command: &str) -> bool {
    command == "request_application_quit"
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OrderingScope {
    RepositoryMembership,
    WorkspaceState,
    NotionConfiguration,
    RepositoryBase,
    BranchBase,
    TerminalSize,
    TerminalOutput,
    Watch,
    ApplicationSettings,
    ExternalEditor,
    WorkflowConfiguration,
    CrashReporting,
    PerformanceTelemetry,
    MountedTerminals,
}

pub fn ordering_scope(command: &str) -> Option<OrderingScope> {
    use OrderingScope::*;
    Some(match command {
        "add_repo_path" | "remove_repo_path" => RepositoryMembership,
        "save_workspace_state" => WorkspaceState,
        "save_notion_config" | "delete_notion_config" => NotionConfiguration,
        "set_releash_base" => RepositoryBase,
        "set_branch_base" => BranchBase,
        "resize_terminal_surface" => TerminalSize,
        "ack_terminal_surface_output" => TerminalOutput,
        "stop_watching" => Watch,
        "update_app_settings" | "update_login_item_preference" => ApplicationSettings,
        "update_external_editor" => ExternalEditor,
        "update_workflow_config" => WorkflowConfiguration,
        "update_crash_reporting" => CrashReporting,
        "update_performance_telemetry" => PerformanceTelemetry,
        "report_mounted_xterm_count" => MountedTerminals,
        _ => return None,
    })
}

pub fn is_watch(command: &str) -> bool {
    matches!(command, "start_watching" | "start_git_dir_watching")
}

pub fn may_replay(command: &str, same_generation: bool, expired: bool) -> bool {
    if is_watch(command) {
        return !expired;
    }
    match recovery(command) {
        Recovery::Read => !expired,
        Recovery::Idempotent => !expired && (same_generation || replay_after_restart(command)),
        Recovery::CallerAttempt => !expired && (same_generation || replay_after_restart(command)),
        _ => false,
    }
}

pub fn waits_for_result(command: &str) -> bool {
    persists_for_restart(command) || is_watch(command)
}

pub fn polls_result(command: &str) -> bool {
    recovery(command) != Recovery::Read
}

pub fn disconnect_action(command: &str, expired: bool) -> &'static str {
    match recovery(command) {
        Recovery::Read if expired => "release",
        Recovery::Connection if !is_watch(command) => "release",
        _ => "query",
    }
}

pub fn deadline_ms(command: &str) -> u64 {
    if LONG_COMMANDS.contains(&command) {
        120_000
    } else if INPUT_COMMANDS.contains(&command) {
        3_000
    } else if recovery(command) == Recovery::Read {
        10_000
    } else {
        30_000
    }
}

#[cfg(test)]
#[path = "policy_test.rs"]
mod policy_tests;

pub fn persists_for_restart(command: &str) -> bool {
    !matches!(recovery(command), Recovery::Read | Recovery::Connection)
}
