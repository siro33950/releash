pub(crate) const COMMAND_NAMES: &[&str] = &[
    "fetch_pr_status",
    "get_cached_pr_status",
    "fetch_issues",
    "get_cached_issues",
];

pub(crate) fn register(router: &mut super::CommandRouter) {
    router.register_domain(
        COMMAND_NAMES,
        Box::new(super::client::handle_registered_invoke),
    );
}
