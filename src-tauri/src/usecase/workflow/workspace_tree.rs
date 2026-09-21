use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use super::workspace_node_command::{
    WorkspaceNodeActionResolver, WorkspaceNodeApprovalTarget, WorkspaceNodeRetryTarget,
    WorkspaceSessionNodeRenameTarget,
};
use super::WorkflowUsecase;
use crate::domain::workflow::{WorkflowError, WorkflowExecutionId};
use crate::usecase::agent_session::AgentSessionItemDto;

fn unix_timestamp_seconds() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceTreeSnapshotDto {
    pub nodes: Vec<WorkspaceTreeItemDto>,
    pub archived_sessions: Vec<AgentSessionItemDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferred_node_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceTreeSelectionSnapshotDto {
    pub snapshot: WorkspaceTreeSnapshotDto,
    pub reconciliation: WorkspaceSelectionReconciliationDto,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceSelectionReconciliationDto {
    pub selection_in_snapshot: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub(crate) enum WorkspaceTreeItemDto {
    Node(WorkspaceNodeDto),
    Sequence(WorkspaceSequenceDto),
    Fanout(WorkspaceFanoutDto),
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceNodeDto {
    pub process_presence: &'static str,
    pub id: String,
    pub title: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_reason: Option<String>,
    pub content_kind: &'static str,
    pub capabilities: WorkspaceNodeCapabilitiesDto,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_capabilities: Option<WorkspaceWorkflowCapabilitiesDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_capabilities: Option<WorkspaceSessionCapabilitiesDto>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<WorkspaceTreeItemDto>,
    #[serde(serialize_with = "serialize_past_attempts")]
    pub past_attempts: Vec<WorkspaceNodeDto>,
    pub past_attempts_collapsed: bool,
    pub updated_at: f64,
}

fn serialize_past_attempts<S: serde::Serializer>(
    attempts: &[WorkspaceNodeDto],
    serializer: S,
) -> Result<S::Ok, S::Error> {
    #[derive(Serialize)]
    #[serde(tag = "kind", rename_all = "camelCase")]
    enum PastAttempt<'a> {
        Node(&'a WorkspaceNodeDto),
    }
    serializer.collect_seq(attempts.iter().map(PastAttempt::Node))
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceSessionCapabilitiesDto {
    pub session_ref: String,
    pub can_archive: bool,
    pub can_delete: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceNodeCapabilitiesDto {
    pub can_rename: bool,
    pub can_approve: bool,
    pub can_retry: bool,
    pub can_resume_session: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceSequenceDto {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree: Option<NodeWorktreeDto>,
    pub id: String,
    pub title: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_capabilities: Option<WorkspaceWorkflowCapabilitiesDto>,
    pub children: Vec<WorkspaceTreeItemDto>,
    pub updated_at: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceWorkflowCapabilitiesDto {
    pub can_abort: bool,
    pub can_archive: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceFanoutDto {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree: Option<NodeWorktreeDto>,
    pub id: String,
    pub title: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_capabilities: Option<WorkspaceWorkflowCapabilitiesDto>,
    pub children: Vec<WorkspaceTreeItemDto>,
    pub updated_at: f64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct NodeWorktreeDto {
    pub branch: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceNodeDetailDto {
    pub process_presence: &'static str,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree: Option<NodeWorktreeDto>,
    pub id: String,
    pub title: String,
    pub status: String,
    pub status_classification: String,
    pub submit_received: bool,
    pub stop_received: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waiting_for: Option<&'static str>,
    pub has_artifact: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery_reason: Option<String>,
    pub capabilities: WorkspaceNodeCapabilitiesDto,
    pub updated_at: f64,
    pub content: WorkspaceNodeContentDto,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub(crate) enum WorkspaceNodeContentDto {
    Session(WorkspaceSessionNodeContentDto),
    Command(WorkspaceCommandNodeContentDto),
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceSessionNodeContentDto {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceCommandNodeContentDto {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<WorkspaceCommandResultDto>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceCommandResultDto {
    pub exit_code: i64,
    /// Milliseconds, matching the command Artifact reserved field.
    pub duration: u64,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceWorkflowHistoryItemDto {
    pub execution_id: String,
    pub worktree_path: String,
    pub title: String,
    pub status: String,
    pub updated_at: f64,
    pub archived_at: f64,
    pub archive_reason: String,
}

impl WorkflowUsecase {
    pub(crate) fn list_workspace_tree_nodes(
        &self,
        worktree_path: &str,
    ) -> Result<WorkspaceTreeSnapshotDto, WorkflowError> {
        let workspace = crate::domain::workspace_tree::WorkspaceIdentity::new(
            self.resolve_worktree_path(worktree_path)?,
        );
        self.workspace_query.workspace_tree(&workspace)
    }

    pub(crate) fn list_workspace_workflow_history(
        &self,
        worktree_path: &str,
    ) -> Result<Vec<WorkspaceWorkflowHistoryItemDto>, WorkflowError> {
        let workspace = crate::domain::workspace_tree::WorkspaceIdentity::new(
            self.resolve_worktree_path(worktree_path)?,
        );
        self.workspace_query.workflow_history(&workspace)
    }

    pub(crate) fn get_workspace_node_detail(
        &self,
        worktree_path: &str,
        node_id: &str,
    ) -> Result<Option<WorkspaceNodeDetailDto>, WorkflowError> {
        let workspace = crate::domain::workspace_tree::WorkspaceIdentity::new(
            self.resolve_worktree_path(worktree_path)?,
        );
        self.workspace_query.node_detail(&workspace, node_id)
    }

    pub(crate) fn get_workspace_session_node_id(
        &self,
        worktree_path: &str,
        session_id: &str,
    ) -> Result<Option<String>, WorkflowError> {
        let workspace = crate::domain::workspace_tree::WorkspaceIdentity::new(
            self.resolve_worktree_path(worktree_path)?,
        );
        self.workspace_query.session_node_id(&workspace, session_id)
    }

    pub(crate) fn get_workspace_tree_selection_reconciliation(
        &self,
        worktree_path: &str,
        selected_node_id: &str,
    ) -> Result<WorkspaceTreeSelectionSnapshotDto, WorkflowError> {
        let workspace = crate::domain::workspace_tree::WorkspaceIdentity::new(
            self.resolve_worktree_path(worktree_path)?,
        );
        self.workspace_query
            .workspace_tree(&workspace)
            .map(|snapshot| reconcile_workspace_tree_selection(snapshot, selected_node_id))
    }

    pub(crate) fn archive_workspace_workflow_execution(
        &self,
        worktree_path: &str,
        execution_id: &str,
    ) -> Result<(), WorkflowError> {
        let execution_id = WorkflowExecutionId::new(execution_id.to_string())?;
        if self
            .authorize_execution_summary_for_worktree(execution_id.as_str(), worktree_path)?
            .is_none()
        {
            return Err(WorkflowError::external(format!(
                "Workflow execution not found: {execution_id}"
            )));
        }
        self.execution_archives
            .archive_manual(&execution_id, unix_timestamp_seconds())
    }

    pub(crate) fn restore_workspace_workflow_execution(
        &self,
        worktree_path: &str,
        execution_id: &str,
    ) -> Result<(), WorkflowError> {
        let execution_id = WorkflowExecutionId::new(execution_id.to_string())?;
        if self
            .authorize_execution_summary_for_worktree(execution_id.as_str(), worktree_path)?
            .is_none()
        {
            return Err(WorkflowError::external(format!(
                "Workflow execution not found: {execution_id}"
            )));
        }
        self.execution_archives
            .restore_manual(&execution_id, unix_timestamp_seconds())
    }
}

impl WorkspaceNodeActionResolver for WorkflowUsecase {
    fn resolve_approval_target(
        &self,
        worktree_path: &str,
        node_id: &str,
    ) -> Result<WorkspaceNodeApprovalTarget, WorkflowError> {
        let workspace = crate::domain::workspace_tree::WorkspaceIdentity::new(
            self.resolve_worktree_path(worktree_path)?,
        );
        let node = self
            .workspace_nodes
            .load_node(&workspace, node_id)
            .map_err(|error| WorkflowError::external(error.to_string()))?
            .ok_or_else(|| {
                WorkflowError::NotFound(format!("Workspace node not found: {node_id}"))
            })?;
        let (Some(execution_id), Some(node_execution_id), Some(node_name)) =
            (node.execution_id, node.node_execution_id, node.node_name)
        else {
            return Err(WorkflowError::invalid_state(
                "Workspace node is not a Workflow Node",
            ));
        };
        Ok(WorkspaceNodeApprovalTarget {
            execution_id,
            node_name,
            node_execution_id,
        })
    }

    fn resolve_retry_target(
        &self,
        worktree_path: &str,
        node_id: &str,
    ) -> Result<WorkspaceNodeRetryTarget, WorkflowError> {
        let workspace = crate::domain::workspace_tree::WorkspaceIdentity::new(
            self.resolve_worktree_path(worktree_path)?,
        );
        let node = self
            .workspace_nodes
            .load_node(&workspace, node_id)
            .map_err(|error| WorkflowError::external(error.to_string()))?
            .ok_or_else(|| {
                WorkflowError::NotFound(format!("Workspace node not found: {node_id}"))
            })?;
        let (Some(execution_id), Some(node_execution_id)) =
            (node.execution_id, node.node_execution_id)
        else {
            return Err(WorkflowError::invalid_state(
                "Workspace node is not a Workflow Node",
            ));
        };
        Ok(WorkspaceNodeRetryTarget {
            execution_id,
            node_execution_id,
        })
    }

    fn resolve_session_resume_target(
        &self,
        worktree_path: &str,
        node_id: &str,
    ) -> Result<super::command::ResumeSessionNodeCommand, WorkflowError> {
        let workspace = crate::domain::workspace_tree::WorkspaceIdentity::new(
            self.resolve_worktree_path(worktree_path)?,
        );
        let node = self
            .workspace_nodes
            .load_node(&workspace, node_id)
            .map_err(|error| WorkflowError::external(error.to_string()))?
            .ok_or_else(|| {
                WorkflowError::NotFound(format!("Workspace node not found: {node_id}"))
            })?;
        let (Some(execution_id), Some(node_execution_id)) =
            (node.execution_id, node.node_execution_id)
        else {
            return Err(WorkflowError::invalid_state(
                "Workspace node is not a Workflow Node",
            ));
        };
        Ok(super::command::ResumeSessionNodeCommand {
            execution_id,
            node_execution_id,
        })
    }

    fn resolve_session_rename_target(
        &self,
        worktree_path: &str,
        node_id: &str,
    ) -> Result<WorkspaceSessionNodeRenameTarget, WorkflowError> {
        let workspace = crate::domain::workspace_tree::WorkspaceIdentity::new(
            self.resolve_worktree_path(worktree_path)?,
        );
        let node = self
            .workspace_nodes
            .load_node(&workspace, node_id)
            .map_err(|error| WorkflowError::external(error.to_string()))?
            .ok_or_else(|| {
                WorkflowError::NotFound(format!("Workspace node not found: {node_id}"))
            })?;
        if !node.can_rename {
            return Err(WorkflowError::invalid_state(
                "Workspace node cannot be renamed",
            ));
        }
        let agent_session_id = node
            .session_id
            .ok_or_else(|| WorkflowError::invalid_state("Workspace Session Node is not bound"))?;
        Ok(WorkspaceSessionNodeRenameTarget { agent_session_id })
    }
}

fn reconcile_workspace_tree_selection(
    snapshot: WorkspaceTreeSnapshotDto,
    selected_node_id: &str,
) -> WorkspaceTreeSelectionSnapshotDto {
    let selection_in_snapshot = workspace_tree_contains_node(&snapshot.nodes, selected_node_id);
    WorkspaceTreeSelectionSnapshotDto {
        snapshot,
        reconciliation: WorkspaceSelectionReconciliationDto {
            selection_in_snapshot,
        },
    }
}

fn workspace_tree_contains_node(nodes: &[WorkspaceTreeItemDto], node_id: &str) -> bool {
    nodes.iter().any(|item| match item {
        WorkspaceTreeItemDto::Node(node) => {
            node.id == node_id
                || workspace_tree_contains_node(&node.children, node_id)
                || node.past_attempts.iter().any(|past| {
                    past.id == node_id || workspace_tree_contains_node(&past.children, node_id)
                })
        }
        WorkspaceTreeItemDto::Sequence(sequence) => {
            workspace_tree_contains_node(&sequence.children, node_id)
        }
        WorkspaceTreeItemDto::Fanout(fanout) => {
            workspace_tree_contains_node(&fanout.children, node_id)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nested_snapshot() -> WorkspaceTreeSnapshotDto {
        WorkspaceTreeSnapshotDto {
            nodes: vec![WorkspaceTreeItemDto::Sequence(WorkspaceSequenceDto {
                worktree: None,
                id: "workflow".to_string(),
                title: "main".to_string(),
                status: "active".to_string(),
                workflow_capabilities: Some(WorkspaceWorkflowCapabilitiesDto {
                    can_abort: true,
                    can_archive: false,
                }),
                children: vec![WorkspaceTreeItemDto::Fanout(WorkspaceFanoutDto {
                    worktree: None,
                    id: "fanout".to_string(),
                    title: "Fanout".to_string(),
                    status: "active".to_string(),
                    workflow_capabilities: None,
                    children: vec![WorkspaceTreeItemDto::Node(WorkspaceNodeDto {
                        process_presence: "unknown",
                        id: "selected-node".to_string(),
                        title: "Child".to_string(),
                        status: "active".to_string(),
                        error_reason: None,
                        content_kind: "session",
                        capabilities: WorkspaceNodeCapabilitiesDto {
                            can_resume_session: false,
                            can_rename: false,
                            can_approve: false,
                            can_retry: false,
                        },
                        workflow_capabilities: None,
                        session_capabilities: None,
                        children: Vec::new(),
                        past_attempts: Vec::new(),
                        past_attempts_collapsed: false,
                        updated_at: 1.0,
                    })],
                    updated_at: 1.0,
                })],
                updated_at: 1.0,
            })],
            archived_sessions: Vec::new(),
            preferred_node_id: Some("selected-node".to_string()),
        }
    }

    #[test]
    fn selection_reconciliation_preserves_a_nested_node_in_the_new_snapshot() {
        let reconciliation = reconcile_workspace_tree_selection(nested_snapshot(), "selected-node");

        assert!(reconciliation.reconciliation.selection_in_snapshot);
        assert_eq!(
            reconciliation.snapshot.preferred_node_id.as_deref(),
            Some("selected-node")
        );
    }

    #[test]
    fn selection_reconciliation_reports_a_removed_node_without_replacing_selection() {
        let reconciliation = reconcile_workspace_tree_selection(nested_snapshot(), "removed-node");

        assert!(!reconciliation.reconciliation.selection_in_snapshot);
        assert_eq!(
            reconciliation.snapshot.preferred_node_id.as_deref(),
            Some("selected-node")
        );
    }

    #[test]
    fn test_選択整合契約_返却snapshotの全行が5分類だけを持つ() {
        fn assert_classifications(items: &[WorkspaceTreeItemDto]) {
            for item in items {
                let (status, children) = match item {
                    WorkspaceTreeItemDto::Node(node) => (node.status.as_str(), None),
                    WorkspaceTreeItemDto::Sequence(sequence) => {
                        (sequence.status.as_str(), Some(sequence.children.as_slice()))
                    }
                    WorkspaceTreeItemDto::Fanout(fanout) => {
                        (fanout.status.as_str(), Some(fanout.children.as_slice()))
                    }
                };
                assert!(["active", "attention", "failure", "idle", "unbound"].contains(&status));
                assert_ne!(status, "interrupted");
                if let Some(children) = children {
                    assert_classifications(children);
                }
            }
        }

        // Given
        let snapshot = nested_snapshot();

        // When
        let reconciliation = reconcile_workspace_tree_selection(snapshot, "selected-node");

        // Then
        assert_classifications(&reconciliation.snapshot.nodes);
    }
    #[test]
    fn test_delegate_子を選択した状態はsnapshotの照合で維持される() {
        // Given
        let snapshot = nested_snapshot();
        let WorkspaceTreeItemDto::Sequence(root) = &snapshot.nodes[0] else {
            panic!()
        };
        let WorkspaceTreeItemDto::Fanout(fanout) = &root.children[0] else {
            panic!()
        };
        let WorkspaceTreeItemDto::Node(child) = &fanout.children[0] else {
            panic!()
        };
        let mut parent = child.clone();
        parent.id = "parent".into();
        parent.children = vec![WorkspaceTreeItemDto::Node(child.clone())];
        // When / Then
        assert!(workspace_tree_contains_node(
            &[WorkspaceTreeItemDto::Node(parent)],
            &child.id
        ));
    }
    #[test]
    fn test_選択整合_過去attemptとdelegate部分木の存在を新snapshotで判定する() {
        // Given
        let mut snapshot = nested_snapshot();
        let WorkspaceTreeItemDto::Sequence(root) = &mut snapshot.nodes[0] else {
            panic!()
        };
        let WorkspaceTreeItemDto::Fanout(fanout) = &mut root.children[0] else {
            panic!()
        };
        let WorkspaceTreeItemDto::Node(current) = &mut fanout.children[0] else {
            panic!()
        };
        let mut past = current.clone();
        past.id = "past-parent".into();
        let mut leaf = current.clone();
        leaf.id = "past-judge".into();
        past.children = vec![WorkspaceTreeItemDto::Sequence(WorkspaceSequenceDto {
            id: "past-checks".into(),
            title: "checks".into(),
            status: "idle".into(),
            worktree: None,
            workflow_capabilities: None,
            children: vec![WorkspaceTreeItemDto::Node(leaf)],
            updated_at: 1.0,
        })];
        current.past_attempts = vec![past];
        // When / Then
        for (id, present) in [
            ("past-parent", true),
            ("past-judge", true),
            ("removed-judge", false),
        ] {
            let reconciled = reconcile_workspace_tree_selection(snapshot.clone(), id);
            assert_eq!(
                reconciled.reconciliation.selection_in_snapshot, present,
                "{id}"
            );
            assert_eq!(reconciled.snapshot, snapshot);
        }
    }
}
