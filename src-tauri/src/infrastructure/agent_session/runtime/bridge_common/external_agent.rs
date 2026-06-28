use super::process_registry::{
    AgentProcess, AgentProcessMap, BridgeState, PendingMessage, TurnPhase,
};
use super::recovery::{remove_pgid, save_pgid, spawn_turn_watchdog};
use super::session_lifecycle::{
    build_and_persist_session_system_prompt, prepare_pending_turn_messages,
    prompt_input_for_started_turn, start_pending_message_turn, sweep_process_group,
    take_pending_message,
};
use super::shared::{
    notify_status_transition, resolve_mentions_or_fallback_from_port, CODEX_BACKEND_ID,
    GENERATION_COUNTER,
};
use super::stream_emit::{emit_session_state_changed, spawn_streaming_timer};
use super::turn_event_log::{begin_turn_event_log, projected_session_state_for_current_turn};
use crate::infrastructure::agent_session::runtime::runtime_coordinator::clear_pending_turn_starting;
use crate::infrastructure::agent_session::runtime::runtime_coordinator::clear_session_closing;
use crate::infrastructure::agent_session::runtime::runtime_coordinator::mark_session_closing;
use crate::infrastructure::agent_session::runtime::runtime_coordinator::prune_session_runtime_lock;
use crate::infrastructure::agent_session::runtime::AgentEditorContext;
use crate::infrastructure::agent_session::runtime::ImageAttachment;
use crate::infrastructure::platform::app_data_dir::resolve_data_dir;
use crate::usecase::agent_session::context::BranchDiffContextPort;
use crate::usecase::agent_session::event_log::TurnEventLog;
use crate::usecase::agent_session::event_log::WorkflowTurnCompleteInput;
use crate::usecase::agent_session::session::now_timestamp;
use crate::usecase::agent_session::session::ContextCarryState;
#[cfg(test)]
use crate::usecase::agent_session::session::MessagePart;
use crate::usecase::agent_session::session::SessionStore;
use std::collections::HashMap;
use std::collections::VecDeque;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;
use tauri::Manager;
use tokio::process::Child;
use tokio::process::ChildStdin;
use tokio::sync::Mutex;

#[allow(dead_code)]
pub(crate) struct ExternalBridgeMessageState {
    pub(crate) last_persist_time: Instant,
    pub(crate) branch_diff_context: Option<Arc<dyn BranchDiffContextPort>>,
}

impl Default for ExternalBridgeMessageState {
    fn default() -> Self {
        Self::new(None)
    }
}

impl ExternalBridgeMessageState {
    pub(crate) fn new(branch_diff_context: Option<Arc<dyn BranchDiffContextPort>>) -> Self {
        Self {
            last_persist_time: Instant::now(),
            branch_diff_context,
        }
    }
}

pub(crate) struct ExternalAgentTurnStart<'a> {
    pub permission_mode: &'a str,
    pub streaming_message_id: &'a str,
    pub prompt: &'a str,
    pub images: &'a [ImageAttachment],
}
pub(crate) async fn start_external_agent_turn_state<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    branch_diff_context: Option<Arc<dyn BranchDiffContextPort>>,
    session_store: &Arc<SessionStore>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    chat_session_id: &str,
    turn: ExternalAgentTurnStart<'_>,
) -> Result<(), String> {
    let canonical_permission_mode =
        crate::domain::agent_session::PermissionMode::parse(turn.permission_mode)
            .map_err(|e| e.to_string())?;
    let started_turn_prompt = prompt_input_for_started_turn(
        Some(app),
        Some(session_store),
        chat_session_id,
        turn.streaming_message_id,
        turn.prompt,
        turn.images,
    );
    let projected_session_state = {
        let mut map = handles.lock().await;
        let proc = map
            .get_mut(chat_session_id)
            .ok_or_else(|| format!("No agent process for session {chat_session_id}"))?;
        proc.current_permission_mode = canonical_permission_mode.as_str().to_string();
        proc.state = BridgeState::Streaming;
        proc.turn_phase = TurnPhase::Streaming;
        proc.streaming_message_id = Some(turn.streaming_message_id.to_string());
        proc.active_turn_token = Some(turn.streaming_message_id.to_string());
        proc.reset_streaming_state_for_new_turn();
        proc.begin_turn_liveness();
        begin_turn_event_log(
            proc,
            &started_turn_prompt.message_id,
            started_turn_prompt.prompt,
            turn.streaming_message_id,
            now_timestamp(),
        );
        spawn_streaming_timer(app, handles, chat_session_id, proc);
        spawn_turn_watchdog(
            app,
            branch_diff_context,
            handles,
            session_store,
            chat_session_id,
            proc,
        );
        projected_session_state_for_current_turn(proc)
    };
    emit_session_state_changed(
        app,
        chat_session_id,
        TurnPhase::Streaming,
        None,
        false,
        None,
    );
    notify_status_transition(
        app,
        session_store,
        chat_session_id,
        TurnPhase::Streaming,
        projected_session_state,
    );
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn register_external_agent_process<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    session_store: &Arc<SessionStore>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    chat_session_id: &str,
    backend_id: String,
    child: Child,
    stdin: ChildStdin,
    #[cfg(unix)] pgid: Option<u32>,
    permission_mode: String,
    selected_model: Option<String>,
    stale_timeout: std::time::Duration,
    sdk_session_id: Option<String>,
    context_carry_on_ready: Option<ContextCarryState>,
) -> Result<u64, String> {
    if let Err(err) = crate::domain::agent_session::PermissionMode::parse(&permission_mode) {
        cleanup_unregistered_agent_process(
            child,
            #[cfg(unix)]
            pgid,
        )
        .await;
        return Err(err.to_string());
    }
    #[cfg(unix)]
    if let Some(pg) = pgid {
        let data_dir = match resolve_data_dir(app)
            .map_err(|e| format!("Failed to resolve data dir for session {chat_session_id}: {e}"))
        {
            Ok(data_dir) => data_dir,
            Err(err) => {
                cleanup_unregistered_agent_process(child, pgid).await;
                return Err(err);
            }
        };
        if let Err(err) = save_pgid(&data_dir, chat_session_id, pg) {
            cleanup_unregistered_agent_process(child, pgid).await;
            return Err(err);
        }
    }

    let gen_id = GENERATION_COUNTER.fetch_add(1, Ordering::SeqCst);
    let new_process = AgentProcess {
        stdin: Arc::new(Mutex::new(stdin)),
        backend_id,
        state: BridgeState::Initializing,
        turn_phase: TurnPhase::Idle,
        sdk_session_id,
        system_prompt_fingerprint: None,
        context_carry_on_ready,
        child,
        generation_id: gen_id,
        #[cfg(unix)]
        pgid,
        streaming_message_id: None,
        active_turn_token: None,
        turn_latency: None,
        post_turn_message_token: None,
        streaming_parts: Vec::new(),
        confirmed_stream_part_len: 0,
        turn_event_log: TurnEventLog::default(),
        last_message_id: None,
        post_turn_base_untrusted_message_id: None,
        task_id_map: HashMap::new(),
        pending_messages: VecDeque::new(),
        current_permission_mode: permission_mode,
        available_models: Vec::new(),
        selected_model,
        stale_timeout,
        last_result_token_usage: None,
        current_turn_stop_reason: None,
        latest_token_usage: None,
        pending_stream_parts: Vec::new(),
        pending_stream_part_rollbacks: Vec::new(),
        retry_stream_delta: None,
        pending_stream_bytes: 0,
        streaming_delta_seq: 0,
        streaming_delta_seq_by_message: HashMap::new(),
        pending_persisted_tool_output_resyncs: HashMap::new(),
        last_stream_emit_at: None,
        streaming_timer_active: false,
        last_progress_at: None,
        turn_phase_since: Instant::now(),
        turn_seq: 0,
        turn_watchdog_active: false,
    };
    let replaced = {
        let mut map = handles.lock().await;
        map.remove(chat_session_id)
    };
    if let Some(replaced) = replaced {
        cleanup_replaced_agent_process(chat_session_id, replaced).await;
    }
    handles
        .lock()
        .await
        .insert(chat_session_id.to_string(), new_process);
    notify_status_transition(app, session_store, chat_session_id, TurnPhase::Idle, None);
    Ok(gen_id)
}

async fn cleanup_replaced_agent_process(chat_session_id: &str, mut proc: AgentProcess) {
    #[cfg(unix)]
    if let Some(pg) = proc.pgid {
        sweep_process_group(pg).await;
    }
    if let Err(e) = proc.child.kill().await {
        log::warn!("Failed to kill replaced agent process {chat_session_id}: {e}");
    }
    let _ = tokio::time::timeout(std::time::Duration::from_secs(1), proc.child.wait()).await;
}

pub(super) async fn cleanup_unregistered_agent_process(
    mut child: Child,
    #[cfg(unix)] pgid: Option<u32>,
) {
    #[cfg(unix)]
    if let Some(pg) = pgid {
        sweep_process_group(pg).await;
    }
    if let Err(e) = child.kill().await {
        log::warn!("Failed to kill unregistered agent process: {e}");
    }
}

#[allow(dead_code)]
pub(crate) async fn close_external_agent_process<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    chat_session_id: &str,
) -> Result<(), String> {
    mark_session_closing(chat_session_id).await;
    let removed = {
        let mut map = handles.lock().await;
        map.remove(chat_session_id)
    };

    let Some(mut proc) = removed else {
        clear_session_closing(chat_session_id).await;
        return Ok(());
    };

    #[cfg(unix)]
    {
        if let Some(pg) = proc.pgid {
            sweep_process_group(pg).await;
        } else if let Err(e) = proc.child.kill().await {
            log::warn!("Failed to kill external agent process {chat_session_id}: {e}");
        }
        if let Ok(data_dir) = resolve_data_dir(app) {
            remove_pgid(&data_dir, chat_session_id);
        }
    }
    #[cfg(not(unix))]
    if let Err(e) = proc.child.kill().await {
        log::warn!("Failed to kill external agent process {chat_session_id}: {e}");
    }

    let _ = tokio::time::timeout(std::time::Duration::from_secs(1), proc.child.wait()).await;
    clear_session_closing(chat_session_id).await;
    prune_session_runtime_lock(chat_session_id).await;
    Ok(())
}

#[allow(dead_code)]
pub(crate) struct ExternalPendingTurn {
    pub queued_turn_id: String,
    pub worktree_path: String,
    pub permission_mode: String,
    pub plan_mode: bool,
    pub permission_profile_id: Option<String>,
    pub prompt: String,
    pub agent_message_id: String,
    pub images: Vec<ImageAttachment>,
    pub editor_context: Option<AgentEditorContext>,
    pub system_prompt: Option<String>,
}

#[allow(dead_code)]
pub(crate) async fn prepare_external_pending_message_turn<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    branch_diff_context: Option<Arc<dyn BranchDiffContextPort>>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    session_store: &Arc<SessionStore>,
    chat_session_id: &str,
) -> Result<Option<ExternalPendingTurn>, String> {
    let Some(pending) = take_pending_message(handles, chat_session_id).await else {
        return Ok(None);
    };

    let data_dir = match resolve_data_dir(app) {
        Ok(data_dir) => data_dir,
        Err(e) => {
            requeue_pending_message(handles, chat_session_id, pending).await;
            clear_pending_turn_starting(chat_session_id).await;
            return Err(format!("failed to resolve data dir: {e}"));
        }
    };
    let session = match session_store.get_session_shell(&data_dir, chat_session_id) {
        Ok(Some(session)) => session,
        Ok(None) => {
            requeue_pending_message(handles, chat_session_id, pending).await;
            clear_pending_turn_starting(chat_session_id).await;
            return Err(format!("Session not found: {chat_session_id}"));
        }
        Err(e) => {
            requeue_pending_message(handles, chat_session_id, pending).await;
            clear_pending_turn_starting(chat_session_id).await;
            return Err(e.to_string());
        }
    };
    let backend_id = session.backend_id.as_deref().unwrap_or(CODEX_BACKEND_ID);
    let system_prompt = match build_and_persist_session_system_prompt(
        branch_diff_context.as_deref(),
        session_store,
        &data_dir,
        &session,
        backend_id,
        session.selected_model.as_deref(),
        None,
        &pending.mentions,
        pending.editor_context.as_ref(),
    ) {
        Ok(system_prompt) => system_prompt,
        Err(e) => {
            requeue_pending_message(handles, chat_session_id, pending).await;
            clear_pending_turn_starting(chat_session_id).await;
            return Err(format!("failed to build pending system context: {e}"));
        }
    };
    let (human_msg, agent_msg, emit_consumed_messages) =
        match prepare_pending_turn_messages(session_store, &data_dir, chat_session_id, &pending) {
            Ok(messages) => messages,
            Err(e) => {
                requeue_pending_message(handles, chat_session_id, pending).await;
                clear_pending_turn_starting(chat_session_id).await;
                return Err(format!("failed to prepare pending messages: {e}"));
            }
        };
    let permission_profile_id = session.permission_profile_id.clone();

    if emit_consumed_messages {
        use tauri::Emitter;
        let _ = app.emit(
            "agent-pending-message-consumed",
            serde_json::json!({
                "chat_session_id": chat_session_id,
                "queued_turn_id": pending.id,
                "human_message": human_msg,
                "agent_message": agent_msg,
            }),
        );
    }

    let prompt = resolve_mentions_or_fallback_from_port(
        app,
        &pending.worktree_path,
        &pending.content,
        &pending.mentions,
    );
    Ok(Some(ExternalPendingTurn {
        queued_turn_id: pending.id,
        worktree_path: pending.worktree_path,
        permission_mode: pending.permission_mode,
        plan_mode: pending.plan_mode,
        permission_profile_id,
        prompt,
        agent_message_id: agent_msg.id,
        images: pending.images,
        editor_context: pending.editor_context,
        system_prompt,
    }))
}

async fn requeue_pending_message(
    handles: &Arc<Mutex<AgentProcessMap>>,
    chat_session_id: &str,
    pending: PendingMessage,
) {
    let mut map = handles.lock().await;
    if let Some(proc) = map.get_mut(chat_session_id) {
        proc.pending_messages.push_front(pending);
    }
}

#[allow(dead_code)]
pub(crate) async fn finish_external_pending_message_turn_start(chat_session_id: &str) {
    clear_pending_turn_starting(chat_session_id).await;
}

#[cfg(test)]
pub(super) fn workflow_final_text_parts(final_parts: &[MessagePart]) -> Vec<String> {
    final_parts
        .iter()
        .filter_map(|part| match part {
            MessagePart::Text { content, .. } => Some(content.clone()),
            _ => None,
        })
        .collect()
}

/// turn_complete 後の Workflow Engine 通知と pending message 消費を、
/// `session_runtime_lock` を保持しない経路で実施する共通ヘルパー。
///
/// engine 内で同 session への turn 再投入があると tokio Mutex の非再入性により
/// 再入デッドロックするため、呼び出し側は lock を保持してはならない。内部で
/// `std::thread::spawn + block_on` し、呼び出し元の lock スコープから切り離す。
///
/// `pending` は streaming 中にキューされた人間メッセージ（無ければ `None`）。
/// engine 通知後に消費する。Codex app-server は独自の pending キュー
/// (`start_next_app_server_pending_turn`) を持つため、その経路では `None` を渡す。
///
/// Claude（stdout 読み取りループ）と Codex/legacy（`handle_external_bridge_message`）
/// の両経路から呼ばれ、turn 完了 → workflow 進行の通知ロジックを一本化する。
#[allow(clippy::too_many_arguments)]
pub(super) fn spawn_workflow_turn_complete_notification<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    branch_diff_context: Option<Arc<dyn BranchDiffContextPort>>,
    session_store: Arc<SessionStore>,
    handles: Arc<Mutex<AgentProcessMap>>,
    chat_session_id: String,
    workflow_turn_complete: Option<WorkflowTurnCompleteInput>,
    pending: Option<PendingMessage>,
) {
    let workflow_runtime: Option<Arc<crate::usecase::workflow::WorkflowRuntimeUsecase>> = app
        .try_state::<Arc<crate::usecase::workflow::WorkflowRuntimeUsecase>>()
        .map(|s| Arc::clone(&s));
    let handle = tokio::runtime::Handle::current();
    std::thread::spawn(move || {
        handle.block_on(async move {
            if let (Some(runtime), Some(projected)) = (workflow_runtime, workflow_turn_complete) {
                if runtime.is_session_running(&chat_session_id).await {
                    let final_text_parts = projected.final_text_parts.clone();
                    let token_usage = projected.token_usage.map(|usage| {
                        crate::usecase::workflow::ports::WorkflowTurnTokenUsage {
                            input_tokens: usage.input_tokens,
                            output_tokens: usage.output_tokens,
                        }
                    });
                    let command =
                        crate::usecase::workflow::ports::WorkflowTurnCompleteNotification {
                            chat_session_id: chat_session_id.clone(),
                            exit_code: projected.exit_code,
                            final_text_parts,
                            failure_signal: projected.failure_signal.map(|signal| match signal {
                                crate::usecase::agent_session::event_log::AgentTurnFailureSignal::ModelRefusal => {
                                    crate::usecase::workflow::ports::WorkflowTurnFailureSignal::ModelRefusal
                                }
                            }),
                            token_usage,
                            interrupted: projected.interrupted,
                        };
                    if let Err(e) = runtime.complete_turn(command).await {
                        log::error!("Workflow turn completion error for {chat_session_id}: {e}");
                    }
                }
            }
            if let Some(pending) = pending {
                start_pending_message_turn(
                    &app,
                    branch_diff_context,
                    &handles,
                    &session_store,
                    &chat_session_id,
                    pending,
                )
                .await;
            }
        });
    });
}
#[cfg(test)]
mod moved_tests {
    use super::super::external_agent::*;

    use super::super::process_registry::*;
    use super::super::sdk_message::handle_external_bridge_message;

    use super::super::shared::test_support::*;
    use super::super::shared::*;
    use super::super::skills::*;

    use crate::usecase::agent_session::session::{
        add_message_internal, create_session_internal, parts_to_legacy, ChatMessage, MessagePart,
        MessageRole,
    };

    use std::sync::Arc;

    use tokio::sync::Mutex;

    #[tokio::test]
    async fn external_agent_turn_state_records_saved_human_prompt_input() {
        let temp = tempfile::tempdir().unwrap();
        let app = tauri::test::mock_builder()
            .manage(crate::infrastructure::platform::app_data_dir::TestDataDir(
                temp.path().to_path_buf(),
            ))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let store = Arc::new(crate::test_support::build_session_store());
        let session = create_session_internal(
            &store,
            temp.path(),
            "/repo",
            Some(CODEX_BACKEND_ID.to_string()),
        )
        .unwrap();
        let image =
            validate_and_encode_image(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]).unwrap();
        let parts = vec![
            MessagePart::Text {
                content: "saved prompt".to_string(),
                parent_tool_use_id: None,
            },
            MessagePart::Image {
                data: image.data.clone(),
                media_type: image.media_type.clone(),
            },
        ];
        let mentions = vec![crate::domain::code::MentionReference {
            file_path: "src/lib.rs".to_string(),
            start_line: Some(2),
            end_line: Some(4),
        }];
        let human_message = add_message_internal(
            &store,
            temp.path(),
            &session.id,
            MessageRole::Human,
            "saved prompt",
            Some(parts.clone()),
            Some(mentions),
        )
        .unwrap();
        let agent_message = add_message_internal(
            &store,
            temp.path(),
            &session.id,
            MessageRole::Agent,
            "",
            None,
            None,
        )
        .unwrap();
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let mut proc = make_test_agent_process();
        proc.backend_id = CODEX_BACKEND_ID.to_string();
        handles.lock().await.insert(session.id.clone(), proc);

        start_external_agent_turn_state(
            &app.handle(),
            None,
            &store,
            &handles,
            &session.id,
            ExternalAgentTurnStart {
                permission_mode: "edit",
                streaming_message_id: &agent_message.id,
                prompt: "fallback prompt",
                images: std::slice::from_ref(&image),
            },
        )
        .await
        .unwrap();

        let read_model = {
            let map = handles.lock().await;
            map.get(&session.id).unwrap().turn_event_log.project()
        };
        let human = read_model
            .messages
            .iter()
            .find(|message| message.role == MessageRole::Human)
            .expect("projected human message");
        assert_eq!(human.id.as_str(), human_message.id.as_str());
        assert_eq!(human.content, "saved prompt");
        assert_eq!(human.mentions.as_ref(), human_message.mentions.as_ref());
        assert_eq!(human.parts.as_ref(), human_message.parts.as_ref());

        let mut proc = handles.lock().await.remove(&session.id).unwrap();
        let _ = proc.child.kill().await;
    }

    #[test]
    fn workflow_final_text_parts_extracts_only_text_in_order() {
        let parts = vec![
            MessagePart::Text {
                content: "one".to_string(),
                parent_tool_use_id: None,
            },
            MessagePart::ToolResult {
                content: "ignored".to_string(),
                is_error: false,
                tool_use_id: Some("tool-1".to_string()),
                parent_tool_use_id: None,
                content_ref: None,
                summary: None,
            },
            MessagePart::Text {
                content: "two".to_string(),
                parent_tool_use_id: Some("tool-1".to_string()),
            },
        ];

        assert_eq!(
            workflow_final_text_parts(&parts),
            vec!["one".to_string(), "two".to_string()]
        );
    }

    #[tokio::test]
    async fn stale_turn_complete_token_does_not_complete_active_turn() {
        let temp = tempfile::tempdir().unwrap();
        let app = tauri::test::mock_builder()
            .manage(crate::infrastructure::platform::app_data_dir::TestDataDir(
                temp.path().to_path_buf(),
            ))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let store = Arc::new(crate::test_support::build_session_store());
        let mut session = create_session_internal(
            &store,
            temp.path(),
            "/repo",
            Some(CLAUDE_BACKEND_ID.to_string()),
        )
        .unwrap();
        session.agent_session_id = Some("previous-good-session".to_string());
        store
            .save_full_session_for_migration_or_restore(temp.path(), &session)
            .unwrap();

        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let mut proc = make_test_agent_process();
        proc.backend_id = CLAUDE_BACKEND_ID.to_string();
        proc.state = BridgeState::Streaming;
        proc.turn_phase = TurnPhase::Streaming;
        proc.streaming_message_id = Some("new-agent-message".to_string());
        proc.active_turn_token = Some("new-agent-message".to_string());
        proc.sdk_session_id = Some("new-sdk-session".to_string());
        handles.lock().await.insert(session.id.clone(), proc);
        let mut state = ExternalBridgeMessageState::default();

        handle_external_bridge_message(
            &app.handle(),
            &store,
            &handles,
            &session.id,
            serde_json::json!({
                "type": "turn_complete",
                "session_id": "old-sdk-session",
                "exit_code": 0,
                "turn_token": "old-agent-message",
            }),
            &mut state,
        )
        .await;

        let loaded = store
            .load_full_session_for_restore(temp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(
            loaded.agent_session_id.as_deref(),
            Some("previous-good-session")
        );
        let removed = handles.lock().await.remove(&session.id);
        if let Some(mut proc) = removed {
            assert_eq!(proc.state, BridgeState::Streaming);
            assert_eq!(proc.turn_phase, TurnPhase::Streaming);
            assert_eq!(
                proc.streaming_message_id.as_deref(),
                Some("new-agent-message")
            );
            assert_eq!(proc.active_turn_token.as_deref(), Some("new-agent-message"));
            let _ = proc.child.kill().await;
        }
    }

    #[tokio::test]
    async fn stale_stream_token_does_not_append_to_active_message() {
        let temp = tempfile::tempdir().unwrap();
        let app = tauri::test::mock_builder()
            .manage(crate::infrastructure::platform::app_data_dir::TestDataDir(
                temp.path().to_path_buf(),
            ))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let store = Arc::new(crate::test_support::build_session_store());
        let session = create_session_internal(
            &store,
            temp.path(),
            "/repo",
            Some(CLAUDE_BACKEND_ID.to_string()),
        )
        .unwrap();

        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let mut proc = make_test_agent_process();
        proc.backend_id = CLAUDE_BACKEND_ID.to_string();
        proc.state = BridgeState::Streaming;
        proc.turn_phase = TurnPhase::Streaming;
        proc.streaming_message_id = Some("new-agent-message".to_string());
        proc.active_turn_token = Some("new-agent-message".to_string());
        handles.lock().await.insert(session.id.clone(), proc);
        let mut state = ExternalBridgeMessageState::default();

        handle_external_bridge_message(
            &app.handle(),
            &store,
            &handles,
            &session.id,
            serde_json::json!({
                "type": "stream_event",
                "turn_token": "old-agent-message",
                "event": {
                    "type": "content_block_delta",
                    "delta": {"type": "text_delta", "text": "old tail"}
                }
            }),
            &mut state,
        )
        .await;

        let removed = handles.lock().await.remove(&session.id);
        if let Some(mut proc) = removed {
            assert!(proc.streaming_parts.is_empty());
            assert_eq!(proc.turn_phase, TurnPhase::Streaming);
            let _ = proc.child.kill().await;
        }
    }

    #[tokio::test]
    async fn external_post_turn_events_reseed_from_store_without_duplication() {
        let temp = tempfile::tempdir().unwrap();
        let app = tauri::test::mock_builder()
            .manage(crate::infrastructure::platform::app_data_dir::TestDataDir(
                temp.path().to_path_buf(),
            ))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let store = Arc::new(crate::test_support::build_session_store());
        let mut session = create_session_internal(
            &store,
            temp.path(),
            "/repo",
            Some(CODEX_BACKEND_ID.to_string()),
        )
        .unwrap();
        let message_id = "agent-message-1";
        let base_parts = vec![
            MessagePart::Text {
                content: "base".to_string(),
                parent_tool_use_id: None,
            },
            MessagePart::ToolUse {
                tool: "Bash".to_string(),
                input: serde_json::json!({ "cmd": "date" }),
                id: "tool-1".to_string(),
                parent_tool_use_id: None,
            },
        ];
        let (content, thinking, activities) = parts_to_legacy(&base_parts);
        session.messages.push(ChatMessage {
            id: message_id.to_string(),
            role: MessageRole::Agent,
            content,
            thinking,
            activities,
            parts: Some(base_parts.clone()),
            streaming_final_seq: 0,
            timestamp: 10.0,
            mentions: None,
        });
        store
            .save_full_session_for_migration_or_restore(temp.path(), &session)
            .unwrap();

        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let mut proc = make_test_agent_process();
        proc.backend_id = CODEX_BACKEND_ID.to_string();
        proc.state = BridgeState::Ready;
        proc.turn_phase = TurnPhase::Idle;
        proc.last_message_id = Some(message_id.to_string());
        handles.lock().await.insert(session.id.clone(), proc);
        let mut state = ExternalBridgeMessageState::default();

        handle_external_bridge_message(
            &app.handle(),
            &store,
            &handles,
            &session.id,
            post_turn_tool_result_message("tool-1", "first"),
            &mut state,
        )
        .await;

        {
            let map = handles.lock().await;
            let proc = map.get(&session.id).unwrap();
            assert!(proc.streaming_parts.is_empty());
        }
        let first_expected = vec![
            base_parts[0].clone(),
            base_parts[1].clone(),
            MessagePart::ToolResult {
                content: "first".to_string(),
                is_error: false,
                tool_use_id: Some("tool-1".to_string()),
                parent_tool_use_id: None,
                content_ref: None,
                summary: None,
            },
        ];
        let loaded = store
            .load_full_session_for_restore(temp.path(), &session.id)
            .unwrap()
            .unwrap();
        let loaded_message = loaded
            .messages
            .iter()
            .find(|message| message.id == message_id)
            .expect("agent message persisted");
        assert_eq!(
            loaded_message.parts.as_deref(),
            Some(first_expected.as_slice())
        );

        handle_external_bridge_message(
            &app.handle(),
            &store,
            &handles,
            &session.id,
            post_turn_tool_result_message("tool-1", " second"),
            &mut state,
        )
        .await;

        let final_expected = vec![
            base_parts[0].clone(),
            base_parts[1].clone(),
            MessagePart::ToolResult {
                content: "first second".to_string(),
                is_error: false,
                tool_use_id: Some("tool-1".to_string()),
                parent_tool_use_id: None,
                content_ref: None,
                summary: None,
            },
        ];
        let loaded = store
            .load_full_session_for_restore(temp.path(), &session.id)
            .unwrap()
            .unwrap();
        let loaded_message = loaded
            .messages
            .iter()
            .find(|message| message.id == message_id)
            .expect("agent message persisted");
        assert_eq!(
            loaded_message.parts.as_deref(),
            Some(final_expected.as_slice())
        );
        let mut proc = handles.lock().await.remove(&session.id).unwrap();
        assert!(proc.streaming_parts.is_empty());
        let _ = proc.child.kill().await;
    }

    #[tokio::test]
    async fn prepare_external_pending_message_turn_requeues_pending_on_prepare_failure() {
        let temp = tempfile::tempdir().unwrap();
        let app = tauri::test::mock_builder()
            .manage(crate::infrastructure::platform::app_data_dir::TestDataDir(
                temp.path().to_path_buf(),
            ))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let store = Arc::new(crate::test_support::build_session_store());
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let session_id = "missing-session";
        let mut proc = make_test_agent_process();
        proc.pending_messages
            .push_back(test_pending_message("queued-1", "hello"));
        handles.lock().await.insert(session_id.to_string(), proc);

        let err = match prepare_external_pending_message_turn(
            &app.handle(),
            None,
            &handles,
            &store,
            session_id,
        )
        .await
        {
            Ok(_) => panic!("prepare must fail for a missing session"),
            Err(err) => err,
        };

        assert!(err.contains("Session not found"));
        assert!(
            !crate::infrastructure::agent_session::runtime::runtime_coordinator::is_pending_turn_starting(session_id).await,
            "pending turn starting marker must be cleared on failure"
        );
        {
            let map = handles.lock().await;
            let proc = map.get(session_id).unwrap();
            assert_eq!(proc.pending_messages.len(), 1);
            assert_eq!(proc.pending_messages.front().unwrap().id, "queued-1");
        }
        let mut proc = handles.lock().await.remove(session_id).unwrap();
        let _ = proc.child.kill().await;
    }

    #[tokio::test]
    async fn prepare_external_pending_message_turn_builds_system_prompt_for_pending_turn() {
        let temp = tempfile::tempdir().unwrap();
        let worktree_path = temp.path().join("repo");
        std::fs::create_dir_all(&worktree_path).unwrap();
        std::fs::write(
            worktree_path.join("AGENTS.md"),
            "Use the pending repo context.",
        )
        .unwrap();
        let app = tauri::test::mock_builder()
            .manage(crate::infrastructure::platform::app_data_dir::TestDataDir(
                temp.path().to_path_buf(),
            ))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let store = Arc::new(crate::test_support::build_session_store());
        let session = create_session_internal(
            &store,
            temp.path(),
            worktree_path.to_str().unwrap(),
            Some(CODEX_BACKEND_ID.to_string()),
        )
        .unwrap();
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let mut proc = make_test_agent_process();
        let mut pending = test_pending_message("queued-1", "hello");
        pending.worktree_path = worktree_path.to_string_lossy().to_string();
        proc.pending_messages.push_back(pending);
        handles.lock().await.insert(session.id.clone(), proc);

        let pending = prepare_external_pending_message_turn(
            &app.handle(),
            None,
            &handles,
            &store,
            &session.id,
        )
        .await
        .expect("prepare pending turn")
        .expect("pending turn");

        let system_prompt = pending.system_prompt.expect("system prompt");
        assert!(system_prompt.contains("<releash_project_instructions>"));
        assert!(system_prompt.contains("Use the pending repo context."));

        let mut proc = handles.lock().await.remove(&session.id).unwrap();
        let _ = proc.child.kill().await;
    }
}
