mod adaptor;
#[cfg(all(debug_assertions, feature = "desktop"))]
pub mod agent_session_tui_acceptance;
pub mod cli;
#[cfg(all(debug_assertions, feature = "desktop"))]
pub mod client_api_acceptance;
mod domain;
mod infrastructure;
mod other;
#[cfg(all(debug_assertions, feature = "desktop"))]
pub mod provider_lifecycle_acceptance;
#[cfg(all(debug_assertions, feature = "desktop"))]
pub mod workflow_control_plane_acceptance;
#[cfg(all(debug_assertions, feature = "desktop"))]
pub mod workflow_diagnostics_acceptance;
pub mod terminal_surface {
    pub use crate::adaptor::controller::terminal_surface_runtime::{
        TerminalSurfaceEventFault, TerminalSurfaceEventFaultController, TerminalSurfaceRuntime,
        TerminalSurfaceWireAttachment,
    };
    pub use crate::adaptor::protocol::terminal::{
        GetOrSpawnTerminalV1, TerminalProcessLaunchV1, TerminalSurfaceOwnerV1,
        TerminalSurfaceStreamItemV1, TerminalSurfaceV1,
    };
}
// Test-only helpers are intentionally kept as a root module.
#[cfg(test)]
mod test_support;
mod usecase;

#[cfg(feature = "desktop")]
mod desktop;
#[cfg(all(debug_assertions, feature = "desktop"))]
use desktop::application_context;
#[cfg(feature = "desktop")]
pub use desktop::run;

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

#[cfg(all(debug_assertions, feature = "desktop"))]
mod desktop_test_support;
