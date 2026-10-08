//! テスト専用の生成型は、`test-support` の区画から利用します。
//!
//! ```compile_fail,E0432
//! use releashd::desktop_api::wire::StateChange;
//! ```
//!
//! ```compile_fail,E0432
//! use releashd::desktop_api::wire::Unit;
//! ```
//!
//! ```compile_fail,E0432
//! use releashd::desktop_api::to_rpc;
//! ```

pub mod wire {
    pub use crate::adaptor::presenter::client::{
        command_error, command_request, command_result, state_payload, state_subscription_event,
        CheckLoginRegistrationRequest, CommandError, DesktopSettings, LoginRegistrationResult,
        LoginRegistrationStatus, ServerInfo, StatePayload, StateSubscriptionEvent,
        StopDaemonRequest, StopDaemonResponse, UpdateLoginItemPreferenceRequest,
    };
}
pub use crate::adaptor::presenter::client_calls::call;
pub use crate::adaptor::presenter::connect_wire::{rpc, to_wire};
pub mod descriptor {
    pub use crate::adaptor::presenter::client::descriptor::{option, pool};
}
pub use crate::adaptor::gateway::app_config::read_config_if_exists;
pub use crate::adaptor::gateway::telemetry::TelemetryGateway;
#[cfg(feature = "test-support")]
pub use crate::client_api_acceptance::ClientEndpoint;
pub use crate::common::operation_context::{sleep, spawn_blocking, with_timeout};
pub use crate::common::retry::{RetryBackoff, RetryLimiter};
pub use crate::domain::failure::{TechnicalFailure, TechnicalFailureNature};
pub use crate::infrastructure::local_api::{
    lookup_process_start_time, process_start_time, LocalApiDiscovery, ProcessStartTimeLookup,
};
pub use crate::infrastructure::local_log::{init as init_local_log, LocalLogProcess};
pub use crate::infrastructure::platform::data_dir::{default_data_dir_for_profile, BuildProfile};
pub use crate::infrastructure::telemetry::metrics::{
    record_startup_from_origin, set_startup_origin, Startup,
};
pub use crate::infrastructure::telemetry::{init_telemetry, TelemetryGuard};
pub use crate::usecase::app_config::query_service::DesktopSettingsDto;
pub use crate::usecase::telemetry::TelemetryPort;
#[cfg(feature = "test-support")]
pub mod test_support {
    pub use crate::adaptor::presenter::client::{StateChange, Unit, COMMAND_NAMES};
    pub use crate::adaptor::presenter::connect::command_error;
    pub use crate::adaptor::presenter::connect_wire::to_rpc;
    pub use crate::adaptor::presenter::error::AppError;
    pub use crate::infrastructure::local_api::LocalApiDiscoveryFile;
    pub use crate::infrastructure::telemetry::crash::{crash_reporting_enabled, reset_for_tests};
    pub use crate::infrastructure::telemetry::metrics::{
        is_performance_active as performance_telemetry_active, lock_test_telemetry,
        reset_test_metrics, set_performance_configured, set_performance_enabled,
        test_metric_records,
    };
    pub fn performance_telemetry_configured() -> bool {
        use crate::infrastructure::telemetry::config;
        config::configured(config::endpoint(), config::license_key())
    }
    pub use crate::infrastructure::telemetry::test_helpers::{install_test_exporter, TEST_LOCK};
}
