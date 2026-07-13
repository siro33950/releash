//! [05] Command / Event Boundary: workflow engine 内部の node 完了 / 失敗遷移の
//! typed 表現。
//!
//! 外部 mutation は usecase / gateway が resolved runtime primitive wrapper を直接呼ぶ。
//! 本 enum は engine 内部の event 発行点を typed に保つための internal-only 境界であり、
//! 外部 adapter（Tauri command / CLI / agent path）から組み立てる経路を提供しない。
//!
//! ハンドラ実体は engine 側（`engine.rs`）に置く。本ファイルは型の所有のみを担い、
//! 外部向け workflow command 型には依存しない。

use crate::adaptor::gateway::workflow::event::TokenUsage;
use crate::domain::workflow::WorkflowStepFailureKind;

/// workflow engine 内部の node 完了 / 失敗遷移を表す typed command。
///
/// UI / Tauri command / CLI はこの enum を組み立てない。外部 mutation は
/// `WorkflowRuntimeService::{abort_workflow_run, resolve_workflow_approval, submit_workflow_output}`
/// などの runtime primitive wrapper を呼び、この enum は internal node event の commit
/// 境界だけに閉じ込める。
#[derive(Debug, Clone)]
pub(crate) enum InternalNodeCommand {
    /// [05] internal-only: engine 内部の node 完了遷移。発行 event は
    /// `WorkflowEvent::NodeCompleted` と同一 shape のフィールドを持つ。
    ///
    /// 外部 adapter からこの variant を組み立てる経路は提供しない。
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
    FailNode {
        run_id: String,
        workflow_name: String,
        node_name: String,
        reason: String,
        failure_kind: WorkflowStepFailureKind,
        retry_count: Option<u32>,
        timestamp: f64,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    /// [05] internal variants: `InternalNodeCommand::CompleteNode` / `FailNode` は外部
    /// caller 向けではないが `InternalNodeCommand` の variant として typed boundary 上に
    /// 揃え、対応する `WorkflowEvent::NodeCompleted` / `NodeFailed` と同一 shape の
    /// フィールドを持つことを境界仕様として確認する。外部 adapter からは到達不能にし、
    /// internal node event の commit 境界に閉じ込める（spec [05] internal command の
    /// 非公開境界 / 責務配置）。
    #[test]
    fn internal_node_command_variants_carry_event_shape_fields() {
        let complete = InternalNodeCommand::CompleteNode {
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
            InternalNodeCommand::CompleteNode {
                ref run_id,
                ref node_name,
                ..
            } => {
                assert_eq!(run_id, "00000000-0000-0000-0000-000000000020");
                assert_eq!(node_name, "step1");
            }
            _ => panic!("expected CompleteNode"),
        }
        let fail = InternalNodeCommand::FailNode {
            run_id: "00000000-0000-0000-0000-000000000021".to_string(),
            workflow_name: "wf".to_string(),
            node_name: "step2".to_string(),
            reason: "boom".to_string(),
            failure_kind: WorkflowStepFailureKind::InfrastructureCrash,
            retry_count: None,
            timestamp: 200.0,
        };
        match fail {
            InternalNodeCommand::FailNode {
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
}
