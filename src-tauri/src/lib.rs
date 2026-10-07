mod adaptor;
#[cfg(feature = "test-support")]
mod agent_session_tui_acceptance;
#[cfg(feature = "test-support")]
mod client_api_acceptance;
mod common;
mod domain;
mod infrastructure;
#[cfg(feature = "test-support")]
mod provider_lifecycle_acceptance;
#[cfg(feature = "test-support")]
mod terminal_subscription_acceptance;
#[cfg(feature = "test-support")]
mod workflow_control_plane_acceptance;
#[cfg(feature = "test-support")]
mod workflow_delegate_acceptance;
#[cfg(feature = "test-support")]
mod workflow_diagnostics_acceptance;
#[cfg(feature = "test-support")]
mod terminal_surface {
    pub use crate::adaptor::controller::terminal_surface_runtime::{
        initialize_background_work_for_acceptance, BackgroundWork, TerminalSurfaceEventFault,
        TerminalSurfaceRuntime,
    };
    pub use crate::adaptor::presenter::terminal::{
        TerminalProcessLaunchV1, TerminalSurfaceOwnerV1, TerminalSurfaceStreamItemV1,
        TerminalSurfaceV1,
    };
}
pub mod desktop_api;
// Test-only helpers are intentionally kept as a root module.
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
mod usecase;

pub fn run_daemon(data_dir: Option<std::path::PathBuf>) -> i32 {
    infrastructure::process::parent_lifetime::watch_parent_pipe();
    let result = (|| -> Result<std::convert::Infallible, Box<dyn std::error::Error>> {
        let data_dir = match data_dir {
            Some(path) => path,
            None => infrastructure::platform::app_data_dir::resolve_data_dir()?,
        };
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        let provider_initial_search_path =
            infrastructure::process::search_path::capture_login_shell_path(
                infrastructure::process::search_path::LOGIN_SHELL_PATH_TIMEOUT,
            );
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        if let Ok(search_path) = &provider_initial_search_path {
            std::env::set_var("PATH", search_path);
        }
        infrastructure::platform::path_aliases::ensure_release_data_dir_env_for_resolved_path(
            &data_dir,
        );
        if let Err(error) = infrastructure::local_log::init(
            &data_dir,
            infrastructure::local_log::LocalLogProcess::Daemon,
        ) {
            eprintln!("{error}");
        }
        #[cfg(unix)]
        if let Err(error) = infrastructure::process::fd_limit::raise_open_file_limit() {
            log::warn!("failed to raise open file limit: {error}");
        }
        let runtime = tokio::runtime::Runtime::new()?;
        runtime.block_on(async {
            let daemon = adaptor::controller::daemon::compose(
                data_dir,
                #[cfg(any(target_os = "macos", target_os = "linux"))]
                provider_initial_search_path,
            )
            .await?;
            daemon.wait().await.map_err(Into::into)
        })
    })();
    let error = result.unwrap_err();
    log::logger().flush();
    eprintln!("{error}");
    1
}

#[doc(hidden)]
pub fn run_background_worker() -> i32 {
    adaptor::controller::background_worker::run()
}

#[cfg(any(test, feature = "test-support"))]
mod acceptance_test_support;
