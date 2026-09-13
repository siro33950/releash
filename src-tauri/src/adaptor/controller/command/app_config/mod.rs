pub(crate) const COMMAND_NAMES: &[&str] = &[
    "update_performance_telemetry",
    "get_workflow_config",
    "update_workflow_config",
    "get_app_settings",
    "update_app_settings",
    "get_crash_reporting_enabled",
    "get_performance_telemetry_enabled",
    "update_crash_reporting",
];

pub(crate) fn register(router: &mut super::CommandRouter) {
    router.register_domain(
        COMMAND_NAMES,
        Box::new(super::client::handle_registered_invoke),
    );
}
