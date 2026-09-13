pub(crate) const COMMAND_NAMES: &[&str] = &[
    "report_frontend_error",
    "report_mounted_xterm_count",
    "report_usage_event",
];

pub(crate) fn register(router: &mut super::CommandRouter) {
    router.register_domain(
        COMMAND_NAMES,
        Box::new(super::client::handle_registered_invoke),
    );
}
