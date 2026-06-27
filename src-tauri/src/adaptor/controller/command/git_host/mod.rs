pub(crate) mod issue;
pub(crate) mod pr;

use crate::other::AppError;

const COMMAND_NAMES: &[&str] = &[
    "check_pr_provider_status",
    "fetch_pr_status",
    "get_cached_pr_status",
    "fetch_issues",
    "get_cached_issues",
];

pub(crate) fn register(router: &mut super::CommandRouter) {
    router.register_domain(COMMAND_NAMES, Box::new(invoke_handler()));
}

pub(crate) fn invoke_handler(
) -> impl Fn(tauri::ipc::Invoke<tauri::Wry>) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        pr::check_pr_provider_status,
        pr::fetch_pr_status,
        pr::get_cached_pr_status,
        issue::fetch_issues,
        issue::get_cached_issues,
    ]
}

pub(super) async fn run_blocking<T, F>(f: F) -> Result<T, AppError>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| AppError::new(format!("task join error: {e}")))
}
