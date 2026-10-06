#[path = "support/internal/adaptor/gateway/daemon_supervision_test.rs"]
mod daemon_supervision_tests;
#[path = "support/internal/adaptor/gateway/desktop_client_test.rs"]
mod desktop_client_tests;
#[path = "support/internal/desktop_test.rs"]
mod desktop_tests;
#[cfg(unix)]
#[path = "support/internal/infrastructure/platform/single_instance_test.rs"]
mod single_instance_tests;
