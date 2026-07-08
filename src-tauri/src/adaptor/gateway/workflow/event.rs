//! [04] Command / Event Boundary: workflow engine が発行する append-only な事実列の型。
//!
//! 旧 `WorkflowLogEvent` 列挙体は本 issue で完全廃止し、本ファイルの `WorkflowEvent` に
//! 完全置換する。旧 NDJSON 在庫は破棄前提（互換 wrapper は導入しない）。
//!
//! 仕様詳細は `docs/spec/issues-1013.md` / `docs/workflow-engine-model-boundary.md` 参照。
//! `CompleteNode` / `FailNode` に相当する内部 typed 遷移 command は [05] で導入する。

use serde::{Deserialize, Serialize};

use crate::adaptor::gateway::workflow::failure_wire::default_failure_kind;
use crate::adaptor::gateway::workflow::schema::Workflow;
use crate::domain::workflow::{FailureDisposition, WorkflowStepFailureKind, STEP_STATE_COMPLETED};

fn default_parallel_child_completed_state() -> String {
    STEP_STATE_COMPLETED.to_string()
}

fn default_contract_repair_run_index() -> u32 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

impl TokenUsage {
    pub fn add(&mut self, other: &TokenUsage) {
        self.input_tokens += other.input_tokens;
        self.output_tokens += other.output_tokens;
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RunAbortedStepSnapshot {
    pub step_name: String,
    pub completed_at: f64,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub result: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub token_usage: Option<TokenUsage>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub structured_output: Option<serde_json::Value>,
    #[serde(default)]
    pub run_index: u32,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub child_outputs: Option<Vec<RunAbortedChildOutputSnapshot>>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RunAbortedChildOutcome {
    Completed,
    Aborted,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RunAbortedChildOutputSnapshot {
    pub step_name: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub result: Option<String>,
    pub run_index: u32,
    pub completed_at: f64,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub structured_output: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub artifact_contract: Option<String>,
    #[serde(alias = "state")]
    pub outcome: RunAbortedChildOutcome,
}

/// workflow engine が発行する append-only な事実列の型。
///
/// `run_id` を主語とし、過去事実は書き換えない（撤回も追加 event として表現する）。
/// NDJSON 永続化時の tag は `event` フィールドに snake_case で出力される。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "event")]
pub enum WorkflowEvent {
    /// start workflow primitive を engine が受理し、新しい run を開始した。
    RunStarted {
        run_id: String,
        workflow_name: String,
        workflow_file_stem: String,
        worktree_path: String,
        /// reconstruct 経路の必須フィールド（[02] 互換境界）。
        workflow_definition: Workflow,
        timestamp: f64,
    },
    /// node が実行開始された（agent / approval / bash いずれも対象）。
    NodeStarted {
        run_id: String,
        workflow_name: String,
        node_name: String,
        execution_count: u32,
        timestamp: f64,
    },
    /// 逐次 step の AgentSession が起動された（session_id を観測経路に露出する）。
    /// `ParallelChildStarted` 相当の単発版で、event projection から
    /// `current_session_id` を populate するために用いる。
    StepSessionStarted {
        run_id: String,
        workflow_name: String,
        node_name: String,
        execution_count: u32,
        session_id: String,
        timestamp: f64,
    },
    /// agent session の無出力 timeout を workflow の非終端観測材料として記録した。
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
        timestamp: f64,
    },
    /// agent session の出力/keepalive/permission resume により無出力観測が解消された。
    WorkflowStallCleared {
        run_id: String,
        workflow_name: String,
        chat_session_id: String,
        timestamp: f64,
    },
    /// node が完了した（approval 経由の completion も含む）。
    NodeCompleted {
        run_id: String,
        workflow_name: String,
        node_name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        result: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        token_usage: Option<TokenUsage>,
        #[serde(skip_serializing_if = "Option::is_none")]
        structured_output: Option<serde_json::Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        run_index: Option<u32>,
        timestamp: f64,
    },
    /// node が失敗した。
    NodeFailed {
        run_id: String,
        workflow_name: String,
        node_name: String,
        reason: String,
        #[serde(default = "default_failure_kind")]
        failure_kind: WorkflowStepFailureKind,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        retry_count: Option<u32>,
        timestamp: f64,
    },
    /// approval runtime primitive の受理直前に、approval 対象の到達を記録する。
    ApprovalRequested {
        run_id: String,
        workflow_name: String,
        node_name: String,
        timestamp: f64,
    },
    /// approval node に対するユーザー判断（approve / reject / abort）が受理された。
    ApprovalResolved {
        run_id: String,
        workflow_name: String,
        node_name: String,
        decision: ApprovalDecisionRecord,
        #[serde(skip_serializing_if = "Option::is_none")]
        comment: Option<String>,
        timestamp: f64,
    },
    /// run 全体が成功完了した。
    RunCompleted {
        run_id: String,
        workflow_name: String,
        total_token_usage: TokenUsage,
        timestamp: f64,
    },
    /// run 全体が失敗終了した。
    RunFailed {
        run_id: String,
        workflow_name: String,
        reason: String,
        #[serde(default = "default_failure_kind")]
        failure_kind: WorkflowStepFailureKind,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        retry_count: Option<u32>,
        timestamp: f64,
    },
    /// abort runtime primitive の受理結果として run が中断された。
    RunAborted {
        run_id: String,
        workflow_name: String,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        aborted_step: Option<RunAbortedStepSnapshot>,
        timestamp: f64,
    },
    /// collect step の reduce 結果。
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
        timestamp: f64,
    },
    /// 並列ブロックが開始された（parent node の入口マーカー）。
    ParallelStarted {
        run_id: String,
        workflow_name: String,
        parent_node_name: String,
        child_node_names: Vec<String>,
        timestamp: f64,
    },
    /// 並列ブロックの子 node が実行開始された。
    ParallelChildStarted {
        run_id: String,
        workflow_name: String,
        parent_node_name: String,
        child_node_name: String,
        session_id: String,
        execution_count: u32,
        timestamp: f64,
    },
    /// 並列ブロックの子 node が完了した。
    ParallelChildCompleted {
        run_id: String,
        workflow_name: String,
        parent_node_name: String,
        child_node_name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        result: Option<String>,
        session_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        token_usage: Option<TokenUsage>,
        #[serde(skip_serializing_if = "Option::is_none")]
        structured_output: Option<serde_json::Value>,
        run_index: u32,
        #[serde(default = "default_parallel_child_completed_state")]
        state: String,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        failure_kind: Option<WorkflowStepFailureKind>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        failure_disposition: Option<FailureDisposition>,
        timestamp: f64,
    },
    /// 並列ブロック全体が完了し、aggregate 評価結果に基づき遷移する。
    ParallelCompleted {
        run_id: String,
        workflow_name: String,
        parent_node_name: String,
        aggregate_result: String,
        timestamp: f64,
    },
    /// artifact_contract repair prompt が送信された。
    ContractRepairRequested {
        run_id: String,
        workflow_name: String,
        node_name: String,
        #[serde(default = "default_contract_repair_run_index")]
        run_index: u32,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        request_id: Option<String>,
        attempt: u32,
        violation_reason: String,
        timestamp: f64,
    },
    /// [06] CLI 経路で workflow run に対する mutation が engine に到達した事実。
    ///
    /// approval / abort mutation を CLI 経由で
    /// engine が受け取ったことを `run_id` 主語で append-only に記録する。stale target /
    /// validation reject でも engine 到達後の拒否は記録する。実 state
    /// 変化の事実は引き続き `ApprovalResolved` / `RunAborted` で表現し、本 variant
    /// は「いつ・どの経路から・何が要求されたか」を観測経路から透過的に読めるよう
    /// にするためのもの（spec [06] 観測経路境界）。
    ///
    /// `request` の自由記述（reason / comment）は平文で永続化される。秘匿情報を
    /// 含めない運用前提（spec [06] 要求の運用境界）。
    CliMutationRequested {
        run_id: String,
        workflow_name: String,
        /// 外部 caller が発行した state 変化要求の id。engine 側の重複 dispatch を
        /// 冪等化する key。pending file store の entry id は adapter 側で本値へ
        /// 写像し、event log には pending store の実装詳細を露出しない。
        request_id: String,
        request: CliMutationRequestRecord,
        /// CLI が pending command を書き出した時刻（caller 側 timestamp）。
        requested_at: f64,
        /// engine が本 event を受理・追記した時刻（commit timestamp）。
        timestamp: f64,
    },
    /// [08] step に対する構造化出力が contract 適合判定を経て確定した事実。
    ///
    /// CLI / in-process 経路が engine の submit-output primitive で受理され、
    /// contract 適合判定 → `step_outputs` / `workflow_variables` 更新と同一
    /// トランザクションで append される。contract 不適合・stale step・不在 step
    /// などの拒否は本 event を残さない（spec [08] ArtifactProduced append の
    /// 不可分性境界）。
    ArtifactProduced {
        run_id: String,
        workflow_name: String,
        node_name: String,
        /// 対象 step の `artifact_contract`。command stdout など contract を持たない
        /// Artifact では `None`。
        #[serde(skip_serializing_if = "Option::is_none", default)]
        contract: Option<String>,
        /// contract 適合判定を通過した Artifact 値。
        value: serde_json::Value,
        /// CLI pending command 経由で提出された場合の caller 側 request id。
        /// in-process 経路（Tauri command 等）で提出された場合は `None`。
        #[serde(skip_serializing_if = "Option::is_none", default)]
        request_id: Option<String>,
        /// caller が pending command を書き出した時刻（Unix 秒）。
        /// in-process 経路では `None`。
        #[serde(skip_serializing_if = "Option::is_none", default)]
        submitted_at: Option<f64>,
        /// engine が本 event を append した時刻。
        timestamp: f64,
    },
    /// [06] / [08] CLI 経路の mutation が engine 判断によって拒否された事実
    /// （5-3 / 5-4 修正）。
    ///
    /// `CliMutationRequested` が「リクエストを受理した事実」であるのに対し、
    /// 本 event は「engine が無効と判断して状態を変えなかった事実」を表す。
    /// 両者は独立に発火し、両方記録される場合（reject rule なし node への
    /// reject 等）と本 event のみ記録される場合（SubmitOutput の silent-drop
    /// だった経路）がある。
    ///
    /// spec [08] Rule 1「SubmitOutput の拒否は事実履歴に残さない」の意味は、
    /// accepted のメイン履歴（`ArtifactProduced` / `CliMutationRequested`）に
    /// 出ないことを指すと再定義する。観測経路用の補助履歴として本 event は
    /// 並列に追記される。
    ///
    /// `request` の自由記述（reason / comment）は平文で永続化される（spec
    /// [06] 要求の運用境界と同じ）。
    CliMutationRejected {
        run_id: String,
        workflow_name: String,
        /// 元のリクエスト id（`CliMutationRequested` と同じ値）。
        request_id: String,
        /// 拒否されたリクエストの内容。SubmitOutput の場合は payload 本体を
        /// 含めず `step_name` と `contract` のみ。
        request: CliMutationRequestRecord,
        /// 拒否理由の typed 分類。CLI ユーザはここを見て後続操作を判断する。
        reason: CliMutationRejectionReason,
        /// 人間可読の拒否理由（`WorkflowEngineError::to_string()` 由来）。
        message: String,
        /// caller が pending command を書き出した時刻（Unix 秒）。
        requested_at: f64,
        /// engine が本 event を append した時刻。
        timestamp: f64,
    },
}

/// [06] CLI mutating CLI が要求した内容の typed 表現。
///
/// 各 variant の `node_name` は対象 node の限定の有無を表す（`None` = run 全体 /
/// 現在の承認待ち node を engine 側で解決する、`Some` = caller が明示的に node
/// を限定）。`Reject` の `reason` は CLI 入口で必須化済み。
///
/// `SubmitOutput` variant は `CliMutationRejected` でのみ使用する（accepted 経路
/// は `ArtifactProduced` event が一次表現）。payload 本体は容量が大きい可能性が
/// あるため `step_name` と `contract` のみを保持する。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum CliMutationRequestRecord {
    Approve {
        #[serde(skip_serializing_if = "Option::is_none", default)]
        node_name: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        comment: Option<String>,
    },
    Reject {
        #[serde(skip_serializing_if = "Option::is_none", default)]
        node_name: Option<String>,
        reason: String,
    },
    Abort {
        #[serde(skip_serializing_if = "Option::is_none", default)]
        node_name: Option<String>,
    },
    /// SubmitOutput リクエストの拒否事実用 representation（accepted 経路では
    /// 使用しない）。容量肥大を避けるため structured_output 本体は含めない。
    SubmitOutput { step_name: String, contract: String },
}

/// [06] / [08] `CliMutationRejected` event の typed 拒否理由（5-3 / 5-4 修正）。
///
/// CLI ユーザはこの分類を読んで「再試行可能か」「workflow 設計の問題か」を
/// 判断する。新しい拒否理由が判明した場合は variant を追加する（破壊変更ではなく
/// `serde(rename_all = "snake_case")` の蓄積的拡張として扱う）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CliMutationRejectionReason {
    /// 対象 run が engine から見つからない（CLI 側 5-2 チェックを通り抜けた
    /// race condition 等）。
    RunNotFound,
    /// 対象 run が terminal 状態のため mutation を受け付けない。
    RunNotActive,
    /// 対象 node が workflow に存在しない。
    NodeNotFound,
    /// 現在 approval を待っていない node に approve/reject を要求した。
    NotWaitingApproval,
    /// 5-4: reject rule が定義されていない approval node に reject を要求した。
    NoRejectRule,
    /// 5-3: 構造化出力の受領を受け付けていない step に submit を要求した
    /// （pending 等）。
    StepNotAccepting,
    /// 構造化出力の contract が step の expected と一致しない。
    ContractMismatch,
    /// 上記いずれにも分類されない、engine の InvalidState / Validation 由来の拒否。
    Other,
}

/// approval 判断結果の typed 表現。NDJSON 上は snake_case として出力される。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecisionRecord {
    Approve,
    Reject,
    Abort,
}

/// collect step の各子要素出力エントリ。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectedOutputEntry {
    pub node_name: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub result: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub structured_output: Option<serde_json::Value>,
}

impl WorkflowEvent {
    /// event の primary key となる `run_id` を返す。
    pub fn run_id(&self) -> &str {
        match self {
            Self::RunStarted { run_id, .. }
            | Self::NodeStarted { run_id, .. }
            | Self::StepSessionStarted { run_id, .. }
            | Self::WorkflowStallObserved { run_id, .. }
            | Self::WorkflowStallCleared { run_id, .. }
            | Self::NodeCompleted { run_id, .. }
            | Self::NodeFailed { run_id, .. }
            | Self::ApprovalRequested { run_id, .. }
            | Self::ApprovalResolved { run_id, .. }
            | Self::RunCompleted { run_id, .. }
            | Self::RunFailed { run_id, .. }
            | Self::RunAborted { run_id, .. }
            | Self::OutputCollected { run_id, .. }
            | Self::ParallelStarted { run_id, .. }
            | Self::ParallelChildStarted { run_id, .. }
            | Self::ParallelChildCompleted { run_id, .. }
            | Self::ParallelCompleted { run_id, .. }
            | Self::ContractRepairRequested { run_id, .. }
            | Self::CliMutationRequested { run_id, .. }
            | Self::ArtifactProduced { run_id, .. }
            | Self::CliMutationRejected { run_id, .. } => run_id,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_workflow() -> Workflow {
        use crate::adaptor::gateway::workflow::schema::{
            FacetRefs, NodeDefinition, NodeKind, SessionSpec,
        };
        Workflow {
            variables: Default::default(),
            name: "wf".to_string(),
            description: "".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![NodeDefinition {
                name: "n1".to_string(),
                kind: NodeKind::Session(SessionSpec {
                    facets: FacetRefs {
                        instruction: Some("do".to_string()),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
                ..NodeDefinition::default()
            }],
        }
    }

    #[test]
    fn run_started_serializes_with_event_tag() {
        let event = WorkflowEvent::RunStarted {
            run_id: "00000000-0000-0000-0000-000000000001".to_string(),
            workflow_name: "wf".to_string(),
            workflow_file_stem: "wf".to_string(),
            worktree_path: "/repo".to_string(),
            workflow_definition: minimal_workflow(),
            timestamp: 1.0,
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"event\":\"run_started\""));
        assert!(json.contains("\"run_id\":\"00000000-0000-0000-0000-000000000001\""));
        let back: WorkflowEvent = serde_json::from_str(&json).unwrap();
        assert!(matches!(back, WorkflowEvent::RunStarted { .. }));
    }

    #[test]
    fn node_failed_missing_failure_kind_defaults_to_infrastructure_crash() {
        let event: WorkflowEvent = serde_json::from_value(serde_json::json!({
            "event": "node_failed",
            "run_id": "run-1",
            "workflow_name": "wf",
            "node_name": "review",
            "reason": "legacy failure",
            "timestamp": 1.0
        }))
        .unwrap();

        assert!(matches!(
            event,
            WorkflowEvent::NodeFailed {
                failure_kind: WorkflowStepFailureKind::InfrastructureCrash,
                ..
            }
        ));
    }

    #[test]
    fn run_aborted_step_snapshot_serializes_without_step_history_display_state() {
        let event = WorkflowEvent::RunAborted {
            run_id: "00000000-0000-0000-0000-000000000801".to_string(),
            workflow_name: "wf".to_string(),
            aborted_step: Some(RunAbortedStepSnapshot {
                step_name: "review".to_string(),
                completed_at: 2.0,
                result: None,
                session_id: Some("session-review".to_string()),
                token_usage: None,
                structured_output: None,
                run_index: 1,
                child_outputs: Some(vec![RunAbortedChildOutputSnapshot {
                    step_name: "child-review".to_string(),
                    session_id: Some("session-child-review".to_string()),
                    result: None,
                    run_index: 1,
                    completed_at: 2.0,
                    structured_output: None,
                    artifact_contract: None,
                    outcome: RunAbortedChildOutcome::Aborted,
                }]),
            }),
            timestamp: 2.0,
        };

        let value = serde_json::to_value(&event).unwrap();
        assert_eq!(value["event"].as_str(), Some("run_aborted"));
        let aborted_step = value
            .get("aborted_step")
            .expect("aborted_step snapshot must be present");
        assert_eq!(aborted_step["stepName"].as_str(), Some("review"));
        assert_eq!(aborted_step["sessionId"].as_str(), Some("session-review"));
        assert!(
            aborted_step.get("state").is_none(),
            "RunAborted stores an event snapshot, not StepHistoryEntry"
        );
        let child = &aborted_step["childOutputs"][0];
        assert!(
            child.get("state").is_none(),
            "RunAborted child snapshot stores an event fact, not read-model state"
        );
        assert_eq!(child["outcome"].as_str(), Some("aborted"));
    }

    /// approval 判断結果は典型的な NDJSON 観測者から snake_case で読める。
    #[test]
    fn approval_resolved_decision_serde_round_trips() {
        for decision in [
            ApprovalDecisionRecord::Approve,
            ApprovalDecisionRecord::Reject,
            ApprovalDecisionRecord::Abort,
        ] {
            let event = WorkflowEvent::ApprovalResolved {
                run_id: "00000000-0000-0000-0000-000000000002".to_string(),
                workflow_name: "wf".to_string(),
                node_name: "review".to_string(),
                decision,
                comment: Some("c".to_string()),
                timestamp: 2.0,
            };
            let json = serde_json::to_string(&event).unwrap();
            assert!(json.contains("\"event\":\"approval_resolved\""));
            let back: WorkflowEvent = serde_json::from_str(&json).unwrap();
            match back {
                WorkflowEvent::ApprovalResolved {
                    decision: back_decision,
                    ..
                } => assert_eq!(back_decision, decision),
                _ => panic!("expected ApprovalResolved"),
            }
        }
    }

    /// [06] CLI 経由 mutation 要求の事実は `WorkflowEvent` 列に typed event として
    /// 記録される（spec [06] 観測経路境界）。3 種の payload variant が
    /// 観測経路から平文で読み出せる shape として round-trip する。
    #[test]
    fn cli_mutation_requested_serde_round_trips_all_variants() {
        let approve = WorkflowEvent::CliMutationRequested {
            run_id: "00000000-0000-0000-0000-000000000400".to_string(),
            workflow_name: "wf".to_string(),
            request_id: "00000000-0000-0000-0000-000000000500".to_string(),
            request: CliMutationRequestRecord::Approve {
                node_name: Some("review".to_string()),
                comment: Some("LGTM".to_string()),
            },
            requested_at: 100.0,
            timestamp: 101.0,
        };
        let reject = WorkflowEvent::CliMutationRequested {
            run_id: "00000000-0000-0000-0000-000000000401".to_string(),
            workflow_name: "wf".to_string(),
            request_id: "00000000-0000-0000-0000-000000000501".to_string(),
            request: CliMutationRequestRecord::Reject {
                node_name: None,
                reason: "must rework".to_string(),
            },
            requested_at: 200.0,
            timestamp: 201.0,
        };
        let abort = WorkflowEvent::CliMutationRequested {
            run_id: "00000000-0000-0000-0000-000000000402".to_string(),
            workflow_name: "wf".to_string(),
            request_id: "00000000-0000-0000-0000-000000000502".to_string(),
            request: CliMutationRequestRecord::Abort {
                node_name: Some("review".to_string()),
            },
            requested_at: 300.0,
            timestamp: 301.0,
        };
        for event in [approve, reject, abort] {
            let json = serde_json::to_string(&event).unwrap();
            assert!(json.contains("\"event\":\"cli_mutation_requested\""));
            let back: WorkflowEvent = serde_json::from_str(&json).unwrap();
            let json2 = serde_json::to_string(&back).unwrap();
            assert_eq!(json, json2);
        }
    }

    /// [06] 自由記述データの観測経路境界: reason / comment は平文として
    /// NDJSON serialize 結果に出現する（マスキング・短縮されない）。
    #[test]
    fn cli_mutation_requested_reason_and_comment_appear_in_serialized_json_verbatim() {
        let reject = WorkflowEvent::CliMutationRequested {
            run_id: "00000000-0000-0000-0000-000000000403".to_string(),
            workflow_name: "wf".to_string(),
            request_id: "00000000-0000-0000-0000-000000000503".to_string(),
            request: CliMutationRequestRecord::Reject {
                node_name: None,
                reason: "free-form reject reason".to_string(),
            },
            requested_at: 100.0,
            timestamp: 101.0,
        };
        let reject_json = serde_json::to_string(&reject).unwrap();
        assert!(reject_json.contains("free-form reject reason"));

        let approve = WorkflowEvent::CliMutationRequested {
            run_id: "00000000-0000-0000-0000-000000000404".to_string(),
            workflow_name: "wf".to_string(),
            request_id: "00000000-0000-0000-0000-000000000504".to_string(),
            request: CliMutationRequestRecord::Approve {
                node_name: None,
                comment: Some("free-form approve comment".to_string()),
            },
            requested_at: 102.0,
            timestamp: 103.0,
        };
        let approve_json = serde_json::to_string(&approve).unwrap();
        assert!(approve_json.contains("free-form approve comment"));
    }

    /// `WorkflowEvent::run_id()` がすべての variant で primary key を露出する。
    #[test]
    fn run_id_accessor_exposes_primary_key_for_all_variants() {
        let rid = "00000000-0000-0000-0000-000000000003";
        let events = vec![
            WorkflowEvent::RunStarted {
                run_id: rid.to_string(),
                workflow_name: "w".to_string(),
                workflow_file_stem: "w".to_string(),
                worktree_path: "/r".to_string(),
                workflow_definition: minimal_workflow(),
                timestamp: 0.0,
            },
            WorkflowEvent::NodeStarted {
                run_id: rid.to_string(),
                workflow_name: "w".to_string(),
                node_name: "n".to_string(),
                execution_count: 1,
                timestamp: 0.0,
            },
            WorkflowEvent::StepSessionStarted {
                run_id: rid.to_string(),
                workflow_name: "w".to_string(),
                node_name: "n".to_string(),
                execution_count: 1,
                session_id: "s".to_string(),
                timestamp: 0.0,
            },
            WorkflowEvent::WorkflowStallObserved {
                run_id: rid.to_string(),
                workflow_name: "w".to_string(),
                chat_session_id: "s".to_string(),
                step_name: "n".to_string(),
                run_index: 1,
                turn_phase: "streaming".to_string(),
                idle_secs: 180,
                signal_count: 1,
                cap_reached: false,
                timestamp: 0.0,
            },
            WorkflowEvent::WorkflowStallCleared {
                run_id: rid.to_string(),
                workflow_name: "w".to_string(),
                chat_session_id: "s".to_string(),
                timestamp: 0.0,
            },
            WorkflowEvent::NodeCompleted {
                run_id: rid.to_string(),
                workflow_name: "w".to_string(),
                node_name: "n".to_string(),
                result: None,
                session_id: None,
                token_usage: None,
                structured_output: None,
                run_index: None,
                timestamp: 0.0,
            },
            WorkflowEvent::NodeFailed {
                run_id: rid.to_string(),
                workflow_name: "w".to_string(),
                node_name: "n".to_string(),
                reason: "x".to_string(),
                failure_kind: WorkflowStepFailureKind::InfrastructureCrash,
                retry_count: None,
                timestamp: 0.0,
            },
            WorkflowEvent::ApprovalRequested {
                run_id: rid.to_string(),
                workflow_name: "w".to_string(),
                node_name: "n".to_string(),
                timestamp: 0.0,
            },
            WorkflowEvent::ApprovalResolved {
                run_id: rid.to_string(),
                workflow_name: "w".to_string(),
                node_name: "n".to_string(),
                decision: ApprovalDecisionRecord::Approve,
                comment: None,
                timestamp: 0.0,
            },
            WorkflowEvent::RunCompleted {
                run_id: rid.to_string(),
                workflow_name: "w".to_string(),
                total_token_usage: TokenUsage::default(),
                timestamp: 0.0,
            },
            WorkflowEvent::RunFailed {
                run_id: rid.to_string(),
                workflow_name: "w".to_string(),
                reason: "x".to_string(),
                failure_kind: WorkflowStepFailureKind::InfrastructureCrash,
                retry_count: None,
                timestamp: 0.0,
            },
            WorkflowEvent::RunAborted {
                run_id: rid.to_string(),
                workflow_name: "w".to_string(),
                aborted_step: None,
                timestamp: 0.0,
            },
        ];
        for event in events {
            assert_eq!(event.run_id(), rid);
        }
    }

    /// 5-3 / 5-4 修正: `CliMutationRejected` event が全ての `request` variant
    /// （SubmitOutput 含む）について NDJSON 上で round-trip し、`event` タグが
    /// `cli_mutation_rejected` として出力される。`reason` は snake_case で
    /// 出力される。
    #[test]
    fn cli_mutation_rejected_serde_round_trips_all_variants() {
        let cases = vec![
            (
                WorkflowEvent::CliMutationRejected {
                    run_id: "00000000-0000-0000-0000-000000000600".to_string(),
                    workflow_name: "wf".to_string(),
                    request_id: "00000000-0000-0000-0000-000000000700".to_string(),
                    request: CliMutationRequestRecord::Reject {
                        node_name: Some("plan_architecture".to_string()),
                        reason: "rule なし node 拒否".to_string(),
                    },
                    reason: CliMutationRejectionReason::NoRejectRule,
                    message: "Step 'plan_architecture' does not allow reject".to_string(),
                    requested_at: 100.0,
                    timestamp: 101.0,
                },
                "no_reject_rule",
            ),
            (
                WorkflowEvent::CliMutationRejected {
                    run_id: "00000000-0000-0000-0000-000000000601".to_string(),
                    workflow_name: "wf".to_string(),
                    request_id: "00000000-0000-0000-0000-000000000701".to_string(),
                    request: CliMutationRequestRecord::SubmitOutput {
                        step_name: "plan_fix_policy".to_string(),
                        contract: "fix-policy".to_string(),
                    },
                    reason: CliMutationRejectionReason::StepNotAccepting,
                    message: "step 'plan_fix_policy' is not currently accepting structured output"
                        .to_string(),
                    requested_at: 200.0,
                    timestamp: 201.0,
                },
                "step_not_accepting",
            ),
            (
                WorkflowEvent::CliMutationRejected {
                    run_id: "00000000-0000-0000-0000-000000000602".to_string(),
                    workflow_name: "wf".to_string(),
                    request_id: "00000000-0000-0000-0000-000000000702".to_string(),
                    request: CliMutationRequestRecord::Approve {
                        node_name: None,
                        comment: None,
                    },
                    reason: CliMutationRejectionReason::NotWaitingApproval,
                    message: "approval target stale".to_string(),
                    requested_at: 300.0,
                    timestamp: 301.0,
                },
                "not_waiting_approval",
            ),
        ];
        for (event, reason_str) in cases {
            let json = serde_json::to_string(&event).unwrap();
            assert!(
                json.contains("\"event\":\"cli_mutation_rejected\""),
                "event tag must be snake_case `cli_mutation_rejected`, got: {json}"
            );
            assert!(
                json.contains(&format!("\"reason\":\"{reason_str}\"")),
                "reason must serialize as snake_case `{reason_str}`, got: {json}"
            );
            let back: WorkflowEvent = serde_json::from_str(&json).unwrap();
            let json2 = serde_json::to_string(&back).unwrap();
            assert_eq!(json, json2);
        }
    }

    /// 5-3 / 5-4 修正: `CliMutationRejected` event の `run_id()` accessor が
    /// `run_id` を返す。
    #[test]
    fn cli_mutation_rejected_run_id_accessor_returns_run_id() {
        let rid = "00000000-0000-0000-0000-000000000888";
        let event = WorkflowEvent::CliMutationRejected {
            run_id: rid.to_string(),
            workflow_name: "wf".to_string(),
            request_id: "00000000-0000-0000-0000-000000000999".to_string(),
            request: CliMutationRequestRecord::Abort { node_name: None },
            reason: CliMutationRejectionReason::Other,
            message: "x".to_string(),
            requested_at: 0.0,
            timestamp: 0.0,
        };
        assert_eq!(event.run_id(), rid);
    }
}
