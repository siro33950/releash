pub(crate) mod commands;

pub(super) const COMMAND_NAMES: &[&str] = &[
    "query_notion_tasks",
    "fetch_notion_label_options",
    "save_notion_config",
    "get_notion_config",
    "delete_notion_config",
    "validate_notion_config",
];

pub(crate) fn register(router: &mut super::CommandRouter) {
    router.register_domain(COMMAND_NAMES, Box::new(invoke_handler()));
}

pub(crate) fn invoke_handler(
) -> impl Fn(tauri::ipc::Invoke<tauri::Wry>) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        commands::query_notion_tasks,
        commands::fetch_notion_label_options,
        commands::save_notion_config,
        commands::get_notion_config,
        commands::delete_notion_config,
        commands::validate_notion_config,
    ]
}
