//! Strict public V1 DTOs for application quit and shutdown.
//!
//! These transport shapes deliberately contain no repository or usecase
//! behavior. Tauri and loopback WebSocket adapters share them through the
//! application-lifecycle presenter.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum ApplicationStartupOutcomeDtoV1 {
    Ready,
    Failed {
        kind: StartupFailureKindDtoV1,
        #[serde(rename = "safeDescription")]
        safe_description: String,
        #[serde(rename = "correlationId")]
        correlation_id: String,
        #[serde(rename = "retryOnNextLaunch")]
        retry_on_next_launch: bool,
        actions: [StartupFailureActionDtoV1; 1],
    },
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StartupFailureKindDtoV1 {
    StoreInUse,
    StorageUnavailable,
    UnsupportedRuntime,
    UnsupportedStoreVersion,
    InitializationStateInvalid,
    StoreValidationFailed,
    SchemaEvolutionFailed,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StartupFailureActionDtoV1 {
    Quit,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum StartupFailureQuitOutcomeDtoV1 {
    Accepted {
        #[serde(rename = "correlationId")]
        correlation_id: String,
    },
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum ApplicationQuitIntentDtoV1 {
    Exit { code: i32 },
    Restart { code: i32 },
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ApplicationQuitRequestDtoV1 {
    pub intent: ApplicationQuitIntentDtoV1,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum ApplicationQuitOutcomeDtoV1 {
    Accepted,
}

#[cfg(test)]
#[path = "application_lifecycle_v1_test.rs"]
mod application_lifecycle_v1_tests;
