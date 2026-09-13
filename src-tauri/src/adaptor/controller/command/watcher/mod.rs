pub(crate) const COMMAND_NAMES: &[&str] =
    &["start_watching", "start_git_dir_watching", "stop_watching"];

pub(crate) fn register(router: &mut super::CommandRouter) {
    router.register_domain(
        COMMAND_NAMES,
        Box::new(super::client::handle_registered_invoke),
    );
}
