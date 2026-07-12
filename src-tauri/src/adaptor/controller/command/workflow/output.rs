use std::sync::Arc;

use crate::adaptor::controller::command::workflow::validate_run_id;
use crate::adaptor::controller::state::AppState;
use crate::usecase::workflow::command::SubmitOutputCommand;
use crate::usecase::workflow::{
    WorkflowGetOutputResult, WorkflowRuntimeUsecase, WorkflowValidateOutputResult,
};

/// [08] `workflow output` 系 Tauri command の共通 authorize ヘルパー。
///
/// 3 ハンドラ（submit / validate / get）で「`validate_run_id` →
/// `WorkflowUsecase::authorize_run_summary_for_worktree`」を完全に同一フローで行うため、
/// 共通化する。
/// 認可外 worktree / 不存在 run のいずれも `Workflow run not found` 同表現で
/// 拒否し、存在情報を漏らさない（spec [08] L169 / L182）。
async fn authorize_output_run_access(
    query: &Arc<crate::usecase::workflow::WorkflowUsecase>,
    worktree_path: String,
    run_id: &str,
) -> Result<(), String> {
    validate_run_id(run_id)?;
    let query = query.clone();
    let run_id = run_id.to_string();
    let run_id_for_lookup = run_id.clone();
    tokio::task::spawn_blocking(move || {
        query
            .authorize_run_summary_for_worktree(&run_id_for_lookup, &worktree_path)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
    .and_then(|summary| {
        if summary.is_some() {
            Ok(())
        } else {
            Err(format!("Workflow run not found: {run_id}"))
        }
    })
}

/// [08] Tauri command 経路: step に対する構造化出力を typed 提出する。
///
/// in-process caller (UI / Remote / 外部 Tauri caller) は workflow runtime usecase
/// を経由して、CLI 経路と同一の state mutation 境界に合流する（spec [08] L169）。
/// worktree-scoped 認可境界（spec [08] が依拠する [05] 観測経路の認可境界）を通過しない caller には
/// 「該当 run なし」と同表現で拒否を返し、存在情報を漏らさない。
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn workflow_submit_output(
    state: tauri::State<'_, AppState>,
    runtime: tauri::State<'_, Arc<WorkflowRuntimeUsecase>>,
    worktree_path: String,
    run_id: String,
    step_name: String,
    contract: String,
    structured_output: serde_json::Value,
) -> Result<(), String> {
    authorize_output_run_access(&state.workflow_usecase, worktree_path, &run_id).await?;
    runtime
        .submit_output(SubmitOutputCommand {
            run_id,
            step_name,
            contract,
            structured_output,
        })
        .await
        .map_err(|e| e.to_string())
}

/// [08] structured output の contract 適合性のみを副作用なしで判定する Tauri command。
/// `submit` と異なり engine state / event log には触れない（spec [08] 振る舞い定義 Rule 2）。
///
/// caller は run_id / step_name を主語に呼び出す。contract type は backend 側で
/// event log の `RunStarted.workflow_definition` から解決する（spec [08] 要求:
/// 「run_id と step_name を主語に CLI/API 経由で engine に提出できる」境界。
/// caller 指定の contract 文字列を信用しない）。
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn workflow_validate_output(
    state: tauri::State<'_, AppState>,
    worktree_path: String,
    run_id: String,
    step_name: String,
    structured_output: serde_json::Value,
) -> Result<WorkflowValidateOutputResponse, String> {
    authorize_output_run_access(&state.workflow_usecase, worktree_path, &run_id).await?;
    // [08] preflight と本 submit (`handle_submit_output`) で同一の前処理 + validation を
    // 共有するため、masking + validate を集約した `preprocess_and_validate_output` を経由する。
    // raw JSON のまま `validate_contract_value` を呼ぶと submit 側の redaction 後の値と
    // 判定が食い違う構造になるため、usecase 側の SecretSourceGateway 経由で redaction 後に判定する。
    let usecase = state.workflow_usecase.clone();
    let result = tokio::task::spawn_blocking(move || {
        usecase
            .validate_output(&run_id, &step_name, structured_output)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("task join error: {e}"))??;
    Ok(match result {
        WorkflowValidateOutputResult::Valid => WorkflowValidateOutputResponse::Valid,
        WorkflowValidateOutputResult::Invalid { reason, details } => {
            WorkflowValidateOutputResponse::Invalid { reason, details }
        }
    })
}

/// [08] 提出済みの構造化出力を取得する Tauri command。未提出の場合は決定論的に
/// `NotSubmitted` を返す（spec [08] 振る舞い定義 Rule 3）。
#[tauri::command]
pub async fn workflow_get_output(
    state: tauri::State<'_, AppState>,
    worktree_path: String,
    run_id: String,
    step_name: String,
) -> Result<WorkflowGetOutputResponse, String> {
    authorize_output_run_access(&state.workflow_usecase, worktree_path, &run_id).await?;
    let usecase = state.workflow_usecase.clone();
    let result = tokio::task::spawn_blocking(move || {
        usecase
            .get_output(&run_id, &step_name)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("task join error: {e}"))??;
    Ok(match result {
        WorkflowGetOutputResult::Submitted {
            contract,
            structured_output,
            submitted_at,
            request_id,
            timestamp,
        } => WorkflowGetOutputResponse::Submitted {
            contract,
            structured_output,
            submitted_at,
            request_id,
            timestamp,
        },
        WorkflowGetOutputResult::NotSubmitted => WorkflowGetOutputResponse::NotSubmitted,
    })
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum WorkflowValidateOutputResponse {
    Valid,
    Invalid { reason: String, details: String },
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum WorkflowGetOutputResponse {
    Submitted {
        contract: Option<String>,
        structured_output: serde_json::Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        submitted_at: Option<f64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        request_id: Option<String>,
        timestamp: f64,
    },
    NotSubmitted,
}
