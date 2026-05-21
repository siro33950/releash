//! [04] Command / Event Boundary: `WorkflowEvent` 列から `WorkflowState` を再構築する projection。
//!
//! spec の責務配置に従い、`workflow/log.rs` は NDJSON の append/read 機構へ責務を限定し、
//! event 列 → WorkflowState の射影 (projection) は engine 側の本モジュールに置く。
//! 過去 NDJSON 在庫の互換性は spec [02]/[04] の範囲で別途扱う。

use std::collections::HashMap;

use crate::workflow::event::WorkflowEvent;
use crate::workflow::schema::Workflow;
use crate::workflow::state::{
    ParallelStepState, StepHistoryEntry, StepOutput, TokenUsage, WorkflowExecutionState,
    WorkflowState,
};

/// イベント列からWorkflowStateを再構築する。
///
/// 引数の `events` は時系列順に append された `WorkflowEvent` 列を想定する。
/// `workflow` は `RunStarted.workflow_definition` から取得した workflow を渡す
/// （[02] schema 境界で snapshot 経由の再構築のみ許容）。
///
/// [04] 本関数は workflow モジュール内（`workflow::commands` / `workflow::log` 等）
/// からのみ参照される内部 API のため `pub(crate)` に絞る。
pub(crate) fn reconstruct_state_from_events(
    run_id: &str,
    events: &[WorkflowEvent],
    workflow: &Workflow,
) -> Result<Option<WorkflowState>, String> {
    if events.is_empty() {
        return Ok(None);
    }

    let mut started_at = 0.0;
    let mut updated_at = 0.0;
    let mut step_history: Vec<StepHistoryEntry> = Vec::new();
    let mut step_execution_counts: HashMap<String, u32> = HashMap::new();
    let mut step_outputs: HashMap<String, StepOutput> = HashMap::new();
    let mut total_token_usage = TokenUsage::default();
    let mut exec_state = WorkflowExecutionState::Running;
    let mut current_step_name = workflow
        .nodes
        .first()
        .map(|s| s.name.clone())
        .unwrap_or_default();
    let mut current_step_index = 0usize;
    let mut workflow_name = String::new();
    let mut active_parallel_steps: Vec<ParallelStepState> = Vec::new();

    for event in events {
        match event {
            WorkflowEvent::RunStarted {
                timestamp,
                workflow_name: wn,
                ..
            } => {
                started_at = *timestamp;
                updated_at = *timestamp;
                workflow_name = wn.clone();
            }
            WorkflowEvent::NodeStarted {
                node_name,
                execution_count,
                timestamp,
                ..
            } => {
                current_step_name = node_name.clone();
                current_step_index = workflow
                    .nodes
                    .iter()
                    .position(|s| s.name == *node_name)
                    .unwrap_or(0);
                step_execution_counts.insert(node_name.clone(), *execution_count);
                // [04] approval を経て次 node が開始した場合に exec_state を
                // Running へ復元する。ApprovalRequested で WaitingApproval に
                // 切り替わったまま NodeStarted が来ると、復元 state が承認待ち
                // のまま固定されてしまうため、本イベントで Running に戻す。
                if matches!(exec_state, WorkflowExecutionState::WaitingApproval) {
                    exec_state = WorkflowExecutionState::Running;
                }
                updated_at = *timestamp;
            }
            WorkflowEvent::NodeCompleted {
                node_name,
                result,
                session_id,
                token_usage,
                structured_output,
                run_index,
                timestamp,
                ..
            } => {
                let ri = run_index
                    .unwrap_or_else(|| step_execution_counts.get(node_name).copied().unwrap_or(0));
                step_history.push(StepHistoryEntry {
                    step_name: node_name.clone(),
                    completed_at: *timestamp,
                    result: result.clone(),
                    session_id: session_id.clone(),
                    token_usage: token_usage.clone(),
                    structured_output: structured_output.clone(),
                    run_index: ri,
                    child_outputs: None,
                });
                if structured_output.is_some() {
                    step_outputs.insert(
                        node_name.clone(),
                        StepOutput {
                            step_name: node_name.clone(),
                            run_index: ri,
                            session_id: session_id.clone(),
                            result: result.clone(),
                            structured_output: structured_output.clone(),
                            output_contract: None,
                            token_usage: token_usage.clone(),
                            completed_at: *timestamp,
                        },
                    );
                }
                if let Some(ref usage) = token_usage {
                    total_token_usage.add(usage);
                }
                updated_at = *timestamp;
            }
            WorkflowEvent::NodeFailed {
                reason, timestamp, ..
            } => {
                for ps in &mut active_parallel_steps {
                    if ps.state == "running" {
                        ps.state = "failed".to_string();
                    }
                }
                exec_state = WorkflowExecutionState::Failed {
                    reason: reason.clone(),
                };
                updated_at = *timestamp;
            }
            WorkflowEvent::ApprovalRequested {
                node_name,
                timestamp,
                ..
            } => {
                // [04] approval 到達は state 上 WaitingApproval を意味する。
                // event 列から復元した state が Running のまま残ると、UI / observer から
                // 承認待ち run を識別できなくなる。current_step_name / current_step_index も
                // approval 対象 node に揃える。
                current_step_name = node_name.clone();
                current_step_index = workflow
                    .nodes
                    .iter()
                    .position(|s| s.name == *node_name)
                    .unwrap_or(current_step_index);
                exec_state = WorkflowExecutionState::WaitingApproval;
                updated_at = *timestamp;
            }
            WorkflowEvent::ApprovalResolved { timestamp, .. } => {
                updated_at = *timestamp;
            }
            WorkflowEvent::RunCompleted {
                total_token_usage: tu,
                timestamp,
                ..
            } => {
                exec_state = WorkflowExecutionState::Completed;
                total_token_usage = tu.clone();
                updated_at = *timestamp;
            }
            WorkflowEvent::RunFailed {
                reason, timestamp, ..
            } => {
                exec_state = WorkflowExecutionState::Failed {
                    reason: reason.clone(),
                };
                updated_at = *timestamp;
            }
            WorkflowEvent::RunAborted { timestamp, .. } => {
                exec_state = WorkflowExecutionState::Aborted;
                updated_at = *timestamp;
            }
            WorkflowEvent::OutputCollected { timestamp, .. } => {
                updated_at = *timestamp;
            }
            WorkflowEvent::ParallelStarted {
                parent_node_name,
                child_node_names,
                timestamp,
                ..
            } => {
                current_step_name = parent_node_name.clone();
                current_step_index = workflow
                    .nodes
                    .iter()
                    .position(|s| s.name == *parent_node_name)
                    .unwrap_or(current_step_index);
                if matches!(exec_state, WorkflowExecutionState::WaitingApproval) {
                    exec_state = WorkflowExecutionState::Running;
                }
                active_parallel_steps = child_node_names
                    .iter()
                    .map(|name| ParallelStepState {
                        step_name: name.clone(),
                        state: "running".to_string(),
                        session_id: None,
                        result: None,
                        run_index: 0,
                        completed_at: None,
                        structured_output: None,
                        output_contract: None,
                    })
                    .collect();
                updated_at = *timestamp;
            }
            WorkflowEvent::ParallelChildStarted {
                child_node_name,
                session_id,
                execution_count,
                timestamp,
                ..
            } => {
                step_execution_counts.insert(child_node_name.clone(), *execution_count);
                if let Some(ps) = active_parallel_steps
                    .iter_mut()
                    .find(|p| p.step_name == *child_node_name)
                {
                    ps.state = "running".to_string();
                    ps.session_id = Some(session_id.clone());
                    ps.result = None;
                    ps.run_index = *execution_count;
                    ps.completed_at = None;
                } else {
                    active_parallel_steps.push(ParallelStepState {
                        step_name: child_node_name.clone(),
                        state: "running".to_string(),
                        session_id: Some(session_id.clone()),
                        result: None,
                        run_index: *execution_count,
                        completed_at: None,
                        structured_output: None,
                        output_contract: None,
                    });
                }
                updated_at = *timestamp;
            }
            WorkflowEvent::ParallelChildCompleted {
                child_node_name,
                result,
                session_id,
                token_usage,
                structured_output,
                run_index,
                timestamp,
                ..
            } => {
                if let Some(ps) = active_parallel_steps
                    .iter_mut()
                    .find(|p| p.step_name == *child_node_name)
                {
                    ps.state = "completed".to_string();
                    ps.result = result.clone();
                    ps.completed_at = Some(*timestamp);
                    ps.structured_output = structured_output.clone();
                }
                step_outputs.insert(
                    child_node_name.clone(),
                    StepOutput {
                        step_name: child_node_name.clone(),
                        run_index: *run_index,
                        session_id: Some(session_id.clone()),
                        result: result.clone(),
                        structured_output: structured_output.clone(),
                        output_contract: None,
                        token_usage: token_usage.clone(),
                        completed_at: *timestamp,
                    },
                );
                if let Some(ref usage) = token_usage {
                    total_token_usage.add(usage);
                }
                updated_at = *timestamp;
            }
            WorkflowEvent::ParallelCompleted { timestamp, .. } => {
                active_parallel_steps.clear();
                updated_at = *timestamp;
            }
            WorkflowEvent::ContractRepairRequested { timestamp, .. } => {
                updated_at = *timestamp;
            }
        }
    }

    let step_states = crate::workflow::state::compute_step_states(
        workflow,
        current_step_index,
        &exec_state,
        &step_history,
    );

    Ok(Some(WorkflowState {
        execution_id: run_id.to_string(),
        workflow_name,
        chat_session_id: None,
        state: exec_state,
        current_step_index,
        current_step_name,
        current_session_id: None,
        total_steps: workflow.nodes.len(),
        step_history,
        step_execution_counts,
        workflow_definition: workflow.clone(),
        total_token_usage,
        step_outputs,
        step_states,
        active_parallel_steps,
        workflow_variables: HashMap::new(),
        approval_operations: None,
        started_at,
        updated_at,
    }))
}
