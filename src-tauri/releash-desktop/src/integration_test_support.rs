pub mod desktop {
    pub use crate::desktop::apply_desktop_settings;
    pub use crate::infrastructure::platform::window_lifecycle::NORMAL_WINDOW_LABEL;
    pub use crate::infrastructure::platform::window_lifecycle::STARTUP_FAILURE_WINDOW_LABEL;
}
pub mod desktop_client {
    pub use crate::adaptor::gateway::desktop_client::client;
    pub use crate::adaptor::gateway::desktop_client::connection_failure;
    pub use crate::adaptor::gateway::desktop_client::error_message;
    pub use crate::adaptor::gateway::desktop_client::stream_client;
    pub use crate::adaptor::gateway::desktop_client::DesktopClient;
    pub use crate::adaptor::gateway::desktop_client::SettingsSubscription;
    pub use crate::adaptor::gateway::desktop_client::POLICY;
}
pub mod single_instance {
    pub use crate::infrastructure::platform::single_instance::acquire;
}

pub mod login_item {
    pub use crate::infrastructure::platform::login_item::registration_location;
}

pub mod daemon_connection {
    pub use crate::adaptor::gateway::daemon_connection::DaemonConnection;
    pub use crate::domain::daemon_connection::{
        ConnectionError, DaemonConnectionPort, DaemonEndpoint,
    };
    pub use crate::usecase::daemon_connection::DaemonConnectionUsecase;
    pub use crate::usecase::daemon_connection_query::{
        ConnectionFailure, DaemonConnectionQueryService,
    };
}
