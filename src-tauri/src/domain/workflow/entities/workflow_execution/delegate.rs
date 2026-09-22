use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct DelegateRuntime {
    pub(super) iterations: u32,
    pub(super) last_child: Option<String>,
    pub(super) previous_child: Option<String>,
    pub(super) phase: DelegatePhase,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) enum DelegatePhase {
    #[default]
    AcceptingSubmission,
    StartChild,
    WaitingChild,
    InjectResult,
    Complete,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DelegateInjection {
    pub node_execution_id: String,
    pub child_execution_id: String,
}

impl ExecutionTree {
    pub fn is_delegate_parent(&self, id: &str) -> bool {
        self.node_execution(id)
            .and_then(|execution| self.workflow.node_by_name(&execution.node_name))
            .is_some_and(|node| node.completion.delegate.is_some())
    }

    #[cfg(test)]
    pub fn delegate_waits_for_child(&self, id: &str) -> bool {
        self.delegates.get(id).is_some_and(|state| {
            matches!(
                state.phase,
                DelegatePhase::WaitingChild | DelegatePhase::StartChild
            )
        })
    }

    fn delegate_predicate(&self, id: &str) -> bool {
        let Some(execution) = self.node_execution(id) else {
            return false;
        };
        let Some(delegate) = self
            .workflow
            .node_by_name(&execution.node_name)
            .and_then(|node| node.completion.delegate.as_ref())
        else {
            return false;
        };
        delegate.when.evaluate(&mut |field| {
            crate::domain::workflow::FieldPath::from_dotted(field)
                .ok()
                .and_then(|path| {
                    workflow_reference::resolve_value_at_path(execution.artifact.as_ref()?, &path)
                })
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false)
        })
    }

    pub(super) fn accept_submitted_artifact(
        &mut self,
        id: &str,
        result: Option<String>,
        value: serde_json::Value,
        contract: Option<String>,
        timestamp: f64,
    ) -> Option<TransitionOutcome> {
        let value = self.prepare_delegate_submission(id, value)?;
        let outcome =
            self.record_pending_result(id, result, Some(value), contract, None, timestamp);
        self.evaluate_delegate_submission(id);
        Some(outcome)
    }

    fn prepare_delegate_submission(
        &mut self,
        id: &str,
        mut value: serde_json::Value,
    ) -> Option<serde_json::Value> {
        if !self.is_delegate_parent(id) {
            return Some(value);
        }
        let state = self.delegates.entry(id.to_string()).or_default();
        if state.phase != DelegatePhase::AcceptingSubmission {
            return None;
        }
        let iterations = state.iterations;
        let last_child = state.last_child.clone();
        let node = self.node_execution(id)?;
        let max = self
            .workflow
            .node_by_name(&node.node_name)?
            .completion
            .delegate
            .as_ref()?
            .max_iterations;
        let exhausted = iterations >= max;
        let child = if exhausted {
            last_child
                .as_deref()
                .and_then(|id| self.node_execution(id))
                .and_then(|child| child.artifact.clone())
                .unwrap_or(serde_json::Value::Null)
        } else {
            serde_json::Value::Null
        };
        value.as_object_mut()?.insert("child".into(), child);
        self.delegates.get_mut(id)?.phase = if exhausted {
            DelegatePhase::Complete
        } else {
            DelegatePhase::StartChild
        };
        Some(value)
    }

    fn evaluate_delegate_submission(&mut self, id: &str) {
        if self
            .delegates
            .get(id)
            .is_some_and(|state| state.phase == DelegatePhase::StartChild)
            && self.delegate_predicate(id)
        {
            self.delegates.get_mut(id).unwrap().phase = DelegatePhase::Complete;
        }
    }

    pub(super) fn delegate_child_started(&mut self, parent: &str, child: &str) {
        let state = self.delegates.entry(parent.to_string()).or_default();
        state.iterations += 1;
        state.previous_child = state.last_child.take();
        state.last_child = Some(child.to_string());
        state.phase = DelegatePhase::WaitingChild;
    }

    pub(super) fn delegate_bindings(&self, parent_id: &str) -> Vec<(String, serde_json::Value)> {
        let Some(parent) = self.node_execution(parent_id) else {
            return Vec::new();
        };
        let Some(node) = self.workflow.node_by_name(&parent.node_name) else {
            return Vec::new();
        };
        let Some(delegate) = &node.completion.delegate else {
            return Vec::new();
        };
        let mut values: HashMap<_, _> = self
            .leaf_start_for(parent_id)
            .map(|leaf| leaf.bindings.into_iter().collect())
            .unwrap_or_default();
        values.insert(
            "request".into(),
            serde_json::Value::String(self.request.clone().unwrap_or_default()),
        );
        if let Some(mut artifact) = parent.artifact.clone() {
            if let Some(last) = self
                .delegates
                .get(parent_id)
                .and_then(|state| {
                    if state.phase == DelegatePhase::StartChild {
                        state.last_child.as_deref()
                    } else {
                        state.previous_child.as_deref()
                    }
                })
                .and_then(|id| self.node_execution(id))
                .and_then(|child| child.artifact.clone())
            {
                if let Some(object) = artifact.as_object_mut() {
                    object.insert("child".into(), last);
                }
            }
            values.insert(node.name.clone(), artifact);
        }
        workflow_reference::resolve_entry_bindings(Some(&delegate.child_entry()), &values)
    }

    pub(super) fn settle_delegate_child(
        &mut self,
        parent_id: &str,
        child_id: &str,
        effects: &mut AdvanceEffects<'_>,
        timestamp: f64,
    ) -> Result<(), crate::domain::workflow::WorkflowError> {
        let parent_id = self.delegate_continuation_parent(parent_id).to_string();
        let parent_id = parent_id.as_str();
        let child = self
            .node_execution(child_id)
            .and_then(|child| child.artifact.clone())
            .ok_or_else(|| {
                crate::domain::workflow::WorkflowError::invalid_state(
                    "delegate child has no Artifact",
                )
            })?;
        let parent = self.node_execution(parent_id).ok_or_else(|| {
            crate::domain::workflow::WorkflowError::invalid_state("delegate parent is missing")
        })?;
        let mut artifact = parent.artifact.clone().ok_or_else(|| {
            crate::domain::workflow::WorkflowError::invalid_state("delegate parent has no Artifact")
        })?;
        artifact
            .as_object_mut()
            .ok_or_else(|| {
                crate::domain::workflow::WorkflowError::invalid_state(
                    "delegate Artifact is not an object",
                )
            })?
            .insert("child".into(), child);
        self.record_pending_result(parent_id, None, Some(artifact), None, None, timestamp);
        let complete = self.delegate_predicate(parent_id);
        self.delegates
            .get_mut(parent_id)
            .ok_or_else(|| {
                crate::domain::workflow::WorkflowError::invalid_state("delegate state is missing")
            })?
            .phase = if complete {
            DelegatePhase::Complete
        } else {
            DelegatePhase::InjectResult
        };
        match effects {
            AdvanceEffects::Derive => self
                .derive_session_settlement(parent_id, timestamp)
                .map_err(crate::domain::workflow::WorkflowError::invalid_state),
            AdvanceEffects::Live {
                new_id,
                events,
                starts,
            } => {
                let applied =
                    self.apply_node_completion_handshake(parent_id, *new_id, timestamp)?;
                events.extend(applied.events);
                if let Some(ExecutionAdvanceDecision::StartNodes(next)) = applied.advance {
                    starts.extend(next);
                }
                Ok(())
            }
        }
    }

    pub(super) fn delegate_continuation_parent<'a>(&'a self, parent_id: &'a str) -> &'a str {
        let mut current = parent_id;
        for node in &self.runtime.node_executions {
            if self
                .runtime
                .retry_predecessors
                .get(&node.id)
                .map(String::as_str)
                == Some(current)
            {
                current = &node.id;
            }
        }
        current
    }

    pub(super) fn inherit_pending_delegate_result(&mut self, previous_id: &str, next_id: &str) {
        let Some(state) = self
            .delegates
            .get(previous_id)
            .filter(|state| {
                matches!(
                    state.phase,
                    DelegatePhase::StartChild
                        | DelegatePhase::WaitingChild
                        | DelegatePhase::InjectResult
                )
            })
            .cloned()
        else {
            return;
        };
        let previous = self.node_execution(previous_id).unwrap();
        let artifact = self.with_worktree_artifact(next_id, previous.artifact.clone());
        let completion_signals = previous.completion_signals;
        let next = self
            .runtime
            .node_executions
            .iter_mut()
            .find(|node| node.id == next_id)
            .unwrap();
        next.artifact = artifact;
        next.completion_signals = completion_signals;
        self.delegates.insert(next_id.to_string(), state);
    }

    pub fn pending_delegate_injection(&self, parent_id: &str) -> Option<DelegateInjection> {
        let state = self.delegates.get(parent_id)?;
        let parent = self.node_execution(parent_id)?;
        (state.phase == DelegatePhase::InjectResult
            && parent.completion_signals.is_ready()
            && parent.status == RuntimeNodeExecutionStatus::Running)
            .then(|| DelegateInjection {
                node_execution_id: parent_id.to_string(),
                child_execution_id: state.last_child.clone().unwrap(),
            })
    }

    #[cfg(test)]
    pub fn pending_delegate_injections(&self) -> Vec<DelegateInjection> {
        self.delegates
            .keys()
            .filter_map(|id| self.pending_delegate_injection(id))
            .collect()
    }

    pub fn record_delegate_injected(
        &mut self,
        injection: &DelegateInjection,
        timestamp: f64,
    ) -> TransitionOutcome {
        if self
            .pending_delegate_injection(&injection.node_execution_id)
            .as_ref()
            != Some(injection)
        {
            return TransitionOutcome::NotApplicable;
        }
        self.delegates
            .get_mut(&injection.node_execution_id)
            .unwrap()
            .phase = DelegatePhase::AcceptingSubmission;
        let parent = self
            .runtime
            .node_executions
            .iter_mut()
            .find(|node| node.id == injection.node_execution_id)
            .unwrap();
        parent.completion_signals = NodeCompletionSignalState::Pending;
        self.runtime.updated_at = timestamp;
        TransitionOutcome::Applied
    }
}

#[cfg(test)]
#[path = "delegate_test.rs"]
mod delegate_tests;
