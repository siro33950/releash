pub(crate) mod tests {
    use super::super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
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
