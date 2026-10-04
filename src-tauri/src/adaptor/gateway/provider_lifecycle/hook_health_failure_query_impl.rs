use std::path::PathBuf;

use serde::Deserialize;

use crate::domain::provider_lifecycle::{ProviderKind, ProviderLifecycleUnavailableReason};
use crate::infrastructure::provider_lifecycle::{
    read_provider_hook_local_api_failures, ProviderHookHealthMarkerError,
};
use crate::usecase::provider_lifecycle::{
    ProviderHookHealthFailureObservation, ProviderHookHealthFailureQuery,
    ProviderHookHealthFailureQueryError,
};

const MAX_SCANNED_MARKERS: usize = 1024;

pub(crate) struct LocalProviderHookHealthFailureQuery {
    data_dir: PathBuf,
}

impl LocalProviderHookHealthFailureQuery {
    pub(crate) fn new(data_dir: PathBuf) -> Self {
        Self { data_dir }
    }
}

#[async_trait::async_trait]
impl ProviderHookHealthFailureQuery for LocalProviderHookHealthFailureQuery {
    async fn list(
        &self,
        limit: usize,
    ) -> Result<
        Vec<Result<ProviderHookHealthFailureObservation, ProviderHookHealthFailureQueryError>>,
        ProviderHookHealthFailureQueryError,
    > {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let data_dir = self.data_dir.clone();
        let failures = crate::common::operation_context::spawn_blocking(move || {
            read_provider_hook_local_api_failures(&data_dir, MAX_SCANNED_MARKERS)
        })
        .await
        .map_err(|error| ProviderHookHealthFailureQueryError::Technical(error.into()))?
        .map_err(map_marker_error)?;
        Ok(failures
            .into_iter()
            .map(|failure| {
                failure.map_err(map_marker_error).and_then(|failure| {
                    parse_observation(&failure.contents)
                        .ok_or(ProviderHookHealthFailureQueryError::Corrupt)
                })
            })
            .take(limit)
            .collect())
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredHookHealthFailure {
    provider: String,
    launch_id: String,
    reason: String,
}

fn parse_observation(contents: &[u8]) -> Option<ProviderHookHealthFailureObservation> {
    let stored: StoredHookHealthFailure = serde_json::from_slice(contents).ok()?;
    let provider = match stored.provider.as_str() {
        "claude" => ProviderKind::Claude,
        "codex" => ProviderKind::Codex,
        _ => return None,
    };
    if stored.launch_id.trim().is_empty() || stored.reason != "local_api_unavailable" {
        return None;
    }
    Some(ProviderHookHealthFailureObservation {
        provider,
        launch_id: stored.launch_id,
        reason: ProviderLifecycleUnavailableReason::LocalApiUnavailable,
    })
}

fn map_marker_error(error: ProviderHookHealthMarkerError) -> ProviderHookHealthFailureQueryError {
    match error {
        ProviderHookHealthMarkerError::Encode(error) => {
            ProviderHookHealthFailureQueryError::Technical(
                crate::domain::failure::TechnicalFailure {
                    nature: crate::domain::failure::TechnicalFailureNature::Other,
                    message: error.to_string(),
                },
            )
        }
        ProviderHookHealthMarkerError::InvalidPath => ProviderHookHealthFailureQueryError::Corrupt,
        ProviderHookHealthMarkerError::Io(error) => {
            ProviderHookHealthFailureQueryError::Technical(error.into())
        }
    }
}
