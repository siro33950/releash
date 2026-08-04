pub(crate) mod commands;

pub(super) const COMMAND_NAMES: &[&str] = &[
    "attach_pty",
    "detach_pty",
    "write_pty",
    "write_paths_to_pty",
    "resize_pty",
    "kill_pty",
    "list_terminal_surfaces",
    "reconcile_terminal_surfaces",
    "get_terminal_surface",
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
        commands::attach_pty,
        commands::detach_pty,
        commands::write_pty,
        commands::write_paths_to_pty,
        commands::resize_pty,
        commands::kill_pty,
        commands::list_terminal_surfaces,
        commands::reconcile_terminal_surfaces,
        commands::get_terminal_surface,
        commands::get_or_spawn_pty,
        commands::kill_ptys_by_worktree,
        commands::gc_ptys_for_worktree,
    ]
}
