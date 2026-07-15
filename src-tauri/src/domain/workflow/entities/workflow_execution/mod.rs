//! Test-only workflow execution aggregate plus shared execution projection helpers.
//!
//! The stateful `WorkflowExecution` aggregate in this module is retained for
//! domain unit tests. Production execution state is owned by the workflow
//! gateway runtime state; pure validation lives in workflow services.

#[cfg(test)]
use std::collections::HashMap;

#[cfg(test)]
use crate::domain::workflow::services::{fanout, history, projection, validation};
#[cfg(test)]
use crate::domain::workflow::value_objects::{
    NodeDefinition, NodeHistoryEntry, RuntimeArtifact, RuntimeExecutionState, WorkflowDefinition,
    WorkflowExecutionId, WorkflowRuntimeSnapshot, WorkspaceWorktreePath,
};
use crate::domain::workflow::value_objects::{NodeExecutionFailureKind, TokenUsage};
use crate::domain::workflow::FailureDisposition;
#[cfg(test)]
use crate::domain::workflow::WorkflowError;

/// Test-only stateful aggregate used by domain unit tests.
#[derive(Debug, Clone, PartialEq)]
#[cfg(test)]
pub struct WorkflowExecution {
    id: WorkflowExecutionId,
    workflow: WorkflowDefinition,
    state: RuntimeExecutionState,
    current_node_index: usize,
    node_execution_counts: HashMap<String, u32>,
    node_history: Vec<NodeHistoryEntry>,
    worktree_path: WorkspaceWorktreePath,
    started_at: f64,
    updated_at: f64,
    current_session_id: Option<String>,
    current_node_token_usage: TokenUsage,
    terminal_total_token_usage: Option<TokenUsage>,
    artifacts: HashMap<String, RuntimeArtifact>,
    request: String,
    fanout_runtime: Option<FanoutRuntimeState>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FanoutRuntimeState {
    pub parent_node_name: String,
    pub children: Vec<FanoutChildRuntime>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FanoutChildRuntime {
    pub node_name: String,
    pub session_id: String,
    pub state: FanoutChildRuntimeState,
    pub result: Option<String>,
    pub artifact: Option<serde_json::Value>,
    pub contract: Option<String>,
    pub failure_kind: Option<NodeExecutionFailureKind>,
    pub failure_disposition: Option<FailureDisposition>,
    pub token_usage: TokenUsage,
    pub attempt: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FanoutChildRuntimeState {
    Running,
    Completed,
    Failed,
    Interrupted,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg(test)]
pub struct NodeCompletion {
    pub node_name: String,
    pub result: Option<String>,
    pub session_id: Option<String>,
    pub token_usage: Option<TokenUsage>,
    pub artifact: Option<serde_json::Value>,
    pub contract: Option<String>,
    pub attempt: Option<u32>,
    pub completed_at: f64,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg(test)]
pub struct FanoutChildCompletion {
    pub child_node_name: String,
    pub result: Option<String>,
    pub session_id: String,
    pub token_usage: Option<TokenUsage>,
    pub artifact: Option<serde_json::Value>,
    pub attempt: u32,
    pub completed_at: f64,
}

#[cfg(test)]
impl WorkflowExecution {
    pub fn new(
        id: WorkflowExecutionId,
        workflow: WorkflowDefinition,
        worktree_path: WorkspaceWorktreePath,
        request: Option<String>,
        started_at: f64,
    ) -> Result<Self, WorkflowError> {
        validation::validate_workflow_shape(&workflow)?;
        let first_node_name = workflow
            .nodes
            .first()
            .map(|node| node.name.clone())
            .ok_or_else(|| WorkflowError::validation("workflow has no nodes"))?;
        Ok(Self {
            id,
            workflow,
            state: RuntimeExecutionState::Running,
            current_node_index: 0,
            node_execution_counts: HashMap::from([(first_node_name, 1)]),
            node_history: Vec::new(),
            worktree_path,
            started_at,
            updated_at: started_at,
            current_session_id: None,
            current_node_token_usage: TokenUsage::default(),
            terminal_total_token_usage: None,
            artifacts: HashMap::new(),
            request: request.unwrap_or_default(),
            fanout_runtime: None,
        })
    }

    pub fn request(&self) -> &str {
        &self.request
    }

    pub fn to_snapshot(&self) -> WorkflowRuntimeSnapshot {
        let total_token_usage = self
            .terminal_total_token_usage
            .clone()
            .unwrap_or_else(|| projection::total_token_usage(&self.node_history));

        WorkflowRuntimeSnapshot {
            execution_id: self.id.to_string(),
            workflow_name: self.workflow.name.clone(),
            worktree_path: self.worktree_path.to_string(),
            created_from: crate::domain::workflow::ExecutionOrigin::DesktopUi,
            request: self.request.clone(),
            error_reason: match &self.state {
                RuntimeExecutionState::Failed { reason, .. } => Some(reason.clone()),
                _ => None,
            },
            state: self.state.clone(),
            current_node_index: self.current_node_index,
            current_node_name: self.current_node_name().unwrap_or_default().to_string(),
            current_session_id: self.current_session_id.clone(),
            node_history: self.node_history.clone(),
            node_execution_counts: self.node_execution_counts.clone(),
            workflow_definition: self.workflow.clone(),
            total_token_usage,
            artifacts: self.artifacts.clone(),
            node_executions: Vec::new(),
            started_at: self.started_at,
            updated_at: self.updated_at,
        }
    }

    pub fn record_node_completed(
        &mut self,
        completion: NodeCompletion,
    ) -> Result<(), WorkflowError> {
        self.node_index(&completion.node_name)?;
        let attempt = completion.attempt.unwrap_or_else(|| {
            self.node_execution_counts
                .get(&completion.node_name)
                .copied()
                .unwrap_or(0)
        });
        let entry = history::completed_node_history_entry(history::CompletedNodeHistoryInput {
            node_name: completion.node_name,
            completed_at: completion.completed_at,
            result: completion.result,
            session_id: completion.session_id,
            token_usage: completion.token_usage.clone(),
            artifact: completion.artifact,
            attempt,
        });
        if let Some(output) =
            history::artifact_from_completed_history_entry(&entry, completion.contract)
        {
            self.artifacts.insert(entry.node_name.clone(), output);
        }
        self.node_history.push(entry);
        self.current_node_token_usage = TokenUsage::default();
        self.updated_at = completion.completed_at;
        Ok(())
    }

    pub fn request_approval(
        &mut self,
        node_name: &str,
        timestamp: f64,
    ) -> Result<(), WorkflowError> {
        self.current_node_index = self.node_index(node_name)?;
        self.state = RuntimeExecutionState::WaitingApproval;
        self.updated_at = timestamp;
        Ok(())
    }

    pub fn abort_execution(&mut self, timestamp: f64) {
        self.state = RuntimeExecutionState::Aborted;
        if let Some(entry) = self.aborted_fanout_history_entry(timestamp) {
            self.node_history.push(entry);
        } else if let Some(entry) = self.aborted_current_history_entry(timestamp) {
            self.node_history.push(entry);
        }
        self.fanout_runtime = None;
        self.updated_at = timestamp;
    }

    pub fn start_fanout(
        &mut self,
        parent_node_name: &str,
        child_node_names: Vec<String>,
        timestamp: f64,
    ) -> Result<(), WorkflowError> {
        self.current_node_index = self.node_index(parent_node_name)?;
        if matches!(self.state, RuntimeExecutionState::WaitingApproval) {
            self.state = RuntimeExecutionState::Running;
        }
        let children = child_node_names
            .into_iter()
            .map(|node_name| FanoutChildRuntime {
                node_name,
                session_id: String::new(),
                state: FanoutChildRuntimeState::Running,
                result: None,
                artifact: None,
                contract: None,
                failure_kind: None,
                failure_disposition: None,
                token_usage: TokenUsage::default(),
                attempt: 0,
            })
            .collect();
        self.fanout_runtime = Some(FanoutRuntimeState {
            parent_node_name: parent_node_name.to_string(),
            children,
        });
        self.updated_at = timestamp;
        Ok(())
    }

    pub fn record_fanout_child_started(
        &mut self,
        child_node_name: &str,
        session_id: String,
        execution_count: u32,
        timestamp: f64,
    ) {
        self.node_execution_counts
            .insert(child_node_name.to_string(), execution_count);
        let Some(fanout_runtime) = &mut self.fanout_runtime else {
            self.updated_at = timestamp;
            return;
        };
        if let Some(child) = fanout_runtime
            .children
            .iter_mut()
            .find(|child| child.node_name == child_node_name)
        {
            child.session_id = session_id;
            child.state = FanoutChildRuntimeState::Running;
            child.result = None;
            child.failure_kind = None;
            child.failure_disposition = None;
            child.attempt = execution_count;
        } else {
            fanout_runtime.children.push(FanoutChildRuntime {
                node_name: child_node_name.to_string(),
                session_id,
                state: FanoutChildRuntimeState::Running,
                result: None,
                artifact: None,
                contract: None,
                failure_kind: None,
                failure_disposition: None,
                token_usage: TokenUsage::default(),
                attempt: execution_count,
            });
        }
        self.updated_at = timestamp;
    }

    pub fn record_fanout_child_completed(&mut self, completion: FanoutChildCompletion) {
        let prior = self.artifacts.get(&completion.child_node_name).cloned();
        let output_merge = fanout::merge_fanout_child_completion_output(
            completion.artifact.clone(),
            prior.as_ref().and_then(|output| output.artifact.clone()),
            prior.as_ref().and_then(|output| output.contract.clone()),
        );

        if let Some(fanout_runtime) = &mut self.fanout_runtime {
            if let Some(child) = fanout_runtime
                .children
                .iter_mut()
                .find(|child| child.node_name == completion.child_node_name)
            {
                child.state = FanoutChildRuntimeState::Completed;
                child.result = completion.result.clone();
                child.session_id = completion.session_id.clone();
                child.token_usage = completion.token_usage.clone().unwrap_or_default();
                child.artifact = output_merge.artifact.clone();
                child.contract = output_merge.contract.clone();
                child.failure_kind = None;
                child.failure_disposition = None;
                child.attempt = completion.attempt;
            }
        }

        self.artifacts.insert(
            completion.child_node_name.clone(),
            RuntimeArtifact {
                node_name: completion.child_node_name,
                attempt: completion.attempt,
                session_id: Some(completion.session_id),
                result: completion.result,
                artifact: output_merge.artifact,
                contract: output_merge.contract,
                token_usage: completion.token_usage.clone(),
                completed_at: completion.completed_at,
            },
        );
        if let Some(usage) = &completion.token_usage {
            self.current_node_token_usage.add(usage);
        }
        self.updated_at = completion.completed_at;
    }

    pub fn submit_output(
        &mut self,
        node_name: &str,
        contract: &str,
        artifact: serde_json::Value,
        result: Option<String>,
        timestamp: f64,
    ) {
        let attempt = self
            .node_execution_counts
            .get(node_name)
            .copied()
            .unwrap_or(0);
        self.artifacts.insert(
            node_name.to_string(),
            RuntimeArtifact {
                node_name: node_name.to_string(),
                attempt,
                session_id: None,
                result,
                artifact: Some(artifact),
                contract: Some(contract.to_string()),
                token_usage: None,
                completed_at: timestamp,
            },
        );
        self.updated_at = timestamp;
    }

    fn current_node_name(&self) -> Option<&str> {
        self.workflow
            .nodes
            .get(self.current_node_index)
            .map(|node| node.name.as_str())
    }

    fn node_index(&self, node_name: &str) -> Result<usize, WorkflowError> {
        self.workflow
            .nodes
            .iter()
            .position(|node| node.name == node_name)
            .ok_or_else(|| WorkflowError::validation(format!("node not found: {node_name}")))
    }

    fn aborted_current_history_entry(&mut self, timestamp: f64) -> Option<NodeHistoryEntry> {
        let node_name = self.current_node_name()?.to_string();
        let attempt = self.node_execution_counts.get(&node_name).copied()?;
        let already_in_history = self
            .node_history
            .last()
            .is_some_and(|entry| entry.node_name == node_name && entry.attempt == attempt);
        if already_in_history {
            return None;
        }
        let token_usage = std::mem::take(&mut self.current_node_token_usage);
        Some(history::aborted_node_history_entry(
            node_name,
            attempt,
            self.current_session_id.clone(),
            token_usage,
            timestamp,
        ))
    }

    fn aborted_fanout_history_entry(&self, timestamp: f64) -> Option<NodeHistoryEntry> {
        let fanout_runtime = self.fanout_runtime.as_ref()?;
        let parent_attempt = self
            .node_execution_counts
            .get(&fanout_runtime.parent_node_name)
            .copied()
            .unwrap_or(0);
        Some(history::aborted_fanout_history_entry(
            fanout_runtime,
            &self.artifacts,
            parent_attempt,
            timestamp,
        ))
    }
}

impl FanoutChildRuntimeState {
    pub fn is_completed(self) -> bool {
        matches!(self, Self::Completed)
    }
}

#[cfg(test)]
mod aggregate_tests {
    use super::*;
    use crate::domain::workflow::value_objects::{
        CommandSpec, FacetRefs, FanoutSpec, NodeKind, SessionGate, SessionSpec, WorkflowDefinition,
    };

    fn execution_id() -> WorkflowExecutionId {
        WorkflowExecutionId::new("00000000-0000-4000-8000-000000000001").unwrap()
    }

    fn worktree() -> WorkspaceWorktreePath {
        WorkspaceWorktreePath::new("/tmp/repo").unwrap()
    }

    enum TestNodeKind {
        Session,
        ApprovalSession,
        Command,
        Fanout,
    }

    fn node(name: &str, kind: TestNodeKind) -> NodeDefinition {
        let kind = match kind {
            TestNodeKind::Session => NodeKind::Session(SessionSpec {
                facets: FacetRefs {
                    instruction: Some("implement".to_string()),
                    ..Default::default()
                },
                ..Default::default()
            }),
            TestNodeKind::ApprovalSession => NodeKind::Session(SessionSpec {
                gate: SessionGate::Approval,
                facets: FacetRefs {
                    instruction: Some("implement".to_string()),
                    ..Default::default()
                },
                ..Default::default()
            }),
            TestNodeKind::Command => NodeKind::Command(CommandSpec {
                command: "cargo test".to_string(),
            }),
            TestNodeKind::Fanout => NodeKind::Fanout(FanoutSpec {
                child: vec!["a".to_string(), "b".to_string()],
                items: None,
            }),
        };
        NodeDefinition {
            name: name.to_string(),
            kind,
            ..Default::default()
        }
    }

    fn workflow(nodes: Vec<NodeDefinition>) -> WorkflowDefinition {
        WorkflowDefinition {
            name: "wf".to_string(),
            description: String::new(),
            builtin: false,
            schemas: Default::default(),
            nodes,
        }
    }

    #[test]
    fn new_rejects_empty_and_accepts_command_workflows() {
        let empty = workflow(Vec::new());
        assert!(WorkflowExecution::new(execution_id(), empty, worktree(), None, 1.0).is_err());

        let with_command = workflow(vec![node("script", TestNodeKind::Command)]);
        assert!(
            WorkflowExecution::new(execution_id(), with_command, worktree(), None, 1.0).is_ok()
        );
    }

    #[test]
    fn snapshot_computes_node_statuses_and_approval_operations() {
        let approval = node("approve", TestNodeKind::ApprovalSession);
        let mut exec = WorkflowExecution::new(
            execution_id(),
            workflow(vec![node("plan", TestNodeKind::Session), approval]),
            worktree(),
            Some("task".to_string()),
            1.0,
        )
        .unwrap();
        exec.record_node_completed(NodeCompletion {
            node_name: "plan".to_string(),
            result: Some("ok".to_string()),
            session_id: Some("session-plan".to_string()),
            token_usage: Some(TokenUsage {
                input_tokens: 2,
                output_tokens: 3,
            }),
            artifact: None,
            contract: None,
            attempt: Some(1),
            completed_at: 2.0,
        })
        .unwrap();
        exec.request_approval("approve", 3.0).unwrap();

        let snapshot = exec.to_snapshot();
        assert_eq!(snapshot.total_token_usage.input_tokens, 2);
        assert_eq!(exec.request(), "task");
    }

    #[test]
    fn abort_fanout_records_parent_with_child_snapshots() {
        let mut exec = WorkflowExecution::new(
            execution_id(),
            workflow(vec![
                node("fanout-review", TestNodeKind::Fanout),
                node("a", TestNodeKind::Session),
                node("b", TestNodeKind::Session),
            ]),
            worktree(),
            None,
            1.0,
        )
        .unwrap();
        exec.start_fanout("fanout-review", vec!["a".to_string(), "b".to_string()], 1.1)
            .unwrap();
        exec.record_fanout_child_started("a", "session-a".to_string(), 1, 1.2);
        exec.record_fanout_child_started("b", "session-b".to_string(), 1, 1.2);
        exec.record_fanout_child_completed(FanoutChildCompletion {
            child_node_name: "a".to_string(),
            result: Some("LGTM".to_string()),
            session_id: "session-a".to_string(),
            token_usage: None,
            artifact: None,
            attempt: 1,
            completed_at: 1.5,
        });

        exec.abort_execution(2.0);
        let snapshot = exec.to_snapshot();
        assert_eq!(snapshot.state, RuntimeExecutionState::Aborted);
        assert!(snapshot.node_executions.is_empty());
        let parent = snapshot.node_history.first().unwrap();
        assert_eq!(parent.node_name, "fanout-review");
        assert_eq!(parent.state, "aborted");
        let children = parent.fanout_children.as_ref().unwrap();
        assert_eq!(children.len(), 2);
        assert_eq!(
            children
                .iter()
                .find(|child| child.node_name == "a")
                .unwrap()
                .state,
            "completed"
        );
        assert_eq!(
            children
                .iter()
                .find(|child| child.node_name == "b")
                .unwrap()
                .state,
            "aborted"
        );
    }

    #[test]
    fn submit_output_updates_node_output_without_workflow_variable_side_effects() {
        let mut exec = WorkflowExecution::new(
            execution_id(),
            workflow(vec![node("spec", TestNodeKind::Session)]),
            worktree(),
            None,
            1.0,
        )
        .unwrap();
        exec.submit_output(
            "spec",
            "spec-directory",
            serde_json::json!({"spec_dir": "docs/spec"}),
            None,
            2.0,
        );

        let snapshot = exec.to_snapshot();
        assert_eq!(
            snapshot.artifacts["spec"].contract.as_deref(),
            Some("spec-directory")
        );
    }
}
