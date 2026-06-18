use std::sync::Arc;

use crate::adaptor::controller::command::workflow::validate_run_id;
use crate::adaptor::controller::state::AppState;
use crate::adaptor::protocol::workflow::WorkflowStateView;
use crate::domain::workflow::WorkflowRunSummary;
use crate::usecase::workflow::{WorkflowEventView, WorkflowStepDetailView};

/// [05] read-only API: 過去 / 進行中の workflow run 一覧を返す。
/// `worktree_path` は必須。caller の認可済み worktree のみを対象にすることで
/// 別 worktree の run を観測できる経路を閉じる（spec [05] L104-108 観測経路の
/// 認可境界）。`status` は optional な filter。
#[tauri::command]
pub async fn list_workflow_runs(
    state: tauri::State<'_, AppState>,
    status: Option<String>,
    worktree_path: String,
) -> Result<Vec<WorkflowRunSummary>, String> {
    let status = match status.as_deref() {
        None | Some("") => None,
        Some("active") => Some(crate::domain::workflow::RunStatusFilter::Active),
        Some("terminal") => Some(crate::domain::workflow::RunStatusFilter::Terminal),
        Some(other) => return Err(format!("Invalid status filter: {other}")),
    };
    let query = state.workflow_usecase.clone();
    tokio::task::spawn_blocking(move || {
        query
            .list_runs_for_worktree(status, &worktree_path)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

/// [05] read-only API: 単一 run の summary metadata を返す。
/// active / terminal のいずれであっても返す。該当 run なし、または run の
/// worktree_path が caller の認可済み worktree に合致しない場合は `Ok(None)`
/// （spec [05] L104-108 / L182）。
#[tauri::command]
pub async fn get_workflow_run(
    state: tauri::State<'_, AppState>,
    run_id: String,
) -> Result<Option<WorkflowRunSummary>, String> {
    validate_run_id(&run_id)?;
    let query = state.workflow_usecase.clone();
    tokio::task::spawn_blocking(move || {
        query
            .authorize_run_summary(&run_id)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

/// spec issues-1023: 永続化 event の timestamp は engine 内 `current_timestamp()`
/// 由来の秒単位 f64 だが、frontend 表示用に usecase projection 境界で ms 単位の
/// view に変換する。view 型分離で「秒 / ms」の二重意味を排除する。
/// [05] read-only API: 指定 run の event log を返す。
/// `workflow_logs/{run_id}.ndjson` を engine 一次 owner の log source として読む。
/// 該当 run なし、または認可不一致は `None`（spec [05] L104-108 / L182）。
///
/// spec issues-1023 L132/L150: 観測 invoke は caller の現 worktree path を必須引数
/// として受け取り、`canonicalize_managed_worktree_path` + run metadata の
/// `worktree_path` 一致を二重に検証してから event log を返す。
#[tauri::command]
pub async fn get_workflow_run_log(
    state: tauri::State<'_, AppState>,
    worktree_path: String,
    run_id: String,
) -> Result<Option<Vec<WorkflowEventView>>, String> {
    get_workflow_run_log_inner(&state.workflow_usecase, worktree_path, run_id).await
}

/// [05] 内部経路。Tauri command 側は injected state を受け取り本関数に委譲する。
#[cfg(test)]
pub(super) async fn get_workflow_run_log_inner(
    query: &Arc<crate::usecase::workflow::WorkflowUsecase>,
    worktree_path: String,
    run_id: String,
) -> Result<Option<Vec<WorkflowEventView>>, String> {
    get_workflow_run_log_inner_impl(query, worktree_path, run_id).await
}

#[cfg(not(test))]
async fn get_workflow_run_log_inner(
    query: &Arc<crate::usecase::workflow::WorkflowUsecase>,
    worktree_path: String,
    run_id: String,
) -> Result<Option<Vec<WorkflowEventView>>, String> {
    get_workflow_run_log_inner_impl(query, worktree_path, run_id).await
}

async fn get_workflow_run_log_inner_impl(
    query: &Arc<crate::usecase::workflow::WorkflowUsecase>,
    worktree_path: String,
    run_id: String,
) -> Result<Option<Vec<WorkflowEventView>>, String> {
    let query = query.clone();
    let events = tokio::task::spawn_blocking(move || {
        validate_run_id(&run_id)?;
        if query
            .authorize_run_summary_for_worktree(&run_id, &worktree_path)
            .map_err(|e| e.to_string())?
            .is_none()
        {
            return Ok(None);
        }
        query
            .get_run_log(&run_id)
            .map(Some)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("task join error: {e}"))??;
    Ok(events)
}

/// [05] read-only API: 指定 run の現在 state を返す。
/// NDJSON event log から `reconstruct_state_from_events` で投影する。
///
/// 観測結果の露出範囲境界（spec [05]）に従い、戻り値は engine が一次 owner として
/// 保持している event log / state の純粋投影のみを含む。`AgentProcessMap` /
/// `OpenTabRegistry` 由来の runtime_active / tab_open enrichment は read-only API
/// 経路では含めない（同種情報は live emission 経路の `WorkflowStateChanged` event を
/// 通じて UI に届ける）。
///
/// 認可不一致は `None`（spec [05] L104-108 / L182）。
///
/// spec issues-1023 L132/L150: 観測 invoke は caller の現 worktree path を必須引数
/// として受け取り、`canonicalize_managed_worktree_path` + run metadata の
/// `worktree_path` 一致を二重に検証してから state を返す。
#[tauri::command]
pub async fn get_workflow_run_state(
    state: tauri::State<'_, AppState>,
    worktree_path: String,
    run_id: String,
) -> Result<Option<WorkflowStateView>, String> {
    get_workflow_run_state_inner(&state.workflow_usecase, worktree_path, run_id).await
}

/// [05] 内部経路。Tauri command 側は injected state を受け取り本関数に委譲する。
#[cfg(test)]
pub(super) async fn get_workflow_run_state_inner(
    query: &Arc<crate::usecase::workflow::WorkflowUsecase>,
    worktree_path: String,
    run_id: String,
) -> Result<Option<WorkflowStateView>, String> {
    get_workflow_run_state_inner_impl(query, worktree_path, run_id).await
}

#[cfg(not(test))]
async fn get_workflow_run_state_inner(
    query: &Arc<crate::usecase::workflow::WorkflowUsecase>,
    worktree_path: String,
    run_id: String,
) -> Result<Option<WorkflowStateView>, String> {
    get_workflow_run_state_inner_impl(query, worktree_path, run_id).await
}

async fn get_workflow_run_state_inner_impl(
    query: &Arc<crate::usecase::workflow::WorkflowUsecase>,
    worktree_path: String,
    run_id: String,
) -> Result<Option<WorkflowStateView>, String> {
    let query = query.clone();
    let state = tokio::task::spawn_blocking(move || {
        validate_run_id(&run_id)?;
        if query
            .authorize_run_summary_for_worktree(&run_id, &worktree_path)
            .map_err(|e| e.to_string())?
            .is_none()
        {
            return Ok(None);
        }
        query.get_run_state(&run_id).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("task join error: {e}"))??;
    Ok(state.map(|state| {
        WorkflowStateView::from_parts(
            crate::adaptor::presenter::workflow::workflow_state_to_view(state),
            std::collections::HashMap::new(),
        )
    }))
}

/// spec issues-1023: 選択 step の入出力・遷移結果・所要時間を 1 つの View で返す
/// 観測用 API。frontend で `WorkflowState` を再走査する代わりに、`worktree_path` /
/// `run_id` / `node_name` / `run_index` を渡してこの View を受け取る境界。
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn get_workflow_step_detail(
    state: tauri::State<'_, AppState>,
    worktree_path: String,
    run_id: String,
    node_name: String,
    run_index: Option<u32>,
) -> Result<Option<WorkflowStepDetailView>, String> {
    get_workflow_step_detail_inner(
        &state.workflow_usecase,
        worktree_path,
        run_id,
        node_name,
        run_index,
    )
    .await
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub(super) async fn get_workflow_step_detail_inner(
    query: &Arc<crate::usecase::workflow::WorkflowUsecase>,
    worktree_path: String,
    run_id: String,
    node_name: String,
    run_index: Option<u32>,
) -> Result<Option<WorkflowStepDetailView>, String> {
    get_workflow_step_detail_inner_impl(query, worktree_path, run_id, node_name, run_index).await
}

#[cfg(not(test))]
#[allow(clippy::too_many_arguments)]
async fn get_workflow_step_detail_inner(
    query: &Arc<crate::usecase::workflow::WorkflowUsecase>,
    worktree_path: String,
    run_id: String,
    node_name: String,
    run_index: Option<u32>,
) -> Result<Option<WorkflowStepDetailView>, String> {
    get_workflow_step_detail_inner_impl(query, worktree_path, run_id, node_name, run_index).await
}

#[allow(clippy::too_many_arguments)]
async fn get_workflow_step_detail_inner_impl(
    query: &Arc<crate::usecase::workflow::WorkflowUsecase>,
    worktree_path: String,
    run_id: String,
    node_name: String,
    run_index: Option<u32>,
) -> Result<Option<WorkflowStepDetailView>, String> {
    let query = query.clone();
    let detail = tokio::task::spawn_blocking(move || {
        validate_run_id(&run_id)?;
        if query
            .authorize_run_summary_for_worktree(&run_id, &worktree_path)
            .map_err(|e| e.to_string())?
            .is_none()
        {
            return Ok(None);
        }
        query
            .get_step_detail(&run_id, &node_name, run_index)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("task join error: {e}"))??;
    Ok(detail)
}

/// worktree_path から active な run_id を解決する（双方向 lookup の一方向）。
#[tauri::command]
pub async fn resolve_active_run_by_worktree(
    state: tauri::State<'_, AppState>,
    worktree_path: String,
) -> Result<Option<String>, String> {
    let query = state.workflow_usecase.clone();
    tokio::task::spawn_blocking(move || {
        query
            .list_runs_for_worktree(
                Some(crate::domain::workflow::RunStatusFilter::Active),
                &worktree_path,
            )
            .map(|runs| runs.into_iter().next().map(|run| run.run_id))
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

/// run_id から worktree_path を解決する（双方向 lookup のもう一方向）。
/// active / 終了済みの両方について metadata 経由で解決する。
/// path traversal 対策として command 入口で UUID 形式を検証する。
#[tauri::command]
pub async fn resolve_worktree_by_run(
    state: tauri::State<'_, AppState>,
    run_id: String,
) -> Result<Option<String>, String> {
    validate_run_id(&run_id)?;
    let query = state.workflow_usecase.clone();
    tokio::task::spawn_blocking(move || {
        query
            .resolve_worktree_by_run(&run_id)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}
