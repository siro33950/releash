use crate::domain::provider_lifecycle::{ProviderKind, ProviderLifecycleUnavailableReason};
use crate::usecase::agent_session::ProviderUnavailableReasonDto;
use crate::usecase::provider_dto::AgentSessionProviderDto;
use serde::Serialize;

fn provider_name(provider: AgentSessionProviderDto) -> &'static str {
    match provider {
        AgentSessionProviderDto::Claude => "claude",
        AgentSessionProviderDto::Codex => "codex",
    }
}

fn unavailable_reason(reason: ProviderUnavailableReasonDto) -> &'static str {
    match reason {
        ProviderUnavailableReasonDto::NotFound => "not_found",
        ProviderUnavailableReasonDto::NotExecutable => "not_executable",
        ProviderUnavailableReasonDto::SearchPathUnavailable => "search_path_unavailable",
        ProviderUnavailableReasonDto::ProbeFailed => "probe_failed",
    }
}

fn hook_health_reason(reason: ProviderLifecycleUnavailableReason) -> &'static str {
    match reason {
        ProviderLifecycleUnavailableReason::SessionStartDeadlineExceeded => {
            "session_start_deadline_exceeded"
        }
        ProviderLifecycleUnavailableReason::CodexHookDeliveryUnconfirmed => {
            "codex_hook_delivery_unconfirmed"
        }
        ProviderLifecycleUnavailableReason::ProviderHookConfigurationRejected => {
            "provider_hook_configuration_rejected"
        }
        ProviderLifecycleUnavailableReason::LocalApiUnavailable => "local_api_unavailable",
    }
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
    pub(crate) configuration_revision: u32,
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
                    provider: provider_name(entry.provider).to_string(),
                    display_name: entry.display_name,
                    default_executable: entry.default_executable,
                    configured_executable: entry.configured_executable,
                    configuration_revision: entry.configuration_revision,
                    effective_executable: entry.effective_executable,
                    available: entry.available,
                    resolved_executable: entry.resolved_executable,
                    unavailable_reason: entry
                        .unavailable_reason
                        .map(|reason| unavailable_reason(reason).to_string()),
                })
                .collect(),
        }
    }
}

impl From<crate::usecase::provider_lifecycle::ProviderHookHealthWarning>
    for ProviderHookHealthWarningResponse
{
    fn from(value: crate::usecase::provider_lifecycle::ProviderHookHealthWarning) -> Self {
        Self {
            provider: match value.provider {
                ProviderKind::Claude => ProviderHookHealthProviderResponse::Claude,
                ProviderKind::Codex => ProviderHookHealthProviderResponse::Codex,
            },
            launch_id: value.launch_id,
            reason: hook_health_reason(value.reason).to_string(),
        }
    }
}

#[cfg(test)]
#[path = "agent_session_test.rs"]
pub(crate) mod agent_session_tests;
