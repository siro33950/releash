use super::*;
use crate::domain::workflow::entities::workflow_execution::ExecutionAdvanceDecision;
use crate::domain::workflow::entities::workflow_execution::{
    CompositePreparation, DelegateInjection,
};
use std::collections::VecDeque;

pub(super) enum NodePreparation {
    Leaf(LeafStart),
    Composite(CompositePreparation),
}

impl NodePreparation {
    fn node_execution_id(&self) -> &str {
        match self {
            Self::Leaf(leaf) => &leaf.node_execution_id,
            Self::Composite(composite) => &composite.node_execution_id,
        }
    }
}

#[derive(Default)]
pub(super) struct PreparedNodes {
    pub(super) leaves: Vec<LeafStart>,
    pub(super) injections: Vec<DelegateInjection>,
}

pub(super) fn partition_actions(
    actions: Vec<NodeStart>,
) -> (Vec<NodePreparation>, Vec<DelegateInjection>) {
    let mut preparations = Vec::new();
    let mut injections = Vec::new();
    for action in actions {
        match action {
            NodeStart::Leaf(leaf) => preparations.push(NodePreparation::Leaf(leaf)),
            NodeStart::PrepareComposite(composite) => {
                preparations.push(NodePreparation::Composite(composite))
            }
            NodeStart::InjectDelegate(injection) => injections.push(injection),
        }
    }
    (preparations, injections)
}

impl WorkflowRuntimeHost {
    pub(super) async fn prepare_isolated_starts<R: tauri::Runtime + 'static>(
        &self,
        app: &tauri::AppHandle<R>,
        execution_id: &str,
        worktree_path: &str,
        starts: Vec<NodePreparation>,
    ) -> Result<PreparedNodes, WorkflowRuntimeError> {
        let gate = self.runtime_activation_gate(execution_id).await;
        let guard = gate.lock.lock().await;
        let mut pending = VecDeque::from(starts);
        let mut prepared = PreparedNodes::default();
        let mut failures = Vec::new();
        let mut committed = None;
        while let Some(start) = pending.pop_front() {
            let context = {
                let executions = self.executions.lock().await;
                let execution = executions.get(execution_id).ok_or_else(|| {
                    WorkflowRuntimeError::ExecutionNotFound(execution_id.to_string())
                })?;
                let node = execution
                    .node_execution(start.node_execution_id())
                    .filter(|node| node.status == NodeExecutionStatus::Running);
                node.map(|node| {
                    (
                        node.worktree.clone(),
                        execution.parent_worktree_path(&node.id).map(str::to_string),
                    )
                })
            };
            let Some((worktree, parent)) = context else {
                continue;
            };
            if let Some(worktree) = worktree {
                let parent = parent.ok_or_else(|| {
                    WorkflowRuntimeError::InvalidState(
                        "parent execution worktree is unavailable".to_string(),
                    )
                })?;
                let gateway = self.isolated_worktrees.clone();
                let result = tokio::task::spawn_blocking(move || {
                    if !gateway.is_created(&parent, &worktree)? {
                        gateway.create(&parent, &worktree)?;
                    }
                    Ok::<_, crate::domain::workflow::WorkflowError>(())
                })
                .await
                .map_err(|error| WorkflowRuntimeError::SessionStore(error.to_string()))?
                .map_err(|error| WorkflowRuntimeError::SessionStore(error.to_string()));
                if let Err(error) = result {
                    failures.push((start.node_execution_id().to_string(), error));
                    continue;
                }
            }
            let composite = match start {
                NodePreparation::Leaf(leaf) => {
                    prepared.leaves.push(leaf);
                    continue;
                }
                NodePreparation::Composite(composite) => composite,
            };
            let (before, snapshot, applied) = {
                let mut executions = self.executions.lock().await;
                let execution = executions.get_mut(execution_id).ok_or_else(|| {
                    WorkflowRuntimeError::ExecutionNotFound(execution_id.to_string())
                })?;
                let before = execution.clone();
                let applied = execution
                    .start_prepared_composite(
                        &composite.node_execution_id,
                        &mut new_node_execution_id,
                        current_timestamp(),
                    )
                    .map_err(|error| WorkflowRuntimeError::InvalidState(error.to_string()))?;
                (
                    before,
                    RuntimeCommitSnapshot::from_execution(execution)?,
                    applied,
                )
            };
            let result = self
                .commit_required_events(
                    app,
                    RequiredEventCommit {
                        execution_id,
                        snapshot_for_commit: &snapshot,
                        snapshot_before: before,
                        execution_store_snapshot_before: self
                            .execution_store
                            .active_execution_snapshot(execution_id)
                            .await,
                        required_events: applied.events,
                        append_error_context: "isolated composite child start append failed",
                    },
                )
                .await;
            if let Err(error) = result {
                failures.push((composite.node_execution_id, error));
                continue;
            }
            if let ExecutionAdvanceDecision::StartNodes(children) = applied.decision {
                let (children, injections) = partition_actions(children);
                pending.extend(children);
                prepared.injections.extend(injections);
            }
            committed = Some(snapshot);
        }
        drop(guard);
        drop(gate);
        if let Some(snapshot) = committed {
            self.finalize_after_commit(app, &snapshot, worktree_path)
                .await;
        }
        for (node_execution_id, error) in failures {
            Box::pin(self.settle_runtime_failure_for_node(
                app,
                execution_id,
                &node_execution_id,
                &error,
            ))
            .await?;
        }
        Ok(prepared)
    }
}
