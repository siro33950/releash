pub(crate) mod commands;

const COMMAND_NAMES: &[&str] = &[
    "detect_editors",
    "open_in_editor",
    "open_folder_in_editor",
    "get_external_editor",
    "update_external_editor",
];

pub(crate) fn register(router: &mut super::CommandRouter) {
    router.register_domain(COMMAND_NAMES, Box::new(invoke_handler()));
}

pub(crate) fn invoke_handler(
) -> impl Fn(tauri::ipc::Invoke<tauri::Wry>) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        commands::detect_editors,
        commands::open_in_editor,
        commands::open_folder_in_editor,
        commands::get_external_editor,
        commands::update_external_editor,
    ]
}
