pub(crate) const COMMAND_NAMES: &[&str] = &["load_workspace_state", "save_workspace_state"];

pub(crate) fn register(router: &mut super::CommandRouter) {
    router.register_domain(
        COMMAND_NAMES,
        Box::new(super::client::handle_registered_invoke),
    );
}
