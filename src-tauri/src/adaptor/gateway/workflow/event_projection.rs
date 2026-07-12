//! `WorkflowEvent` 列から workflow read model / runtime state を再構築する projection。
//!
//! spec の責務配置に従い、`workflow/log.rs` は NDJSON の append/read 機構へ責務を限定し、
//! event 列 → WorkflowState の射影 (projection) は gateway 側の本モジュールに置く。
//! 過去 NDJSON 在庫の互換性は spec [02]/[04] の範囲で別途扱う。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::adaptor::gateway::workflow::domain_mapping::workflow_schemas_to_domain;
#[cfg(test)]
use crate::adaptor::gateway::workflow::event::{
    ApprovalDecisionRecord, CliMutationRejectionReason, CliMutationRequestRecord,
    CollectedOutputEntry,
};
use crate::adaptor::gateway::workflow::event::{
    RunAbortedChildOutcome, RunAbortedChildOutputSnapshot, RunAbortedStepSnapshot,
    TokenUsage as EventTokenUsage, WorkflowEvent,
};
use crate::adaptor::gateway::workflow::schema::Workflow;
use crate::adaptor::gateway::workflow::state::{
    ChildOutputSnapshot, ParallelStepState, StepHistoryEntry, StepOutput, TokenUsage,
    WorkflowExecutionState, WorkflowStallObservation, WorkflowState,
};
use crate::domain::workflow::services::{
    contract as workflow_contract, parallel as workflow_parallel,
};
use crate::domain::workflow::{
    ContractValidationResult, STEP_STATE_ABORTED, STEP_STATE_COMPLETED, STEP_STATE_FAILED,
    STEP_STATE_INTERRUPTED, STEP_STATE_PENDING, STEP_STATE_RUNNING,
};
#[cfg(test)]
use crate::domain::workflow::{FailureDisposition, WorkflowStepFailureKind};

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

fn completed_parallel_history_result(aggregate_result: &str) -> String {
    if aggregate_result == "advance" {
        "complete".to_string()
    } else {
        aggregate_result.to_string()
    }
}

/// spec issues-1023: event 列から (step_name, run_index) ごとの started/completed/duration を
/// 集約する純粋関数。engine 側 event projection の責務として、所要時間計算と
/// 単位変換（秒 → ミリ秒）を担う（frontend は表示用フォーマットのみ）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowStepTimingView {
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
    // (step_name, run_index) -> (started_at, completed_at) 秒単位。
    let mut buckets: HashMap<(String, u32), (Option<f64>, Option<f64>)> = HashMap::new();
    let mut last_started_idx: HashMap<String, u32> = HashMap::new();
    let mut order: Vec<(String, u32)> = Vec::new();

    for event in events {
        match event {
            WorkflowEvent::NodeStarted {
                node_name,
                execution_count,
                timestamp,
                ..
            }
            | WorkflowEvent::StepSessionStarted {
                node_name,
                execution_count,
                timestamp,
                ..
            } => {
                let key = (node_name.clone(), *execution_count);
                last_started_idx.insert(node_name.clone(), *execution_count);
                let entry = buckets.entry(key.clone()).or_insert((None, None));
                if entry.0.is_none() {
                    entry.0 = Some(*timestamp);
                    order.push(key);
                }
            }
            WorkflowEvent::NodeCompleted {
                node_name,
                run_index,
                timestamp,
                ..
            } => {
                let idx = run_index
                    .or_else(|| last_started_idx.get(node_name).copied())
                    .unwrap_or(0);
                let key = (node_name.clone(), idx);
                let entry = buckets.entry(key.clone()).or_insert((None, None));
                entry.1 = Some(*timestamp);
                if !order.contains(&key) {
                    order.push(key);
                }
            }
            WorkflowEvent::ParallelChildStarted {
                child_node_name,
                execution_count,
                timestamp,
                ..
            } => {
                let key = (child_node_name.clone(), *execution_count);
                let entry = buckets.entry(key.clone()).or_insert((None, None));
                if entry.0.is_none() {
                    entry.0 = Some(*timestamp);
                    order.push(key);
                }
            }
            WorkflowEvent::ParallelChildCompleted {
                child_node_name,
                run_index,
                timestamp,
                ..
            } => {
                let key = (child_node_name.clone(), *run_index);
                let entry = buckets.entry(key.clone()).or_insert((None, None));
                entry.1 = Some(*timestamp);
                if !order.contains(&key) {
                    order.push(key);
                }
            }
            _ => {}
        }
    }

    order
        .into_iter()
        .map(|key| {
            let (started_at, completed_at) = buckets.remove(&key).unwrap_or((None, None));
            let duration = match (started_at, completed_at) {
                (Some(s), Some(c)) if c >= s => Some(c - s),
                _ => None,
            };
            WorkflowStepTimingView {
                step_name: key.0,
                run_index: key.1,
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
        node_name: String,
        execution_count: u32,
        #[serde(rename = "timestampMs")]
        timestamp_ms: f64,
    },
    StepSessionStarted {
        run_id: String,
        workflow_name: String,
        node_name: String,
        execution_count: u32,
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
        node_name: String,
        #[serde(rename = "timestampMs")]
        timestamp_ms: f64,
    },
    ApprovalResolved {
        run_id: String,
        workflow_name: String,
        node_name: String,
        decision: ApprovalDecisionRecord,
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
    ParallelStarted {
        run_id: String,
        workflow_name: String,
        parent_node_name: String,
        child_node_names: Vec<String>,
        #[serde(rename = "timestampMs")]
        timestamp_ms: f64,
    },
    ParallelChildStarted {
        run_id: String,
        workflow_name: String,
        parent_node_name: String,
        child_node_name: String,
        session_id: String,
        execution_count: u32,
        #[serde(rename = "timestampMs")]
        timestamp_ms: f64,
    },
    ParallelChildCompleted {
        run_id: String,
        workflow_name: String,
        parent_node_name: String,
        child_node_name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        result: Option<String>,
        session_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        token_usage: Option<EventTokenUsage>,
        #[serde(skip_serializing_if = "Option::is_none")]
        structured_output: Option<serde_json::Value>,
        run_index: u32,
        state: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        failure_kind: Option<WorkflowStepFailureKind>,
        #[serde(skip_serializing_if = "Option::is_none")]
        failure_disposition: Option<FailureDisposition>,
        #[serde(rename = "timestampMs")]
        timestamp_ms: f64,
    },
    ParallelCompleted {
        run_id: String,
        workflow_name: String,
        parent_node_name: String,
        aggregate_result: String,
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
                node_name,
                execution_count,
                timestamp,
            } => WorkflowEventView::NodeStarted {
                run_id,
                workflow_name,
                node_name,
                execution_count,
                timestamp_ms: seconds_to_ms(timestamp),
            },
            WorkflowEvent::StepSessionStarted {
                run_id,
                workflow_name,
                node_name,
                execution_count,
                session_id,
                timestamp,
            } => WorkflowEventView::StepSessionStarted {
                run_id,
                workflow_name,
                node_name,
                execution_count,
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
                node_name,
                reason,
                failure_kind,
                retry_count,
                timestamp,
            } => WorkflowEventView::NodeFailed {
                run_id,
                workflow_name,
                node_name,
                reason,
                failure_kind,
                retry_count,
                timestamp_ms: seconds_to_ms(timestamp),
            },
            WorkflowEvent::ApprovalRequested {
                run_id,
                workflow_name,
                node_name,
                timestamp,
            } => WorkflowEventView::ApprovalRequested {
                run_id,
                workflow_name,
                node_name,
                timestamp_ms: seconds_to_ms(timestamp),
            },
            WorkflowEvent::ApprovalResolved {
                run_id,
                workflow_name,
                node_name,
                decision,
                comment,
                timestamp,
            } => WorkflowEventView::ApprovalResolved {
                run_id,
                workflow_name,
                node_name,
                decision,
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
            WorkflowEvent::ParallelStarted {
                run_id,
                workflow_name,
                parent_node_name,
                child_node_names,
                timestamp,
            } => WorkflowEventView::ParallelStarted {
                run_id,
                workflow_name,
                parent_node_name,
                child_node_names,
                timestamp_ms: seconds_to_ms(timestamp),
            },
            WorkflowEvent::ParallelChildStarted {
                run_id,
                workflow_name,
                parent_node_name,
                child_node_name,
                session_id,
                execution_count,
                timestamp,
            } => WorkflowEventView::ParallelChildStarted {
                run_id,
                workflow_name,
                parent_node_name,
                child_node_name,
                session_id,
                execution_count,
                timestamp_ms: seconds_to_ms(timestamp),
            },
            WorkflowEvent::ParallelChildCompleted {
                run_id,
                workflow_name,
                parent_node_name,
                child_node_name,
                result,
                session_id,
                token_usage,
                structured_output,
                run_index,
                state,
                failure_kind,
                failure_disposition,
                timestamp,
            } => WorkflowEventView::ParallelChildCompleted {
                run_id,
                workflow_name,
                parent_node_name,
                child_node_name,
                result,
                session_id,
                token_usage,
                structured_output,
                run_index,
                state,
                failure_kind,
                failure_disposition,
                timestamp_ms: seconds_to_ms(timestamp),
            },
            WorkflowEvent::ParallelCompleted {
                run_id,
                workflow_name,
                parent_node_name,
                aggregate_result,
                timestamp,
            } => WorkflowEventView::ParallelCompleted {
                run_id,
                workflow_name,
                parent_node_name,
                aggregate_result,
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
                node_name,
                contract,
                value,
                request_id,
                submitted_at,
                timestamp,
            } => WorkflowEventView::ArtifactProduced {
                run_id,
                workflow_name,
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
        // 直前 top-level node の出力を input として参照する境界。
        if let Some(idx) = workflow.nodes.iter().position(|n| n.name == node_name) {
            if idx > 0 {
                view.previous_step_name = Some(workflow.nodes[idx - 1].name.clone());
            }
        }
        return (Some(node.kind_name().as_str()), view);
    }
    // parallel child node
    for parent in &workflow.nodes {
        if let Some(fanout) = parent.fanout() {
            let children = &fanout.parallel_children;
            if let Some(child) = children.iter().find(|c| c.name == node_name) {
                let view = WorkflowStepInputView {
                    instruction: child.facets.instruction.clone(),
                    policy: child.facets.policy.clone(),
                    knowledge: child.facets.knowledge.clone(),
                    artifact: child.artifact.clone(),
                    input: child.input.clone(),
                    previous_step_name: Some(parent.name.clone()),
                    ..WorkflowStepInputView::default()
                };
                return (Some("session"), view);
            }
        }
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

    if let Some(entry) = history_entry {
        // history detail は実行回 (run_index) 単位の表示。state.step_states は
        // step_name 単位の最新状態しか保持しないため、過去 run を開いた際に
        // 最新 run の state で上書きされないよう entry.state を使う。
        return Some(WorkflowStepDetailView {
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

    // parallel child（active）
    for ps in &state.active_parallel_steps {
        if ps.step_name == node_name && (run_index.is_none() || run_index == Some(ps.run_index)) {
            return Some(WorkflowStepDetailView {
                step_name: node_name.to_string(),
                node_type: node_type_str.unwrap_or("unknown").to_string(),
                run_index: ps.run_index,
                state: ps.state.clone(),
                session_id: ps.session_id.clone(),
                result: ps.result.clone(),
                structured_output: ps.structured_output.clone(),
                token_usage: None,
                started_at_ms: timing.and_then(|t| t.started_at_ms),
                completed_at_ms: timing
                    .and_then(|t| t.completed_at_ms)
                    .or(ps.completed_at.map(seconds_to_ms)),
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
    let mut active_parallel_steps: Vec<ParallelStepState> = Vec::new();
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
                // 新しい node が始まった時点では session 起動前。
                // `StepSessionStarted` が後続で観測されるまで current_session_id は None。
                current_session_id = None;
                stall_observations.clear();
                // [04] approval を経て次 node が開始した場合に exec_state を
                // Running へ復元する。ApprovalRequested で WaitingApproval に
                // 切り替わったまま NodeStarted が来ると、復元 state が承認待ち
                // のまま固定されてしまうため、本イベントで Running に戻す。
                if matches!(exec_state, WorkflowExecutionState::WaitingApproval) {
                    exec_state = WorkflowExecutionState::Running;
                }
                updated_at = *timestamp;
            }
            WorkflowEvent::StepSessionStarted {
                node_name,
                session_id,
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
                current_session_id = Some(session_id.clone());
                step_execution_counts.insert(node_name.clone(), *execution_count);
                if matches!(exec_state, WorkflowExecutionState::WaitingApproval) {
                    exec_state = WorkflowExecutionState::Running;
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
                    existing.session_id = completed_entry.session_id.or(existing.session_id.take());
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
                            result: result
                                .clone()
                                .or_else(|| prior_output.as_ref().and_then(|p| p.result.clone())),
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
                if let Some(ref usage) = token_usage {
                    total_token_usage.add(usage);
                }
                current_session_id = None;
                stall_observations.clear();
                updated_at = *timestamp;
            }
            WorkflowEvent::NodeFailed {
                reason,
                failure_kind,
                retry_count,
                timestamp,
                ..
            } => {
                for ps in &mut active_parallel_steps {
                    if ps.state == STEP_STATE_RUNNING {
                        ps.state = STEP_STATE_FAILED.to_string();
                    }
                }
                exec_state = WorkflowExecutionState::Failed {
                    reason: reason.clone(),
                    kind: *failure_kind,
                    retry_count: *retry_count,
                };
                current_session_id = None;
                stall_observations.clear();
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
                active_parallel_steps.clear();
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
                    // spec issues-1023: 中断時に走っていた current step / parallel
                    // children を `step_history` に "aborted" 状態として記録する。
                    // session log への到達経路（child の session_id）を残すために
                    // active_parallel_steps の snapshot を child_outputs に転写する。
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

                    if !active_parallel_steps.is_empty() {
                        let parent_run_index = step_execution_counts
                            .get(&current_step_name)
                            .copied()
                            .unwrap_or(0);
                        let child_snapshots: Vec<
                            crate::adaptor::gateway::workflow::state::ChildOutputSnapshot,
                        > = active_parallel_steps
                            .iter()
                            .map(|child| {
                                let snapshot_state = if child.state == STEP_STATE_COMPLETED {
                                    STEP_STATE_COMPLETED
                                } else {
                                    STEP_STATE_ABORTED
                                };
                                crate::adaptor::gateway::workflow::state::ChildOutputSnapshot {
                                    step_name: child.step_name.clone(),
                                    session_id: child.session_id.clone(),
                                    result: child.result.clone(),
                                    run_index: child.run_index,
                                    completed_at: child.completed_at.unwrap_or(*timestamp),
                                    structured_output: child.structured_output.clone(),
                                    artifact_contract: child.artifact_contract.clone(),
                                    state: snapshot_state.to_string(),
                                    failure_kind: None,
                                    failure_disposition: None,
                                }
                            })
                            .collect();
                        step_history.push(StepHistoryEntry {
                            step_name: current_step_name.clone(),
                            completed_at: *timestamp,
                            result: None,
                            session_id: None,
                            token_usage: None,
                            structured_output: None,
                            run_index: parent_run_index,
                            child_outputs: Some(child_snapshots),
                            state: STEP_STATE_ABORTED.to_string(),
                        });
                    } else if current_started
                        && !already_in_history
                        && !current_step_name.is_empty()
                    {
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

                active_parallel_steps.clear();
                current_session_id = None;
                stall_observations.clear();
                updated_at = *timestamp;
            }
            WorkflowEvent::RunInterrupted { timestamp, .. } => {
                exec_state = WorkflowExecutionState::Interrupted;
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

                active_parallel_steps.clear();
                current_session_id = None;
                stall_observations.clear();
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
                // parallel parent には逐次 session が無い。
                current_session_id = None;
                if matches!(exec_state, WorkflowExecutionState::WaitingApproval) {
                    exec_state = WorkflowExecutionState::Running;
                }
                active_parallel_steps = child_node_names
                    .iter()
                    .map(|name| ParallelStepState {
                        step_name: name.clone(),
                        state: STEP_STATE_RUNNING.to_string(),
                        session_id: None,
                        result: None,
                        run_index: 0,
                        completed_at: None,
                        structured_output: None,
                        artifact_contract: None,
                        failure_kind: None,
                        failure_disposition: None,
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
                    ps.state = STEP_STATE_RUNNING.to_string();
                    ps.session_id = Some(session_id.clone());
                    ps.result = None;
                    ps.run_index = *execution_count;
                    ps.completed_at = None;
                } else {
                    active_parallel_steps.push(ParallelStepState {
                        step_name: child_node_name.clone(),
                        state: STEP_STATE_RUNNING.to_string(),
                        session_id: Some(session_id.clone()),
                        result: None,
                        run_index: *execution_count,
                        completed_at: None,
                        structured_output: None,
                        artifact_contract: None,
                        failure_kind: None,
                        failure_disposition: None,
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
                state,
                failure_kind,
                failure_disposition,
                timestamp,
                ..
            } => {
                // [08] 先行する ArtifactProduced で確定済みの structured_output /
                //   artifact_contract は ParallelChildCompleted（prose 抽出廃止後は
                //   `structured_output: None` 固定）で上書きしない。reload 経路でも
                //   live と同じく「経路非依存に提出済み output を参照できる」
                //   （spec [08] Rule 3 Scenario 1）を満たすため、既存 slot から
                //   merge する。
                let prior = step_outputs.get(child_node_name).cloned();
                let output_merge = workflow_parallel::merge_parallel_child_completion_output(
                    structured_output.clone(),
                    prior.as_ref().and_then(|p| p.structured_output.clone()),
                    prior.as_ref().and_then(|p| p.artifact_contract.clone()),
                );
                let merged_structured_output = output_merge.structured_output;
                let merged_artifact_contract = output_merge.artifact_contract;
                if let Some(ps) = active_parallel_steps
                    .iter_mut()
                    .find(|p| p.step_name == *child_node_name)
                {
                    ps.state = state.clone();
                    ps.result = result.clone();
                    ps.completed_at = Some(*timestamp);
                    ps.structured_output = merged_structured_output.clone();
                    ps.artifact_contract = merged_artifact_contract.clone();
                    ps.failure_kind = *failure_kind;
                    ps.failure_disposition = *failure_disposition;
                }
                step_outputs.insert(
                    child_node_name.clone(),
                    StepOutput {
                        step_name: child_node_name.clone(),
                        run_index: *run_index,
                        session_id: Some(session_id.clone()),
                        result: result.clone(),
                        structured_output: merged_structured_output,
                        artifact_contract: merged_artifact_contract,
                        token_usage: token_usage.clone(),
                        completed_at: *timestamp,
                    },
                );
                if let Some(ref usage) = token_usage {
                    total_token_usage.add(usage);
                }
                stall_observations.retain(|observation| observation.session_id != *session_id);
                updated_at = *timestamp;
            }
            WorkflowEvent::ParallelCompleted {
                parent_node_name,
                aggregate_result,
                timestamp,
                ..
            } => {
                let parent_run_index = step_execution_counts
                    .get(parent_node_name)
                    .copied()
                    .unwrap_or_else(|| {
                        step_history
                            .iter()
                            .filter(|entry| entry.step_name == *parent_node_name)
                            .count() as u32
                            + 1
                    });
                let mut combined_tokens = TokenUsage::default();
                let mut child_tokens_seen = false;
                let mut children_output = serde_json::Map::new();
                let child_snapshots = active_parallel_steps
                    .iter()
                    .map(|child| {
                        let output = step_outputs.get(&child.step_name);
                        if let Some(usage) = output.and_then(|output| output.token_usage.as_ref()) {
                            child_tokens_seen = true;
                            combined_tokens.add(usage);
                        }
                        if let Some(output) = output {
                            children_output.insert(
                                child.step_name.clone(),
                                output
                                    .structured_output
                                    .clone()
                                    .unwrap_or(serde_json::Value::Null),
                            );
                        } else if let Some(structured_output) = child.structured_output.clone() {
                            children_output.insert(child.step_name.clone(), structured_output);
                        }
                        ChildOutputSnapshot {
                            step_name: child.step_name.clone(),
                            session_id: output
                                .and_then(|output| output.session_id.clone())
                                .or_else(|| child.session_id.clone()),
                            result: output
                                .and_then(|output| output.result.clone())
                                .or_else(|| child.result.clone()),
                            run_index: if child.run_index == 0 {
                                output.map(|output| output.run_index).unwrap_or(0)
                            } else {
                                child.run_index
                            },
                            completed_at: output
                                .map(|output| output.completed_at)
                                .or(child.completed_at)
                                .unwrap_or(*timestamp),
                            structured_output: child.structured_output.clone().or_else(|| {
                                output.and_then(|output| output.structured_output.clone())
                            }),
                            artifact_contract: child.artifact_contract.clone().or_else(|| {
                                output.and_then(|output| output.artifact_contract.clone())
                            }),
                            state: child.state.clone(),
                            failure_kind: child.failure_kind,
                            failure_disposition: child.failure_disposition,
                        }
                    })
                    .collect::<Vec<_>>();
                if !child_snapshots.is_empty() {
                    step_outputs.insert(
                        parent_node_name.clone(),
                        StepOutput {
                            step_name: parent_node_name.clone(),
                            run_index: parent_run_index,
                            session_id: None,
                            result: None,
                            structured_output: Some(serde_json::Value::Object(children_output)),
                            artifact_contract: None,
                            token_usage: Some(combined_tokens.clone()),
                            completed_at: *timestamp,
                        },
                    );
                    step_history.push(StepHistoryEntry {
                        step_name: parent_node_name.clone(),
                        completed_at: *timestamp,
                        result: Some(completed_parallel_history_result(aggregate_result)),
                        session_id: None,
                        token_usage: if child_tokens_seen {
                            Some(combined_tokens)
                        } else {
                            None
                        },
                        structured_output: None,
                        run_index: parent_run_index,
                        child_outputs: Some(child_snapshots),
                        state: crate::adaptor::gateway::workflow::state::default_step_entry_state(),
                    });
                }
                active_parallel_steps.clear();
                stall_observations.clear();
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
                node_name,
                contract,
                value,
                timestamp,
                ..
            } => {
                // [08] CLI / in-process 経由で確定した step output を state に復元する。
                // 後続 step が `input_reference` で経路非依存に参照できる shape に揃える。
                // `result` は engine の live state と同じ値（contract validator の戻り値）
                // を再導出する。これにより live と reload 経路で aggregate 評価が乖離しない。
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
        active_parallel_steps,
        approval_operations: None,
        stall_observations,
        started_at,
        updated_at,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adaptor::gateway::workflow::schema::{
        FacetRefs, NodeDefinition, NodeKind, SchemaDef, SessionSpec, Workflow,
    };

    fn agent_node(name: &str) -> NodeDefinition {
        NodeDefinition {
            name: name.to_string(),
            kind: session_kind("x"),
            ..NodeDefinition::default()
        }
    }

    fn session_kind(instruction: &str) -> NodeKind {
        NodeKind::Session(SessionSpec {
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

    #[test]
    fn projection_restores_workflow_stall_observations() {
        let run_id = "exec-stall";
        let workflow = workflow_with_nodes("wf", vec!["review"]);
        let events = vec![
            run_started(run_id, workflow),
            WorkflowEvent::NodeStarted {
                run_id: run_id.to_string(),
                workflow_name: "wf".to_string(),
                node_name: "review".to_string(),
                execution_count: 1,
                timestamp: 1001.0,
            },
            WorkflowEvent::StepSessionStarted {
                run_id: run_id.to_string(),
                workflow_name: "wf".to_string(),
                node_name: "review".to_string(),
                execution_count: 1,
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
                node_name: "review".to_string(),
                execution_count: 1,
                timestamp: 1001.0,
            },
            WorkflowEvent::StepSessionStarted {
                run_id: run_id.to_string(),
                workflow_name: "wf".to_string(),
                node_name: "review".to_string(),
                execution_count: 1,
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
                node_name: "review".to_string(),
                execution_count: 1,
                timestamp: 1001.0,
            },
            WorkflowEvent::StepSessionStarted {
                run_id: run_id.to_string(),
                workflow_name: "wf".to_string(),
                node_name: "review".to_string(),
                execution_count: 1,
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
                node_name: "review".to_string(),
                execution_count: 1,
                timestamp: 1001.0,
            },
            WorkflowEvent::StepSessionStarted {
                run_id: run_id.to_string(),
                workflow_name: "wf".to_string(),
                node_name: "review".to_string(),
                execution_count: 1,
                session_id: "session-1".to_string(),
                timestamp: 1002.0,
            },
            stall_observed(run_id, "session-1", "review"),
            WorkflowEvent::NodeCompleted {
                run_id: run_id.to_string(),
                workflow_name: "wf".to_string(),
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
                node_name: "plan".to_string(),
                execution_count: 1,
                timestamp: 1001.0,
            },
            WorkflowEvent::StepSessionStarted {
                run_id: run_id.to_string(),
                workflow_name: "wf".to_string(),
                node_name: "plan".to_string(),
                execution_count: 1,
                session_id: "session-plan".to_string(),
                timestamp: 1002.0,
            },
            stall_observed(run_id, "session-plan", "plan"),
            WorkflowEvent::NodeStarted {
                run_id: run_id.to_string(),
                workflow_name: "wf".to_string(),
                node_name: "review".to_string(),
                execution_count: 1,
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
                node_name: "review".to_string(),
                execution_count: 1,
                timestamp: 1001.0,
            },
            WorkflowEvent::StepSessionStarted {
                run_id: run_id.to_string(),
                workflow_name: "wf".to_string(),
                node_name: "review".to_string(),
                execution_count: 1,
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
    fn projection_clears_workflow_stall_observations_on_parallel_completion() {
        let run_id = "exec-stall-parallel";
        let workflow = workflow_with_nodes("wf", vec!["parallel-review"]);
        let events = vec![
            run_started(run_id, workflow),
            WorkflowEvent::ParallelStarted {
                run_id: run_id.to_string(),
                workflow_name: "wf".to_string(),
                parent_node_name: "parallel-review".to_string(),
                child_node_names: vec!["review-a".to_string(), "review-b".to_string()],
                timestamp: 1001.0,
            },
            stall_observed(run_id, "session-review-a", "review-a"),
            WorkflowEvent::ParallelCompleted {
                run_id: run_id.to_string(),
                workflow_name: "wf".to_string(),
                parent_node_name: "parallel-review".to_string(),
                aggregate_result: "advance".to_string(),
                timestamp: 1004.0,
            },
        ];

        let state = reconstruct_state_from_events(run_id, &events)
            .unwrap()
            .unwrap();

        assert!(state.stall_observations.is_empty());
        assert!(state.active_parallel_steps.is_empty());
    }

    #[test]
    fn projection_clears_workflow_stall_observation_on_parallel_child_completion() {
        let run_id = "exec-stall-parallel-child";
        let workflow = workflow_with_nodes("wf", vec!["parallel-review"]);
        let base_events = vec![
            run_started(run_id, workflow),
            WorkflowEvent::ParallelStarted {
                run_id: run_id.to_string(),
                workflow_name: "wf".to_string(),
                parent_node_name: "parallel-review".to_string(),
                child_node_names: vec!["review-a".to_string(), "review-b".to_string()],
                timestamp: 1001.0,
            },
            WorkflowEvent::ParallelChildStarted {
                run_id: run_id.to_string(),
                workflow_name: "wf".to_string(),
                parent_node_name: "parallel-review".to_string(),
                child_node_name: "review-a".to_string(),
                session_id: "session-review-a".to_string(),
                execution_count: 1,
                timestamp: 1002.0,
            },
            WorkflowEvent::ParallelChildStarted {
                run_id: run_id.to_string(),
                workflow_name: "wf".to_string(),
                parent_node_name: "parallel-review".to_string(),
                child_node_name: "review-b".to_string(),
                session_id: "session-review-b".to_string(),
                execution_count: 1,
                timestamp: 1002.0,
            },
            stall_observed(run_id, "session-review-a", "review-a"),
            stall_observed(run_id, "session-review-b", "review-b"),
            WorkflowEvent::ParallelChildCompleted {
                run_id: run_id.to_string(),
                workflow_name: "wf".to_string(),
                parent_node_name: "parallel-review".to_string(),
                child_node_name: "review-a".to_string(),
                session_id: "session-review-a".to_string(),
                run_index: 1,
                result: Some("LGTM".to_string()),
                token_usage: None,
                structured_output: None,
                state: STEP_STATE_COMPLETED.to_string(),
                failure_kind: None,
                failure_disposition: None,
                timestamp: 1004.0,
            },
        ];

        let state = reconstruct_state_from_events(run_id, &base_events)
            .unwrap()
            .unwrap();

        assert_eq!(state.stall_observations.len(), 1);
        assert_eq!(state.stall_observations[0].session_id, "session-review-b");

        let mut events = base_events;
        events.push(WorkflowEvent::ParallelChildCompleted {
            run_id: run_id.to_string(),
            workflow_name: "wf".to_string(),
            parent_node_name: "parallel-review".to_string(),
            child_node_name: "review-b".to_string(),
            session_id: "session-review-b".to_string(),
            run_index: 1,
            result: Some("model_refusal".to_string()),
            token_usage: None,
            structured_output: Some(serde_json::json!({
                "failureKind": "model_refusal",
                "disposition": "partial",
                "exitCode": 1,
            })),
            state: STEP_STATE_FAILED.to_string(),
            failure_kind: Some(WorkflowStepFailureKind::ModelRefusal),
            failure_disposition: Some(FailureDisposition::Partial),
            timestamp: 1005.0,
        });

        let state = reconstruct_state_from_events(run_id, &events)
            .unwrap()
            .unwrap();

        assert!(state.stall_observations.is_empty());
    }

    #[test]
    fn projection_restores_current_session_from_node_session_started() {
        let workflow = workflow_with_nodes("wf", vec!["implement"]);
        let events = vec![
            run_started("exec-1", workflow),
            WorkflowEvent::NodeStarted {
                run_id: "exec-1".to_string(),
                workflow_name: "wf".to_string(),
                node_name: "implement".to_string(),
                execution_count: 1,
                timestamp: 1001.0,
            },
            WorkflowEvent::StepSessionStarted {
                run_id: "exec-1".to_string(),
                workflow_name: "wf".to_string(),
                node_name: "implement".to_string(),
                execution_count: 1,
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
                failure_kind: WorkflowStepFailureKind::InfrastructureCrash,
                retry_count: None,
                timestamp: 2.0,
            },
            WorkflowEvent::RunFailed {
                run_id: "exec-pf".to_string(),
                workflow_name: "wf".to_string(),
                reason: "child failed".to_string(),
                failure_kind: WorkflowStepFailureKind::InfrastructureCrash,
                retry_count: None,
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

    /// `ParallelStarted` 後に `RunAborted` で終端した場合、`active_parallel_steps` は
    /// 空になり、parent step が "aborted" entry として step_history に積まれる。
    /// 未完了 child は child_outputs に "aborted" 状態で snapshot される。
    ///
    /// spec issues-1023: 中断時に走っていた parallel children も step_history に
    /// 集約することで、UI から session log への到達経路を保ち続ける。
    #[test]
    fn projection_clears_active_parallel_steps_on_run_aborted() {
        let snapshot = workflow_with_nodes("wf", vec!["parallel-review"]);
        let events = vec![
            run_started("exec-pa", snapshot),
            WorkflowEvent::NodeStarted {
                run_id: "exec-pa".to_string(),
                workflow_name: "wf".to_string(),
                node_name: "parallel-review".to_string(),
                execution_count: 1,
                timestamp: 1.0,
            },
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
                aborted_step: None,
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
        assert_eq!(
            state.step_history.len(),
            1,
            "RunAborted で parent entry が 1 件積まれる"
        );
        let parent = &state.step_history[0];
        assert_eq!(parent.step_name, "parallel-review");
        assert_eq!(parent.state, "aborted");
        let children = parent
            .child_outputs
            .as_ref()
            .expect("aborted parallel parent は child_outputs を持つ");
        assert_eq!(children.len(), 2, "全 child が snapshot される");
        for child in children {
            assert_eq!(
                child.state, "aborted",
                "未完了 child は aborted として snapshot される"
            );
        }
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
                node_name: "plan".to_string(),
                execution_count: 1,
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
                node_name: "plan".to_string(),
                execution_count: 1,
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

    /// spec issues-1023: parallel ブロック実行中に一部 child が完了し、残りが
    /// 未完了のまま `RunAborted` が来ると、parent entry の child_outputs は
    /// 完了 child を "completed"、未完了 child を "aborted" として snapshot する。
    #[test]
    fn projection_records_aborted_parallel_with_mixed_child_states() {
        let snapshot = workflow_with_nodes("wf", vec!["parallel-review"]);
        let events = vec![
            run_started("exec-mixed", snapshot),
            WorkflowEvent::NodeStarted {
                run_id: "exec-mixed".to_string(),
                workflow_name: "wf".to_string(),
                node_name: "parallel-review".to_string(),
                execution_count: 1,
                timestamp: 1.0,
            },
            WorkflowEvent::ParallelStarted {
                run_id: "exec-mixed".to_string(),
                workflow_name: "wf".to_string(),
                parent_node_name: "parallel-review".to_string(),
                child_node_names: vec!["a".to_string(), "b".to_string()],
                timestamp: 1.0,
            },
            WorkflowEvent::ParallelChildStarted {
                run_id: "exec-mixed".to_string(),
                workflow_name: "wf".to_string(),
                parent_node_name: "parallel-review".to_string(),
                child_node_name: "a".to_string(),
                session_id: "session-a".to_string(),
                execution_count: 1,
                timestamp: 1.1,
            },
            WorkflowEvent::ParallelChildStarted {
                run_id: "exec-mixed".to_string(),
                workflow_name: "wf".to_string(),
                parent_node_name: "parallel-review".to_string(),
                child_node_name: "b".to_string(),
                session_id: "session-b".to_string(),
                execution_count: 1,
                timestamp: 1.1,
            },
            WorkflowEvent::ParallelChildCompleted {
                run_id: "exec-mixed".to_string(),
                workflow_name: "wf".to_string(),
                parent_node_name: "parallel-review".to_string(),
                child_node_name: "a".to_string(),
                session_id: "session-a".to_string(),
                run_index: 1,
                result: Some("LGTM".to_string()),
                token_usage: None,
                structured_output: None,
                state: STEP_STATE_COMPLETED.to_string(),
                failure_kind: None,
                failure_disposition: None,
                timestamp: 1.5,
            },
            WorkflowEvent::RunAborted {
                run_id: "exec-mixed".to_string(),
                workflow_name: "wf".to_string(),
                aborted_step: None,
                timestamp: 2.0,
            },
        ];
        let state = reconstruct_state_from_events("exec-mixed", &events)
            .unwrap()
            .unwrap();
        assert_eq!(state.state, WorkflowExecutionState::Aborted);
        assert_eq!(state.step_history.len(), 1);
        let parent = &state.step_history[0];
        assert_eq!(parent.state, "aborted");
        let children = parent.child_outputs.as_ref().unwrap();
        assert_eq!(children.len(), 2);
        let child_a = children.iter().find(|c| c.step_name == "a").unwrap();
        let child_b = children.iter().find(|c| c.step_name == "b").unwrap();
        assert_eq!(
            child_a.state, "completed",
            "ParallelChildCompleted 済み child は completed のまま snapshot"
        );
        assert_eq!(
            child_a.session_id.as_deref(),
            Some("session-a"),
            "完了済み child の session_id が child_outputs に残る"
        );
        assert_eq!(child_b.state, "aborted", "未完了 child は aborted snapshot");
        assert_eq!(
            child_b.session_id.as_deref(),
            Some("session-b"),
            "未完了 child でも ParallelChildStarted で得た session_id が残る"
        );
    }

    #[test]
    fn projection_records_completed_parallel_child_sessions_in_parent_history() {
        let snapshot = workflow_with_nodes("wf", vec!["parallel-review", "done"]);
        let events = vec![
            run_started("exec-parallel-complete", snapshot),
            WorkflowEvent::NodeStarted {
                run_id: "exec-parallel-complete".to_string(),
                workflow_name: "wf".to_string(),
                node_name: "parallel-review".to_string(),
                execution_count: 1,
                timestamp: 1.0,
            },
            WorkflowEvent::ParallelStarted {
                run_id: "exec-parallel-complete".to_string(),
                workflow_name: "wf".to_string(),
                parent_node_name: "parallel-review".to_string(),
                child_node_names: vec!["review-a".to_string(), "review-b".to_string()],
                timestamp: 1.1,
            },
            WorkflowEvent::ParallelChildStarted {
                run_id: "exec-parallel-complete".to_string(),
                workflow_name: "wf".to_string(),
                parent_node_name: "parallel-review".to_string(),
                child_node_name: "review-a".to_string(),
                session_id: "session-review-a".to_string(),
                execution_count: 1,
                timestamp: 1.2,
            },
            WorkflowEvent::ParallelChildStarted {
                run_id: "exec-parallel-complete".to_string(),
                workflow_name: "wf".to_string(),
                parent_node_name: "parallel-review".to_string(),
                child_node_name: "review-b".to_string(),
                session_id: "session-review-b".to_string(),
                execution_count: 1,
                timestamp: 1.2,
            },
            WorkflowEvent::ArtifactProduced {
                run_id: "exec-parallel-complete".to_string(),
                workflow_name: "wf".to_string(),
                node_name: "review-a".to_string(),
                contract: Some("review-verdict".to_string()),
                value: serde_json::json!({ "verdict": "LGTM" }),
                request_id: None,
                submitted_at: Some(1.3),
                timestamp: 1.3,
            },
            WorkflowEvent::ParallelChildCompleted {
                run_id: "exec-parallel-complete".to_string(),
                workflow_name: "wf".to_string(),
                parent_node_name: "parallel-review".to_string(),
                child_node_name: "review-a".to_string(),
                session_id: "session-review-a".to_string(),
                run_index: 1,
                result: Some("LGTM".to_string()),
                token_usage: Some(TokenUsage {
                    input_tokens: 10,
                    output_tokens: 3,
                }),
                structured_output: None,
                state: STEP_STATE_COMPLETED.to_string(),
                failure_kind: None,
                failure_disposition: None,
                timestamp: 1.5,
            },
            WorkflowEvent::ParallelChildCompleted {
                run_id: "exec-parallel-complete".to_string(),
                workflow_name: "wf".to_string(),
                parent_node_name: "parallel-review".to_string(),
                child_node_name: "review-b".to_string(),
                session_id: "session-review-b".to_string(),
                run_index: 1,
                result: Some("LGTM".to_string()),
                token_usage: Some(TokenUsage {
                    input_tokens: 7,
                    output_tokens: 2,
                }),
                structured_output: Some(serde_json::json!({ "verdict": "LGTM" })),
                state: STEP_STATE_COMPLETED.to_string(),
                failure_kind: None,
                failure_disposition: None,
                timestamp: 1.6,
            },
            WorkflowEvent::ParallelCompleted {
                run_id: "exec-parallel-complete".to_string(),
                workflow_name: "wf".to_string(),
                parent_node_name: "parallel-review".to_string(),
                aggregate_result: "advance".to_string(),
                timestamp: 1.7,
            },
        ];

        let state = reconstruct_state_from_events("exec-parallel-complete", &events)
            .unwrap()
            .unwrap();
        assert!(
            state.active_parallel_steps.is_empty(),
            "ParallelCompleted 後は active_parallel_steps を空にする"
        );
        assert_eq!(state.step_history.len(), 1);
        let parent = &state.step_history[0];
        assert_eq!(parent.step_name, "parallel-review");
        assert_eq!(parent.result.as_deref(), Some("complete"));
        assert_eq!(parent.run_index, 1);
        assert_eq!(
            parent.token_usage.as_ref().map(|usage| usage.input_tokens),
            Some(17)
        );
        assert_eq!(
            parent.token_usage.as_ref().map(|usage| usage.output_tokens),
            Some(5)
        );
        let children = parent
            .child_outputs
            .as_ref()
            .expect("completed parallel parent は child_outputs を持つ");
        assert_eq!(children.len(), 2);
        let child_a = children
            .iter()
            .find(|child| child.step_name == "review-a")
            .expect("review-a snapshot");
        assert_eq!(child_a.session_id.as_deref(), Some("session-review-a"));
        assert_eq!(child_a.state.as_str(), STEP_STATE_COMPLETED);
        assert_eq!(
            child_a.structured_output,
            Some(serde_json::json!({ "verdict": "LGTM" }))
        );
        assert_eq!(child_a.artifact_contract.as_deref(), Some("review-verdict"));
        let child_b = children
            .iter()
            .find(|child| child.step_name == "review-b")
            .expect("review-b snapshot");
        assert_eq!(child_b.session_id.as_deref(), Some("session-review-b"));
        assert_eq!(child_b.state.as_str(), STEP_STATE_COMPLETED);
    }

    #[test]
    fn projection_preserves_delegated_parallel_child_failure_state() {
        let snapshot = workflow_with_nodes("wf", vec!["parallel-review", "done"]);
        let events = vec![
            run_started("exec-parallel-partial", snapshot),
            WorkflowEvent::NodeStarted {
                run_id: "exec-parallel-partial".to_string(),
                workflow_name: "wf".to_string(),
                node_name: "parallel-review".to_string(),
                execution_count: 1,
                timestamp: 1.0,
            },
            WorkflowEvent::ParallelStarted {
                run_id: "exec-parallel-partial".to_string(),
                workflow_name: "wf".to_string(),
                parent_node_name: "parallel-review".to_string(),
                child_node_names: vec!["review-a".to_string()],
                timestamp: 1.1,
            },
            WorkflowEvent::ParallelChildStarted {
                run_id: "exec-parallel-partial".to_string(),
                workflow_name: "wf".to_string(),
                parent_node_name: "parallel-review".to_string(),
                child_node_name: "review-a".to_string(),
                session_id: "session-review-a".to_string(),
                execution_count: 1,
                timestamp: 1.2,
            },
            WorkflowEvent::ParallelChildCompleted {
                run_id: "exec-parallel-partial".to_string(),
                workflow_name: "wf".to_string(),
                parent_node_name: "parallel-review".to_string(),
                child_node_name: "review-a".to_string(),
                session_id: "session-review-a".to_string(),
                run_index: 1,
                result: Some("model_refusal: provider policy".to_string()),
                token_usage: None,
                structured_output: Some(serde_json::json!({
                    "failure_kind": "model_refusal",
                    "failure_disposition": "partial",
                })),
                state: STEP_STATE_FAILED.to_string(),
                failure_kind: Some(WorkflowStepFailureKind::ModelRefusal),
                failure_disposition: Some(FailureDisposition::Partial),
                timestamp: 1.5,
            },
            WorkflowEvent::ParallelCompleted {
                run_id: "exec-parallel-partial".to_string(),
                workflow_name: "wf".to_string(),
                parent_node_name: "parallel-review".to_string(),
                aggregate_result: "fix".to_string(),
                timestamp: 1.7,
            },
        ];

        let state = reconstruct_state_from_events("exec-parallel-partial", &events)
            .unwrap()
            .unwrap();

        let parent = state.step_history.first().expect("parallel parent history");
        let child = parent
            .child_outputs
            .as_ref()
            .and_then(|children| children.iter().find(|child| child.step_name == "review-a"))
            .expect("delegated failed child snapshot");
        assert_eq!(child.state.as_str(), STEP_STATE_FAILED);
        assert_eq!(
            child.result.as_deref(),
            Some("model_refusal: provider policy")
        );
        assert_eq!(
            child.structured_output,
            Some(serde_json::json!({
                "failure_kind": "model_refusal",
                "failure_disposition": "partial",
            }))
        );
        assert_eq!(
            child.failure_kind,
            Some(WorkflowStepFailureKind::ModelRefusal)
        );
        assert_eq!(child.failure_disposition, Some(FailureDisposition::Partial));
    }

    #[test]
    fn projection_keeps_submitted_parallel_child_output_on_child_completed() {
        let snapshot = workflow_with_nodes("wf", vec!["parallel-review"]);
        let events = vec![
            run_started("exec-parallel-output", snapshot),
            WorkflowEvent::ParallelStarted {
                run_id: "exec-parallel-output".to_string(),
                workflow_name: "wf".to_string(),
                parent_node_name: "parallel-review".to_string(),
                child_node_names: vec!["review-a".to_string()],
                timestamp: 1.0,
            },
            WorkflowEvent::ParallelChildStarted {
                run_id: "exec-parallel-output".to_string(),
                workflow_name: "wf".to_string(),
                parent_node_name: "parallel-review".to_string(),
                child_node_name: "review-a".to_string(),
                session_id: "session-review-a".to_string(),
                execution_count: 1,
                timestamp: 1.1,
            },
            WorkflowEvent::ArtifactProduced {
                run_id: "exec-parallel-output".to_string(),
                workflow_name: "wf".to_string(),
                node_name: "review-a".to_string(),
                contract: Some("review-verdict".to_string()),
                value: serde_json::json!({ "verdict": "LGTM" }),
                request_id: None,
                submitted_at: Some(1.2),
                timestamp: 1.2,
            },
            WorkflowEvent::ParallelChildCompleted {
                run_id: "exec-parallel-output".to_string(),
                workflow_name: "wf".to_string(),
                parent_node_name: "parallel-review".to_string(),
                child_node_name: "review-a".to_string(),
                session_id: "session-review-a".to_string(),
                run_index: 1,
                result: Some("LGTM".to_string()),
                token_usage: None,
                structured_output: None,
                state: STEP_STATE_COMPLETED.to_string(),
                failure_kind: None,
                failure_disposition: None,
                timestamp: 1.5,
            },
        ];

        let state = reconstruct_state_from_events("exec-parallel-output", &events)
            .unwrap()
            .unwrap();
        let output = state.step_outputs.get("review-a").unwrap();
        assert_eq!(output.artifact_contract.as_deref(), Some("review-verdict"));
        assert_eq!(
            output.structured_output,
            Some(serde_json::json!({ "verdict": "LGTM" }))
        );
        let active_child = state
            .active_parallel_steps
            .iter()
            .find(|step| step.step_name == "review-a")
            .unwrap();
        assert_eq!(
            active_child.artifact_contract.as_deref(),
            Some("review-verdict")
        );
        assert_eq!(
            active_child.structured_output,
            Some(serde_json::json!({ "verdict": "LGTM" }))
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
                node_name: "plan".to_string(),
                execution_count: 1,
                timestamp: 1001.0,
            },
            WorkflowEvent::CliMutationRequested {
                run_id: "exec-cli".to_string(),
                workflow_name: "wf".to_string(),
                request_id: "00000000-0000-0000-0000-000000000601".to_string(),
                request: CliMutationRequestRecord::Approve {
                    node_name: Some("plan".to_string()),
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
                node_name: "plan".to_string(),
                execution_count: 1,
                timestamp: 1000.0,
            },
            WorkflowEvent::NodeCompleted {
                run_id: "r".to_string(),
                workflow_name: "wf".to_string(),
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
    fn compute_step_timings_handles_parallel_children() {
        let events = vec![
            WorkflowEvent::ParallelChildStarted {
                run_id: "r".to_string(),
                workflow_name: "wf".to_string(),
                parent_node_name: "parent".to_string(),
                child_node_name: "child-a".to_string(),
                session_id: "s-a".to_string(),
                execution_count: 1,
                timestamp: 100.0,
            },
            WorkflowEvent::ParallelChildCompleted {
                run_id: "r".to_string(),
                workflow_name: "wf".to_string(),
                parent_node_name: "parent".to_string(),
                child_node_name: "child-a".to_string(),
                result: None,
                session_id: "s-a".to_string(),
                token_usage: None,
                structured_output: None,
                run_index: 1,
                state: STEP_STATE_COMPLETED.to_string(),
                failure_kind: None,
                failure_disposition: None,
                timestamp: 300.0,
            },
        ];
        let timings = compute_step_timings(&events);
        assert_eq!(timings.len(), 1);
        assert_eq!(timings[0].step_name, "child-a");
        assert_eq!(timings[0].duration_ms, Some(200_000.0));
    }

    #[test]
    fn compute_step_timings_uses_node_session_started_as_start_marker() {
        let events = vec![
            WorkflowEvent::StepSessionStarted {
                run_id: "r".to_string(),
                workflow_name: "wf".to_string(),
                node_name: "plan".to_string(),
                execution_count: 1,
                session_id: "s1".to_string(),
                timestamp: 100.0,
            },
            WorkflowEvent::NodeCompleted {
                run_id: "r".to_string(),
                workflow_name: "wf".to_string(),
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
                node_name: "plan".to_string(),
                execution_count: 1,
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
                node_name: "plan".to_string(),
                execution_count: 1,
                timestamp: 1100.0,
            },
            WorkflowEvent::NodeCompleted {
                run_id: "exec-detail".to_string(),
                workflow_name: "wf".to_string(),
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
                node_name: "review".to_string(),
                execution_count: 1,
                timestamp: 1300.0,
            },
            WorkflowEvent::NodeCompleted {
                run_id: "exec-detail".to_string(),
                workflow_name: "wf".to_string(),
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
                node_name: "plan".to_string(),
                execution_count: 1,
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
                node_name: "review".to_string(),
                execution_count: 1,
                timestamp: 1001.0,
            },
            WorkflowEvent::ArtifactProduced {
                run_id: "exec-submit".to_string(),
                workflow_name: "wf".to_string(),
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
                node_name: "review".to_string(),
                execution_count: 1,
                timestamp: 1001.0,
            },
            WorkflowEvent::ArtifactProduced {
                run_id: "exec-submit-complete".to_string(),
                workflow_name: "wf".to_string(),
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
                node_name: "spec".to_string(),
                execution_count: 1,
                timestamp: 1001.0,
            },
            WorkflowEvent::ArtifactProduced {
                run_id: "exec-spec".to_string(),
                workflow_name: "wf".to_string(),
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
