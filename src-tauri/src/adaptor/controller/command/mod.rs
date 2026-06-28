pub(crate) mod agent_session;
pub(crate) mod app_config;
pub(crate) mod code;
pub(crate) mod comment;
pub(crate) mod external_editor;
pub(crate) mod git_host;
pub(crate) mod hooks;
pub(crate) mod menu;
pub(crate) mod notification;
pub(crate) mod notion;
pub(crate) mod pty_session;
pub(crate) mod repository;
pub(crate) mod telemetry;
pub(crate) mod watcher;
pub(crate) mod workflow;
pub(crate) mod workspace_state;
pub(crate) mod workspace_tree;

type InvokeHandler = Box<dyn Fn(tauri::ipc::Invoke<tauri::Wry>) -> bool + Send + Sync>;

pub(crate) struct CommandRouter {
    fallback: InvokeHandler,
    domains: Vec<CommandDomainRoute>,
}

struct CommandDomainRoute {
    command_names: &'static [&'static str],
    handler: InvokeHandler,
}

impl CommandRouter {
    fn new(fallback: InvokeHandler) -> Self {
        Self {
            fallback,
            domains: Vec::new(),
        }
    }

    pub(crate) fn register_domain(
        &mut self,
        command_names: &'static [&'static str],
        handler: InvokeHandler,
    ) {
        self.domains.push(CommandDomainRoute {
            command_names,
            handler,
        });
    }

    fn handle(&self, invoke: tauri::ipc::Invoke<tauri::Wry>) -> bool {
        let route_index = self.domain_route_index(invoke.message.command());

        if let Some(route_index) = route_index {
            (self.domains[route_index].handler)(invoke)
        } else {
            (self.fallback)(invoke)
        }
    }

    fn domain_route_index(&self, command: &str) -> Option<usize> {
        self.domains
            .iter()
            .position(|domain| domain.command_names.contains(&command))
    }
}

pub(crate) fn register_all(builder: tauri::Builder<tauri::Wry>) -> tauri::Builder<tauri::Wry> {
    let app_handler: InvokeHandler = Box::new(|_invoke| false);
    let mut router = CommandRouter::new(app_handler);
    agent_session::register(&mut router);
    app_config::register(&mut router);
    code::register(&mut router);
    comment::register(&mut router);
    external_editor::register(&mut router);
    git_host::register(&mut router);
    hooks::register(&mut router);
    menu::register(&mut router);
    notification::register(&mut router);
    notion::register(&mut router);
    pty_session::register(&mut router);
    repository::register(&mut router);
    telemetry::register(&mut router);
    watcher::register(&mut router);
    workspace_state::register(&mut router);
    workspace_tree::register(&mut router);
    workflow::register(&mut router);
    builder.invoke_handler(move |invoke: tauri::ipc::Invoke<tauri::Wry>| router.handle(invoke))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn dummy_handler() -> InvokeHandler {
        Box::new(|_invoke| true)
    }

    type RegisterFn = fn(&mut CommandRouter);

    fn command_domains() -> Vec<(&'static str, &'static [&'static str], RegisterFn)> {
        vec![
            (
                "agent_session",
                agent_session::COMMAND_NAMES,
                agent_session::register,
            ),
            (
                "app_config",
                app_config::COMMAND_NAMES,
                app_config::register,
            ),
            ("code", code::COMMAND_NAMES, code::register),
            ("comment", comment::COMMAND_NAMES, comment::register),
            (
                "external_editor",
                external_editor::COMMAND_NAMES,
                external_editor::register,
            ),
            ("git_host", git_host::COMMAND_NAMES, git_host::register),
            ("hooks", hooks::COMMAND_NAMES, hooks::register),
            ("menu", menu::COMMAND_NAMES, menu::register),
            (
                "notification",
                notification::COMMAND_NAMES,
                notification::register,
            ),
            ("notion", notion::COMMAND_NAMES, notion::register),
            (
                "pty_session",
                pty_session::COMMAND_NAMES,
                pty_session::register,
            ),
            (
                "repository",
                repository::COMMAND_NAMES,
                repository::register,
            ),
            ("telemetry", telemetry::COMMAND_NAMES, telemetry::register),
            ("watcher", watcher::COMMAND_NAMES, watcher::register),
            (
                "workspace_state",
                workspace_state::COMMAND_NAMES,
                workspace_state::register,
            ),
            (
                "workspace_tree",
                workspace_tree::COMMAND_NAMES,
                workspace_tree::register,
            ),
            ("workflow", workflow::COMMAND_NAMES, workflow::register),
        ]
    }

    fn registered_command_names() -> Vec<&'static str> {
        command_domains()
            .into_iter()
            .flat_map(|(_, command_names, _)| command_names.iter().copied())
            .collect()
    }

    fn canonical_command_names() -> &'static [&'static str] {
        &[
            "abort_workflow",
            "add_message",
            "add_repo_path",
            "append_review_comment",
            "apply_hooks_config",
            "approve_workflow_step",
            "archive_open_session",
            "archive_session",
            "archive_workspace_workflow_run",
            "build_agent_edit_preview",
            "build_agent_edited_multi_edit_tool_input",
            "build_agent_edited_multi_edit_tool_input_all",
            "build_agent_edited_tool_input",
            "build_agent_prompt_suggestion",
            "build_agent_task_list_report",
            "build_diff_file_tree",
            "build_review_thread_handoff",
            "cancel_agent_queued_turn",
            "check_pr_provider_status",
            "close_agent_session",
            "close_session",
            "compute_hidden_ranges",
            "compute_hidden_ranges_from_content",
            "compute_markdown_diff_ranges",
            "compute_markdown_inline_chunks",
            "compute_markdown_split_rows",
            "compute_visible_markdown_blocks",
            "create_review_thread",
            "create_session",
            "create_worktree",
            "delete_branch",
            "delete_facet",
            "delete_notion_config",
            "delete_review_thread",
            "delete_workflow",
            "detect_editors",
            "diagnose_all_cmd",
            "duplicate_facet",
            "duplicate_workflow",
            "expand_pasted_text_blocks",
            "fetch_issues",
            "fetch_notion_label_options",
            "fetch_pr_status",
            "fork_session",
            "gc_ptys_for_worktree",
            "generate_hooks_config",
            "get_agent_shortcut_settings",
            "get_app_settings",
            "get_automation_config_dir",
            "get_binary_file_at_branch_base",
            "get_binary_file_at_ref",
            "get_binary_staged_content",
            "get_branch_base",
            "get_branch_diff_summary",
            "get_cached_issues",
            "get_cached_pr_status",
            "get_crash_reporting_enabled",
            "get_current_branch",
            "get_cwd",
            "get_default_branch",
            "get_external_editor",
            "get_facet",
            "get_file_at_branch_base",
            "get_file_at_ref",
            "get_file_navigation",
            "get_git_log",
            "get_git_status",
            "get_git_status_snapshot",
            "get_head_diff_file_tree_snapshot",
            "get_hooks_status",
            "get_language_from_path",
            "get_main_repo_path",
            "get_notion_config",
            "get_notify_config",
            "get_or_spawn_pty",
            "get_performance_telemetry_enabled",
            "get_pty_buffered_output",
            "get_relative_path",
            "get_releash_base",
            "get_repo_git_dir",
            "get_repo_paths",
            "get_review_file_view",
            "get_review_snapshot",
            "get_review_thread",
            "get_review_thread_history",
            "get_session",
            "get_session_attachment",
            "get_session_page",
            "get_session_status",
            "get_session_tool_output",
            "get_staged_content",
            "get_status_diff_stats",
            "get_status_diff_stats_snapshot",
            "get_workflow",
            "get_workflow_config",
            "get_workflow_run",
            "get_workflow_run_log",
            "get_workflow_run_state",
            "get_workflow_state",
            "get_workflow_step_detail",
            "get_workspace_status",
            "get_workspace_workflow_step_detail",
            "get_worktree_dirty_count",
            "git_create_branch",
            "git_stage",
            "git_stage_review_group",
            "git_unstage",
            "git_unstage_review_group",
            "init_agent_sessions",
            "interrupt_agent_query",
            "is_agent_command_enabled",
            "kill_pty",
            "kill_ptys_by_worktree",
            "list_agent_backends",
            "list_branches",
            "list_branches_with_status",
            "list_branches_with_status_snapshot",
            "list_closed_sessions",
            "list_facet_summaries",
            "list_facets",
            "list_mentionable_files",
            "list_pty_sessions",
            "list_review_threads",
            "list_session_statuses",
            "list_sessions",
            "list_workflow_runs",
            "list_workflow_step_statuses",
            "list_workflows",
            "list_workspace_statuses",
            "list_workspace_workflow_history",
            "list_workspace_worktree_nodes",
            "list_worktrees",
            "load_workspace_state",
            "open_facet_in_editor",
            "open_folder_in_editor",
            "open_in_editor",
            "open_workflow_in_editor",
            "plan_agent_chat_eviction",
            "prepare_image_attachment",
            "prepare_image_attachments_from_paths",
            "prepare_pasted_text_block",
            "present_agent_command_palette",
            "present_agent_permission_request",
            "present_agent_tool_activity",
            "query_notion_tasks",
            "read_codex_mentionable_files",
            "read_codex_skill_catalog",
            "reconcile_pty_sessions",
            "register_active_terminal",
            "remove_repo_path",
            "remove_worktree",
            "render_facet_preview",
            "report_frontend_error",
            "report_mounted_xterm_count",
            "report_usage_event",
            "reset_agent_shortcut_settings",
            "resize_pty",
            "resolve_active_run_by_worktree",
            "resolve_review_thread",
            "resolve_worktree_by_run",
            "respond_agent_permission",
            "restore_session",
            "restore_workspace_workflow_run",
            "resync_streaming_message",
            "save_facet",
            "save_notion_config",
            "save_workflow",
            "save_workspace_state",
            "scan_agent_skills",
            "search_agent_session_messages",
            "search_agent_sessions",
            "send_agent_message",
            "send_workflow_approval_chat_message",
            "set_agent_model",
            "set_agent_permission_mode",
            "set_agent_plan_mode",
            "set_branch_base",
            "set_menu_items_enabled",
            "set_releash_base",
            "set_session_backend",
            "set_session_title",
            "start_agent_session",
            "start_git_dir_watching",
            "start_watching",
            "start_workflow",
            "stop_watching",
            "sync_mentions_with_text",
            "unregister_active_terminal",
            "update_agent_shortcut_settings",
            "update_app_settings",
            "update_crash_reporting",
            "update_external_editor",
            "update_notify_config",
            "update_performance_telemetry",
            "update_session_agent_info",
            "update_session_state",
            "update_webhook_url",
            "update_workflow_config",
            "validate_notion_config",
            "workflow_get_output",
            "workflow_submit_output",
            "workflow_validate_output",
            "write_paths_to_pty",
            "write_pty",
        ]
    }

    #[test]
    fn every_domain_command_routes_to_its_registered_domain() {
        let domains = command_domains();
        let mut router = CommandRouter::new(dummy_handler());
        for (_, _, register) in &domains {
            register(&mut router);
        }

        for (domain_index, (domain_name, command_names, _)) in domains.iter().enumerate() {
            for command_name in *command_names {
                assert_eq!(
                    router.domain_route_index(command_name),
                    Some(domain_index),
                    "{command_name} should route to {domain_name}"
                );
            }
        }
        assert_eq!(router.domain_route_index("unknown_releash_command"), None);
    }

    #[test]
    fn domain_command_names_match_canonical_command_set() {
        let registered = registered_command_names();
        let registered_set = registered.iter().copied().collect::<BTreeSet<_>>();
        let expected = canonical_command_names();
        let expected_set = expected.iter().copied().collect::<BTreeSet<_>>();

        assert_eq!(
            registered.len(),
            registered_set.len(),
            "duplicate command names in registered domain COMMAND_NAMES"
        );
        assert_eq!(
            expected.len(),
            expected_set.len(),
            "duplicate command names in canonical command set"
        );
        assert!(!expected_set.contains("get_review_text_diff"));
        assert!(!expected_set.contains("get_review_image_diff"));
        assert_eq!(registered_set, expected_set);
    }

    #[test]
    fn git_host_register_routes_git_host_commands_before_fallback() {
        let mut router = CommandRouter::new(dummy_handler());

        git_host::register(&mut router);

        assert_eq!(
            router.domain_route_index("check_pr_provider_status"),
            Some(0)
        );
        assert_eq!(router.domain_route_index("get_cached_issues"), Some(0));
        assert_eq!(router.domain_route_index("get_git_status"), None);
    }

    #[test]
    fn notion_register_routes_notion_commands_before_fallback() {
        let mut router = CommandRouter::new(dummy_handler());

        notion::register(&mut router);

        assert_eq!(router.domain_route_index("query_notion_tasks"), Some(0));
        assert_eq!(
            router.domain_route_index("fetch_notion_label_options"),
            Some(0)
        );
        assert_eq!(router.domain_route_index("save_notion_config"), Some(0));
        assert_eq!(router.domain_route_index("get_notion_config"), Some(0));
        assert_eq!(router.domain_route_index("delete_notion_config"), Some(0));
        assert_eq!(router.domain_route_index("validate_notion_config"), Some(0));
        assert_eq!(router.domain_route_index("get_git_status"), None);
    }
}
