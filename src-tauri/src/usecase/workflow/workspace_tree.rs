use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use super::workspace_node_command::{
    CloseWorkspaceNodeError, WorkspaceNodeActionResolver, WorkspaceNodeApprovalTarget,
    WorkspaceNodeCloseTarget, WorkspaceNodeRetryTarget,
};
use super::WorkflowUsecase;
use crate::domain::workflow::{WorkflowError, WorkflowExecutionId};

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
    Workflow(WorkspaceWorkflowDto),
    Fanout(WorkspaceFanoutDto),
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceNodeDto {
    pub id: String,
    pub title: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_reason: Option<String>,
    pub content_kind: &'static str,
    pub capabilities: WorkspaceNodeCapabilitiesDto,
    pub updated_at: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceNodeCapabilitiesDto {
    pub can_approve: bool,
    pub can_retry: bool,
    pub can_close: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceWorkflowDto {
    pub id: String,
    pub title: String,
    pub status: String,
    pub capabilities: WorkspaceWorkflowCapabilitiesDto,
    pub children: Vec<WorkspaceTreeItemDto>,
    pub updated_at: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceWorkflowCapabilitiesDto {
    pub can_stop: bool,
    pub can_resume: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_unavailable_reason: Option<String>,
    pub can_abort: bool,
    pub can_archive: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceFanoutDto {
    pub id: String,
    pub title: String,
    pub status: String,
    pub children: Vec<WorkspaceTreeItemDto>,
    pub updated_at: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceNodeDetailDto {
    pub id: String,
    pub title: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt: Option<u32>,
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
    ProviderAgentSession(WorkspaceSessionNodeContentDto),
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
    fn resolve_close_target(
        &self,
        worktree_path: &str,
        node_id: &str,
    ) -> Result<WorkspaceNodeCloseTarget, CloseWorkspaceNodeError> {
        let workspace = crate::domain::workspace_tree::WorkspaceIdentity::new(
            self.resolve_worktree_path(worktree_path)
                .map_err(CloseWorkspaceNodeError::Resolution)?,
        );
        let node = self
            .workspace_nodes
            .load_node(&workspace, node_id)
            .map_err(|error| {
                CloseWorkspaceNodeError::Resolution(WorkflowError::external(error.to_string()))
            })?
            .ok_or_else(|| CloseWorkspaceNodeError::NodeNotFound {
                node_id: node_id.to_string(),
            })?;
        match (node.can_close, node.session_id) {
            (true, Some(session_id)) => Ok(WorkspaceNodeCloseTarget { session_id }),
            _ => Err(CloseWorkspaceNodeError::CloseNotSupported {
                node_id: node_id.to_string(),
            }),
        }
    }

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
        WorkspaceTreeItemDto::Node(node) => node.id == node_id,
        WorkspaceTreeItemDto::Workflow(workflow) => {
            workspace_tree_contains_node(&workflow.children, node_id)
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
            nodes: vec![WorkspaceTreeItemDto::Workflow(WorkspaceWorkflowDto {
                id: "workflow".to_string(),
                title: "Workflow".to_string(),
                status: "running".to_string(),
                capabilities: WorkspaceWorkflowCapabilitiesDto {
                    can_stop: true,
                    can_resume: false,
                    resume_unavailable_reason: None,
                    can_abort: true,
                    can_archive: false,
                },
                children: vec![WorkspaceTreeItemDto::Fanout(WorkspaceFanoutDto {
                    id: "fanout".to_string(),
                    title: "Fanout".to_string(),
                    status: "running".to_string(),
                    children: vec![WorkspaceTreeItemDto::Node(WorkspaceNodeDto {
                        id: "selected-node".to_string(),
                        title: "Child".to_string(),
                        status: "running".to_string(),
                        error_reason: None,
                        content_kind: "session",
                        capabilities: WorkspaceNodeCapabilitiesDto {
                            can_approve: false,
                            can_retry: false,
                            can_close: false,
                        },
                        updated_at: 1.0,
                    })],
                    updated_at: 1.0,
                })],
                updated_at: 1.0,
            })],
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
}
