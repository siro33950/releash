pub(crate) mod commands;

const COMMAND_NAMES: &[&str] = &[
    "write_pty",
    "resize_pty",
    "kill_pty",
    "list_pty_sessions",
    "reconcile_pty_sessions",
    "get_pty_buffered_output",
    "get_or_spawn_pty",
    "kill_ptys_by_worktree",
    "gc_ptys_for_worktree",
    "register_active_terminal",
    "unregister_active_terminal",
];

pub(crate) fn register(router: &mut super::CommandRouter) {
    router.register_domain(COMMAND_NAMES, Box::new(invoke_handler()));
}

pub(crate) fn invoke_handler(
) -> impl Fn(tauri::ipc::Invoke<tauri::Wry>) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        commands::write_pty,
        commands::resize_pty,
        commands::kill_pty,
        commands::list_pty_sessions,
        commands::reconcile_pty_sessions,
        commands::get_pty_buffered_output,
        commands::get_or_spawn_pty,
        commands::kill_ptys_by_worktree,
        commands::gc_ptys_for_worktree,
        commands::register_active_terminal,
        commands::unregister_active_terminal,
    ]
}
