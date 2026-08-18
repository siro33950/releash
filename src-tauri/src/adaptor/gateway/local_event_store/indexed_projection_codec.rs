//! Gateway-owned codecs for indexed Session and Workflow query records.

use serde::{Deserialize, Serialize};

use crate::domain::local_event::{SessionProjectionRecord, WorkflowExecutionMetadataRecord};
use crate::domain::workflow::{
    ExecutionInterruptionReason, ExecutionOrigin, ExecutionStatus, NodeCompletionSignalState,
    TokenUsage,
};
use crate::domain::workspace_tree::{
    WorkspaceCommandResult, WorkspaceNodeKind, WorkspaceNodeStatus, WorkspaceTreeNode,
};

pub(crate) const EXECUTION_RECORD_SCHEMA: &str = "workflow_execution_record_v1";
pub(crate) const EXECUTION_NODE_RECORD_SCHEMA: &str = "workflow_execution_node_record_v1";
const NODE_TREE_SCHEMA: &str = "workflow_execution_node_tree_v1";
const NODE_DETAIL_SCHEMA: &str = "workflow_execution_node_detail_v1";

pub(crate) struct IndexedExecutionRow {
    pub execution_id: String,
    pub workspace_identity: String,
    pub status: &'static str,
    pub list_kind: &'static str,
    pub sort_at_bits: i64,
    pub record: String,
}

pub(crate) struct IndexedExecutionNodeRow {
    pub execution_id: String,
    pub node_id: String,
    pub parent_id: Option<String>,
    pub sibling_order: i64,
    pub session_id: Option<String>,
    pub node_execution_id: Option<String>,
    pub tree_record: String,
    pub detail_record: String,
}

pub(crate) struct IndexedSessionPublicColumns {
    pub workspace_identity: Option<String>,
    pub list_kind: Option<&'static str>,
    pub sort_key_bits: Option<i64>,
    pub summary: Option<String>,
}

pub(crate) fn indexed_execution_row(
    execution: &WorkflowExecutionMetadataRecord,
) -> Result<IndexedExecutionRow, String> {
    if execution.execution_id.is_empty() {
        return Err("Workflow execution identity is empty".to_string());
    }
    if execution.worktree_path.is_empty()
        || crate::domain::repository::normalize_repo_path(&execution.worktree_path)
            != execution.worktree_path
    {
        return Err("Workflow execution workspace identity is invalid".to_string());
    }
    if !f64::from_bits(execution.started_at_bits).is_finite()
        || !f64::from_bits(execution.updated_at_bits).is_finite()
        || execution
            .completed_at_bits
            .is_some_and(|bits| !f64::from_bits(bits).is_finite())
    {
        return Err("Workflow execution timestamp is not finite".to_string());
    }
    Ok(IndexedExecutionRow {
        execution_id: execution.execution_id.clone(),
        workspace_identity: execution.worktree_path.clone(),
        status: execution.status.as_str(),
        list_kind: if execution.status.is_finished() {
            "terminal"
        } else {
            "active"
        },
        sort_at_bits: i64::try_from(
            execution
                .completed_at_bits
                .unwrap_or(execution.updated_at_bits),
        )
        .map_err(|error| format!("invalid Workflow execution sort key: {error}"))?,
        record: encode_workflow_execution_record_v1(execution)?,
    })
}

pub(crate) fn indexed_execution_node_row(
    execution_id: &str,
    node: &WorkspaceTreeNode,
) -> Result<IndexedExecutionNodeRow, String> {
    if execution_id.is_empty()
        || node.id.is_empty()
        || node.execution_id.as_deref() != Some(execution_id)
    {
        return Err("Workflow execution node identity is inconsistent".to_string());
    }
    let (tree_record, detail_record) = encode_workflow_execution_node_v1(node)?;
    Ok(IndexedExecutionNodeRow {
        execution_id: execution_id.to_string(),
        node_id: node.id.clone(),
        parent_id: node.parent_id.clone(),
        sibling_order: i64::try_from(node.sibling_order)
            .map_err(|error| format!("invalid Workflow node sibling order: {error}"))?,
        session_id: node.session_id.clone(),
        node_execution_id: node.node_execution_id.clone(),
        tree_record,
        detail_record,
    })
}

pub(crate) fn indexed_session_public_columns(
    projection: &SessionProjectionRecord,
) -> Result<IndexedSessionPublicColumns, String> {
    match projection {
        SessionProjectionRecord::AgentSession(session) => Ok(IndexedSessionPublicColumns {
            workspace_identity: Some(session.workspace_identity.clone()),
            list_kind: None,
            sort_key_bits: None,
            summary: None,
        }),
        SessionProjectionRecord::WorkflowExecution(_)
        | SessionProjectionRecord::ProviderSessionOwnership(_)
        | SessionProjectionRecord::ProviderHookHealth(_)
        | SessionProjectionRecord::WorkflowWorktreeOwner(_) => Ok(IndexedSessionPublicColumns {
            workspace_identity: None,
            list_kind: None,
            sort_key_bits: None,
            summary: None,
        }),
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredWorkflowExecutionNodeTreeRecordV1 {
    schema: String,
    node: StoredWorkflowExecutionNodeTreeV1,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredWorkflowExecutionNodeTreeV1 {
    id: String,
    parent_id: Option<String>,
    sibling_order: u64,
    kind: String,
    title: String,
    status: String,
    error_reason: Option<String>,
    updated_at_bits: u64,
    execution_id: Option<String>,
    node_execution_id: Option<String>,
    node_name: Option<String>,
    attempt: Option<u32>,
    #[serde(default)]
    completion_signals: String,
    #[serde(default)]
    has_artifact: bool,
    session_id: Option<String>,
    can_approve: bool,
    #[serde(default)]
    can_retry: bool,
    can_close: bool,
    can_stop: bool,
    can_resume: bool,
    recovery_owner_reason: Option<String>,
    resume_unavailable_reason: Option<String>,
    can_abort: bool,
    can_archive: bool,
    dynamic_fanout: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredWorkflowExecutionNodeDetailRecordV1 {
    schema: String,
    display_command: Option<String>,
    command_result: Option<StoredWorkspaceCommandResultV1>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredWorkspaceCommandResultV1 {
    exit_code: i64,
    duration: u64,
    stdout: String,
    stderr: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredExecutionSummaryV1 {
    schema: String,
    execution_id: String,
    workflow_name: String,
    status: String,
    worktree_path: String,
    current_node: Option<String>,
    created_from: String,
    started_at_bits: u64,
    updated_at_bits: u64,
    completed_at_bits: Option<u64>,
    error_reason: Option<String>,
    interruption_reason: Option<String>,
    resume_from_node: Option<String>,
    input_tokens: u64,
    output_tokens: u64,
}

pub(crate) fn encode_workflow_execution_node_v1(
    node: &WorkspaceTreeNode,
) -> Result<(String, String), String> {
    let tree_record = serde_json::to_string(&StoredWorkflowExecutionNodeTreeRecordV1 {
        schema: NODE_TREE_SCHEMA.to_string(),
        node: encode_node_tree(node),
    })
    .map_err(|error| format!("failed to encode Workflow execution node tree record: {error}"))?;
    let detail_record = serde_json::to_string(&StoredWorkflowExecutionNodeDetailRecordV1 {
        schema: NODE_DETAIL_SCHEMA.to_string(),
        display_command: node.display_command.clone(),
        command_result: node
            .command_result
            .as_ref()
            .map(|result| StoredWorkspaceCommandResultV1 {
                exit_code: result.exit_code,
                duration: result.duration,
                stdout: result.stdout.clone(),
                stderr: result.stderr.clone(),
            }),
    })
    .map_err(|error| format!("failed to encode Workflow execution node detail record: {error}"))?;
    Ok((tree_record, detail_record))
}

pub(crate) fn decode_workflow_execution_node_tree_v1(
    raw: &str,
) -> Result<WorkspaceTreeNode, String> {
    let stored: StoredWorkflowExecutionNodeTreeRecordV1 =
        serde_json::from_str(raw).map_err(|error| {
            format!("failed to decode Workflow execution node tree record: {error}")
        })?;
    if stored.schema != NODE_TREE_SCHEMA {
        return Err("unsupported Workflow execution node tree schema".to_string());
    }
    decode_node_tree(stored.node)
}

pub(crate) fn decode_workflow_execution_node_detail_v1(
    tree_record: &str,
    detail_record: &str,
) -> Result<WorkspaceTreeNode, String> {
    let mut node = decode_workflow_execution_node_tree_v1(tree_record)?;
    let detail: StoredWorkflowExecutionNodeDetailRecordV1 = serde_json::from_str(detail_record)
        .map_err(|error| {
            format!("failed to decode Workflow execution node detail record: {error}")
        })?;
    if detail.schema != NODE_DETAIL_SCHEMA {
        return Err("unsupported Workflow execution node detail schema".to_string());
    }
    node.display_command = detail.display_command;
    node.command_result = detail.command_result.map(|result| WorkspaceCommandResult {
        exit_code: result.exit_code,
        duration: result.duration,
        stdout: result.stdout,
        stderr: result.stderr,
    });
    Ok(node)
}

fn encode_node_tree(node: &WorkspaceTreeNode) -> StoredWorkflowExecutionNodeTreeV1 {
    StoredWorkflowExecutionNodeTreeV1 {
        id: node.id.clone(),
        parent_id: node.parent_id.clone(),
        sibling_order: node.sibling_order,
        kind: node_kind_label(node.kind).to_string(),
        title: node.title.clone(),
        status: node_status_label(node.status).to_string(),
        error_reason: node.error_reason.clone(),
        updated_at_bits: node.updated_at_bits,
        execution_id: node.execution_id.clone(),
        node_execution_id: node.node_execution_id.clone(),
        node_name: node.node_name.clone(),
        attempt: node.attempt,
        completion_signals: completion_signal_state_label(node.completion_signals).to_string(),
        has_artifact: node.has_artifact,
        session_id: node.session_id.clone(),
        can_approve: node.can_approve,
        can_retry: node.can_retry,
        can_close: node.can_close,
        can_stop: node.can_stop,
        can_resume: node.can_resume,
        recovery_owner_reason: node.recovery_owner_reason.clone(),
        resume_unavailable_reason: node.resume_unavailable_reason.clone(),
        can_abort: node.can_abort,
        can_archive: node.can_archive,
        dynamic_fanout: node.dynamic_fanout,
    }
}

fn decode_node_tree(node: StoredWorkflowExecutionNodeTreeV1) -> Result<WorkspaceTreeNode, String> {
    Ok(WorkspaceTreeNode {
        id: node.id,
        parent_id: node.parent_id,
        sibling_order: node.sibling_order,
        kind: parse_node_kind(&node.kind)?,
        title: node.title,
        status: parse_node_status(&node.status)?,
        error_reason: node.error_reason,
        updated_at_bits: node.updated_at_bits,
        execution_id: node.execution_id,
        node_execution_id: node.node_execution_id,
        node_name: node.node_name,
        attempt: node.attempt,
        completion_signals: parse_completion_signal_state(&node.completion_signals)?,
        has_artifact: node.has_artifact,
        session_id: node.session_id,
        can_approve: node.can_approve,
        can_retry: node.can_retry,
        can_close: node.can_close,
        can_stop: node.can_stop,
        can_resume: node.can_resume,
        recovery_owner_reason: node.recovery_owner_reason,
        resume_unavailable_reason: node.resume_unavailable_reason,
        can_abort: node.can_abort,
        can_archive: node.can_archive,
        display_command: None,
        command_result: None,
        dynamic_fanout: node.dynamic_fanout,
    })
}

pub(crate) fn encode_workflow_execution_record_v1(
    execution: &WorkflowExecutionMetadataRecord,
) -> Result<String, String> {
    serde_json::to_string(&StoredExecutionSummaryV1 {
        schema: EXECUTION_RECORD_SCHEMA.to_string(),
        execution_id: execution.execution_id.clone(),
        workflow_name: execution.workflow_name.clone(),
        status: execution.status.as_str().to_string(),
        worktree_path: execution.worktree_path.clone(),
        current_node: execution.current_node.clone(),
        created_from: execution.created_from.as_public_value().to_string(),
        started_at_bits: execution.started_at_bits,
        updated_at_bits: execution.updated_at_bits,
        completed_at_bits: execution.completed_at_bits,
        error_reason: execution.error_reason.clone(),
        interruption_reason: execution
            .interruption_reason
            .map(ExecutionInterruptionReason::as_str)
            .map(str::to_string),
        resume_from_node: execution.resume_from_node.clone(),
        input_tokens: execution.total_token_usage.input_tokens,
        output_tokens: execution.total_token_usage.output_tokens,
    })
    .map_err(|error| format!("failed to encode Workspace execution summary: {error}"))
}

pub(crate) fn decode_workflow_execution_record_v1(
    raw: &str,
) -> Result<WorkflowExecutionMetadataRecord, String> {
    let stored: StoredExecutionSummaryV1 = serde_json::from_str(raw)
        .map_err(|error| format!("failed to decode Workspace execution summary: {error}"))?;
    if stored.schema != EXECUTION_RECORD_SCHEMA {
        return Err("unsupported Workflow execution record schema".to_string());
    }
    Ok(WorkflowExecutionMetadataRecord {
        execution_id: stored.execution_id,
        workflow_name: stored.workflow_name,
        status: parse_execution_status(&stored.status)?,
        worktree_path: stored.worktree_path,
        current_node: stored.current_node,
        created_from: ExecutionOrigin::from_public_value(&stored.created_from)
            .map_err(|_| "invalid Workspace execution origin".to_string())?,
        started_at_bits: stored.started_at_bits,
        updated_at_bits: stored.updated_at_bits,
        completed_at_bits: stored.completed_at_bits,
        error_reason: stored.error_reason,
        interruption_reason: stored
            .interruption_reason
            .as_deref()
            .map(|reason| {
                ExecutionInterruptionReason::from_reason(reason)
                    .ok_or_else(|| "invalid Workspace interruption reason".to_string())
            })
            .transpose()?,
        resume_from_node: stored.resume_from_node,
        total_token_usage: TokenUsage {
            input_tokens: stored.input_tokens,
            output_tokens: stored.output_tokens,
        },
    })
}

fn parse_execution_status(value: &str) -> Result<ExecutionStatus, String> {
    match value {
        "running" => Ok(ExecutionStatus::Running),
        #[cfg(test)]
        "waiting_approval" => Ok(ExecutionStatus::WaitingApproval),
        "completed" => Ok(ExecutionStatus::Completed),
        "aborted" => Ok(ExecutionStatus::Aborted),
        #[cfg(test)]
        "interrupted" => Ok(ExecutionStatus::Interrupted),
        _ => Err("invalid Workspace execution status".to_string()),
    }
}

fn node_kind_label(value: WorkspaceNodeKind) -> &'static str {
    match value {
        WorkspaceNodeKind::Workflow => "workflow",
        WorkspaceNodeKind::Fanout => "fanout",
        WorkspaceNodeKind::Sequence => "sequence",
        WorkspaceNodeKind::WorkflowSession => "workflow_session",
        WorkspaceNodeKind::WorkflowCommand => "workflow_command",
    }
}

fn parse_node_kind(value: &str) -> Result<WorkspaceNodeKind, String> {
    match value {
        "workflow" => Ok(WorkspaceNodeKind::Workflow),
        "fanout" => Ok(WorkspaceNodeKind::Fanout),
        "sequence" => Ok(WorkspaceNodeKind::Sequence),
        "workflow_session" => Ok(WorkspaceNodeKind::WorkflowSession),
        "workflow_command" => Ok(WorkspaceNodeKind::WorkflowCommand),
        _ => Err("invalid Workspace node kind".to_string()),
    }
}

fn node_status_label(value: WorkspaceNodeStatus) -> &'static str {
    match value {
        WorkspaceNodeStatus::Running => "running",
        WorkspaceNodeStatus::Paused => "paused",
        WorkspaceNodeStatus::Failed => "failed",
        WorkspaceNodeStatus::Error => "error",
        WorkspaceNodeStatus::Waiting => "waiting",
        WorkspaceNodeStatus::Interrupted => "interrupted",
        WorkspaceNodeStatus::Aborted => "aborted",
        WorkspaceNodeStatus::Completed => "completed",
    }
}

fn parse_node_status(value: &str) -> Result<WorkspaceNodeStatus, String> {
    match value {
        "running" => Ok(WorkspaceNodeStatus::Running),
        "paused" => Ok(WorkspaceNodeStatus::Paused),
        "failed" => Ok(WorkspaceNodeStatus::Failed),
        "error" => Ok(WorkspaceNodeStatus::Error),
        "waiting" => Ok(WorkspaceNodeStatus::Waiting),
        "interrupted" => Ok(WorkspaceNodeStatus::Interrupted),
        "aborted" => Ok(WorkspaceNodeStatus::Aborted),
        "completed" => Ok(WorkspaceNodeStatus::Completed),
        _ => Err("invalid Workspace node status".to_string()),
    }
}

fn completion_signal_state_label(value: NodeCompletionSignalState) -> &'static str {
    match value {
        NodeCompletionSignalState::Pending => "pending",
        NodeCompletionSignalState::SubmitReceived => "submit_received",
        NodeCompletionSignalState::StopReceived => "stop_received",
        NodeCompletionSignalState::Ready => "ready",
    }
}

fn parse_completion_signal_state(value: &str) -> Result<NodeCompletionSignalState, String> {
    match value {
        "" | "pending" => Ok(NodeCompletionSignalState::Pending),
        "submit_received" => Ok(NodeCompletionSignalState::SubmitReceived),
        "stop_received" => Ok(NodeCompletionSignalState::StopReceived),
        "ready" => Ok(NodeCompletionSignalState::Ready),
        _ => Err("invalid Workflow node completion signal state".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn execution() -> WorkflowExecutionMetadataRecord {
        WorkflowExecutionMetadataRecord {
            execution_id: "execution".to_string(),
            workflow_name: "review".to_string(),
            status: ExecutionStatus::Interrupted,
            worktree_path: "/repo".to_string(),
            current_node: Some("test".to_string()),
            created_from: ExecutionOrigin::DesktopUi,
            started_at_bits: 1.0f64.to_bits(),
            updated_at_bits: 2.0f64.to_bits(),
            completed_at_bits: None,
            error_reason: None,
            interruption_reason: Some(ExecutionInterruptionReason::Crash),
            resume_from_node: Some("test".to_string()),
            total_token_usage: TokenUsage {
                input_tokens: 10,
                output_tokens: 5,
            },
        }
    }

    fn node() -> WorkspaceTreeNode {
        WorkspaceTreeNode {
            id: "node".to_string(),
            parent_id: Some("execution".to_string()),
            sibling_order: 1,
            kind: WorkspaceNodeKind::WorkflowCommand,
            title: "Run tests".to_string(),
            status: WorkspaceNodeStatus::Completed,
            error_reason: None,
            updated_at_bits: 5.0f64.to_bits(),
            execution_id: Some("execution".to_string()),
            node_execution_id: Some("node-execution".to_string()),
            node_name: Some("test".to_string()),
            attempt: Some(1),
            completion_signals: Default::default(),
            has_artifact: false,
            session_id: None,
            can_approve: false,
            can_retry: false,
            can_close: false,
            can_stop: false,
            can_resume: false,
            recovery_owner_reason: None,
            resume_unavailable_reason: None,
            can_abort: false,
            can_archive: false,
            display_command: Some("cargo test".to_string()),
            command_result: Some(WorkspaceCommandResult {
                exit_code: 0,
                duration: 12,
                stdout: "ok".to_string(),
                stderr: String::new(),
            }),
            dynamic_fanout: false,
        }
    }

    #[test]
    fn execution_and_node_records_round_trip() {
        let execution = execution();
        assert_eq!(
            decode_workflow_execution_record_v1(
                &encode_workflow_execution_record_v1(&execution).unwrap()
            )
            .unwrap(),
            execution
        );

        let node = node();
        let (tree, detail) = encode_workflow_execution_node_v1(&node).unwrap();
        assert_eq!(
            decode_workflow_execution_node_detail_v1(&tree, &detail).unwrap(),
            node
        );
    }

    #[test]
    fn indexed_rows_own_all_derived_insert_values_and_validation() {
        let mut active = execution();
        active.status = ExecutionStatus::Running;
        let active_row = indexed_execution_row(&active).unwrap();
        assert_eq!(active_row.status, "running");
        assert_eq!(active_row.list_kind, "active");
        assert_eq!(active_row.sort_at_bits, active.updated_at_bits as i64);
        assert_eq!(
            decode_workflow_execution_record_v1(&active_row.record).unwrap(),
            active
        );

        let mut terminal = execution();
        terminal.status = ExecutionStatus::Completed;
        terminal.completed_at_bits = Some(9.0f64.to_bits());
        let terminal_row = indexed_execution_row(&terminal).unwrap();
        assert_eq!(terminal_row.list_kind, "terminal");
        assert_eq!(
            terminal_row.sort_at_bits,
            terminal.completed_at_bits.unwrap() as i64
        );

        let node_row = indexed_execution_node_row("execution", &node()).unwrap();
        assert_eq!(node_row.execution_id, "execution");
        assert_eq!(node_row.sibling_order, 1);

        terminal.worktree_path = "/repo/".to_string();
        assert!(indexed_execution_row(&terminal).is_err());
        assert!(indexed_execution_node_row("other-execution", &node()).is_err());
    }

    #[test]
    fn codecs_reject_schema_tag_mismatch() {
        let (tree, _) = encode_workflow_execution_node_v1(&node()).unwrap();
        let mut value: serde_json::Value = serde_json::from_str(&tree).unwrap();
        value["schema"] = serde_json::Value::String("future_schema".to_string());
        assert!(decode_workflow_execution_node_tree_v1(&value.to_string()).is_err());
    }

    #[test]
    fn tree_record_does_not_retain_command_detail_payloads() {
        let (tree, detail) = encode_workflow_execution_node_v1(&node()).unwrap();
        assert!(!tree.contains("display_command"));
        assert!(!tree.contains("command_result"));
        assert!(!tree.contains("cargo test"));
        assert!(!tree.contains("\"stdout\""));
        let detail: serde_json::Value = serde_json::from_str(&detail).unwrap();
        assert_eq!(detail["display_command"], "cargo test");
        assert_eq!(detail["command_result"]["stdout"], "ok");
    }
}
