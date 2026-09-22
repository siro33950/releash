use super::*;

impl ExecutionTree {
    pub fn can_prepare_node_worktree(&self, node_execution_id: &str) -> bool {
        self.state() != &RuntimeExecutionState::Aborted
            && self.node_execution(node_execution_id).is_some_and(|node| {
                node.can_start_process()
                    || node.kind.is_composite_kind()
                        && matches!(
                            node.status,
                            RuntimeNodeExecutionStatus::Running
                                | RuntimeNodeExecutionStatus::Succeeded
                        )
            })
    }

    pub fn execution_worktree_path(&self, node_execution_id: &str) -> Option<&str> {
        let node = self.node_execution(node_execution_id)?;
        let ancestors = std::iter::successors(Some(node), |node| {
            node.parent
                .as_ref()
                .and_then(|parent| self.node_execution(&parent.parent_id))
        });
        Some(
            crate::domain::workflow::WorktreeInheritance::effective_path(
                &self.runtime.worktree_path,
                ancestors.map(|node| {
                    (
                        crate::domain::workflow::WorktreeInheritance::new(
                            self.node_definition(&node.node_name)
                                .and_then(|definition| definition.worktree),
                        ),
                        node.worktree.as_ref(),
                    )
                }),
            ),
        )
    }

    pub fn parent_worktree_path(&self, node_execution_id: &str) -> Option<&str> {
        let node = self.node_execution(node_execution_id)?;
        match &node.parent {
            Some(parent) => self.execution_worktree_path(&parent.parent_id),
            None => Some(&self.runtime.worktree_path),
        }
    }

    pub fn isolated_composite_start(&self, scope_id: &str) -> Option<CompositePreparation> {
        let node = self.node_execution(scope_id)?;
        if node.worktree.is_none()
            || !node.kind.is_composite_kind()
            || node.status != RuntimeNodeExecutionStatus::Running
        {
            return None;
        }
        self.scope(scope_id)?;
        if self.runtime.node_executions.iter().any(|child| {
            child
                .parent
                .as_ref()
                .is_some_and(|parent| parent.parent_id == scope_id)
        }) {
            return None;
        }
        Some(CompositePreparation {
            node_execution_id: scope_id.to_string(),
            node_name: node.node_name.clone(),
        })
    }

    pub fn start_prepared_composite(
        &mut self,
        scope_id: &str,
        new_id: &mut dyn FnMut() -> String,
        timestamp: f64,
    ) -> Result<AppliedAdvance, crate::domain::workflow::WorkflowError> {
        let node = self.node_execution(scope_id).ok_or_else(|| {
            crate::domain::workflow::WorkflowError::invalid_state("composite execution is missing")
        })?;
        if node.status == RuntimeNodeExecutionStatus::Succeeded {
            let ancestors = std::iter::successors(Some(node), |node| {
                node.parent
                    .as_ref()
                    .and_then(|parent| self.node_execution(&parent.parent_id))
            })
            .map(|node| node.id.clone())
            .collect::<Vec<_>>();
            if let Some(advance) = self.derive_pending_advances().into_iter().find(|advance| {
                matches!(advance, PendingAdvance::AfterChild { scope_id, .. } if ancestors.contains(scope_id))
            }) {
                return self.apply_pending_advance(&advance, new_id, timestamp);
            }
            let starts = ancestors
                .iter()
                .filter_map(|id| self.pending_delegate_injection(id))
                .filter(|injection| ancestors.contains(&injection.child_execution_id))
                .map(NodeStart::InjectDelegate)
                .collect::<Vec<_>>();
            return Ok(AppliedAdvance {
                decision: if starts.is_empty() {
                    ExecutionAdvanceDecision::Persist
                } else {
                    ExecutionAdvanceDecision::StartNodes(starts)
                },
                events: Vec::new(),
            });
        }
        let advance = match node.kind {
            NodeKindName::Sequence => PendingAdvance::StartEntry {
                scope_id: scope_id.to_string(),
            },
            NodeKindName::Fanout => PendingAdvance::ExpandFanout {
                scope_id: scope_id.to_string(),
            },
            _ => {
                return Err(crate::domain::workflow::WorkflowError::invalid_state(
                    "node is not a composite",
                ))
            }
        };
        self.apply_pending_advance(&advance, new_id, timestamp)
    }

    pub(super) fn with_worktree_artifact(
        &self,
        node_execution_id: &str,
        artifact: Option<serde_json::Value>,
    ) -> Option<serde_json::Value> {
        let Some(worktree) = self
            .node_execution(node_execution_id)
            .and_then(|node| node.worktree.as_ref())
        else {
            return artifact;
        };
        Some(worktree.with_artifact(artifact))
    }
}

#[cfg(test)]
#[path = "worktree_test.rs"]
mod worktree_tests;
