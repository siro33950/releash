use std::sync::Arc;

use tokio::sync::Mutex;

use crate::agent_message_dispatcher::{
    dispatch_agent_message, AgentMessageDispatchContext, AgentMessageDispatchRequest,
};
use crate::backends::{AgentBackendRegistry, ImageAttachment};
use crate::session::errors::session_target_rejected;
use crate::session::{resolve_data_dir, SessionStore};
use crate::workflow::engine::WorkflowEngine;

fn reject_explicit_start_for_workflow_step_session(
    session: &crate::session::ChatSession,
    cwd: &str,
) -> Result<(), String> {
    if session.worktree_path != cwd || session.workflow_step_session {
        return Err(session_target_rejected());
    }
    Ok(())
}

fn should_skip_close_agent_session(session: Option<&crate::session::ChatSession>) -> bool {
    session.is_some_and(|session| session.workflow_step_session)
}

#[tauri::command]
pub async fn start_agent_session(
    app: tauri::AppHandle,
    handles: tauri::State<'_, Arc<Mutex<crate::agent_sdk::AgentProcessMap>>>,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    chat_session_id: String,
    cwd: String,
    permission_mode: Option<String>,
) -> Result<(), String> {
    let data_dir = resolve_data_dir(&app)?;
    let session = session_store
        .get_session(&data_dir, &chat_session_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Session not found: {chat_session_id}"))?;
    reject_explicit_start_for_workflow_step_session(&session, &cwd)?;
    crate::agent_sdk::start_agent_session_internal(
        &app,
        handles.inner(),
        session_store.inner(),
        &chat_session_id,
        &cwd,
        permission_mode,
        None,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session_for_start_guard(workflow_step_session: bool) -> crate::session::ChatSession {
        crate::session::ChatSession {
            id: "session-1".to_string(),
            worktree_path: "/repo".to_string(),
            messages: Vec::new(),
            state: crate::session::SessionState::Idle,
            created_at: 1.0,
            updated_at: 1.0,
            agent_session_id: Some("sdk-session".to_string()),
            permission_mode: "acceptEdits".to_string(),
            selected_model: None,
            workflow_state: None,
            backend_id: Some(crate::agent_sdk::CLAUDE_BACKEND_ID.to_string()),
            workflow_step_session,
        }
    }

    #[test]
    fn start_agent_session_guard_rejects_workflow_step_session_before_runtime_start() {
        let session = session_for_start_guard(true);
        let handles = crate::agent_sdk::AgentProcessMap::new();

        let result = reject_explicit_start_for_workflow_step_session(&session, "/repo");

        assert_eq!(result.unwrap_err(), session_target_rejected());
        assert!(handles.is_empty());
    }

    #[test]
    fn start_agent_session_guard_allows_regular_session_in_matching_worktree() {
        let session = session_for_start_guard(false);

        assert!(reject_explicit_start_for_workflow_step_session(&session, "/repo").is_ok());
    }

    #[tokio::test]
    async fn close_agent_session_guard_keeps_workflow_step_runtime() {
        let session = session_for_start_guard(true);
        let handles = Arc::new(Mutex::new(crate::agent_sdk::AgentProcessMap::new()));
        handles.lock().await.insert(
            session.id.clone(),
            crate::agent_sdk::make_test_agent_process(),
        );

        assert!(should_skip_close_agent_session(Some(&session)));
        assert!(handles.lock().await.contains_key(&session.id));
    }
}

#[tauri::command]
pub async fn close_agent_session(
    app: tauri::AppHandle,
    handles: tauri::State<'_, Arc<Mutex<crate::agent_sdk::AgentProcessMap>>>,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    chat_session_id: String,
) -> Result<(), String> {
    let data_dir = resolve_data_dir(&app)?;
    let session = session_store
        .get_session(&data_dir, &chat_session_id)
        .map_err(|e| e.to_string())?;
    if should_skip_close_agent_session(session.as_ref()) {
        return Ok(());
    }
    crate::agent_sdk::close_agent_session_internal(&app, handles.inner(), &chat_session_id).await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn send_agent_message(
    app: tauri::AppHandle,
    handles: tauri::State<'_, Arc<Mutex<crate::agent_sdk::AgentProcessMap>>>,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    registry: tauri::State<'_, Arc<AgentBackendRegistry>>,
    engine: tauri::State<'_, Arc<WorkflowEngine>>,
    open_tabs: tauri::State<'_, Arc<crate::session::OpenTabRegistry>>,
    chat_session_id: Option<String>,
    worktree_path: String,
    content: String,
    permission_mode: Option<String>,
    backend_id: Option<String>,
    images: Option<Vec<ImageAttachment>>,
    mentions: Option<Vec<crate::file_mention::MentionReference>>,
) -> Result<crate::agent_sdk::SendMessageResponse, String> {
    let response = dispatch_agent_message(
        AgentMessageDispatchContext {
            app: &app,
            session_store: session_store.inner(),
            registry: registry.inner(),
            handles: handles.inner(),
        },
        AgentMessageDispatchRequest {
            chat_session_id,
            worktree_path,
            content,
            permission_mode,
            backend_id,
            images,
            mentions,
        },
    )
    .await?;
    crate::workflow_state_events::emit_after_workflow_step_message(
        &app,
        engine.inner(),
        &response.session,
        handles.inner(),
        open_tabs.inner(),
    )
    .await;
    Ok(response)
}
