//! 発見ファイルの読み取りエラーは、戻り値から扱います。
//!
//! ```compile_fail,E0432
//! use releash_lib::desktop_api::LocalApiDiscoveryReadError;
//! ```
//!
//! テスト専用の生成型は、`test-support` の区画から利用します。
//!
//! ```compile_fail,E0432
//! use releash_lib::desktop_api::wire::StateChange;
//! ```
//!
//! ```compile_fail,E0432
//! use releash_lib::desktop_api::wire::Unit;
//! ```
//!
//! ```compile_fail,E0432
//! use releash_lib::desktop_api::to_rpc;
//! ```

pub mod wire {
    pub use crate::adaptor::presenter::client::{
        application_quit_intent_dto_v1, application_quit_outcome_dto_v1, command_error,
        command_request, command_result, state_payload, state_subscription_event,
        ApplicationQuitIntentDtoV1, ApplicationQuitIntentDtoV1Exit,
        ApplicationQuitIntentDtoV1Restart, ApplicationQuitOutcomeDtoV1,
        ApplicationQuitRequestDtoV1, CommandError, DesktopSettings, RequestApplicationQuitRequest,
        ServerInfo, StatePayload, StateSubscriptionEvent, UpdateLoginItemPreferenceRequest,
    };
}
pub use crate::adaptor::presenter::client_calls::call;
pub use crate::adaptor::presenter::connect_wire::{rpc, to_wire};
pub mod descriptor {
    pub use crate::adaptor::presenter::client::descriptor::{option, pool};
}
pub use crate::adaptor::gateway::app_config::read_config_if_exists;
pub use crate::adaptor::gateway::local_api::ClientConnectionFileQuery;
pub use crate::adaptor::gateway::telemetry::TelemetryGateway;
#[cfg(feature = "test-support")]
pub use crate::client_api_acceptance::ClientEndpoint;
pub use crate::common::operation_context::{sleep, spawn_blocking, with_timeout};
pub use crate::common::retry::{RetryBackoff, RetryLimiter};
pub use crate::domain::failure::{TechnicalFailure, TechnicalFailureNature};
pub use crate::infrastructure::local_api::{
    lookup_process_start_time, process_start_time, read_local_api_discovery, LocalApiDiscovery,
    ProcessStartTimeLookup,
};
pub use crate::infrastructure::local_log::{init as init_local_log, LocalLogProcess};
pub use crate::infrastructure::platform::app_data_dir::resolve_data_dir;
pub use crate::infrastructure::process::parent_lifetime::terminate_descendants;
pub use crate::infrastructure::telemetry::init_telemetry;
pub use crate::infrastructure::telemetry::metrics::{
    record_startup_from_origin, set_startup_origin, Startup,
};
pub use crate::usecase::app_config::query_service::DesktopSettingsDto;
pub use crate::usecase::client_connection::{
    ClientConnectionDto, ClientConnectionError, ClientConnectionQueryService,
};
pub use crate::usecase::telemetry::TelemetryPort;
#[cfg(feature = "test-support")]
pub mod test_support {
    pub use crate::adaptor::presenter::client::{StateChange, Unit, COMMAND_NAMES};
    pub use crate::adaptor::presenter::connect::command_error;
    pub use crate::adaptor::presenter::connect_wire::to_rpc;
    pub use crate::adaptor::presenter::error::AppError;
    pub use crate::infrastructure::local_api::LocalApiDiscoveryFile;
    pub use crate::infrastructure::telemetry::crash::{
        crash_test_helpers::{install_test_exporter, TEST_LOCK},
        reset_for_tests,
    };
    pub use crate::infrastructure::telemetry::metrics::{
        lock_test_telemetry, reset_test_metrics, set_performance_configured,
        set_performance_enabled, test_metric_records,
    };
}
