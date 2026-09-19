pub(crate) mod application_lifecycle;
pub(crate) mod client;
pub(crate) mod desktop_lifecycle;
pub(crate) mod menu;

type InvokeHandler<R = tauri::Wry> = Box<dyn Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync>;

pub(crate) struct CommandRouter<H = InvokeHandler> {
    fallback: H,
    domains: Vec<CommandDomainRoute<H>>,
}

struct CommandDomainRoute<H> {
    command_names: &'static [&'static str],
    handler: H,
}

use super::client::command_admitted;
#[cfg(test)]
use super::client::dispatch::STARTUP_COMMANDS;

fn shell_operation(command: &str) -> crate::domain::daemon_supervision::ShellOperation {
    use crate::domain::daemon_supervision::ShellOperation;
    match command {
        "get_daemon_status"
        | "retry_daemon"
        | "quit_desktop"
        | "get_application_startup_outcome"
        | "quit_after_startup_failure"
        | "validate_daemon_connection"
        | "get_client_endpoint"
        | "fail_desktop_restoration"
        | "complete_desktop_restoration" => ShellOperation::Supervision,
        "get_login_item_status" => ShellOperation::RestoreState,
        "apply_desktop_settings" => ShellOperation::ApplySettings,
        _ => ShellOperation::Normal,
    }
}

pub(crate) fn gate_invoke_before_domain_routing<R: tauri::Runtime>(
    invoke: tauri::ipc::Invoke<R>,
) -> Result<tauri::ipc::Invoke<R>, bool> {
    let admitted = {
        let authority = invoke.message.state_ref().try_get::<std::sync::Arc<
            crate::usecase::application_startup::ApplicationStartupAuthority,
        >>();
        let supervisor = invoke.message.state_ref().try_get::<std::sync::Arc<crate::usecase::daemon_supervision::DaemonSupervisionUsecase>>();
        supervisor.is_none_or(|supervisor| {
            supervisor.command_admitted(shell_operation(invoke.message.command()))
        }) && command_admitted(
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

impl<H> CommandRouter<H> {
    pub(crate) fn new(fallback: H) -> Self {
        Self {
            fallback,
            domains: Vec::new(),
        }
    }

    pub(crate) fn register_domain(&mut self, command_names: &'static [&'static str], handler: H) {
        self.domains.push(CommandDomainRoute {
            command_names,
            handler,
        });
    }

    fn resolve(&self, command: &str) -> &H {
        self.domain_route_index(command)
            .map(|index| &self.domains[index].handler)
            .unwrap_or(&self.fallback)
    }

    fn domain_route_index(&self, command: &str) -> Option<usize> {
        self.domains
            .iter()
            .position(|domain| domain.command_names.contains(&command))
    }
}

impl<R: tauri::Runtime> CommandRouter<InvokeHandler<R>> {
    pub(crate) fn handle(&self, invoke: tauri::ipc::Invoke<R>) -> bool {
        let invoke = match gate_invoke_before_domain_routing(invoke) {
            Ok(invoke) => invoke,
            Err(handled) => return handled,
        };
        (self.resolve(invoke.message.command()))(invoke)
    }
}

pub(crate) fn register_all(builder: tauri::Builder<tauri::Wry>) -> tauri::Builder<tauri::Wry> {
    let app_handler: InvokeHandler = Box::new(|_invoke| false);
    let mut router = CommandRouter::new(app_handler);
    register_shell_commands(&mut router);
    builder.invoke_handler(move |invoke: tauri::ipc::Invoke<tauri::Wry>| router.handle(invoke))
}

fn register_shell_commands(router: &mut CommandRouter) {
    desktop_lifecycle::register(router);
    client::register(router);
    application_lifecycle::register(router);
    menu::register(router);
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
    fn test_本番tauri受付はshellのcommandだけを登録する() {
        // Given
        let mut router = CommandRouter::new(dummy_handler());
        // When
        register_shell_commands(&mut router);
        // Then
        let registered: Vec<_> = router
            .domains
            .iter()
            .flat_map(|domain| domain.command_names.iter().copied())
            .collect();
        assert_eq!(
            registered,
            desktop_lifecycle::COMMAND_NAMES
                .iter()
                .copied()
                .chain(client::COMMAND_NAMES.iter().copied())
                .chain([
                    "get_application_startup_outcome",
                    "quit_after_startup_failure",
                    "set_menu_items_enabled",
                ])
                .collect::<Vec<_>>()
        );
        for command in crate::adaptor::controller::api::protocol::client::COMMAND_NAMES {
            if !STARTUP_COMMANDS.contains(command) {
                assert_eq!(router.domain_route_index(command), None, "{command}");
            }
        }
    }

    #[test]
    fn test_廃止済みagent_commandをprotocolに残さない() {
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

    type RegisterFn = fn(&mut CommandRouter);

    fn command_domains() -> Vec<(&'static str, &'static [&'static str], RegisterFn)> {
        vec![
            (
                "desktop_lifecycle",
                desktop_lifecycle::COMMAND_NAMES,
                desktop_lifecycle::register,
            ),
            ("client", client::COMMAND_NAMES, client::register),
            (
                "application_lifecycle",
                application_lifecycle::COMMAND_NAMES,
                application_lifecycle::register,
            ),
            ("menu", menu::COMMAND_NAMES, menu::register),
        ]
    }

    pub(super) fn registered_command_names() -> Vec<&'static str> {
        crate::adaptor::controller::api::protocol::client::COMMAND_NAMES
            .iter()
            .copied()
            .chain(desktop_lifecycle::COMMAND_NAMES.iter().copied())
            .chain(client::COMMAND_NAMES.iter().copied())
            .chain(menu::COMMAND_NAMES.iter().copied())
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

    #[tokio::test(start_paused = true)]
    async fn test_起動中ipc_通常handlerの副作用をrust入口で拒否する() {
        // Given
        let gateway = Arc::new(crate::usecase::test_helpers::FakeDaemon::default());
        let supervisor =
            crate::usecase::daemon_supervision::DaemonSupervisionUsecase::start(gateway.clone());
        let (app, effects) = command_gate_test_app(Some(Arc::new(
            crate::usecase::application_startup::ApplicationStartupAuthority::ready(),
        )));
        app.manage(supervisor.clone());
        let window = tauri::WebviewWindowBuilder::new(&app, "startup-failure", Default::default())
            .build()
            .unwrap();
        // When / Then
        for command in [
            "record_normal_command_effect",
            "set_login_item_enabled",
            "install_cli",
            "apply_desktop_settings",
        ] {
            let error =
                tauri::test::get_ipc_response(&window, invoke_request(command)).unwrap_err();
            assert_eq!(
                error,
                serde_json::json!({ "type": "application_unavailable" })
            );
        }
        assert_eq!(effects.load(Ordering::SeqCst), 0);
        // When
        gateway.ready.store(true, Ordering::SeqCst);
        crate::usecase::test_helpers::tick(200).await;
        crate::usecase::test_helpers::restore_desktop(&supervisor).await;
        // Then
        assert!(tauri::test::get_ipc_response(
            &window,
            invoke_request("record_normal_command_effect")
        )
        .is_ok());
        assert_eq!(effects.load(Ordering::SeqCst), 1);
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
    fn test_共有dispatch_対象外commandは既存domain_handlerとfallbackへ届く() {
        use crate::adaptor::controller::client::ClientCommandDispatch;
        use crate::usecase::application_startup::ApplicationStartupAuthority;
        use std::sync::atomic::{AtomicUsize, Ordering};
        // Given
        let effects = Arc::new(AtomicUsize::new(0));
        let authority = Arc::new(ApplicationStartupAuthority::ready());
        let dispatch = Arc::new(ClientCommandDispatch::new(
            Arc::new(crate::adaptor::controller::wiring::build_repository_usecase()),
            authority.clone(),
        ));
        let mut router: CommandRouter<InvokeHandler<tauri::test::MockRuntime>> =
            CommandRouter::new(Box::new(|invoke| {
                invoke.resolver.resolve("fallback-result");
                true
            }));
        router.register_domain(
            &["record_normal_command_effect"],
            Box::new(tauri::generate_handler![record_normal_command_effect]),
        );
        let app = tauri::test::mock_builder()
            .manage(authority)
            .manage(dispatch)
            .manage(effects.clone())
            .invoke_handler(move |invoke| router.handle(invoke))
            .build(crate::application_context())
            .unwrap();
        let window = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .unwrap();
        // When / Then
        for (command, expected) in [
            ("record_normal_command_effect", "normal-effect-ran"),
            ("unregistered", "fallback-result"),
        ] {
            let result = tauri::test::get_ipc_response(&window, invoke_request(command))
                .unwrap()
                .deserialize::<String>()
                .unwrap();
            assert_eq!(result, expected);
        }
        assert_eq!(effects.load(Ordering::SeqCst), 1);
    }
}

#[cfg(test)]
#[path = "legacy_hook_registration_test.rs"]
mod legacy_hook_registration_tests;
