use crate::domain::workflow::{ExecutionOrigin, ExecutionStatus, TokenUsage as WorkflowTokenUsage};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentSessionProviderRecord {
    Claude,
    Codex,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderSessionOwnershipProjectionRecord {
    pub provider: AgentSessionProviderRecord,
    pub provider_session_id: String,
    pub agent_session_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderHookHealthProjectionRecord {
    pub provider: AgentSessionProviderRecord,
    pub latest_launch_id: String,
    pub latest_launch_session_started: bool,
    pub warning_launch_id: Option<String>,
    pub warning_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SessionProjectionRecord {
    ProviderSessionOwnership(ProviderSessionOwnershipProjectionRecord),
    ProviderHookHealth(ProviderHookHealthProjectionRecord),
}

impl SessionProjectionRecord {
    pub(crate) fn semantic_bytes(&self) -> usize {
        fn optional(value: &Option<String>) -> usize {
            value.as_ref().map_or(0, String::len)
        }
        match self {
            Self::ProviderSessionOwnership(value) => {
                128 + value.provider_session_id.len() + optional(&value.agent_session_id)
            }
            Self::ProviderHookHealth(value) => {
                96 + value.latest_launch_id.len()
                    + optional(&value.warning_launch_id)
                    + optional(&value.warning_reason)
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowExecutionMetadataRecord {
    pub execution_id: String,
    pub workflow_name: String,
    pub status: ExecutionStatus,
    pub worktree_path: String,
    pub current_node: Option<String>,
    pub created_from: ExecutionOrigin,
    pub started_at_bits: u64,
    pub updated_at_bits: u64,
    pub completed_at_bits: Option<u64>,
    pub error_reason: Option<String>,
    pub total_token_usage: WorkflowTokenUsage,
}
