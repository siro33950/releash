use super::*;
use crate::domain::workflow::NodeFact;

impl ExecutionTree {
    pub fn restore_without_definition(
        tree_id: &str,
        root: &crate::domain::workflow::TreeRootFact,
        timestamp: f64,
    ) -> Self {
        Self {
            state: RuntimeExecutionState::Running,
            archive: None,
            pending_restart: None,
            delegates: HashMap::new(),
            pending_empty_fanout: None,
            runtime: ExecutionTreeView {
                id: tree_id.to_string(),
                workflow_name: root.workflow_name.clone(),
                workflow: None,
                node_history: Vec::new(),
                workflow_defaults: WorkflowDefaults,
                worktree_path: root.worktree_path.clone(),
                workspace_identity: root.workspace_identity.clone(),
                repository_root: root.repository_root.clone(),
                launched_as: root.launched_as,
                created_from: root.created_from,
                error_reason: None,
                started_at: timestamp,
                updated_at: timestamp,
                current_session_id: None,
                scopes: Vec::new(),
                node_executions: Vec::new(),
                retry_predecessors: HashMap::new(),
                request: (!root.request.is_empty()).then(|| root.request.clone()),
                current_stall_observations: Vec::new(),
            },
        }
    }

    pub fn replay_terminal_fact(&mut self, fact: &NodeFact, timestamp: f64) {
        let Some(state) = fact.terminal_state() else {
            return;
        };
        self.state = state.clone();
        self.runtime.error_reason = match fact {
            NodeFact::AbortRequested(fact) => fact.reason.clone(),
            _ => None,
        };
        for node in &mut self.runtime.node_executions {
            if node.status.is_active() {
                node.status = match state {
                    RuntimeExecutionState::Completed => RuntimeNodeExecutionStatus::Succeeded,
                    _ => RuntimeNodeExecutionStatus::Aborted,
                };
                node.completed_at = Some(timestamp);
            }
        }
        if self.runtime.workflow.is_none() {
            self.restore_terminal_artifacts();
        }
        self.runtime.scopes.clear();
        self.runtime.updated_at = timestamp;
    }

    fn restore_terminal_artifacts(&mut self) {
        let mut children: HashMap<String, Vec<usize>> = HashMap::new();
        for (index, node) in self.runtime.node_executions.iter().enumerate() {
            if let Some(parent) = &node.parent {
                children
                    .entry(parent.parent_id.clone())
                    .or_default()
                    .push(index);
            }
        }
        for index in (0..self.runtime.node_executions.len()).rev() {
            let node = &self.runtime.node_executions[index];
            if node.status != RuntimeNodeExecutionStatus::Succeeded {
                continue;
            }
            let child_indices = children
                .get(&node.id)
                .map(Vec::as_slice)
                .unwrap_or_default();
            let mut artifact = node.artifact.clone();
            let mut token_usage = (node.kind == NodeKindName::Fanout).then(TokenUsage::default);
            if node.kind.is_composite_kind() {
                let width = child_indices
                    .iter()
                    .filter_map(|index| {
                        self.runtime.node_executions[*index]
                            .parent
                            .as_ref()?
                            .fanout_slot()
                            .map(|slot| slot.child_index + 1)
                    })
                    .max()
                    .unwrap_or(0);
                let mut values = serde_json::Map::new();
                for child_index in child_indices {
                    let child = &self.runtime.node_executions[*child_index];
                    if child.status != RuntimeNodeExecutionStatus::Succeeded
                        || (node.kind == NodeKindName::Sequence && child.artifact.is_none())
                    {
                        continue;
                    }
                    if let (Some(total), Some(usage)) = (&mut token_usage, &child.token_usage) {
                        total.add(usage);
                    }
                    let key = match child
                        .parent
                        .as_ref()
                        .and_then(|parent| parent.fanout_slot())
                    {
                        Some(slot) if slot.item_index.is_some() => {
                            (slot.item_index.unwrap() * width + slot.child_index).to_string()
                        }
                        _ => child.node_name.clone(),
                    };
                    values.insert(
                        key,
                        child.artifact.clone().unwrap_or(serde_json::Value::Null),
                    );
                }
                artifact = Some(serde_json::Value::Object(values));
            } else if node.kind == NodeKindName::Session {
                if let Some(child) = child_indices
                    .iter()
                    .rev()
                    .map(|index| &self.runtime.node_executions[*index])
                    .find(|child| {
                        child.status == RuntimeNodeExecutionStatus::Succeeded
                            && child
                                .parent
                                .as_ref()
                                .is_some_and(ExecutionParentRef::is_delegate_child)
                    })
                {
                    if let (Some(object), Some(value)) = (
                        artifact.as_mut().and_then(serde_json::Value::as_object_mut),
                        child.artifact.as_ref(),
                    ) {
                        object.insert("child".into(), value.clone());
                    }
                }
            }
            if let Some(worktree) = &node.worktree {
                artifact = Some(worktree.with_artifact(artifact));
            }
            self.runtime.node_executions[index].artifact = artifact;
            if let Some(usage) = token_usage {
                self.runtime.node_executions[index].token_usage = Some(usage);
            }
        }
    }
}

#[cfg(test)]
#[path = "terminal_replay_test.rs"]
mod terminal_replay_tests;
