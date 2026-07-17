use std::sync::Arc;

use crate::infrastructure::platform::app_data_dir::resolve_data_dir;
use crate::other::error::AppError;
use crate::usecase::agent_session::session::errors::session_target_rejected;
use crate::usecase::agent_session::session::{
    AgentTaskListReport, AgentThreadSearchMatch, PageCursor, SessionPage, SessionSearchResult,
    SessionStore, DEFAULT_SESSION_PAGE_LIMIT,
};

fn reject_explicit_start_for_workflow_node_session(
    session: &crate::usecase::agent_session::session::ChatSession,
    cwd: &str,
) -> Result<(), String> {
    if session.worktree_path != cwd || session.is_workflow_node_session() {
        return Err(session_target_rejected());
    }
    Ok(())
}

/// Tauri invoke 境界で permission_mode を検証し、検証済み抽象モードを返す。
/// 欠落（None）は空文字相当として扱い、対象外値とともに [`crate::domain::agent_session::InvalidPermissionMode`]
/// で拒否する。command 経路と単体テスト経路の両方で同じ拒否ロジックを共有する（Spec issues-947）。
fn validate_invoke_permission_mode(
    permission_mode: Option<String>,
) -> Result<crate::domain::agent_session::PermissionMode, String> {
    let permission_value = permission_mode.unwrap_or_default();
    crate::domain::agent_session::PermissionMode::parse(&permission_value)
        .map_err(|e| e.to_string())
}

fn should_skip_close_agent_session(
    session: Option<&crate::usecase::agent_session::session::ChatSession>,
) -> bool {
    session.is_some_and(|session| session.is_workflow_node_session())
}

#[tauri::command]
pub async fn set_session_backend(
    runtime: tauri::State<
        '_,
        Arc<crate::usecase::agent_session::runtime::AgentSessionRuntimeUsecase>,
    >,
    chat_session_id: String,
    backend_id: String,
) -> Result<crate::usecase::agent_session::session::GetSessionResponse, String> {
    runtime
        .set_session_backend(&chat_session_id, &backend_id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn get_session(
    runtime: tauri::State<
        '_,
        Arc<crate::usecase::agent_session::runtime::AgentSessionRuntimeUsecase>,
    >,
    session_id: String,
) -> Result<Option<crate::usecase::agent_session::session::GetSessionResponse>, String> {
    runtime
        .get_session(&session_id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn get_session_page(
    state: tauri::State<'_, Arc<SessionStore>>,
    app: tauri::AppHandle,
    session_id: String,
    cursor: Option<String>,
    limit: Option<usize>,
) -> Result<Option<SessionPage>, String> {
    let data_dir = resolve_data_dir(&app)?;
    let cursor = cursor
        .as_deref()
        .map(|value| {
            value
                .parse::<u64>()
                .map(PageCursor)
                .map_err(|_| "Invalid session page cursor".to_string())
        })
        .transpose()?;
    state.get_session_page(
        &data_dir,
        &session_id,
        cursor,
        limit.unwrap_or(DEFAULT_SESSION_PAGE_LIMIT),
    )
}

#[tauri::command]
pub fn plan_agent_chat_eviction(
    request: crate::usecase::agent_session::session::AgentChatEvictionPlanRequest,
) -> Result<crate::usecase::agent_session::session::AgentChatEvictionPlan, String> {
    Ok(crate::usecase::agent_session::session::plan_agent_chat_eviction(request))
}

#[tauri::command]
pub async fn get_session_attachment(
    state: tauri::State<'_, Arc<SessionStore>>,
    app: tauri::AppHandle,
    session_id: String,
    attachment_id: String,
) -> Result<Option<crate::usecase::agent_session::session::ImageAttachment>, String> {
    let data_dir = resolve_data_dir(&app)?;
    Ok(state
        .get_session_attachment(&data_dir, &session_id, &attachment_id)?
        .map(
            |attachment| crate::usecase::agent_session::session::ImageAttachment {
                data: attachment.data,
                media_type: attachment.media_type,
            },
        ))
}

#[tauri::command]
pub async fn get_session_tool_output(
    state: tauri::State<'_, Arc<SessionStore>>,
    app: tauri::AppHandle,
    session_id: String,
    tool_output_id: String,
) -> Result<Option<crate::usecase::agent_session::session::SessionToolOutput>, String> {
    let data_dir = resolve_data_dir(&app)?;
    state.get_session_tool_output(&data_dir, &session_id, &tool_output_id)
}

#[tauri::command]
pub async fn search_agent_sessions(
    app: tauri::AppHandle,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    worktree_path: String,
    query: String,
    include_workflow: Option<bool>,
    limit: Option<usize>,
) -> Result<Vec<SessionSearchResult>, String> {
    let data_dir = resolve_data_dir(&app)?;
    crate::usecase::agent_session::session::search_agent_sessions(
        session_store.inner(),
        &data_dir,
        &worktree_path,
        &query,
        include_workflow.unwrap_or(false),
        limit.unwrap_or(20),
    )
}

#[tauri::command]
pub async fn search_agent_session_messages(
    app: tauri::AppHandle,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    session_id: String,
    query: String,
) -> Result<Vec<AgentThreadSearchMatch>, String> {
    let data_dir = resolve_data_dir(&app)?;
    crate::usecase::agent_session::session::search_agent_session_messages(
        session_store.inner(),
        &data_dir,
        &session_id,
        &query,
    )
}

#[tauri::command]
pub async fn interrupt_agent_query(
    runtime: tauri::State<
        '_,
        Arc<crate::usecase::agent_session::runtime::AgentSessionRuntimeUsecase>,
    >,
    chat_session_id: String,
) -> Result<(), String> {
    runtime
        .interrupt(&chat_session_id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn resume_agent_queue(
    runtime: tauri::State<
        '_,
        Arc<crate::usecase::agent_session::runtime::AgentSessionRuntimeUsecase>,
    >,
    chat_session_id: String,
) -> Result<(), AppError> {
    runtime
        .resume_queue(&chat_session_id)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
pub async fn cancel_agent_queued_turn(
    runtime: tauri::State<
        '_,
        Arc<crate::usecase::agent_session::runtime::AgentSessionRuntimeUsecase>,
    >,
    chat_session_id: String,
    queued_turn_id: Option<String>,
) -> Result<crate::usecase::agent_session::runtime::usecase::CancelQueuedTurnResponse, String> {
    runtime
        .cancel_queued_turn(&chat_session_id, queued_turn_id.as_deref())
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn build_agent_task_list_report(
    runtime: tauri::State<
        '_,
        Arc<crate::usecase::agent_session::runtime::AgentSessionRuntimeUsecase>,
    >,
    chat_session_id: String,
) -> Result<AgentTaskListReport, String> {
    runtime
        .build_agent_task_list_report(&chat_session_id)
        .await
        .map_err(|error| error.to_string())
}
#[tauri::command]
pub async fn init_agent_sessions(
    runtime: tauri::State<
        '_,
        Arc<crate::usecase::agent_session::runtime::AgentSessionRuntimeUsecase>,
    >,
    open_tabs: tauri::State<'_, Arc<crate::usecase::agent_session::session::OpenTabRegistry>>,
    worktree_path: String,
) -> Result<crate::usecase::agent_session::runtime::usecase::InitSessionsResponse, String> {
    runtime
        .init_sessions(&worktree_path, open_tabs.inner())
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn start_agent_session(
    app: tauri::AppHandle,
    runtime: tauri::State<
        '_,
        Arc<crate::usecase::agent_session::runtime::AgentSessionRuntimeUsecase>,
    >,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    chat_session_id: String,
    cwd: String,
    permission_mode: Option<String>,
    plan_mode: Option<bool>,
) -> Result<(), String> {
    // 外部境界（Tauri invoke）では permission_mode 欠落・対象外値を InvalidPermissionMode で拒否する。
    // None は空文字相当として扱い、内部経路の保存値フォールバックには進めない。
    let validated_permission_mode = validate_invoke_permission_mode(permission_mode)?;

    let data_dir = resolve_data_dir(&app)?;
    let session = session_store
        .get_session_shell(&data_dir, &chat_session_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Session not found: {chat_session_id}"))?;
    reject_explicit_start_for_workflow_node_session(&session, &cwd)?;
    let validated_plan_mode = plan_mode.unwrap_or(false);
    runtime
        .start_session(
            &chat_session_id,
            crate::usecase::agent_session::runtime::usecase::StartSessionOptions {
                permission_mode: validated_permission_mode,
                plan_mode: validated_plan_mode,
            },
        )
        .await
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resume_agent_queue_errors_preserve_typed_variants_and_string_wire_format() {
        let startup =
            crate::usecase::agent_session::runtime::usecase::AgentRuntimeError::StartupTimeout {
                retry_count: 1,
                max_retries: 2,
            };
        let startup_message = startup.to_string();
        let startup = AppError::from(startup);
        assert!(matches!(
            startup,
            AppError::AgentStartupTimeout {
                retry_count: 1,
                max_retries: 2
            }
        ));
        assert_eq!(serde_json::to_value(&startup).unwrap(), startup_message);

        let other = AppError::from(
            crate::usecase::agent_session::runtime::usecase::AgentRuntimeError::Other(
                "resume failed".to_string(),
            ),
        );
        assert!(matches!(other, AppError::Internal(ref message) if message == "resume failed"));
        assert_eq!(serde_json::to_value(&other).unwrap(), "resume failed");
    }

    fn session_for_start_guard(
        workflow_node_session: bool,
    ) -> crate::usecase::agent_session::session::ChatSession {
        crate::usecase::agent_session::session::ChatSession {
            id: uuid::Uuid::new_v4().to_string(),
            worktree_path: "/repo".to_string(),
            messages: Vec::new(),
            state: crate::usecase::agent_session::session::SessionState::Idle,
            error_reason: None,
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
            workflow_node_session,
            workflow_node_context: None,
            context_epoch: None,
        }
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

    // Tauri invoke 境界が拒否したとき、保存値が変更されないことを
    // 上位の command 経路を模した手順で確認する。
    #[test]
    fn start_agent_session_invalid_permission_mode_does_not_mutate_persisted_state() {
        let data_dir = tempfile::tempdir().unwrap();
        let store = Arc::new(crate::test_support::build_session_store());
        let session = crate::usecase::agent_session::session::ChatSession {
            id: uuid::Uuid::new_v4().to_string(),
            worktree_path: "/repo".to_string(),
            messages: Vec::new(),
            state: crate::usecase::agent_session::session::SessionState::Idle,
            error_reason: None,
            created_at: 1.0,
            updated_at: 1.0,
            agent_session_id: None,
            context_carry: None,
            permission_mode: "edit".to_string(),
            plan_mode: false,
            permission_profile_id: None,
            selected_model: None,
            backend_id: Some(
                crate::infrastructure::agent_session::claude::CLAUDE_BACKEND_ID.to_string(),
            ),
            workflow_node_session: false,
            workflow_node_context: None,
            context_epoch: None,
        };
        store
            .save_full_session_for_migration_or_restore(data_dir.path(), &session)
            .unwrap();

        for invalid in [None, Some(String::new()), Some("acceptEdits".to_string())] {
            let result = validate_invoke_permission_mode(invalid.clone());
            assert!(result.is_err(), "{invalid:?} must be rejected");
            // command 本体は ? で早期 return するため、保存値は不変。
            let saved = store
                .load_full_session_for_restore(data_dir.path(), &session.id)
                .unwrap()
                .unwrap();
            assert_eq!(saved.permission_mode, "edit");
        }
    }

    #[test]
    fn close_agent_session_guard_keeps_workflow_node_runtime() {
        let session = session_for_start_guard(true);

        assert!(should_skip_close_agent_session(Some(&session)));
    }
}

#[tauri::command]
pub async fn close_agent_session(
    app: tauri::AppHandle,
    runtime: tauri::State<
        '_,
        Arc<crate::usecase::agent_session::runtime::AgentSessionRuntimeUsecase>,
    >,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    chat_session_id: String,
) -> Result<(), String> {
    let data_dir = resolve_data_dir(&app)?;
    let session = session_store
        .get_session_shell(&data_dir, &chat_session_id)
        .map_err(|e| e.to_string())?;
    if should_skip_close_agent_session(session.as_ref()) {
        return Ok(());
    }
    runtime
        .close_session(&chat_session_id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn send_agent_message(
    app: tauri::AppHandle,
    runtime: tauri::State<
        '_,
        Arc<crate::usecase::agent_session::runtime::AgentSessionRuntimeUsecase>,
    >,
    chat_session_id: Option<String>,
    worktree_path: String,
    content: String,
    permission_mode: Option<String>,
    plan_mode: Option<bool>,
    backend_id: Option<String>,
    model_id: Option<String>,
    images: Option<Vec<crate::usecase::agent_session::session::ImageAttachment>>,
    mentions: Option<Vec<crate::adaptor::protocol::mention::MentionReferenceInput>>,
    editor_context: Option<crate::usecase::agent_session::runtime::usecase::AgentEditorContext>,
) -> Result<crate::usecase::agent_session::runtime::usecase::SendMessageResponse, String> {
    let permission_mode = validate_invoke_permission_mode(permission_mode)?;
    let mentions = mentions.map(crate::adaptor::protocol::mention::into_domain_vec);
    let response = runtime
        .send_message(
            crate::usecase::agent_session::runtime::usecase::SendAgentMessageRequest {
                chat_session_id,
                worktree_path,
                content,
                permission_mode,
                plan_mode: plan_mode.unwrap_or(false),
                backend_id,
                model_id,
                images,
                mentions,
                editor_context,
            },
        )
        .await
        .map_err(|error| error.to_string())?;
    crate::adaptor::controller_support::emit_after_workflow_node_message(&app, &response.session)
        .await;
    Ok(response)
}
