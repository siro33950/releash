use std::sync::Arc;

use tauri::State;

use crate::infrastructure::platform::app_data_dir::resolve_data_dir;
use crate::usecase::agent_session::backend_registry::AgentBackendRegistry;
use crate::usecase::agent_session::runtime::AgentSessionRuntimeUsecase;
use crate::usecase::agent_session::session::{
    add_message_internal, ChatMessage, ChatSession, CloseSessionOutcome, MessageRole,
    RestoreSessionOutcome, RestoreSessionResponse, SessionStore, SessionSummary,
    StoredSessionClosePort, StoredSessionLifecycleUsecase,
};
use crate::usecase::agent_session::workspace_session_creation::{
    SessionCreationRequest, WorkspaceSessionCreationRequest, WorkspaceSessionCreationUsecase,
};

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
    creation: State<'_, Arc<WorkspaceSessionCreationUsecase>>,
    registry: State<'_, Arc<AgentBackendRegistry>>,
    app: tauri::AppHandle,
    worktree_path: String,
    permission_mode: String,
    backend_id: Option<String>,
    model_id: Option<String>,
) -> Result<ChatSession, String> {
    let data_dir = resolve_data_dir(&app)?;
    creation.create_session(
        registry.inner().as_ref(),
        &data_dir,
        SessionCreationRequest {
            worktree_path,
            permission_mode,
            backend_id,
            model_id,
        },
    )
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub fn create_workspace_session(
    creation: State<'_, Arc<WorkspaceSessionCreationUsecase>>,
    registry: State<'_, Arc<AgentBackendRegistry>>,
    app: tauri::AppHandle,
    request_id: String,
    worktree_path: String,
    permission_mode: String,
    backend_id: Option<String>,
    model_id: Option<String>,
) -> Result<String, String> {
    let data_dir = resolve_data_dir(&app)?;
    creation.create_workspace_session(
        registry.inner().as_ref(),
        &data_dir,
        WorkspaceSessionCreationRequest {
            request_id,
            session: SessionCreationRequest {
                worktree_path,
                permission_mode,
                backend_id,
                model_id,
            },
        },
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
    lifecycle: State<'_, Arc<StoredSessionLifecycleUsecase>>,
    app: tauri::AppHandle,
    session_id: String,
) -> Result<(), String> {
    let data_dir = resolve_data_dir(&app)?;
    lifecycle.archive_session(&data_dir, &session_id).await
}

#[tauri::command]
pub async fn archive_open_session(
    lifecycle: State<'_, Arc<StoredSessionLifecycleUsecase>>,
    app: tauri::AppHandle,
    session_id: String,
) -> Result<(), String> {
    let data_dir = resolve_data_dir(&app)?;
    lifecycle.archive_open_session(&data_dir, &session_id).await
}

#[tauri::command]
pub async fn fork_session(
    lifecycle: State<'_, Arc<StoredSessionLifecycleUsecase>>,
    app: tauri::AppHandle,
    session_id: String,
) -> Result<ChatSession, String> {
    let data_dir = resolve_data_dir(&app)?;
    lifecycle.fork_session(&data_dir, &session_id).await
}

#[tauri::command]
pub async fn set_session_title(
    state: State<'_, Arc<SessionStore>>,
    runtime: State<'_, Arc<AgentSessionRuntimeUsecase>>,
    app: tauri::AppHandle,
    session_id: String,
    title: Option<String>,
) -> Result<SessionSummary, String> {
    let data_dir = resolve_data_dir(&app)?;
    state
        .get_session_meta(&data_dir, &session_id)?
        .ok_or_else(|| format!("Session not found: {session_id}"))?;
    let summary = state.set_session_title(&data_dir, &session_id, title.as_deref())?;
    let has_custom_title = title
        .as_deref()
        .map(|value| value.split_whitespace().collect::<Vec<_>>().join(" "))
        .is_some_and(|value| !value.is_empty());
    if has_custom_title {
        if let Err(err) = runtime
            .set_session_title(&session_id, &summary.first_message)
            .await
        {
            log::debug!("skipped runtime title sync for {session_id}: {err}");
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
    update_session_agent_info_in_store(
        state.inner().as_ref(),
        &data_dir,
        &session_id,
        agent_session_id,
    )
}

fn update_session_agent_info_in_store(
    state: &SessionStore,
    data_dir: &std::path::Path,
    session_id: &str,
    agent_session_id: Option<String>,
) -> Result<(), String> {
    state.update_agent_session_id(data_dir, session_id, agent_session_id)
}

#[tauri::command]
pub async fn close_session(
    lifecycle: State<'_, Arc<StoredSessionLifecycleUsecase>>,
    app: tauri::AppHandle,
    session_id: String,
) -> Result<(), String> {
    let data_dir = resolve_data_dir(&app)?;
    let outcome =
        close_session_with_usecase(lifecycle.inner().as_ref(), &data_dir, &session_id).await?;
    if let CloseSessionOutcome::WorkflowNodeTabClosed { worktree_path } = outcome {
        crate::adaptor::controller_support::emit_workflow_node_target_state(&app, &worktree_path)
            .await;
    }
    Ok(())
}

async fn close_session_with_usecase(
    lifecycle: &dyn StoredSessionClosePort,
    data_dir: &std::path::Path,
    session_id: &str,
) -> Result<CloseSessionOutcome, String> {
    lifecycle.close_session(data_dir, session_id).await
}

#[tauri::command]
pub async fn restore_session(
    lifecycle: State<'_, Arc<StoredSessionLifecycleUsecase>>,
    registry: State<'_, Arc<AgentBackendRegistry>>,
    app: tauri::AppHandle,
    session_id: String,
) -> Result<RestoreSessionResponse, String> {
    let data_dir = resolve_data_dir(&app)?;
    let outcome = lifecycle
        .restore_session(&data_dir, &session_id, registry.inner().as_ref())
        .await?;
    let response = restore_session_response(&outcome);
    if let RestoreSessionOutcome::WorkflowNodeTabRestored { worktree_path } = outcome {
        crate::adaptor::controller_support::emit_workflow_node_target_state(&app, &worktree_path)
            .await;
    }
    Ok(response)
}

fn restore_session_response(outcome: &RestoreSessionOutcome) -> RestoreSessionResponse {
    RestoreSessionResponse {
        restored_workflow_node: matches!(
            outcome,
            RestoreSessionOutcome::WorkflowNodeTabRestored { .. }
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use parking_lot::Mutex;

    #[derive(Default)]
    struct RecordingClosePort {
        calls: Mutex<Vec<(std::path::PathBuf, String)>>,
    }

    #[async_trait::async_trait]
    impl StoredSessionClosePort for RecordingClosePort {
        async fn close_session(
            &self,
            data_dir: &std::path::Path,
            session_id: &str,
        ) -> Result<CloseSessionOutcome, String> {
            self.calls
                .lock()
                .push((data_dir.to_path_buf(), session_id.to_string()));
            Ok(CloseSessionOutcome::StoredSessionClosed)
        }
    }

    #[tokio::test]
    async fn close_session_command_boundary_delegates_to_shared_close_usecase() {
        let port = RecordingClosePort::default();
        let data_dir = std::path::Path::new("/app-data");

        let outcome = close_session_with_usecase(&port, data_dir, "session-a")
            .await
            .unwrap();

        assert_eq!(outcome, CloseSessionOutcome::StoredSessionClosed);
        assert_eq!(
            port.calls.lock().as_slice(),
            [(data_dir.to_path_buf(), "session-a".to_string())]
        );
    }

    #[test]
    fn restore_session_outcome_maps_to_existing_wire_response() {
        assert!(
            !restore_session_response(&RestoreSessionOutcome::StoredSessionRestored)
                .restored_workflow_node
        );
        assert!(
            restore_session_response(&RestoreSessionOutcome::WorkflowNodeTabRestored {
                worktree_path: "/repo".to_string(),
            })
            .restored_workflow_node
        );
    }

    #[test]
    fn update_session_agent_info_does_not_infer_context_carry_from_manual_id() {
        let tmp = tempfile::TempDir::new().unwrap();
        let session_store = crate::test_support::build_session_store();
        let session = crate::usecase::agent_session::session::create_session_internal(
            &session_store,
            tmp.path(),
            "/repo",
            Some("claude".to_string()),
        )
        .unwrap();

        update_session_agent_info_in_store(
            &session_store,
            tmp.path(),
            &session.id,
            Some("manual-sdk-session".to_string()),
        )
        .unwrap();

        let loaded = session_store
            .load_full_session_for_restore(tmp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(
            loaded.agent_session_id.as_deref(),
            Some("manual-sdk-session")
        );
        assert_eq!(loaded.context_carry, None);
    }
}
