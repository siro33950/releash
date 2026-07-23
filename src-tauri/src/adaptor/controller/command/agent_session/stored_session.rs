use std::sync::Arc;

use tauri::State;

use crate::adaptor::protocol::agent_session_v1::{ChatSessionDtoV1, SessionSummaryDtoV1};
use crate::infrastructure::platform::app_data_dir::resolve_data_dir;
use crate::usecase::agent_session::backend_registry::AgentBackendRegistry;
use crate::usecase::agent_session::runtime::AgentSessionRuntimeUsecase;
#[cfg(test)]
use crate::usecase::agent_session::session::{CloseSessionOutcome, StoredSessionClosePort};
use crate::usecase::agent_session::session::{
    RestoreSessionOutcome, RestoreSessionResponse, SessionStore, StoredSessionLifecycleUsecase,
};
use crate::usecase::agent_session::workspace_session_creation::{
    SessionCreationRequest, WorkspaceSessionCreationRequest, WorkspaceSessionCreationUsecase,
};

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn list_sessions(
    runtime: State<'_, Arc<AgentSessionRuntimeUsecase>>,
    worktree_path: String,
) -> Result<Vec<SessionSummaryDtoV1>, String> {
    runtime
        .list_sessions(&worktree_path)
        .await
        .map(|sessions| sessions.into_iter().map(Into::into).collect())
        .map_err(|error| error.to_string())
}

#[tauri::command]
#[allow(clippy::too_many_arguments)] // Tauri exposes state handles and request fields as separate command arguments.
pub async fn create_session(
    local_store: State<'_, Arc<crate::adaptor::gateway::local_event_store::LocalEventStore>>,
    creation: State<'_, Arc<WorkspaceSessionCreationUsecase>>,
    registry: State<'_, Arc<AgentBackendRegistry>>,
    app: tauri::AppHandle,
    worktree_path: String,
    permission_mode: String,
    backend_id: Option<String>,
    model_id: Option<String>,
) -> Result<ChatSessionDtoV1, String> {
    super::session::ensure_mutation_admission_message(local_store.inner().as_ref()).await?;
    let data_dir = resolve_data_dir(&app)?;
    creation
        .create_session(
            registry.inner().as_ref(),
            &data_dir,
            SessionCreationRequest {
                worktree_path,
                permission_mode,
                backend_id,
                model_id,
            },
        )
        .map(Into::into)
        .map_err(super::session::normalize_mutation_error)
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn create_workspace_session(
    local_store: State<'_, Arc<crate::adaptor::gateway::local_event_store::LocalEventStore>>,
    creation: State<'_, Arc<WorkspaceSessionCreationUsecase>>,
    registry: State<'_, Arc<AgentBackendRegistry>>,
    app: tauri::AppHandle,
    request_id: String,
    worktree_path: String,
    permission_mode: String,
    backend_id: Option<String>,
    model_id: Option<String>,
) -> Result<String, String> {
    super::session::ensure_mutation_admission_message(local_store.inner().as_ref()).await?;
    let data_dir = resolve_data_dir(&app)?;
    creation
        .create_workspace_session(
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
        .map_err(super::session::normalize_mutation_error)
}

#[tauri::command]
pub async fn list_closed_sessions(
    state: State<'_, Arc<SessionStore>>,
    app: tauri::AppHandle,
    worktree_path: String,
) -> Result<Vec<SessionSummaryDtoV1>, String> {
    let data_dir = resolve_data_dir(&app)?;
    state
        .list_closed_sessions(&data_dir, &worktree_path)
        .map(|sessions| sessions.into_iter().map(Into::into).collect())
}

#[tauri::command]
pub async fn fork_session(
    local_store: State<'_, Arc<crate::adaptor::gateway::local_event_store::LocalEventStore>>,
    lifecycle: State<'_, Arc<StoredSessionLifecycleUsecase>>,
    app: tauri::AppHandle,
    session_id: String,
) -> Result<ChatSessionDtoV1, String> {
    super::session::ensure_mutation_admission_message(local_store.inner().as_ref()).await?;
    let data_dir = resolve_data_dir(&app)?;
    lifecycle
        .fork_session(&data_dir, &session_id)
        .await
        .map(Into::into)
        .map_err(super::session::normalize_mutation_error)
}

#[tauri::command]
pub async fn set_session_title(
    local_store: State<'_, Arc<crate::adaptor::gateway::local_event_store::LocalEventStore>>,
    state: State<'_, Arc<SessionStore>>,
    app: tauri::AppHandle,
    session_id: String,
    title: Option<String>,
) -> Result<SessionSummaryDtoV1, String> {
    super::session::ensure_mutation_admission_message(local_store.inner().as_ref()).await?;
    let data_dir = resolve_data_dir(&app)?;
    state
        .get_session_meta(&data_dir, &session_id)?
        .ok_or_else(|| format!("Session not found: {session_id}"))?;
    let summary = state
        .set_session_title(&data_dir, &session_id, title.as_deref())
        .map_err(super::session::normalize_mutation_error)?;
    Ok(summary.into())
}

#[cfg(test)]
async fn close_session_with_usecase(
    lifecycle: &dyn StoredSessionClosePort,
    data_dir: &std::path::Path,
    session_id: &str,
) -> Result<CloseSessionOutcome, String> {
    lifecycle.close_session(data_dir, session_id).await
}

#[tauri::command]
pub async fn restore_session(
    local_store: State<'_, Arc<crate::adaptor::gateway::local_event_store::LocalEventStore>>,
    lifecycle: State<'_, Arc<StoredSessionLifecycleUsecase>>,
    registry: State<'_, Arc<AgentBackendRegistry>>,
    app: tauri::AppHandle,
    session_id: String,
) -> Result<RestoreSessionResponse, String> {
    super::session::ensure_mutation_admission_message(local_store.inner().as_ref()).await?;
    let data_dir = resolve_data_dir(&app)?;
    let outcome = lifecycle
        .restore_session(&data_dir, &session_id, registry.inner().as_ref())
        .await
        .map_err(super::session::normalize_mutation_error)?;
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
}
