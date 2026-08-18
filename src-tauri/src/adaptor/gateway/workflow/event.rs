//! Append-only workflow execution event schema.
//!
//! Existing NDJSON inventory is intentionally not compatible with this schema.

use serde::{Deserialize, Serialize};

pub use crate::domain::workflow::{
    ContractViolationRecord, ExecutionParentRef, TokenUsage, WorkflowEvent,
};
use crate::domain::workflow::{
    ExecutionInterruptionReason, ExecutionOrigin, NodeExecutionFailureKind,
};
use crate::domain::workflow::{NodeKindName, WorkflowDefinition as WorkflowDefinitionYaml};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum StoredExecutionOriginV1 {
    #[serde(alias = "desktop-ui")]
    DesktopUi,
    Cli,
    Agent,
    Api,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum StoredNodeExecutionFailureKindV1 {
    StartupTimeout,
    StaleRuntimeTimeout,
    ModelRefusal,
    StructuredOutputMismatch,
    ValidationFailure,
    UserAbort,
    InfrastructureCrash,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum StoredExecutionInterruptionReasonV1 {
    Crash,
    Stale,
    Stop,
    Orphan,
}

/// Gateway-owned V1 NDJSON record. Scalar fields cross explicit total
/// converters, while `definition` / `parent` / `token_usage` /
/// `violations` reuse the domain types whose serde shapes double as the wire
/// shapes; those shapes are pinned by the contract tests below.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "event")]
enum StoredWorkflowEventV1 {
    ExecutionStarted {
        execution_id: String,
        workflow_name: String,
        worktree_path: String,
        created_from: StoredExecutionOriginV1,
        request: String,
        definition: WorkflowDefinitionYaml,
        timestamp: f64,
    },
    NodeStarted {
        execution_id: String,
        node_execution_id: String,
        node_name: String,
        kind: NodeKindName,
        attempt: u32,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        parent: Option<ExecutionParentRef>,
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
        failure_kind: StoredNodeExecutionFailureKindV1,
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
        reason: StoredExecutionInterruptionReasonV1,
        timestamp: f64,
    },
    ExecutionResumed {
        execution_id: String,
        resume_from_node: String,
        timestamp: f64,
    },
}

impl From<ExecutionOrigin> for StoredExecutionOriginV1 {
    fn from(value: ExecutionOrigin) -> Self {
        match value {
            ExecutionOrigin::DesktopUi => Self::DesktopUi,
            ExecutionOrigin::Cli => Self::Cli,
            ExecutionOrigin::Agent => Self::Agent,
            ExecutionOrigin::Api => Self::Api,
        }
    }
}

impl From<StoredExecutionOriginV1> for ExecutionOrigin {
    fn from(value: StoredExecutionOriginV1) -> Self {
        match value {
            StoredExecutionOriginV1::DesktopUi => Self::DesktopUi,
            StoredExecutionOriginV1::Cli => Self::Cli,
            StoredExecutionOriginV1::Agent => Self::Agent,
            StoredExecutionOriginV1::Api => Self::Api,
        }
    }
}

impl From<NodeExecutionFailureKind> for StoredNodeExecutionFailureKindV1 {
    fn from(value: NodeExecutionFailureKind) -> Self {
        match value {
            NodeExecutionFailureKind::StartupTimeout => Self::StartupTimeout,
            NodeExecutionFailureKind::StaleRuntimeTimeout => Self::StaleRuntimeTimeout,
            NodeExecutionFailureKind::ModelRefusal => Self::ModelRefusal,
            NodeExecutionFailureKind::StructuredOutputMismatch => Self::StructuredOutputMismatch,
            NodeExecutionFailureKind::ValidationFailure => Self::ValidationFailure,
            NodeExecutionFailureKind::UserAbort => Self::UserAbort,
            NodeExecutionFailureKind::InfrastructureCrash => Self::InfrastructureCrash,
        }
    }
}

impl From<StoredNodeExecutionFailureKindV1> for NodeExecutionFailureKind {
    fn from(value: StoredNodeExecutionFailureKindV1) -> Self {
        match value {
            StoredNodeExecutionFailureKindV1::StartupTimeout => Self::StartupTimeout,
            StoredNodeExecutionFailureKindV1::StaleRuntimeTimeout => Self::StaleRuntimeTimeout,
            StoredNodeExecutionFailureKindV1::ModelRefusal => Self::ModelRefusal,
            StoredNodeExecutionFailureKindV1::StructuredOutputMismatch => {
                Self::StructuredOutputMismatch
            }
            StoredNodeExecutionFailureKindV1::ValidationFailure => Self::ValidationFailure,
            StoredNodeExecutionFailureKindV1::UserAbort => Self::UserAbort,
            StoredNodeExecutionFailureKindV1::InfrastructureCrash => Self::InfrastructureCrash,
        }
    }
}

impl From<ExecutionInterruptionReason> for StoredExecutionInterruptionReasonV1 {
    fn from(value: ExecutionInterruptionReason) -> Self {
        match value {
            ExecutionInterruptionReason::Crash => Self::Crash,
            ExecutionInterruptionReason::Stale => Self::Stale,
            ExecutionInterruptionReason::Stop => Self::Stop,
            ExecutionInterruptionReason::Orphan => Self::Orphan,
        }
    }
}

impl From<StoredExecutionInterruptionReasonV1> for ExecutionInterruptionReason {
    fn from(value: StoredExecutionInterruptionReasonV1) -> Self {
        match value {
            StoredExecutionInterruptionReasonV1::Crash => Self::Crash,
            StoredExecutionInterruptionReasonV1::Stale => Self::Stale,
            StoredExecutionInterruptionReasonV1::Stop => Self::Stop,
            StoredExecutionInterruptionReasonV1::Orphan => Self::Orphan,
        }
    }
}

impl From<&WorkflowEvent> for StoredWorkflowEventV1 {
    fn from(event: &WorkflowEvent) -> Self {
        match event {
            WorkflowEvent::ExecutionStarted {
                execution_id,
                workflow_name,
                worktree_path,
                created_from,
                request,
                definition,
                timestamp,
            } => Self::ExecutionStarted {
                execution_id: execution_id.clone(),
                workflow_name: workflow_name.clone(),
                worktree_path: worktree_path.clone(),
                created_from: (*created_from).into(),
                request: request.clone(),
                definition: definition.clone(),
                timestamp: *timestamp,
            },
            WorkflowEvent::NodeStarted {
                execution_id,
                node_execution_id,
                node_name,
                kind,
                attempt,
                parent,
                timestamp,
            } => Self::NodeStarted {
                execution_id: execution_id.clone(),
                node_execution_id: node_execution_id.clone(),
                node_name: node_name.clone(),
                kind: *kind,
                attempt: *attempt,
                parent: parent.clone(),
                timestamp: *timestamp,
            },
            WorkflowEvent::SessionAttached {
                execution_id,
                node_execution_id,
                session_id,
                timestamp,
            } => Self::SessionAttached {
                execution_id: execution_id.clone(),
                node_execution_id: node_execution_id.clone(),
                session_id: session_id.clone(),
                timestamp: *timestamp,
            },
            WorkflowEvent::NodeSubmitReceived {
                execution_id,
                node_execution_id,
                timestamp,
            } => Self::NodeSubmitReceived {
                execution_id: execution_id.clone(),
                node_execution_id: node_execution_id.clone(),
                timestamp: *timestamp,
            },
            WorkflowEvent::NodeStopReceived {
                execution_id,
                node_execution_id,
                timestamp,
            } => Self::NodeStopReceived {
                execution_id: execution_id.clone(),
                node_execution_id: node_execution_id.clone(),
                timestamp: *timestamp,
            },
            WorkflowEvent::NodeRetryRequested {
                execution_id,
                node_execution_id,
                timestamp,
            } => Self::NodeRetryRequested {
                execution_id: execution_id.clone(),
                node_execution_id: node_execution_id.clone(),
                timestamp: *timestamp,
            },
            WorkflowEvent::NodePaused {
                execution_id,
                node_execution_id,
                timestamp,
            } => Self::NodePaused {
                execution_id: execution_id.clone(),
                node_execution_id: node_execution_id.clone(),
                timestamp: *timestamp,
            },
            WorkflowEvent::NodeResumed {
                execution_id,
                node_execution_id,
                timestamp,
            } => Self::NodeResumed {
                execution_id: execution_id.clone(),
                node_execution_id: node_execution_id.clone(),
                timestamp: *timestamp,
            },
            WorkflowEvent::CommandPrepared {
                execution_id,
                node_execution_id,
                display_command,
                timestamp,
            } => Self::CommandPrepared {
                execution_id: execution_id.clone(),
                node_execution_id: node_execution_id.clone(),
                display_command: display_command.clone(),
                timestamp: *timestamp,
            },
            WorkflowEvent::ArtifactProduced {
                execution_id,
                node_execution_id,
                node_name,
                contract,
                value,
                request_id,
                submitted_at,
                timestamp,
            } => Self::ArtifactProduced {
                execution_id: execution_id.clone(),
                node_execution_id: node_execution_id.clone(),
                node_name: node_name.clone(),
                contract: contract.clone(),
                value: value.clone(),
                request_id: request_id.clone(),
                submitted_at: *submitted_at,
                timestamp: *timestamp,
            },
            WorkflowEvent::NodeCompleted {
                execution_id,
                node_execution_id,
                node_name,
                attempt,
                result_summary,
                token_usage,
                timestamp,
            } => Self::NodeCompleted {
                execution_id: execution_id.clone(),
                node_execution_id: node_execution_id.clone(),
                node_name: node_name.clone(),
                attempt: *attempt,
                result_summary: result_summary.clone(),
                token_usage: token_usage.clone(),
                timestamp: *timestamp,
            },
            WorkflowEvent::NodeFailed {
                execution_id,
                node_execution_id,
                node_name,
                attempt,
                reason,
                failure_kind,
                retry_count,
                timestamp,
            } => Self::NodeFailed {
                execution_id: execution_id.clone(),
                node_execution_id: node_execution_id.clone(),
                node_name: node_name.clone(),
                attempt: *attempt,
                reason: reason.clone(),
                failure_kind: (*failure_kind).into(),
                retry_count: *retry_count,
                timestamp: *timestamp,
            },
            WorkflowEvent::ApprovalRequested {
                execution_id,
                node_execution_id,
                node_name,
                timestamp,
            } => Self::ApprovalRequested {
                execution_id: execution_id.clone(),
                node_execution_id: node_execution_id.clone(),
                node_name: node_name.clone(),
                timestamp: *timestamp,
            },
            WorkflowEvent::ApprovalResolved {
                execution_id,
                node_execution_id,
                node_name,
                comment,
                timestamp,
            } => Self::ApprovalResolved {
                execution_id: execution_id.clone(),
                node_execution_id: node_execution_id.clone(),
                node_name: node_name.clone(),
                comment: comment.clone(),
                timestamp: *timestamp,
            },
            WorkflowEvent::ContractViolated {
                execution_id,
                node_execution_id,
                node_name,
                violations,
                repair_attempt,
                request_id,
                timestamp,
            } => Self::ContractViolated {
                execution_id: execution_id.clone(),
                node_execution_id: node_execution_id.clone(),
                node_name: node_name.clone(),
                violations: violations.clone(),
                repair_attempt: *repair_attempt,
                request_id: request_id.clone(),
                timestamp: *timestamp,
            },
            WorkflowEvent::StallObserved {
                execution_id,
                node_execution_id,
                node_name,
                attempt,
                session_id,
                turn_phase,
                idle_secs,
                signal_count,
                cap_reached,
                timestamp,
            } => Self::StallObserved {
                execution_id: execution_id.clone(),
                node_execution_id: node_execution_id.clone(),
                node_name: node_name.clone(),
                attempt: *attempt,
                session_id: session_id.clone(),
                turn_phase: turn_phase.clone(),
                idle_secs: *idle_secs,
                signal_count: *signal_count,
                cap_reached: *cap_reached,
                timestamp: *timestamp,
            },
            WorkflowEvent::StallCleared {
                execution_id,
                node_execution_id,
                session_id,
                timestamp,
            } => Self::StallCleared {
                execution_id: execution_id.clone(),
                node_execution_id: node_execution_id.clone(),
                session_id: session_id.clone(),
                timestamp: *timestamp,
            },
            WorkflowEvent::ExecutionCompleted {
                execution_id,
                total_token_usage,
                timestamp,
            } => Self::ExecutionCompleted {
                execution_id: execution_id.clone(),
                total_token_usage: total_token_usage.clone(),
                timestamp: *timestamp,
            },
            WorkflowEvent::ExecutionAborted {
                execution_id,
                aborted_node,
                timestamp,
            } => Self::ExecutionAborted {
                execution_id: execution_id.clone(),
                aborted_node: aborted_node.clone(),
                timestamp: *timestamp,
            },
            WorkflowEvent::ExecutionInterrupted {
                execution_id,
                reason,
                timestamp,
            } => Self::ExecutionInterrupted {
                execution_id: execution_id.clone(),
                reason: (*reason).into(),
                timestamp: *timestamp,
            },
            WorkflowEvent::ExecutionResumed {
                execution_id,
                resume_from_node,
                timestamp,
            } => Self::ExecutionResumed {
                execution_id: execution_id.clone(),
                resume_from_node: resume_from_node.clone(),
                timestamp: *timestamp,
            },
        }
    }
}

impl From<StoredWorkflowEventV1> for WorkflowEvent {
    fn from(event: StoredWorkflowEventV1) -> Self {
        match event {
            StoredWorkflowEventV1::ExecutionStarted {
                execution_id,
                workflow_name,
                worktree_path,
                created_from,
                request,
                definition,
                timestamp,
            } => Self::ExecutionStarted {
                execution_id,
                workflow_name,
                worktree_path,
                created_from: created_from.into(),
                request,
                definition,
                timestamp,
            },
            StoredWorkflowEventV1::NodeStarted {
                execution_id,
                node_execution_id,
                node_name,
                kind,
                attempt,
                parent,
                timestamp,
            } => Self::NodeStarted {
                execution_id,
                node_execution_id,
                node_name,
                kind,
                attempt,
                parent,
                timestamp,
            },
            StoredWorkflowEventV1::SessionAttached {
                execution_id,
                node_execution_id,
                session_id,
                timestamp,
            } => Self::SessionAttached {
                execution_id,
                node_execution_id,
                session_id,
                timestamp,
            },
            StoredWorkflowEventV1::NodeSubmitReceived {
                execution_id,
                node_execution_id,
                timestamp,
            } => Self::NodeSubmitReceived {
                execution_id,
                node_execution_id,
                timestamp,
            },
            StoredWorkflowEventV1::NodeStopReceived {
                execution_id,
                node_execution_id,
                timestamp,
            } => Self::NodeStopReceived {
                execution_id,
                node_execution_id,
                timestamp,
            },
            StoredWorkflowEventV1::NodeRetryRequested {
                execution_id,
                node_execution_id,
                timestamp,
            } => Self::NodeRetryRequested {
                execution_id,
                node_execution_id,
                timestamp,
            },
            StoredWorkflowEventV1::NodePaused {
                execution_id,
                node_execution_id,
                timestamp,
            } => Self::NodePaused {
                execution_id,
                node_execution_id,
                timestamp,
            },
            StoredWorkflowEventV1::NodeResumed {
                execution_id,
                node_execution_id,
                timestamp,
            } => Self::NodeResumed {
                execution_id,
                node_execution_id,
                timestamp,
            },
            StoredWorkflowEventV1::CommandPrepared {
                execution_id,
                node_execution_id,
                display_command,
                timestamp,
            } => Self::CommandPrepared {
                execution_id,
                node_execution_id,
                display_command,
                timestamp,
            },
            StoredWorkflowEventV1::ArtifactProduced {
                execution_id,
                node_execution_id,
                node_name,
                contract,
                value,
                request_id,
                submitted_at,
                timestamp,
            } => Self::ArtifactProduced {
                execution_id,
                node_execution_id,
                node_name,
                contract,
                value,
                request_id,
                submitted_at,
                timestamp,
            },
            StoredWorkflowEventV1::NodeCompleted {
                execution_id,
                node_execution_id,
                node_name,
                attempt,
                result_summary,
                token_usage,
                timestamp,
            } => Self::NodeCompleted {
                execution_id,
                node_execution_id,
                node_name,
                attempt,
                result_summary,
                token_usage,
                timestamp,
            },
            StoredWorkflowEventV1::NodeFailed {
                execution_id,
                node_execution_id,
                node_name,
                attempt,
                reason,
                failure_kind,
                retry_count,
                timestamp,
            } => Self::NodeFailed {
                execution_id,
                node_execution_id,
                node_name,
                attempt,
                reason,
                failure_kind: failure_kind.into(),
                retry_count,
                timestamp,
            },
            StoredWorkflowEventV1::ApprovalRequested {
                execution_id,
                node_execution_id,
                node_name,
                timestamp,
            } => Self::ApprovalRequested {
                execution_id,
                node_execution_id,
                node_name,
                timestamp,
            },
            StoredWorkflowEventV1::ApprovalResolved {
                execution_id,
                node_execution_id,
                node_name,
                comment,
                timestamp,
            } => Self::ApprovalResolved {
                execution_id,
                node_execution_id,
                node_name,
                comment,
                timestamp,
            },
            StoredWorkflowEventV1::ContractViolated {
                execution_id,
                node_execution_id,
                node_name,
                violations,
                repair_attempt,
                request_id,
                timestamp,
            } => Self::ContractViolated {
                execution_id,
                node_execution_id,
                node_name,
                violations,
                repair_attempt,
                request_id,
                timestamp,
            },
            StoredWorkflowEventV1::StallObserved {
                execution_id,
                node_execution_id,
                node_name,
                attempt,
                session_id,
                turn_phase,
                idle_secs,
                signal_count,
                cap_reached,
                timestamp,
            } => Self::StallObserved {
                execution_id,
                node_execution_id,
                node_name,
                attempt,
                session_id,
                turn_phase,
                idle_secs,
                signal_count,
                cap_reached,
                timestamp,
            },
            StoredWorkflowEventV1::StallCleared {
                execution_id,
                node_execution_id,
                session_id,
                timestamp,
            } => Self::StallCleared {
                execution_id,
                node_execution_id,
                session_id,
                timestamp,
            },
            StoredWorkflowEventV1::ExecutionCompleted {
                execution_id,
                total_token_usage,
                timestamp,
            } => Self::ExecutionCompleted {
                execution_id,
                total_token_usage,
                timestamp,
            },
            StoredWorkflowEventV1::ExecutionAborted {
                execution_id,
                aborted_node,
                timestamp,
            } => Self::ExecutionAborted {
                execution_id,
                aborted_node,
                timestamp,
            },
            StoredWorkflowEventV1::ExecutionInterrupted {
                execution_id,
                reason,
                timestamp,
            } => Self::ExecutionInterrupted {
                execution_id,
                reason: reason.into(),
                timestamp,
            },
            StoredWorkflowEventV1::ExecutionResumed {
                execution_id,
                resume_from_node,
                timestamp,
            } => Self::ExecutionResumed {
                execution_id,
                resume_from_node,
                timestamp,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg(test)]
pub(crate) struct StoredWorkflowPayloadSource {
    pub source_id: String,
    pub record_ordinal: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg(test)]
pub(crate) struct PreservedStoredWorkflowPayload {
    pub source: StoredWorkflowPayloadSource,
    pub payload_version: u32,
    pub type_tag: String,
    pub raw_bytes: Vec<u8>,
}

#[derive(Debug, Clone)]
#[cfg(test)]
pub(crate) struct DecodedStoredWorkflowEventV1 {
    pub event: WorkflowEvent,
    pub preserved_additive_payload: Option<PreservedStoredWorkflowPayload>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("incompatible stored workflow event type={type_tag} version={payload_version}: {reason}")]
#[cfg(test)]
pub(crate) struct IncompatibleStoredWorkflowEvent {
    pub type_tag: String,
    pub payload_version: u32,
    pub reason: String,
}

#[cfg(test)]
pub(crate) fn decode_stored_workflow_event_v1(
    raw: &[u8],
    payload_version: u32,
    source: StoredWorkflowPayloadSource,
) -> Result<DecodedStoredWorkflowEventV1, IncompatibleStoredWorkflowEvent> {
    if payload_version != 1 {
        return Err(IncompatibleStoredWorkflowEvent {
            type_tag: "workflow_event".to_string(),
            payload_version,
            reason: "unsupported required payload version".to_string(),
        });
    }
    let original: serde_json::Value = serde_json::from_slice(raw).map_err(|error| {
        incompatible_workflow_event("workflow_event", format!("invalid JSON: {error}"))
    })?;
    let type_tag = original
        .as_object()
        .and_then(|object| object.get("event"))
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| incompatible_workflow_event("workflow_event", "missing required event tag"))?
        .to_string();
    let stored: StoredWorkflowEventV1 =
        serde_json::from_value(original.clone()).map_err(|error| {
            incompatible_workflow_event(type_tag.clone(), format!("invalid known payload: {error}"))
        })?;
    let canonical = serde_json::to_value(&stored)
        .expect("stored workflow event serialization must be deterministic");
    let has_additive = contains_additive_fields(&original, &canonical);
    let event = WorkflowEvent::from(stored);
    to_domain_event(&event).map_err(|error| {
        incompatible_workflow_event(type_tag.clone(), format!("invalid semantics: {error}"))
    })?;
    Ok(DecodedStoredWorkflowEventV1 {
        event,
        preserved_additive_payload: has_additive.then(|| PreservedStoredWorkflowPayload {
            source,
            payload_version,
            type_tag,
            raw_bytes: raw.to_vec(),
        }),
    })
}

pub(crate) fn encode_stored_workflow_event_v1(
    event: &WorkflowEvent,
) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(&StoredWorkflowEventV1::from(event))
}

#[cfg(test)]
fn incompatible_workflow_event(
    type_tag: impl Into<String>,
    reason: impl Into<String>,
) -> IncompatibleStoredWorkflowEvent {
    IncompatibleStoredWorkflowEvent {
        type_tag: type_tag.into(),
        payload_version: 1,
        reason: reason.into(),
    }
}

#[cfg(test)]
fn contains_additive_fields(original: &serde_json::Value, canonical: &serde_json::Value) -> bool {
    match (original, canonical) {
        (serde_json::Value::Object(original), serde_json::Value::Object(canonical)) => {
            original.iter().any(|(key, value)| {
                canonical
                    .get(key)
                    .is_none_or(|canonical_value| contains_additive_fields(value, canonical_value))
            })
        }
        (serde_json::Value::Array(original), serde_json::Value::Array(canonical)) => {
            original.len() != canonical.len()
                || original
                    .iter()
                    .zip(canonical)
                    .any(|(value, canonical_value)| {
                        contains_additive_fields(value, canonical_value)
                    })
        }
        _ => false,
    }
}

impl WorkflowEvent {
    pub fn execution_id(&self) -> &str {
        match self {
            Self::ExecutionStarted { execution_id, .. }
            | Self::NodeStarted { execution_id, .. }
            | Self::SessionAttached { execution_id, .. }
            | Self::NodeSubmitReceived { execution_id, .. }
            | Self::NodeStopReceived { execution_id, .. }
            | Self::NodeRetryRequested { execution_id, .. }
            | Self::NodePaused { execution_id, .. }
            | Self::NodeResumed { execution_id, .. }
            | Self::CommandPrepared { execution_id, .. }
            | Self::ArtifactProduced { execution_id, .. }
            | Self::NodeCompleted { execution_id, .. }
            | Self::NodeFailed { execution_id, .. }
            | Self::ApprovalRequested { execution_id, .. }
            | Self::ApprovalResolved { execution_id, .. }
            | Self::ContractViolated { execution_id, .. }
            | Self::StallObserved { execution_id, .. }
            | Self::StallCleared { execution_id, .. }
            | Self::ExecutionCompleted { execution_id, .. }
            | Self::ExecutionAborted { execution_id, .. }
            | Self::ExecutionInterrupted { execution_id, .. }
            | Self::ExecutionResumed { execution_id, .. } => execution_id,
        }
    }

    pub fn timestamp(&self) -> f64 {
        match self {
            Self::ExecutionStarted { timestamp, .. }
            | Self::NodeStarted { timestamp, .. }
            | Self::SessionAttached { timestamp, .. }
            | Self::NodeSubmitReceived { timestamp, .. }
            | Self::NodeStopReceived { timestamp, .. }
            | Self::NodeRetryRequested { timestamp, .. }
            | Self::NodePaused { timestamp, .. }
            | Self::NodeResumed { timestamp, .. }
            | Self::CommandPrepared { timestamp, .. }
            | Self::ArtifactProduced { timestamp, .. }
            | Self::NodeCompleted { timestamp, .. }
            | Self::NodeFailed { timestamp, .. }
            | Self::ApprovalRequested { timestamp, .. }
            | Self::ApprovalResolved { timestamp, .. }
            | Self::ContractViolated { timestamp, .. }
            | Self::StallObserved { timestamp, .. }
            | Self::StallCleared { timestamp, .. }
            | Self::ExecutionCompleted { timestamp, .. }
            | Self::ExecutionAborted { timestamp, .. }
            | Self::ExecutionInterrupted { timestamp, .. }
            | Self::ExecutionResumed { timestamp, .. } => *timestamp,
        }
    }
}

/// Total conversion from the versioned NDJSON DTO into the canonical workflow
/// domain event. Node-execution semantics are therefore owned only by
/// `domain::workflow::WorkflowDomainEvent`.
pub(crate) fn to_domain_event(
    event: &WorkflowEvent,
) -> Result<crate::domain::workflow::WorkflowDomainEvent, crate::domain::workflow::WorkflowError> {
    use crate::domain::workflow::{
        NodeKindName as DomainNodeKindName, TokenUsage as DomainTokenUsage,
        WorkflowContractViolation, WorkflowDomainEvent as Domain, WorkflowJsonPayload,
    };
    use WorkflowEvent as Stored;

    let token_usage = |usage: &TokenUsage| DomainTokenUsage {
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
    };
    let kind = |kind: NodeKindName| match kind {
        NodeKindName::Command => DomainNodeKindName::Command,
        NodeKindName::Session => DomainNodeKindName::Session,
        NodeKindName::Fanout => DomainNodeKindName::Fanout,
        NodeKindName::Sequence => DomainNodeKindName::Sequence,
    };

    Ok(match event {
        Stored::ExecutionStarted {
            execution_id,
            workflow_name,
            worktree_path,
            created_from,
            request,
            definition,
            timestamp,
        } => Domain::WorkflowExecutionStarted {
            execution_id: execution_id.clone(),
            workflow_name: workflow_name.clone(),
            worktree_path: worktree_path.clone(),
            created_from: *created_from,
            request: request.clone(),
            definition: crate::adaptor::gateway::workflow::mapper::schema_workflow_to_domain(
                definition.clone(),
            )?,
            timestamp: *timestamp,
        },
        Stored::NodeStarted {
            execution_id,
            node_execution_id,
            node_name,
            kind: node_kind,
            attempt,
            parent,
            timestamp,
        } => Domain::NodeExecutionStarted {
            execution_id: execution_id.clone(),
            node_execution_id: node_execution_id.clone(),
            node_name: node_name.clone(),
            kind: kind(*node_kind),
            attempt: *attempt,
            parent: parent.clone(),
            timestamp: *timestamp,
        },
        Stored::SessionAttached {
            execution_id,
            node_execution_id,
            session_id,
            timestamp,
        } => Domain::NodeExecutionAgentBound {
            execution_id: execution_id.clone(),
            node_execution_id: node_execution_id.clone(),
            session_id: session_id.clone(),
            timestamp: *timestamp,
        },
        Stored::NodeSubmitReceived {
            execution_id,
            node_execution_id,
            timestamp,
        } => Domain::NodeExecutionSubmitReceived {
            execution_id: execution_id.clone(),
            node_execution_id: node_execution_id.clone(),
            timestamp: *timestamp,
        },
        Stored::NodeStopReceived {
            execution_id,
            node_execution_id,
            timestamp,
        } => Domain::NodeExecutionStopReceived {
            execution_id: execution_id.clone(),
            node_execution_id: node_execution_id.clone(),
            timestamp: *timestamp,
        },
        Stored::NodeRetryRequested {
            execution_id,
            node_execution_id,
            timestamp,
        } => Domain::NodeExecutionRetryRequested {
            execution_id: execution_id.clone(),
            node_execution_id: node_execution_id.clone(),
            timestamp: *timestamp,
        },
        Stored::NodePaused {
            execution_id,
            node_execution_id,
            timestamp,
        } => Domain::NodeExecutionPaused {
            execution_id: execution_id.clone(),
            node_execution_id: node_execution_id.clone(),
            timestamp: *timestamp,
        },
        Stored::NodeResumed {
            execution_id,
            node_execution_id,
            timestamp,
        } => Domain::NodeExecutionResumed {
            execution_id: execution_id.clone(),
            node_execution_id: node_execution_id.clone(),
            timestamp: *timestamp,
        },
        Stored::CommandPrepared {
            execution_id,
            node_execution_id,
            display_command,
            timestamp,
        } => Domain::NodeExecutionCommandPrepared {
            execution_id: execution_id.clone(),
            node_execution_id: node_execution_id.clone(),
            display_command: display_command.clone(),
            timestamp: *timestamp,
        },
        Stored::ArtifactProduced {
            execution_id,
            node_execution_id,
            node_name,
            contract,
            value,
            request_id,
            submitted_at,
            timestamp,
        } => Domain::WorkflowArtifactProduced {
            execution_id: execution_id.clone(),
            node_execution_id: node_execution_id.clone(),
            node_name: node_name.clone(),
            contract: contract.clone(),
            value: WorkflowJsonPayload::new_validated(
                serde_json::to_string(value).expect("JSON value serialization cannot fail"),
            ),
            request_id: request_id.clone(),
            submitted_at: *submitted_at,
            timestamp: *timestamp,
        },
        Stored::NodeCompleted {
            execution_id,
            node_execution_id,
            node_name,
            attempt,
            result_summary,
            token_usage: usage,
            timestamp,
        } => Domain::NodeExecutionCompleted {
            execution_id: execution_id.clone(),
            node_execution_id: node_execution_id.clone(),
            node_name: node_name.clone(),
            attempt: *attempt,
            result_summary: result_summary.clone(),
            token_usage: usage.as_ref().map(token_usage),
            timestamp: *timestamp,
        },
        Stored::NodeFailed {
            execution_id,
            node_execution_id,
            node_name,
            attempt,
            reason,
            failure_kind,
            retry_count,
            timestamp,
        } => Domain::NodeExecutionFailed {
            execution_id: execution_id.clone(),
            node_execution_id: node_execution_id.clone(),
            node_name: node_name.clone(),
            attempt: *attempt,
            reason: reason.clone(),
            failure_kind: *failure_kind,
            retry_count: *retry_count,
            timestamp: *timestamp,
        },
        Stored::ApprovalRequested {
            execution_id,
            node_execution_id,
            node_name,
            timestamp,
        } => Domain::WorkflowApprovalRequested {
            execution_id: execution_id.clone(),
            node_execution_id: node_execution_id.clone(),
            node_name: node_name.clone(),
            timestamp: *timestamp,
        },
        Stored::ApprovalResolved {
            execution_id,
            node_execution_id,
            node_name,
            comment,
            timestamp,
        } => Domain::WorkflowApprovalResolved {
            execution_id: execution_id.clone(),
            node_execution_id: node_execution_id.clone(),
            node_name: node_name.clone(),
            comment: comment.clone(),
            timestamp: *timestamp,
        },
        Stored::ContractViolated {
            execution_id,
            node_execution_id,
            node_name,
            violations,
            repair_attempt,
            request_id,
            timestamp,
        } => Domain::WorkflowContractViolated {
            execution_id: execution_id.clone(),
            node_execution_id: node_execution_id.clone(),
            node_name: node_name.clone(),
            violations: violations
                .iter()
                .map(|violation| WorkflowContractViolation {
                    path: violation.path.clone(),
                    reason: violation.reason.clone(),
                })
                .collect(),
            repair_attempt: *repair_attempt,
            request_id: request_id.clone(),
            timestamp: *timestamp,
        },
        Stored::StallObserved {
            execution_id,
            node_execution_id,
            node_name,
            attempt,
            session_id,
            turn_phase,
            idle_secs,
            signal_count,
            cap_reached,
            timestamp,
        } => Domain::NodeExecutionStallObserved {
            execution_id: execution_id.clone(),
            node_execution_id: node_execution_id.clone(),
            node_name: node_name.clone(),
            attempt: *attempt,
            session_id: session_id.clone(),
            turn_phase: turn_phase.clone(),
            idle_secs: *idle_secs,
            signal_count: *signal_count,
            cap_reached: *cap_reached,
            timestamp: *timestamp,
        },
        Stored::StallCleared {
            execution_id,
            node_execution_id,
            session_id,
            timestamp,
        } => Domain::NodeExecutionStallCleared {
            execution_id: execution_id.clone(),
            node_execution_id: node_execution_id.clone(),
            session_id: session_id.clone(),
            timestamp: *timestamp,
        },
        Stored::ExecutionCompleted {
            execution_id,
            total_token_usage,
            timestamp,
        } => Domain::WorkflowExecutionCompleted {
            execution_id: execution_id.clone(),
            total_token_usage: token_usage(total_token_usage),
            timestamp: *timestamp,
        },
        Stored::ExecutionAborted {
            execution_id,
            aborted_node,
            timestamp,
        } => Domain::WorkflowExecutionAborted {
            execution_id: execution_id.clone(),
            aborted_node: aborted_node.clone(),
            timestamp: *timestamp,
        },
        Stored::ExecutionInterrupted {
            execution_id,
            reason,
            timestamp,
        } => Domain::WorkflowExecutionInterrupted {
            execution_id: execution_id.clone(),
            reason: *reason,
            timestamp: *timestamp,
        },
        Stored::ExecutionResumed {
            execution_id,
            resume_from_node,
            timestamp,
        } => Domain::WorkflowExecutionResumed {
            execution_id: execution_id.clone(),
            resume_from_node: resume_from_node.clone(),
            timestamp: *timestamp,
        },
    })
}

/// Total conversion from the canonical workflow domain event into the
/// versioned gateway DTO used by both legacy NDJSON and the SQLite codec.
pub(crate) fn from_domain_event(
    event: &crate::domain::workflow::WorkflowDomainEvent,
) -> Result<WorkflowEvent, crate::domain::workflow::WorkflowError> {
    use crate::domain::workflow::{NodeKindName as DomainNodeKindName, WorkflowDomainEvent as D};

    let token_usage = |usage: &crate::domain::workflow::TokenUsage| TokenUsage {
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
    };
    let node_kind = |kind: DomainNodeKindName| match kind {
        DomainNodeKindName::Command => NodeKindName::Command,
        DomainNodeKindName::Session => NodeKindName::Session,
        DomainNodeKindName::Fanout => NodeKindName::Fanout,
        DomainNodeKindName::Sequence => NodeKindName::Sequence,
    };

    Ok(match event {
        D::WorkflowExecutionStarted {
            execution_id,
            workflow_name,
            worktree_path,
            created_from,
            request,
            definition,
            timestamp,
        } => WorkflowEvent::ExecutionStarted {
            execution_id: execution_id.clone(),
            workflow_name: workflow_name.clone(),
            worktree_path: worktree_path.clone(),
            created_from: *created_from,
            request: request.clone(),
            definition: crate::adaptor::gateway::workflow::mapper::domain_workflow_to_schema(
                definition,
            )?,
            timestamp: *timestamp,
        },
        D::NodeExecutionStarted {
            execution_id,
            node_execution_id,
            node_name,
            kind,
            attempt,
            parent,
            timestamp,
        } => WorkflowEvent::NodeStarted {
            execution_id: execution_id.clone(),
            node_execution_id: node_execution_id.clone(),
            node_name: node_name.clone(),
            kind: node_kind(*kind),
            attempt: *attempt,
            parent: parent.clone(),
            timestamp: *timestamp,
        },
        D::NodeExecutionAgentBound {
            execution_id,
            node_execution_id,
            session_id,
            timestamp,
        } => WorkflowEvent::SessionAttached {
            execution_id: execution_id.clone(),
            node_execution_id: node_execution_id.clone(),
            session_id: session_id.clone(),
            timestamp: *timestamp,
        },
        D::NodeExecutionSubmitReceived {
            execution_id,
            node_execution_id,
            timestamp,
        } => WorkflowEvent::NodeSubmitReceived {
            execution_id: execution_id.clone(),
            node_execution_id: node_execution_id.clone(),
            timestamp: *timestamp,
        },
        D::NodeExecutionStopReceived {
            execution_id,
            node_execution_id,
            timestamp,
        } => WorkflowEvent::NodeStopReceived {
            execution_id: execution_id.clone(),
            node_execution_id: node_execution_id.clone(),
            timestamp: *timestamp,
        },
        D::NodeExecutionRetryRequested {
            execution_id,
            node_execution_id,
            timestamp,
        } => WorkflowEvent::NodeRetryRequested {
            execution_id: execution_id.clone(),
            node_execution_id: node_execution_id.clone(),
            timestamp: *timestamp,
        },
        D::NodeExecutionPaused {
            execution_id,
            node_execution_id,
            timestamp,
        } => WorkflowEvent::NodePaused {
            execution_id: execution_id.clone(),
            node_execution_id: node_execution_id.clone(),
            timestamp: *timestamp,
        },
        D::NodeExecutionResumed {
            execution_id,
            node_execution_id,
            timestamp,
        } => WorkflowEvent::NodeResumed {
            execution_id: execution_id.clone(),
            node_execution_id: node_execution_id.clone(),
            timestamp: *timestamp,
        },
        D::NodeExecutionCommandPrepared {
            execution_id,
            node_execution_id,
            display_command,
            timestamp,
        } => WorkflowEvent::CommandPrepared {
            execution_id: execution_id.clone(),
            node_execution_id: node_execution_id.clone(),
            display_command: display_command.clone(),
            timestamp: *timestamp,
        },
        D::WorkflowArtifactProduced {
            execution_id,
            node_execution_id,
            node_name,
            contract,
            value,
            request_id,
            submitted_at,
            timestamp,
        } => WorkflowEvent::ArtifactProduced {
            execution_id: execution_id.clone(),
            node_execution_id: node_execution_id.clone(),
            node_name: node_name.clone(),
            contract: contract.clone(),
            value: serde_json::from_str(value.as_str()).map_err(|_| {
                crate::domain::workflow::WorkflowError::validation(
                    "workflow artifact payload is not valid JSON",
                )
            })?,
            request_id: request_id.clone(),
            submitted_at: *submitted_at,
            timestamp: *timestamp,
        },
        D::NodeExecutionCompleted {
            execution_id,
            node_execution_id,
            node_name,
            attempt,
            result_summary,
            token_usage: usage,
            timestamp,
        } => WorkflowEvent::NodeCompleted {
            execution_id: execution_id.clone(),
            node_execution_id: node_execution_id.clone(),
            node_name: node_name.clone(),
            attempt: *attempt,
            result_summary: result_summary.clone(),
            token_usage: usage.as_ref().map(token_usage),
            timestamp: *timestamp,
        },
        D::NodeExecutionFailed {
            execution_id,
            node_execution_id,
            node_name,
            attempt,
            reason,
            failure_kind,
            retry_count,
            timestamp,
        } => WorkflowEvent::NodeFailed {
            execution_id: execution_id.clone(),
            node_execution_id: node_execution_id.clone(),
            node_name: node_name.clone(),
            attempt: *attempt,
            reason: reason.clone(),
            failure_kind: *failure_kind,
            retry_count: *retry_count,
            timestamp: *timestamp,
        },
        D::WorkflowApprovalRequested {
            execution_id,
            node_execution_id,
            node_name,
            timestamp,
        } => WorkflowEvent::ApprovalRequested {
            execution_id: execution_id.clone(),
            node_execution_id: node_execution_id.clone(),
            node_name: node_name.clone(),
            timestamp: *timestamp,
        },
        D::WorkflowApprovalResolved {
            execution_id,
            node_execution_id,
            node_name,
            comment,
            timestamp,
        } => WorkflowEvent::ApprovalResolved {
            execution_id: execution_id.clone(),
            node_execution_id: node_execution_id.clone(),
            node_name: node_name.clone(),
            comment: comment.clone(),
            timestamp: *timestamp,
        },
        D::WorkflowContractViolated {
            execution_id,
            node_execution_id,
            node_name,
            violations,
            repair_attempt,
            request_id,
            timestamp,
        } => WorkflowEvent::ContractViolated {
            execution_id: execution_id.clone(),
            node_execution_id: node_execution_id.clone(),
            node_name: node_name.clone(),
            violations: violations
                .iter()
                .map(|violation| ContractViolationRecord {
                    path: violation.path.clone(),
                    reason: violation.reason.clone(),
                })
                .collect(),
            repair_attempt: *repair_attempt,
            request_id: request_id.clone(),
            timestamp: *timestamp,
        },
        D::NodeExecutionStallObserved {
            execution_id,
            node_execution_id,
            node_name,
            attempt,
            session_id,
            turn_phase,
            idle_secs,
            signal_count,
            cap_reached,
            timestamp,
        } => WorkflowEvent::StallObserved {
            execution_id: execution_id.clone(),
            node_execution_id: node_execution_id.clone(),
            node_name: node_name.clone(),
            attempt: *attempt,
            session_id: session_id.clone(),
            turn_phase: turn_phase.clone(),
            idle_secs: *idle_secs,
            signal_count: *signal_count,
            cap_reached: *cap_reached,
            timestamp: *timestamp,
        },
        D::NodeExecutionStallCleared {
            execution_id,
            node_execution_id,
            session_id,
            timestamp,
        } => WorkflowEvent::StallCleared {
            execution_id: execution_id.clone(),
            node_execution_id: node_execution_id.clone(),
            session_id: session_id.clone(),
            timestamp: *timestamp,
        },
        D::WorkflowExecutionCompleted {
            execution_id,
            total_token_usage,
            timestamp,
        } => WorkflowEvent::ExecutionCompleted {
            execution_id: execution_id.clone(),
            total_token_usage: token_usage(total_token_usage),
            timestamp: *timestamp,
        },
        D::WorkflowExecutionAborted {
            execution_id,
            aborted_node,
            timestamp,
        } => WorkflowEvent::ExecutionAborted {
            execution_id: execution_id.clone(),
            aborted_node: aborted_node.clone(),
            timestamp: *timestamp,
        },
        D::WorkflowExecutionInterrupted {
            execution_id,
            reason,
            timestamp,
        } => WorkflowEvent::ExecutionInterrupted {
            execution_id: execution_id.clone(),
            reason: *reason,
            timestamp: *timestamp,
        },
        D::WorkflowExecutionResumed {
            execution_id,
            resume_from_node,
            timestamp,
        } => WorkflowEvent::ExecutionResumed {
            execution_id: execution_id.clone(),
            resume_from_node: resume_from_node.clone(),
            timestamp: *timestamp,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adaptor::gateway::workflow::schema::{
        FacetRefs, NodeDefinition, NodeKind, SessionSpec,
    };

    fn minimal_workflow() -> WorkflowDefinitionYaml {
        WorkflowDefinitionYaml {
            name: "wf".to_string(),
            description: String::new(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![NodeDefinition {
                name: "main".to_string(),
                kind: NodeKind::Session(SessionSpec {
                    facets: FacetRefs {
                        instruction: Some("review".to_string()),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
                ..NodeDefinition::default()
            }],
            entry: "main".to_string(),
        }
    }

    #[test]
    fn stored_v1_does_not_embed_domain_enum_serialization() {
        let source = include_str!("event.rs");
        let stored = source
            .split("enum StoredWorkflowEventV1")
            .nth(1)
            .unwrap()
            .split("impl From<ExecutionOrigin>")
            .next()
            .unwrap();
        assert!(!stored.contains("created_from: ExecutionOrigin"));
        assert!(!stored.contains("failure_kind: NodeExecutionFailureKind"));
        assert!(!stored.contains("reason: ExecutionInterruptionReason"));
    }

    #[test]
    fn stored_v1_codec_preserves_existing_semantic_json_shape() {
        let events = [
            WorkflowEvent::ExecutionStarted {
                execution_id: "00000000-0000-4000-8000-000000000001".into(),
                workflow_name: "wf".into(),
                worktree_path: "/repo".into(),
                created_from: ExecutionOrigin::DesktopUi,
                request: "run".into(),
                definition: minimal_workflow(),
                timestamp: 1.0,
            },
            WorkflowEvent::ExecutionInterrupted {
                execution_id: "00000000-0000-4000-8000-000000000001".into(),
                reason: ExecutionInterruptionReason::Stop,
                timestamp: 3.0,
            },
        ];
        for event in events {
            let semantic = serde_json::to_value(&event).unwrap();
            let stored: serde_json::Value =
                serde_json::from_slice(&encode_stored_workflow_event_v1(&event).unwrap()).unwrap();
            assert_eq!(stored, semantic);
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
        assert!(value.get("permission_mode").is_none());
        assert!(serde_json::from_value::<WorkflowEvent>(value).is_ok());
        let domain = to_domain_event(&event).unwrap();
        assert!(matches!(
            domain,
            crate::domain::workflow::WorkflowDomainEvent::WorkflowExecutionStarted {
                ref workflow_name,
                ..
            } if workflow_name == "wf"
        ));
    }

    #[test]
    fn execution_started_round_trips_loop_guard() {
        let mut definition = minimal_workflow();
        definition.nodes[0].name = "fix".to_string();
        definition.nodes.push(NodeDefinition {
            name: "main".to_string(),
            kind: NodeKind::Sequence(crate::adaptor::gateway::workflow::schema::SequenceSpec {
                entry: None,
                output: None,
                children: vec![crate::domain::workflow::ChildEntry {
                    name: "fix".to_string(),
                    inputs: Vec::new(),
                    rules: Some(vec![
                        crate::adaptor::gateway::workflow::schema::Rule::LoopGuard {
                            max_iterations: 2,
                            on_exhausted: "fix".to_string(),
                        },
                    ]),
                }],
            }),
            ..NodeDefinition::default()
        });
        let event = WorkflowEvent::ExecutionStarted {
            execution_id: "00000000-0000-4000-8000-000000000001".to_string(),
            workflow_name: "wf".to_string(),
            worktree_path: "/repo".to_string(),
            created_from: ExecutionOrigin::Cli,
            request: "review".to_string(),
            definition,
            timestamp: 1.0,
        };

        let serialized = serde_json::to_value(event).unwrap();
        assert_eq!(
            serialized["definition"]["nodes"]["main"]["sequence"]["children"][0]["fix"]["rules"][0]
                ["loop_guard"]["max_iterations"],
            2
        );

        let restored = serde_json::from_value::<WorkflowEvent>(serialized).unwrap();
        let WorkflowEvent::ExecutionStarted { definition, .. } = restored else {
            panic!("expected execution_started event");
        };
        let sequence = definition
            .nodes
            .iter()
            .find(|node| node.name == "main")
            .and_then(|node| node.sequence())
            .expect("main sequence survives the round trip");
        assert!(matches!(
            sequence.children[0].rules.as_deref(),
            Some(
                [crate::adaptor::gateway::workflow::schema::Rule::LoopGuard {
                    max_iterations: 2,
                    ..
                }]
            )
        ));
    }

    #[test]
    fn execution_started_reads_legacy_scalar_knowledge_snapshot() {
        let event = WorkflowEvent::ExecutionStarted {
            execution_id: "00000000-0000-4000-8000-000000000001".to_string(),
            workflow_name: "wf".to_string(),
            worktree_path: "/repo".to_string(),
            created_from: ExecutionOrigin::Cli,
            request: "review".to_string(),
            definition: minimal_workflow(),
            timestamp: 1.0,
        };
        let mut value = serde_json::to_value(event).unwrap();
        value["definition"]["nodes"]["main"]["session"]["facets"]["knowledge"] =
            serde_json::json!("legacy-knowledge");

        let restored = serde_json::from_value::<WorkflowEvent>(value).unwrap();
        let WorkflowEvent::ExecutionStarted { definition, .. } = &restored else {
            panic!("expected execution_started event");
        };
        assert_eq!(
            definition.nodes[0].session().unwrap().facets.knowledge,
            vec!["legacy-knowledge"]
        );

        let serialized = serde_json::to_value(restored).unwrap();
        assert_eq!(
            serialized["definition"]["nodes"]["main"]["session"]["facets"]["knowledge"],
            serde_json::json!("legacy-knowledge")
        );
    }

    #[test]
    fn execution_started_round_trips_multiple_knowledge_snapshot() {
        let mut definition = minimal_workflow();
        definition.nodes[0].session_mut().unwrap().facets.knowledge =
            vec!["knowledge-a".to_string(), "knowledge-b".to_string()];
        let event = WorkflowEvent::ExecutionStarted {
            execution_id: "00000000-0000-4000-8000-000000000001".to_string(),
            workflow_name: "wf".to_string(),
            worktree_path: "/repo".to_string(),
            created_from: ExecutionOrigin::Cli,
            request: "review".to_string(),
            definition,
            timestamp: 1.0,
        };

        let serialized = serde_json::to_value(&event).unwrap();
        assert_eq!(
            serialized["definition"]["nodes"]["main"]["session"]["facets"]["knowledge"],
            serde_json::json!(["knowledge-a", "knowledge-b"])
        );

        let restored = serde_json::from_value::<WorkflowEvent>(serialized).unwrap();
        let WorkflowEvent::ExecutionStarted { definition, .. } = restored else {
            panic!("expected execution_started event");
        };
        assert_eq!(
            definition.nodes[0].session().unwrap().facets.knowledge,
            vec!["knowledge-a", "knowledge-b"]
        );
    }

    #[test]
    fn execution_started_does_not_serialize_releash_owned_permission_mode() {
        let event = WorkflowEvent::ExecutionStarted {
            execution_id: "00000000-0000-4000-8000-000000000001".to_string(),
            workflow_name: "wf".to_string(),
            worktree_path: "/repo".to_string(),
            created_from: ExecutionOrigin::Cli,
            request: "review".to_string(),
            definition: minimal_workflow(),
            timestamp: 1.0,
        };
        let value = serde_json::to_value(event).unwrap();
        assert!(value.get("permission_mode").is_none());
        assert!(serde_json::from_value::<WorkflowEvent>(value).is_ok());
    }

    #[test]
    fn command_prepared_round_trips_masked_display_command() {
        let event = WorkflowEvent::CommandPrepared {
            execution_id: "00000000-0000-4000-8000-000000000001".to_string(),
            node_execution_id: "00000000-0000-4000-8000-000000000002".to_string(),
            display_command: "printf '%s' '[REDACTED]'".to_string(),
            timestamp: 2.0,
        };

        let value = serde_json::to_value(&event).unwrap();
        assert_eq!(value["event"], "command_prepared");
        assert_eq!(value["display_command"], "printf '%s' '[REDACTED]'");
        assert_eq!(value["execution_id"], event.execution_id());
        assert!(serde_json::from_value::<WorkflowEvent>(value).is_ok());
    }

    #[test]
    fn node_completion_signal_events_are_part_of_the_durable_contract() {
        for event_name in ["node_submit_received", "node_stop_received"] {
            let value = serde_json::json!({
                "event": event_name,
                "execution_id": "00000000-0000-4000-8000-000000000001",
                "node_execution_id": "00000000-0000-4000-8000-000000000002",
                "timestamp": 3.0,
            });

            let event = serde_json::from_value::<WorkflowEvent>(value)
                .unwrap_or_else(|_| panic!("{event_name} must be a replayable Workflow event"));
            let encoded = encode_stored_workflow_event_v1(&event).unwrap();
            let decoded = decode_stored_workflow_event_v1(
                &encoded,
                1,
                StoredWorkflowPayloadSource {
                    source_id: "test".to_string(),
                    record_ordinal: 1,
                },
            )
            .unwrap();
            assert_eq!(decoded.event, event);
            let domain = to_domain_event(&event).unwrap();
            assert_eq!(from_domain_event(&domain).unwrap(), event);
        }
    }

    #[test]
    fn node_retry_requested_is_part_of_the_durable_contract() {
        let value = serde_json::json!({
            "event": "node_retry_requested",
            "execution_id": "00000000-0000-4000-8000-000000000001",
            "node_execution_id": "00000000-0000-4000-8000-000000000002",
            "timestamp": 4.0,
        });

        let event = serde_json::from_value::<WorkflowEvent>(value)
            .expect("node_retry_requested must be a replayable Workflow event");
        let encoded = encode_stored_workflow_event_v1(&event).unwrap();
        let decoded = decode_stored_workflow_event_v1(
            &encoded,
            1,
            StoredWorkflowPayloadSource {
                source_id: "test".to_string(),
                record_ordinal: 1,
            },
        )
        .unwrap();
        assert_eq!(decoded.event, event);
        let domain = to_domain_event(&event).unwrap();
        assert_eq!(from_domain_event(&domain).unwrap(), event);
    }

    #[test]
    fn interruption_and_resume_round_trip_canonical_vocabulary() {
        let interrupted = WorkflowEvent::ExecutionInterrupted {
            execution_id: "00000000-0000-4000-8000-000000000001".to_string(),
            reason: ExecutionInterruptionReason::Stale,
            timestamp: 2.0,
        };
        let value = serde_json::to_value(&interrupted).unwrap();
        assert_eq!(value["event"], "execution_interrupted");
        assert_eq!(value["reason"], "stale");
        assert!(matches!(
            serde_json::from_value::<WorkflowEvent>(value).unwrap(),
            WorkflowEvent::ExecutionInterrupted {
                reason: ExecutionInterruptionReason::Stale,
                ..
            }
        ));

        let resumed = WorkflowEvent::ExecutionResumed {
            execution_id: "00000000-0000-4000-8000-000000000001".to_string(),
            resume_from_node: "review".to_string(),
            timestamp: 3.0,
        };
        let value = serde_json::to_value(&resumed).unwrap();
        assert_eq!(value["event"], "execution_resumed");
        assert_eq!(value["resume_from_node"], "review");
        assert!(serde_json::from_value::<WorkflowEvent>(value).is_ok());
    }

    #[test]
    fn interrupted_event_rejects_noncanonical_reason() {
        let value = serde_json::json!({
            "event": "execution_interrupted",
            "execution_id": "00000000-0000-4000-8000-000000000001",
            "reason": "app_exit",
            "timestamp": 2.0
        });
        assert!(serde_json::from_value::<WorkflowEvent>(value).is_err());
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
    fn canonical_variants_preserve_additive_identity_fields_but_reject_unknown_event() {
        let execution = serde_json::json!({
            "event": "execution_aborted",
            "execution_id": "00000000-0000-4000-8000-000000000001",
            "run_id": "00000000-0000-4000-8000-000000000001",
            "timestamp": 1.0
        });
        let execution_raw = serde_json::to_vec(&execution).unwrap();
        assert!(decode_stored_workflow_event_v1(
            &execution_raw,
            1,
            StoredWorkflowPayloadSource {
                source_id: "workflow.ndjson".into(),
                record_ordinal: 0,
            },
        )
        .unwrap()
        .preserved_additive_payload
        .is_some());

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
        let node_raw = serde_json::to_vec(&node).unwrap();
        assert!(decode_stored_workflow_event_v1(
            &node_raw,
            1,
            StoredWorkflowPayloadSource {
                source_id: "workflow.ndjson".into(),
                record_ordinal: 1,
            },
        )
        .unwrap()
        .preserved_additive_payload
        .is_some());

        let retired_mutation_event = ["cli_mutation", "requested"].join("_");
        let mutation = serde_json::json!({
            "event": retired_mutation_event,
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
    fn canonical_variants_raw_preserve_unknown_nested_fields() {
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
        let raw = serde_json::to_vec(&usage).unwrap();
        let decoded = decode_stored_workflow_event_v1(
            &raw,
            1,
            StoredWorkflowPayloadSource {
                source_id: "workflow.ndjson".into(),
                record_ordinal: 0,
            },
        )
        .unwrap();
        assert_eq!(decoded.preserved_additive_payload.unwrap().raw_bytes, raw);
    }
}
