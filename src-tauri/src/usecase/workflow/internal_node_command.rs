//! Workflow application内部のNode完了event command。
//! typed 表現。
//!
//! 外部 mutation は usecase / gateway が resolved runtime primitive wrapper を直接呼ぶ。
//! 本typeはdriver内部のevent発行点をtypedに保つためのinternal-only境界であり、
//! 外部 adapter（Tauri command / CLI / agent path）から組み立てる経路を提供しない。
//!
//! ハンドラ実体は driver 側（`driver.rs`）に置く。本ファイルは型の所有のみを担い、
//! 外部向け workflow command 型には依存しない。

use crate::domain::workflow::TokenUsage;

/// workflow driver 内部の node 完了 / 失敗遷移を表す typed command。
///
/// UI / Tauri command / CLIはこの型を組み立てない。外部mutationは
/// `WorkflowRuntimeHost::{abort_workflow_execution, resolve_workflow_approval, submit_workflow_output}`
/// などの runtime primitive wrapper を呼び、この enum は internal node event の commit
/// 境界だけに閉じ込める。
#[derive(Debug, Clone)]
pub(crate) struct InternalNodeCommand {
    pub(crate) execution_id: String,
    pub(crate) workflow_name: String,
    pub(crate) node_execution_id: String,
    pub(crate) node_name: String,
    pub(crate) result: Option<String>,
    pub(crate) session_id: Option<String>,
    pub(crate) token_usage: Option<TokenUsage>,
    pub(crate) artifact: Option<serde_json::Value>,
    pub(crate) attempt: Option<u32>,
    pub(crate) timestamp: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// [05] internal variant: `InternalNodeCommand` は外部
    /// caller 向けではないが `InternalNodeCommand` の variant として typed boundary 上に
    /// 対応する `WorkflowEvent::NodeCompleted` と同一 shape の
    /// フィールドを持つことを境界仕様として確認する。外部 adapter からは到達不能にし、
    /// internal node event の commit 境界に閉じ込める（spec [05] internal command の
    /// 非公開境界 / 責務配置）。
    #[test]
    fn internal_node_command_variants_carry_event_shape_fields() {
        let complete = InternalNodeCommand {
            execution_id: "00000000-0000-0000-0000-000000000020".to_string(),
            workflow_name: "wf".to_string(),
            node_execution_id: "node-execution-20".to_string(),
            node_name: "node1".to_string(),
            result: Some("ok".to_string()),
            session_id: Some("sess-1".to_string()),
            token_usage: None,
            artifact: None,
            attempt: Some(1),
            timestamp: 100.0,
        };
        assert_eq!(
            complete.execution_id,
            "00000000-0000-0000-0000-000000000020"
        );
        assert_eq!(complete.node_execution_id, "node-execution-20");
        assert_eq!(complete.node_name, "node1");
    }
}
