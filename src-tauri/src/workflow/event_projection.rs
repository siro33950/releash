//! [04] Command / Event Boundary: `WorkflowEvent` 列から `WorkflowState` を再構築する projection。
//!
//! spec の責務配置に従い、`workflow/log.rs` は NDJSON の append/read 機構へ責務を限定し、
//! event 列 → WorkflowState の射影 (projection) は engine 側の本モジュールに置く。
//! 過去 NDJSON 在庫の互換性は spec [02]/[04] の範囲で別途扱う。

use std::collections::HashMap;

use crate::workflow::event::WorkflowEvent;
use crate::workflow::state::{
    ParallelStepState, StepHistoryEntry, StepOutput, TokenUsage, WorkflowExecutionState,
    WorkflowState,
};

/// イベント列からWorkflowStateを再構築する。
///
/// 引数の `events` は時系列順に append された `WorkflowEvent` 列を想定する。
///
/// [04] schema 境界の不変条件: 復元に用いる `Workflow` 定義は必ず `RunStarted.workflow_definition`
/// snapshot から取り出す。呼び出し側から `Workflow` を受け取らないことで、現在の workflow
/// 定義に依存した「ライブ定義」での再構築を構造的に排除する（workflow 編集後にも過去
/// 実行時点の `total_steps` / `current_step_index` / `step_states` を保つ）。
///
/// `RunStarted` を含まない events 列（empty / 旧 NDJSON 互換破棄後の異常入力）は `Ok(None)`
/// を返す。
///
/// 本関数は workflow モジュール内（`workflow::commands` / `workflow::log` 等）
/// からのみ参照される内部 API のため `pub(crate)` に絞る。
pub(crate) fn reconstruct_state_from_events(
    run_id: &str,
    events: &[WorkflowEvent],
) -> Result<Option<WorkflowState>, String> {
    if events.is_empty() {
        return Ok(None);
    }

    // [04] schema 境界: 復元に使う workflow 定義は必ず RunStarted snapshot から取る。
    // 呼び出し側から渡されたライブ定義を採用しないことで、workflow 編集後にも
    // 過去実行時点の total_steps / current_step_index / step_states を保つ。
    let Some(workflow) = events.iter().find_map(|e| match e {
        WorkflowEvent::RunStarted {
            workflow_definition,
            ..
        } => Some(workflow_definition.clone()),
        _ => None,
    }) else {
        return Ok(None);
    };
    let workflow = &workflow;

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
                // [04] 終端遷移では active_parallel_steps を必ず空にする。
                // `WorkflowState.active_parallel_steps` は engine ライブ state では
                // 「現在進行中の並列子のみ」を意味し、`ParallelCompleted` を経由せずに
                // 終端へ到達した経路（NodeFailed → RunFailed 等）でも UI が stale な
                // 並列子を表示しないよう projection 側でも同じ不変条件を担保する。
                active_parallel_steps.clear();
                updated_at = *timestamp;
            }
            WorkflowEvent::RunFailed {
                reason, timestamp, ..
            } => {
                exec_state = WorkflowExecutionState::Failed {
                    reason: reason.clone(),
                };
                active_parallel_steps.clear();
                updated_at = *timestamp;
            }
            WorkflowEvent::RunAborted { timestamp, .. } => {
                exec_state = WorkflowExecutionState::Aborted;
                active_parallel_steps.clear();
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::schema::{NodeDefinition, NodeType, Workflow};

    fn agent_node(name: &str) -> NodeDefinition {
        NodeDefinition {
            name: name.to_string(),
            node_type: NodeType::Agent,
            instruction: Some("x".to_string()),
            ..NodeDefinition::default()
        }
    }

    fn workflow_with_nodes(name: &str, nodes: Vec<&str>) -> Workflow {
        Workflow {
            name: name.to_string(),
            description: String::new(),
            builtin: false,
            nodes: nodes.into_iter().map(agent_node).collect(),
        }
    }

    fn run_started(run_id: &str, workflow: Workflow) -> WorkflowEvent {
        WorkflowEvent::RunStarted {
            run_id: run_id.to_string(),
            workflow_name: workflow.name.clone(),
            workflow_file_stem: workflow.name.clone(),
            worktree_path: "/repo".to_string(),
            workflow_definition: workflow,
            timestamp: 1000.0,
        }
    }

    /// [04] schema 境界: 復元に使う Workflow は `RunStarted.workflow_definition` snapshot
    /// から取り出し、関数 API は外部から Workflow を受け取らない。RunStarted を含まない
    /// events 列は `Ok(None)` を返す（empty / 異常入力の取り扱い）。
    #[test]
    fn projection_returns_none_when_run_started_missing() {
        let events = vec![WorkflowEvent::NodeStarted {
            run_id: "exec-x".to_string(),
            workflow_name: "wf".to_string(),
            node_name: "a".to_string(),
            execution_count: 1,
            timestamp: 1.0,
        }];
        let result = reconstruct_state_from_events("exec-x", &events).unwrap();
        assert!(
            result.is_none(),
            "RunStarted を含まない events 列は復元対象外"
        );
    }

    /// [04] C1 担保: `total_steps` / `step_states` は `RunStarted.workflow_definition`
    /// snapshot に従って復元される。snapshot が 3 ノード（plan/implement/review）で
    /// あれば、後から workflow を編集しても復元結果はこの snapshot に従う（ライブ定義
    /// を引きずらない構造的不変条件）。
    #[test]
    fn projection_uses_run_started_snapshot_for_total_steps_and_step_states() {
        let snapshot = workflow_with_nodes("wf", vec!["plan", "implement", "review"]);
        let events = vec![
            run_started("exec-snap", snapshot),
            WorkflowEvent::NodeStarted {
                run_id: "exec-snap".to_string(),
                workflow_name: "wf".to_string(),
                node_name: "plan".to_string(),
                execution_count: 1,
                timestamp: 1001.0,
            },
            WorkflowEvent::NodeCompleted {
                run_id: "exec-snap".to_string(),
                workflow_name: "wf".to_string(),
                node_name: "plan".to_string(),
                result: Some("ok".to_string()),
                session_id: None,
                token_usage: None,
                structured_output: None,
                run_index: Some(1),
                timestamp: 1002.0,
            },
            WorkflowEvent::RunCompleted {
                run_id: "exec-snap".to_string(),
                workflow_name: "wf".to_string(),
                total_token_usage: TokenUsage::default(),
                timestamp: 1003.0,
            },
        ];

        let state = reconstruct_state_from_events("exec-snap", &events)
            .unwrap()
            .unwrap();
        assert_eq!(state.total_steps, 3, "snapshot の nodes.len() に従う");
        assert_eq!(state.step_states["plan"], "completed");
        assert_eq!(state.step_states["implement"], "pending");
        assert_eq!(state.step_states["review"], "pending");
        assert_eq!(state.workflow_definition.nodes.len(), 3);
    }

    /// 並列 child を抱えたまま `RunCompleted` で終端した場合、`active_parallel_steps`
    /// は空になる（[04] C2: ライブ state の不変条件 — 「現在進行中の並列子のみ」 — を
    /// projection でも担保する）。
    #[test]
    fn projection_clears_active_parallel_steps_on_run_completed() {
        let snapshot = workflow_with_nodes("wf", vec!["parallel-review"]);
        let events = vec![
            run_started("exec-pc", snapshot),
            WorkflowEvent::ParallelStarted {
                run_id: "exec-pc".to_string(),
                workflow_name: "wf".to_string(),
                parent_node_name: "parallel-review".to_string(),
                child_node_names: vec!["a".to_string(), "b".to_string()],
                timestamp: 1.0,
            },
            WorkflowEvent::RunCompleted {
                run_id: "exec-pc".to_string(),
                workflow_name: "wf".to_string(),
                total_token_usage: TokenUsage::default(),
                timestamp: 2.0,
            },
        ];
        let state = reconstruct_state_from_events("exec-pc", &events)
            .unwrap()
            .unwrap();
        assert_eq!(state.state, WorkflowExecutionState::Completed);
        assert!(
            state.active_parallel_steps.is_empty(),
            "RunCompleted では active_parallel_steps は空"
        );
    }

    /// `ParallelStarted` 後に `NodeFailed` → `RunFailed` で終端しても、
    /// `active_parallel_steps` は空になる（`ParallelCompleted` を経由しない経路）。
    #[test]
    fn projection_clears_active_parallel_steps_on_run_failed() {
        let snapshot = workflow_with_nodes("wf", vec!["parallel-review"]);
        let events = vec![
            run_started("exec-pf", snapshot),
            WorkflowEvent::ParallelStarted {
                run_id: "exec-pf".to_string(),
                workflow_name: "wf".to_string(),
                parent_node_name: "parallel-review".to_string(),
                child_node_names: vec!["a".to_string(), "b".to_string()],
                timestamp: 1.0,
            },
            WorkflowEvent::NodeFailed {
                run_id: "exec-pf".to_string(),
                workflow_name: "wf".to_string(),
                node_name: "a".to_string(),
                reason: "boom".to_string(),
                timestamp: 2.0,
            },
            WorkflowEvent::RunFailed {
                run_id: "exec-pf".to_string(),
                workflow_name: "wf".to_string(),
                reason: "child failed".to_string(),
                timestamp: 3.0,
            },
        ];
        let state = reconstruct_state_from_events("exec-pf", &events)
            .unwrap()
            .unwrap();
        assert!(matches!(state.state, WorkflowExecutionState::Failed { .. }));
        assert!(
            state.active_parallel_steps.is_empty(),
            "RunFailed では active_parallel_steps は空"
        );
    }

    /// `ParallelStarted` 後に `RunAborted` で終端しても、`active_parallel_steps` は空になる。
    #[test]
    fn projection_clears_active_parallel_steps_on_run_aborted() {
        let snapshot = workflow_with_nodes("wf", vec!["parallel-review"]);
        let events = vec![
            run_started("exec-pa", snapshot),
            WorkflowEvent::ParallelStarted {
                run_id: "exec-pa".to_string(),
                workflow_name: "wf".to_string(),
                parent_node_name: "parallel-review".to_string(),
                child_node_names: vec!["a".to_string(), "b".to_string()],
                timestamp: 1.0,
            },
            WorkflowEvent::RunAborted {
                run_id: "exec-pa".to_string(),
                workflow_name: "wf".to_string(),
                timestamp: 2.0,
            },
        ];
        let state = reconstruct_state_from_events("exec-pa", &events)
            .unwrap()
            .unwrap();
        assert_eq!(state.state, WorkflowExecutionState::Aborted);
        assert!(
            state.active_parallel_steps.is_empty(),
            "RunAborted では active_parallel_steps は空"
        );
    }
}
