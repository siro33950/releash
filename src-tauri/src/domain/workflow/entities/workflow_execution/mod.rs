//! Test-only workflow execution aggregate plus shared execution projection helpers.
//!
//! The stateful `WorkflowExecution` aggregate in this module is retained for
//! domain unit tests. Production execution state is owned by the workflow
//! gateway runtime state; pure validation lives in workflow services.

use std::collections::{HashMap, HashSet};

#[cfg(test)]
use crate::domain::workflow::services::{history, parallel, projection, validation};
#[cfg(test)]
use crate::domain::workflow::value_objects::{
    ApprovalOperations, NodeDefinition, RunId, StepOutput, WorkflowStateSnapshot, WorktreePath,
};
use crate::domain::workflow::value_objects::{
    ParallelAggregate, StepHistoryEntry, TokenUsage, WorkflowDefinition, WorkflowExecutionState,
    WorkflowStepFailureKind, STEP_STATE_COMPLETED, STEP_STATE_FAILED, STEP_STATE_INTERRUPTED,
    STEP_STATE_PENDING, STEP_STATE_RUNNING,
};
use crate::domain::workflow::FailureDisposition;
#[cfg(test)]
use crate::domain::workflow::WorkflowError;

/// Test-only stateful aggregate used by domain unit tests.
#[derive(Debug, Clone, PartialEq)]
#[cfg(test)]
pub struct WorkflowExecution {
    id: RunId,
    workflow: WorkflowDefinition,
    state: WorkflowExecutionState,
    current_step_index: usize,
    step_execution_counts: HashMap<String, u32>,
    step_history: Vec<StepHistoryEntry>,
    worktree_path: WorktreePath,
    started_at: f64,
    updated_at: f64,
    current_session_id: Option<String>,
    current_step_token_usage: TokenUsage,
    terminal_total_token_usage: Option<TokenUsage>,
    step_outputs: HashMap<String, StepOutput>,
    task: Option<String>,
    parallel_run: Option<ParallelRunState>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParallelRunState {
    pub parent_step_name: String,
    pub aggregate: Option<ParallelAggregate>,
    pub children: Vec<ParallelChildRun>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParallelChildRun {
    pub step_name: String,
    pub session_id: String,
    pub state: ParallelChildState,
    pub result: Option<String>,
    pub structured_output: Option<serde_json::Value>,
    pub artifact_contract: Option<String>,
    pub failure_kind: Option<WorkflowStepFailureKind>,
    pub failure_disposition: Option<FailureDisposition>,
    pub token_usage: TokenUsage,
    pub run_index: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParallelChildState {
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
    pub structured_output: Option<serde_json::Value>,
    pub artifact_contract: Option<String>,
    pub run_index: Option<u32>,
    pub completed_at: f64,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg(test)]
pub struct ParallelChildCompletion {
    pub child_node_name: String,
    pub result: Option<String>,
    pub session_id: String,
    pub token_usage: Option<TokenUsage>,
    pub structured_output: Option<serde_json::Value>,
    pub run_index: u32,
    pub completed_at: f64,
}

#[cfg(test)]
impl WorkflowExecution {
    pub fn new(
        id: RunId,
        workflow: WorkflowDefinition,
        worktree_path: WorktreePath,
        task: Option<String>,
        started_at: f64,
    ) -> Result<Self, WorkflowError> {
        validation::validate_workflow_shape(&workflow)?;
        let first_step_name = workflow
            .nodes
            .first()
            .map(|node| node.name.clone())
            .ok_or_else(|| WorkflowError::validation("workflow has no nodes"))?;
        Ok(Self {
            id,
            workflow,
            state: WorkflowExecutionState::Running,
            current_step_index: 0,
            step_execution_counts: HashMap::from([(first_step_name, 1)]),
            step_history: Vec::new(),
            worktree_path,
            started_at,
            updated_at: started_at,
            current_session_id: None,
            current_step_token_usage: TokenUsage::default(),
            terminal_total_token_usage: None,
            step_outputs: HashMap::new(),
            task,
            parallel_run: None,
        })
    }

    pub fn task(&self) -> Option<&str> {
        self.task.as_deref()
    }

    pub fn to_snapshot(&self) -> WorkflowStateSnapshot {
        let total_token_usage = self
            .terminal_total_token_usage
            .clone()
            .unwrap_or_else(|| projection::total_token_usage(&self.step_history));

        let step_states = compute_step_states(
            &self.workflow,
            self.current_step_index,
            &self.state,
            &self.step_history,
        );

        WorkflowStateSnapshot {
            execution_id: self.id.to_string(),
            workflow_name: self.workflow.name.clone(),
            state: self.state.clone(),
            current_step_index: self.current_step_index,
            current_step_name: self.current_step_name().unwrap_or_default().to_string(),
            current_session_id: self.current_session_id.clone(),
            total_steps: self.workflow.nodes.len(),
            step_history: self.step_history.clone(),
            step_execution_counts: self.step_execution_counts.clone(),
            workflow_definition: self.workflow.clone(),
            total_token_usage,
            step_states,
            step_outputs: self.step_outputs.clone(),
            active_parallel_steps: projection::active_parallel_steps(self.parallel_run.as_ref()),
            approval_operations: self.build_approval_operations(),
            stall_observations: Vec::new(),
            started_at: self.started_at,
            updated_at: self.updated_at,
        }
    }

    pub fn record_node_completed(
        &mut self,
        completion: NodeCompletion,
    ) -> Result<(), WorkflowError> {
        self.node_index(&completion.node_name)?;
        let run_index = completion.run_index.unwrap_or_else(|| {
            self.step_execution_counts
                .get(&completion.node_name)
                .copied()
                .unwrap_or(0)
        });
        let entry = history::completed_step_history_entry(history::CompletedStepHistoryInput {
            step_name: completion.node_name,
            completed_at: completion.completed_at,
            result: completion.result,
            session_id: completion.session_id,
            token_usage: completion.token_usage.clone(),
            structured_output: completion.structured_output,
            run_index,
        });
        if let Some(output) =
            history::step_output_from_completed_history_entry(&entry, completion.artifact_contract)
        {
            self.step_outputs.insert(entry.step_name.clone(), output);
        }
        self.step_history.push(entry);
        self.current_step_token_usage = TokenUsage::default();
        self.updated_at = completion.completed_at;
        Ok(())
    }

    pub fn request_approval(
        &mut self,
        node_name: &str,
        timestamp: f64,
    ) -> Result<(), WorkflowError> {
        self.current_step_index = self.node_index(node_name)?;
        self.state = WorkflowExecutionState::WaitingApproval;
        self.updated_at = timestamp;
        Ok(())
    }

    pub fn abort_run(&mut self, timestamp: f64) {
        self.state = WorkflowExecutionState::Aborted;
        if let Some(entry) = self.aborted_parallel_history_entry(timestamp) {
            self.step_history.push(entry);
        } else if let Some(entry) = self.aborted_current_history_entry(timestamp) {
            self.step_history.push(entry);
        }
        self.parallel_run = None;
        self.updated_at = timestamp;
    }

    pub fn start_parallel(
        &mut self,
        parent_node_name: &str,
        child_node_names: Vec<String>,
        aggregate: Option<ParallelAggregate>,
        timestamp: f64,
    ) -> Result<(), WorkflowError> {
        self.current_step_index = self.node_index(parent_node_name)?;
        if matches!(self.state, WorkflowExecutionState::WaitingApproval) {
            self.state = WorkflowExecutionState::Running;
        }
        let children = child_node_names
            .into_iter()
            .map(|step_name| ParallelChildRun {
                step_name,
                session_id: String::new(),
                state: ParallelChildState::Running,
                result: None,
                structured_output: None,
                artifact_contract: None,
                failure_kind: None,
                failure_disposition: None,
                token_usage: TokenUsage::default(),
                run_index: 0,
            })
            .collect();
        self.parallel_run = Some(ParallelRunState {
            parent_step_name: parent_node_name.to_string(),
            aggregate,
            children,
        });
        self.updated_at = timestamp;
        Ok(())
    }

    pub fn record_parallel_child_started(
        &mut self,
        child_node_name: &str,
        session_id: String,
        execution_count: u32,
        timestamp: f64,
    ) {
        self.step_execution_counts
            .insert(child_node_name.to_string(), execution_count);
        let Some(parallel_run) = &mut self.parallel_run else {
            self.updated_at = timestamp;
            return;
        };
        if let Some(child) = parallel_run
            .children
            .iter_mut()
            .find(|child| child.step_name == child_node_name)
        {
            child.session_id = session_id;
            child.state = ParallelChildState::Running;
            child.result = None;
            child.failure_kind = None;
            child.failure_disposition = None;
            child.run_index = execution_count;
        } else {
            parallel_run.children.push(ParallelChildRun {
                step_name: child_node_name.to_string(),
                session_id,
                state: ParallelChildState::Running,
                result: None,
                structured_output: None,
                artifact_contract: None,
                failure_kind: None,
                failure_disposition: None,
                token_usage: TokenUsage::default(),
                run_index: execution_count,
            });
        }
        self.updated_at = timestamp;
    }

    pub fn record_parallel_child_completed(&mut self, completion: ParallelChildCompletion) {
        let prior = self.step_outputs.get(&completion.child_node_name).cloned();
        let output_merge = parallel::merge_parallel_child_completion_output(
            completion.structured_output.clone(),
            prior
                .as_ref()
                .and_then(|output| output.structured_output.clone()),
            prior
                .as_ref()
                .and_then(|output| output.artifact_contract.clone()),
        );

        if let Some(parallel_run) = &mut self.parallel_run {
            if let Some(child) = parallel_run
                .children
                .iter_mut()
                .find(|child| child.step_name == completion.child_node_name)
            {
                child.state = ParallelChildState::Completed;
                child.result = completion.result.clone();
                child.session_id = completion.session_id.clone();
                child.token_usage = completion.token_usage.clone().unwrap_or_default();
                child.structured_output = output_merge.structured_output.clone();
                child.artifact_contract = output_merge.artifact_contract.clone();
                child.failure_kind = None;
                child.failure_disposition = None;
                child.run_index = completion.run_index;
            }
        }

        self.step_outputs.insert(
            completion.child_node_name.clone(),
            StepOutput {
                step_name: completion.child_node_name,
                run_index: completion.run_index,
                session_id: Some(completion.session_id),
                result: completion.result,
                structured_output: output_merge.structured_output,
                artifact_contract: output_merge.artifact_contract,
                token_usage: completion.token_usage.clone(),
                completed_at: completion.completed_at,
            },
        );
        if let Some(usage) = &completion.token_usage {
            self.current_step_token_usage.add(usage);
        }
        self.updated_at = completion.completed_at;
    }

    pub fn submit_output(
        &mut self,
        node_name: &str,
        contract: &str,
        structured_output: serde_json::Value,
        result: Option<String>,
        timestamp: f64,
    ) {
        let run_index = self
            .step_execution_counts
            .get(node_name)
            .copied()
            .unwrap_or(0);
        self.step_outputs.insert(
            node_name.to_string(),
            StepOutput {
                step_name: node_name.to_string(),
                run_index,
                session_id: None,
                result,
                structured_output: Some(structured_output),
                artifact_contract: Some(contract.to_string()),
                token_usage: None,
                completed_at: timestamp,
            },
        );
        self.updated_at = timestamp;
    }

    fn current_step_name(&self) -> Option<&str> {
        self.workflow
            .nodes
            .get(self.current_step_index)
            .map(|node| node.name.as_str())
    }

    fn current_step(&self) -> Option<&NodeDefinition> {
        self.workflow.nodes.get(self.current_step_index)
    }

    fn node_index(&self, node_name: &str) -> Result<usize, WorkflowError> {
        self.workflow
            .nodes
            .iter()
            .position(|node| node.name == node_name)
            .ok_or_else(|| WorkflowError::validation(format!("node not found: {node_name}")))
    }

    fn build_approval_operations(&self) -> Option<ApprovalOperations> {
        projection::approval_operations(&self.state, self.current_step())
    }

    fn aborted_current_history_entry(&mut self, timestamp: f64) -> Option<StepHistoryEntry> {
        let step_name = self.current_step_name()?.to_string();
        let run_index = self.step_execution_counts.get(&step_name).copied()?;
        let already_in_history = self
            .step_history
            .last()
            .is_some_and(|entry| entry.step_name == step_name && entry.run_index == run_index);
        if already_in_history {
            return None;
        }
        let token_usage = std::mem::take(&mut self.current_step_token_usage);
        Some(history::aborted_step_history_entry(
            step_name,
            run_index,
            self.current_session_id.clone(),
            token_usage,
            timestamp,
        ))
    }

    fn aborted_parallel_history_entry(&self, timestamp: f64) -> Option<StepHistoryEntry> {
        let parallel_run = self.parallel_run.as_ref()?;
        let parent_run_index = self
            .step_execution_counts
            .get(&parallel_run.parent_step_name)
            .copied()
            .unwrap_or(0);
        Some(history::aborted_parallel_history_entry(
            parallel_run,
            &self.step_outputs,
            parent_run_index,
            timestamp,
        ))
    }
}

impl ParallelChildState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => STEP_STATE_RUNNING,
            Self::Completed => STEP_STATE_COMPLETED,
            Self::Failed => STEP_STATE_FAILED,
            Self::Interrupted => STEP_STATE_INTERRUPTED,
        }
    }

    pub fn is_completed(self) -> bool {
        matches!(self, Self::Completed)
    }
}

pub fn compute_step_states(
    workflow: &WorkflowDefinition,
    current_step_index: usize,
    state: &WorkflowExecutionState,
    step_history: &[StepHistoryEntry],
) -> HashMap<String, String> {
    let completed: HashSet<&str> = step_history
        .iter()
        .map(|entry| entry.step_name.as_str())
        .collect();
    workflow
        .nodes
        .iter()
        .enumerate()
        .map(|(index, node)| {
            let in_history = completed.contains(node.name.as_str());
            let node_state = if index == current_step_index {
                if matches!(state, WorkflowExecutionState::Failed { .. }) && in_history {
                    STEP_STATE_COMPLETED
                } else {
                    state.as_str()
                }
            } else if in_history {
                STEP_STATE_COMPLETED
            } else {
                STEP_STATE_PENDING
            };
            (node.name.clone(), node_state.to_string())
        })
        .collect()
}

#[cfg(test)]
mod aggregate_tests {
    use super::*;
    use crate::domain::workflow::value_objects::{
        CommandSpec, FacetRefs, FanoutSpec, InterimChild, NodeKind, SessionGate, SessionSpec,
        WorkflowDefinition,
    };

    fn run_id() -> RunId {
        RunId::new("00000000-0000-4000-8000-000000000001").unwrap()
    }

    fn worktree() -> WorktreePath {
        WorktreePath::new("/tmp/repo").unwrap()
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
                parallel_children: vec![child("a"), child("b")],
                aggregate: None,
            }),
        };
        NodeDefinition {
            name: name.to_string(),
            kind,
            ..Default::default()
        }
    }

    fn child(name: &str) -> InterimChild {
        InterimChild {
            name: name.to_string(),
            facets: FacetRefs {
                instruction: Some("implement".to_string()),
                ..Default::default()
            },
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
    fn new_rejects_empty_and_bash_workflows() {
        let empty = workflow(Vec::new());
        assert!(WorkflowExecution::new(run_id(), empty, worktree(), None, 1.0).is_err());

        let with_command = workflow(vec![node("script", TestNodeKind::Command)]);
        assert!(WorkflowExecution::new(run_id(), with_command, worktree(), None, 1.0).is_err());
    }

    #[test]
    fn snapshot_computes_step_states_and_approval_operations() {
        let approval = node("approve", TestNodeKind::ApprovalSession);
        let mut exec = WorkflowExecution::new(
            run_id(),
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
            structured_output: None,
            artifact_contract: None,
            run_index: Some(1),
            completed_at: 2.0,
        })
        .unwrap();
        exec.request_approval("approve", 3.0).unwrap();

        let snapshot = exec.to_snapshot();
        assert_eq!(snapshot.step_states["plan"], "completed");
        assert_eq!(snapshot.step_states["approve"], "waiting_approval");
        assert_eq!(snapshot.total_token_usage.input_tokens, 2);
        assert!(!snapshot.approval_operations.unwrap().can_reject);
        assert_eq!(exec.task(), Some("task"));
    }

    #[test]
    fn abort_parallel_records_parent_with_child_snapshots() {
        let mut exec = WorkflowExecution::new(
            run_id(),
            workflow(vec![node("parallel-review", TestNodeKind::Fanout)]),
            worktree(),
            None,
            1.0,
        )
        .unwrap();
        exec.start_parallel(
            "parallel-review",
            vec!["a".to_string(), "b".to_string()],
            None,
            1.1,
        )
        .unwrap();
        exec.record_parallel_child_started("a", "session-a".to_string(), 1, 1.2);
        exec.record_parallel_child_started("b", "session-b".to_string(), 1, 1.2);
        exec.record_parallel_child_completed(ParallelChildCompletion {
            child_node_name: "a".to_string(),
            result: Some("LGTM".to_string()),
            session_id: "session-a".to_string(),
            token_usage: None,
            structured_output: None,
            run_index: 1,
            completed_at: 1.5,
        });

        exec.abort_run(2.0);
        let snapshot = exec.to_snapshot();
        assert_eq!(snapshot.state, WorkflowExecutionState::Aborted);
        assert!(snapshot.active_parallel_steps.is_empty());
        let parent = snapshot.step_history.first().unwrap();
        assert_eq!(parent.step_name, "parallel-review");
        assert_eq!(parent.state, "aborted");
        let children = parent.child_outputs.as_ref().unwrap();
        assert_eq!(children.len(), 2);
        assert_eq!(
            children
                .iter()
                .find(|child| child.step_name == "a")
                .unwrap()
                .state,
            "completed"
        );
        assert_eq!(
            children
                .iter()
                .find(|child| child.step_name == "b")
                .unwrap()
                .state,
            "aborted"
        );
    }

    #[test]
    fn submit_output_updates_step_output_without_workflow_variable_side_effects() {
        let mut exec = WorkflowExecution::new(
            run_id(),
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
            snapshot.step_outputs["spec"].artifact_contract.as_deref(),
            Some("spec-directory")
        );
    }
}
