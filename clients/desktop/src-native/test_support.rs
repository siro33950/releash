pub use crate::desktop_client_acceptance::{
    apply_desktop_update, apply_observed_desktop_settings, command_rejection_app,
    desktop_client_endpoint, desktop_connection_app, desktop_connection_app_parts,
    desktop_connection_status, desktop_login_item_calls, desktop_login_preference,
    desktop_window_preferences, initialize_desktop, initialize_desktop_settings,
    initialize_desktop_with_settings_applied, show_desktop, spawn_desktop_successor,
    start_desktop_daemon, stop_desktop_daemon, wait_for_desktop_predecessor,
};
#[cfg(feature = "test-support")]
#[path = "integration_test_support.rs"]
pub mod integration;

#[cfg(target_os = "macos")]
pub use crate::desktop_client_acceptance::{
    dispatch_desktop_tray_quit, install_desktop_native_quit,
};

#[cfg(target_os = "macos")]
pub use crate::desktop_client_acceptance::quit_desktop;
