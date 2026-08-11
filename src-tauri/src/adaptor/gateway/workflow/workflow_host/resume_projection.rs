//! Pure event-log projection used by `ResumeExecution`.
//!
//! `NodeCompleted` is the only confirmation boundary. An artifact emitted by an unfinished node
//! is intentionally ignored, while completed fanout children are retained by their stable
//! parent/item/child coordinates.

use std::collections::HashMap;

use crate::domain::workflow::services::event_replay::project_retained_workflow_execution;
use crate::domain::workflow::services::routing::LoopGuardResetBaselines;
use crate::domain::workflow::{ExecutionOrigin, WorkflowExecution as DomainWorkflowExecution};
use crate::domain::workflow::{WorkflowDefinition, WorkflowEvent};

/// Canonical active execution shape used for application restart reconciliation.
/// This deliberately carries only
/// event-projected state; no runtime-local session/command state is trusted.
#[derive(Debug, Clone)]
pub(crate) struct ActiveRestartProjection {
    pub(crate) execution_id: String,
    pub(crate) workflow: WorkflowDefinition,
    pub(crate) worktree_path: String,
    pub(crate) request: String,
    pub(crate) created_from: ExecutionOrigin,
    pub(crate) started_at: f64,
    pub(crate) node_execution_counts: HashMap<String, u32>,
    pub(crate) loop_guard_reset_baselines: LoopGuardResetBaselines,
    pub(crate) projected_execution: DomainWorkflowExecution,
}

#[derive(Debug, Clone)]
struct ExecutionStartSnapshot {
    workflow: WorkflowDefinition,
    worktree_path: String,
    request: String,
    created_from: ExecutionOrigin,
    started_at: f64,
}

fn unique_execution_start(
    execution_id: &str,
    events: &[WorkflowEvent],
) -> Result<ExecutionStartSnapshot, String> {
    let starts = events
        .iter()
        .filter_map(|event| match event {
            WorkflowEvent::ExecutionStarted {
                definition,
                worktree_path,
                request,
                created_from,
                timestamp,
                ..
            } => Some(ExecutionStartSnapshot {
                workflow: definition.clone(),
                worktree_path: worktree_path.clone(),
                request: request.clone(),
                created_from: *created_from,
                started_at: *timestamp,
            }),
            _ => None,
        })
        .collect::<Vec<_>>();
    let [start] = starts.as_slice() else {
        return Err(format!(
            "execution {execution_id} must contain exactly one execution_started event"
        ));
    };
    Ok(start.clone())
}

pub(crate) fn project_restart_checkpoint(
    execution_id: &str,
    events: &[WorkflowEvent],
) -> Result<ActiveRestartProjection, String> {
    let projection = project_retained_workflow_execution(execution_id, events)?
        .ok_or_else(|| format!("execution {execution_id} has no execution_started event"))?;
    let start = unique_execution_start(execution_id, events)?;
    Ok(ActiveRestartProjection {
        execution_id: execution_id.to_string(),
        workflow: start.workflow,
        worktree_path: start.worktree_path,
        request: start.request,
        created_from: start.created_from,
        started_at: start.started_at,
        node_execution_counts: projection.node_execution_counts,
        loop_guard_reset_baselines: projection.loop_guard_reset_baselines,
        projected_execution: projection.execution,
    })
}
