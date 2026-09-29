use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AgentSessionOpenResponse {
    Attached,
    Resumed,
    Restored,
    Paused,
    Indeterminate,
    GarbageCollected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AgentSessionArchiveResponse {
    Archived,
    AlreadyArchived,
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

impl From<crate::usecase::agent_session::ProviderAvailabilitySnapshotDto>
    for ProviderAvailabilitySnapshotResponse
{
    fn from(value: crate::usecase::agent_session::ProviderAvailabilitySnapshotDto) -> Self {
        Self {
            providers: value
                .providers
                .into_iter()
                .map(|entry| ProviderAvailabilityItemResponse {
                    provider: match entry.provider {
                        crate::usecase::agent_session::AgentSessionProviderDto::Claude => "claude",
                        crate::usecase::agent_session::AgentSessionProviderDto::Codex => "codex",
                    }
                    .to_string(),
                    display_name: entry.display_name,
                    default_executable: entry.default_executable,
                    configured_executable: entry.configured_executable,
                    effective_executable: entry.effective_executable,
                    available: entry.available,
                    resolved_executable: entry.resolved_executable,
                    unavailable_reason: entry.unavailable_reason,
                })
                .collect(),
        }
    }
}

impl From<crate::usecase::provider_lifecycle::ProviderHookHealthWarningDto>
    for ProviderHookHealthWarningResponse
{
    fn from(value: crate::usecase::provider_lifecycle::ProviderHookHealthWarningDto) -> Self {
        Self {
            provider: match value.provider {
                crate::usecase::agent_session::AgentSessionProviderDto::Claude => {
                    ProviderHookHealthProviderResponse::Claude
                }
                crate::usecase::agent_session::AgentSessionProviderDto::Codex => {
                    ProviderHookHealthProviderResponse::Codex
                }
            },
            launch_id: value.launch_id,
            reason: value.reason,
        }
    }
}
