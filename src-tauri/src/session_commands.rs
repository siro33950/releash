use std::sync::Arc;

use tauri::State;
use tokio::sync::Mutex;

use crate::app_data_dir::resolve_data_dir;
use crate::infrastructure::agent_session::runtime::codex::configured_cli_path;
use crate::infrastructure::agent_session::runtime::codex_app_server::{
    build_thread_archive_request, build_thread_fork_request, build_thread_unarchive_request,
    CodexAppServerProcess,
};
use crate::infrastructure::agent_session::runtime::AgentBackendRegistry;
use crate::infrastructure::agent_session::runtime::{
    AgentProcessMap, SessionHandle, CODEX_BACKEND_ID,
};
use crate::usecase::agent_session::session::{
    add_message_internal, resolve_session_backend, update_session_state_in_data_dir,
    validate_session_permission_mode, ChatMessage, ChatSession, MessageRole, OpenTabRegistry,
    RestoreSessionResponse, SessionState, SessionStore, SessionSummary,
};
use crate::workflow::engine::WorkflowEngine;

fn saved_codex_thread_id(session: &ChatSession) -> Option<String> {
    if session.backend_id.as_deref() != Some(CODEX_BACKEND_ID) {
        return None;
    }
    session
        .agent_session_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn codex_thread_id_from_fork_response(response: &serde_json::Value) -> Option<String> {
    response
        .get("thread")
        .and_then(|thread| thread.get("id"))
        .and_then(serde_json::Value::as_str)
        .or_else(|| response.get("threadId").and_then(serde_json::Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

async fn send_codex_thread_lifecycle_request(
    app: &tauri::AppHandle,
    thread_id: &str,
    archive: bool,
) -> Result<(), String> {
    let cli_path = configured_cli_path(app).unwrap_or_else(|| "codex".to_string());
    let mut process = CodexAppServerProcess::spawn(&cli_path)?;
    let result = async {
        process.initialize(env!("CARGO_PKG_VERSION")).await?;
        let id = process.next_request_id();
        let request = if archive {
            build_thread_archive_request(id, thread_id)
        } else {
            build_thread_unarchive_request(id, thread_id)
        };
        process.send(&request).await?;
        process.read_response_result(id).await?;
        Ok(())
    }
    .await;
    process.shutdown().await;
    result
}

async fn fork_codex_thread_for_session(
    app: &tauri::AppHandle,
    session: &ChatSession,
) -> Result<Option<String>, String> {
    let Some(thread_id) = saved_codex_thread_id(session) else {
        return Ok(None);
    };
    let cli_path = configured_cli_path(app).unwrap_or_else(|| "codex".to_string());
    let mut process = CodexAppServerProcess::spawn(&cli_path)?;
    let result = async {
        process.initialize(env!("CARGO_PKG_VERSION")).await?;
        let id = process.next_request_id();
        let request = build_thread_fork_request(
            id,
            &thread_id,
            &session.worktree_path,
            session.selected_model.as_deref(),
            Some(&session.permission_mode),
            false,
            session.permission_profile_id.as_deref(),
        )?;
        process.send(&request).await?;
        let response = process.read_response_result(id).await?;
        codex_thread_id_from_fork_response(&response)
            .map(Some)
            .ok_or_else(|| "Codex thread/fork response did not include thread.id".to_string())
    }
    .await;
    process.shutdown().await;
    result
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn list_sessions(
    state: State<'_, Arc<SessionStore>>,
    app: tauri::AppHandle,
    worktree_path: String,
) -> Result<Vec<SessionSummary>, String> {
    let data_dir = resolve_data_dir(&app)?;
    state.list_sessions(&data_dir, &worktree_path)
}

#[tauri::command]
pub fn create_session(
    state: State<'_, Arc<SessionStore>>,
    registry: State<'_, Arc<AgentBackendRegistry>>,
    app: tauri::AppHandle,
    worktree_path: String,
    permission_mode: String,
    backend_id: Option<String>,
    model_id: Option<String>,
) -> Result<ChatSession, String> {
    let data_dir = resolve_data_dir(&app)?;
    let permission_mode =
        crate::permission::PermissionMode::parse(&permission_mode).map_err(|e| e.to_string())?;
    let resolved_model = match model_id.as_deref() {
        Some(model_id) => Some(registry.resolve_model_entry(model_id)?),
        None => None,
    };
    let resolved_backend_id = registry.resolve_backend_id(
        resolved_model
            .as_ref()
            .map(|entry| entry.backend.clone())
            .or(backend_id),
    )?;
    crate::usecase::agent_session::session::create_session_with_model_and_plan_mode(
        state.inner().as_ref(),
        registry.inner().as_ref(),
        &data_dir,
        &worktree_path,
        resolved_backend_id,
        permission_mode,
        resolved_model.map(|entry| entry.model_id),
        false,
    )
}

#[tauri::command]
pub fn add_message(
    state: State<'_, Arc<SessionStore>>,
    app: tauri::AppHandle,
    session_id: String,
    role: MessageRole,
    content: String,
) -> Result<ChatMessage, String> {
    let data_dir = resolve_data_dir(&app)?;
    add_message_internal(&state, &data_dir, &session_id, role, &content, None, None)
}

#[tauri::command]
pub fn update_session_state(
    state: State<'_, Arc<SessionStore>>,
    app: tauri::AppHandle,
    session_id: String,
    new_state: SessionState,
) -> Result<(), String> {
    let data_dir = resolve_data_dir(&app)?;
    update_session_state_in_data_dir(&state, &data_dir, &session_id, new_state)
}

#[tauri::command]
pub async fn list_closed_sessions(
    state: State<'_, Arc<SessionStore>>,
    app: tauri::AppHandle,
    worktree_path: String,
) -> Result<Vec<SessionSummary>, String> {
    let data_dir = resolve_data_dir(&app)?;
    state.list_closed_sessions(&data_dir, &worktree_path)
}

#[tauri::command]
pub async fn archive_session(
    state: State<'_, Arc<SessionStore>>,
    app: tauri::AppHandle,
    session_id: String,
) -> Result<(), String> {
    let data_dir = resolve_data_dir(&app)?;
    state.archive_session(&data_dir, &session_id)?;
    let codex_thread_id = state
        .get_session(&data_dir, &session_id)
        .ok()
        .flatten()
        .and_then(|session| saved_codex_thread_id(&session));
    if let Some(thread_id) = codex_thread_id {
        if let Err(err) = send_codex_thread_lifecycle_request(&app, &thread_id, true).await {
            log::debug!("skipped Codex runtime thread archive sync for {session_id}: {err}");
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn archive_open_session(
    state: State<'_, Arc<SessionStore>>,
    handles: State<'_, Arc<Mutex<AgentProcessMap>>>,
    app: tauri::AppHandle,
    session_id: String,
) -> Result<(), String> {
    let data_dir = resolve_data_dir(&app)?;
    state.archive_open_session(&data_dir, &session_id)?;
    crate::infrastructure::agent_session::runtime::close_agent_session_internal(
        &app,
        handles.inner(),
        &session_id,
    )
    .await?;
    let codex_thread_id = state
        .get_session(&data_dir, &session_id)
        .ok()
        .flatten()
        .and_then(|session| saved_codex_thread_id(&session));
    if let Some(thread_id) = codex_thread_id {
        if let Err(err) = send_codex_thread_lifecycle_request(&app, &thread_id, true).await {
            log::debug!("skipped Codex runtime open-thread archive sync for {session_id}: {err}");
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn fork_session(
    state: State<'_, Arc<SessionStore>>,
    app: tauri::AppHandle,
    session_id: String,
) -> Result<ChatSession, String> {
    let data_dir = resolve_data_dir(&app)?;
    let source_session = state
        .get_session(&data_dir, &session_id)?
        .ok_or_else(|| format!("Session not found: {session_id}"))?;
    let mut forked = state.fork_session(&data_dir, &session_id)?;
    match fork_codex_thread_for_session(&app, &source_session).await {
        Ok(Some(thread_id)) => {
            forked.agent_session_id = Some(thread_id);
            state.save_session(&data_dir, &forked)?;
        }
        Ok(None) => {}
        Err(err) => {
            log::debug!("skipped Codex runtime thread fork sync for {session_id}: {err}");
        }
    }
    Ok(forked)
}

#[tauri::command]
pub async fn set_session_title(
    state: State<'_, Arc<SessionStore>>,
    registry: State<'_, Arc<AgentBackendRegistry>>,
    app: tauri::AppHandle,
    session_id: String,
    title: Option<String>,
) -> Result<SessionSummary, String> {
    let data_dir = resolve_data_dir(&app)?;
    let session = state
        .get_session(&data_dir, &session_id)?
        .ok_or_else(|| format!("Session not found: {session_id}"))?;
    let summary = state.set_session_title(&data_dir, &session_id, title.as_deref())?;
    let has_custom_title = title
        .as_deref()
        .map(|value| value.split_whitespace().collect::<Vec<_>>().join(" "))
        .is_some_and(|value| !value.is_empty());
    if session.backend_id.as_deref() == Some(CODEX_BACKEND_ID) && has_custom_title {
        if let Some(backend) = registry.get(CODEX_BACKEND_ID) {
            if let Err(err) = backend
                .set_thread_name(
                    &SessionHandle {
                        chat_session_id: session_id.clone(),
                        backend_id: CODEX_BACKEND_ID.to_string(),
                    },
                    &summary.first_message,
                )
                .await
            {
                log::debug!("skipped Codex runtime title sync for {session_id}: {err}");
            }
        }
    }
    Ok(summary)
}

#[tauri::command]
pub fn update_session_agent_info(
    state: State<'_, Arc<SessionStore>>,
    app: tauri::AppHandle,
    session_id: String,
    agent_session_id: Option<String>,
) -> Result<(), String> {
    let data_dir = resolve_data_dir(&app)?;
    let mut session = state
        .get_session(&data_dir, &session_id)?
        .ok_or_else(|| format!("Session not found: {session_id}"))?;
    session.agent_session_id = agent_session_id;
    session.updated_at = crate::usecase::agent_session::session::now_timestamp();
    state.save_session(&data_dir, &session)?;
    Ok(())
}

#[tauri::command]
pub async fn close_session(
    state: State<'_, Arc<SessionStore>>,
    engine: State<'_, Arc<WorkflowEngine>>,
    handles: State<'_, Arc<Mutex<AgentProcessMap>>>,
    open_tabs: State<'_, Arc<OpenTabRegistry>>,
    app: tauri::AppHandle,
    session_id: String,
) -> Result<(), String> {
    let lifecycle = crate::workflow_step_lifecycle_adapters::TauriWorkflowStepLifecycle::new(
        &app,
        state.inner().as_ref(),
        handles.inner(),
        open_tabs.inner().as_ref(),
    );
    if let Some(target) = lifecycle
        .close_tab_target(&session_id)
        .await
        .map_err(|_| crate::workflow::session_errors::workflow_step_tab_operation_failed())?
    {
        crate::workflow_state_events::emit_workflow_step_target_state(
            &app,
            engine.inner(),
            &target,
            handles.inner(),
            open_tabs.inner(),
        )
        .await;
        return Ok(());
    }

    crate::infrastructure::agent_session::runtime::close_agent_session_internal(
        &app,
        handles.inner(),
        &session_id,
    )
    .await?;
    let data_dir = resolve_data_dir(&app)?;
    crate::usecase::agent_session::session::lifecycle_controller::SessionLifecycleController {
        session_store: state.inner(),
        data_dir: &data_dir,
    }
    .close_session_state(&session_id)?;
    Ok(())
}

#[cfg(test)]
async fn close_session_state_after_runtime<F, Fut>(
    session_store: &Arc<SessionStore>,
    data_dir: &std::path::Path,
    session_id: &str,
    close_runtime: F,
) -> Result<(), String>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<(), String>>,
{
    close_runtime().await?;
    crate::usecase::agent_session::session::lifecycle_controller::SessionLifecycleController {
        session_store,
        data_dir,
    }
    .close_session_state(session_id)
}

#[tauri::command]
pub async fn restore_session(
    state: State<'_, Arc<SessionStore>>,
    registry: State<'_, Arc<AgentBackendRegistry>>,
    engine: State<'_, Arc<WorkflowEngine>>,
    handles: State<'_, Arc<Mutex<AgentProcessMap>>>,
    open_tabs: State<'_, Arc<OpenTabRegistry>>,
    app: tauri::AppHandle,
    session_id: String,
) -> Result<RestoreSessionResponse, String> {
    let lifecycle = crate::workflow_step_lifecycle_adapters::TauriWorkflowStepLifecycle::new(
        &app,
        state.inner().as_ref(),
        handles.inner(),
        open_tabs.inner().as_ref(),
    );
    if let Some(target) = lifecycle
        .try_open_tab(&session_id)
        .await
        .map_err(|_| crate::workflow::session_errors::workflow_step_tab_operation_failed())?
    {
        crate::workflow_state_events::emit_workflow_step_target_state(
            &app,
            engine.inner(),
            &target,
            handles.inner(),
            open_tabs.inner(),
        )
        .await;
        return Ok(RestoreSessionResponse {
            restored_workflow_step: true,
        });
    }

    let data_dir = resolve_data_dir(&app)?;
    let mut session = state
        .get_session(&data_dir, &session_id)?
        .ok_or_else(|| format!("Session not found: {session_id}"))?;
    validate_session_permission_mode(&session)?;
    resolve_session_backend(&mut session, registry.inner())?;
    let codex_thread_id = saved_codex_thread_id(&session);
    let response =
        crate::usecase::agent_session::session::lifecycle_controller::SessionLifecycleController {
            session_store: state.inner(),
            data_dir: &data_dir,
        }
        .restore_session_state(session)?;
    if let Some(thread_id) = codex_thread_id {
        if let Err(err) = send_codex_thread_lifecycle_request(&app, &thread_id, false).await {
            log::debug!("skipped Codex runtime thread unarchive sync for {session_id}: {err}");
        }
    }
    Ok(response)
}

#[cfg(test)]
fn restore_workflow_step_session_tab_state(
    session_store: &SessionStore,
    data_dir: &std::path::Path,
    open_tabs: &OpenTabRegistry,
    session_id: &str,
) -> Result<Option<(RestoreSessionResponse, String)>, String> {
    let Some(target) = crate::workflow_step_lifecycle_adapters::resolve_step_session_with_data_dir(
        session_store,
        data_dir,
        session_id,
    )
    .map_err(|_| crate::workflow::session_errors::workflow_step_tab_operation_failed())?
    else {
        return Ok(None);
    };
    crate::workflow_step_lifecycle_adapters::open_step_session_tab_state(
        session_store,
        data_dir,
        open_tabs,
        &target.session_id,
    )
    .map_err(|_| crate::workflow::session_errors::workflow_step_tab_operation_failed())?;
    Ok(Some((
        RestoreSessionResponse {
            restored_workflow_step: true,
        },
        target.worktree_path,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usecase::agent_session::session::SessionState;
    use serde_json::json;

    fn workflow_step_session_for_test(
        session_id: &str,
    ) -> crate::usecase::agent_session::session::ChatSession {
        crate::usecase::agent_session::session::ChatSession {
            id: session_id.to_string(),
            worktree_path: "/repo".to_string(),
            messages: vec![],
            state: SessionState::Closed,
            created_at: 1.0,
            updated_at: 1.0,
            agent_session_id: Some("sdk-session".to_string()),
            permission_mode: "edit".to_string(),
            plan_mode: false,
            permission_profile_id: None,
            selected_model: None,
            backend_id: Some(
                crate::infrastructure::agent_session::runtime::CLAUDE_BACKEND_ID.to_string(),
            ),
            workflow_step_session: true,
        }
    }

    #[tokio::test]
    async fn restore_workflow_step_session_tab_reopens_history_without_starting_runtime() {
        let tmp = tempfile::TempDir::new().unwrap();
        let session_store = SessionStore::default();
        let open_tabs = OpenTabRegistry::default();
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let session_id = uuid::Uuid::new_v4().to_string();

        session_store
            .save_session(tmp.path(), &workflow_step_session_for_test(&session_id))
            .unwrap();

        let (response, worktree_path) = restore_workflow_step_session_tab_state(
            &session_store,
            tmp.path(),
            &open_tabs,
            &session_id,
        )
        .unwrap()
        .expect("workflow step restore outcome");

        assert!(response.restored_workflow_step);
        assert_eq!(worktree_path, "/repo");
        assert!(open_tabs.contains(&session_id));
        assert!(handles.lock().await.is_empty());
        let session = session_store
            .get_session(tmp.path(), &session_id)
            .unwrap()
            .expect("session remains as history");
        assert_eq!(session.state, SessionState::Idle);
    }

    #[tokio::test]
    async fn close_session_state_after_runtime_marks_session_closed_on_success() {
        let tmp = tempfile::TempDir::new().unwrap();
        let session_store = Arc::new(SessionStore::default());
        let session = crate::usecase::agent_session::session::create_session_internal(
            &session_store,
            tmp.path(),
            "/repo",
            Some("claude".to_string()),
        )
        .unwrap();

        close_session_state_after_runtime(&session_store, tmp.path(), &session.id, || async {
            Ok(())
        })
        .await
        .unwrap();

        let loaded = session_store
            .get_session(tmp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(loaded.state, SessionState::Closed);
    }

    #[tokio::test]
    async fn close_session_state_after_runtime_keeps_state_on_runtime_failure() {
        let tmp = tempfile::TempDir::new().unwrap();
        let session_store = Arc::new(SessionStore::default());
        let session = crate::usecase::agent_session::session::create_session_internal(
            &session_store,
            tmp.path(),
            "/repo",
            Some("claude".to_string()),
        )
        .unwrap();

        let err =
            close_session_state_after_runtime(&session_store, tmp.path(), &session.id, || async {
                Err("runtime close failed".to_string())
            })
            .await
            .unwrap_err();

        let loaded = session_store
            .get_session(tmp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(err, "runtime close failed");
        assert_eq!(loaded.state, session.state);
    }

    #[test]
    fn saved_codex_thread_id_requires_codex_backend_and_non_empty_id() {
        let mut session = workflow_step_session_for_test("session-1");
        session.backend_id = Some(CODEX_BACKEND_ID.to_string());
        session.agent_session_id = Some(" thread-1 ".to_string());
        assert_eq!(
            saved_codex_thread_id(&session),
            Some("thread-1".to_string())
        );

        session.backend_id = Some("claude".to_string());
        assert_eq!(saved_codex_thread_id(&session), None);

        session.backend_id = Some(CODEX_BACKEND_ID.to_string());
        session.agent_session_id = Some("   ".to_string());
        assert_eq!(saved_codex_thread_id(&session), None);
    }

    #[test]
    fn codex_thread_id_from_fork_response_reads_thread_id() {
        let response = json!({
            "thread": {
                "id": "thread-forked",
                "sessionId": "tree-1"
            }
        });
        assert_eq!(
            codex_thread_id_from_fork_response(&response),
            Some("thread-forked".to_string())
        );

        assert_eq!(
            codex_thread_id_from_fork_response(&json!({ "threadId": "legacy-id" })),
            Some("legacy-id".to_string())
        );
        assert_eq!(codex_thread_id_from_fork_response(&json!({})), None);
    }
}
