pub(crate) mod commands;

pub(super) const COMMAND_NAMES: &[&str] = &[
    "list_review_threads",
    "get_review_thread",
    "create_review_thread",
    "append_review_comment",
    "resolve_review_thread",
    "delete_review_thread",
    "get_review_thread_history",
    "build_review_thread_handoff",
];

pub(crate) fn register(router: &mut super::CommandRouter) {
    router.register_domain(COMMAND_NAMES, Box::new(invoke_handler()));
}

pub(crate) fn invoke_handler(
) -> impl Fn(tauri::ipc::Invoke<tauri::Wry>) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        commands::list_review_threads,
        commands::get_review_thread,
        commands::create_review_thread,
        commands::append_review_comment,
        commands::resolve_review_thread,
        commands::delete_review_thread,
        commands::get_review_thread_history,
        commands::build_review_thread_handoff,
    ]
}
