use std::sync::Arc;

use crate::adaptor::controller::client::workflow::validate_execution_id;
use crate::adaptor::controller::state::AppState;
use crate::adaptor::protocol::workflow::WorkflowExecutionView;

/// [05] read-only API: 指定 execution の現在 state を返す。
/// 事実ログから canonical `ExecutionTree` read model を投影する。
///
/// 観測結果の露出範囲境界（spec [05]）に従い、戻り値は engine が一次 owner として
/// 保持している event log / state の純粋投影のみを含む。live runtime registry /
/// `OpenTabRegistry` 由来の runtime_active / tab_open enrichment は read-only API
/// 経路では含めない（同種情報は live emission 経路の `WorkflowStateChanged` event を
/// 通じて UI に届ける）。
///
/// 認可不一致は `None`（spec [05] L104-108 / L182）。
///
/// spec issues-1023 L132/L150: 観測 invoke は caller の現 worktree path を必須引数
/// として受け取り、`canonicalize_managed_worktree_path` + execution metadata の
/// `worktree_path` 一致を二重に検証してから state を返す。
pub(crate) async fn get_workflow_execution_state_shared(
    state: &AppState,
    worktree_path: String,
    execution_id: String,
) -> Result<Option<WorkflowExecutionView>, String> {
    get_workflow_execution_state_impl(&state.workflow_usecase, worktree_path, execution_id).await
}

/// [05] 内部経路。Tauri command 側は injected state を受け取り本関数に委譲する。
pub(crate) async fn get_workflow_execution_state_impl(
    query: &Arc<crate::usecase::workflow::WorkflowUsecase>,
    worktree_path: String,
    execution_id: String,
) -> Result<Option<WorkflowExecutionView>, String> {
    let query = query.clone();
    let state = tokio::task::spawn_blocking(move || {
        validate_execution_id(&execution_id)?;
        if query
            .authorize_execution_summary_for_worktree(&execution_id, &worktree_path)
            .map_err(|e| e.to_string())?
            .is_none()
        {
            return Ok(None);
        }
        query
            .get_execution_state(&execution_id)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("task join error: {e}"))??;
    Ok(state.map(crate::adaptor::presenter::workflow::workflow_execution_to_view))
}

/// worktree_path から active な execution_id を解決する（双方向 lookup の一方向）。
pub(crate) async fn resolve_active_execution_by_worktree_shared(
    state: &AppState,
    worktree_path: String,
) -> Result<Option<String>, String> {
    let query = state.workflow_usecase.clone();
    tokio::task::spawn_blocking(move || {
        query
            .list_executions_for_worktree(
                Some(crate::domain::workflow::ExecutionStatusFilter::Active),
                &worktree_path,
            )
            .map(|executions| {
                executions
                    .into_iter()
                    .next()
                    .map(|execution| execution.execution_id)
            })
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}
