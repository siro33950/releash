pub(crate) const COMMAND_NAMES: &[&str] = &[
    "query_notion_tasks",
    "fetch_notion_label_options",
    "save_notion_config",
    "get_notion_config",
    "delete_notion_config",
    "validate_notion_config",
];

pub(crate) fn register(router: &mut super::CommandRouter) {
    router.register_domain(
        COMMAND_NAMES,
        Box::new(super::client::handle_registered_invoke),
    );
}
