pub(crate) mod tests {
    use super::super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tauri::Manager;

    #[test]
    fn test_起動状態の購読はsupervision操作として受理する() {
        use crate::domain::daemon_supervision::ShellOperation;

        // Given
        for command in ["subscribe_daemon_status", "stop_daemon_status_subscription"] {
            // When
            let operation = shell_operation(command);
            // Then
            assert_eq!(operation, ShellOperation::Supervision);
        }
    }

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
                .chain(["set_menu_items_enabled"])
                .collect::<Vec<_>>()
        );
        for command in releashd::desktop_api::test_support::COMMAND_NAMES {
            assert_eq!(router.domain_route_index(command), None, "{command}");
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
            ("menu", menu::COMMAND_NAMES, menu::register),
        ]
    }

    pub(crate) fn registered_command_names() -> Vec<&'static str> {
        releashd::desktop_api::test_support::COMMAND_NAMES
            .iter()
            .copied()
            .chain(desktop_lifecycle::COMMAND_NAMES.iter().copied())
            .chain(client::COMMAND_NAMES.iter().copied())
            .chain(menu::COMMAND_NAMES.iter().copied())
            .collect()
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
        tauri::generate_handler![record_normal_command_effect]
    }

    fn command_gate_test_app() -> (tauri::App<tauri::test::MockRuntime>, Arc<AtomicUsize>) {
        let effects = Arc::new(AtomicUsize::new(0));
        let builder = tauri::test::mock_builder().manage(effects.clone());
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

    #[tokio::test(start_paused = true)]
    async fn test_起動中ipc_通常handlerの副作用をrust入口で拒否する() {
        // Given
        let gateway = Arc::new(crate::usecase::test_helpers::FakeDaemon::default());
        let supervisor = crate::usecase::test_helpers::start_supervision(gateway.clone());
        let (app, effects) = command_gate_test_app();
        app.manage(supervisor.clone());
        let window = tauri::WebviewWindowBuilder::new(&app, "startup-failure", Default::default())
            .build()
            .unwrap();
        // When / Then
        for command in [
            "record_normal_command_effect",
            "set_login_item_enabled",
            "install_cli",
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
    fn test_共有dispatch_対象外commandは既存domain_handlerとfallbackへ届く() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        // Given
        let effects = Arc::new(AtomicUsize::new(0));
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
