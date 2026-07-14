//! `WorkflowEvent` 列から workflow read model / runtime state を再構築する projection。
//!
//! spec の責務配置に従い、`workflow/log.rs` は NDJSON の append/read 機構へ責務を限定し、
//! event 列 → WorkflowState の射影 (projection) は gateway 側の本モジュールに置く。
//! 過去 NDJSON 在庫の互換性は spec [02]/[04] の範囲で別途扱う。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::adaptor::gateway::workflow::domain_mapping::{
    node_definition_to_domain, workflow_execution_state_to_domain, workflow_schemas_to_domain,
};
#[cfg(test)]
use crate::adaptor::gateway::workflow::event::{
    CliMutationRejectionReason, CliMutationRequestRecord, CollectedOutputEntry, FanoutParentRef,
};
use crate::adaptor::gateway::workflow::event::{
    RunAbortedChildOutcome, RunAbortedChildOutputSnapshot, RunAbortedStepSnapshot,
    TokenUsage as EventTokenUsage, WorkflowEvent,
};
#[cfg(test)]
use crate::adaptor::gateway::workflow::schema::NodeKindName;
use crate::adaptor::gateway::workflow::schema::Workflow;
use crate::adaptor::gateway::workflow::state::{
    ApprovalOperations, ChildOutputSnapshot, NodeExecution, NodeExecutionFailure,
    NodeExecutionStatus, StepHistoryEntry, StepOutput, TokenUsage, WorkflowExecutionState,
    WorkflowStallObservation, WorkflowState,
};
use crate::domain::workflow::services::{
    contract as workflow_contract, projection as workflow_projection,
};
#[cfg(test)]
use crate::domain::workflow::WorkflowStepFailureKind;
use crate::domain::workflow::{
    ContractValidationResult, STEP_STATE_ABORTED, STEP_STATE_COMPLETED, STEP_STATE_INTERRUPTED,
    STEP_STATE_PENDING,
};

/// 秒単位の f64 タイムスタンプ（engine 内 `current_timestamp()` 由来）を
/// frontend 表示用のミリ秒単位に変換するための係数。
const SECONDS_TO_MS: f64 = 1000.0;

#[inline]
fn seconds_to_ms(value: f64) -> f64 {
    value * SECONDS_TO_MS
}

fn step_history_entry_from_run_aborted_snapshot(
    snapshot: &RunAbortedStepSnapshot,
) -> StepHistoryEntry {
    StepHistoryEntry {
        step_name: snapshot.step_name.clone(),
        completed_at: snapshot.completed_at,
        result: snapshot.result.clone(),
        session_id: snapshot.session_id.clone(),
        token_usage: snapshot.token_usage.clone(),
        structured_output: snapshot.structured_output.clone(),
        run_index: snapshot.run_index,
        child_outputs: snapshot.child_outputs.as_ref().map(|children| {
            children
                .iter()
                .map(child_output_from_run_aborted_snapshot)
                .collect()
        }),
        state: STEP_STATE_ABORTED.to_string(),
    }
}

fn child_output_from_run_aborted_snapshot(
    snapshot: &RunAbortedChildOutputSnapshot,
) -> ChildOutputSnapshot {
    ChildOutputSnapshot {
        step_name: snapshot.step_name.clone(),
        session_id: snapshot.session_id.clone(),
        result: snapshot.result.clone(),
        run_index: snapshot.run_index,
        completed_at: snapshot.completed_at,
        structured_output: snapshot.structured_output.clone(),
        artifact_contract: snapshot.artifact_contract.clone(),
        state: match snapshot.outcome {
            RunAbortedChildOutcome::Completed => STEP_STATE_COMPLETED,
            RunAbortedChildOutcome::Aborted => STEP_STATE_ABORTED,
        }
        .to_string(),
        failure_kind: None,
        failure_disposition: None,
    }
}

/// spec issues-1023: event 列から NodeExecution ID ごとの started/completed/duration を
/// 集約する純粋関数。engine 側 event projection の責務として、所要時間計算と
/// 単位変換（秒 → ミリ秒）を担う（frontend は表示用フォーマットのみ）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowStepTimingView {
    pub node_execution_id: String,
    pub step_name: String,
    pub run_index: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<f64>,
}

pub(crate) fn compute_step_timings(events: &[WorkflowEvent]) -> Vec<WorkflowStepTimingView> {
    // node_execution_id -> (node_name, attempt, started_at, completed_at) 秒単位。
    let mut buckets: HashMap<String, (String, u32, Option<f64>, Option<f64>)> = HashMap::new();
    let mut order: Vec<String> = Vec::new();

    for event in events {
        match event {
            WorkflowEvent::NodeStarted {
                node_execution_id,
                node_name,
                attempt,
                timestamp,
                ..
            } => {
                let entry = buckets.entry(node_execution_id.clone()).or_insert_with(|| {
                    order.push(node_execution_id.clone());
                    (node_name.clone(), *attempt, None, None)
                });
                if entry.2.is_none() {
                    entry.2 = Some(*timestamp);
                }
            }
            WorkflowEvent::NodeCompleted {
                node_execution_id,
                node_name,
                run_index,
                timestamp,
                ..
            } => {
                let entry = buckets.entry(node_execution_id.clone()).or_insert_with(|| {
                    order.push(node_execution_id.clone());
                    (node_name.clone(), run_index.unwrap_or(0), None, None)
                });
                entry.3 = Some(*timestamp);
            }
            WorkflowEvent::NodeFailed {
                node_execution_id,
                node_name,
                timestamp,
                ..
            } => {
                let entry = buckets.entry(node_execution_id.clone()).or_insert_with(|| {
                    order.push(node_execution_id.clone());
                    (node_name.clone(), 0, None, None)
                });
                entry.3 = Some(*timestamp);
            }
            _ => {}
        }
    }

    order
        .into_iter()
        .map(|node_execution_id| {
            let (node_name, attempt, started_at, completed_at) = buckets
                .remove(&node_execution_id)
                .unwrap_or_else(|| (String::new(), 0, None, None));
            let duration = match (started_at, completed_at) {
                (Some(s), Some(c)) if c >= s => Some(c - s),
                _ => None,
            };
            WorkflowStepTimingView {
                node_execution_id,
                step_name: node_name,
                run_index: attempt,
                started_at_ms: started_at.map(seconds_to_ms),
                completed_at_ms: completed_at.map(seconds_to_ms),
                duration_ms: duration.map(seconds_to_ms),
            }
        })
        .collect()
}

/// spec issues-1023: frontend へ返す event 列の view 型。
///
/// `WorkflowEvent` (domain) は engine 内の正本で timestamp が秒単位 f64。
/// frontend / 観測経路では ms 単位に揃えたいが、同じ `WorkflowEvent` 型のまま
/// 単位を変えると単一の `timestamp` フィールドが「秒 / ms」の二重意味を持ち、
/// 経路間の混乱・取り違えが生まれる。本 view 型は「ms 単位の timestamp」を
/// 型名・フィールド名（`timestamp_ms` / `requested_at_ms`）で明示し、秒/ms の
/// 二重意味を構造的に排除する。serialize 結果は camelCase（`timestampMs` 等）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "event")]
#[cfg(test)]
pub enum WorkflowEventView {
    RunStarted {
        run_id: String,
        workflow_name: String,
        workflow_file_stem: String,
        worktree_path: String,
        workflow_definition: Workflow,
        request: String,
        #[serde(rename = "timestampMs")]
        timestamp_ms: f64,
    },
    NodeStarted {
        run_id: String,
        workflow_name: String,
        node_execution_id: String,
        node_name: String,
        kind: NodeKindName,
        attempt: u32,
        #[serde(skip_serializing_if = "Option::is_none")]
        fanout_parent: Option<FanoutParentRef>,
        #[serde(rename = "timestampMs")]
        timestamp_ms: f64,
    },
    SessionAttached {
        run_id: String,
        workflow_name: String,
        node_execution_id: String,
        node_name: String,
        session_id: String,
        #[serde(rename = "timestampMs")]
        timestamp_ms: f64,
    },
    WorkflowStallObserved {
        run_id: String,
        workflow_name: String,
        chat_session_id: String,
        step_name: String,
        run_index: u32,
        turn_phase: String,
        idle_secs: u64,
        signal_count: u32,
        cap_reached: bool,
        #[serde(rename = "timestampMs")]
        timestamp_ms: f64,
    },
    WorkflowStallCleared {
        run_id: String,
        workflow_name: String,
        chat_session_id: String,
        #[serde(rename = "timestampMs")]
        timestamp_ms: f64,
    },
    NodeCompleted {
        run_id: String,
        workflow_name: String,
        node_execution_id: String,
        node_name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        result: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        token_usage: Option<EventTokenUsage>,
        #[serde(skip_serializing_if = "Option::is_none")]
        structured_output: Option<serde_json::Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        run_index: Option<u32>,
        #[serde(rename = "timestampMs")]
        timestamp_ms: f64,
    },
    NodeFailed {
        run_id: String,
        workflow_name: String,
        node_execution_id: String,
        node_name: String,
        reason: String,
        failure_kind: WorkflowStepFailureKind,
        #[serde(skip_serializing_if = "Option::is_none")]
        retry_count: Option<u32>,
        #[serde(rename = "timestampMs")]
        timestamp_ms: f64,
    },
    ApprovalRequested {
        run_id: String,
        workflow_name: String,
        node_execution_id: String,
        node_name: String,
        #[serde(rename = "timestampMs")]
        timestamp_ms: f64,
    },
    ApprovalResolved {
        run_id: String,
        workflow_name: String,
        node_execution_id: String,
        node_name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        comment: Option<String>,
        #[serde(rename = "timestampMs")]
        timestamp_ms: f64,
    },
    RunCompleted {
        run_id: String,
        workflow_name: String,
        total_token_usage: TokenUsage,
        #[serde(rename = "timestampMs")]
        timestamp_ms: f64,
    },
    RunFailed {
        run_id: String,
        workflow_name: String,
        reason: String,
        failure_kind: WorkflowStepFailureKind,
        #[serde(skip_serializing_if = "Option::is_none")]
        retry_count: Option<u32>,
        #[serde(rename = "timestampMs")]
        timestamp_ms: f64,
    },
    RunAborted {
        run_id: String,
        workflow_name: String,
        #[serde(rename = "timestampMs")]
        timestamp_ms: f64,
    },
    RunInterrupted {
        run_id: String,
        workflow_name: String,
        reason: String,
        #[serde(rename = "timestampMs")]
        timestamp_ms: f64,
    },
    OutputCollected {
        run_id: String,
        workflow_name: String,
        node_name: String,
        node_outputs: Vec<CollectedOutputEntry>,
        reduce_strategy: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        reduce_result: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        reduce_structured_output: Option<serde_json::Value>,
        #[serde(rename = "timestampMs")]
        timestamp_ms: f64,
    },
    ContractRepairRequested {
        run_id: String,
        workflow_name: String,
        node_name: String,
        run_index: u32,
        #[serde(skip_serializing_if = "Option::is_none")]
        request_id: Option<String>,
        attempt: u32,
        violation_reason: String,
        #[serde(rename = "timestampMs")]
        timestamp_ms: f64,
    },
    CliMutationRequested {
        run_id: String,
        workflow_name: String,
        request_id: String,
        request: CliMutationRequestRecord,
        #[serde(rename = "requestedAtMs")]
        requested_at_ms: f64,
        #[serde(rename = "timestampMs")]
        timestamp_ms: f64,
    },
    ArtifactProduced {
        run_id: String,
        workflow_name: String,
        node_execution_id: String,
        node_name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        contract: Option<String>,
        value: serde_json::Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        request_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none", rename = "submittedAtMs")]
        submitted_at_ms: Option<f64>,
        #[serde(rename = "timestampMs")]
        timestamp_ms: f64,
    },
    /// [06] / [08] engine が拒否した CLI mutation 要求の事実（5-3 / 5-4 修正）。
    CliMutationRejected {
        run_id: String,
        workflow_name: String,
        request_id: String,
        request: CliMutationRequestRecord,
        reason: CliMutationRejectionReason,
        message: String,
        #[serde(rename = "requestedAtMs")]
        requested_at_ms: f64,
        #[serde(rename = "timestampMs")]
        timestamp_ms: f64,
    },
}

#[cfg(test)]
impl From<WorkflowEvent> for WorkflowEventView {
    fn from(event: WorkflowEvent) -> Self {
        match event {
            WorkflowEvent::RunStarted {
                run_id,
                workflow_name,
                workflow_file_stem,
                worktree_path,
                workflow_definition,
                request,
                timestamp,
            } => WorkflowEventView::RunStarted {
                run_id,
                workflow_name,
                workflow_file_stem,
                worktree_path,
                workflow_definition,
                request,
                timestamp_ms: seconds_to_ms(timestamp),
            },
            WorkflowEvent::NodeStarted {
                run_id,
                workflow_name,
                node_execution_id,
                node_name,
                kind,
                attempt,
                fanout_parent,
                timestamp,
            } => WorkflowEventView::NodeStarted {
                run_id,
                workflow_name,
                node_execution_id,
                node_name,
                kind,
                attempt,
                fanout_parent,
                timestamp_ms: seconds_to_ms(timestamp),
            },
            WorkflowEvent::SessionAttached {
                run_id,
                workflow_name,
                node_execution_id,
                node_name,
                session_id,
                timestamp,
            } => WorkflowEventView::SessionAttached {
                run_id,
                workflow_name,
                node_execution_id,
                node_name,
                session_id,
                timestamp_ms: seconds_to_ms(timestamp),
            },
            WorkflowEvent::WorkflowStallObserved {
                run_id,
                workflow_name,
                chat_session_id,
                step_name,
                run_index,
                turn_phase,
                idle_secs,
                signal_count,
                cap_reached,
                timestamp,
            } => WorkflowEventView::WorkflowStallObserved {
                run_id,
                workflow_name,
                chat_session_id,
                step_name,
                run_index,
                turn_phase,
                idle_secs,
                signal_count,
                cap_reached,
                timestamp_ms: seconds_to_ms(timestamp),
            },
            WorkflowEvent::WorkflowStallCleared {
                run_id,
                workflow_name,
                chat_session_id,
                timestamp,
            } => WorkflowEventView::WorkflowStallCleared {
                run_id,
                workflow_name,
                chat_session_id,
                timestamp_ms: seconds_to_ms(timestamp),
            },
            WorkflowEvent::NodeCompleted {
                run_id,
                workflow_name,
                node_execution_id,
                node_name,
                result,
                session_id,
                token_usage,
                structured_output,
                run_index,
                timestamp,
            } => WorkflowEventView::NodeCompleted {
                run_id,
                workflow_name,
                node_execution_id,
                node_name,
                result,
                session_id,
                token_usage,
                structured_output,
                run_index,
                timestamp_ms: seconds_to_ms(timestamp),
            },
            WorkflowEvent::NodeFailed {
                run_id,
                workflow_name,
                node_execution_id,
                node_name,
                reason,
                failure_kind,
                retry_count,
                timestamp,
            } => WorkflowEventView::NodeFailed {
                run_id,
                workflow_name,
                node_execution_id,
                node_name,
                reason,
                failure_kind,
                retry_count,
                timestamp_ms: seconds_to_ms(timestamp),
            },
            WorkflowEvent::ApprovalRequested {
                run_id,
                workflow_name,
                node_execution_id,
                node_name,
                timestamp,
            } => WorkflowEventView::ApprovalRequested {
                run_id,
                workflow_name,
                node_execution_id,
                node_name,
                timestamp_ms: seconds_to_ms(timestamp),
            },
            WorkflowEvent::ApprovalResolved {
                run_id,
                workflow_name,
                node_execution_id,
                node_name,
                comment,
                timestamp,
            } => WorkflowEventView::ApprovalResolved {
                run_id,
                workflow_name,
                node_execution_id,
                node_name,
                comment,
                timestamp_ms: seconds_to_ms(timestamp),
            },
            WorkflowEvent::RunCompleted {
                run_id,
                workflow_name,
                total_token_usage,
                timestamp,
            } => WorkflowEventView::RunCompleted {
                run_id,
                workflow_name,
                total_token_usage,
                timestamp_ms: seconds_to_ms(timestamp),
            },
            WorkflowEvent::RunFailed {
                run_id,
                workflow_name,
                reason,
                failure_kind,
                retry_count,
                timestamp,
            } => WorkflowEventView::RunFailed {
                run_id,
                workflow_name,
                reason,
                failure_kind,
                retry_count,
                timestamp_ms: seconds_to_ms(timestamp),
            },
            WorkflowEvent::RunAborted {
                run_id,
                workflow_name,
                timestamp,
                ..
            } => WorkflowEventView::RunAborted {
                run_id,
                workflow_name,
                timestamp_ms: seconds_to_ms(timestamp),
            },
            WorkflowEvent::RunInterrupted {
                run_id,
                workflow_name,
                reason,
                timestamp,
            } => WorkflowEventView::RunInterrupted {
                run_id,
                workflow_name,
                reason,
                timestamp_ms: seconds_to_ms(timestamp),
            },
            WorkflowEvent::OutputCollected {
                run_id,
                workflow_name,
                node_name,
                node_outputs,
                reduce_strategy,
                reduce_result,
                reduce_structured_output,
                timestamp,
            } => WorkflowEventView::OutputCollected {
                run_id,
                workflow_name,
                node_name,
                node_outputs,
                reduce_strategy,
                reduce_result,
                reduce_structured_output,
                timestamp_ms: seconds_to_ms(timestamp),
            },
            WorkflowEvent::ContractRepairRequested {
                run_id,
                workflow_name,
                node_name,
                run_index,
                request_id,
                attempt,
                violation_reason,
                timestamp,
            } => WorkflowEventView::ContractRepairRequested {
                run_id,
                workflow_name,
                node_name,
                run_index,
                request_id,
                attempt,
                violation_reason,
                timestamp_ms: seconds_to_ms(timestamp),
            },
            WorkflowEvent::CliMutationRequested {
                run_id,
                workflow_name,
                request_id,
                request,
                requested_at,
                timestamp,
            } => WorkflowEventView::CliMutationRequested {
                run_id,
                workflow_name,
                request_id,
                request,
                requested_at_ms: seconds_to_ms(requested_at),
                timestamp_ms: seconds_to_ms(timestamp),
            },
            WorkflowEvent::ArtifactProduced {
                run_id,
                workflow_name,
                node_execution_id,
                node_name,
                contract,
                value,
                request_id,
                submitted_at,
                timestamp,
            } => WorkflowEventView::ArtifactProduced {
                run_id,
                workflow_name,
                node_execution_id,
                node_name,
                contract,
                value,
                request_id,
                submitted_at_ms: submitted_at.map(seconds_to_ms),
                timestamp_ms: seconds_to_ms(timestamp),
            },
            WorkflowEvent::CliMutationRejected {
                run_id,
                workflow_name,
                request_id,
                request,
                reason,
                message,
                requested_at,
                timestamp,
            } => WorkflowEventView::CliMutationRejected {
                run_id,
                workflow_name,
                request_id,
                request,
                reason,
                message,
                requested_at_ms: seconds_to_ms(requested_at),
                timestamp_ms: seconds_to_ms(timestamp),
            },
        }
    }
}

/// spec issues-1023: event log を frontend に返す境界。秒単位の domain `WorkflowEvent`
/// を ms 単位の `WorkflowEventView` に変換する。
#[cfg(test)]
pub(crate) fn events_with_ms_timestamps(events: Vec<WorkflowEvent>) -> Vec<WorkflowEventView> {
    events.into_iter().map(WorkflowEventView::from).collect()
}

/// spec issues-1023: timeline 上で選択した step（node 実行）の入出力・遷移結果・
/// 所要時間を 1 つの View にまとめた projection。frontend は `WorkflowState` を
/// 再走査せず、`worktree_path + run_id + node_name + run_index` を渡すだけで
/// この型を受け取る境界。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowStepDetailView {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_execution_id: Option<String>,
    pub step_name: String,
    pub node_type: String,
    pub run_index: u32,
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub structured_output: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_usage: Option<EventTokenUsage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<f64>,
    /// 入力 facts: node 定義に静的に含まれる instruction と、当該 step に対する
    /// 直前 step（parallel parent / 直前 step_history entry）の structured_output。
    pub input: WorkflowStepInputView,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowStepInputView {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instruction: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub knowledge: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_step_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_step_structured_output: Option<serde_json::Value>,
}

fn node_input_from_definition(
    workflow: &Workflow,
    node_name: &str,
) -> (Option<&'static str>, WorkflowStepInputView) {
    // top-level node
    if let Some(node) = workflow.nodes.iter().find(|n| n.name == node_name) {
        let facets = node.session().map(|session| &session.facets);
        let mut view = WorkflowStepInputView {
            instruction: facets.and_then(|facets| facets.instruction.clone()),
            policy: facets.and_then(|facets| facets.policy.clone()),
            knowledge: facets.and_then(|facets| facets.knowledge.clone()),
            artifact: node.artifact.clone(),
            input: node.input.clone(),
            ..WorkflowStepInputView::default()
        };
        // fanout child は top-level NodeDefinition だが、入力の親は宣言上の直前 node
        // ではなく参照元 fanout。通常 node だけ従来どおり直前 node を用いる。
        if let Some(parent) = workflow.nodes.iter().find(|candidate| {
            candidate
                .fanout()
                .is_some_and(|fanout| fanout.child.iter().any(|child| child == node_name))
        }) {
            view.previous_step_name = Some(parent.name.clone());
        } else if let Some(idx) = workflow.nodes.iter().position(|n| n.name == node_name) {
            if idx > 0 {
                view.previous_step_name = Some(workflow.nodes[idx - 1].name.clone());
            }
        }
        return (Some(node.kind_name().as_str()), view);
    }
    (None, WorkflowStepInputView::default())
}

/// `WorkflowState` 再構築結果と event 列を組み合わせて、選択 step の詳細 View を
/// 返す。`run_index` が None の場合は履歴中の最新エントリ（最大 run_index）に解決する。
/// 該当 step が見つからない場合は `None` を返す。
pub(crate) fn compute_step_detail(
    state: &WorkflowState,
    events: &[WorkflowEvent],
    node_name: &str,
    run_index: Option<u32>,
) -> Option<WorkflowStepDetailView> {
    let (node_type_str, mut input_view) =
        node_input_from_definition(&state.workflow_definition, node_name);

    // history (top-level / parallel parent) から探す。
    // spec issues-1023: run_index が Some の場合は厳密一致のみ許可する。loop / retry
    // 経路で別 run_index の履歴へフォールバックすると、別実行回の detail が返り、
    // 選択 step の事実列が汚染されるため、不一致なら history_entry は None を返す。
    let mut history_match = state
        .step_history
        .iter()
        .filter(|e| e.step_name == node_name)
        .collect::<Vec<_>>();
    history_match.sort_by_key(|e| e.run_index);
    let history_entry = match run_index {
        Some(ri) => history_match.iter().find(|e| e.run_index == ri).copied(),
        None => history_match.last().copied(),
    };

    // spec issues-1023: 直前 step の structured_output は、選択した history_entry の
    // 位置「以前」に発生した previous_step_name の最新履歴を引き当てる。
    // step_history は chronological 順なので、選択 entry の index より前で
    // previous_step_name に一致する最新の entry を探す。選択中 step が未到達
    // （history_entry が None）の場合は previous_step_name の最新を引く。
    if let Some(prev_name) = input_view.previous_step_name.clone() {
        let cutoff_idx = history_entry.and_then(|entry| {
            state
                .step_history
                .iter()
                .position(|h| std::ptr::eq(h, entry))
        });
        let prev_entry = match cutoff_idx {
            Some(idx) => state.step_history[..idx]
                .iter()
                .rev()
                .find(|h| h.step_name == prev_name),
            None => state
                .step_history
                .iter()
                .rev()
                .find(|h| h.step_name == prev_name),
        };
        if let Some(prev) = prev_entry {
            input_view.previous_step_structured_output = prev.structured_output.clone();
        }
    }

    // parallel child の結果は parent の childOutputs か step_outputs 経由で取れる。
    let timings = compute_step_timings(events);
    let resolved_run_index = run_index
        .or_else(|| history_entry.map(|h| h.run_index))
        .unwrap_or_else(|| {
            state
                .step_execution_counts
                .get(node_name)
                .copied()
                .unwrap_or(0)
        });
    // spec issues-1023: timing は (step_name, run_index) 厳密一致のみとし、別 run の
    // timestamp を拾わない。history_entry が Some の場合 resolved_run_index は
    // entry.run_index と一致するため、history detail でも別 run の timing を引かない。
    let timing = timings
        .iter()
        .find(|t| t.step_name == node_name && t.run_index == resolved_run_index);
    let matching_execution = state.node_executions.iter().rev().find(|execution| {
        execution.node_name == node_name
            && (run_index.is_none() || run_index == Some(execution.attempt))
    });
    let timing = matching_execution
        .and_then(|execution| {
            timings
                .iter()
                .find(|timing| timing.node_execution_id == execution.id)
        })
        .or(timing);

    if let Some(entry) = history_entry {
        // history detail は実行回 (run_index) 単位の表示。state.step_states は
        // step_name 単位の最新状態しか保持しないため、過去 run を開いた際に
        // 最新 run の state で上書きされないよう entry.state を使う。
        return Some(WorkflowStepDetailView {
            node_execution_id: matching_execution.map(|execution| execution.id.clone()),
            step_name: node_name.to_string(),
            node_type: node_type_str.unwrap_or("unknown").to_string(),
            run_index: entry.run_index,
            state: entry.state.clone(),
            session_id: entry.session_id.clone(),
            result: entry.result.clone(),
            structured_output: entry.structured_output.clone(),
            token_usage: entry.token_usage.clone(),
            started_at_ms: timing.and_then(|t| t.started_at_ms),
            completed_at_ms: timing
                .and_then(|t| t.completed_at_ms)
                .or(Some(seconds_to_ms(entry.completed_at))),
            duration_ms: timing.and_then(|t| t.duration_ms),
            input: input_view,
        });
    }

    // fanout child を含む NodeExecution read model。
    for execution in state.node_executions.iter().rev() {
        if execution.node_name == node_name
            && (run_index.is_none() || run_index == Some(execution.attempt))
        {
            let timing = timings
                .iter()
                .find(|timing| timing.node_execution_id == execution.id);
            return Some(WorkflowStepDetailView {
                node_execution_id: Some(execution.id.clone()),
                step_name: node_name.to_string(),
                node_type: execution.kind.as_str().to_string(),
                run_index: execution.attempt,
                state: execution.status.as_str().to_string(),
                session_id: execution.session_id.clone(),
                result: None,
                structured_output: execution.artifact.clone(),
                token_usage: execution.token_usage.clone(),
                started_at_ms: timing.and_then(|t| t.started_at_ms),
                completed_at_ms: timing
                    .and_then(|t| t.completed_at_ms)
                    .or(execution.completed_at.map(seconds_to_ms)),
                duration_ms: timing.and_then(|t| t.duration_ms),
                input: input_view,
            });
        }
    }

    // current step（running / waiting_approval）
    if state.current_step_name == node_name {
        let output = state.step_outputs.get(node_name);
        let state_str = state
            .step_states
            .get(node_name)
            .cloned()
            .unwrap_or_else(|| state.state.as_str().to_string());
        return Some(WorkflowStepDetailView {
            node_execution_id: matching_execution.map(|execution| execution.id.clone()),
            step_name: node_name.to_string(),
            node_type: node_type_str.unwrap_or("unknown").to_string(),
            run_index: state
                .step_execution_counts
                .get(node_name)
                .copied()
                .unwrap_or(0),
            state: state_str,
            session_id: state.current_session_id.clone(),
            result: output.and_then(|o| o.result.clone()),
            structured_output: output.and_then(|o| o.structured_output.clone()),
            token_usage: output.and_then(|o| o.token_usage.clone()),
            started_at_ms: timing.and_then(|t| t.started_at_ms),
            completed_at_ms: timing.and_then(|t| t.completed_at_ms),
            duration_ms: timing.and_then(|t| t.duration_ms),
            input: input_view,
        });
    }

    // 上記いずれにも該当しない既知 node は pending 扱い（input のみ返す）。
    if node_type_str.is_some() {
        return Some(WorkflowStepDetailView {
            node_execution_id: None,
            step_name: node_name.to_string(),
            node_type: node_type_str.unwrap_or("unknown").to_string(),
            run_index: 0,
            state: state
                .step_states
                .get(node_name)
                .cloned()
                .unwrap_or_else(|| STEP_STATE_PENDING.to_string()),
            session_id: None,
            result: None,
            structured_output: None,
            token_usage: None,
            started_at_ms: None,
            completed_at_ms: None,
            duration_ms: None,
            input: input_view,
        });
    }
    None
}

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
/// 本関数は workflow runtime 内（controller command / event log 等）
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
    let domain_schemas = workflow_schemas_to_domain(&workflow.schemas);

    let mut started_at = 0.0;
    let mut updated_at = 0.0;
    let mut step_history: Vec<StepHistoryEntry> = Vec::new();
    let mut step_execution_counts: HashMap<String, u32> = HashMap::new();
    let mut step_outputs: HashMap<String, StepOutput> = HashMap::new();
    let mut node_executions: Vec<NodeExecution> = Vec::new();
    let mut total_token_usage = TokenUsage::default();
    let mut exec_state = WorkflowExecutionState::Running;
    let mut current_step_name = workflow
        .nodes
        .first()
        .map(|s| s.name.clone())
        .unwrap_or_default();
    let mut current_step_index = 0usize;
    let mut current_session_id: Option<String> = None;
    let mut workflow_name = String::new();
    let mut stall_observations: Vec<WorkflowStallObservation> = Vec::new();

    for event in events {
        match event {
            WorkflowEvent::RunStarted {
                timestamp,
                workflow_name: wn,
                request,
                ..
            } => {
                started_at = *timestamp;
                updated_at = *timestamp;
                workflow_name = wn.clone();
                step_outputs.insert(
                    crate::domain::workflow::services::reference::REQUEST_ARTIFACT.to_string(),
                    crate::adaptor::gateway::workflow::prompt_rendering::request_step_output(
                        request, *timestamp,
                    ),
                );
            }
            WorkflowEvent::NodeStarted {
                node_execution_id,
                node_name,
                kind,
                attempt,
                fanout_parent,
                timestamp,
                ..
            } => {
                if node_executions
                    .iter()
                    .any(|execution| execution.id == *node_execution_id)
                {
                    return Err(format!(
                        "duplicate NodeStarted node_execution_id '{node_execution_id}'"
                    ));
                }
                node_executions.push(NodeExecution {
                    id: node_execution_id.clone(),
                    execution_id: run_id.to_string(),
                    node_name: node_name.clone(),
                    kind: *kind,
                    attempt: *attempt,
                    status: NodeExecutionStatus::Running,
                    session_id: None,
                    artifact: None,
                    token_usage: None,
                    failure: None,
                    fanout_parent: fanout_parent.clone(),
                    started_at: *timestamp,
                    completed_at: None,
                });
                step_execution_counts
                    .entry(node_name.clone())
                    .and_modify(|count| *count = (*count).max(*attempt))
                    .or_insert(*attempt);

                // fanout child は通常 NodeExecution だが workflow の current node は親のまま。
                if fanout_parent.is_none() {
                    current_step_name = node_name.clone();
                    current_step_index = workflow
                        .nodes
                        .iter()
                        .position(|s| s.name == *node_name)
                        .unwrap_or(0);
                    current_session_id = None;
                    stall_observations.clear();
                    if matches!(exec_state, WorkflowExecutionState::WaitingApproval) {
                        exec_state = WorkflowExecutionState::Running;
                    }
                }
                updated_at = *timestamp;
            }
            WorkflowEvent::SessionAttached {
                node_execution_id,
                node_name,
                session_id,
                timestamp,
                ..
            } => {
                let execution = node_executions
                    .iter_mut()
                    .find(|execution| execution.id == *node_execution_id)
                    .ok_or_else(|| {
                        format!(
                            "SessionAttached references unknown node_execution_id '{node_execution_id}'"
                        )
                    })?;
                if execution.node_name != *node_name {
                    return Err(format!(
                        "SessionAttached node mismatch for node_execution_id '{node_execution_id}': event='{node_name}', execution='{}'",
                        execution.node_name
                    ));
                }
                execution.session_id = Some(session_id.clone());
                if execution.fanout_parent.is_none() {
                    current_step_name = node_name.clone();
                    current_step_index = workflow
                        .nodes
                        .iter()
                        .position(|s| s.name == *node_name)
                        .unwrap_or(0);
                    current_session_id = Some(session_id.clone());
                    if matches!(exec_state, WorkflowExecutionState::WaitingApproval) {
                        exec_state = WorkflowExecutionState::Running;
                    }
                }
                updated_at = *timestamp;
            }
            WorkflowEvent::WorkflowStallObserved {
                chat_session_id,
                step_name,
                run_index,
                turn_phase,
                idle_secs,
                signal_count,
                cap_reached,
                timestamp,
                ..
            } => {
                stall_observations.retain(|observation| observation.session_id != *chat_session_id);
                stall_observations.push(WorkflowStallObservation {
                    session_id: chat_session_id.clone(),
                    step_name: step_name.clone(),
                    run_index: *run_index,
                    turn_phase: turn_phase.clone(),
                    idle_secs: *idle_secs,
                    signal_count: *signal_count,
                    cap_reached: *cap_reached,
                    observed_at: *timestamp,
                });
                updated_at = *timestamp;
            }
            WorkflowEvent::WorkflowStallCleared {
                chat_session_id,
                timestamp,
                ..
            } => {
                stall_observations.retain(|observation| observation.session_id != *chat_session_id);
                updated_at = *timestamp;
            }
            WorkflowEvent::NodeCompleted {
                node_execution_id,
                node_name,
                result,
                session_id,
                token_usage,
                structured_output,
                run_index,
                timestamp,
                ..
            } => {
                let execution = node_executions
                    .iter_mut()
                    .find(|execution| execution.id == *node_execution_id)
                    .ok_or_else(|| {
                        format!(
                            "NodeCompleted references unknown node_execution_id '{node_execution_id}'"
                        )
                    })?;
                if execution.node_name != *node_name {
                    return Err(format!(
                        "NodeCompleted node mismatch for node_execution_id '{node_execution_id}': event='{node_name}', execution='{}'",
                        execution.node_name
                    ));
                }
                execution.status = NodeExecutionStatus::Succeeded;
                execution.session_id = session_id.clone().or_else(|| execution.session_id.clone());
                execution.token_usage = token_usage.clone();
                execution.completed_at = Some(*timestamp);
                let is_fanout_child = execution.fanout_parent.is_some();
                let ri = run_index
                    .unwrap_or_else(|| step_execution_counts.get(node_name).copied().unwrap_or(0));
                if !is_fanout_child {
                    let completed_entry = StepHistoryEntry {
                        step_name: node_name.clone(),
                        completed_at: *timestamp,
                        result: result.clone(),
                        session_id: session_id.clone(),
                        token_usage: token_usage.clone(),
                        structured_output: structured_output.clone(),
                        run_index: ri,
                        child_outputs: None,
                        state: crate::adaptor::gateway::workflow::state::default_step_entry_state(),
                    };
                    if let Some(existing) = step_history
                        .last_mut()
                        .filter(|entry| entry.step_name == *node_name && entry.run_index == ri)
                    {
                        existing.completed_at = completed_entry.completed_at;
                        existing.result = completed_entry.result.or(existing.result.take());
                        existing.session_id =
                            completed_entry.session_id.or(existing.session_id.take());
                        existing.token_usage =
                            completed_entry.token_usage.or(existing.token_usage.take());
                        existing.structured_output = completed_entry
                            .structured_output
                            .or(existing.structured_output.take());
                        existing.state = completed_entry.state;
                    } else {
                        step_history.push(completed_entry);
                    }
                    let prior_output = step_outputs.get(node_name).cloned();
                    let merged_structured_output = structured_output.clone().or_else(|| {
                        prior_output
                            .as_ref()
                            .and_then(|p| p.structured_output.clone())
                    });
                    if merged_structured_output.is_some() || prior_output.is_some() {
                        step_outputs.insert(
                            node_name.clone(),
                            StepOutput {
                                step_name: node_name.clone(),
                                run_index: ri,
                                session_id: session_id.clone().or_else(|| {
                                    prior_output.as_ref().and_then(|p| p.session_id.clone())
                                }),
                                result: result.clone().or_else(|| {
                                    prior_output.as_ref().and_then(|p| p.result.clone())
                                }),
                                structured_output: merged_structured_output,
                                artifact_contract: prior_output
                                    .as_ref()
                                    .and_then(|p| p.artifact_contract.clone()),
                                token_usage: token_usage.clone().or_else(|| {
                                    prior_output.as_ref().and_then(|p| p.token_usage.clone())
                                }),
                                completed_at: *timestamp,
                            },
                        );
                    }
                }
                // fanout child usage is folded into the parent fanout NodeCompleted usage.
                // Counting both lifecycle events would double the run total on replay.
                if !is_fanout_child {
                    if let Some(ref usage) = token_usage {
                        total_token_usage.add(usage);
                    }
                }
                if !is_fanout_child {
                    current_session_id = None;
                    stall_observations.clear();
                } else if let Some(session_id) = session_id {
                    stall_observations.retain(|observation| observation.session_id != *session_id);
                }
                updated_at = *timestamp;
            }
            WorkflowEvent::NodeFailed {
                node_execution_id,
                node_name,
                reason,
                failure_kind,
                retry_count,
                timestamp,
                ..
            } => {
                let execution = node_executions
                    .iter_mut()
                    .find(|execution| execution.id == *node_execution_id)
                    .ok_or_else(|| {
                        format!(
                            "NodeFailed references unknown node_execution_id '{node_execution_id}'"
                        )
                    })?;
                if execution.node_name != *node_name {
                    return Err(format!(
                        "NodeFailed node mismatch for node_execution_id '{node_execution_id}': event='{node_name}', execution='{}'",
                        execution.node_name
                    ));
                }
                execution.status = NodeExecutionStatus::Failed;
                execution.failure = Some(NodeExecutionFailure {
                    reason: reason.clone(),
                    kind: *failure_kind,
                });
                execution.completed_at = Some(*timestamp);
                let is_fanout_child = execution.fanout_parent.is_some();
                let failed_session_id = execution.session_id.clone();
                if !is_fanout_child {
                    exec_state = WorkflowExecutionState::Failed {
                        reason: reason.clone(),
                        kind: *failure_kind,
                        retry_count: *retry_count,
                    };
                    current_session_id = None;
                    stall_observations.clear();
                } else if let Some(session_id) = failed_session_id {
                    stall_observations.retain(|observation| observation.session_id != session_id);
                }
                updated_at = *timestamp;
            }
            WorkflowEvent::ApprovalRequested {
                node_execution_id,
                node_name,
                timestamp,
                ..
            } => {
                let execution = node_executions
                    .iter_mut()
                    .find(|execution| execution.id == *node_execution_id)
                    .ok_or_else(|| {
                        format!(
                            "ApprovalRequested references unknown node_execution_id '{node_execution_id}'"
                        )
                    })?;
                if execution.node_name != *node_name {
                    return Err(format!(
                        "ApprovalRequested node mismatch for node_execution_id '{node_execution_id}': event='{node_name}', execution='{}'",
                        execution.node_name
                    ));
                }
                execution.status = NodeExecutionStatus::WaitingApproval;
                let is_fanout_child = execution.fanout_parent.is_some();
                // fanout child approval is local to that NodeExecution. The workflow remains
                // Running on its parent fanout; only a top-level approval node gates the run.
                if !is_fanout_child {
                    current_step_name = node_name.clone();
                    current_step_index = workflow
                        .nodes
                        .iter()
                        .position(|s| s.name == *node_name)
                        .unwrap_or(current_step_index);
                    exec_state = WorkflowExecutionState::WaitingApproval;
                }
                updated_at = *timestamp;
            }
            WorkflowEvent::ApprovalResolved {
                node_execution_id,
                node_name,
                timestamp,
                ..
            } => {
                let execution = node_executions
                    .iter_mut()
                    .find(|execution| execution.id == *node_execution_id)
                    .ok_or_else(|| {
                        format!(
                            "ApprovalResolved references unknown node_execution_id '{node_execution_id}'"
                        )
                    })?;
                if execution.node_name != *node_name {
                    return Err(format!(
                        "ApprovalResolved node mismatch for node_execution_id '{node_execution_id}': event='{node_name}', execution='{}'",
                        execution.node_name
                    ));
                }
                execution.status = NodeExecutionStatus::Running;
                exec_state = if node_executions.iter().any(|execution| {
                    execution.fanout_parent.is_none()
                        && execution.status == NodeExecutionStatus::WaitingApproval
                }) {
                    WorkflowExecutionState::WaitingApproval
                } else {
                    WorkflowExecutionState::Running
                };
                updated_at = *timestamp;
            }
            WorkflowEvent::RunCompleted {
                total_token_usage: tu,
                timestamp,
                ..
            } => {
                exec_state = WorkflowExecutionState::Completed;
                total_token_usage = tu.clone();
                current_session_id = None;
                stall_observations.clear();
                updated_at = *timestamp;
            }
            WorkflowEvent::RunFailed {
                reason,
                failure_kind,
                retry_count,
                timestamp,
                ..
            } => {
                exec_state = WorkflowExecutionState::Failed {
                    reason: reason.clone(),
                    kind: *failure_kind,
                    retry_count: *retry_count,
                };
                // Fatal fanout event streams record NodeFailed for the failing child and parent,
                // then close the run. Siblings have no dedicated event, so RunFailed closes every
                // execution that is still active while preserving Failed/Succeeded executions.
                for execution in node_executions
                    .iter_mut()
                    .filter(|execution| execution.status.is_active())
                {
                    execution.status = NodeExecutionStatus::Aborted;
                    execution.completed_at = Some(*timestamp);
                }
                current_session_id = None;
                stall_observations.clear();
                updated_at = *timestamp;
            }
            WorkflowEvent::RunAborted {
                aborted_step,
                timestamp,
                ..
            } => {
                exec_state = WorkflowExecutionState::Aborted;
                for execution in node_executions
                    .iter_mut()
                    .filter(|execution| execution.status.is_active())
                {
                    execution.status = NodeExecutionStatus::Aborted;
                    execution.completed_at = Some(*timestamp);
                }

                if let Some(aborted_step) = aborted_step {
                    let aborted_step = step_history_entry_from_run_aborted_snapshot(aborted_step);
                    let already_in_history = step_history.last().is_some_and(|entry| {
                        entry.step_name == aborted_step.step_name
                            && entry.run_index == aborted_step.run_index
                            && entry.state == aborted_step.state
                    });
                    if !already_in_history {
                        step_history.push(aborted_step);
                    }
                } else {
                    // spec issues-1023: 中断時に走っていた current step を
                    // `step_history` に "aborted" 状態として記録する。fanout child の
                    // session / status は NodeExecution read model に保持する。
                    // 旧 RunAborted event は通常 step の current_session_id を持たないため、
                    // 通常 step entry の session_id は None になる。
                    // spec issues-1023: 同名 step の retry を中断したケース（例:
                    // `plan#1` 完了 → `plan#2` 開始 → RunAborted）でも aborted entry を
                    // 残せるよう、step_name に加えて run_index も比較する。step_name のみ
                    // 比較すると `plan#1` の完了 entry に当たって `plan#2` の aborted
                    // entry の追加がスキップされ、session log への到達経路が失われる。
                    let current_run_index = step_execution_counts.get(&current_step_name).copied();
                    let already_in_history = step_history.last().is_some_and(|e| {
                        e.step_name == current_step_name && Some(e.run_index) == current_run_index
                    });
                    let current_started = current_run_index.is_some();

                    if current_started && !already_in_history && !current_step_name.is_empty() {
                        let run_index = step_execution_counts
                            .get(&current_step_name)
                            .copied()
                            .unwrap_or(0);
                        step_history.push(StepHistoryEntry {
                            step_name: current_step_name.clone(),
                            completed_at: *timestamp,
                            result: None,
                            session_id: None,
                            token_usage: None,
                            structured_output: None,
                            run_index,
                            child_outputs: None,
                            state: STEP_STATE_ABORTED.to_string(),
                        });
                    }
                }

                current_session_id = None;
                stall_observations.clear();
                updated_at = *timestamp;
            }
            WorkflowEvent::RunInterrupted { timestamp, .. } => {
                exec_state = WorkflowExecutionState::Interrupted;
                for execution in node_executions
                    .iter_mut()
                    .filter(|execution| execution.status.is_active())
                {
                    execution.status = NodeExecutionStatus::Aborted;
                    execution.completed_at = Some(*timestamp);
                }
                let current_run_index = step_execution_counts.get(&current_step_name).copied();
                let already_in_history = step_history.last().is_some_and(|entry| {
                    entry.step_name == current_step_name
                        && Some(entry.run_index) == current_run_index
                });

                if current_run_index.is_some()
                    && !already_in_history
                    && !current_step_name.is_empty()
                {
                    step_history.push(StepHistoryEntry {
                        step_name: current_step_name.clone(),
                        completed_at: *timestamp,
                        result: None,
                        session_id: current_session_id.clone(),
                        token_usage: None,
                        structured_output: None,
                        run_index: current_run_index.unwrap_or(0),
                        child_outputs: None,
                        state: STEP_STATE_INTERRUPTED.to_string(),
                    });
                }

                current_session_id = None;
                stall_observations.clear();
                updated_at = *timestamp;
            }
            WorkflowEvent::OutputCollected { timestamp, .. } => {
                updated_at = *timestamp;
            }
            WorkflowEvent::ContractRepairRequested { timestamp, .. } => {
                updated_at = *timestamp;
            }
            WorkflowEvent::CliMutationRequested { .. } => {
                // [06] CLI mutation 要求の事実は append-only な観測情報のみで、
                // engine domain state には影響しない。
            }
            WorkflowEvent::ArtifactProduced {
                node_execution_id,
                node_name,
                contract,
                value,
                timestamp,
                ..
            } => {
                let is_fanout_child = node_executions
                    .iter_mut()
                    .find(|execution| execution.id == *node_execution_id)
                    .ok_or_else(|| {
                        format!(
                            "ArtifactProduced references unknown node_execution_id '{node_execution_id}'"
                        )
                    })
                    .and_then(|execution| {
                        if execution.node_name != *node_name {
                            return Err(format!(
                                "ArtifactProduced node mismatch for node_execution_id '{node_execution_id}': event='{node_name}', execution='{}'",
                                execution.node_name
                            ));
                        }
                        execution.artifact = Some(value.clone());
                        Ok(execution.fanout_parent.is_some())
                    })?;
                // [08] CLI / in-process 経由で確定した step output を state に復元する。
                // 後続 step が `input_reference` で経路非依存に参照できる shape に揃える。
                // `result` は engine の live state と同じ値（contract validator の戻り値）
                // を再導出する。これにより live と reload 経路で aggregate 評価が乖離しない。
                if is_fanout_child {
                    // fanout child Artifact は NodeExecution にだけ保持し、node-name map
                    // には載せない。親 fanout の ArtifactProduced(array) が唯一の参照面。
                    updated_at = *timestamp;
                    continue;
                }

                let ri = step_execution_counts.get(node_name).copied().unwrap_or(0);
                let restored_result = if let Some(contract) = contract {
                    match workflow_contract::validate_artifact_value(
                        &domain_schemas,
                        contract,
                        value.clone(),
                    ) {
                        ContractValidationResult::Valid { result, .. } => result,
                        // append-only ログに記録された ArtifactProduced は engine 側で validator を
                        // 通過しているため通常ここには到達しない。validator が将来変更されて
                        // 不適合判定になっても、result を None にして live と同等に振る舞う
                        // （aggregate 評価では match なしになるだけ）。
                        ContractValidationResult::Invalid(_) => None,
                    }
                } else {
                    None
                };
                step_outputs.insert(
                    node_name.clone(),
                    StepOutput {
                        step_name: node_name.clone(),
                        run_index: ri,
                        session_id: None,
                        result: restored_result,
                        structured_output: Some(value.clone()),
                        artifact_contract: contract.clone(),
                        token_usage: None,
                        completed_at: *timestamp,
                    },
                );
                updated_at = *timestamp;
            }
            // [06] / [08] CliMutationRejected は観測経路用の補助履歴であり、
            // engine の workflow state には影響しない（accepted 経路の event のみが
            // 一次表現）。projection では no-op として扱う（5-3 / 5-4 修正）。
            WorkflowEvent::CliMutationRejected { .. } => {}
        }
    }

    let step_states = crate::adaptor::gateway::workflow::state::compute_step_states(
        workflow,
        current_step_index,
        &exec_state,
        &step_history,
    );
    let domain_state = workflow_execution_state_to_domain(&exec_state);
    let current_step = workflow
        .nodes
        .get(current_step_index)
        .map(node_definition_to_domain);
    let approval_operations =
        workflow_projection::approval_operations(&domain_state, current_step.as_ref()).map(
            |operations| ApprovalOperations {
                can_approve: operations.can_approve,
            },
        );

    Ok(Some(WorkflowState {
        execution_id: run_id.to_string(),
        workflow_name,
        state: exec_state,
        current_step_index,
        current_step_name,
        current_session_id,
        total_steps: workflow.nodes.len(),
        step_history,
        step_execution_counts,
        workflow_definition: workflow.clone(),
        total_token_usage,
        step_outputs,
        step_states,
        node_executions,
        approval_operations,
        stall_observations,
        started_at,
        updated_at,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adaptor::gateway::workflow::schema::{
        FacetRefs, FanoutSpec, NodeDefinition, NodeKind, SchemaDef, SessionGate, SessionSpec,
        Workflow,
    };

    fn agent_node(name: &str) -> NodeDefinition {
        NodeDefinition {
            name: name.to_string(),
            kind: session_kind("x"),
            ..NodeDefinition::default()
        }
    }

    fn session_kind(instruction: &str) -> NodeKind {
        session_kind_with_gate(SessionGate::Auto, instruction)
    }

    fn approval_session_kind(instruction: &str) -> NodeKind {
        session_kind_with_gate(SessionGate::Approval, instruction)
    }

    fn session_kind_with_gate(gate: SessionGate, instruction: &str) -> NodeKind {
        NodeKind::Session(SessionSpec {
            gate,
            facets: FacetRefs {
                instruction: Some(instruction.to_string()),
                ..Default::default()
            },
            ..Default::default()
        })
    }

    fn workflow_with_nodes(name: &str, nodes: Vec<&str>) -> Workflow {
        Workflow {
            name: name.to_string(),
            description: String::new(),
            builtin: false,
            schemas: Default::default(),
            nodes: nodes.into_iter().map(agent_node).collect(),
        }
    }

    fn run_started(run_id: &str, workflow: Workflow) -> WorkflowEvent {
        WorkflowEvent::RunStarted {
            run_id: run_id.to_string(),
            workflow_name: workflow.name.clone(),
            workflow_file_stem: workflow.name.clone(),
            worktree_path: "/repo".to_string(),
            request: String::new(),
            workflow_definition: workflow,
            timestamp: 1000.0,
        }
    }

    fn stall_observed(run_id: &str, session_id: &str, step_name: &str) -> WorkflowEvent {
        WorkflowEvent::WorkflowStallObserved {
            run_id: run_id.to_string(),
            workflow_name: "wf".to_string(),
            chat_session_id: session_id.to_string(),
            step_name: step_name.to_string(),
            run_index: 1,
            turn_phase: "streaming".to_string(),
            idle_secs: 181,
            signal_count: 1,
            cap_reached: false,
            timestamp: 1003.0,
        }
    }

    fn stall_cleared(run_id: &str, session_id: &str) -> WorkflowEvent {
        WorkflowEvent::WorkflowStallCleared {
            run_id: run_id.to_string(),
            workflow_name: "wf".to_string(),
            chat_session_id: session_id.to_string(),
            timestamp: 1004.0,
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
            node_execution_id: "ne-a-1".to_string(),
            node_name: "a".to_string(),
            kind: NodeKindName::Session,
            attempt: 1,
            fanout_parent: None,
            timestamp: 1.0,
        }];
        let result = reconstruct_state_from_events("exec-x", &events).unwrap();
        assert!(
            result.is_none(),
            "RunStarted を含まない events 列は復元対象外"
        );
    }

    #[test]
    fn projection_rejects_lifecycle_event_with_unknown_node_execution_id() {
        let run_id = "run-unknown-node-execution";
        let events = vec![
            run_started(run_id, workflow_with_nodes("wf", vec!["implement"])),
            WorkflowEvent::NodeCompleted {
                run_id: run_id.to_string(),
                workflow_name: "wf".to_string(),
                node_execution_id: "missing".to_string(),
                node_name: "implement".to_string(),
                result: Some("done".to_string()),
                session_id: None,
                token_usage: None,
                structured_output: None,
                run_index: Some(1),
                timestamp: 1001.0,
            },
        ];

        let error = reconstruct_state_from_events(run_id, &events).unwrap_err();

        assert!(error.contains("unknown node_execution_id 'missing'"));
    }

    #[test]
    fn projection_run_failed_aborts_active_fanout_siblings() {
        let run_id = "run-fanout-terminal-failure";
        let failure_kind = WorkflowStepFailureKind::InfrastructureCrash;
        let events = vec![
            run_started(
                run_id,
                workflow_with_nodes("wf", vec!["fanout", "child-a", "child-b"]),
            ),
            WorkflowEvent::NodeStarted {
                run_id: run_id.to_string(),
                workflow_name: "wf".to_string(),
                node_execution_id: "ne-parent".to_string(),
                node_name: "fanout".to_string(),
                kind: NodeKindName::Fanout,
                attempt: 1,
                fanout_parent: None,
                timestamp: 1001.0,
            },
            WorkflowEvent::NodeStarted {
                run_id: run_id.to_string(),
                workflow_name: "wf".to_string(),
                node_execution_id: "ne-child-a".to_string(),
                node_name: "child-a".to_string(),
                kind: NodeKindName::Session,
                attempt: 1,
                fanout_parent: Some(FanoutParentRef {
                    parent_node: "fanout".to_string(),
                    parent_attempt: 1,
                    item_index: None,
                    child_index: 0,
                }),
                timestamp: 1002.0,
            },
            WorkflowEvent::NodeStarted {
                run_id: run_id.to_string(),
                workflow_name: "wf".to_string(),
                node_execution_id: "ne-child-b".to_string(),
                node_name: "child-b".to_string(),
                kind: NodeKindName::Session,
                attempt: 1,
                fanout_parent: Some(FanoutParentRef {
                    parent_node: "fanout".to_string(),
                    parent_attempt: 1,
                    item_index: None,
                    child_index: 1,
                }),
                timestamp: 1002.0,
            },
            WorkflowEvent::NodeFailed {
                run_id: run_id.to_string(),
                workflow_name: "wf".to_string(),
                node_execution_id: "ne-child-a".to_string(),
                node_name: "child-a".to_string(),
                reason: "child failed".to_string(),
                failure_kind,
                retry_count: None,
                timestamp: 1003.0,
            },
            WorkflowEvent::NodeFailed {
                run_id: run_id.to_string(),
                workflow_name: "wf".to_string(),
                node_execution_id: "ne-parent".to_string(),
                node_name: "fanout".to_string(),
                reason: "fanout failed".to_string(),
                failure_kind,
                retry_count: None,
                timestamp: 1004.0,
            },
            WorkflowEvent::RunFailed {
                run_id: run_id.to_string(),
                workflow_name: "wf".to_string(),
                reason: "fanout failed".to_string(),
                failure_kind,
                retry_count: None,
                timestamp: 1005.0,
            },
        ];

        let state = reconstruct_state_from_events(run_id, &events)
            .unwrap()
            .unwrap();
        assert!(matches!(state.state, WorkflowExecutionState::Failed { .. }));
        let status = |id: &str| {
            state
                .node_executions
                .iter()
                .find(|execution| execution.id == id)
                .map(|execution| (execution.status, execution.completed_at))
                .expect("node execution must be projected")
        };
        assert_eq!(status("ne-parent").0, NodeExecutionStatus::Failed);
        assert_eq!(status("ne-child-a").0, NodeExecutionStatus::Failed);
        assert_eq!(
            status("ne-child-b"),
            (NodeExecutionStatus::Aborted, Some(1005.0))
        );
        assert!(
            state
                .node_executions
                .iter()
                .all(|execution| !execution.status.is_active()),
            "RunFailed replay must leave no live fanout child"
        );
    }

    #[test]
    fn projection_run_completed_keeps_completed_fanout_child_succeeded_and_leaves_no_active_nodes()
    {
        let run_id = "run-fanout-normal-complete";
        let events = vec![
            run_started(run_id, workflow_with_nodes("wf", vec!["fanout", "child-a"])),
            WorkflowEvent::NodeStarted {
                run_id: run_id.to_string(),
                workflow_name: "wf".to_string(),
                node_execution_id: "ne-parent".to_string(),
                node_name: "fanout".to_string(),
                kind: NodeKindName::Fanout,
                attempt: 1,
                fanout_parent: None,
                timestamp: 1001.0,
            },
            WorkflowEvent::NodeStarted {
                run_id: run_id.to_string(),
                workflow_name: "wf".to_string(),
                node_execution_id: "ne-child-a".to_string(),
                node_name: "child-a".to_string(),
                kind: NodeKindName::Session,
                attempt: 1,
                fanout_parent: Some(FanoutParentRef {
                    parent_node: "fanout".to_string(),
                    parent_attempt: 1,
                    item_index: None,
                    child_index: 0,
                }),
                timestamp: 1002.0,
            },
            WorkflowEvent::NodeCompleted {
                run_id: run_id.to_string(),
                workflow_name: "wf".to_string(),
                node_execution_id: "ne-child-a".to_string(),
                node_name: "child-a".to_string(),
                result: Some("LGTM".to_string()),
                session_id: Some("session-child-a".to_string()),
                token_usage: None,
                structured_output: Some(serde_json::json!({ "verdict": "LGTM" })),
                run_index: Some(1),
                timestamp: 1003.0,
            },
            WorkflowEvent::NodeCompleted {
                run_id: run_id.to_string(),
                workflow_name: "wf".to_string(),
                node_execution_id: "ne-parent".to_string(),
                node_name: "fanout".to_string(),
                result: Some("complete".to_string()),
                session_id: None,
                token_usage: None,
                structured_output: Some(serde_json::json!([{ "verdict": "LGTM" }])),
                run_index: Some(1),
                timestamp: 1004.0,
            },
            WorkflowEvent::RunCompleted {
                run_id: run_id.to_string(),
                workflow_name: "wf".to_string(),
                total_token_usage: TokenUsage::default(),
                timestamp: 1005.0,
            },
        ];

        let state = reconstruct_state_from_events(run_id, &events)
            .unwrap()
            .unwrap();
        let status = |id: &str| {
            state
                .node_executions
                .iter()
                .find(|execution| execution.id == id)
                .map(|execution| execution.status)
                .expect("node execution must be projected")
        };

        assert_eq!(state.state, WorkflowExecutionState::Completed);
        assert_eq!(status("ne-parent"), NodeExecutionStatus::Succeeded);
        assert_eq!(status("ne-child-a"), NodeExecutionStatus::Succeeded);
        assert!(
            state
                .node_executions
                .iter()
                .all(|execution| !execution.status.is_active()),
            "RunCompleted replay must leave no active fanout node execution"
        );
    }

    #[test]
    fn projection_restores_approval_operations_for_approval_gate_waiting_state() {
        let run_id = "exec-approval-ops";
        let mut workflow = workflow_with_nodes("wf", vec!["review"]);
        workflow.nodes[0].kind = approval_session_kind("review the diff");
        let events = vec![
            run_started(run_id, workflow),
            WorkflowEvent::NodeStarted {
                run_id: run_id.to_string(),
                workflow_name: "wf".to_string(),
                node_execution_id: "ne-review-1".to_string(),
                node_name: "review".to_string(),
                kind: NodeKindName::Session,
                attempt: 1,
                fanout_parent: None,
                timestamp: 1000.5,
            },
            WorkflowEvent::ApprovalRequested {
                run_id: run_id.to_string(),
                workflow_name: "wf".to_string(),
                node_execution_id: "ne-review-1".to_string(),
                node_name: "review".to_string(),
                timestamp: 1001.0,
            },
        ];

        let state = reconstruct_state_from_events(run_id, &events)
            .unwrap()
            .unwrap();

        assert_eq!(state.state, WorkflowExecutionState::WaitingApproval);
        assert_eq!(state.current_step_name, "review");
        assert_eq!(
            state
                .approval_operations
                .map(|operations| operations.can_approve),
            Some(true)
        );
    }

    #[test]
    fn projection_does_not_restore_approval_operations_for_non_approval_gate() {
        let run_id = "exec-auto-gate-waiting";
        let workflow = workflow_with_nodes("wf", vec!["review"]);
        let events = vec![
            run_started(run_id, workflow),
            WorkflowEvent::NodeStarted {
                run_id: run_id.to_string(),
                workflow_name: "wf".to_string(),
                node_execution_id: "ne-review-1".to_string(),
                node_name: "review".to_string(),
                kind: NodeKindName::Session,
                attempt: 1,
                fanout_parent: None,
                timestamp: 1000.5,
            },
            WorkflowEvent::ApprovalRequested {
                run_id: run_id.to_string(),
                workflow_name: "wf".to_string(),
                node_execution_id: "ne-review-1".to_string(),
                node_name: "review".to_string(),
                timestamp: 1001.0,
            },
        ];

        let state = reconstruct_state_from_events(run_id, &events)
            .unwrap()
            .unwrap();

        assert_eq!(state.state, WorkflowExecutionState::WaitingApproval);
        assert!(state.approval_operations.is_none());
    }

    #[test]
    fn projection_keeps_workflow_running_when_fanout_child_requests_approval() {
        let run_id = "exec-fanout-approval";
        let mut workflow = workflow_with_nodes("wf", vec!["fanout-review", "review"]);
        workflow.nodes[0].kind = NodeKind::Fanout(FanoutSpec {
            child: vec!["review".to_string()],
            items: None,
            aggregate: None,
        });
        workflow.nodes[1].kind = approval_session_kind("review the diff");
        let events = vec![
            run_started(run_id, workflow),
            WorkflowEvent::NodeStarted {
                run_id: run_id.to_string(),
                workflow_name: "wf".to_string(),
                node_execution_id: "ne-parent".to_string(),
                node_name: "fanout-review".to_string(),
                kind: NodeKindName::Fanout,
                attempt: 1,
                fanout_parent: None,
                timestamp: 1000.5,
            },
            WorkflowEvent::NodeStarted {
                run_id: run_id.to_string(),
                workflow_name: "wf".to_string(),
                node_execution_id: "ne-review-1".to_string(),
                node_name: "review".to_string(),
                kind: NodeKindName::Session,
                attempt: 1,
                fanout_parent: Some(FanoutParentRef {
                    parent_node: "fanout-review".to_string(),
                    parent_attempt: 1,
                    item_index: None,
                    child_index: 0,
                }),
                timestamp: 1000.6,
            },
            WorkflowEvent::ApprovalRequested {
                run_id: run_id.to_string(),
                workflow_name: "wf".to_string(),
                node_execution_id: "ne-review-1".to_string(),
                node_name: "review".to_string(),
                timestamp: 1001.0,
            },
        ];

        let state = reconstruct_state_from_events(run_id, &events)
            .unwrap()
            .unwrap();

        assert_eq!(state.state, WorkflowExecutionState::Running);
        assert_eq!(state.current_step_name, "fanout-review");
        assert!(state.approval_operations.is_none());
        assert_eq!(
            state
                .node_executions
                .iter()
                .find(|execution| execution.id == "ne-parent")
                .unwrap()
                .status,
            NodeExecutionStatus::Running
        );
        assert_eq!(
            state
                .node_executions
                .iter()
                .find(|execution| execution.id == "ne-review-1")
                .unwrap()
                .status,
            NodeExecutionStatus::WaitingApproval
        );
    }

    #[test]
    fn projection_restores_workflow_stall_observations() {
        let run_id = "exec-stall";
        let workflow = workflow_with_nodes("wf", vec!["review"]);
        let events = vec![
            run_started(run_id, workflow),
            WorkflowEvent::NodeStarted {
                run_id: run_id.to_string(),
                workflow_name: "wf".to_string(),
                node_execution_id: "ne-review-1".to_string(),
                node_name: "review".to_string(),
                kind: NodeKindName::Session,
                attempt: 1,
                fanout_parent: None,
                timestamp: 1001.0,
            },
            WorkflowEvent::SessionAttached {
                run_id: run_id.to_string(),
                workflow_name: "wf".to_string(),
                node_execution_id: "ne-review-1".to_string(),
                node_name: "review".to_string(),
                session_id: "session-1".to_string(),
                timestamp: 1002.0,
            },
            WorkflowEvent::WorkflowStallObserved {
                run_id: run_id.to_string(),
                workflow_name: "wf".to_string(),
                chat_session_id: "session-1".to_string(),
                step_name: "review".to_string(),
                run_index: 1,
                turn_phase: "streaming".to_string(),
                idle_secs: 181,
                signal_count: 2,
                cap_reached: false,
                timestamp: 1003.0,
            },
        ];

        let state = reconstruct_state_from_events(run_id, &events)
            .unwrap()
            .unwrap();

        assert_eq!(state.current_session_id.as_deref(), Some("session-1"));
        assert_eq!(state.stall_observations.len(), 1);
        let observation = &state.stall_observations[0];
        assert_eq!(observation.session_id, "session-1");
        assert_eq!(observation.step_name, "review");
        assert_eq!(observation.run_index, 1);
        assert_eq!(observation.turn_phase, "streaming");
        assert_eq!(observation.idle_secs, 181);
        assert_eq!(observation.signal_count, 2);
        assert!(!observation.cap_reached);
        assert_eq!(observation.observed_at, 1003.0);
        assert_eq!(state.updated_at, 1003.0);
    }

    #[test]
    fn projection_replaces_duplicate_workflow_stall_observation_for_session() {
        let run_id = "exec-stall-replace";
        let workflow = workflow_with_nodes("wf", vec!["review"]);
        let events = vec![
            run_started(run_id, workflow),
            WorkflowEvent::NodeStarted {
                run_id: run_id.to_string(),
                workflow_name: "wf".to_string(),
                node_execution_id: "ne-review-1".to_string(),
                node_name: "review".to_string(),
                kind: NodeKindName::Session,
                attempt: 1,
                fanout_parent: None,
                timestamp: 1001.0,
            },
            WorkflowEvent::SessionAttached {
                run_id: run_id.to_string(),
                workflow_name: "wf".to_string(),
                node_execution_id: "ne-review-1".to_string(),
                node_name: "review".to_string(),
                session_id: "session-1".to_string(),
                timestamp: 1002.0,
            },
            stall_observed(run_id, "session-1", "review"),
            WorkflowEvent::WorkflowStallObserved {
                run_id: run_id.to_string(),
                workflow_name: "wf".to_string(),
                chat_session_id: "session-1".to_string(),
                step_name: "review".to_string(),
                run_index: 1,
                turn_phase: "streaming".to_string(),
                idle_secs: 240,
                signal_count: 2,
                cap_reached: true,
                timestamp: 1005.0,
            },
        ];

        let state = reconstruct_state_from_events(run_id, &events)
            .unwrap()
            .unwrap();

        assert_eq!(state.stall_observations.len(), 1);
        let observation = &state.stall_observations[0];
        assert_eq!(observation.session_id, "session-1");
        assert_eq!(observation.idle_secs, 240);
        assert_eq!(observation.signal_count, 2);
        assert!(observation.cap_reached);
        assert_eq!(observation.observed_at, 1005.0);
    }

    #[test]
    fn projection_clears_workflow_stall_observation_on_progress_clear_event() {
        let run_id = "exec-stall-clear";
        let workflow = workflow_with_nodes("wf", vec!["review"]);
        let events = vec![
            run_started(run_id, workflow),
            WorkflowEvent::NodeStarted {
                run_id: run_id.to_string(),
                workflow_name: "wf".to_string(),
                node_execution_id: "ne-review-1".to_string(),
                node_name: "review".to_string(),
                kind: NodeKindName::Session,
                attempt: 1,
                fanout_parent: None,
                timestamp: 1001.0,
            },
            WorkflowEvent::SessionAttached {
                run_id: run_id.to_string(),
                workflow_name: "wf".to_string(),
                node_execution_id: "ne-review-1".to_string(),
                node_name: "review".to_string(),
                session_id: "session-1".to_string(),
                timestamp: 1002.0,
            },
            stall_observed(run_id, "session-1", "review"),
            stall_cleared(run_id, "session-1"),
        ];

        let state = reconstruct_state_from_events(run_id, &events)
            .unwrap()
            .unwrap();

        assert!(state.stall_observations.is_empty());
        assert_eq!(state.current_session_id.as_deref(), Some("session-1"));
        assert_eq!(state.updated_at, 1004.0);
    }

    #[test]
    fn projection_clears_workflow_stall_observations_on_step_completion() {
        let run_id = "exec-stall-complete";
        let workflow = workflow_with_nodes("wf", vec!["review"]);
        let events = vec![
            run_started(run_id, workflow),
            WorkflowEvent::NodeStarted {
                run_id: run_id.to_string(),
                workflow_name: "wf".to_string(),
                node_execution_id: "ne-review-1".to_string(),
                node_name: "review".to_string(),
                kind: NodeKindName::Session,
                attempt: 1,
                fanout_parent: None,
                timestamp: 1001.0,
            },
            WorkflowEvent::SessionAttached {
                run_id: run_id.to_string(),
                workflow_name: "wf".to_string(),
                node_execution_id: "ne-review-1".to_string(),
                node_name: "review".to_string(),
                session_id: "session-1".to_string(),
                timestamp: 1002.0,
            },
            stall_observed(run_id, "session-1", "review"),
            WorkflowEvent::NodeCompleted {
                run_id: run_id.to_string(),
                workflow_name: "wf".to_string(),
                node_execution_id: "ne-review-1".to_string(),
                node_name: "review".to_string(),
                result: Some("ok".to_string()),
                session_id: Some("session-1".to_string()),
                token_usage: None,
                structured_output: None,
                run_index: Some(1),
                timestamp: 1004.0,
            },
        ];

        let state = reconstruct_state_from_events(run_id, &events)
            .unwrap()
            .unwrap();

        assert!(state.stall_observations.is_empty());
        assert_eq!(state.current_session_id, None);
    }

    #[test]
    fn projection_clears_workflow_stall_observations_on_next_step_start() {
        let run_id = "exec-stall-next-step";
        let workflow = workflow_with_nodes("wf", vec!["plan", "review"]);
        let events = vec![
            run_started(run_id, workflow),
            WorkflowEvent::NodeStarted {
                run_id: run_id.to_string(),
                workflow_name: "wf".to_string(),
                node_execution_id: "ne-plan-1".to_string(),
                node_name: "plan".to_string(),
                kind: NodeKindName::Session,
                attempt: 1,
                fanout_parent: None,
                timestamp: 1001.0,
            },
            WorkflowEvent::SessionAttached {
                run_id: run_id.to_string(),
                workflow_name: "wf".to_string(),
                node_execution_id: "ne-plan-1".to_string(),
                node_name: "plan".to_string(),
                session_id: "session-plan".to_string(),
                timestamp: 1002.0,
            },
            stall_observed(run_id, "session-plan", "plan"),
            WorkflowEvent::NodeStarted {
                run_id: run_id.to_string(),
                workflow_name: "wf".to_string(),
                node_execution_id: "ne-review-1".to_string(),
                node_name: "review".to_string(),
                kind: NodeKindName::Session,
                attempt: 1,
                fanout_parent: None,
                timestamp: 1004.0,
            },
        ];

        let state = reconstruct_state_from_events(run_id, &events)
            .unwrap()
            .unwrap();

        assert!(state.stall_observations.is_empty());
        assert_eq!(state.current_step_name, "review");
        assert_eq!(state.current_session_id, None);
    }

    #[test]
    fn projection_clears_workflow_stall_observations_on_run_terminal_event() {
        let run_id = "exec-stall-terminal";
        let workflow = workflow_with_nodes("wf", vec!["review"]);
        let events = vec![
            run_started(run_id, workflow),
            WorkflowEvent::NodeStarted {
                run_id: run_id.to_string(),
                workflow_name: "wf".to_string(),
                node_execution_id: "ne-review-1".to_string(),
                node_name: "review".to_string(),
                kind: NodeKindName::Session,
                attempt: 1,
                fanout_parent: None,
                timestamp: 1001.0,
            },
            WorkflowEvent::SessionAttached {
                run_id: run_id.to_string(),
                workflow_name: "wf".to_string(),
                node_execution_id: "ne-review-1".to_string(),
                node_name: "review".to_string(),
                session_id: "session-1".to_string(),
                timestamp: 1002.0,
            },
            stall_observed(run_id, "session-1", "review"),
            WorkflowEvent::RunCompleted {
                run_id: run_id.to_string(),
                workflow_name: "wf".to_string(),
                total_token_usage: TokenUsage::default(),
                timestamp: 1004.0,
            },
        ];

        let state = reconstruct_state_from_events(run_id, &events)
            .unwrap()
            .unwrap();

        assert!(state.stall_observations.is_empty());
        assert_eq!(state.current_session_id, None);
        assert_eq!(state.state, WorkflowExecutionState::Completed);
    }

    #[test]
    fn projection_clears_only_completed_fanout_child_stall_observation() {
        let run_id = "exec-stall-parallel-child";
        let workflow = workflow_with_nodes("wf", vec!["parallel-review", "review-a", "review-b"]);
        let events = vec![
            run_started(run_id, workflow),
            WorkflowEvent::NodeStarted {
                run_id: run_id.to_string(),
                workflow_name: "wf".to_string(),
                node_execution_id: "ne-parent".to_string(),
                node_name: "parallel-review".to_string(),
                kind: NodeKindName::Fanout,
                attempt: 1,
                fanout_parent: None,
                timestamp: 1001.0,
            },
            WorkflowEvent::NodeStarted {
                run_id: run_id.to_string(),
                workflow_name: "wf".to_string(),
                node_execution_id: "ne-a".to_string(),
                node_name: "review-a".to_string(),
                kind: NodeKindName::Session,
                attempt: 1,
                fanout_parent: Some(FanoutParentRef {
                    parent_node: "parallel-review".to_string(),
                    parent_attempt: 1,
                    item_index: None,
                    child_index: 0,
                }),
                timestamp: 1002.0,
            },
            WorkflowEvent::SessionAttached {
                run_id: run_id.to_string(),
                workflow_name: "wf".to_string(),
                node_execution_id: "ne-a".to_string(),
                node_name: "review-a".to_string(),
                session_id: "session-review-a".to_string(),
                timestamp: 1002.0,
            },
            WorkflowEvent::NodeStarted {
                run_id: run_id.to_string(),
                workflow_name: "wf".to_string(),
                node_execution_id: "ne-b".to_string(),
                node_name: "review-b".to_string(),
                kind: NodeKindName::Session,
                attempt: 1,
                fanout_parent: Some(FanoutParentRef {
                    parent_node: "parallel-review".to_string(),
                    parent_attempt: 1,
                    item_index: None,
                    child_index: 1,
                }),
                timestamp: 1002.0,
            },
            WorkflowEvent::SessionAttached {
                run_id: run_id.to_string(),
                workflow_name: "wf".to_string(),
                node_execution_id: "ne-b".to_string(),
                node_name: "review-b".to_string(),
                session_id: "session-review-b".to_string(),
                timestamp: 1002.0,
            },
            stall_observed(run_id, "session-review-a", "review-a"),
            stall_observed(run_id, "session-review-b", "review-b"),
            WorkflowEvent::NodeCompleted {
                run_id: run_id.to_string(),
                workflow_name: "wf".to_string(),
                node_execution_id: "ne-a".to_string(),
                node_name: "review-a".to_string(),
                result: Some("LGTM".to_string()),
                session_id: Some("session-review-a".to_string()),
                token_usage: None,
                structured_output: None,
                run_index: Some(1),
                timestamp: 1004.0,
            },
        ];

        let state = reconstruct_state_from_events(run_id, &events)
            .unwrap()
            .unwrap();

        assert_eq!(state.stall_observations.len(), 1);
        assert_eq!(state.stall_observations[0].session_id, "session-review-b");
        assert_eq!(
            state.node_executions[1].status,
            NodeExecutionStatus::Succeeded
        );
        assert_eq!(
            state.node_executions[2].status,
            NodeExecutionStatus::Running
        );
    }

    #[test]
    fn projection_restores_current_session_from_node_session_started() {
        let workflow = workflow_with_nodes("wf", vec!["implement"]);
        let events = vec![
            run_started("exec-1", workflow),
            WorkflowEvent::NodeStarted {
                run_id: "exec-1".to_string(),
                workflow_name: "wf".to_string(),
                node_execution_id: "ne-implement-1".to_string(),
                node_name: "implement".to_string(),
                kind: NodeKindName::Session,
                attempt: 1,
                fanout_parent: None,
                timestamp: 1001.0,
            },
            WorkflowEvent::SessionAttached {
                run_id: "exec-1".to_string(),
                workflow_name: "wf".to_string(),
                node_execution_id: "ne-implement-1".to_string(),
                node_name: "implement".to_string(),
                session_id: "step-session-1".to_string(),
                timestamp: 1002.0,
            },
        ];

        let state = reconstruct_state_from_events("exec-1", &events)
            .unwrap()
            .expect("state should be restored");

        assert_eq!(state.current_step_name, "implement");
        assert_eq!(state.current_session_id.as_deref(), Some("step-session-1"));
        assert_eq!(state.step_execution_counts.get("implement"), Some(&1));
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
                node_execution_id: "ne-plan-1".to_string(),
                node_name: "plan".to_string(),
                kind: NodeKindName::Session,
                attempt: 1,
                fanout_parent: None,
                timestamp: 1001.0,
            },
            WorkflowEvent::NodeCompleted {
                run_id: "exec-snap".to_string(),
                workflow_name: "wf".to_string(),
                node_execution_id: "ne-plan-1".to_string(),
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

    /// spec issues-1023: 通常 step が走っている最中に `RunAborted` で終端した場合、
    /// 当該 step が `state="aborted"` の entry として step_history に積まれる。
    /// projection 経路では `current_session_id` を追えないため session_id は None。
    #[test]
    fn projection_records_aborted_entry_for_current_step_on_run_aborted() {
        let snapshot = workflow_with_nodes("wf", vec!["plan"]);
        let events = vec![
            run_started("exec-abort-current", snapshot),
            WorkflowEvent::NodeStarted {
                run_id: "exec-abort-current".to_string(),
                workflow_name: "wf".to_string(),
                node_execution_id: "ne-plan-1".to_string(),
                node_name: "plan".to_string(),
                kind: NodeKindName::Session,
                attempt: 1,
                fanout_parent: None,
                timestamp: 1.0,
            },
            WorkflowEvent::RunAborted {
                run_id: "exec-abort-current".to_string(),
                workflow_name: "wf".to_string(),
                aborted_step: None,
                timestamp: 2.0,
            },
        ];
        let state = reconstruct_state_from_events("exec-abort-current", &events)
            .unwrap()
            .unwrap();
        assert_eq!(state.state, WorkflowExecutionState::Aborted);
        assert_eq!(state.step_history.len(), 1);
        let entry = &state.step_history[0];
        assert_eq!(entry.step_name, "plan");
        assert_eq!(entry.state, "aborted");
        assert_eq!(entry.session_id, None);
        assert_eq!(entry.run_index, 1);
        assert!(entry.child_outputs.is_none());
    }

    /// issues-1196: `RunAborted.aborted_step` は event 専用 snapshot として永続化し、
    /// projection 境界で `StepHistoryEntry` に変換する。
    #[test]
    fn projection_uses_run_aborted_step_snapshot_when_present() {
        let snapshot = workflow_with_nodes("wf", vec!["plan"]);
        let events = vec![
            run_started("exec-abort-snapshot", snapshot),
            WorkflowEvent::NodeStarted {
                run_id: "exec-abort-snapshot".to_string(),
                workflow_name: "wf".to_string(),
                node_execution_id: "ne-plan-1".to_string(),
                node_name: "plan".to_string(),
                kind: NodeKindName::Session,
                attempt: 1,
                fanout_parent: None,
                timestamp: 1.0,
            },
            WorkflowEvent::RunAborted {
                run_id: "exec-abort-snapshot".to_string(),
                workflow_name: "wf".to_string(),
                aborted_step: Some(RunAbortedStepSnapshot {
                    step_name: "plan".to_string(),
                    completed_at: 2.0,
                    result: None,
                    session_id: Some("session-plan".to_string()),
                    token_usage: None,
                    structured_output: None,
                    run_index: 1,
                    child_outputs: Some(vec![
                        RunAbortedChildOutputSnapshot {
                            step_name: "child-completed".to_string(),
                            session_id: Some("session-child-completed".to_string()),
                            result: Some("ok".to_string()),
                            run_index: 1,
                            completed_at: 1.5,
                            structured_output: None,
                            artifact_contract: None,
                            outcome: RunAbortedChildOutcome::Completed,
                        },
                        RunAbortedChildOutputSnapshot {
                            step_name: "child-aborted".to_string(),
                            session_id: Some("session-child-aborted".to_string()),
                            result: None,
                            run_index: 1,
                            completed_at: 2.0,
                            structured_output: None,
                            artifact_contract: None,
                            outcome: RunAbortedChildOutcome::Aborted,
                        },
                    ]),
                }),
                timestamp: 2.0,
            },
        ];

        let state = reconstruct_state_from_events("exec-abort-snapshot", &events)
            .unwrap()
            .unwrap();

        assert_eq!(state.state, WorkflowExecutionState::Aborted);
        assert_eq!(state.step_history.len(), 1);
        let entry = &state.step_history[0];
        assert_eq!(entry.step_name, "plan");
        assert_eq!(entry.session_id.as_deref(), Some("session-plan"));
        assert_eq!(entry.state.as_str(), STEP_STATE_ABORTED);
        let children = entry.child_outputs.as_ref().expect("children restored");
        let completed_child = children
            .iter()
            .find(|child| child.step_name == "child-completed")
            .expect("completed child restored");
        assert_eq!(completed_child.state.as_str(), STEP_STATE_COMPLETED);
        let aborted_child = children
            .iter()
            .find(|child| child.step_name == "child-aborted")
            .expect("aborted child restored");
        assert_eq!(aborted_child.state.as_str(), STEP_STATE_ABORTED);
    }

    #[test]
    fn projection_keeps_repeated_fanout_child_artifacts_by_execution_id_only() {
        let run_id = "exec-fanout-items";
        let workflow = workflow_with_nodes("wf", vec!["workers", "worker"]);
        let mut events = vec![
            run_started(run_id, workflow),
            WorkflowEvent::NodeStarted {
                run_id: run_id.to_string(),
                workflow_name: "wf".to_string(),
                node_execution_id: "ne-parent".to_string(),
                node_name: "workers".to_string(),
                kind: NodeKindName::Fanout,
                attempt: 1,
                fanout_parent: None,
                timestamp: 1.0,
            },
        ];

        for (item_index, node_execution_id, value) in
            [(0, "ne-worker-0", 10), (1, "ne-worker-1", 20)]
        {
            events.extend([
                WorkflowEvent::NodeStarted {
                    run_id: run_id.to_string(),
                    workflow_name: "wf".to_string(),
                    node_execution_id: node_execution_id.to_string(),
                    node_name: "worker".to_string(),
                    kind: NodeKindName::Command,
                    attempt: 1,
                    fanout_parent: Some(FanoutParentRef {
                        parent_node: "workers".to_string(),
                        parent_attempt: 1,
                        item_index: Some(item_index),
                        child_index: 0,
                    }),
                    timestamp: 2.0 + item_index as f64,
                },
                WorkflowEvent::ArtifactProduced {
                    run_id: run_id.to_string(),
                    workflow_name: "wf".to_string(),
                    node_execution_id: node_execution_id.to_string(),
                    node_name: "worker".to_string(),
                    contract: None,
                    value: serde_json::json!({"value": value}),
                    request_id: None,
                    submitted_at: None,
                    timestamp: 4.0 + item_index as f64,
                },
                WorkflowEvent::NodeCompleted {
                    run_id: run_id.to_string(),
                    workflow_name: "wf".to_string(),
                    node_execution_id: node_execution_id.to_string(),
                    node_name: "worker".to_string(),
                    result: None,
                    session_id: None,
                    token_usage: None,
                    structured_output: None,
                    run_index: Some(1),
                    timestamp: 6.0 + item_index as f64,
                },
            ]);
        }

        let parent_artifact = serde_json::json!([{"value": 10}, {"value": 20}]);
        events.extend([
            WorkflowEvent::ArtifactProduced {
                run_id: run_id.to_string(),
                workflow_name: "wf".to_string(),
                node_execution_id: "ne-parent".to_string(),
                node_name: "workers".to_string(),
                contract: None,
                value: parent_artifact.clone(),
                request_id: None,
                submitted_at: None,
                timestamp: 8.0,
            },
            WorkflowEvent::NodeCompleted {
                run_id: run_id.to_string(),
                workflow_name: "wf".to_string(),
                node_execution_id: "ne-parent".to_string(),
                node_name: "workers".to_string(),
                result: Some("complete".to_string()),
                session_id: None,
                token_usage: None,
                structured_output: None,
                run_index: Some(1),
                timestamp: 9.0,
            },
        ]);

        let state = reconstruct_state_from_events(run_id, &events)
            .unwrap()
            .unwrap();

        assert_eq!(state.node_executions.len(), 3);
        assert_eq!(state.current_step_name, "workers");
        assert_eq!(
            state.node_executions[1].artifact,
            Some(serde_json::json!({"value": 10}))
        );
        assert_eq!(
            state.node_executions[2].artifact,
            Some(serde_json::json!({"value": 20}))
        );
        assert!(state.step_outputs.get("worker").is_none());
        assert_eq!(
            state
                .step_outputs
                .get("workers")
                .and_then(|output| output.structured_output.clone()),
            Some(parent_artifact)
        );
        assert_eq!(state.step_history.len(), 1);
        assert_eq!(state.step_history[0].step_name, "workers");

        let timings = compute_step_timings(&events);
        assert!(timings
            .iter()
            .any(|timing| timing.node_execution_id == "ne-worker-0"));
        assert!(timings
            .iter()
            .any(|timing| timing.node_execution_id == "ne-worker-1"));
    }

    #[test]
    fn projection_counts_fanout_token_usage_once_via_parent_completion() {
        let run_id = "exec-fanout-token-usage";
        let mut workflow = workflow_with_nodes("wf", vec!["fanout-review", "review-a", "review-b"]);
        workflow.nodes[0].kind = NodeKind::Fanout(FanoutSpec {
            child: vec!["review-a".to_string(), "review-b".to_string()],
            items: None,
            aggregate: None,
        });
        let mut events = vec![
            run_started(run_id, workflow),
            WorkflowEvent::NodeStarted {
                run_id: run_id.to_string(),
                workflow_name: "wf".to_string(),
                node_execution_id: "ne-parent".to_string(),
                node_name: "fanout-review".to_string(),
                kind: NodeKindName::Fanout,
                attempt: 1,
                fanout_parent: None,
                timestamp: 1.0,
            },
        ];
        for (child_index, (node_execution_id, node_name, input_tokens, output_tokens)) in [
            ("ne-review-a", "review-a", 3, 5),
            ("ne-review-b", "review-b", 7, 11),
        ]
        .into_iter()
        .enumerate()
        {
            events.extend([
                WorkflowEvent::NodeStarted {
                    run_id: run_id.to_string(),
                    workflow_name: "wf".to_string(),
                    node_execution_id: node_execution_id.to_string(),
                    node_name: node_name.to_string(),
                    kind: NodeKindName::Session,
                    attempt: 1,
                    fanout_parent: Some(FanoutParentRef {
                        parent_node: "fanout-review".to_string(),
                        parent_attempt: 1,
                        item_index: None,
                        child_index,
                    }),
                    timestamp: 2.0 + child_index as f64,
                },
                WorkflowEvent::NodeCompleted {
                    run_id: run_id.to_string(),
                    workflow_name: "wf".to_string(),
                    node_execution_id: node_execution_id.to_string(),
                    node_name: node_name.to_string(),
                    result: Some("LGTM".to_string()),
                    session_id: Some(format!("session-{node_name}")),
                    token_usage: Some(TokenUsage {
                        input_tokens,
                        output_tokens,
                    }),
                    structured_output: None,
                    run_index: Some(1),
                    timestamp: 4.0 + child_index as f64,
                },
            ]);
        }

        let child_only_state = reconstruct_state_from_events(run_id, &events)
            .unwrap()
            .unwrap();
        assert_eq!(child_only_state.total_token_usage, TokenUsage::default());

        events.push(WorkflowEvent::NodeCompleted {
            run_id: run_id.to_string(),
            workflow_name: "wf".to_string(),
            node_execution_id: "ne-parent".to_string(),
            node_name: "fanout-review".to_string(),
            result: Some("complete".to_string()),
            session_id: None,
            token_usage: Some(TokenUsage {
                input_tokens: 10,
                output_tokens: 16,
            }),
            structured_output: None,
            run_index: Some(1),
            timestamp: 6.0,
        });

        let completed_state = reconstruct_state_from_events(run_id, &events)
            .unwrap()
            .unwrap();
        assert_eq!(
            completed_state.total_token_usage,
            TokenUsage {
                input_tokens: 10,
                output_tokens: 16,
            }
        );
        assert_eq!(
            completed_state
                .node_executions
                .iter()
                .find(|execution| execution.id == "ne-parent")
                .and_then(|execution| execution.token_usage.clone()),
            Some(TokenUsage {
                input_tokens: 10,
                output_tokens: 16,
            })
        );
    }

    /// [06] CLI 経由 mutation 要求の事実は append-only な観測情報であり、engine
    /// domain state には影響しない（spec [06] 観測経路境界 / 既存 `ApprovalResolved`
    /// / `RunAborted` が実 state 変化の事実を担う境界を温存する）。projection 上で
    /// domain state / updated_at が変化しないことを境界として担保する。
    #[test]
    fn projection_treats_cli_mutation_requested_as_observation_only() {
        use crate::adaptor::gateway::workflow::event::CliMutationRequestRecord;
        let snapshot = workflow_with_nodes("wf", vec!["plan", "review"]);
        let events = vec![
            run_started("exec-cli", snapshot),
            WorkflowEvent::NodeStarted {
                run_id: "exec-cli".to_string(),
                workflow_name: "wf".to_string(),
                node_execution_id: "ne-plan-1".to_string(),
                node_name: "plan".to_string(),
                kind: NodeKindName::Session,
                attempt: 1,
                fanout_parent: None,
                timestamp: 1001.0,
            },
            WorkflowEvent::CliMutationRequested {
                run_id: "exec-cli".to_string(),
                workflow_name: "wf".to_string(),
                request_id: "00000000-0000-0000-0000-000000000601".to_string(),
                request: CliMutationRequestRecord::Approve {
                    node_name: "plan".to_string(),
                    comment: Some("LGTM".to_string()),
                },
                requested_at: 1500.0,
                timestamp: 1500.0,
            },
        ];
        let state = reconstruct_state_from_events("exec-cli", &events)
            .unwrap()
            .unwrap();
        // NodeStarted → Running が維持されている（CliMutationRequested で state は遷移しない）。
        assert_eq!(state.state, WorkflowExecutionState::Running);
        assert_eq!(state.current_step_name, "plan");
        assert_eq!(state.updated_at, 1001.0);
    }

    /// spec issues-1023: event timestamp は engine 内 `current_timestamp()` 由来の
    /// 秒単位 f64。`compute_step_timings` は per-step duration を集計しつつ ms へ
    /// 正規化する境界（frontend は表示用フォーマットに留まる）。
    #[test]
    fn compute_step_timings_pairs_node_started_and_node_completed() {
        let events = vec![
            WorkflowEvent::NodeStarted {
                run_id: "r".to_string(),
                workflow_name: "wf".to_string(),
                node_execution_id: "ne-plan-1".to_string(),
                node_name: "plan".to_string(),
                kind: NodeKindName::Session,
                attempt: 1,
                fanout_parent: None,
                timestamp: 1000.0,
            },
            WorkflowEvent::NodeCompleted {
                run_id: "r".to_string(),
                workflow_name: "wf".to_string(),
                node_execution_id: "ne-plan-1".to_string(),
                node_name: "plan".to_string(),
                result: Some("LGTM".to_string()),
                session_id: Some("s1".to_string()),
                token_usage: None,
                structured_output: None,
                run_index: Some(1),
                timestamp: 1750.0,
            },
        ];
        let timings = compute_step_timings(&events);
        assert_eq!(timings.len(), 1);
        let t = &timings[0];
        assert_eq!(t.step_name, "plan");
        assert_eq!(t.run_index, 1);
        // 秒 → ミリ秒へ正規化された値が返る境界。
        assert_eq!(t.started_at_ms, Some(1_000_000.0));
        assert_eq!(t.completed_at_ms, Some(1_750_000.0));
        assert_eq!(t.duration_ms, Some(750_000.0));
    }

    #[test]
    fn compute_step_timings_handles_fanout_child_node_execution() {
        let events = vec![
            WorkflowEvent::NodeStarted {
                run_id: "r".to_string(),
                workflow_name: "wf".to_string(),
                node_execution_id: "ne-child-a".to_string(),
                node_name: "child-a".to_string(),
                kind: NodeKindName::Session,
                attempt: 1,
                fanout_parent: Some(FanoutParentRef {
                    parent_node: "parent".to_string(),
                    parent_attempt: 1,
                    item_index: Some(0),
                    child_index: 0,
                }),
                timestamp: 100.0,
            },
            WorkflowEvent::NodeCompleted {
                run_id: "r".to_string(),
                workflow_name: "wf".to_string(),
                node_execution_id: "ne-child-a".to_string(),
                node_name: "child-a".to_string(),
                result: None,
                session_id: Some("s-a".to_string()),
                token_usage: None,
                structured_output: None,
                run_index: Some(1),
                timestamp: 300.0,
            },
        ];
        let timings = compute_step_timings(&events);
        assert_eq!(timings.len(), 1);
        assert_eq!(timings[0].node_execution_id, "ne-child-a");
        assert_eq!(timings[0].step_name, "child-a");
        assert_eq!(timings[0].duration_ms, Some(200_000.0));
    }

    #[test]
    fn compute_step_timings_uses_node_started_and_ignores_session_attach_time() {
        let events = vec![
            WorkflowEvent::NodeStarted {
                run_id: "r".to_string(),
                workflow_name: "wf".to_string(),
                node_execution_id: "ne-plan".to_string(),
                node_name: "plan".to_string(),
                kind: NodeKindName::Session,
                attempt: 1,
                fanout_parent: None,
                timestamp: 100.0,
            },
            WorkflowEvent::SessionAttached {
                run_id: "r".to_string(),
                workflow_name: "wf".to_string(),
                node_execution_id: "ne-plan".to_string(),
                node_name: "plan".to_string(),
                session_id: "s1".to_string(),
                timestamp: 125.0,
            },
            WorkflowEvent::NodeCompleted {
                run_id: "r".to_string(),
                workflow_name: "wf".to_string(),
                node_execution_id: "ne-plan".to_string(),
                node_name: "plan".to_string(),
                result: None,
                session_id: Some("s1".to_string()),
                token_usage: None,
                structured_output: None,
                run_index: Some(1),
                timestamp: 250.0,
            },
        ];

        let timings = compute_step_timings(&events);

        assert_eq!(timings.len(), 1);
        assert_eq!(timings[0].started_at_ms, Some(100_000.0));
        assert_eq!(timings[0].duration_ms, Some(150_000.0));
    }

    #[test]
    fn compute_step_timings_leaves_duration_none_when_started_missing() {
        let events = vec![WorkflowEvent::NodeCompleted {
            run_id: "r".to_string(),
            workflow_name: "wf".to_string(),
            node_execution_id: "ne-plan-1".to_string(),
            node_name: "plan".to_string(),
            result: None,
            session_id: None,
            token_usage: None,
            structured_output: None,
            run_index: Some(1),
            timestamp: 200.0,
        }];
        let timings = compute_step_timings(&events);
        assert_eq!(timings.len(), 1);
        assert!(timings[0].started_at_ms.is_none());
        assert!(timings[0].duration_ms.is_none());
    }

    /// spec issues-1023: `events_with_ms_timestamps` は event 列をそのままの順序で
    /// 返しつつ、すべての timestamp フィールドを秒 → ミリ秒に正規化する。
    #[test]
    fn events_with_ms_timestamps_converts_seconds_to_ms() {
        let snapshot = workflow_with_nodes("wf", vec!["plan"]);
        let events = vec![
            run_started("exec-ms", snapshot),
            WorkflowEvent::NodeStarted {
                run_id: "exec-ms".to_string(),
                workflow_name: "wf".to_string(),
                node_execution_id: "ne-plan-1".to_string(),
                node_name: "plan".to_string(),
                kind: NodeKindName::Session,
                attempt: 1,
                fanout_parent: None,
                timestamp: 1.5,
            },
        ];
        let converted = events_with_ms_timestamps(events);
        assert_eq!(converted.len(), 2);
        if let WorkflowEventView::RunStarted { timestamp_ms, .. } = &converted[0] {
            assert_eq!(*timestamp_ms, 1_000_000.0);
        } else {
            panic!("first event must remain RunStarted");
        }
        if let WorkflowEventView::NodeStarted { timestamp_ms, .. } = &converted[1] {
            assert_eq!(*timestamp_ms, 1500.0);
        } else {
            panic!("second event must remain NodeStarted");
        }
    }

    /// spec issues-1023: `compute_step_detail` は `(node_name, run_index)` を主語に
    /// step の入出力・遷移結果・所要時間を返す。node 定義から input facts を引き、
    /// history entry から output facts を引く。timestamps は ms 正規化される。
    #[test]
    fn compute_step_detail_returns_completed_step_with_input_and_output() {
        let mut workflow = workflow_with_nodes("wf", vec!["plan", "review"]);
        workflow.nodes[1].kind = session_kind("review the diff");
        let snapshot = workflow.clone();
        let events = vec![
            run_started("exec-detail", snapshot),
            WorkflowEvent::NodeStarted {
                run_id: "exec-detail".to_string(),
                workflow_name: "wf".to_string(),
                node_execution_id: "ne-plan-1".to_string(),
                node_name: "plan".to_string(),
                kind: NodeKindName::Session,
                attempt: 1,
                fanout_parent: None,
                timestamp: 1100.0,
            },
            WorkflowEvent::NodeCompleted {
                run_id: "exec-detail".to_string(),
                workflow_name: "wf".to_string(),
                node_execution_id: "ne-plan-1".to_string(),
                node_name: "plan".to_string(),
                result: Some("ok".to_string()),
                session_id: Some("plan-session".to_string()),
                token_usage: None,
                structured_output: Some(serde_json::json!({"summary": "diff is ok"})),
                run_index: Some(1),
                timestamp: 1200.0,
            },
            WorkflowEvent::NodeStarted {
                run_id: "exec-detail".to_string(),
                workflow_name: "wf".to_string(),
                node_execution_id: "ne-review-1".to_string(),
                node_name: "review".to_string(),
                kind: NodeKindName::Session,
                attempt: 1,
                fanout_parent: None,
                timestamp: 1300.0,
            },
            WorkflowEvent::NodeCompleted {
                run_id: "exec-detail".to_string(),
                workflow_name: "wf".to_string(),
                node_execution_id: "ne-review-1".to_string(),
                node_name: "review".to_string(),
                result: Some("LGTM".to_string()),
                session_id: Some("review-session".to_string()),
                token_usage: None,
                structured_output: Some(serde_json::json!({"verdict": "LGTM"})),
                run_index: Some(1),
                timestamp: 1500.0,
            },
        ];
        let state = reconstruct_state_from_events("exec-detail", &events)
            .unwrap()
            .unwrap();
        let detail = compute_step_detail(&state, &events, "review", Some(1)).unwrap();
        assert_eq!(detail.step_name, "review");
        assert_eq!(detail.node_type, "session");
        assert_eq!(detail.run_index, 1);
        assert_eq!(detail.result.as_deref(), Some("LGTM"));
        assert_eq!(detail.session_id.as_deref(), Some("review-session"));
        assert_eq!(detail.started_at_ms, Some(1_300_000.0));
        assert_eq!(detail.completed_at_ms, Some(1_500_000.0));
        assert_eq!(detail.duration_ms, Some(200_000.0));
        assert_eq!(detail.input.instruction.as_deref(), Some("review the diff"));
        assert_eq!(detail.input.previous_step_name.as_deref(), Some("plan"));
        assert_eq!(
            detail.input.previous_step_structured_output,
            Some(serde_json::json!({"summary": "diff is ok"}))
        );
        assert_eq!(
            detail.structured_output,
            Some(serde_json::json!({"verdict": "LGTM"}))
        );
    }

    /// spec issues-1023: 未到達の pending node でも node 定義から引いた input facts を
    /// 返し、frontend が timeline 上で選択した瞬間に static な情報を表示できる。
    #[test]
    fn compute_step_detail_returns_input_for_pending_step() {
        let mut workflow = workflow_with_nodes("wf", vec!["plan", "review"]);
        workflow.nodes[1].kind = session_kind("review later");
        let snapshot = workflow.clone();
        let events = vec![
            run_started("exec-pending", snapshot),
            WorkflowEvent::NodeStarted {
                run_id: "exec-pending".to_string(),
                workflow_name: "wf".to_string(),
                node_execution_id: "ne-plan-1".to_string(),
                node_name: "plan".to_string(),
                kind: NodeKindName::Session,
                attempt: 1,
                fanout_parent: None,
                timestamp: 1.0,
            },
        ];
        let state = reconstruct_state_from_events("exec-pending", &events)
            .unwrap()
            .unwrap();
        let detail = compute_step_detail(&state, &events, "review", None).unwrap();
        assert_eq!(detail.step_name, "review");
        assert_eq!(detail.state, "pending");
        assert!(detail.result.is_none());
        assert_eq!(detail.input.instruction.as_deref(), Some("review later"));
    }

    /// [08] ArtifactProduced projection: live engine と同じ shape で `StepOutput` slot を
    /// 復元する。`result` は contract validator を再導出するため、reload 経路でも
    /// `input_reference` で経路非依存に参照できる（spec [08] Rule 3 Scenario 1/3）。
    #[test]
    fn projection_restores_step_output_from_artifact_produced_with_validator_derived_result() {
        let mut workflow = workflow_with_nodes("wf", vec!["review"]);
        workflow.schemas = [(
            "review-verdict".to_string(),
            SchemaDef::Object {
                properties: [("verdict".to_string(), SchemaDef::String { r#enum: None })]
                    .into_iter()
                    .collect(),
                required: ["verdict".to_string()].into_iter().collect(),
                additional_properties: false,
            },
        )]
        .into_iter()
        .collect();
        workflow.nodes[0].artifact = Some("review-verdict".to_string());
        let events = vec![
            run_started("exec-submit", workflow),
            WorkflowEvent::NodeStarted {
                run_id: "exec-submit".to_string(),
                workflow_name: "wf".to_string(),
                node_execution_id: "ne-review-1".to_string(),
                node_name: "review".to_string(),
                kind: NodeKindName::Session,
                attempt: 1,
                fanout_parent: None,
                timestamp: 1001.0,
            },
            WorkflowEvent::ArtifactProduced {
                run_id: "exec-submit".to_string(),
                workflow_name: "wf".to_string(),
                node_execution_id: "ne-review-1".to_string(),
                node_name: "review".to_string(),
                contract: Some("review-verdict".to_string()),
                value: serde_json::json!({"verdict": "LGTM"}),
                request_id: Some("00000000-0000-0000-0000-0000000000aa".to_string()),
                submitted_at: Some(1010.0),
                timestamp: 1011.0,
            },
        ];

        let state = reconstruct_state_from_events("exec-submit", &events)
            .unwrap()
            .unwrap();
        let so = state
            .step_outputs
            .get("review")
            .expect("ArtifactProduced should restore step_outputs slot");
        assert_eq!(so.artifact_contract.as_deref(), Some("review-verdict"));
        assert!(so.structured_output.is_some());
        // validator から再導出された result を持つ（live と同じ shape）。
        assert_eq!(so.result.as_deref(), Some("LGTM"));
    }

    #[test]
    fn node_completed_projection_preserves_submitted_artifact_output() {
        let mut workflow = workflow_with_nodes("wf", vec!["review"]);
        workflow.schemas = [(
            "review-verdict".to_string(),
            SchemaDef::Object {
                properties: [("verdict".to_string(), SchemaDef::String { r#enum: None })]
                    .into_iter()
                    .collect(),
                required: ["verdict".to_string()].into_iter().collect(),
                additional_properties: false,
            },
        )]
        .into_iter()
        .collect();
        workflow.nodes[0].artifact = Some("review-verdict".to_string());
        let artifact_value = serde_json::json!({"verdict": "LGTM"});
        let mut events = vec![
            run_started("exec-submit-complete", workflow),
            WorkflowEvent::NodeStarted {
                run_id: "exec-submit-complete".to_string(),
                workflow_name: "wf".to_string(),
                node_execution_id: "ne-review-1".to_string(),
                node_name: "review".to_string(),
                kind: NodeKindName::Session,
                attempt: 1,
                fanout_parent: None,
                timestamp: 1001.0,
            },
            WorkflowEvent::ArtifactProduced {
                run_id: "exec-submit-complete".to_string(),
                workflow_name: "wf".to_string(),
                node_execution_id: "ne-review-1".to_string(),
                node_name: "review".to_string(),
                contract: Some("review-verdict".to_string()),
                value: artifact_value.clone(),
                request_id: Some("00000000-0000-0000-0000-0000000000bb".to_string()),
                submitted_at: Some(1010.0),
                timestamp: 1011.0,
            },
        ];
        let submitted_state = reconstruct_state_from_events("exec-submit-complete", &events)
            .unwrap()
            .unwrap();
        let submitted_output = submitted_state.step_outputs["review"].clone();

        events.push(WorkflowEvent::NodeCompleted {
            run_id: "exec-submit-complete".to_string(),
            workflow_name: "wf".to_string(),
            node_execution_id: "ne-review-1".to_string(),
            node_name: "review".to_string(),
            result: Some("LGTM".to_string()),
            session_id: Some("session-review".to_string()),
            token_usage: None,
            structured_output: Some(artifact_value),
            run_index: Some(1),
            timestamp: 1012.0,
        });

        let completed_state = reconstruct_state_from_events("exec-submit-complete", &events)
            .unwrap()
            .unwrap();
        let completed_output = &completed_state.step_outputs["review"];
        assert_eq!(
            completed_output.artifact_contract,
            submitted_output.artifact_contract
        );
        assert_eq!(
            completed_output.structured_output,
            submitted_output.structured_output
        );
        assert_eq!(completed_output.result, submitted_output.result);
        assert_eq!(
            completed_output.session_id.as_deref(),
            Some("session-review")
        );
    }

    #[test]
    fn projection_restores_schema_valid_artifact_without_workflow_variable_side_effects() {
        let mut workflow = workflow_with_nodes("wf", vec!["spec"]);
        workflow.schemas = [(
            "spec-directory".to_string(),
            SchemaDef::Object {
                properties: [("spec_dir".to_string(), SchemaDef::String { r#enum: None })]
                    .into_iter()
                    .collect(),
                required: ["spec_dir".to_string()].into_iter().collect(),
                additional_properties: false,
            },
        )]
        .into_iter()
        .collect();
        workflow.nodes[0].artifact = Some("spec-directory".to_string());
        let events = vec![
            run_started("exec-spec", workflow),
            WorkflowEvent::NodeStarted {
                run_id: "exec-spec".to_string(),
                workflow_name: "wf".to_string(),
                node_execution_id: "ne-spec-1".to_string(),
                node_name: "spec".to_string(),
                kind: NodeKindName::Session,
                attempt: 1,
                fanout_parent: None,
                timestamp: 1001.0,
            },
            WorkflowEvent::ArtifactProduced {
                run_id: "exec-spec".to_string(),
                workflow_name: "wf".to_string(),
                node_execution_id: "ne-spec-1".to_string(),
                node_name: "spec".to_string(),
                contract: Some("spec-directory".to_string()),
                value: serde_json::json!({"spec_dir": "docs/specs/feat-issues-978"}),
                request_id: None,
                submitted_at: Some(1010.0),
                timestamp: 1011.0,
            },
        ];

        let state = reconstruct_state_from_events("exec-spec", &events)
            .unwrap()
            .unwrap();
        assert_eq!(
            state.step_outputs["spec"]
                .structured_output
                .as_ref()
                .unwrap()["spec_dir"],
            "docs/specs/feat-issues-978"
        );
    }
}
