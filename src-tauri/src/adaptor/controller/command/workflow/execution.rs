use std::sync::Arc;

use crate::adaptor::controller::command::workflow::validate_execution_id;
use crate::adaptor::controller::state::AppState;
use crate::adaptor::protocol::workflow::{NodeExecutionView, WorkflowExecutionView};
use crate::usecase::workflow::dto::{
    workflow_execution_summary_to_dto, WorkflowExecutionSummaryDto,
};
use crate::usecase::workflow::WorkflowEventView;

/// [05] read-only API: 過去 / 進行中の workflow execution 一覧を返す。
/// `worktree_path` は必須。caller の認可済み worktree のみを対象にすることで
/// 別 worktree の execution を観測できる経路を閉じる（spec [05] L104-108 観測経路の
/// 認可境界）。`status` は optional な filter。
#[tauri::command]
pub async fn list_workflow_executions(
    state: tauri::State<'_, AppState>,
    status: Option<String>,
    worktree_path: String,
) -> Result<Vec<WorkflowExecutionSummaryDto>, String> {
    let status =
        crate::domain::workflow::ExecutionStatusFilter::from_public_filter(status.as_deref())
            .map_err(|_| {
                format!(
                    "Invalid status filter: {}",
                    status.as_deref().unwrap_or_default()
                )
            })?;
    let query = state.workflow_usecase.clone();
    tokio::task::spawn_blocking(move || {
        query
            .list_executions_for_worktree(status, &worktree_path)
            .map(|executions| {
                executions
                    .into_iter()
                    .map(workflow_execution_summary_to_dto)
                    .collect()
            })
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

/// [05] read-only API: 単一 execution の summary metadata を返す。
/// active / terminal のいずれであっても返す。該当 execution なし、または execution の
/// worktree_path が caller の認可済み worktree に合致しない場合は `Ok(None)`
/// （spec [05] L104-108 / L182）。
#[tauri::command]
pub async fn get_workflow_execution(
    state: tauri::State<'_, AppState>,
    execution_id: String,
) -> Result<Option<WorkflowExecutionSummaryDto>, String> {
    validate_execution_id(&execution_id)?;
    let query = state.workflow_usecase.clone();
    tokio::task::spawn_blocking(move || {
        query
            .authorize_execution_summary(&execution_id)
            .map(|execution| execution.map(workflow_execution_summary_to_dto))
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

/// spec issues-1023: 永続化 event の timestamp は engine 内 `current_timestamp()`
/// 由来の秒単位 f64 だが、frontend 表示用に usecase projection 境界で ms 単位の
/// view に変換する。view 型分離で「秒 / ms」の二重意味を排除する。
/// [05] read-only API: 指定 execution の event log を返す。
/// `workflow_execution_logs/{execution_id}.ndjson` を engine 一次 owner の log source として読む。
/// 該当 execution なし、または認可不一致は `None`（spec [05] L104-108 / L182）。
///
/// spec issues-1023 L132/L150: 観測 invoke は caller の現 worktree path を必須引数
/// として受け取り、`canonicalize_managed_worktree_path` + execution metadata の
/// `worktree_path` 一致を二重に検証してから event log を返す。
#[tauri::command]
pub async fn get_workflow_execution_log(
    state: tauri::State<'_, AppState>,
    worktree_path: String,
    execution_id: String,
) -> Result<Option<Vec<WorkflowEventView>>, String> {
    get_workflow_execution_log_impl(&state.workflow_usecase, worktree_path, execution_id).await
}

/// [05] 内部経路。Tauri command 側は injected state を受け取り本関数に委譲する。
pub(super) async fn get_workflow_execution_log_impl(
    query: &Arc<crate::usecase::workflow::WorkflowUsecase>,
    worktree_path: String,
    execution_id: String,
) -> Result<Option<Vec<WorkflowEventView>>, String> {
    let query = query.clone();
    let events = tokio::task::spawn_blocking(move || {
        validate_execution_id(&execution_id)?;
        if query
            .authorize_execution_summary_for_worktree(&execution_id, &worktree_path)
            .map_err(|e| e.to_string())?
            .is_none()
        {
            return Ok(None);
        }
        query
            .get_execution_log(&execution_id)
            .map(Some)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("task join error: {e}"))??;
    Ok(events)
}

/// [05] read-only API: 指定 execution の現在 state を返す。
/// NDJSON event log から canonical `WorkflowExecution` read model を投影する。
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
#[tauri::command]
pub async fn get_workflow_execution_state(
    state: tauri::State<'_, AppState>,
    worktree_path: String,
    execution_id: String,
) -> Result<Option<WorkflowExecutionView>, String> {
    get_workflow_execution_state_impl(&state.workflow_usecase, worktree_path, execution_id).await
}

/// [05] 内部経路。Tauri command 側は injected state を受け取り本関数に委譲する。
pub(super) async fn get_workflow_execution_state_impl(
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

/// spec issues-1023: 選択 node の入出力・遷移結果・所要時間を 1 つの View で返す
/// 観測用 API。frontend で execution 全体を再走査する代わりに、`worktree_path` /
/// `execution_id` / `node_execution_id` を渡してこの View を受け取る境界。
#[tauri::command]
pub async fn get_workflow_node_detail(
    state: tauri::State<'_, AppState>,
    worktree_path: String,
    execution_id: String,
    node_execution_id: String,
) -> Result<Option<NodeExecutionView>, String> {
    get_workflow_node_detail_impl(
        &state.workflow_usecase,
        worktree_path,
        execution_id,
        node_execution_id,
    )
    .await
}

pub(super) async fn get_workflow_node_detail_impl(
    query: &Arc<crate::usecase::workflow::WorkflowUsecase>,
    worktree_path: String,
    execution_id: String,
    node_execution_id: String,
) -> Result<Option<NodeExecutionView>, String> {
    let query = query.clone();
    let detail = tokio::task::spawn_blocking(move || {
        validate_execution_id(&execution_id)?;
        if query
            .authorize_execution_summary_for_worktree(&execution_id, &worktree_path)
            .map_err(|e| e.to_string())?
            .is_none()
        {
            return Ok(None);
        }
        query
            .get_node_detail(&execution_id, &node_execution_id)
            .map(|detail| detail.map(crate::adaptor::presenter::workflow::node_execution_to_view))
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("task join error: {e}"))??;
    Ok(detail)
}

/// worktree_path から active な execution_id を解決する（双方向 lookup の一方向）。
#[tauri::command]
pub async fn resolve_active_execution_by_worktree(
    state: tauri::State<'_, AppState>,
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

/// execution_id から worktree_path を解決する（双方向 lookup のもう一方向）。
/// active / 終了済みの両方について metadata 経由で解決する。
/// path traversal 対策として command 入口で UUID 形式を検証する。
#[tauri::command]
pub async fn resolve_worktree_by_execution(
    state: tauri::State<'_, AppState>,
    execution_id: String,
) -> Result<Option<String>, String> {
    validate_execution_id(&execution_id)?;
    let query = state.workflow_usecase.clone();
    tokio::task::spawn_blocking(move || {
        query
            .resolve_worktree_by_execution(&execution_id)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}
