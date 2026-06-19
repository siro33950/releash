pub(crate) mod commands;

const COMMAND_NAMES: &[&str] = &["load_workspace_state", "save_workspace_state"];

pub(crate) fn register(router: &mut super::CommandRouter) {
    router.register_domain(COMMAND_NAMES, Box::new(invoke_handler()));
}

pub(crate) fn invoke_handler(
) -> impl Fn(tauri::ipc::Invoke<tauri::Wry>) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        commands::load_workspace_state,
        commands::save_workspace_state
    ]
}
