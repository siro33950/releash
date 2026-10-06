use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct StartExecutionRequest {
    pub(crate) workflow_name: String,
    pub(crate) worktree_path: String,
    pub(crate) request: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) created_from: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct ApproveNodeRequest {
    pub(crate) node: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) node_execution_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) comment: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct SubmitOutputArtifactRequest {
    pub(crate) contract: String,
    pub(crate) value: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct SubmitOutputRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) artifact: Option<SubmitOutputArtifactRequest>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct RetryNodeRequest {
    pub(crate) node_execution_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct ValidateArtifactRequest {
    pub(crate) node: String,
    pub(crate) contract: String,
    pub(crate) value: serde_json::Value,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderLifecycleProvider {
    Claude,
    Codex,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderActivityRequest {
    Working,
    AwaitingAnswer,
    AwaitingInstruction,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "event", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProviderLifecycleSignalRequest {
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
    ActivityObserved {
        provider_session_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        transcript_ref: Option<String>,
        activity: ProviderActivityRequest,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProviderLifecycleReceiveRequest {
    pub(crate) slot_id: String,
    pub(crate) binding_id: String,
    pub(crate) capability: String,
    pub provider: ProviderLifecycleProvider,
    pub(crate) agent_session_id: String,
    pub signal: ProviderLifecycleSignalRequest,
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
    pub provider: ProviderLifecycleProvider,
    pub(crate) agent_session_id: String,
    pub(crate) reason: ProviderLifecycleUnavailableReasonRequest,
}

#[cfg(test)]
#[path = "protocol_test.rs"]
mod protocol_tests;
