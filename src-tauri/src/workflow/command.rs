//! [04] / [05] Command / Event Boundary: workflow state を変化させる唯一の入口の
//! typed 表現。
//!
//! 外部入口を持つ 4 command（`StartRun` / `AbortRun` / `ApproveNode` / `RejectNode`）
//! に加え、engine 内部の node 完了 / 失敗遷移を typed 化する internal-only な
//! 2 variant（`CompleteNode` / `FailNode`）を [05] で追加した。internal variant は
//! 外部 adapter（Tauri command / CLI / agent path）から組み立てる経路を提供せず、
//! `WorkflowEngine::dispatch` 経由で外部から到達した場合は内部不整合として `Err` に
//! 変換する境界（spec [05] internal command の非公開境界）を engine 側で担保する。
//!
//! `SubmitOutput`（[08]）は本ファイルでは導入しない。
//!
//! ハンドラ実体は engine 側（`engine.rs`）に置く。本ファイルは型の所有のみを担い、
//! `ApprovalDecision` 等の engine domain 型には依存しない。

use crate::permission::PermissionMode;
use crate::workflow::event::TokenUsage;
use crate::workflow::run::TriggerSource;

/// workflow engine の state を変化させる typed command。
///
/// UI / Tauri command / 内部呼び出し元は、本 enum を組み立てて
/// `WorkflowEngine::dispatch` に渡す経路のみを使う。`run_id` 主語の管理は
/// [03] Run Store に揃える。
///
/// 外部入口を持つ 4 variant（`StartRun` / `AbortRun` / `ApproveNode` /
/// `RejectNode`）に加え、engine 内部の node 完了 / 失敗遷移を typed 化する
/// internal-only な 2 variant（`CompleteNode` / `FailNode`）を持つ。
/// internal variant は外部 adapter（Tauri command / CLI / agent path）から
/// 組み立てる経路を提供せず、`WorkflowEngine::dispatch` 経由で外部から到達した
/// 場合は内部不整合として `Err` に変換する境界を engine 側で担保する
/// （spec [05] internal command の非公開境界）。
#[derive(Debug, Clone)]
pub enum WorkflowCommand {
    /// 新しい workflow run の起動。
    ///
    /// `workflow_file_stem` は保存済み YAML / builtin の ファイル名 stem（拡張子なし）
    /// であり、command 境界で「どの workflow template を起動するか」を解決する識別子。
    /// 論理 workflow 名（`Workflow::name`）は load 済み definition から導出する想定で、
    /// command boundary 上に重複した source of truth を持たない。Storage 層は
    /// `Summary::name` を file stem として扱う（[03] Run Store / Summary 境界)。
    StartRun {
        workflow_file_stem: String,
        worktree_path: String,
        task: Option<String>,
        trigger_source: TriggerSource,
        permission_mode: PermissionMode,
    },
    /// 進行中の workflow run の中断。
    ///
    /// `expected_node_name` が Some の場合、approval UI 由来の Abort として
    /// 現在の承認待ち node と一致するかを engine 側で検証する。これにより
    /// stale な approval UI から任意フェーズの run を誤って中断することを防ぐ。
    /// None の場合は run 全体の中断要求として扱う。
    AbortRun {
        run_id: String,
        expected_node_name: Option<String>,
    },
    /// approval node に対する承認判断（任意のコメント付き）。
    ApproveNode {
        run_id: String,
        node_name: String,
        comment: Option<String>,
    },
    /// approval node に対する却下判断（理由必須）。
    RejectNode {
        run_id: String,
        node_name: String,
        reason: String,
    },
    /// [05] internal-only: engine 内部の node 完了遷移。発行 event は
    /// `WorkflowEvent::NodeCompleted` と同一 shape のフィールドを持つ。
    ///
    /// 外部 adapter からこの variant を組み立てる経路は提供しない。
    /// `WorkflowEngine::dispatch` 経由で到達した場合は内部不整合として `Err`。
    CompleteNode {
        run_id: String,
        workflow_name: String,
        node_name: String,
        result: Option<String>,
        session_id: Option<String>,
        token_usage: Option<TokenUsage>,
        structured_output: Option<serde_json::Value>,
        run_index: Option<u32>,
        timestamp: f64,
    },
    /// [05] internal-only: engine 内部の node 失敗遷移。発行 event は
    /// `WorkflowEvent::NodeFailed` と同一 shape のフィールドを持つ。
    ///
    /// 外部 adapter からこの variant を組み立てる経路は提供しない。
    /// `WorkflowEngine::dispatch` 経由で到達した場合は内部不整合として `Err`。
    FailNode {
        run_id: String,
        workflow_name: String,
        node_name: String,
        reason: String,
        timestamp: f64,
    },
}

/// `WorkflowEngine::dispatch` が返す typed boundary 結果。
///
/// `Result<String, _>` で空文字列 sentinel を返す旧シグネチャを廃し、command の
/// 受理結果を variant ごとに表現する。Tauri adapter 側で `String` / `()` に薄く
/// 変換する境界に揃え、engine 公開 API には sentinel 値を持ち込まない（spec [04]）。
///
/// 本ファイルでは特定 variant への変換ヘルパー（特に `Accepted` を空文字列 `run_id` に
/// sentinel 化する変換）は提供しない。adapter 側（`commands.rs`）が `match` で variant 別
/// に射影し、本来到達しない variant は内部不整合として `Err` に変換する（spec [04] 責務配置）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkflowCommandResult {
    /// `StartRun` の受理結果。新しく払い出された `run_id` を返す。
    RunStarted { run_id: String },
    /// `AbortRun` / `ApproveNode` / `RejectNode` の受理結果。
    /// state 変化の事実は engine の event 列で観測する。
    Accepted,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// approval UI 由来の Abort は `expected_node_name` を伴って構築され、
    /// engine 側で current approval node との一致を検証する経路に乗る。
    /// run 全体の Abort（expected_node_name == None）と区別できることを確認する。
    #[test]
    fn abort_run_distinguishes_approval_target_from_full_run_abort() {
        let rid = "00000000-0000-0000-0000-000000000010".to_string();
        let approval_abort = WorkflowCommand::AbortRun {
            run_id: rid.clone(),
            expected_node_name: Some("review".to_string()),
        };
        let full_abort = WorkflowCommand::AbortRun {
            run_id: rid,
            expected_node_name: None,
        };
        match (approval_abort, full_abort) {
            (
                WorkflowCommand::AbortRun {
                    expected_node_name: Some(ref n),
                    ..
                },
                WorkflowCommand::AbortRun {
                    expected_node_name: None,
                    ..
                },
            ) => {
                assert_eq!(n, "review");
            }
            _ => panic!("AbortRun variants must distinguish expected_node_name"),
        }
    }

    /// [05] internal variants: `WorkflowCommand::CompleteNode` / `FailNode` は外部
    /// caller 向けではないが `WorkflowCommand` の variant として typed boundary 上に
    /// 揃え、対応する `WorkflowEvent::NodeCompleted` / `NodeFailed` と同一 shape の
    /// フィールドを持つことを境界仕様として確認する。外部 adapter からは到達不能、
    /// `dispatch` 経由で外部から流入した場合は engine 側で `Err` に変換する境界を
    /// 担保する（spec [05] internal command の非公開境界 / 責務配置）。
    #[test]
    fn internal_node_command_variants_carry_event_shape_fields() {
        let complete = WorkflowCommand::CompleteNode {
            run_id: "00000000-0000-0000-0000-000000000020".to_string(),
            workflow_name: "wf".to_string(),
            node_name: "step1".to_string(),
            result: Some("ok".to_string()),
            session_id: Some("sess-1".to_string()),
            token_usage: None,
            structured_output: None,
            run_index: Some(1),
            timestamp: 100.0,
        };
        match complete {
            WorkflowCommand::CompleteNode {
                ref run_id,
                ref node_name,
                ..
            } => {
                assert_eq!(run_id, "00000000-0000-0000-0000-000000000020");
                assert_eq!(node_name, "step1");
            }
            _ => panic!("expected CompleteNode"),
        }
        let fail = WorkflowCommand::FailNode {
            run_id: "00000000-0000-0000-0000-000000000021".to_string(),
            workflow_name: "wf".to_string(),
            node_name: "step2".to_string(),
            reason: "boom".to_string(),
            timestamp: 200.0,
        };
        match fail {
            WorkflowCommand::FailNode {
                ref reason,
                ref node_name,
                ..
            } => {
                assert_eq!(reason, "boom");
                assert_eq!(node_name, "step2");
            }
            _ => panic!("expected FailNode"),
        }
    }

    /// ApproveNode.comment は engine の `ApprovalResolved.comment` に伝播される domain
    /// field として command に持たせている。任意コメント付き承認の意図が command 境界で
    /// 表現可能であることを確認する。
    #[test]
    fn approve_node_carries_optional_comment_across_command_boundary() {
        let cmd = WorkflowCommand::ApproveNode {
            run_id: "00000000-0000-0000-0000-000000000011".to_string(),
            node_name: "review".to_string(),
            comment: Some("LGTM with notes".to_string()),
        };
        match cmd {
            WorkflowCommand::ApproveNode { comment, .. } => {
                assert_eq!(comment.as_deref(), Some("LGTM with notes"));
            }
            _ => panic!("expected ApproveNode"),
        }
    }
}
