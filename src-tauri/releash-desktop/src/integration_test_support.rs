pub mod desktop {
    pub use crate::desktop::apply_desktop_settings;
    pub use crate::infrastructure::platform::window_lifecycle::NORMAL_WINDOW_LABEL;
    pub use crate::infrastructure::platform::window_lifecycle::STARTUP_FAILURE_WINDOW_LABEL;
}
pub mod daemon_supervision {
    pub use crate::adaptor::gateway::daemon_supervision::DaemonProcessGateway;
    pub use crate::adaptor::gateway::daemon_supervision::PendingConnection;
    pub use crate::domain::daemon_supervision::DaemonProcessPort;
    pub use crate::domain::daemon_supervision::Failure;
    pub use crate::domain::daemon_supervision::FailureStage;
    pub use crate::usecase::daemon_supervision::DaemonGateway;
}
pub mod desktop_client {
    pub use crate::adaptor::gateway::desktop_client::client;
    pub use crate::adaptor::gateway::desktop_client::error_message;
    pub use crate::adaptor::gateway::desktop_client::liveness_failure;
    pub use crate::adaptor::gateway::desktop_client::server_info;
    pub use crate::adaptor::gateway::desktop_client::stream_client;
    pub use crate::adaptor::gateway::desktop_client::DesktopClient;
    pub use crate::adaptor::gateway::desktop_client::SettingsSubscription;
    pub use crate::adaptor::gateway::desktop_client::POLICY;
}
pub mod single_instance {
    pub use crate::infrastructure::platform::single_instance::acquire;
}
