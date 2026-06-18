use std::collections::HashMap;

use crate::adaptor::gateway::workflow::runtime_state::WorkflowExecution;

/// `set_execution_state_inner` の lookup 戦略。worktree_path 起点の `find_by_worktree_mut`
/// で active な run を解決する。broadcast / cleanup 用の worktree_path は
/// `set_execution_state_inner` 内で解決した exec から取得するため、target には保持しない
/// （Spec issues-1011 finding 12: 意図不明な未使用 field を残さない）。
///
/// run_id 主語の遷移経路（run 全体中断）は
/// `abort_workflow_by_run_id` 側で typed AbortOutcome + 必須 RunAborted append として
/// 完結するため、ここでは worktree variant のみを扱う。
pub(crate) enum ExecutionStateTarget {
    Worktree(String),
}

/// `execs: HashMap<run_id, WorkflowExecution>` から、worktree_path 属性が一致する
/// **active な** `(run_id, exec)` を線形走査で取得する補助関数。
///
/// Spec issues-1011: engine 内部キーは run_id だが、production 経路で session_id や
/// worktree_path 起点の操作（on_turn_complete, handle_approval, abort 等）が残るため、
/// その下流で active な実行だけを引くための secondary lookup として使う。終了済み
/// （terminal）実行は executions に残るが、ここでは除外して active な run のみ返す
/// （同一 worktree に terminal run と active run が共存しても active を取り違えない）。
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
/// active filter を適用しない（terminal な過去 run は通過させて Ok 判定にする）。
pub(crate) fn find_any_by_worktree<'a>(
    execs: &'a HashMap<String, WorkflowExecution>,
    worktree_path: &str,
) -> Option<&'a WorkflowExecution> {
    execs.values().find(|e| e.worktree_path == worktree_path)
}
