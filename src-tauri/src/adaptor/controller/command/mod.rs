pub(crate) mod agent_session;
pub(crate) mod app_config;
pub(crate) mod application_lifecycle;
pub(crate) mod code;
pub(crate) mod comment;
pub(crate) mod external_editor;
pub(crate) mod git_host;
pub(crate) mod menu;
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
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tauri::Manager;

    fn dummy_handler() -> InvokeHandler {
        Box::new(|_invoke| true)
    }

    #[test]
    fn test_agent_session_commandを本番routerの単一routeとして登録する() {
        let mut router = CommandRouter::new(dummy_handler());

        agent_session::register(&mut router);

        assert_eq!(router.domains.len(), 1);
        assert!(router.domains.iter().any(|route| {
            route.command_names.contains(&"create_agent_session")
                && !route.command_names.contains(&"create_session")
        }));
    }

    #[test]
    fn test_agent_tui_atomic_cutover_agent_sessionはcanonicalな単一routeだけを登録する() {
        let mut router = CommandRouter::new(dummy_handler());

        agent_session::register(&mut router);

        assert_eq!(router.domains.len(), 1);
        let commands = router.domains[0].command_names;
        assert_eq!(
            commands,
            [
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
            ]
        );
        let registered = registered_command_names();
        for removed in [
            "create_session",
            "create_workspace_session",
            "fork_session",
            "restore_session",
            "init_agent_sessions",
            "list_sessions",
            "list_closed_sessions",
            "get_session",
            "get_session_page",
            "get_session_status",
            "get_session_attachment",
            "get_session_tool_output",
            "request_session_lifecycle",
            "get_session_lifecycle_operation",
            "send_agent_message",
            "respond_agent_permission",
            "stop_agent_session",
            "resume_agent_queue",
            "cancel_agent_queued_turn",
            "set_agent_model",
            "set_agent_permission_mode",
            "set_agent_plan_mode",
            "set_session_title",
            "search_agent_sessions",
            "search_agent_session_messages",
            "list_agent_backends",
            "scan_agent_skills",
            "prepare_image_attachment",
            "prepare_image_attachments_from_paths",
            "prepare_pasted_text_block",
            "expand_pasted_text_blocks",
            "present_agent_permission_request",
            "present_agent_tool_activity",
            "report_agent_permission_request_observed",
            "get_agent_permission_response_operation",
            "get_agent_send_operation",
            "get_agent_session_display_window",
            "get_agent_session_notice",
            "update_agent_session_notice",
            "list_agent_session_feedback",
            "retry_agent_session_feedback",
            "dismiss_agent_session_feedback",
            "send_workflow_approval_chat_message",
            "acknowledge_agent_attempt",
            "list_pending_agent_attempts",
            "list_pending_agent_recovery",
            "get_pending_recovery_snapshot",
            "resolve_pending_recovery_action",
            "get_recovery_action",
            "get_stop_operation",
            "plan_agent_chat_eviction",
            "build_agent_edit_preview",
            "build_agent_edited_tool_input",
            "build_agent_edited_multi_edit_tool_input",
            "build_agent_edited_multi_edit_tool_input_all",
            "build_agent_prompt_suggestion",
            "build_agent_task_list_report",
        ] {
            assert!(
                !registered.contains(&removed),
                "still registered: {removed}"
            );
        }
    }

    #[test]
    fn test_agent_tui_atomic_cutover_compositionはlegacy_agent_runtimeを要求しない() {
        let composition = include_str!("../../../lib.rs");

        for removed in [
            "AgentBackendRegistry",
            "AgentSessionRuntimeUsecase",
            "compose_agent_session_runtime",
        ] {
            assert!(
                !composition.contains(removed),
                "legacy Agent runtime remains in production composition: {removed}"
            );
        }
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

        assert_eq!(router.domain_route_index("fetch_pr_status"), Some(0));
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
