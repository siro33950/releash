use super::{WorkspaceNodeKind, WorkspaceNodeStatusClassification, WorkspaceTree};
use crate::domain::provider_lifecycle::ProviderKind;
use crate::domain::workflow::ExecutionTreeLaunch;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeExecutionSummary {
    pub id: String,
    pub title: String,
    pub is_workflow: bool,
    pub provider: Option<ProviderKind>,
    pub status: WorkspaceNodeStatusClassification,
    pub node_count: usize,
    pub session_states: Vec<WorkspaceNodeStatusClassification>,
}

impl WorkspaceTree {
    pub fn card_executions(&self) -> Vec<WorktreeExecutionSummary> {
        let mut summaries = self
            .executions()
            .iter()
            .filter(|execution| execution.archive.is_none())
            .filter_map(|execution| {
                let root = self.nodes().iter().find(|node| {
                    node.kind == WorkspaceNodeKind::Workflow
                        && node.execution_id.as_deref() == Some(execution.execution_id.as_str())
                })?;
                let nodes = self
                    .nodes()
                    .iter()
                    .filter(|node| {
                        node.execution_id.as_deref() == Some(execution.execution_id.as_str())
                            && !node.is_retry_history
                            && node.is_leaf()
                    })
                    .collect::<Vec<_>>();
                let is_workflow = execution.launched_as == ExecutionTreeLaunch::Workflow;
                Some(WorktreeExecutionSummary {
                    id: execution.execution_id.clone(),
                    title: if is_workflow {
                        execution.workflow_name.clone()
                    } else {
                        nodes.first().map_or_else(
                            || execution.workflow_name.clone(),
                            |node| node.title.clone(),
                        )
                    },
                    is_workflow,
                    provider: execution.session.as_ref().map(|session| session.provider()),
                    status: root.status_classification,
                    node_count: nodes.len(),
                    session_states: nodes
                        .into_iter()
                        .filter(|node| node.kind == WorkspaceNodeKind::WorkflowSession)
                        .map(|node| node.status_classification)
                        .collect(),
                })
            })
            .collect::<Vec<_>>();
        summaries.sort_by_key(|summary| summary.is_workflow);
        summaries
    }

    pub fn card_status(&self) -> Option<WorkspaceNodeStatusClassification> {
        self.executions()
            .iter()
            .filter(|execution| execution.archive.is_none())
            .filter_map(|execution| {
                self.nodes().iter().find(|node| {
                    node.kind == WorkspaceNodeKind::Workflow
                        && node.execution_id.as_deref() == Some(execution.execution_id.as_str())
                })
            })
            .map(|node| node.status_classification)
            .reduce(WorkspaceNodeStatusClassification::most_severe)
    }
}

#[cfg(test)]
#[path = "card_test.rs"]
mod card_tests;
