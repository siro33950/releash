use std::sync::Arc;

use tauri::State;

use crate::adaptor::controller_support::WorkflowStepLifecycleUsecaseState;
use crate::infrastructure::platform::app_data_dir::resolve_data_dir;
use crate::usecase::agent_session::backend_registry::AgentBackendRegistry;
use crate::usecase::agent_session::runtime::AgentSessionRuntimeUsecase;
use crate::usecase::agent_session::session::{
    add_message_internal, ChatMessage, ChatSession, MessageRole, OpenTabRegistry,
    RestoreSessionResponse, SessionStore, SessionSummary, StoredSessionLifecycleUsecase,
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
    state: State<'_, Arc<SessionStore>>,
    registry: State<'_, Arc<AgentBackendRegistry>>,
    app: tauri::AppHandle,
    worktree_path: String,
    permission_mode: String,
    backend_id: Option<String>,
    model_id: Option<String>,
) -> Result<ChatSession, String> {
    let data_dir = resolve_data_dir(&app)?;
    let permission_mode = crate::domain::agent_session::PermissionMode::parse(&permission_mode)
        .map_err(|e| e.to_string())?;
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
    state: State<'_, Arc<SessionStore>>,
    runtime: State<'_, Arc<AgentSessionRuntimeUsecase>>,
    open_tabs: State<'_, Arc<OpenTabRegistry>>,
    step_lifecycle: State<'_, WorkflowStepLifecycleUsecaseState>,
    app: tauri::AppHandle,
    session_id: String,
) -> Result<(), String> {
    if let Some(target) = step_lifecycle
        .close_tab_target(&session_id)
        .await
        .map_err(|_| {
            crate::adaptor::controller::command::workflow::session_errors::workflow_step_tab_operation_failed()
        })?
    {
        crate::adaptor::controller_support::emit_workflow_step_target_state(
            &app,
            &target,
            runtime.inner(),
            open_tabs.inner(),
        )
        .await;
        return Ok(());
    }

    runtime
        .close_session(&session_id)
        .await
        .map_err(|error| error.to_string())?;
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
    lifecycle: State<'_, Arc<StoredSessionLifecycleUsecase>>,
    registry: State<'_, Arc<AgentBackendRegistry>>,
    runtime: State<'_, Arc<AgentSessionRuntimeUsecase>>,
    open_tabs: State<'_, Arc<OpenTabRegistry>>,
    step_lifecycle: State<'_, WorkflowStepLifecycleUsecaseState>,
    app: tauri::AppHandle,
    session_id: String,
) -> Result<RestoreSessionResponse, String> {
    if let Some(target) = step_lifecycle
        .try_open_tab(&session_id)
        .await
        .map_err(|_| {
            crate::adaptor::controller::command::workflow::session_errors::workflow_step_tab_operation_failed()
        })?
    {
        crate::adaptor::controller_support::emit_workflow_step_target_state(
            &app,
            &target,
            runtime.inner(),
            open_tabs.inner(),
        )
        .await;
        return Ok(RestoreSessionResponse {
            restored_workflow_step: true,
        });
    }

    let data_dir = resolve_data_dir(&app)?;
    lifecycle
        .restore_session(&data_dir, &session_id, registry.inner().as_ref())
        .await
}

#[cfg(test)]
fn restore_workflow_step_session_tab_state(
    session_store: &SessionStore,
    data_dir: &std::path::Path,
    open_tabs: &OpenTabRegistry,
    session_id: &str,
) -> Result<Option<(RestoreSessionResponse, String)>, String> {
    let Some(target) = crate::adaptor::gateway::workflow::resolve_step_session_with_data_dir(
        session_store,
        data_dir,
        session_id,
    )
    .map_err(|_| {
        crate::adaptor::controller::command::workflow::session_errors::workflow_step_tab_operation_failed()
    })?
    else {
        return Ok(None);
    };
    crate::adaptor::gateway::workflow::open_step_session_tab_state(
        session_store,
        data_dir,
        open_tabs,
        &target.session_id,
    )
    .map_err(|_| {
        crate::adaptor::controller::command::workflow::session_errors::workflow_step_tab_operation_failed()
    })?;
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
            context_carry: Some(crate::usecase::agent_session::session::ContextCarryState::Resumed),
            permission_mode: "edit".to_string(),
            plan_mode: false,
            permission_profile_id: None,
            selected_model: None,
            backend_id: Some(
                crate::infrastructure::agent_session::claude::CLAUDE_BACKEND_ID.to_string(),
            ),
            workflow_step_session: true,
            workflow_step_context: None,
            context_epoch: None,
        }
    }

    #[tokio::test]
    async fn restore_workflow_step_session_tab_reopens_history_without_starting_runtime() {
        let tmp = tempfile::TempDir::new().unwrap();
        let session_store = crate::test_support::build_session_store();
        let open_tabs = OpenTabRegistry::default();
        let session_id = uuid::Uuid::new_v4().to_string();

        session_store
            .save_full_session_for_migration_or_restore(
                tmp.path(),
                &workflow_step_session_for_test(&session_id),
            )
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
        let session = session_store
            .load_full_session_for_restore(tmp.path(), &session_id)
            .unwrap()
            .expect("session remains as history");
        assert_eq!(session.state, SessionState::Idle);
    }

    #[tokio::test]
    async fn close_session_state_after_runtime_marks_session_closed_on_success() {
        let tmp = tempfile::TempDir::new().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
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
            .load_full_session_for_restore(tmp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(loaded.state, SessionState::Closed);
    }

    #[tokio::test]
    async fn close_session_state_after_runtime_keeps_state_on_runtime_failure() {
        let tmp = tempfile::TempDir::new().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
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
            .load_full_session_for_restore(tmp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(err, "runtime close failed");
        assert_eq!(loaded.state, session.state);
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
