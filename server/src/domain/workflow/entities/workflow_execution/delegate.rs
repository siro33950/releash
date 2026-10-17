use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct DelegateRuntime {
    pub(crate) iterations: u32,
    pub(crate) last_child: Option<String>,
    pub(crate) previous_child: Option<String>,
    pub(crate) phase: DelegatePhase,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) enum DelegatePhase {
    #[default]
    AcceptingSubmission,
    StartChild,
    WaitingChild,
    InjectResult,
    Complete,
}

impl DelegateRuntime {
    pub(crate) fn prepare_submission(&mut self, max: u32) -> bool {
        if self.phase != DelegatePhase::AcceptingSubmission {
            return false;
        }
        self.phase = if self.iterations >= max {
            DelegatePhase::Complete
        } else {
            DelegatePhase::StartChild
        };
        true
    }

    pub(crate) fn submitted(&mut self, predicate: bool) {
        if self.phase == DelegatePhase::StartChild && predicate {
            self.phase = DelegatePhase::Complete;
        }
    }

    pub(crate) fn prepare_artifact(
        &mut self,
        delegate: &crate::domain::workflow::SessionDelegate,
        mut value: serde_json::Value,
        last_child_artifact: Option<&serde_json::Value>,
    ) -> Option<serde_json::Value> {
        if self.phase != DelegatePhase::AcceptingSubmission {
            return None;
        }
        let child = if self.iterations >= delegate.max_iterations {
            last_child_artifact
                .cloned()
                .unwrap_or(serde_json::Value::Null)
        } else {
            serde_json::Value::Null
        };
        value.as_object_mut()?.insert("child".into(), child);
        self.prepare_submission(delegate.max_iterations);
        Some(value)
    }

    pub(crate) fn inherit_pending_result(
        &self,
        artifact: Option<&serde_json::Value>,
        signals: NodeCompletionSignalState,
    ) -> Option<(Self, Option<serde_json::Value>, NodeCompletionSignalState)> {
        self.has_pending_result()
            .then(|| (self.clone(), artifact.cloned(), signals))
    }

    pub(crate) fn settle_child(
        &mut self,
        delegate: &crate::domain::workflow::SessionDelegate,
        child_id: &str,
        parent: Option<&serde_json::Value>,
        child: Option<&serde_json::Value>,
    ) -> Result<Option<serde_json::Value>, crate::domain::workflow::WorkflowError> {
        if self.last_child.as_deref() != Some(child_id) {
            return Ok(None);
        }
        let child = child.ok_or_else(|| {
            crate::domain::workflow::WorkflowError::invalid_state("delegate child has no Artifact")
        })?;
        let mut artifact = parent.cloned().ok_or_else(|| {
            crate::domain::workflow::WorkflowError::invalid_state("delegate parent has no Artifact")
        })?;
        artifact
            .as_object_mut()
            .ok_or_else(|| {
                crate::domain::workflow::WorkflowError::invalid_state(
                    "delegate Artifact is not an object",
                )
            })?
            .insert("child".into(), child.clone());
        self.child_completed(delegate_predicate(delegate, Some(&artifact)));
        Ok(Some(artifact))
    }

    pub(crate) fn child_started(&mut self, child: &str) {
        self.iterations += 1;
        self.previous_child = self.last_child.take();
        self.last_child = Some(child.to_string());
        self.phase = DelegatePhase::WaitingChild;
    }

    pub(crate) fn child_restarted(&mut self, child: &str) {
        self.last_child = Some(child.to_string());
        self.phase = DelegatePhase::WaitingChild;
    }

    pub(crate) fn has_pending_result(&self) -> bool {
        matches!(
            self.phase,
            DelegatePhase::StartChild | DelegatePhase::WaitingChild | DelegatePhase::InjectResult
        )
    }

    pub(crate) fn child_completed(&mut self, predicate: bool) {
        self.phase = if predicate {
            DelegatePhase::Complete
        } else {
            DelegatePhase::InjectResult
        };
    }

    pub(crate) fn pending_injection(
        &self,
        status: RuntimeNodeExecutionStatus,
        signals: NodeCompletionSignalState,
    ) -> Option<&str> {
        (self.phase == DelegatePhase::InjectResult
            && signals.is_ready()
            && status == RuntimeNodeExecutionStatus::Running)
            .then_some(self.last_child.as_deref())
            .flatten()
    }

    pub(crate) fn inject_result(
        &mut self,
        child: &str,
        status: RuntimeNodeExecutionStatus,
        signals: &mut NodeCompletionSignalState,
    ) -> bool {
        if self.pending_injection(status, *signals) != Some(child) {
            return false;
        }
        *signals = self.injected();
        true
    }

    pub(crate) fn injected(&mut self) -> NodeCompletionSignalState {
        self.phase = DelegatePhase::AcceptingSubmission;
        NodeCompletionSignalState::Pending
    }

    pub(crate) fn handshake(
        &self,
        signals: NodeCompletionSignalState,
    ) -> Option<NodeCompletionHandshakeDecision> {
        use NodeCompletionHandshakeDecision as D;
        match self.phase {
            DelegatePhase::StartChild => Some(D::Delegate),
            DelegatePhase::InjectResult if signals.is_ready() => Some(D::Delegate),
            DelegatePhase::Complete => None,
            _ => Some(D::AwaitingSignal),
        }
    }

    pub(crate) fn waits_for_child(&self) -> bool {
        matches!(
            self.phase,
            DelegatePhase::WaitingChild | DelegatePhase::StartChild
        )
    }
}

pub(crate) fn delegate_continuation<'a>(
    parent: &'a str,
    nodes: impl Iterator<Item = (&'a str, Option<&'a str>)>,
) -> &'a str {
    let mut current = parent;
    for (id, predecessor) in nodes {
        if predecessor == Some(current) {
            current = id;
        }
    }
    current
}

pub(crate) fn delegate_predicate(
    delegate: &crate::domain::workflow::SessionDelegate,
    artifact: Option<&serde_json::Value>,
) -> bool {
    delegate.when.evaluate(&mut |field| {
        crate::domain::workflow::FieldPath::from_dotted(field)
            .ok()
            .and_then(|path| workflow_reference::resolve_value_at_path(artifact?, &path))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DelegateInjection {
    pub node_execution_id: String,
    pub child_execution_id: String,
}

impl ExecutionTree {
    pub fn is_delegate_parent(&self, id: &str) -> bool {
        self.node_execution(id)
            .and_then(|execution| self.node_definition(&execution.node_name))
            .is_some_and(|node| node.completion.delegate.is_some())
    }

    pub fn delegate_waits_for_child(&self, id: &str) -> bool {
        self.delegates
            .get(id)
            .is_some_and(DelegateRuntime::waits_for_child)
    }

    fn delegate_predicate(&self, id: &str) -> bool {
        let Some(execution) = self.node_execution(id) else {
            return false;
        };
        let Some(delegate) = self
            .node_definition(&execution.node_name)
            .and_then(|node| node.completion.delegate.as_ref())
        else {
            return false;
        };
        delegate_predicate(delegate, execution.artifact.as_ref())
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
        value: serde_json::Value,
    ) -> Option<serde_json::Value> {
        if !self.is_delegate_parent(id) {
            return Some(value);
        }
        let node = self.node_execution(id)?;
        let delegate = self
            .node_definition(&node.node_name)?
            .completion
            .delegate
            .as_ref()?;
        let mut state = self.delegates.get(id).cloned().unwrap_or_default();
        let last = state
            .last_child
            .as_deref()
            .and_then(|id| self.node_execution(id))
            .and_then(|node| node.artifact.as_ref());
        let value = state.prepare_artifact(delegate, value, last)?;
        self.delegates.insert(id.to_string(), state);
        Some(value)
    }

    fn evaluate_delegate_submission(&mut self, id: &str) {
        let predicate = self.delegate_predicate(id);
        if let Some(state) = self.delegates.get_mut(id) {
            state.submitted(predicate);
        }
    }

    pub(super) fn delegate_child_started(&mut self, parent: &str, child: &str) {
        self.delegates
            .entry(parent.to_string())
            .or_default()
            .child_started(child);
    }

    pub(super) fn delegate_bindings(&self, parent_id: &str) -> Vec<(String, serde_json::Value)> {
        let Some(parent) = self.node_execution(parent_id) else {
            return Vec::new();
        };
        let Some(node) = self.node_definition(&parent.node_name) else {
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
        let parent = self.node_execution(parent_id).ok_or_else(|| {
            crate::domain::workflow::WorkflowError::invalid_state("delegate parent is missing")
        })?;
        let delegate = self
            .node_definition(&parent.node_name)
            .and_then(|node| node.completion.delegate.as_ref())
            .ok_or_else(|| {
                crate::domain::workflow::WorkflowError::invalid_state(
                    "delegate definition is missing",
                )
            })?;
        let mut state = self.delegates.get(parent_id).cloned().ok_or_else(|| {
            crate::domain::workflow::WorkflowError::invalid_state("delegate state is missing")
        })?;
        let child = self
            .node_execution(child_id)
            .and_then(|node| node.artifact.as_ref());
        let Some(artifact) =
            state.settle_child(delegate, child_id, parent.artifact.as_ref(), child)?
        else {
            return Ok(());
        };
        self.record_pending_result(parent_id, None, Some(artifact), None, None, timestamp);
        self.delegates.insert(parent_id.to_string(), state);
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
        delegate_continuation(
            parent_id,
            self.runtime.node_executions.iter().map(|node| {
                (
                    node.id.as_str(),
                    self.runtime
                        .retry_predecessors
                        .get(&node.id)
                        .map(String::as_str),
                )
            }),
        )
    }

    pub(super) fn inherit_pending_delegate_result(&mut self, previous_id: &str, next_id: &str) {
        let Some(previous) = self.node_execution(previous_id) else {
            return;
        };
        let Some((state, artifact, completion_signals)) =
            self.delegates.get(previous_id).and_then(|state| {
                state
                    .inherit_pending_result(previous.artifact.as_ref(), previous.completion_signals)
            })
        else {
            return;
        };
        let artifact = self.with_worktree_artifact(next_id, artifact);
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
        let child = state.pending_injection(parent.status, parent.completion_signals)?;
        Some(DelegateInjection {
            node_execution_id: parent_id.to_string(),
            child_execution_id: child.to_string(),
        })
    }

    #[cfg(any(test, feature = "test-support"))]
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
        let parent = self
            .runtime
            .node_executions
            .iter_mut()
            .find(|node| node.id == injection.node_execution_id)
            .unwrap();
        self.delegates
            .get_mut(&injection.node_execution_id)
            .unwrap()
            .inject_result(
                &injection.child_execution_id,
                parent.status,
                &mut parent.completion_signals,
            );
        self.runtime.updated_at = timestamp;
        TransitionOutcome::Applied
    }
}

#[cfg(test)]
#[path = "delegate_test.rs"]
pub(crate) mod delegate_tests;
