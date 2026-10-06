#[path = "internal/daemon_supervision.rs"]
mod daemon_supervision_tests;
#[path = "internal/desktop_client.rs"]
mod desktop_client_tests;
#[path = "internal/desktop.rs"]
mod desktop_tests;
#[cfg(unix)]
#[path = "internal/single_instance.rs"]
mod single_instance_tests;
