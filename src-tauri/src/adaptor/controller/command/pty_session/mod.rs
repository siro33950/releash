pub(crate) mod commands;

const COMMAND_NAMES: &[&str] = &[
    "spawn_pty",
    "write_pty",
    "resize_pty",
    "kill_pty",
    "list_pty_sessions",
    "get_or_spawn_pty",
    "kill_ptys_by_worktree",
    "gc_ptys_for_worktree",
];

pub(crate) fn register(router: &mut super::CommandRouter) {
    router.register_domain(COMMAND_NAMES, Box::new(invoke_handler()));
}

pub(crate) fn invoke_handler(
) -> impl Fn(tauri::ipc::Invoke<tauri::Wry>) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        commands::spawn_pty,
        commands::write_pty,
        commands::resize_pty,
        commands::kill_pty,
        commands::list_pty_sessions,
        commands::get_or_spawn_pty,
        commands::kill_ptys_by_worktree,
        commands::gc_ptys_for_worktree,
    ]
}
