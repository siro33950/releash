pub(crate) mod agent_session;
pub(crate) mod app_config;
pub(crate) mod application_lifecycle;
pub(crate) mod code;
pub(crate) mod comment;
pub(crate) mod external_editor;
pub(crate) mod git_host;
pub(crate) mod menu;
pub(crate) mod notification;
pub(crate) mod notion;
pub(crate) mod repository;
pub(crate) mod telemetry;
pub(crate) mod terminal_surface;
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

const STARTUP_COMMANDS: [&str; 2] = [
    "get_application_startup_outcome",
    "quit_after_startup_failure",
];

fn command_admitted(
    command: &str,
    authority: Option<&crate::usecase::application_startup::ApplicationStartupAuthority>,
) -> bool {
    authority.is_some_and(|authority| {
        STARTUP_COMMANDS.contains(&command) || authority.normal_admission_ready()
    })
}

pub(crate) fn gate_invoke_before_domain_routing<R: tauri::Runtime>(
    invoke: tauri::ipc::Invoke<R>,
) -> Result<tauri::ipc::Invoke<R>, bool> {
    let admitted = {
        let authority = invoke.message.state_ref().try_get::<std::sync::Arc<
            crate::usecase::application_startup::ApplicationStartupAuthority,
        >>();
        command_admitted(
            invoke.message.command(),
            authority.map(|authority| authority.inner().as_ref()),
        )
    };
    if !admitted {
        invoke.resolver.reject(
            crate::usecase::application_startup::ApplicationUnavailable::ApplicationUnavailable,
        );
        Err(true)
    } else {
        Ok(invoke)
    }
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
        let invoke = match gate_invoke_before_domain_routing(invoke) {
            Ok(invoke) => invoke,
            Err(handled) => return handled,
        };
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
    application_lifecycle::register(&mut router);
    code::register(&mut router);
    comment::register(&mut router);
    external_editor::register(&mut router);
    git_host::register(&mut router);
    menu::register(&mut router);
    notification::register(&mut router);
    notion::register(&mut router);
    terminal_surface::register(&mut router);
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
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tauri::Manager;

    fn dummy_handler() -> InvokeHandler {
        Box::new(|_invoke| true)
    }

    #[test]
    fn test_agent_session_provider_tui_commandを本番routerの独立routeとして登録する() {
        let mut router = CommandRouter::new(dummy_handler());

        agent_session::register(&mut router);

        assert_eq!(router.domains.len(), 2);
        assert!(router.domains.iter().any(|route| {
            route
                .command_names
                .contains(&"create_provider_agent_session")
                && !route.command_names.contains(&"create_session")
        }));
    }

    type RegisterFn = fn(&mut CommandRouter);

    fn command_domains() -> Vec<(&'static str, &'static [&'static str], RegisterFn)> {
        vec![
            (
                "agent_session_legacy",
                agent_session::LEGACY_COMMAND_NAMES,
                agent_session::register_legacy,
            ),
            (
                "agent_session_provider_tui",
                agent_session::PROVIDER_TUI_COMMAND_NAMES,
                agent_session::register_provider_tui,
            ),
            (
                "app_config",
                app_config::COMMAND_NAMES,
                app_config::register,
            ),
            (
                "application_lifecycle",
                application_lifecycle::COMMAND_NAMES,
                application_lifecycle::register,
            ),
            ("code", code::COMMAND_NAMES, code::register),
            ("comment", comment::COMMAND_NAMES, comment::register),
            (
                "external_editor",
                external_editor::COMMAND_NAMES,
                external_editor::register,
            ),
            ("git_host", git_host::COMMAND_NAMES, git_host::register),
            ("menu", menu::COMMAND_NAMES, menu::register),
            (
                "notification",
                notification::COMMAND_NAMES,
                notification::register,
            ),
            ("notion", notion::COMMAND_NAMES, notion::register),
            (
                "terminal_surface",
                terminal_surface::COMMAND_NAMES,
                terminal_surface::register,
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

    pub(super) fn registered_command_names() -> Vec<&'static str> {
        command_domains()
            .into_iter()
            .flat_map(|(_, command_names, _)| command_names.iter().copied())
            .collect()
    }

    #[test]
    fn failed_startup_admits_only_the_two_safe_commands_before_domain_routing() {
        let failed = crate::usecase::application_startup::ApplicationStartupAuthority::failed_kind(
            crate::usecase::application_startup::StartupFailureKind::StoreValidationFailed,
        );
        let ready = crate::usecase::application_startup::ApplicationStartupAuthority::ready();

        for command in registered_command_names() {
            assert_eq!(
                command_admitted(command, Some(&failed)),
                STARTUP_COMMANDS.contains(&command),
                "unexpected failed-startup admission for {command}"
            );
            assert!(
                command_admitted(command, Some(&ready)),
                "ready startup rejected {command}"
            );
        }
    }

    #[test]
    fn missing_startup_authority_fails_closed_before_any_command_routing() {
        for command in registered_command_names() {
            assert!(
                !command_admitted(command, None),
                "missing startup authority admitted {command}"
            );
        }
        for command in STARTUP_COMMANDS {
            assert!(
                !command_admitted(command, None),
                "startup command {command} cannot run without its authority"
            );
        }
    }

    #[tauri::command]
    fn record_normal_command_effect(effects: tauri::State<'_, Arc<AtomicUsize>>) -> &'static str {
        effects.fetch_add(1, Ordering::SeqCst);
        "normal-effect-ran"
    }

    fn invoke_request(command: &str) -> tauri::webview::InvokeRequest {
        tauri::webview::InvokeRequest {
            cmd: command.to_string(),
            callback: tauri::ipc::CallbackFn(0),
            error: tauri::ipc::CallbackFn(1),
            url: if cfg!(any(windows, target_os = "android")) {
                "http://tauri.localhost"
            } else {
                "tauri://localhost"
            }
            .parse()
            .unwrap(),
            body: tauri::ipc::InvokeBody::default(),
            headers: Default::default(),
            invoke_key: tauri::test::INVOKE_KEY.to_string(),
        }
    }

    fn command_gate_test_handler(
    ) -> impl Fn(tauri::ipc::Invoke<tauri::test::MockRuntime>) -> bool + Send + Sync + 'static {
        tauri::generate_handler![
            application_lifecycle::get_application_startup_outcome,
            record_normal_command_effect
        ]
    }

    fn command_gate_test_app(
        authority: Option<Arc<crate::usecase::application_startup::ApplicationStartupAuthority>>,
    ) -> (tauri::App<tauri::test::MockRuntime>, Arc<AtomicUsize>) {
        let effects = Arc::new(AtomicUsize::new(0));
        let mut builder = tauri::test::mock_builder().manage(effects.clone());
        if let Some(authority) = authority {
            builder = builder.manage(authority);
        }
        let handler = command_gate_test_handler();
        let app = builder
            .invoke_handler(
                move |invoke| match gate_invoke_before_domain_routing(invoke) {
                    Ok(invoke) => handler(invoke),
                    Err(handled) => handled,
                },
            )
            .build(crate::application_context())
            .expect("build startup command gate test app");
        (app, effects)
    }

    #[test]
    fn failed_and_missing_authority_reject_actual_normal_ipc_before_its_effect() {
        let failed = Arc::new(
            crate::usecase::application_startup::ApplicationStartupAuthority::failed_kind(
                crate::usecase::application_startup::StartupFailureKind::StoreValidationFailed,
            ),
        );
        for authority in [Some(failed), None] {
            let (app, effects) = command_gate_test_app(authority);
            let window = tauri::WebviewWindowBuilder::new(
                &app,
                crate::infrastructure::platform::window_lifecycle::STARTUP_FAILURE_WINDOW_LABEL,
                Default::default(),
            )
            .build()
            .expect("build startup command gate window");

            let error = tauri::test::get_ipc_response(
                &window,
                invoke_request("record_normal_command_effect"),
            )
            .expect_err("normal command must be rejected before its handler");
            assert_eq!(
                error,
                serde_json::json!({ "type": "application_unavailable" })
            );
            assert_eq!(effects.load(Ordering::SeqCst), 0);

            let startup = tauri::test::get_ipc_response(
                &window,
                invoke_request("get_application_startup_outcome"),
            );
            if app
                .try_state::<Arc<crate::usecase::application_startup::ApplicationStartupAuthority>>(
                )
                .is_some()
            {
                let startup = startup
                    .expect("failed authority must expose its startup outcome")
                    .deserialize::<serde_json::Value>()
                    .expect("decode startup outcome");
                assert_eq!(startup["type"], "failed");
            } else {
                assert_eq!(
                    startup.expect_err("missing authority must reject even startup commands"),
                    serde_json::json!({ "type": "application_unavailable" })
                );
            }
        }
    }

    fn canonical_command_names() -> &'static [&'static str] {
        &[
            "abort_workflow",
            "ack_terminal_surface_output",
            "acknowledge_agent_attempt",
            "add_repo_path",
            "append_review_comment",
            "approve_workspace_node",
            "approve_workflow_node",
            "attach_terminal_surface",
            "archive_provider_agent_session",
            "archive_workspace_workflow_execution",
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
            "close_workspace_node",
            "compute_hidden_ranges",
            "compute_hidden_ranges_from_content",
            "compute_markdown_diff_ranges",
            "compute_markdown_inline_chunks",
            "compute_markdown_split_rows",
            "compute_visible_markdown_blocks",
            "compact_application_shutdown_details",
            "create_provider_agent_session",
            "create_review_thread",
            "create_session",
            "create_workspace_session",
            "create_worktree",
            "delete_branch",
            "delete_facet",
            "delete_notion_config",
            "delete_review_thread",
            "delete_workflow",
            "detach_terminal_surface",
            "detect_editors",
            "diagnose_all_cmd",
            "duplicate_facet",
            "duplicate_workflow",
            "expand_pasted_text_blocks",
            "fetch_issues",
            "fetch_notion_label_options",
            "fetch_pr_status",
            "fork_session",
            "get_agent_session_display_window",
            "get_agent_session_notice",
            "list_agent_session_feedback",
            "retry_agent_session_feedback",
            "get_recovery_action",
            "get_stop_operation",
            "get_application_quit_operation",
            "get_application_startup_outcome",
            "get_shutdown_plan",
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
            "get_application_shutdown",
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
            "get_language_from_path",
            "get_main_repo_path",
            "get_notion_config",
            "get_notify_config",
            "get_or_spawn_terminal_surface",
            "get_performance_real_app_mode",
            "get_performance_telemetry_enabled",
            "get_pending_recovery_snapshot",
            "get_provider_agent_session",
            "get_terminal_performance_switches",
            "get_terminal_stream_endpoint",
            "get_terminal_surface",
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
            "get_session_lifecycle_operation",
            "get_session_status",
            "get_session_tool_output",
            "get_staged_content",
            "get_agent_send_operation",
            "get_status_diff_stats",
            "get_status_diff_stats_snapshot",
            "get_workflow",
            "get_workflow_config",
            "get_workflow_execution",
            "get_workflow_execution_log",
            "get_workflow_execution_state",
            "get_workflow_source",
            "get_workflow_node_detail",
            "get_workspace_status",
            "get_workspace_node_detail",
            "get_workspace_session_node_id",
            "get_workspace_tree_selection_reconciliation",
            "get_worktree_dirty_count",
            "git_create_branch",
            "git_stage",
            "git_stage_review_group",
            "git_unstage",
            "git_unstage_review_group",
            "init_agent_sessions",
            "kill_terminal_surface",
            "list_agent_backends",
            "list_branches",
            "list_branches_with_status",
            "list_branches_with_status_snapshot",
            "list_closed_sessions",
            "list_facet_summaries",
            "list_facets",
            "list_mentionable_files",
            "list_pending_agent_attempts",
            "list_pending_agent_recovery",
            "list_available_provider_agent_session_providers",
            "list_provider_agent_session_history",
            "list_provider_agent_sessions",
            "list_provider_hook_health_warnings",
            "list_terminal_surfaces",
            "list_review_threads",
            "list_session_statuses",
            "list_sessions",
            "list_workflow_executions",
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
            "present_agent_permission_request",
            "present_agent_tool_activity",
            "query_worktree_node_statuses",
            "sync_worktree_node_statuses",
            "query_notion_tasks",
            "quit_after_startup_failure",
            "reconcile_terminal_surfaces",
            "record_terminal_launch_renderer_phase",
            "remove_repo_path",
            "remove_worktree",
            "render_facet_preview",
            "report_agent_permission_request_observed",
            "get_agent_permission_response_operation",
            "report_frontend_error",
            "report_mounted_xterm_count",
            "report_usage_event",
            "resolve_pending_recovery_action",
            "stop_agent_session",
            "request_application_quit",
            "request_session_lifecycle",
            "resize_terminal_surface",
            "resolve_active_execution_by_worktree",
            "resolve_review_thread",
            "resolve_shutdown_target_action",
            "resolve_worktree_by_execution",
            "resume_workflow",
            "resume_agent_queue",
            "resume_provider_agent_session",
            "resume_provider_agent_session_history_candidate",
            "respond_agent_permission",
            "restore_session",
            "restore_provider_agent_session",
            "restore_workspace_workflow_execution",
            "save_facet",
            "save_notion_config",
            "save_workflow_source",
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
            "set_session_title",
            "start_git_dir_watching",
            "start_terminal_launch_performance_collection",
            "start_terminal_input_performance_collection",
            "start_watching",
            "start_workflow",
            "stop_workflow",
            "stop_watching",
            "sync_mentions_with_text",
            "take_terminal_launch_performance_samples",
            "take_terminal_input_performance_samples",
            "update_agent_session_notice",
            "dismiss_agent_session_feedback",
            "update_app_settings",
            "update_crash_reporting",
            "update_external_editor",
            "update_notify_config",
            "update_performance_telemetry",
            "update_webhook_url",
            "update_workflow_config",
            "validate_notion_config",
            "workflow_get_output",
            "workflow_submit_output",
            "workflow_validate_output",
            "write_paths_to_terminal_surface",
            "write_terminal_surface",
            "open_provider_agent_session",
            "delete_provider_agent_session",
            "confirm_provider_agent_session_archive_delete",
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
    fn retired_agent_command_palette_commands_are_not_registered() {
        let registered = registered_command_names();

        for retired_command in [
            "present_agent_command_palette",
            "is_agent_command_enabled",
            "get_agent_shortcut_settings",
            "update_agent_shortcut_settings",
            "reset_agent_shortcut_settings",
        ] {
            assert!(
                !registered.contains(&retired_command),
                "retired command must not remain public: {retired_command}"
            );
        }
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

#[cfg(test)]
#[path = "legacy_hook_registration_test.rs"]
mod legacy_hook_registration_tests;
