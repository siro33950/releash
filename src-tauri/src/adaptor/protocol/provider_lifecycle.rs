use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProviderLifecycleProvider {
    Claude,
    Codex,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "event", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum ProviderLifecycleSignalRequest {
    SessionStarted {
        provider_session_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        transcript_ref: Option<String>,
    },
    StopObserved {
        provider_session_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        transcript_ref: Option<String>,
    },
    StopFailed {
        provider_session_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        transcript_ref: Option<String>,
        reason: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProviderLifecycleReceiveRequest {
    pub(crate) slot_id: String,
    pub(crate) binding_id: String,
    pub(crate) capability: String,
    pub(crate) provider: ProviderLifecycleProvider,
    pub(crate) agent_session_id: String,
    pub(crate) signal: ProviderLifecycleSignalRequest,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProviderLifecycleUnavailableReasonRequest {
    SessionStartDeadlineExceeded,
    CodexHookDeliveryUnconfirmed,
    ProviderHookConfigurationRejected,
    LocalApiUnavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProviderLifecycleUnavailableRequest {
    pub(crate) slot_id: String,
    pub(crate) binding_id: String,
    pub(crate) capability: String,
    pub(crate) provider: ProviderLifecycleProvider,
    pub(crate) agent_session_id: String,
    pub(crate) reason: ProviderLifecycleUnavailableReasonRequest,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum ProviderLifecycleReceiveResponse {
    Applied,
    Duplicate,
    Rejected { reason: String },
}
