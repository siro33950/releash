use std::collections::HashMap;

use crate::adaptor::gateway::workflow::runtime_state::WorkflowExecution;

/// `execs: HashMap<execution_id, WorkflowExecution>` から、worktree_path 属性が一致する
/// **active な** `(execution_id, exec)` を線形走査で取得する補助関数。
///
/// Spec issues-1011: engine 内部キーは execution_id だが、production 経路で session_id や
/// worktree_path 起点の操作（on_turn_complete, handle_approval, abort 等）が残るため、
/// その下流で active な実行だけを引くための secondary lookup として使う。終了済み
/// （terminal）実行は executions に残るが、ここでは除外して active な execution のみ返す
/// （同一 worktree に terminal execution と active execution が共存しても active を取り違えない）。
pub(crate) fn find_by_worktree<'a>(
    execs: &'a HashMap<String, WorkflowExecution>,
    worktree_path: &str,
) -> Option<(&'a String, &'a WorkflowExecution)> {
    execs
        .iter()
        .find(|(_, e)| e.worktree_path == worktree_path && e.is_active())
}

pub(crate) fn find_by_worktree_mut<'a>(
    execs: &'a mut HashMap<String, WorkflowExecution>,
    worktree_path: &str,
) -> Option<&'a mut WorkflowExecution> {
    execs
        .values_mut()
        .find(|e| e.worktree_path == worktree_path && e.is_active())
}

/// validate_start 用に「active/terminal を問わず worktree_path が一致する exec」を引く。
/// 重複起動拒否（is_active な existing がある場合に限り Err）の判定で必要なため、
/// active filter を適用しない（terminal な過去 execution は通過させて Ok 判定にする）。
pub(crate) fn find_any_by_worktree<'a>(
    execs: &'a HashMap<String, WorkflowExecution>,
    worktree_path: &str,
) -> Option<&'a WorkflowExecution> {
    execs.values().find(|e| e.worktree_path == worktree_path)
}
