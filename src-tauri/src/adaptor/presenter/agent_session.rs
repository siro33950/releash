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
                        crate::usecase::provider_dto::AgentSessionProviderDto::Claude => "claude",
                        crate::usecase::provider_dto::AgentSessionProviderDto::Codex => "codex",
                    }
                    .to_string(),
                    display_name: entry.display_name,
                    default_executable: entry.default_executable,
                    configured_executable: entry.configured_executable,
                    effective_executable: entry.effective_executable,
                    available: entry.available,
                    resolved_executable: entry.resolved_executable,
                    unavailable_reason: entry.unavailable_reason.map(|reason| match reason {
                        crate::usecase::agent_session::ProviderUnavailableReasonDto::NotFound => "not_found",
                        crate::usecase::agent_session::ProviderUnavailableReasonDto::NotExecutable => "not_executable",
                        crate::usecase::agent_session::ProviderUnavailableReasonDto::SearchPathUnavailable => "search_path_unavailable",
                        crate::usecase::agent_session::ProviderUnavailableReasonDto::ProbeFailed => "probe_failed",
                    }.to_string()),
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
                crate::usecase::provider_dto::AgentSessionProviderDto::Claude => {
                    ProviderHookHealthProviderResponse::Claude
                }
                crate::usecase::provider_dto::AgentSessionProviderDto::Codex => {
                    ProviderHookHealthProviderResponse::Codex
                }
            },
            launch_id: value.launch_id,
            reason: match value.reason {
                crate::usecase::provider_lifecycle::ProviderHookHealthReasonDto::SessionStartDeadlineExceeded => "session_start_deadline_exceeded",
                crate::usecase::provider_lifecycle::ProviderHookHealthReasonDto::CodexHookDeliveryUnconfirmed => "codex_hook_delivery_unconfirmed",
                crate::usecase::provider_lifecycle::ProviderHookHealthReasonDto::ProviderHookConfigurationRejected => "provider_hook_configuration_rejected",
                crate::usecase::provider_lifecycle::ProviderHookHealthReasonDto::LocalApiUnavailable => "local_api_unavailable",
            }.to_string(),
        }
    }
}

#[cfg(test)]
#[path = "agent_session_test.rs"]
mod agent_session_tests;
