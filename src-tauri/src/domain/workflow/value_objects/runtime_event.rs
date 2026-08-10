use serde::{Deserialize, Serialize};

use super::{
    ExecutionInterruptionReason, ExecutionOrigin, FanoutParentRef, NodeExecutionFailureKind,
    NodeKindName, TokenUsage, WorkflowDefinition,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ContractViolationRecord {
    pub path: String,
    pub reason: String,
}

/// Canonical facts emitted by a workflow execution.
///
/// The event belongs to the workflow domain, and its serde representation is
/// also the persisted wire shape: gateway codecs serialize this type directly,
/// so serde attribute changes here change stored logs. Compatibility is pinned
/// by the stored-event contract tests in the gateway.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case", tag = "event")]
pub enum WorkflowEvent {
    ExecutionStarted {
        execution_id: String,
        workflow_name: String,
        worktree_path: String,
        #[serde(with = "execution_origin_serde")]
        created_from: ExecutionOrigin,
        request: String,
        permission_mode: String,
        definition: WorkflowDefinition,
        timestamp: f64,
    },
    NodeStarted {
        execution_id: String,
        node_execution_id: String,
        node_name: String,
        kind: NodeKindName,
        attempt: u32,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        fanout_parent: Option<FanoutParentRef>,
        timestamp: f64,
    },
    SessionAttached {
        execution_id: String,
        node_execution_id: String,
        session_id: String,
        timestamp: f64,
    },
    NodeSubmitReceived {
        execution_id: String,
        node_execution_id: String,
        timestamp: f64,
    },
    NodeStopReceived {
        execution_id: String,
        node_execution_id: String,
        timestamp: f64,
    },
    NodeRetryRequested {
        execution_id: String,
        node_execution_id: String,
        timestamp: f64,
    },
    NodePaused {
        execution_id: String,
        node_execution_id: String,
        timestamp: f64,
    },
    NodeResumed {
        execution_id: String,
        node_execution_id: String,
        timestamp: f64,
    },
    CommandPrepared {
        execution_id: String,
        node_execution_id: String,
        display_command: String,
        timestamp: f64,
    },
    ArtifactProduced {
        execution_id: String,
        node_execution_id: String,
        node_name: String,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        contract: Option<String>,
        value: serde_json::Value,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        request_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        submitted_at: Option<f64>,
        timestamp: f64,
    },
    NodeCompleted {
        execution_id: String,
        node_execution_id: String,
        node_name: String,
        attempt: u32,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        result_summary: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        token_usage: Option<TokenUsage>,
        timestamp: f64,
    },
    NodeFailed {
        execution_id: String,
        node_execution_id: String,
        node_name: String,
        attempt: u32,
        reason: String,
        failure_kind: NodeExecutionFailureKind,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        retry_count: Option<u32>,
        timestamp: f64,
    },
    ApprovalRequested {
        execution_id: String,
        node_execution_id: String,
        node_name: String,
        timestamp: f64,
    },
    ApprovalResolved {
        execution_id: String,
        node_execution_id: String,
        node_name: String,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        comment: Option<String>,
        timestamp: f64,
    },
    ContractViolated {
        execution_id: String,
        node_execution_id: String,
        node_name: String,
        violations: Vec<ContractViolationRecord>,
        repair_attempt: u32,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        request_id: Option<String>,
        timestamp: f64,
    },
    StallObserved {
        execution_id: String,
        node_execution_id: String,
        node_name: String,
        attempt: u32,
        session_id: String,
        turn_phase: String,
        idle_secs: u64,
        signal_count: u32,
        cap_reached: bool,
        timestamp: f64,
    },
    StallCleared {
        execution_id: String,
        node_execution_id: String,
        session_id: String,
        timestamp: f64,
    },
    ExecutionCompleted {
        execution_id: String,
        total_token_usage: TokenUsage,
        timestamp: f64,
    },
    ExecutionAborted {
        execution_id: String,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        aborted_node: Option<String>,
        timestamp: f64,
    },
    ExecutionInterrupted {
        execution_id: String,
        #[serde(with = "execution_interruption_reason_serde")]
        reason: ExecutionInterruptionReason,
        timestamp: f64,
    },
    ExecutionResumed {
        execution_id: String,
        resume_from_node: String,
        timestamp: f64,
    },
}

mod execution_origin_serde {
    use serde::{Deserialize, Deserializer, Serializer};

    use super::ExecutionOrigin;

    pub(super) fn serialize<S>(value: &ExecutionOrigin, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(value.as_public_value())
    }

    pub(super) fn deserialize<'de, D>(deserializer: D) -> Result<ExecutionOrigin, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        ExecutionOrigin::from_public_value(&value).map_err(serde::de::Error::custom)
    }
}

mod execution_interruption_reason_serde {
    use serde::{Deserialize, Deserializer, Serializer};

    use super::ExecutionInterruptionReason;

    pub(super) fn serialize<S>(
        value: &ExecutionInterruptionReason,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(value.as_str())
    }

    pub(super) fn deserialize<'de, D>(
        deserializer: D,
    ) -> Result<ExecutionInterruptionReason, D::Error>
    where
        D: Deserializer<'de>,
    {
        match String::deserialize(deserializer)?.as_str() {
            "crash" => Ok(ExecutionInterruptionReason::Crash),
            "stale" => Ok(ExecutionInterruptionReason::Stale),
            "stop" => Ok(ExecutionInterruptionReason::Stop),
            "orphan" => Ok(ExecutionInterruptionReason::Orphan),
            value => Err(serde::de::Error::unknown_variant(
                value,
                &["crash", "stale", "stop", "orphan"],
            )),
        }
    }
}
