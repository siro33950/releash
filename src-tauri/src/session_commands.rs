use std::sync::Arc;

use tauri::State;
use tokio::sync::Mutex;

use crate::app_data_dir::resolve_data_dir;
use crate::infrastructure::agent_session::runtime::AgentBackendRegistry;
use crate::infrastructure::agent_session::runtime::AgentProcessMap;
use crate::usecase::agent_session::session::{
    add_message_internal, create_session_command_inner, resolve_session_backend,
    update_session_state_in_data_dir, validate_session_permission_mode, ChatMessage, ChatSession,
    MessageRole, OpenTabRegistry, RestoreSessionResponse, SessionState, SessionStore,
    SessionSummary,
};
use crate::workflow::engine::WorkflowEngine;

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
) -> Result<ChatSession, String> {
    let data_dir = resolve_data_dir(&app)?;
    create_session_command_inner(
        state.inner().as_ref(),
        registry.inner().as_ref(),
        &data_dir,
        &worktree_path,
        &permission_mode,
        backend_id,
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
    crate::usecase::agent_session::session::lifecycle_controller::SessionLifecycleController {
        session_store: state.inner(),
        data_dir: &data_dir,
    }
    .restore_session_state(session)
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
}
