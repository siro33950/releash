pub(crate) const COMMAND_NAMES: &[&str] = &[
    "get_application_startup_outcome",
    "quit_after_startup_failure",
];

pub(crate) fn register(router: &mut super::CommandRouter) {
    router.register_domain(COMMAND_NAMES, Box::new(invoke_handler()));
}

pub(crate) fn invoke_handler<R: tauri::Runtime>(
) -> impl Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync {
    tauri::generate_handler![get_application_startup_outcome, quit_after_startup_failure]
}

use crate::adaptor::controller::client::application_lifecycle::{
    get_application_startup_outcome_shared, quit_after_startup_failure_shared,
};
use crate::adaptor::protocol::application_lifecycle_v1::{
    ApplicationStartupOutcomeDtoV1, StartupFailureQuitOutcomeDtoV1,
};
use std::sync::Arc;
#[tauri::command]
pub(crate) fn get_application_startup_outcome(
    authority: tauri::State<
        '_,
        Arc<crate::usecase::application_startup::ApplicationStartupAuthority>,
    >,
) -> ApplicationStartupOutcomeDtoV1 {
    get_application_startup_outcome_shared(authority.inner())
}
#[tauri::command]
pub(crate) fn quit_after_startup_failure(
    authority: tauri::State<
        '_,
        Arc<crate::usecase::application_startup::ApplicationStartupAuthority>,
    >,
) -> Result<
    StartupFailureQuitOutcomeDtoV1,
    crate::usecase::application_startup::ApplicationUnavailable,
> {
    quit_after_startup_failure_shared(authority.inner())
}
