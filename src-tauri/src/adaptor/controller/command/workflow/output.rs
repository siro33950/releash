use std::sync::Arc;

use crate::adaptor::controller::state::AppState;
use crate::adaptor::protocol::workflow::{
    WorkflowGetOutputResponse, WorkflowSubmitArtifactInput, WorkflowValidateOutputResponse,
};
use crate::usecase::workflow::command::{SubmitOutputArtifact, SubmitOutputCommand};
use crate::usecase::workflow::{
    WorkflowGetOutputResult, WorkflowRuntimeUsecase, WorkflowValidateOutputResult,
};

/// [08] `workflow output` 系 Tauri command の共通 authorize ヘルパー。
///
/// 3 ハンドラ（submit / validate / get）を同じUsecase認可境界へ接続する。
/// 認可外 worktree / 不存在 execution のいずれも `Workflow execution not found` 同表現で
/// 拒否し、存在情報を漏らさない（spec [08] L169 / L182）。
async fn authorize_output_execution_access(
    query: &Arc<crate::usecase::workflow::WorkflowUsecase>,
    worktree_path: String,
    execution_id: &str,
) -> Result<(), String> {
    let query = query.clone();
    let execution_id = execution_id.to_string();
    tokio::task::spawn_blocking(move || {
        query
            .authorize_execution_access_for_worktree(&execution_id, &worktree_path)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

async fn authorize_output_node_execution_access(
    query: &Arc<crate::usecase::workflow::WorkflowUsecase>,
    worktree_path: String,
    node_execution_id: &str,
) -> Result<(), String> {
    let query = query.clone();
    let node_execution_id = node_execution_id.to_string();
    tokio::task::spawn_blocking(move || {
        query
            .authorize_node_execution_access_for_worktree(&node_execution_id, &worktree_path)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

/// Tauri command 経路: NodeExecutionにSubmit signalとoptional Artifactを提出する。
///
/// in-process caller (UI / API / 外部 Tauri caller) は workflow runtime usecase
/// を経由して、CLI 経路と同一の state mutation 境界に合流する（spec [08] L169）。
/// worktree-scoped 認可境界（spec [08] が依拠する [05] 観測経路の認可境界）を通過しない caller には
/// 「該当 execution なし」と同表現で拒否を返し、存在情報を漏らさない。
#[tauri::command]
pub async fn workflow_submit_output(
    state: tauri::State<'_, AppState>,
    runtime: tauri::State<'_, Arc<WorkflowRuntimeUsecase>>,
    worktree_path: String,
    node_execution_id: String,
    artifact: Option<WorkflowSubmitArtifactInput>,
) -> Result<(), String> {
    authorize_output_node_execution_access(
        &state.workflow_usecase,
        worktree_path,
        &node_execution_id,
    )
    .await?;
    runtime
        .submit_output(SubmitOutputCommand {
            node_execution_id,
            artifact: artifact.map(|artifact| SubmitOutputArtifact {
                contract: artifact.contract,
                value: artifact.value,
            }),
        })
        .await
        .map_err(|e| e.to_string())
}

/// [08] structured output の contract 適合性のみを副作用なしで判定する Tauri command。
/// `submit` と異なり engine state / event log には触れない（spec [08] 振る舞い定義 Rule 2）。
///
/// caller は execution_id / node_name を主語に呼び出す。contract type は backend 側で
/// event log の `ExecutionStarted.workflow_definition` から解決する（spec [08] 要求:
/// 「execution_id と node_name を主語に CLI/API 経由で engine に提出できる」境界。
/// caller 指定の contract 文字列を信用しない）。
#[tauri::command]
pub async fn workflow_validate_output(
    state: tauri::State<'_, AppState>,
    worktree_path: String,
    execution_id: String,
    node_name: String,
    structured_output: serde_json::Value,
) -> Result<WorkflowValidateOutputResponse, String> {
    authorize_output_execution_access(&state.workflow_usecase, worktree_path, &execution_id)
        .await?;
    // [08] preflight と本 submit (`handle_submit_output`) で同一の前処理 + validation を
    // 共有するため、masking + validate を集約した `preprocess_and_validate_output` を経由する。
    // raw JSON のまま `validate_contract_value` を呼ぶと submit 側の redaction 後の値と
    // 判定が食い違う構造になるため、usecase 側の SecretSourceGateway 経由で redaction 後に判定する。
    let usecase = state.workflow_usecase.clone();
    let result = tokio::task::spawn_blocking(move || {
        usecase
            .validate_output(&execution_id, &node_name, structured_output)
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
    execution_id: String,
    node_name: String,
) -> Result<WorkflowGetOutputResponse, String> {
    authorize_output_execution_access(&state.workflow_usecase, worktree_path, &execution_id)
        .await?;
    let usecase = state.workflow_usecase.clone();
    let result = tokio::task::spawn_blocking(move || {
        usecase
            .get_output(&execution_id, &node_name)
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
