use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProviderAgentSessionOpenResponse {
    Attached,
    Resumed,
    Restored,
    Paused,
    Indeterminate,
    GarbageCollected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProviderAgentSessionArchiveResponse {
    Archived,
    AlreadyArchived,
    DeleteConfirmationRequired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ProviderHookHealthProviderResponse {
    Claude,
    Codex,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProviderHookHealthWarningResponse {
    pub(crate) provider: ProviderHookHealthProviderResponse,
    pub(crate) launch_id: String,
    pub(crate) reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProviderAvailabilitySnapshotResponse {
    pub(crate) providers: Vec<ProviderAvailabilityItemResponse>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProviderAvailabilityItemResponse {
    pub(crate) provider: String,
    pub(crate) display_name: String,
    pub(crate) default_executable: String,
    pub(crate) configured_executable: Option<String>,
    pub(crate) effective_executable: String,
    pub(crate) available: bool,
    pub(crate) resolved_executable: Option<String>,
    pub(crate) unavailable_reason: Option<String>,
}

impl From<crate::domain::agent_session::aggregates::ProviderRegistry>
    for ProviderAvailabilitySnapshotResponse
{
    fn from(registry: crate::domain::agent_session::aggregates::ProviderRegistry) -> Self {
        Self {
            providers: registry
                .entries()
                .iter()
                .map(|entry| ProviderAvailabilityItemResponse {
                    provider: match entry.provider() {
                        crate::domain::provider_lifecycle::ProviderKind::Claude => {
                            "claude".to_string()
                        }
                        crate::domain::provider_lifecycle::ProviderKind::Codex => {
                            "codex".to_string()
                        }
                    },
                    display_name: entry.display_name().to_string(),
                    default_executable: entry.default_executable().as_str().to_string(),
                    configured_executable: entry
                        .configured_executable()
                        .map(|executable| executable.as_str().to_string()),
                    effective_executable: entry.effective_executable().as_str().to_string(),
                    available: entry.is_available(),
                    resolved_executable: entry
                        .resolved_executable()
                        .map(|executable| executable.as_path().to_string_lossy().into_owned()),
                    unavailable_reason: entry.unavailable_reason().map(|reason| match reason {
                        crate::domain::agent_session::aggregates::ProviderUnavailableReason::NotFound => "not_found".to_string(),
                        crate::domain::agent_session::aggregates::ProviderUnavailableReason::NotExecutable => "not_executable".to_string(),
                        crate::domain::agent_session::aggregates::ProviderUnavailableReason::SearchPathUnavailable => "search_path_unavailable".to_string(),
                        crate::domain::agent_session::aggregates::ProviderUnavailableReason::ProbeFailed => "probe_failed".to_string(),
                    }),
                })
                .collect(),
        }
    }
}
