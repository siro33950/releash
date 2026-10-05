//! Strict public V1 DTOs for application quit and shutdown.
//!
//! These transport shapes deliberately contain no repository or usecase
//! behavior. Tauri and loopback WebSocket adapters share them through the
//! application-lifecycle presenter.

use serde::{Deserialize, Serialize};

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
