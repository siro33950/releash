pub use crate::desktop_client_acceptance::{
    apply_desktop_update, command_rejection_app, desktop_client_endpoint, desktop_connection_app,
    desktop_supervision_status, desktop_window_preferences, initialize_desktop_settings,
    spawn_desktop_successor, stop_desktop_daemon, terminate_daemon_for_acceptance,
    wait_for_desktop_predecessor,
};
#[cfg(unix)]
pub mod cli_install {
    pub use crate::infrastructure::platform::cli_install::{
        install_cli_symlink_with_runner, CliInstallStatus,
    };
}
#[cfg(feature = "test-support")]
#[path = "integration_test_support.rs"]
pub mod integration;
