//! Append-only workflow execution event schema.
//!
//! Existing NDJSON inventory is intentionally not compatible with this schema.

use serde::{Deserialize, Serialize};

use crate::adaptor::gateway::workflow::schema::{NodeKindName, Workflow};
use crate::domain::workflow::{ExecutionOrigin, NodeExecutionFailureKind};

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

impl TokenUsage {
    pub fn add(&mut self, other: &TokenUsage) {
        self.input_tokens += other.input_tokens;
        self.output_tokens += other.output_tokens;
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FanoutParentRef {
    pub parent_node: String,
    pub parent_attempt: u32,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub item_index: Option<usize>,
    pub child_index: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContractViolationRecord {
    pub path: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "event", deny_unknown_fields)]
pub enum WorkflowEvent {
    ExecutionStarted {
        execution_id: String,
        workflow_name: String,
        worktree_path: String,
        #[serde(with = "execution_origin_serde")]
        created_from: ExecutionOrigin,
        request: String,
        definition: Workflow,
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
    ExecutionFailed {
        execution_id: String,
        reason: String,
        failure_kind: NodeExecutionFailureKind,
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
        reason: String,
        timestamp: f64,
    },
    CliMutationRequested {
        execution_id: String,
        request_id: String,
        request: CliMutationRequestRecord,
        requested_at: f64,
        timestamp: f64,
    },
    CliMutationRejected {
        execution_id: String,
        request_id: String,
        request: CliMutationRequestRecord,
        reason: CliMutationRejectionReason,
        message: String,
        requested_at: f64,
        timestamp: f64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "kind", deny_unknown_fields)]
pub enum CliMutationRequestRecord {
    Approve {
        node_name: String,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        comment: Option<String>,
    },
    Abort {
        #[serde(skip_serializing_if = "Option::is_none", default)]
        node_name: Option<String>,
    },
    SubmitOutput {
        node_name: String,
        contract: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CliMutationRejectionReason {
    ExecutionNotFound,
    ExecutionNotActive,
    NodeNotFound,
    NotWaitingApproval,
    NodeNotAccepting,
    ContractMismatch,
    Other,
}

impl WorkflowEvent {
    pub fn execution_id(&self) -> &str {
        match self {
            Self::ExecutionStarted { execution_id, .. }
            | Self::NodeStarted { execution_id, .. }
            | Self::SessionAttached { execution_id, .. }
            | Self::ArtifactProduced { execution_id, .. }
            | Self::NodeCompleted { execution_id, .. }
            | Self::NodeFailed { execution_id, .. }
            | Self::ApprovalRequested { execution_id, .. }
            | Self::ApprovalResolved { execution_id, .. }
            | Self::ContractViolated { execution_id, .. }
            | Self::StallObserved { execution_id, .. }
            | Self::StallCleared { execution_id, .. }
            | Self::ExecutionCompleted { execution_id, .. }
            | Self::ExecutionFailed { execution_id, .. }
            | Self::ExecutionAborted { execution_id, .. }
            | Self::ExecutionInterrupted { execution_id, .. }
            | Self::CliMutationRequested { execution_id, .. }
            | Self::CliMutationRejected { execution_id, .. } => execution_id,
        }
    }

    pub fn timestamp(&self) -> f64 {
        match self {
            Self::ExecutionStarted { timestamp, .. }
            | Self::NodeStarted { timestamp, .. }
            | Self::SessionAttached { timestamp, .. }
            | Self::ArtifactProduced { timestamp, .. }
            | Self::NodeCompleted { timestamp, .. }
            | Self::NodeFailed { timestamp, .. }
            | Self::ApprovalRequested { timestamp, .. }
            | Self::ApprovalResolved { timestamp, .. }
            | Self::ContractViolated { timestamp, .. }
            | Self::StallObserved { timestamp, .. }
            | Self::StallCleared { timestamp, .. }
            | Self::ExecutionCompleted { timestamp, .. }
            | Self::ExecutionFailed { timestamp, .. }
            | Self::ExecutionAborted { timestamp, .. }
            | Self::ExecutionInterrupted { timestamp, .. }
            | Self::CliMutationRequested { timestamp, .. }
            | Self::CliMutationRejected { timestamp, .. } => *timestamp,
        }
    }
}

mod execution_origin_serde {
    use serde::{Deserialize, Deserializer, Serializer};

    use crate::domain::workflow::ExecutionOrigin;

    pub fn serialize<S>(origin: &ExecutionOrigin, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(match origin {
            ExecutionOrigin::DesktopUi => "desktop_ui",
            ExecutionOrigin::Cli => "cli",
            ExecutionOrigin::Agent => "agent",
            ExecutionOrigin::Api => "api",
        })
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<ExecutionOrigin, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        match value.as_str() {
            "desktop_ui" => Ok(ExecutionOrigin::DesktopUi),
            "cli" => Ok(ExecutionOrigin::Cli),
            "agent" => Ok(ExecutionOrigin::Agent),
            "api" => Ok(ExecutionOrigin::Api),
            _ => Err(serde::de::Error::unknown_variant(
                &value,
                &["desktop_ui", "cli", "agent", "api"],
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adaptor::gateway::workflow::schema::{
        FacetRefs, NodeDefinition, NodeKind, SessionSpec,
    };

    fn minimal_workflow() -> Workflow {
        Workflow {
            name: "wf".to_string(),
            description: String::new(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![NodeDefinition {
                name: "review".to_string(),
                kind: NodeKind::Session(SessionSpec {
                    facets: FacetRefs {
                        instruction: Some("review".to_string()),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
                ..NodeDefinition::default()
            }],
        }
    }

    #[test]
    fn execution_started_round_trips_canonical_schema() {
        let event = WorkflowEvent::ExecutionStarted {
            execution_id: "00000000-0000-4000-8000-000000000001".to_string(),
            workflow_name: "wf".to_string(),
            worktree_path: "/repo".to_string(),
            created_from: ExecutionOrigin::Cli,
            request: "review".to_string(),
            definition: minimal_workflow(),
            timestamp: 1.0,
        };

        let value = serde_json::to_value(&event).unwrap();
        assert_eq!(value["event"], "execution_started");
        assert_eq!(value["execution_id"], event.execution_id());
        assert_eq!(value["created_from"], "cli");
        assert!(serde_json::from_value::<WorkflowEvent>(value).is_ok());
    }

    #[test]
    fn legacy_run_event_is_rejected() {
        let value = serde_json::json!({
            "event": "run_started",
            "run_id": "00000000-0000-4000-8000-000000000001",
            "timestamp": 1.0
        });
        assert!(serde_json::from_value::<WorkflowEvent>(value).is_err());
    }

    #[test]
    fn cli_mutation_uses_execution_and_node_vocabulary() {
        let event = WorkflowEvent::CliMutationRejected {
            execution_id: "execution-1".to_string(),
            request_id: "request-1".to_string(),
            request: CliMutationRequestRecord::SubmitOutput {
                node_name: "review".to_string(),
                contract: "review_result".to_string(),
            },
            reason: CliMutationRejectionReason::NodeNotAccepting,
            message: "not accepting".to_string(),
            requested_at: 1.0,
            timestamp: 2.0,
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(!json.contains("run_id"));
        assert!(!json.contains("step_name"));
        assert!(json.contains("execution_id"));
        assert!(json.contains("node_name"));
    }

    #[test]
    fn canonical_variants_reject_legacy_identity_fields() {
        let execution = serde_json::json!({
            "event": "execution_aborted",
            "execution_id": "00000000-0000-4000-8000-000000000001",
            "run_id": "00000000-0000-4000-8000-000000000001",
            "timestamp": 1.0
        });
        assert!(serde_json::from_value::<WorkflowEvent>(execution).is_err());

        let node = serde_json::json!({
            "event": "node_started",
            "execution_id": "00000000-0000-4000-8000-000000000001",
            "node_execution_id": "node-1",
            "node_name": "review",
            "step_name": "review",
            "kind": "session",
            "attempt": 1,
            "timestamp": 1.0
        });
        assert!(serde_json::from_value::<WorkflowEvent>(node).is_err());

        let mutation = serde_json::json!({
            "event": "cli_mutation_requested",
            "execution_id": "00000000-0000-4000-8000-000000000001",
            "request_id": "00000000-0000-4000-8000-000000000002",
            "request": {
                "kind": "submit_output",
                "node_name": "review",
                "step_name": "review",
                "contract": "review_result"
            },
            "requested_at": 1.0,
            "timestamp": 1.0
        });
        assert!(serde_json::from_value::<WorkflowEvent>(mutation).is_err());
    }

    #[test]
    fn canonical_variants_reject_unknown_nested_fields() {
        let usage = serde_json::json!({
            "event": "execution_completed",
            "execution_id": "00000000-0000-4000-8000-000000000001",
            "total_token_usage": {
                "inputTokens": 1,
                "outputTokens": 2,
                "legacyTotal": 3
            },
            "timestamp": 1.0
        });
        assert!(serde_json::from_value::<WorkflowEvent>(usage).is_err());
    }
}
