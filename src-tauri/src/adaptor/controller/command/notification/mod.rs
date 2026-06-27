pub(crate) mod commands;

pub(super) const COMMAND_NAMES: &[&str] = &[
    "get_notify_config",
    "update_notify_config",
    "update_webhook_url",
];

pub(crate) fn register(router: &mut super::CommandRouter) {
    router.register_domain(COMMAND_NAMES, Box::new(invoke_handler()));
}

pub(crate) fn invoke_handler(
) -> impl Fn(tauri::ipc::Invoke<tauri::Wry>) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        commands::get_notify_config,
        commands::update_notify_config,
        commands::update_webhook_url,
    ]
}
