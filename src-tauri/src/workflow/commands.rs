use super::engine::{ApprovalDecision, WorkflowEngine};
use super::schema::{Summary, Workflow};
use super::storage;
use crate::agent_sdk::AgentProcessMap;
use crate::config::AppConfig;
use crate::session::{SessionStore, WorkflowState};
use std::sync::Arc;
use tokio::sync::Mutex;

#[tauri::command]
pub async fn list_workflows() -> Result<Vec<Summary>, String> {
    let dir = storage::workflows_dir();
    tokio::task::spawn_blocking(move || storage::list_workflows(&dir).map_err(|e| e.to_string()))
        .await
        .map_err(|e| format!("task join error: {e}"))?
}

#[tauri::command]
pub async fn get_workflow(name: String) -> Result<Workflow, String> {
    let dir = storage::workflows_dir();
    tokio::task::spawn_blocking(move || {
        super::validation::validate_name(&name).map_err(|e| e.to_string())?;
        let file_path = dir.join(format!("{name}.yml"));
        storage::load_workflow(&file_path).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

#[tauri::command]
pub async fn save_workflow(workflow: Workflow) -> Result<(), String> {
    let dir = storage::workflows_dir();
    tokio::task::spawn_blocking(move || {
        storage::save_workflow(&dir, &workflow).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

#[tauri::command]
pub async fn delete_workflow(name: String) -> Result<(), String> {
    let dir = storage::workflows_dir();
    tokio::task::spawn_blocking(move || {
        storage::delete_workflow(&dir, &name).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

#[tauri::command]
pub fn open_workflow_in_editor(
    app: tauri::AppHandle,
    state: tauri::State<'_, Arc<AppConfig>>,
    name: String,
) -> Result<(), String> {
    let dir = storage::workflows_dir();
    let file_path = storage::resolve_workflow_path(&dir, &name).map_err(|e| e.to_string())?;

    let path_str = file_path.to_string_lossy().to_string();
    let config = state.get_config()?;
    crate::external_editor::open_path_with_opener(
        &app,
        &path_str,
        &config.app.external_editor,
        "ワークフロー",
    )
}

#[tauri::command]
pub async fn list_prompt_templates() -> Result<Vec<Summary>, String> {
    let dir = storage::prompts_dir();
    tokio::task::spawn_blocking(move || storage::list_prompts(&dir).map_err(|e| e.to_string()))
        .await
        .map_err(|e| format!("task join error: {e}"))?
}

// ---- ワークフロー実行コマンド ----

#[tauri::command]
pub async fn start_workflow(
    app: tauri::AppHandle,
    handles: tauri::State<'_, Arc<Mutex<AgentProcessMap>>>,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    engine: tauri::State<'_, Arc<WorkflowEngine>>,
    workflow_name: String,
    chat_session_id: String,
) -> Result<(), String> {
    let dir = storage::workflows_dir();
    let workflow = tokio::task::spawn_blocking(move || {
        super::validation::validate_name(&workflow_name).map_err(|e| e.to_string())?;
        let file_path = dir.join(format!("{workflow_name}.yml"));
        storage::load_workflow(&file_path).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("task join error: {e}"))??;

    engine
        .start_workflow(
            &app,
            session_store.inner(),
            handles.inner(),
            workflow,
            &chat_session_id,
        )
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn abort_workflow(
    app: tauri::AppHandle,
    handles: tauri::State<'_, Arc<Mutex<AgentProcessMap>>>,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    engine: tauri::State<'_, Arc<WorkflowEngine>>,
    chat_session_id: String,
) -> Result<(), String> {
    engine
        .abort_workflow(
            &app,
            session_store.inner(),
            handles.inner(),
            &chat_session_id,
        )
        .await
        .map_err(|e| {
            let msg = e.to_string();
            log::error!("abort_workflow failed for session {chat_session_id}: {msg}");
            msg
        })
}

#[tauri::command]
pub async fn get_workflow_state(
    engine: tauri::State<'_, Arc<WorkflowEngine>>,
    chat_session_id: String,
) -> Result<Option<WorkflowState>, String> {
    Ok(engine.get_state(&chat_session_id).await)
}

#[tauri::command]
pub async fn approve_workflow_step(
    app: tauri::AppHandle,
    handles: tauri::State<'_, Arc<Mutex<AgentProcessMap>>>,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    engine: tauri::State<'_, Arc<WorkflowEngine>>,
    chat_session_id: String,
    decision: ApprovalDecision,
) -> Result<(), String> {
    engine
        .handle_approval(
            &app,
            session_store.inner(),
            handles.inner(),
            &chat_session_id,
            decision,
        )
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn complete_interactive_step(
    app: tauri::AppHandle,
    handles: tauri::State<'_, Arc<Mutex<AgentProcessMap>>>,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    engine: tauri::State<'_, Arc<WorkflowEngine>>,
    chat_session_id: String,
    abort: bool,
) -> Result<(), String> {
    engine
        .complete_interactive(
            &app,
            session_store.inner(),
            handles.inner(),
            &chat_session_id,
            abort,
        )
        .await
        .map_err(|e| e.to_string())
}
