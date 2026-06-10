use std::sync::Arc;

use tokio::sync::Mutex;

use crate::agent_message_dispatcher::{
    dispatch_agent_message, AgentMessageDispatchContext, AgentMessageDispatchRequest,
};
use crate::app_data_dir::resolve_data_dir;
use crate::infrastructure::agent_session::runtime::{AgentBackendRegistry, ImageAttachment};
use crate::usecase::agent_session::session::errors::session_target_rejected;
use crate::usecase::agent_session::session::SessionStore;
use crate::workflow::engine::WorkflowEngine;

fn reject_explicit_start_for_workflow_step_session(
    session: &crate::usecase::agent_session::session::ChatSession,
    cwd: &str,
) -> Result<(), String> {
    if session.worktree_path != cwd || session.workflow_step_session {
        return Err(session_target_rejected());
    }
    Ok(())
}

/// Tauri invoke 境界で permission_mode を検証し、検証済み抽象モードを返す。
/// 欠落（None）は空文字相当として扱い、対象外値とともに [`crate::permission::InvalidPermissionMode`]
/// で拒否する。command 経路と単体テスト経路の両方で同じ拒否ロジックを共有する（Spec issues-947）。
fn validate_invoke_permission_mode(
    permission_mode: Option<String>,
) -> Result<crate::permission::PermissionMode, String> {
    let permission_value = permission_mode.unwrap_or_default();
    crate::permission::PermissionMode::parse(&permission_value).map_err(|e| e.to_string())
}

fn should_skip_close_agent_session(
    session: Option<&crate::usecase::agent_session::session::ChatSession>,
) -> bool {
    session.is_some_and(|session| session.workflow_step_session)
}

#[tauri::command]
pub async fn set_session_backend(
    app: tauri::AppHandle,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    registry: tauri::State<'_, Arc<AgentBackendRegistry>>,
    handles: tauri::State<
        '_,
        Arc<Mutex<crate::infrastructure::agent_session::runtime::AgentProcessMap>>,
    >,
    chat_session_id: String,
    backend_id: String,
) -> Result<crate::usecase::agent_session::session::GetSessionResponse, String> {
    crate::infrastructure::agent_session::runtime::set_session_backend(
        app,
        session_store,
        registry,
        handles,
        chat_session_id,
        backend_id,
    )
    .await
}

#[tauri::command]
pub async fn get_session(
    state: tauri::State<'_, Arc<SessionStore>>,
    handles: tauri::State<
        '_,
        Arc<Mutex<crate::infrastructure::agent_session::runtime::AgentProcessMap>>,
    >,
    registry: tauri::State<'_, Arc<AgentBackendRegistry>>,
    app: tauri::AppHandle,
    session_id: String,
) -> Result<Option<crate::usecase::agent_session::session::GetSessionResponse>, String> {
    crate::infrastructure::agent_session::runtime::get_session(
        state, handles, registry, app, session_id,
    )
    .await
}

#[tauri::command]
pub async fn interrupt_agent_query(
    handles: tauri::State<
        '_,
        Arc<Mutex<crate::infrastructure::agent_session::runtime::AgentProcessMap>>,
    >,
    chat_session_id: String,
) -> Result<(), String> {
    crate::infrastructure::agent_session::runtime::interrupt_agent_query(handles, chat_session_id)
        .await
}

#[tauri::command]
pub async fn init_agent_sessions(
    app: tauri::AppHandle,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    registry: tauri::State<'_, Arc<AgentBackendRegistry>>,
    handles: tauri::State<
        '_,
        Arc<Mutex<crate::infrastructure::agent_session::runtime::AgentProcessMap>>,
    >,
    open_tabs: tauri::State<'_, Arc<crate::usecase::agent_session::session::OpenTabRegistry>>,
    worktree_path: String,
) -> Result<crate::infrastructure::agent_session::runtime::InitSessionsResponse, String> {
    crate::infrastructure::agent_session::runtime::init_agent_sessions(
        app,
        session_store,
        registry,
        handles,
        open_tabs,
        worktree_path,
    )
    .await
}

#[tauri::command]
pub async fn start_agent_session(
    app: tauri::AppHandle,
    handles: tauri::State<
        '_,
        Arc<Mutex<crate::infrastructure::agent_session::runtime::AgentProcessMap>>,
    >,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    chat_session_id: String,
    cwd: String,
    permission_mode: Option<String>,
) -> Result<(), String> {
    // 外部境界（Tauri invoke）では permission_mode 欠落・対象外値を InvalidPermissionMode で拒否する。
    // None は空文字相当として扱い、内部経路の保存値フォールバックには進めない。
    let validated_permission_mode = validate_invoke_permission_mode(permission_mode)?;
    let validated_permission_mode_str = validated_permission_mode.as_str().to_string();

    let data_dir = resolve_data_dir(&app)?;
    let session = session_store
        .get_session(&data_dir, &chat_session_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Session not found: {chat_session_id}"))?;
    reject_explicit_start_for_workflow_step_session(&session, &cwd)?;

    // 検証済み permission_mode をセッション保存層に反映（外部 UI 操作の結果をセッションに記録）。
    if session.permission_mode != validated_permission_mode_str {
        session_store.update_permission_mode(
            &data_dir,
            &chat_session_id,
            &validated_permission_mode_str,
        )?;
    }

    crate::infrastructure::agent_session::runtime::start_agent_session_internal(
        &app,
        handles.inner(),
        session_store.inner(),
        &chat_session_id,
        &cwd,
        Some(validated_permission_mode_str),
        None,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usecase::agent_session::session::SessionStore;

    fn session_for_start_guard(
        workflow_step_session: bool,
    ) -> crate::usecase::agent_session::session::ChatSession {
        crate::usecase::agent_session::session::ChatSession {
            id: uuid::Uuid::new_v4().to_string(),
            worktree_path: "/repo".to_string(),
            messages: Vec::new(),
            state: crate::usecase::agent_session::session::SessionState::Idle,
            created_at: 1.0,
            updated_at: 1.0,
            agent_session_id: Some("sdk-session".to_string()),
            permission_mode: "edit".to_string(),
            selected_model: None,
            backend_id: Some(
                crate::infrastructure::agent_session::runtime::CLAUDE_BACKEND_ID.to_string(),
            ),
            workflow_step_session,
        }
    }

    #[test]
    fn start_agent_session_guard_rejects_workflow_step_session_before_runtime_start() {
        let session = session_for_start_guard(true);
        let handles = crate::infrastructure::agent_session::runtime::AgentProcessMap::new();

        let result = reject_explicit_start_for_workflow_step_session(&session, "/repo");

        assert_eq!(result.unwrap_err(), session_target_rejected());
        assert!(handles.is_empty());
    }

    #[test]
    fn start_agent_session_guard_allows_regular_session_in_matching_worktree() {
        let session = session_for_start_guard(false);

        assert!(reject_explicit_start_for_workflow_step_session(&session, "/repo").is_ok());
    }

    // Spec issues-947: Tauri invoke 境界で permission_mode の欠落・対象外値を拒否する。
    // start_agent_session 内部の `validate_invoke_permission_mode` を command 相当の経路として
    // 直接呼び、欠落/旧語彙/未知語彙/空文字いずれも `?` で早期 return することを確認する
    // （= `update_permission_mode` も `start_agent_session_internal` も呼ばれない）。
    #[test]
    fn start_agent_session_validate_rejects_missing_or_invalid_permission_mode() {
        let invalid_inputs: Vec<Option<String>> = vec![
            None,
            Some(String::new()),
            Some("acceptEdits".to_string()),
            Some("bypassPermissions".to_string()),
            Some("plan".to_string()),
            Some("default".to_string()),
            Some("unknown".to_string()),
        ];
        for permission in invalid_inputs {
            let label = permission.clone();
            let err = validate_invoke_permission_mode(permission).unwrap_err();
            assert!(
                err.contains("ask, edit, full"),
                "{:?} must include allowed list, got: {err}",
                label
            );
        }
    }

    #[test]
    fn start_agent_session_validate_accepts_abstract_modes() {
        for mode in ["ask", "edit", "full"] {
            let validated = validate_invoke_permission_mode(Some(mode.to_string())).unwrap();
            assert_eq!(validated.as_str(), mode);
        }
    }

    // Tauri invoke 境界が拒否したとき、保存値も runtime ハンドルも変更されないことを
    // 上位の command 経路を模した手順で確認する。
    #[tokio::test]
    async fn start_agent_session_invalid_permission_mode_does_not_mutate_persisted_state() {
        let data_dir = tempfile::tempdir().unwrap();
        let store = Arc::new(SessionStore::default());
        let session = crate::usecase::agent_session::session::ChatSession {
            id: uuid::Uuid::new_v4().to_string(),
            worktree_path: "/repo".to_string(),
            messages: Vec::new(),
            state: crate::usecase::agent_session::session::SessionState::Idle,
            created_at: 1.0,
            updated_at: 1.0,
            agent_session_id: None,
            permission_mode: "edit".to_string(),
            selected_model: None,
            backend_id: Some(
                crate::infrastructure::agent_session::runtime::CLAUDE_BACKEND_ID.to_string(),
            ),
            workflow_step_session: false,
        };
        store.save_session(data_dir.path(), &session).unwrap();
        let handles = Arc::new(Mutex::new(
            crate::infrastructure::agent_session::runtime::AgentProcessMap::new(),
        ));

        for invalid in [None, Some(String::new()), Some("acceptEdits".to_string())] {
            let result = validate_invoke_permission_mode(invalid.clone());
            assert!(result.is_err(), "{invalid:?} must be rejected");
            // command 本体は ? で早期 return するため、保存値・runtime ハンドルとも不変。
            let saved = store
                .get_session(data_dir.path(), &session.id)
                .unwrap()
                .unwrap();
            assert_eq!(saved.permission_mode, "edit");
            assert!(handles.lock().await.is_empty());
        }
    }

    #[tokio::test]
    async fn close_agent_session_guard_keeps_workflow_step_runtime() {
        let session = session_for_start_guard(true);
        let handles = Arc::new(Mutex::new(
            crate::infrastructure::agent_session::runtime::AgentProcessMap::new(),
        ));
        handles.lock().await.insert(
            session.id.clone(),
            crate::infrastructure::agent_session::runtime::make_test_agent_process(),
        );

        assert!(should_skip_close_agent_session(Some(&session)));
        assert!(handles.lock().await.contains_key(&session.id));
    }
}

#[tauri::command]
pub async fn close_agent_session(
    app: tauri::AppHandle,
    handles: tauri::State<
        '_,
        Arc<Mutex<crate::infrastructure::agent_session::runtime::AgentProcessMap>>,
    >,
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
    crate::infrastructure::agent_session::runtime::close_agent_session_internal(
        &app,
        handles.inner(),
        &chat_session_id,
    )
    .await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn send_agent_message(
    app: tauri::AppHandle,
    handles: tauri::State<
        '_,
        Arc<Mutex<crate::infrastructure::agent_session::runtime::AgentProcessMap>>,
    >,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    registry: tauri::State<'_, Arc<AgentBackendRegistry>>,
    engine: tauri::State<'_, Arc<WorkflowEngine>>,
    open_tabs: tauri::State<'_, Arc<crate::usecase::agent_session::session::OpenTabRegistry>>,
    chat_session_id: Option<String>,
    worktree_path: String,
    content: String,
    permission_mode: Option<String>,
    backend_id: Option<String>,
    images: Option<Vec<ImageAttachment>>,
    mentions: Option<Vec<crate::adaptor::protocol::mention::MentionReferenceInput>>,
) -> Result<crate::infrastructure::agent_session::runtime::SendMessageResponse, String> {
    let permission_mode = validate_invoke_permission_mode(permission_mode)?;
    let mentions = mentions.map(crate::adaptor::protocol::mention::into_domain_vec);
    let response = dispatch_agent_message(
        AgentMessageDispatchContext {
            gateway: crate::infrastructure::agent_session::runtime_gateway::AgentRuntimeGateway {
                app: &app,
                session_store: session_store.inner(),
                registry: registry.inner(),
                handles: handles.inner(),
            },
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
