pub(crate) const COMMAND_NAMES: &[&str] = &[
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
    router.register_domain(
        COMMAND_NAMES,
        Box::new(super::client::handle_registered_invoke),
    );
}
