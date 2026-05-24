//! [04] Command / Event Boundary: `WorkflowEvent` 列から `WorkflowState` を再構築する projection。
//!
//! spec の責務配置に従い、`workflow/log.rs` は NDJSON の append/read 機構へ責務を限定し、
//! event 列 → WorkflowState の射影 (projection) は engine 側の本モジュールに置く。
//! 過去 NDJSON 在庫の互換性は spec [02]/[04] の範囲で別途扱う。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::workflow::event::{
    ApprovalDecisionRecord, CliMutationRejectionReason, CliMutationRequestRecord,
    CollectedOutputEntry, TokenUsage as EventTokenUsage, WorkflowEvent,
};
use crate::workflow::schema::{NodeType, Workflow};
use crate::workflow::state::{
    ParallelStepState, StepHistoryEntry, StepOutput, TokenUsage, WorkflowExecutionState,
    WorkflowState,
};

/// 秒単位の f64 タイムスタンプ（engine 内 `current_timestamp()` 由来）を
/// frontend 表示用のミリ秒単位に変換するための係数。
const SECONDS_TO_MS: f64 = 1000.0;

#[inline]
fn seconds_to_ms(value: f64) -> f64 {
    value * SECONDS_TO_MS
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

/// spec [08]: 提出済みの構造化出力スナップショット。CLI / Tauri の `get` 入口が
/// それぞれ別 DTO（`OutputGetView` / `WorkflowGetOutputResponse`）に map するための
/// 中間型として、最新の `OutputSubmitted` から抽出した contract / structured_output /
/// 付随メタを保持する。本構造体は engine projection 由来の事実情報のみを持ち、
/// 表示用フォーマット変換は呼び出し側に閉じる。
#[derive(Debug, Clone, PartialEq)]
pub struct OutputSubmittedSnapshot {
    pub contract: String,
    pub structured_output: serde_json::Value,
    pub submitted_at: Option<f64>,
    pub request_id: Option<String>,
    pub timestamp: f64,
}

/// spec [08] Rule 3: 指定 step に対する最新の `OutputSubmitted` event を返す。
///
/// 同一 step に複数回 submit があった場合は最後（最新）の event を採用する。
/// 該当 step に対する `OutputSubmitted` が一切なければ `None`（呼び出し側で
/// 「未提出」として表現する）。
pub fn latest_output_submitted_for(
    events: &[WorkflowEvent],
    step: &str,
) -> Option<OutputSubmittedSnapshot> {
    events.iter().rev().find_map(|event| match event {
        WorkflowEvent::OutputSubmitted {
            node_name,
            contract,
            structured_output,
            submitted_at,
            request_id,
            timestamp,
            ..
        } if node_name == step => Some(OutputSubmittedSnapshot {
            contract: contract.clone(),
            structured_output: structured_output.clone(),
            submitted_at: *submitted_at,
            request_id: request_id.clone(),
            timestamp: *timestamp,
        }),
        _ => None,
    })
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
pub enum WorkflowEventView {
    RunStarted {
        run_id: String,
        workflow_name: String,
        workflow_file_stem: String,
        worktree_path: String,
        workflow_definition: Workflow,
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
        #[serde(rename = "timestampMs")]
        timestamp_ms: f64,
    },
    RunAborted {
        run_id: String,
        workflow_name: String,
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
    OutputSubmitted {
        run_id: String,
        workflow_name: String,
        node_name: String,
        contract: String,
        structured_output: serde_json::Value,
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

impl From<WorkflowEvent> for WorkflowEventView {
    fn from(event: WorkflowEvent) -> Self {
        match event {
            WorkflowEvent::RunStarted {
                run_id,
                workflow_name,
                workflow_file_stem,
                worktree_path,
                workflow_definition,
                timestamp,
            } => WorkflowEventView::RunStarted {
                run_id,
                workflow_name,
                workflow_file_stem,
                worktree_path,
                workflow_definition,
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
                timestamp,
            } => WorkflowEventView::NodeFailed {
                run_id,
                workflow_name,
                node_name,
                reason,
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
                timestamp,
            } => WorkflowEventView::RunFailed {
                run_id,
                workflow_name,
                reason,
                timestamp_ms: seconds_to_ms(timestamp),
            },
            WorkflowEvent::RunAborted {
                run_id,
                workflow_name,
                timestamp,
            } => WorkflowEventView::RunAborted {
                run_id,
                workflow_name,
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
                attempt,
                violation_reason,
                timestamp,
            } => WorkflowEventView::ContractRepairRequested {
                run_id,
                workflow_name,
                node_name,
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
            WorkflowEvent::OutputSubmitted {
                run_id,
                workflow_name,
                node_name,
                contract,
                structured_output,
                request_id,
                submitted_at,
                timestamp,
            } => WorkflowEventView::OutputSubmitted {
                run_id,
                workflow_name,
                node_name,
                contract,
                structured_output,
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
    pub output_contract: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub input_contracts: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_step_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_step_structured_output: Option<serde_json::Value>,
}

fn node_type_label(t: NodeType) -> &'static str {
    match t {
        NodeType::Agent => "agent",
        NodeType::Bash => "bash",
        NodeType::Approval => "approval",
        NodeType::Parallel => "parallel",
    }
}

fn node_input_from_definition(
    workflow: &Workflow,
    node_name: &str,
) -> (Option<&'static str>, WorkflowStepInputView) {
    // top-level node
    if let Some(node) = workflow.nodes.iter().find(|n| n.name == node_name) {
        let mut view = WorkflowStepInputView {
            instruction: node.instruction.clone(),
            policy: node.policy.clone(),
            knowledge: node.knowledge.clone(),
            output_contract: node.output_contract.clone(),
            input_contracts: node.input_contracts.clone().unwrap_or_default(),
            ..WorkflowStepInputView::default()
        };
        // 直前 top-level node の出力を input として参照する境界。
        if let Some(idx) = workflow.nodes.iter().position(|n| n.name == node_name) {
            if idx > 0 {
                view.previous_step_name = Some(workflow.nodes[idx - 1].name.clone());
            }
        }
        return (Some(node_type_label(node.node_type)), view);
    }
    // parallel child node
    for parent in &workflow.nodes {
        if let Some(children) = &parent.parallel_children {
            if let Some(child) = children.iter().find(|c| c.name == node_name) {
                let view = WorkflowStepInputView {
                    instruction: child.instruction.clone(),
                    policy: child.policy.clone(),
                    knowledge: child.knowledge.clone(),
                    output_contract: child.output_contract.clone(),
                    input_contracts: child.input_contracts.clone().unwrap_or_default(),
                    previous_step_name: Some(parent.name.clone()),
                    ..WorkflowStepInputView::default()
                };
                return (Some(node_type_label(child.node_type)), view);
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
                .unwrap_or_else(|| "pending".to_string()),
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
    let mut workflow_variables: HashMap<String, String> = HashMap::new();
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
                    state: crate::workflow::state::default_step_entry_state(),
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

                // spec issues-1023: 中断時に走っていた current step / parallel
                // children を `step_history` に "aborted" 状態として記録する。
                // session log への到達経路（child の session_id）を残すために
                // active_parallel_steps の snapshot を child_outputs に転写する。
                // 通常 step 経路では projection 上 `current_session_id` は
                // 追えないため、entry の session_id は None になる（ライブ
                // engine 経路では engine 側で session_id を入れる）。
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
                    let child_snapshots: Vec<crate::workflow::state::ChildOutputSnapshot> =
                        active_parallel_steps
                            .iter()
                            .map(|child| {
                                let snapshot_state = if child.state == "completed" {
                                    "completed"
                                } else {
                                    "aborted"
                                };
                                crate::workflow::state::ChildOutputSnapshot {
                                    step_name: child.step_name.clone(),
                                    session_id: child.session_id.clone(),
                                    result: child.result.clone(),
                                    run_index: child.run_index,
                                    completed_at: child.completed_at.unwrap_or(*timestamp),
                                    structured_output: child.structured_output.clone(),
                                    output_contract: child.output_contract.clone(),
                                    state: snapshot_state.to_string(),
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
                        state: "aborted".to_string(),
                    });
                } else if current_started && !already_in_history && !current_step_name.is_empty() {
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
                        state: "aborted".to_string(),
                    });
                }

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
                // [08] 先行する OutputSubmitted で確定済みの structured_output /
                //   output_contract は ParallelChildCompleted（prose 抽出廃止後は
                //   `structured_output: None` 固定）で上書きしない。reload 経路でも
                //   live と同じく「経路非依存に提出済み output を参照できる」
                //   （spec [08] Rule 3 Scenario 1）を満たすため、既存 slot から
                //   merge する。
                let prior = step_outputs.get(child_node_name).cloned();
                let merged_structured_output = structured_output
                    .clone()
                    .or_else(|| prior.as_ref().and_then(|p| p.structured_output.clone()));
                let merged_output_contract = prior.as_ref().and_then(|p| p.output_contract.clone());
                if let Some(ps) = active_parallel_steps
                    .iter_mut()
                    .find(|p| p.step_name == *child_node_name)
                {
                    ps.state = "completed".to_string();
                    ps.result = result.clone();
                    ps.completed_at = Some(*timestamp);
                    ps.structured_output = merged_structured_output.clone();
                    ps.output_contract = merged_output_contract.clone();
                }
                step_outputs.insert(
                    child_node_name.clone(),
                    StepOutput {
                        step_name: child_node_name.clone(),
                        run_index: *run_index,
                        session_id: Some(session_id.clone()),
                        result: result.clone(),
                        structured_output: merged_structured_output,
                        output_contract: merged_output_contract,
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
            WorkflowEvent::CliMutationRequested { .. } => {
                // [06] CLI mutation 要求の事実は append-only な観測情報のみで、
                // engine domain state には影響しない。
            }
            WorkflowEvent::OutputSubmitted {
                node_name,
                contract,
                structured_output,
                timestamp,
                ..
            } => {
                // [08] CLI / in-process 経由で確定した step output を state に復元する。
                // 後続 step が `pass_output_from` で経路非依存に参照できる shape に揃える。
                // `result` は engine の live state と同じ値（contract validator の戻り値）
                // を再導出する。これにより live と reload 経路で aggregate 評価が乖離しない。
                let ri = step_execution_counts.get(node_name).copied().unwrap_or(0);
                let restored_result = match crate::workflow::contract::validate_contract_value(
                    contract,
                    structured_output.clone(),
                ) {
                    crate::workflow::contract::ContractValidationResult::Valid {
                        result, ..
                    } => result,
                    // append-only ログに記録された OutputSubmitted は engine 側で validator を
                    // 通過しているため通常ここには到達しない。validator が将来変更されて
                    // 不適合判定になっても、result を None にして live と同等に振る舞う
                    // （aggregate 評価では match なしになるだけ）。
                    crate::workflow::contract::ContractValidationResult::Invalid(_) => None,
                };
                step_outputs.insert(
                    node_name.clone(),
                    StepOutput {
                        step_name: node_name.clone(),
                        run_index: ri,
                        session_id: None,
                        result: restored_result,
                        structured_output: Some(structured_output.clone()),
                        output_contract: Some(contract.clone()),
                        token_usage: None,
                        completed_at: *timestamp,
                    },
                );
                // contract 由来の workflow_variables を反映（spec-file-path のみ）。
                if contract == "spec-file-path" {
                    if let Some(path) = structured_output
                        .get("spec_file_path")
                        .and_then(|v| v.as_str())
                    {
                        workflow_variables.insert("spec_file_path".to_string(), path.to_string());
                    }
                }
                updated_at = *timestamp;
            }
            // [06] / [08] CliMutationRejected は観測経路用の補助履歴であり、
            // engine の workflow state には影響しない（accepted 経路の event のみが
            // 一次表現）。projection では no-op として扱う（5-3 / 5-4 修正）。
            WorkflowEvent::CliMutationRejected { .. } => {}
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
        workflow_variables,
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
                timestamp: 1.5,
            },
            WorkflowEvent::RunAborted {
                run_id: "exec-mixed".to_string(),
                workflow_name: "wf".to_string(),
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

    /// [06] CLI 経由 mutation 要求の事実は append-only な観測情報であり、engine
    /// domain state には影響しない（spec [06] 観測経路境界 / 既存 `ApprovalResolved`
    /// / `RunAborted` が実 state 変化の事実を担う境界を温存する）。projection 上で
    /// domain state / updated_at が変化しないことを境界として担保する。
    #[test]
    fn projection_treats_cli_mutation_requested_as_observation_only() {
        use crate::workflow::event::CliMutationRequestRecord;
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
                timestamp: 300.0,
            },
        ];
        let timings = compute_step_timings(&events);
        assert_eq!(timings.len(), 1);
        assert_eq!(timings[0].step_name, "child-a");
        assert_eq!(timings[0].duration_ms, Some(200_000.0));
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
        workflow.nodes[1].instruction = Some("review the diff".to_string());
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
        assert_eq!(detail.node_type, "agent");
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
        workflow.nodes[1].instruction = Some("review later".to_string());
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

    /// [08] OutputSubmitted projection: live engine と同じ shape で `StepOutput` slot を
    /// 復元する。`result` は contract validator を再導出するため、reload 経路でも
    /// `pass_output_from` で経路非依存に参照できる（spec [08] Rule 3 Scenario 1/3）。
    #[test]
    fn projection_restores_step_output_from_output_submitted_with_validator_derived_result() {
        let mut workflow = workflow_with_nodes("wf", vec!["review"]);
        workflow.nodes[0].output_contract = Some("review-verdict".to_string());
        let events = vec![
            run_started("exec-submit", workflow),
            WorkflowEvent::NodeStarted {
                run_id: "exec-submit".to_string(),
                workflow_name: "wf".to_string(),
                node_name: "review".to_string(),
                execution_count: 1,
                timestamp: 1001.0,
            },
            WorkflowEvent::OutputSubmitted {
                run_id: "exec-submit".to_string(),
                workflow_name: "wf".to_string(),
                node_name: "review".to_string(),
                contract: "review-verdict".to_string(),
                structured_output: serde_json::json!({"verdict": "LGTM"}),
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
            .expect("OutputSubmitted should restore step_outputs slot");
        assert_eq!(so.output_contract.as_deref(), Some("review-verdict"));
        assert!(so.structured_output.is_some());
        // validator から再導出された result を持つ（live と同じ shape）。
        assert_eq!(so.result.as_deref(), Some("LGTM"));
    }

    fn output_submitted_event(
        step: &str,
        value: serde_json::Value,
        request_id: Option<&str>,
        submitted_at: Option<f64>,
        timestamp: f64,
    ) -> WorkflowEvent {
        WorkflowEvent::OutputSubmitted {
            run_id: "run-1".to_string(),
            workflow_name: "wf".to_string(),
            node_name: step.to_string(),
            contract: "test-contract".to_string(),
            structured_output: value,
            request_id: request_id.map(str::to_string),
            submitted_at,
            timestamp,
        }
    }

    /// spec [08] Rule 3: 同一 step に複数回 submit があった場合は最新の提出が返る。
    #[test]
    fn latest_output_submitted_for_picks_latest_when_multiple_submitted() {
        let events = vec![
            output_submitted_event(
                "review",
                serde_json::json!({"verdict": "FIRST"}),
                Some("req-1"),
                Some(1.0),
                10.0,
            ),
            output_submitted_event(
                "review",
                serde_json::json!({"verdict": "LATEST"}),
                Some("req-2"),
                Some(2.0),
                20.0,
            ),
        ];

        let snapshot = latest_output_submitted_for(&events, "review")
            .expect("latest OutputSubmitted should be returned");
        assert_eq!(snapshot.contract, "test-contract");
        assert_eq!(
            snapshot.structured_output,
            serde_json::json!({"verdict": "LATEST"})
        );
        assert_eq!(snapshot.request_id.as_deref(), Some("req-2"));
        assert_eq!(snapshot.submitted_at, Some(2.0));
        assert_eq!(snapshot.timestamp, 20.0);
    }

    /// spec [08] Rule 3: 別 step に対する OutputSubmitted は対象 step の最新提出に影響しない。
    #[test]
    fn latest_output_submitted_for_ignores_other_steps() {
        let events = vec![
            output_submitted_event(
                "other",
                serde_json::json!({"unused": true}),
                None,
                None,
                30.0,
            ),
            output_submitted_event(
                "review",
                serde_json::json!({"verdict": "TARGET"}),
                None,
                None,
                25.0,
            ),
        ];

        let snapshot = latest_output_submitted_for(&events, "review")
            .expect("OutputSubmitted for review step should be returned");
        assert_eq!(
            snapshot.structured_output,
            serde_json::json!({"verdict": "TARGET"})
        );
        assert_eq!(snapshot.timestamp, 25.0);
    }

    /// spec [08] Rule 3 Scenario 2: 該当 step に OutputSubmitted が一切なければ None。
    #[test]
    fn latest_output_submitted_for_returns_none_when_no_match() {
        let events = vec![output_submitted_event(
            "other",
            serde_json::json!({}),
            None,
            None,
            10.0,
        )];

        assert!(latest_output_submitted_for(&events, "review").is_none());
        assert!(latest_output_submitted_for(&[], "review").is_none());
    }
}
