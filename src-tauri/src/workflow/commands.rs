use super::engine::{ApprovalDecision, WorkflowEngine};
use super::log::{WorkflowEventLog, WorkflowLogEvent};
use super::schema::{Summary, Workflow};
use super::storage;
use crate::agent_sdk::AgentProcessMap;
use crate::config::AppConfig;
use crate::session::{resolve_data_dir, SessionStore, WorkflowState};
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
    let file_stem = workflow_name.clone();
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
            &file_stem,
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
    worktree_path: String,
) -> Result<(), String> {
    engine
        .abort_workflow(&app, session_store.inner(), handles.inner(), &worktree_path)
        .await
        .map_err(|e| {
            let msg = e.to_string();
            log::error!("abort_workflow failed for worktree {worktree_path}: {msg}");
            msg
        })
}

#[tauri::command]
pub async fn get_workflow_state(
    engine: tauri::State<'_, Arc<WorkflowEngine>>,
    worktree_path: String,
) -> Result<Option<WorkflowState>, String> {
    Ok(engine.get_state(&worktree_path).await)
}

#[tauri::command]
pub async fn approve_workflow_step(
    app: tauri::AppHandle,
    handles: tauri::State<'_, Arc<Mutex<AgentProcessMap>>>,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    engine: tauri::State<'_, Arc<WorkflowEngine>>,
    worktree_path: String,
    decision: ApprovalDecision,
) -> Result<(), String> {
    engine
        .handle_approval(
            &app,
            session_store.inner(),
            handles.inner(),
            &worktree_path,
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
    worktree_path: String,
    abort: bool,
) -> Result<(), String> {
    engine
        .complete_interactive(
            &app,
            session_store.inner(),
            handles.inner(),
            &worktree_path,
            abort,
        )
        .await
        .map_err(|e| e.to_string())
}

fn validate_execution_id(execution_id: &str) -> Result<(), String> {
    if !execution_id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        return Err("Invalid execution_id format".to_string());
    }
    Ok(())
}

// ---- ワークフロー履歴閲覧コマンド ----

#[tauri::command]
pub async fn list_workflow_executions(
    app: tauri::AppHandle,
    worktree_path: String,
) -> Result<Vec<String>, String> {
    let data_dir = resolve_data_dir(&app)?;
    let event_log = WorkflowEventLog::new(&data_dir);
    tokio::task::spawn_blocking(move || event_log.list_execution_ids_for_worktree(&worktree_path))
        .await
        .map_err(|e| format!("task join error: {e}"))?
}

#[tauri::command]
pub async fn get_workflow_execution_log(
    app: tauri::AppHandle,
    execution_id: String,
) -> Result<Vec<WorkflowLogEvent>, String> {
    validate_execution_id(&execution_id)?;
    let data_dir = resolve_data_dir(&app)?;
    let event_log = WorkflowEventLog::new(&data_dir);
    tokio::task::spawn_blocking(move || event_log.read_log(&execution_id))
        .await
        .map_err(|e| format!("task join error: {e}"))?
}

#[tauri::command]
pub async fn get_workflow_execution_state(
    app: tauri::AppHandle,
    execution_id: String,
) -> Result<Option<WorkflowState>, String> {
    validate_execution_id(&execution_id)?;
    let data_dir = resolve_data_dir(&app)?;
    let workflows_dir = storage::workflows_dir();
    tokio::task::spawn_blocking(move || {
        let event_log = WorkflowEventLog::new(&data_dir);
        let events = event_log.read_log(&execution_id)?;
        // ログからワークフロー定義を取得（スナップショット優先、なければYAMLファイルにフォールバック）
        let started = events.iter().find_map(|e| match e {
            super::log::WorkflowLogEvent::WorkflowStarted {
                workflow_definition,
                workflow_file_stem,
                workflow_name,
                ..
            } => {
                let stem = if workflow_file_stem.is_empty() {
                    workflow_name.clone()
                } else {
                    workflow_file_stem.clone()
                };
                Some((workflow_definition.clone(), stem))
            }
            _ => None,
        });
        let Some((snapshot_def, file_stem)) = started else {
            return Ok(None);
        };
        let workflow = if let Some(def) = snapshot_def {
            def
        } else {
            let file_path = workflows_dir.join(format!("{file_stem}.yml"));
            match storage::load_workflow(&file_path) {
                Ok(w) => w,
                Err(e) => {
                    if file_path.exists() {
                        log::warn!(
                            "Failed to load workflow definition '{}': {e}",
                            file_path.display()
                        );
                    }
                    return Ok(None);
                }
            }
        };
        WorkflowEventLog::reconstruct_state_from_events(&execution_id, &events, &workflow)
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}
