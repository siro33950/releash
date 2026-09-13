pub(crate) const COMMAND_NAMES: &[&str] = &[
    "detect_editors",
    "open_in_editor",
    "open_folder_in_editor",
    "get_external_editor",
    "update_external_editor",
];

pub(crate) fn register(router: &mut super::CommandRouter) {
    router.register_domain(
        COMMAND_NAMES,
        Box::new(super::client::handle_registered_invoke),
    );
}
