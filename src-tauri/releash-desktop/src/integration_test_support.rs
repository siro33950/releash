pub mod desktop {
    pub use crate::desktop::{apply_desktop_settings, record_window_ready};
    pub use crate::infrastructure::platform::window_lifecycle::{
        WindowPreferencesState, NORMAL_WINDOW_LABEL, STARTUP_FAILURE_WINDOW_LABEL,
    };
}
pub mod daemon_supervision {
    pub use crate::adaptor::gateway::daemon_supervision::{
        shutdown_response, supervised_connection_failure, DaemonProcessGateway, PendingConnection,
    };
    pub use crate::domain::daemon_supervision::{DaemonProcessPort, Failure, FailureStage};
    pub use crate::usecase::daemon_supervision::DaemonGateway;
}
pub mod desktop_client {
    pub use crate::adaptor::gateway::desktop_client::{
        client, error_message, liveness_failure, server_info, stream_client, DesktopClient,
        SettingsSubscription, POLICY,
    };
}
pub mod single_instance {
    pub use crate::infrastructure::platform::single_instance::acquire;
}
