pub(crate) mod commands;

const COMMAND_NAMES: &[&str] = &[
    "generate_hooks_config",
    "apply_hooks_config",
    "get_hooks_status",
];

pub(crate) fn register(router: &mut super::CommandRouter) {
    router.register_domain(COMMAND_NAMES, Box::new(invoke_handler()));
}

pub(crate) fn invoke_handler(
) -> impl Fn(tauri::ipc::Invoke<tauri::Wry>) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        commands::generate_hooks_config,
        commands::apply_hooks_config,
        commands::get_hooks_status,
    ]
}
