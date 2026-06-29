use super::external_agent::spawn_workflow_turn_complete_notification;
use super::model_selection::{available_models_for_backend, resolve_selected_model_for_response};
use super::permission::sync_pre_turn_settings;
use super::process_registry::{
    AgentProcess, AgentProcessMap, BridgeState, PendingMessage, TurnPhase,
};
use super::recovery::{
    remove_pgid, runtime_requires_spawn_locked, spawn_bridge_process, spawn_turn_watchdog,
    take_runtime_requiring_spawn_locked, RuntimeSpawnDecision, CLOSE_TIMEOUT_SECS,
};
use super::session_persistence::{
    get_persisted_spawn_info, get_required_persisted_spawn_info_before_turn,
    get_required_persisted_spawn_info_for_turn, persist_streaming_parts,
};
use super::shared::{
    build_message_cmd, consolidate_parts_from_slice, fallback_prompt_message_id,
    notify_status_transition, resolve_mentions_or_fallback_from_port,
    runtime_system_prompt_fingerprint, write_bridge_command, CLAUDE_BACKEND_ID, CODEX_BACKEND_ID,
};
use super::stream_emit::{
    emit_persisted_tool_output_resync, emit_session_state_changed,
    flush_streaming_before_transition, has_pending_stream_flush,
    release_completed_turn_streaming_buffer, spawn_streaming_timer,
};
use super::system_context_rendering::compose_system_prompt;
use super::turn_event_log::{
    append_terminal_events_and_project, begin_turn_event_log,
    clear_post_turn_store_base_untrusted_for_message, mark_post_turn_store_base_untrusted,
    projected_session_state_for_current_turn,
};
use crate::infrastructure::agent_session::resolver_ports::{
    FileSystemInstructionSourcePort, MentionResolverPort,
};
use crate::infrastructure::agent_session::runtime::runtime_coordinator::acquire_session_runtime_lock;
use crate::infrastructure::agent_session::runtime::runtime_coordinator::acquire_spawn_session_guard;
use crate::infrastructure::agent_session::runtime::runtime_coordinator::clear_pending_turn_starting;
use crate::infrastructure::agent_session::runtime::runtime_coordinator::clear_session_closing;
use crate::infrastructure::agent_session::runtime::runtime_coordinator::is_pending_turn_starting;
use crate::infrastructure::agent_session::runtime::runtime_coordinator::mark_pending_turn_starting;
use crate::infrastructure::agent_session::runtime::runtime_coordinator::mark_session_closing;
use crate::infrastructure::agent_session::runtime::runtime_coordinator::prune_session_runtime_lock;
use crate::infrastructure::agent_session::runtime::runtime_coordinator::wait_until_session_close_finished;
use crate::infrastructure::agent_session::runtime::turn_latency::{self, TurnLatencyState};
use crate::infrastructure::agent_session::runtime::AgentEditorContext;
use crate::infrastructure::agent_session::runtime::AgentMessage;
use crate::infrastructure::agent_session::runtime::ImageAttachment;
use crate::infrastructure::agent_session::runtime::SessionConfig;
use crate::infrastructure::agent_session::runtime::SessionHandle;
use crate::infrastructure::platform::app_data_dir::resolve_data_dir;
use crate::usecase::agent_session::context::{BranchDiffContextPort, SystemContextEditorInput};
use crate::usecase::agent_session::event_log::human_parts_from_content_images;
use crate::usecase::agent_session::event_log::InterruptReason;
use crate::usecase::agent_session::event_log::PromptInput;
use crate::usecase::agent_session::event_log::WorkflowTurnCompleteInput;
use crate::usecase::agent_session::session::add_message_internal;
use crate::usecase::agent_session::session::lifecycle_controller::SessionLifecycleController;
use crate::usecase::agent_session::session::now_timestamp;
use crate::usecase::agent_session::session::project_tool_output_parts_for_stream;
use crate::usecase::agent_session::session::ChatMessage;
use crate::usecase::agent_session::session::ChatSession;
use crate::usecase::agent_session::session::GetSessionResponse;
use crate::usecase::agent_session::session::InitialSessionPage;
use crate::usecase::agent_session::session::MessagePart;
use crate::usecase::agent_session::session::MessageRole;
use crate::usecase::agent_session::session::PageCursor;
use crate::usecase::agent_session::session::SessionPage;
use crate::usecase::agent_session::session::SessionState;
use crate::usecase::agent_session::session::SessionStore;
use crate::usecase::agent_session::session::SessionSummary;
use crate::usecase::agent_session::session::TokenUsage;
use crate::usecase::agent_session::session::INITIAL_SESSION_PAGE_LIMIT;
use crate::usecase::agent_session::system_prompt::{
    build_session_system_prompt as build_session_system_prompt_context,
    persist_session_system_prompt_build, SessionSystemPromptBuildRequest,
};
use serde::Serialize;
use std::collections::VecDeque;
use std::future::Future;
use std::path::Path;
use std::sync::Arc;
use tauri::Manager;
use tokio::io::AsyncWriteExt;
use tokio::process::Child;
use tokio::sync::Mutex;

use crate::usecase::agent_session::session::errors::session_target_rejected;

pub(super) fn pending_human_parts(pending: &PendingMessage) -> Option<Vec<MessagePart>> {
    let parts = human_parts_from_content_images(
        &pending.content,
        pending
            .images
            .iter()
            .map(|image| (image.data.clone(), image.media_type.clone())),
    );
    (!parts.is_empty()).then_some(parts)
}

fn system_context_editor_input(
    editor_context: Option<&AgentEditorContext>,
) -> Option<SystemContextEditorInput> {
    editor_context.map(|context| SystemContextEditorInput {
        active_editor_path: context.active_editor_path.clone(),
        open_editor_paths: context.open_editor_paths.clone(),
        selection_file_path: context
            .selection
            .as_ref()
            .map(|selection| selection.file_path.clone()),
        payload: serde_json::to_string(context).ok(),
    })
}

#[allow(clippy::too_many_arguments)]
pub(super) fn build_and_persist_session_system_prompt(
    branch_diff_context: Option<&dyn BranchDiffContextPort>,
    session_store: &SessionStore,
    data_dir: &Path,
    session: &ChatSession,
    backend_id: &str,
    model_id: Option<&str>,
    base_system_prompt: Option<String>,
    mentions: &[crate::domain::code::MentionReference],
    editor_context: Option<&AgentEditorContext>,
) -> Result<Option<String>, String> {
    build_and_persist_session_system_prompt_with_workflow_instructions(
        branch_diff_context,
        session_store,
        data_dir,
        session,
        backend_id,
        model_id,
        base_system_prompt,
        mentions,
        editor_context,
        Vec::new(),
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn build_and_persist_session_system_prompt_with_workflow_instructions(
    branch_diff_context: Option<&dyn BranchDiffContextPort>,
    session_store: &SessionStore,
    data_dir: &Path,
    session: &ChatSession,
    backend_id: &str,
    model_id: Option<&str>,
    base_system_prompt: Option<String>,
    mentions: &[crate::domain::code::MentionReference],
    editor_context: Option<&AgentEditorContext>,
    workflow_instructions: Vec<String>,
) -> Result<Option<String>, String> {
    let source = FileSystemInstructionSourcePort;
    let built = build_session_system_prompt_context(SessionSystemPromptBuildRequest {
        session_store,
        data_dir,
        session,
        branch_diff_context,
        instruction_source: &source,
        backend_id,
        model_id,
        mentions,
        editor_context: system_context_editor_input(editor_context),
        workflow_instructions,
    })?;
    let system_prompt = compose_system_prompt(base_system_prompt, &built.system_context);
    persist_session_system_prompt_build(session_store, data_dir, &session.id, &built)?;
    Ok(system_prompt)
}

fn build_internal_turn_system_prompt(
    branch_diff_context: Option<&dyn BranchDiffContextPort>,
    session_store: &SessionStore,
    data_dir: &Path,
    chat_session_id: &str,
    base_system_prompt: Option<String>,
    workflow_instructions: Vec<String>,
) -> Result<Option<String>, String> {
    let session = session_store
        .get_session_shell(data_dir, chat_session_id)?
        .ok_or_else(|| format!("Session not found: {chat_session_id}"))?;
    let backend_id = session.backend_id.as_deref().unwrap_or(CLAUDE_BACKEND_ID);
    build_and_persist_session_system_prompt_with_workflow_instructions(
        branch_diff_context,
        session_store,
        data_dir,
        &session,
        backend_id,
        session.selected_model.as_deref(),
        base_system_prompt,
        &[],
        None,
        workflow_instructions,
    )
}

#[cfg(test)]
pub(crate) fn internal_turn_system_prompt_fingerprint_for_test(
    branch_diff_context: Option<&dyn BranchDiffContextPort>,
    session_store: &SessionStore,
    data_dir: &Path,
    chat_session_id: &str,
    base_system_prompt: Option<String>,
    workflow_instructions: Vec<String>,
) -> Result<Option<String>, String> {
    let system_prompt = build_internal_turn_system_prompt(
        branch_diff_context,
        session_store,
        data_dir,
        chat_session_id,
        base_system_prompt,
        workflow_instructions,
    )?;
    Ok(runtime_system_prompt_fingerprint(system_prompt.as_deref()))
}

pub(super) struct StartedTurnPrompt {
    pub(crate) message_id: String,
    pub(crate) prompt: PromptInput,
}

pub(super) fn started_turn_prompt_from_fallback(
    streaming_message_id: &str,
    content: &str,
    images: &[ImageAttachment],
) -> StartedTurnPrompt {
    StartedTurnPrompt {
        message_id: fallback_prompt_message_id(streaming_message_id),
        prompt: PromptInput::from_content_images(
            content,
            images
                .iter()
                .map(|image| (image.data.clone(), image.media_type.clone())),
        ),
    }
}

pub(super) fn pending_message_to_queued_turn(
    pending: &PendingMessage,
) -> crate::usecase::agent_session::session::QueuedAgentTurn {
    const PREVIEW_MAX_CHARS: usize = 160;
    let mut preview: String = pending.content.chars().take(PREVIEW_MAX_CHARS).collect();
    if pending.content.chars().count() > PREVIEW_MAX_CHARS {
        preview.push_str("...");
    }
    crate::usecase::agent_session::session::QueuedAgentTurn {
        id: pending.id.clone(),
        content_preview: preview,
        created_at: pending.created_at,
        permission_mode: pending.permission_mode.clone(),
        image_count: pending.images.len(),
    }
}

pub(super) fn pending_existing_turn_ids(pending: &PendingMessage) -> Option<(&str, &str)> {
    Some((
        pending.existing_human_message_id.as_deref()?,
        pending.existing_agent_message_id.as_deref()?,
    ))
}

pub(super) fn pending_queue_view(
    proc: &AgentProcess,
) -> Vec<crate::usecase::agent_session::session::QueuedAgentTurn> {
    proc.pending_messages
        .iter()
        .map(pending_message_to_queued_turn)
        .collect()
}

pub(super) fn persist_completed_turn_session_state<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    session_store: &Arc<SessionStore>,
    chat_session_id: &str,
    exit_code: i64,
    interrupted: bool,
) -> Option<SessionState> {
    let data_dir = match resolve_data_dir(app) {
        Ok(data_dir) => data_dir,
        Err(e) => {
            log::warn!(
                "Failed to resolve data dir for completed turn state persist \
                 (session {chat_session_id}): {e}"
            );
            return None;
        }
    };
    let controller = SessionLifecycleController {
        session_store,
        data_dir: &data_dir,
    };
    match controller.complete_turn_state(chat_session_id, exit_code, interrupted) {
        Ok(session_state) => Some(session_state),
        Err(e) => {
            log::warn!("Failed to persist completed turn state for session {chat_session_id}: {e}");
            None
        }
    }
}

pub(super) struct TurnCompletePostOptions {
    pub(crate) consume_pending: bool,
}

/// Effect returned by `run_turn_complete_transition_locked`. Carries the
/// data the caller needs to perform the post-lock follow-ups: state-change
/// emission, message persistence, and workflow hooks. `turn_completed` gates
/// the `agent-session-state-changed(Idle)` emission.
#[derive(Debug, Default)]
pub(super) struct TurnCompleteTransition {
    pub(crate) turn_completed: bool,
    pub(crate) final_msg_id: Option<String>,
    pub(crate) final_parts: Vec<MessagePart>,
    pub(crate) final_streaming_seq: u64,
    pub(crate) workflow_turn_complete: Option<WorkflowTurnCompleteInput>,
    pub(crate) projected_session_state: Option<SessionState>,
    pub(crate) released_streaming_parts: Vec<MessagePart>,
}

/// Run the in-lock part of the `turn_complete` transition: force-flush
/// pending streaming delta first, then mutate `state` / `turn_phase` and
/// snapshot the data the caller needs after releasing the lock. Mirrors the
/// production stdout reader so tests can drive the exact same code path
/// (instead of mirroring prepare/apply inline).
#[cfg(test)]
pub(super) fn run_turn_complete_transition_locked<F>(
    proc: &mut AgentProcess,
    chat_session_id: &str,
    exit_code: i64,
    emit_stream: F,
) -> TurnCompleteTransition
where
    F: FnMut(&str, u64, bool, &[MessagePart]) -> bool,
{
    run_turn_complete_transition_locked_with_interrupt(
        proc,
        chat_session_id,
        exit_code,
        None,
        None,
        emit_stream,
    )
}

pub(super) fn run_turn_complete_transition_locked_with_interrupt<F>(
    proc: &mut AgentProcess,
    chat_session_id: &str,
    exit_code: i64,
    interrupt_reason: Option<InterruptReason>,
    interrupt_error: Option<String>,
    emit_stream: F,
) -> TurnCompleteTransition
where
    F: FnMut(&str, u64, bool, &[MessagePart]) -> bool,
{
    if proc.turn_phase == TurnPhase::Idle && proc.state != BridgeState::Initializing {
        return TurnCompleteTransition::default();
    }
    debug_assert_eq!(
        proc.state == BridgeState::Streaming,
        proc.streaming_message_id.is_some()
    );
    turn_latency::record_complete_latency(&mut proc.turn_latency);
    let _flushed_streaming = flush_streaming_before_transition(proc, chat_session_id, emit_stream);
    proc.state = if exit_code == 0 {
        BridgeState::Ready
    } else {
        BridgeState::Crashed
    };
    proc.turn_phase = TurnPhase::Idle;
    proc.turn_phase_since = std::time::Instant::now();
    proc.last_progress_at = None;
    proc.turn_watchdog_active = false;
    let turn_token_usage = proc.last_result_token_usage.take();
    let final_msg_id = proc.streaming_message_id.take();
    let projected_terminal = final_msg_id.as_ref().and_then(|message_id| {
        append_terminal_events_and_project(
            proc,
            message_id,
            exit_code,
            interrupt_reason.map(|reason| (reason, interrupt_error.clone())),
            turn_token_usage,
        )
    });
    let final_parts = projected_terminal
        .as_ref()
        .map(|terminal| terminal.final_parts.clone())
        .unwrap_or_else(|| consolidate_parts_from_slice(&proc.streaming_parts));
    let turn_completed = projected_terminal
        .as_ref()
        .is_some_and(|terminal| terminal.turn_completed);
    let projected_session_state = projected_terminal
        .as_ref()
        .map(|terminal| terminal.session_state.clone());
    let workflow_turn_complete =
        projected_terminal.and_then(|terminal| terminal.workflow_turn_complete);
    let completed_turn_token = proc.active_turn_token.take();
    proc.post_turn_message_token = if exit_code == 0 && final_msg_id.is_some() {
        completed_turn_token
    } else {
        None
    };
    if final_msg_id.is_some() {
        proc.last_message_id.clone_from(&final_msg_id);
    }
    let final_streaming_seq = final_msg_id
        .as_ref()
        .and_then(|message_id| proc.streaming_delta_seq_by_message.get(message_id).copied())
        .unwrap_or(proc.streaming_delta_seq);
    if turn_completed && !final_parts.is_empty() {
        if let Some(ref mid) = final_msg_id {
            mark_post_turn_store_base_untrusted(proc, mid);
        }
    }
    let released_streaming_parts = if !has_pending_stream_flush(proc) {
        release_completed_turn_streaming_buffer(proc)
    } else {
        Vec::new()
    };
    TurnCompleteTransition {
        turn_completed,
        final_msg_id,
        final_parts,
        final_streaming_seq,
        workflow_turn_complete,
        projected_session_state,
        released_streaming_parts,
    }
}

pub(super) async fn complete_streaming_turn_post_lock<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    branch_diff_context: Option<Arc<dyn BranchDiffContextPort>>,
    session_store: &Arc<SessionStore>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    chat_session_id: &str,
    mut effect: TurnCompleteTransition,
    options: TurnCompletePostOptions,
) {
    if !effect.turn_completed {
        return;
    }
    let Some(projected_turn_complete) = effect.workflow_turn_complete.as_ref() else {
        return;
    };
    let interrupted = projected_turn_complete.interrupted;
    let projected_exit_code = projected_turn_complete.exit_code;

    if let Some(ref mid) = effect.final_msg_id {
        if !effect.final_parts.is_empty() {
            let persisted_parts = persist_streaming_parts(
                session_store,
                app,
                chat_session_id,
                mid,
                &effect.final_parts,
                effect.final_streaming_seq,
                Some(now_timestamp()),
            );
            if let Some(persisted) = persisted_parts {
                clear_post_turn_store_base_untrusted_for_message(handles, chat_session_id, mid)
                    .await;
                emit_persisted_tool_output_resync(
                    app,
                    handles,
                    chat_session_id,
                    mid,
                    effect.final_streaming_seq,
                    &persisted,
                )
                .await;
            }
        }
    }
    drop(std::mem::take(&mut effect.released_streaming_parts));

    let session_state = persist_completed_turn_session_state(
        app,
        session_store,
        chat_session_id,
        projected_exit_code,
        interrupted,
    )
    .or_else(|| effect.projected_session_state.clone());

    emit_session_state_changed(
        app,
        chat_session_id,
        TurnPhase::Idle,
        Some(projected_exit_code),
        interrupted,
        session_state.clone(),
    );
    notify_status_transition(
        app,
        session_store,
        chat_session_id,
        TurnPhase::Idle,
        session_state,
    );

    let pending = if options.consume_pending {
        take_pending_message(handles, chat_session_id).await
    } else {
        None
    };
    spawn_workflow_turn_complete_notification(
        app.clone(),
        branch_diff_context,
        Arc::clone(session_store),
        Arc::clone(handles),
        chat_session_id.to_string(),
        effect.workflow_turn_complete,
        pending,
    );
}

pub(super) fn token_usage_from_result_message(msg: &serde_json::Value) -> Option<TokenUsage> {
    let model_usage = msg.get("modelUsage").and_then(|v| v.as_object())?;
    let mut input_tokens: u64 = 0;
    let mut output_tokens: u64 = 0;
    let mut total_tokens: u64 = 0;
    let mut saw_explicit_total = false;
    let mut context_window_tokens: Option<u64> = None;

    for usage in model_usage.values() {
        let input = usage
            .get("inputTokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let output = usage
            .get("outputTokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        input_tokens += input;
        output_tokens += output;
        if let Some(total) = usage.get("totalTokens").and_then(|v| v.as_u64()) {
            total_tokens += total;
            saw_explicit_total = true;
        }
        if let Some(window) = usage.get("contextWindowTokens").and_then(|v| v.as_u64()) {
            context_window_tokens =
                Some(context_window_tokens.map_or(window, |current| current.max(window)));
        }
    }

    if input_tokens == 0 && output_tokens == 0 && !saw_explicit_total {
        return None;
    }

    Some(TokenUsage {
        input_tokens,
        output_tokens,
        total_tokens: Some(if saw_explicit_total {
            total_tokens
        } else {
            input_tokens + output_tokens
        }),
        context_window_tokens,
    })
}

// Read path: session/page retrieval is kept separate from write-side lifecycle
// commands so future side-effect removal can be scoped without changing command
// signatures in this refactor.
pub(super) async fn get_session_internal<R: tauri::Runtime>(
    session_store: &Arc<SessionStore>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    registry: Option<&Arc<crate::infrastructure::agent_session::runtime::AgentBackendRegistry>>,
    app: &tauri::AppHandle<R>,
    session_id: &str,
) -> Result<Option<GetSessionResponse>, String> {
    let data_dir = resolve_data_dir(app)?;
    get_session_internal_with_data_dir(session_store, handles, registry, &data_dir, session_id)
        .await
}

pub(super) async fn get_session_internal_with_data_dir(
    session_store: &Arc<SessionStore>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    registry: Option<&Arc<crate::infrastructure::agent_session::runtime::AgentBackendRegistry>>,
    data_dir: &Path,
    session_id: &str,
) -> Result<Option<GetSessionResponse>, String> {
    let session = session_store.get_session_with_latest_page(
        data_dir,
        session_id,
        INITIAL_SESSION_PAGE_LIMIT,
    )?;
    match session {
        None => Ok(None),
        Some((mut session, page)) => {
            let initial_page = Some(InitialSessionPage {
                next_cursor: page.next_cursor,
                has_more: page.has_more,
                total_count: page.total_count,
            });
            let (turn_phase, streaming_parts, streaming_mid, pending_queue, latest_token_usage) = {
                let map = handles.lock().await;
                if let Some(proc) = map.get(session_id) {
                    // Prefer the newest queued pending message's permission_mode when present,
                    // because prepare_send_agent_message_internal persists a new mode to
                    // SessionStore and pending_messages while busy without updating
                    // current_permission_mode. Falling back to current_permission_mode
                    // keeps in-flight runtime changes (e.g. SDK-driven transitions) visible.
                    session.permission_mode = proc
                        .pending_messages
                        .back()
                        .map(|pending| pending.permission_mode.clone())
                        .unwrap_or_else(|| proc.current_permission_mode.clone());
                    let phase = proc.turn_phase;
                    let pending_queue = pending_queue_view(proc);
                    let latest_token_usage = proc.latest_token_usage;
                    if proc.state == BridgeState::Streaming {
                        (
                            phase,
                            project_tool_output_parts_for_stream(&consolidate_parts_from_slice(
                                &proc.streaming_parts,
                            )),
                            proc.streaming_message_id.clone(),
                            pending_queue,
                            latest_token_usage,
                        )
                    } else {
                        (phase, Vec::new(), None, pending_queue, latest_token_usage)
                    }
                } else {
                    (TurnPhase::Idle, Vec::new(), None, Vec::new(), None)
                }
            };

            if turn_phase == TurnPhase::Streaming || turn_phase == TurnPhase::WaitingPermission {
                if let Some(ref mid) = streaming_mid {
                    if !streaming_parts.is_empty() {
                        if let Some(msg) = session.messages.iter_mut().find(|m| m.id == *mid) {
                            msg.parts = Some(streaming_parts);
                        }
                    }
                }
            }

            // 永続的なモデル一覧の owner は config.toml 単一。プロセス内キャッシュは
            // 参照しない（プロセス側の `proc.available_models` は emit 整合用にのみ
            // 維持される）。
            // get_session は表示専用経路のため、infrastructure 故障で取得に失敗した場合は
            // warn を残して空一覧を返し、上位の UI 描画を妨げない。
            let backend_id = session
                .backend_id
                .clone()
                .unwrap_or_else(|| CLAUDE_BACKEND_ID.to_string());
            let available_models =
                available_models_for_backend(&backend_id, registry).unwrap_or_else(|e| {
                    log::warn!(
                        "get_session: backend '{backend_id}' のモデル一覧取得に失敗（空一覧で応答）: {e}"
                    );
                    Vec::new()
                });

            // モデル未選択状態は廃止。既存セッションの None は既定モデルへ解決して返す。
            // 応答の selected_model は常に非 null（flatten + skip_serializing_if のため、
            // None だとフィールドが脱落しフロントの必須 string 契約に反する）。
            let selected_model = resolve_selected_model_for_response(
                session.selected_model.take(),
                &backend_id,
                registry,
            )?;
            session.selected_model = selected_model.as_deref().map(|model_id| {
                crate::domain::agent_session::model_entry_id(&backend_id, model_id)
            });

            Ok(Some(GetSessionResponse {
                session,
                turn_phase: turn_phase.into(),
                available_models: available_models.into_iter().map(Into::into).collect(),
                pending_queue_count: pending_queue.len(),
                pending_queue,
                initial_page,
                latest_token_usage,
            }))
        }
    }
}

pub(crate) async fn get_session_page_internal_with_data_dir(
    session_store: &Arc<SessionStore>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    data_dir: &Path,
    session_id: &str,
    cursor: Option<PageCursor>,
    limit: usize,
) -> Result<Option<SessionPage>, String> {
    let mut page = match session_store.get_session_page(data_dir, session_id, cursor, limit)? {
        Some(page) => page,
        None => return Ok(None),
    };
    let (streaming_overlay, latest_token_usage) = {
        let map = handles.lock().await;
        if let Some(proc) = map.get(session_id) {
            let streaming_overlay = if proc.state == BridgeState::Streaming {
                proc.streaming_message_id
                    .as_ref()
                    .filter(|_| !proc.streaming_parts.is_empty())
                    .map(|message_id| {
                        (
                            message_id.clone(),
                            project_tool_output_parts_for_stream(&consolidate_parts_from_slice(
                                &proc.streaming_parts,
                            )),
                        )
                    })
            } else {
                None
            };
            (streaming_overlay, proc.latest_token_usage)
        } else {
            (None, None)
        }
    };
    page.latest_token_usage = latest_token_usage;
    if let Some((message_id, parts)) = streaming_overlay {
        if let Some(message) = page
            .messages
            .iter_mut()
            .find(|message| message.id == message_id)
        {
            message.parts = Some(parts);
        }
    }
    Ok(Some(page))
}

pub(super) fn can_change_session_backend_from_meta(
    session: &crate::usecase::agent_session::session::SessionMeta,
) -> bool {
    session.message_count == 0 && session.agent_session_id.is_none()
}

/// spec issues-1023: 初期 active 候補は workflow step として起動された session を
/// 除外し、free chat（`workflow_step_session == false`）の先頭を採用する。free chat が
/// 1 件もない場合は active 候補無し（`None`）で、UI は空状態を描く。
pub(super) fn pick_initial_active_session_candidate(
    sessions: &[SessionSummary],
) -> Option<&SessionSummary> {
    sessions.iter().find(|s| !s.is_workflow_step_session())
}

pub(super) fn ensure_session_backend_selected(
    session_store: &SessionStore,
    registry: &crate::infrastructure::agent_session::runtime::AgentBackendRegistry,
    data_dir: &Path,
    mut session: ChatSession,
) -> Result<ChatSession, String> {
    if session.backend_id.is_none() {
        let backend_id = registry.resolve_default_id()?;
        let selected_model = registry.default_model_for(&backend_id).ok();
        session.backend_id = Some(backend_id.clone());
        session.selected_model = selected_model.clone();
        session.updated_at = now_timestamp();
        session_store.update_backend_selection(
            data_dir,
            &session.id,
            backend_id,
            selected_model,
        )?;
    }
    Ok(session)
}

pub(super) async fn remove_stale_unstarted_agent_process(
    handles: &Arc<Mutex<AgentProcessMap>>,
    data_dir: &Path,
    chat_session_id: &str,
) {
    let stale_process = {
        let mut map = handles.lock().await;
        map.remove(chat_session_id)
    };

    if let Some(mut proc) = stale_process {
        log::warn!(
            "Removing stale agent process for unstarted session {chat_session_id} after backend change"
        );
        #[cfg(unix)]
        {
            if let Some(pg) = proc.pgid {
                unsafe {
                    libc::killpg(pg as libc::pid_t, libc::SIGKILL);
                }
            } else if let Err(e) = proc.child.kill().await {
                log::warn!("Failed to kill stale agent process {chat_session_id}: {e}");
            }
            remove_pgid(data_dir, chat_session_id);
        }
        #[cfg(not(unix))]
        {
            if let Err(e) = proc.child.kill().await {
                log::warn!("Failed to kill stale agent process {chat_session_id}: {e}");
            }
        }
        let _ = tokio::time::timeout(std::time::Duration::from_secs(1), proc.child.wait()).await;
    }
}

// Write path: backend/session lifecycle updates are intentionally grouped away
// from the read helpers above. Existing side effects are preserved.
pub(super) async fn set_session_backend_internal(
    session_store: &Arc<SessionStore>,
    registry: &Arc<crate::infrastructure::agent_session::runtime::AgentBackendRegistry>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    data_dir: &Path,
    chat_session_id: &str,
    backend_id: String,
) -> Result<GetSessionResponse, String> {
    let resolved_backend_id = registry.resolve_backend_id(Some(backend_id))?;
    let meta = session_store
        .get_session_meta(data_dir, chat_session_id)?
        .ok_or_else(|| format!("Session not found: {chat_session_id}"))?;

    if !can_change_session_backend_from_meta(&meta) {
        return Err(format!(
            "Cannot change backend after the first message has been sent: {chat_session_id}"
        ));
    }

    session_store.update_backend_selection(
        data_dir,
        chat_session_id,
        resolved_backend_id.clone(),
        Some(registry.default_model_for(&resolved_backend_id)?),
    )?;
    remove_stale_unstarted_agent_process(handles, data_dir, chat_session_id).await;

    get_session_internal_with_data_dir(
        session_store,
        handles,
        Some(registry),
        data_dir,
        chat_session_id,
    )
    .await?
    .ok_or_else(|| format!("Session not found: {chat_session_id}"))
}

pub async fn set_session_backend(
    app: tauri::AppHandle,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    registry: tauri::State<
        '_,
        Arc<crate::infrastructure::agent_session::runtime::AgentBackendRegistry>,
    >,
    handles: tauri::State<'_, Arc<Mutex<AgentProcessMap>>>,
    chat_session_id: String,
    backend_id: String,
) -> Result<GetSessionResponse, String> {
    let data_dir = resolve_data_dir(&app)?;
    set_session_backend_internal(
        session_store.inner(),
        registry.inner(),
        handles.inner(),
        &data_dir,
        &chat_session_id,
        backend_id,
    )
    .await
}

pub async fn get_session(
    state: tauri::State<'_, Arc<SessionStore>>,
    handles: tauri::State<'_, Arc<Mutex<AgentProcessMap>>>,
    registry: tauri::State<
        '_,
        Arc<crate::infrastructure::agent_session::runtime::AgentBackendRegistry>,
    >,
    app: tauri::AppHandle,
    session_id: String,
) -> Result<Option<GetSessionResponse>, String> {
    get_session_internal(
        state.inner(),
        handles.inner(),
        Some(registry.inner()),
        &app,
        &session_id,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn start_agent_session_internal<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    branch_diff_context: Option<Arc<dyn BranchDiffContextPort>>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    session_store: &Arc<SessionStore>,
    chat_session_id: &str,
    cwd: &str,
    permission_mode: Option<String>,
    plan_mode: bool,
    system_prompt: Option<String>,
    workflow_instructions: Vec<String>,
) -> Result<(), crate::infrastructure::agent_session::runtime::AgentRuntimeError> {
    // 抽象パーミッションモードを境界で解決・検証する。
    // - Some: その場で検証（Tauri/WS 境界が既に弾いている想定だが内部経路でも二重防御）。
    // - None: 内部呼び出し（workflow engine 等）として保存済みセッション値を明示参照する。
    let resolved_permission_mode = match permission_mode {
        Some(value) => crate::domain::agent_session::PermissionMode::parse(&value)
            .map(|m| m.as_str().to_string())
            .map_err(|e| {
                crate::infrastructure::agent_session::runtime::AgentRuntimeError::Other(
                    e.to_string(),
                )
            })?,
        None => {
            let data_dir = resolve_data_dir(app)
                .map_err(crate::infrastructure::agent_session::runtime::AgentRuntimeError::Other)?;
            let meta = session_store
                .get_session_meta(&data_dir, chat_session_id)?
                .ok_or_else(|| format!("Session not found: {chat_session_id}"))?;
            crate::domain::agent_session::PermissionMode::parse(&meta.permission_mode)
                .map(|m| m.as_str().to_string())
                .map_err(|e| {
                    crate::infrastructure::agent_session::runtime::AgentRuntimeError::Other(
                        e.to_string(),
                    )
                })?
        }
    };

    wait_until_session_close_finished(chat_session_id).await;
    let _spawn_guard = acquire_spawn_session_guard(chat_session_id).await;
    {
        let mut map = handles.lock().await;
        if let Some(proc) = map.get(chat_session_id) {
            if proc.state != BridgeState::Crashed {
                return Ok(());
            }
        }
        map.remove(chat_session_id);
    }

    let spawn_info = get_persisted_spawn_info(app, session_store, chat_session_id)?;
    let context_data_dir = resolve_data_dir(app)?;
    let selected_model_for_context = spawn_info.selected_model.clone();
    let system_prompt = match session_store.get_session_shell(&context_data_dir, chat_session_id)? {
        Some(session) => build_and_persist_session_system_prompt_with_workflow_instructions(
            branch_diff_context.as_deref(),
            session_store,
            &context_data_dir,
            &session,
            &spawn_info.backend_id,
            selected_model_for_context.as_deref(),
            system_prompt,
            &[],
            None,
            workflow_instructions,
        )?,
        None => system_prompt,
    };

    if spawn_info.backend_id == CODEX_BACKEND_ID {
        let backend = codex_backend_from_app(app)?;
        backend
            .start_session_runtime(SessionConfig {
                chat_session_id: chat_session_id.to_string(),
                cwd: cwd.to_string(),
                permission_mode: Some(resolved_permission_mode),
                plan_mode,
                permission_profile_id: spawn_info.permission_profile_id,
                system_prompt,
            })
            .await?;
        return Ok(());
    }

    spawn_bridge_process(
        app,
        handles,
        session_store,
        chat_session_id,
        spawn_info.backend_id,
        spawn_info.resume_sid,
        cwd,
        resolved_permission_mode,
        plan_mode,
        spawn_info.selected_model,
        system_prompt,
        spawn_info.context_restore_plan.restore_context().cloned(),
        branch_diff_context,
    )
    .await
    .map_err(crate::infrastructure::agent_session::runtime::AgentRuntimeError::Other)
}

fn record_ui_to_start_latency_for_turn(
    session: &ChatSession,
    turn: &PreparedAgentTurn,
    resume: bool,
    client_sent_at_ms: Option<f64>,
    request_received_at_ms: Option<f64>,
) {
    if turn.backend_id != CLAUDE_BACKEND_ID {
        return;
    }
    turn_latency::record_ui_to_start_latency(
        &turn.permission_mode,
        session.selected_model.as_deref(),
        session.is_workflow_step_session(),
        resume,
        session.agent_session_id.is_some(),
        client_sent_at_ms,
        request_received_at_ms,
    );
}

fn record_ui_to_start_latency_for_pending_message(
    session: &ChatSession,
    pending: &PendingMessage,
    resume: bool,
) {
    let backend_id = session.backend_id.as_deref().unwrap_or(CLAUDE_BACKEND_ID);
    if backend_id != CLAUDE_BACKEND_ID {
        return;
    }
    turn_latency::record_ui_to_start_latency(
        &pending.permission_mode,
        session.selected_model.as_deref(),
        session.is_workflow_step_session(),
        resume,
        session.agent_session_id.is_some(),
        pending.client_sent_at_ms,
        pending.request_received_at_ms,
    );
}

fn session_has_resume_sid(session: &ChatSession) -> bool {
    session
        .agent_session_id
        .as_deref()
        .map(str::trim)
        .is_some_and(|session_id| !session_id.is_empty())
}

async fn ui_to_start_resume_for_session(
    handles: &Arc<Mutex<AgentProcessMap>>,
    chat_session_id: &str,
    session: &ChatSession,
) -> bool {
    if !session_has_resume_sid(session) {
        return false;
    }
    let mut map = handles.lock().await;
    runtime_requires_spawn_locked(&mut map, chat_session_id)
}

fn maybe_record_bridge_spawn<T, E>(
    result: &Result<T, E>,
    dims: &crate::other::telemetry::AgentTurnDimensions,
    elapsed: std::time::Duration,
) {
    if result.is_ok() {
        turn_latency::record_bridge_spawn(dims, elapsed);
    }
}

async fn replace_ready_runtime_if_system_prompt_changed<R: tauri::Runtime>(
    app: Option<&tauri::AppHandle<R>>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    chat_session_id: &str,
    desired_fingerprint: Option<String>,
) -> Result<VecDeque<PendingMessage>, String> {
    let removed = {
        let mut map = handles.lock().await;
        let should_replace = map.get(chat_session_id).is_some_and(|proc| {
            matches!(proc.state, BridgeState::Ready | BridgeState::Initializing)
                && proc.turn_phase == TurnPhase::Idle
                && proc.system_prompt_fingerprint != desired_fingerprint
        });
        if should_replace {
            map.remove(chat_session_id)
        } else {
            None
        }
    };
    let Some(mut proc) = removed else {
        return Ok(VecDeque::new());
    };
    let preserved_pending_messages = std::mem::take(&mut proc.pending_messages);

    #[cfg(unix)]
    {
        if let Some(pg) = proc.pgid {
            sweep_process_group(pg).await;
            if let Some(app) = app {
                if let Ok(data_dir) = resolve_data_dir(app) {
                    remove_pgid(&data_dir, chat_session_id);
                }
            }
        } else if let Err(e) = proc.child.kill().await {
            log::warn!("Failed to kill system-context-stale agent process {chat_session_id}: {e}");
        }
    }
    #[cfg(not(unix))]
    if let Err(e) = proc.child.kill().await {
        log::warn!("Failed to kill system-context-stale agent process {chat_session_id}: {e}");
    }

    let _ = tokio::time::timeout(std::time::Duration::from_secs(1), proc.child.wait()).await;
    Ok(preserved_pending_messages)
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn start_agent_turn<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    branch_diff_context: Option<Arc<dyn BranchDiffContextPort>>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    session_store: &Arc<SessionStore>,
    chat_session_id: &str,
    cwd: &str,
    permission_mode: &str,
    plan_mode: bool,
    prompt: &str,
    system_prompt: Option<String>,
    streaming_message_id: &str,
    images: &[ImageAttachment],
) -> Result<(), String> {
    let spawn_info =
        get_required_persisted_spawn_info_for_turn(app, session_store, chat_session_id)?;
    if spawn_info.backend_id == CODEX_BACKEND_ID {
        return start_codex_backend_turn(
            app,
            chat_session_id,
            permission_mode,
            plan_mode,
            prompt,
            system_prompt,
            streaming_message_id,
            images,
        )
        .await;
    }

    let preserved_pending_messages = replace_ready_runtime_if_system_prompt_changed(
        Some(app),
        handles,
        chat_session_id,
        runtime_system_prompt_fingerprint(system_prompt.as_deref()),
    )
    .await?;

    let projected_session_state = start_agent_turn_with_runtime_spawner(
        Some(app),
        Some(session_store),
        branch_diff_context.clone(),
        handles,
        chat_session_id,
        permission_mode,
        prompt,
        streaming_message_id,
        images,
        || async {
            wait_until_session_close_finished(chat_session_id).await;
            let spawn_info = get_required_persisted_spawn_info_before_turn(
                app,
                session_store,
                chat_session_id,
                streaming_message_id,
            )?;
            let spawn_dims = turn_latency::dimensions_for_session(
                Some(app),
                Some(session_store),
                chat_session_id,
                permission_mode,
                spawn_info.selected_model.as_deref(),
                spawn_info.resume_sid.is_some(),
                spawn_info.has_session,
            );

            let spawn_started = std::time::Instant::now();
            let result = spawn_bridge_process(
                app,
                handles,
                session_store,
                chat_session_id,
                spawn_info.backend_id,
                spawn_info.resume_sid,
                cwd,
                permission_mode.to_string(),
                plan_mode,
                spawn_info.selected_model,
                system_prompt.clone(),
                spawn_info.context_restore_plan.restore_context().cloned(),
                branch_diff_context,
            )
            .await;
            maybe_record_bridge_spawn(&result, &spawn_dims, spawn_started.elapsed());
            result
        },
    )
    .await?;
    prepend_pending_messages_to_runtime(handles, chat_session_id, preserved_pending_messages).await;

    // Emit state change so frontend can track turn phase
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
pub(super) async fn start_agent_turn_locked<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    branch_diff_context: Option<Arc<dyn BranchDiffContextPort>>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    session_store: &Arc<SessionStore>,
    chat_session_id: &str,
    cwd: &str,
    permission_mode: &str,
    plan_mode: bool,
    prompt: &str,
    system_prompt: Option<String>,
    streaming_message_id: &str,
    images: &[ImageAttachment],
) -> Result<(), String> {
    let spawn_info =
        get_required_persisted_spawn_info_for_turn(app, session_store, chat_session_id)?;
    if spawn_info.backend_id == CODEX_BACKEND_ID {
        return start_codex_backend_turn(
            app,
            chat_session_id,
            permission_mode,
            plan_mode,
            prompt,
            system_prompt,
            streaming_message_id,
            images,
        )
        .await;
    }

    let preserved_pending_messages = replace_ready_runtime_if_system_prompt_changed(
        Some(app),
        handles,
        chat_session_id,
        runtime_system_prompt_fingerprint(system_prompt.as_deref()),
    )
    .await?;

    let projected_session_state = start_agent_turn_with_runtime_spawner_locked(
        Some(app),
        Some(session_store),
        branch_diff_context.clone(),
        handles,
        chat_session_id,
        permission_mode,
        prompt,
        streaming_message_id,
        images,
        || async {
            let spawn_info = get_required_persisted_spawn_info_before_turn(
                app,
                session_store,
                chat_session_id,
                streaming_message_id,
            )?;
            let spawn_dims = turn_latency::dimensions_for_session(
                Some(app),
                Some(session_store),
                chat_session_id,
                permission_mode,
                spawn_info.selected_model.as_deref(),
                spawn_info.resume_sid.is_some(),
                spawn_info.has_session,
            );

            let spawn_started = std::time::Instant::now();
            let result = spawn_bridge_process(
                app,
                handles,
                session_store,
                chat_session_id,
                spawn_info.backend_id,
                spawn_info.resume_sid,
                cwd,
                permission_mode.to_string(),
                plan_mode,
                spawn_info.selected_model,
                system_prompt.clone(),
                spawn_info.context_restore_plan.restore_context().cloned(),
                branch_diff_context,
            )
            .await;
            maybe_record_bridge_spawn(&result, &spawn_dims, spawn_started.elapsed());
            result
        },
    )
    .await?;
    prepend_pending_messages_to_runtime(handles, chat_session_id, preserved_pending_messages).await;

    // Emit state change so frontend can track turn phase
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

pub(super) fn codex_backend_from_app<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> Result<Arc<dyn crate::infrastructure::agent_session::runtime::AgentBackend>, String> {
    let registry = app
        .try_state::<Arc<crate::infrastructure::agent_session::runtime::AgentBackendRegistry>>()
        .ok_or_else(|| "AgentBackendRegistry is not registered".to_string())?;
    registry
        .get(CODEX_BACKEND_ID)
        .ok_or_else(|| format!("Agent backend not found: {CODEX_BACKEND_ID}"))
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn start_codex_backend_turn<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    chat_session_id: &str,
    permission_mode: &str,
    plan_mode: bool,
    prompt: &str,
    system_prompt: Option<String>,
    streaming_message_id: &str,
    images: &[ImageAttachment],
) -> Result<(), String> {
    start_codex_backend_turn_runtime(
        app,
        chat_session_id,
        permission_mode,
        plan_mode,
        prompt,
        system_prompt,
        streaming_message_id,
        images,
    )
    .await
    .map_err(|error| error.to_string())
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn start_codex_backend_turn_runtime<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    chat_session_id: &str,
    permission_mode: &str,
    plan_mode: bool,
    prompt: &str,
    system_prompt: Option<String>,
    streaming_message_id: &str,
    images: &[ImageAttachment],
) -> Result<(), crate::infrastructure::agent_session::runtime::AgentRuntimeError> {
    let backend = codex_backend_from_app(app)?;
    backend
        .send_message_runtime(
            &SessionHandle {
                chat_session_id: chat_session_id.to_string(),
                backend_id: CODEX_BACKEND_ID.to_string(),
            },
            AgentMessage {
                content: prompt.to_string(),
                system_prompt,
                streaming_message_id: streaming_message_id.to_string(),
                images: images.to_vec(),
                permission_mode: permission_mode.to_string(),
                plan_mode,
                permission_profile_id: None,
                editor_context: None,
            },
        )
        .await
}

#[allow(clippy::too_many_arguments)]
async fn start_agent_turn_locked_runtime<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    branch_diff_context: Option<Arc<dyn BranchDiffContextPort>>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    session_store: &Arc<SessionStore>,
    chat_session_id: &str,
    cwd: &str,
    permission_mode: &str,
    plan_mode: bool,
    prompt: &str,
    system_prompt: Option<String>,
    streaming_message_id: &str,
    images: &[ImageAttachment],
) -> Result<(), crate::infrastructure::agent_session::runtime::AgentRuntimeError> {
    let spawn_info =
        get_required_persisted_spawn_info_for_turn(app, session_store, chat_session_id)
            .map_err(crate::infrastructure::agent_session::runtime::AgentRuntimeError::Other)?;
    if spawn_info.backend_id == CODEX_BACKEND_ID {
        return start_codex_backend_turn_runtime(
            app,
            chat_session_id,
            permission_mode,
            plan_mode,
            prompt,
            system_prompt,
            streaming_message_id,
            images,
        )
        .await;
    }

    start_agent_turn_locked(
        app,
        branch_diff_context,
        handles,
        session_store,
        chat_session_id,
        cwd,
        permission_mode,
        plan_mode,
        prompt,
        system_prompt,
        streaming_message_id,
        images,
    )
    .await
    .map_err(crate::infrastructure::agent_session::runtime::AgentRuntimeError::Other)
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn start_agent_turn_with_runtime_spawner<R: tauri::Runtime, F, Fut>(
    app: Option<&tauri::AppHandle<R>>,
    session_store: Option<&Arc<SessionStore>>,
    branch_diff_context: Option<Arc<dyn BranchDiffContextPort>>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    chat_session_id: &str,
    permission_mode: &str,
    prompt: &str,
    streaming_message_id: &str,
    images: &[ImageAttachment],
    spawn_runtime: F,
) -> Result<Option<SessionState>, String>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<(), String>>,
{
    wait_until_session_close_finished(chat_session_id).await;
    let _runtime_guard = acquire_session_runtime_lock(chat_session_id).await;
    start_agent_turn_with_runtime_spawner_locked(
        app,
        session_store,
        branch_diff_context,
        handles,
        chat_session_id,
        permission_mode,
        prompt,
        streaming_message_id,
        images,
        spawn_runtime,
    )
    .await
}

pub(super) fn prompt_input_for_started_turn<R: tauri::Runtime>(
    app: Option<&tauri::AppHandle<R>>,
    session_store: Option<&Arc<SessionStore>>,
    chat_session_id: &str,
    streaming_message_id: &str,
    fallback_prompt: &str,
    fallback_images: &[ImageAttachment],
) -> StartedTurnPrompt {
    let fallback = || {
        started_turn_prompt_from_fallback(streaming_message_id, fallback_prompt, fallback_images)
    };
    let Some(app) = app else {
        return fallback();
    };
    let Some(session_store) = session_store else {
        return fallback();
    };
    let data_dir = match resolve_data_dir(app) {
        Ok(data_dir) => data_dir,
        Err(e) => {
            log::warn!("Failed to resolve data dir for turn event prompt input: {e}");
            return fallback();
        }
    };
    let human_message = match session_store.load_previous_human_message_before_agent(
        &data_dir,
        chat_session_id,
        streaming_message_id,
    ) {
        Ok(Some(message)) => message,
        Ok(None) => {
            return fallback();
        }
        Err(e) => {
            log::warn!("Failed to load session for turn event prompt input: {e}");
            return fallback();
        }
    };
    if human_message.role != MessageRole::Human {
        return fallback();
    }
    StartedTurnPrompt {
        message_id: human_message.id.clone(),
        prompt: PromptInput::from_human_message(&human_message),
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn start_agent_turn_with_runtime_spawner_locked<R: tauri::Runtime, F, Fut>(
    app: Option<&tauri::AppHandle<R>>,
    session_store: Option<&Arc<SessionStore>>,
    branch_diff_context: Option<Arc<dyn BranchDiffContextPort>>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    chat_session_id: &str,
    permission_mode: &str,
    prompt: &str,
    streaming_message_id: &str,
    images: &[ImageAttachment],
    spawn_runtime: F,
) -> Result<Option<SessionState>, String>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<(), String>>,
{
    let canonical_permission_mode =
        crate::domain::agent_session::PermissionMode::parse(permission_mode)
            .map_err(|e| e.to_string())?;
    ensure_runtime_for_turn(handles, chat_session_id, spawn_runtime).await?;
    let started_turn_prompt = prompt_input_for_started_turn(
        app,
        session_store,
        chat_session_id,
        streaming_message_id,
        prompt,
        images,
    );

    // Send message command.
    // Even if a message is sent while the SDK is still processing an interrupt,
    // the Bridge's promptGenerator queues it and only yields after the current turn completes.
    // The SDK calls generator.next() only when ready for the next turn, providing ordering guarantee.
    let msg_cmd = build_message_cmd(prompt, images, Some(streaming_message_id));
    let data = format!("{}\n", msg_cmd);

    let projected_session_state = {
        let mut map = handles.lock().await;
        if let Some(proc) = map.get_mut(chat_session_id) {
            sync_pre_turn_settings(proc, permission_mode).await?;

            proc.current_permission_mode = canonical_permission_mode.as_str().to_string();
            proc.state = BridgeState::Streaming;
            proc.turn_phase = TurnPhase::Streaming;
            proc.streaming_message_id = Some(streaming_message_id.to_string());
            proc.active_turn_token = Some(streaming_message_id.to_string());
            proc.reset_streaming_state_for_new_turn();
            proc.begin_turn_liveness();
            let has_session = proc.sdk_session_id.is_some();
            let latency_dims = turn_latency::dimensions_for_session(
                app,
                session_store,
                chat_session_id,
                canonical_permission_mode.as_str(),
                proc.selected_model.as_deref(),
                has_session,
                has_session,
            );
            proc.turn_latency = Some(TurnLatencyState::new(latency_dims));
            begin_turn_event_log(
                proc,
                &started_turn_prompt.message_id,
                started_turn_prompt.prompt,
                streaming_message_id,
                now_timestamp(),
            );
            let mut stdin = proc.stdin.lock().await;
            stdin
                .write_all(data.as_bytes())
                .await
                .map_err(|e| format!("Failed to write message: {e}"))?;
            stdin
                .flush()
                .await
                .map_err(|e| format!("Failed to flush message: {e}"))?;
            drop(stdin);
            if let Some(app) = app {
                spawn_streaming_timer(app, handles, chat_session_id, proc);
                if let Some(session_store) = session_store {
                    spawn_turn_watchdog(
                        app,
                        branch_diff_context,
                        handles,
                        session_store,
                        chat_session_id,
                        proc,
                    );
                }
            }
            projected_session_state_for_current_turn(proc)
        } else {
            return Err(format!("No agent process for session {chat_session_id}"));
        }
    };

    Ok(projected_session_state)
}

pub(super) async fn ensure_runtime_for_turn<F, Fut>(
    handles: &Arc<Mutex<AgentProcessMap>>,
    chat_session_id: &str,
    spawn_runtime: F,
) -> Result<(), String>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<(), String>>,
{
    let mut removed_crashed_process: Option<AgentProcess> = None;
    let mut preserved_pending_messages = VecDeque::new();
    let needs_spawn = {
        let mut map = handles.lock().await;
        match take_runtime_requiring_spawn_locked(&mut map, chat_session_id) {
            RuntimeSpawnDecision::Missing => true,
            RuntimeSpawnDecision::Replace(mut proc) => {
                preserved_pending_messages.append(&mut proc.pending_messages);
                removed_crashed_process = Some(*proc);
                true
            }
            RuntimeSpawnDecision::Reuse => false,
        }
    };

    if !needs_spawn {
        return Ok(());
    }

    let _spawn_guard = acquire_spawn_session_guard(chat_session_id).await;
    let needs_spawn_after_wait = {
        let mut map = handles.lock().await;
        match take_runtime_requiring_spawn_locked(&mut map, chat_session_id) {
            RuntimeSpawnDecision::Missing => true,
            RuntimeSpawnDecision::Replace(mut proc) => {
                preserved_pending_messages.append(&mut proc.pending_messages);
                if removed_crashed_process.is_none() {
                    removed_crashed_process = Some(*proc);
                }
                true
            }
            RuntimeSpawnDecision::Reuse => false,
        }
    };
    if needs_spawn_after_wait {
        if let Err(e) = spawn_runtime().await {
            let mut map = handles.lock().await;
            if let Some(mut partial_proc) = map.remove(chat_session_id) {
                preserved_pending_messages.append(&mut partial_proc.pending_messages);
            }
            if let Some(mut proc) = removed_crashed_process {
                proc.pending_messages = preserved_pending_messages;
                map.insert(chat_session_id.to_string(), proc);
            }
            return Err(e);
        }
    }
    prepend_pending_messages_to_runtime(handles, chat_session_id, preserved_pending_messages).await;
    Ok(())
}

pub(super) async fn prepend_pending_messages_to_runtime(
    handles: &Arc<Mutex<AgentProcessMap>>,
    chat_session_id: &str,
    mut pending_messages: VecDeque<PendingMessage>,
) {
    if pending_messages.is_empty() {
        return;
    }
    let mut map = handles.lock().await;
    if let Some(proc) = map.get_mut(chat_session_id) {
        let mut existing = std::mem::take(&mut proc.pending_messages);
        pending_messages.append(&mut existing);
        proc.pending_messages = pending_messages;
    }
}

pub(super) async fn requeue_pending_message_to_runtime(
    handles: &Arc<Mutex<AgentProcessMap>>,
    chat_session_id: &str,
    pending: PendingMessage,
) {
    let mut pending_messages = VecDeque::new();
    pending_messages.push_back(pending);
    prepend_pending_messages_to_runtime(handles, chat_session_id, pending_messages).await;
}

pub(super) async fn crash_agent_process_for_context_reinject<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    chat_session_id: &str,
) {
    let mut map = handles.lock().await;
    let Some(proc) = map.get_mut(chat_session_id) else {
        return;
    };
    proc.state = BridgeState::Crashed;
    proc.turn_phase = TurnPhase::Idle;
    proc.turn_watchdog_active = false;
    proc.last_progress_at = None;
    proc.mark_turn_phase_since_now();
    proc.sdk_session_id = None;
    proc.context_carry_on_ready = None;
    #[cfg(unix)]
    {
        if let Some(pg) = proc.pgid {
            unsafe {
                libc::killpg(pg as libc::pid_t, libc::SIGKILL);
            }
        } else if let Err(e) = proc.child.kill().await {
            log::warn!("Failed to kill stale resume process {chat_session_id}: {e}");
        }
        if let Ok(data_dir) = resolve_data_dir(app) {
            remove_pgid(&data_dir, chat_session_id);
        }
    }
    #[cfg(not(unix))]
    if let Err(e) = proc.child.kill().await {
        log::warn!("Failed to kill stale resume process {chat_session_id}: {e}");
    }
}

pub(super) async fn interrupt_active_agent_turn(
    handles: &Arc<Mutex<AgentProcessMap>>,
    registry: &Arc<crate::infrastructure::agent_session::runtime::AgentBackendRegistry>,
    chat_session_id: &str,
) -> Result<(), String> {
    let backend_id = {
        let map = handles.lock().await;
        map.get(chat_session_id)
            .map(|proc| proc.backend_id.clone())
            .ok_or_else(|| format!("No active agent process for session {chat_session_id}"))?
    };

    if backend_id == CODEX_BACKEND_ID {
        let backend = registry
            .get(&backend_id)
            .ok_or_else(|| format!("Agent backend not found: {backend_id}"))?;
        return backend
            .interrupt(
                &crate::infrastructure::agent_session::runtime::SessionHandle {
                    chat_session_id: chat_session_id.to_string(),
                    backend_id,
                },
            )
            .await;
    }

    write_bridge_command(
        handles,
        chat_session_id,
        serde_json::json!({ "type": "interrupt" }),
    )
    .await
}

/// Detach a pending message queued during streaming and mark its follow-up turn
/// as in-flight so tab close observes the step as busy until resume starts.
pub(super) async fn take_pending_message(
    handles: &Arc<Mutex<AgentProcessMap>>,
    chat_session_id: &str,
) -> Option<PendingMessage> {
    let pending = {
        let mut map = handles.lock().await;
        map.get_mut(chat_session_id)
            .and_then(|p| p.pending_messages.pop_front())
    };
    if pending.is_some() {
        mark_pending_turn_starting(chat_session_id).await;
    }
    pending
}

pub(super) fn pending_turn_start_failed_log_message() -> &'static str {
    "consume_pending_message_failed code=pending_turn_start_failed message=failed_to_start_pending_turn"
}

pub(super) fn prepare_pending_turn_messages(
    session_store: &Arc<SessionStore>,
    data_dir: &Path,
    chat_session_id: &str,
    pending: &PendingMessage,
) -> Result<(ChatMessage, ChatMessage, bool), String> {
    if let Some((human_message_id, agent_message_id)) = pending_existing_turn_ids(pending) {
        let session = session_store
            .load_full_session_for_restore(data_dir, chat_session_id)?
            .ok_or_else(|| format!("Session not found: {chat_session_id}"))?;
        let human_msg = session
            .messages
            .iter()
            .find(|message| message.id == human_message_id && message.role == MessageRole::Human)
            .cloned()
            .ok_or_else(|| {
                format!(
                    "Pending turn human message not found: {chat_session_id}/{human_message_id}"
                )
            })?;
        let agent_msg = session
            .messages
            .iter()
            .find(|message| message.id == agent_message_id && message.role == MessageRole::Agent)
            .cloned()
            .ok_or_else(|| {
                format!(
                    "Pending turn agent message not found: {chat_session_id}/{agent_message_id}"
                )
            })?;
        return Ok((human_msg, agent_msg, false));
    }

    let human_parts = pending_human_parts(pending);
    let human_mentions = if pending.mentions.is_empty() {
        None
    } else {
        Some(pending.mentions.clone())
    };
    let human_msg = add_message_internal(
        session_store,
        data_dir,
        chat_session_id,
        MessageRole::Human,
        &pending.content,
        human_parts,
        human_mentions,
    )?;
    let agent_msg = add_message_internal(
        session_store,
        data_dir,
        chat_session_id,
        MessageRole::Agent,
        "",
        None,
        None,
    )?;
    Ok((human_msg, agent_msg, true))
}

/// Consume a pending message queued during streaming and start the follow-up turn.
///
/// Acquires `session_runtime_lock(chat_session_id)` internally via the standard
/// `start_agent_turn` path. Callers must NOT hold the lock for this session id,
/// otherwise tokio Mutex non-reentrancy will deadlock (see issues-929).
pub(super) async fn start_pending_message_turn<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    branch_diff_context: Option<Arc<dyn BranchDiffContextPort>>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    session_store: &Arc<SessionStore>,
    chat_session_id: &str,
    pending: PendingMessage,
) {
    // 2. Add empty agent message
    let data_dir = match resolve_data_dir(app) {
        Ok(d) => d,
        Err(e) => {
            log::error!("consume_pending_message: failed to resolve data dir: {e}");
            requeue_pending_message_to_runtime(handles, chat_session_id, pending).await;
            clear_pending_turn_starting(chat_session_id).await;
            return;
        }
    };

    let session = match session_store.get_session_shell(&data_dir, chat_session_id) {
        Ok(Some(session)) => session,
        Ok(None) => {
            log::warn!("Session not found for pending system context: {chat_session_id}");
            requeue_pending_message_to_runtime(handles, chat_session_id, pending).await;
            clear_pending_turn_starting(chat_session_id).await;
            return;
        }
        Err(e) => {
            log::warn!("Failed to load session for pending system context: {e}");
            requeue_pending_message_to_runtime(handles, chat_session_id, pending).await;
            clear_pending_turn_starting(chat_session_id).await;
            return;
        }
    };
    let backend_id = session.backend_id.as_deref().unwrap_or(CLAUDE_BACKEND_ID);
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
            log::warn!("Failed to build system context for pending turn: {e}");
            requeue_pending_message_to_runtime(handles, chat_session_id, pending).await;
            clear_pending_turn_starting(chat_session_id).await;
            return;
        }
    };

    let (human_msg, agent_msg, emit_consumed_messages) =
        match prepare_pending_turn_messages(session_store, &data_dir, chat_session_id, &pending) {
            Ok(messages) => messages,
            Err(e) => {
                log::error!("consume_pending_message: failed to prepare pending messages: {e}");
                requeue_pending_message_to_runtime(handles, chat_session_id, pending).await;
                clear_pending_turn_starting(chat_session_id).await;
                return;
            }
        };

    // 3. Emit event so UI can update with the new human + agent messages
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

    let resolved_prompt = resolve_mentions_or_fallback_from_port(
        app,
        &pending.worktree_path,
        &pending.content,
        &pending.mentions,
    );
    let resume = if backend_id == CLAUDE_BACKEND_ID {
        ui_to_start_resume_for_session(handles, chat_session_id, &session).await
    } else {
        false
    };
    record_ui_to_start_latency_for_pending_message(&session, &pending, resume);

    if let Err(_e) = start_agent_turn(
        app,
        branch_diff_context,
        handles,
        session_store,
        chat_session_id,
        &pending.worktree_path,
        &pending.permission_mode,
        pending.plan_mode,
        &resolved_prompt,
        system_prompt,
        &agent_msg.id,
        &pending.images,
    )
    .await
    {
        log::error!("{}", pending_turn_start_failed_log_message());
    }
    clear_pending_turn_starting(chat_session_id).await;
}

pub async fn interrupt_agent_query(
    handles: tauri::State<'_, Arc<Mutex<AgentProcessMap>>>,
    registry: tauri::State<
        '_,
        Arc<crate::infrastructure::agent_session::runtime::AgentBackendRegistry>,
    >,
    chat_session_id: String,
) -> Result<(), String> {
    interrupt_active_agent_turn(handles.inner(), registry.inner(), &chat_session_id).await
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CancelQueuedTurnResponse {
    pub session_id: String,
    pub canceled_count: usize,
    pub pending_queue: Vec<crate::usecase::agent_session::session::QueuedAgentTurn>,
    pub pending_queue_count: usize,
}

pub async fn cancel_agent_queued_turn_internal(
    handles: &Arc<Mutex<AgentProcessMap>>,
    chat_session_id: &str,
    queued_turn_id: Option<&str>,
) -> Result<CancelQueuedTurnResponse, String> {
    let mut map = handles.lock().await;
    let proc = map
        .get_mut(chat_session_id)
        .ok_or_else(|| format!("No active agent process for session {chat_session_id}"))?;
    let before = proc.pending_messages.len();
    match queued_turn_id {
        Some(id) => proc.pending_messages.retain(|pending| pending.id != id),
        None => proc.pending_messages.clear(),
    }
    let canceled_count = before.saturating_sub(proc.pending_messages.len());
    if queued_turn_id.is_some() && canceled_count == 0 {
        return Err("Queued turn not found".to_string());
    }
    let pending_queue = pending_queue_view(proc);
    Ok(CancelQueuedTurnResponse {
        session_id: chat_session_id.to_string(),
        canceled_count,
        pending_queue_count: pending_queue.len(),
        pending_queue,
    })
}

pub(crate) async fn close_agent_session_internal<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    chat_session_id: &str,
) -> Result<(), String> {
    #[cfg(unix)]
    let pgid: Option<u32>;
    let child_to_kill: Option<Child>;

    mark_session_closing(chat_session_id).await;
    {
        let mut map = handles.lock().await;
        if let Some(proc) = map.remove(chat_session_id) {
            #[cfg(unix)]
            {
                pgid = proc.pgid;
            }
            {
                let mut stdin = proc.stdin.lock().await;
                if let Err(e) = stdin.write_all(b"{\"type\":\"close\"}\n").await {
                    log::warn!("Failed to send close command for session {chat_session_id}: {e}");
                }
                if let Err(e) = stdin.flush().await {
                    log::warn!("Failed to flush close command for session {chat_session_id}: {e}");
                }
            }
            child_to_kill = Some(proc.child);
        } else {
            // No process to close — already gone. Keep any existing close marker owned by
            // an in-flight close; clearing it here would allow a stale process group race.
            clear_session_closing(chat_session_id).await;
            return Ok(());
        }
    }

    #[cfg(unix)]
    {
        let app_clone = app.clone();
        let csid_for_pid = chat_session_id.to_string();
        tokio::spawn(async move {
            if let Some(mut child) = child_to_kill {
                match tokio::time::timeout(
                    std::time::Duration::from_secs(CLOSE_TIMEOUT_SECS),
                    child.wait(),
                )
                .await
                {
                    Ok(Ok(_)) => {
                        if let Some(pg) = pgid {
                            sweep_process_group(pg).await;
                        }
                    }
                    _ => {
                        if let Some(pg) = pgid {
                            sweep_process_group(pg).await;
                        } else if let Err(e) = child.kill().await {
                            log::warn!("Failed to kill agent process {csid_for_pid}: {e}");
                        }
                        let _ = child.wait().await;
                    }
                }
            }
            if let Ok(data_dir) = resolve_data_dir(&app_clone) {
                remove_pgid(&data_dir, &csid_for_pid);
            }
            clear_session_closing(&csid_for_pid).await;
            prune_session_runtime_lock(&csid_for_pid).await;
        });
    }

    #[cfg(not(unix))]
    if let Some(mut child) = child_to_kill {
        let csid_for_close = chat_session_id.to_string();
        tokio::spawn(async move {
            match tokio::time::timeout(
                std::time::Duration::from_secs(CLOSE_TIMEOUT_SECS),
                child.wait(),
            )
            .await
            {
                Ok(Ok(_)) => {}
                _ => {
                    let _ = child.kill().await;
                    let _ = child.wait().await;
                }
            }
            clear_session_closing(&csid_for_close).await;
            prune_session_runtime_lock(&csid_for_close).await;
        });
    }

    Ok(())
}

#[cfg(unix)]
pub(super) async fn sweep_process_group(pgid: u32) {
    unsafe {
        libc::killpg(pgid as libc::pid_t, libc::SIGTERM);
    }
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    unsafe {
        libc::killpg(pgid as libc::pid_t, libc::SIGKILL);
    }
}

/// Force kill remaining processes in the map and clear it.
/// Returns the list of session IDs that were in the map (for pid file cleanup).
pub(super) async fn force_kill_all_sessions(map: &mut AgentProcessMap) -> Vec<String> {
    let session_ids: Vec<String> = map.keys().cloned().collect();
    for csid in &session_ids {
        if let Some(proc) = map.get_mut(csid) {
            #[cfg(unix)]
            if let Some(pg) = proc.pgid {
                unsafe {
                    libc::killpg(pg as libc::pid_t, libc::SIGKILL);
                }
            }
            #[cfg(not(unix))]
            {
                let _ = proc.child.kill().await;
            }
        }
    }
    map.clear();
    session_ids
}

pub async fn close_all_agent_sessions(
    app: &tauri::AppHandle,
    handles: &Arc<Mutex<AgentProcessMap>>,
) {
    // Send graceful close command to all sessions in a single lock
    {
        let mut map = handles.lock().await;
        let ids: Vec<String> = map.keys().cloned().collect();
        for csid in &ids {
            if let Some(proc) = map.get_mut(csid) {
                let mut stdin = proc.stdin.lock().await;
                let _ = stdin.write_all(b"{\"type\":\"close\"}\n").await;
                let _ = stdin.flush().await;
            }
        }
    }

    // Wait for graceful shutdown
    tokio::time::sleep(std::time::Duration::from_secs(CLOSE_TIMEOUT_SECS)).await;

    // Force kill remaining processes
    let mut map = handles.lock().await;
    let session_ids = force_kill_all_sessions(&mut map).await;
    drop(map);

    // Remove all pid files
    #[cfg(unix)]
    if let Ok(data_dir) = resolve_data_dir(app) {
        for csid in &session_ids {
            remove_pgid(&data_dir, csid);
        }
    }

    #[cfg(not(unix))]
    let _ = app;
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SendMessageResponse {
    pub session: ChatSession,
    pub human_message: ChatMessage,
    pub agent_message: Option<ChatMessage>,
    pub queued_turn: Option<crate::usecase::agent_session::session::QueuedAgentTurn>,
    pub pending_queue: Vec<crate::usecase::agent_session::session::QueuedAgentTurn>,
    pub pending_queue_count: usize,
    pub sessions: Vec<SessionSummary>,
}

#[derive(Clone, Serialize)]
struct AgentTurnPreparedPayload<'a> {
    chat_session_id: &'a str,
    session: &'a ChatSession,
    human_message: &'a ChatMessage,
    agent_message: &'a ChatMessage,
}

fn emit_agent_turn_prepared<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    response: &SendMessageResponse,
) {
    let Some(agent_message) = response.agent_message.as_ref() else {
        return;
    };
    use tauri::Emitter;
    let _ = app.emit(
        "agent-turn-prepared",
        AgentTurnPreparedPayload {
            chat_session_id: &response.session.id,
            session: &response.session,
            human_message: &response.human_message,
            agent_message,
        },
    );
}

pub(super) struct PreparedAgentTurn {
    pub(crate) session_id: String,
    pub(crate) backend_id: String,
    pub(crate) worktree_path: String,
    pub(crate) permission_mode: String,
    pub(crate) plan_mode: bool,
    pub(crate) prompt: String,
    pub(crate) system_prompt: Option<String>,
    pub(crate) agent_message_id: String,
    pub(crate) images: Vec<ImageAttachment>,
    pub(crate) editor_context: Option<AgentEditorContext>,
    pub(crate) branch_diff_context: Option<Arc<dyn BranchDiffContextPort>>,
}

pub(super) struct PreparedAgentSteer {
    pub(crate) session_id: String,
    pub(crate) backend_id: String,
    pub(crate) permission_mode: String,
    pub(crate) plan_mode: bool,
    pub(crate) prompt: String,
    pub(crate) steering_message_id: String,
    pub(crate) images: Vec<ImageAttachment>,
    pub(crate) editor_context: Option<AgentEditorContext>,
}

pub(super) enum PreparedAgentRuntimeInput {
    Turn(PreparedAgentTurn),
    Steer(PreparedAgentSteer),
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn prepare_send_agent_message_internal(
    _app: Option<&tauri::AppHandle>,
    mention_resolver: &dyn MentionResolverPort,
    branch_diff_context: Option<Arc<dyn BranchDiffContextPort>>,
    session_store: &Arc<SessionStore>,
    registry: &Arc<crate::infrastructure::agent_session::runtime::AgentBackendRegistry>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    data_dir: &Path,
    chat_session_id: Option<String>,
    worktree_path: String,
    content: String,
    permission_mode: crate::domain::agent_session::PermissionMode,
    plan_mode: bool,
    backend_id: Option<String>,
    model_id: Option<String>,
    images: Option<Vec<ImageAttachment>>,
    mentions: Option<Vec<crate::domain::code::MentionReference>>,
    editor_context: Option<AgentEditorContext>,
    client_sent_at_ms: Option<f64>,
    request_received_at_ms: Option<f64>,
) -> Result<(SendMessageResponse, Option<PreparedAgentRuntimeInput>), String> {
    let pm = permission_mode.as_str().to_string();
    let images = images.unwrap_or_default();
    let mentions = mentions.unwrap_or_default();

    // 1. Create or get session
    let session = if let Some(ref sid) = chat_session_id {
        let mut session = session_store
            .get_session_shell(data_dir, sid)?
            .ok_or_else(|| format!("Session not found: {sid}"))?;
        if !session.is_workflow_step_session() && session.worktree_path != worktree_path {
            return Err(session_target_rejected());
        }
        // 既存セッション分岐でも検証済み pm をセッション保存層に書き戻す。
        // 新規セッション分岐と対称化し、リモート UI で start → message とした場合に
        // 選択した permission_mode が ChatSession.permission_mode に反映されるようにする。
        if session.permission_mode != pm {
            session_store.update_permission_mode(data_dir, sid, &pm)?;
            session.permission_mode = pm.clone();
        }
        if session.plan_mode != plan_mode {
            session_store.update_plan_mode(data_dir, sid, plan_mode)?;
            session.plan_mode = plan_mode;
        }
        ensure_session_backend_selected(session_store, registry, data_dir, session)?
    } else {
        let resolved_model = match model_id.as_deref() {
            Some(model_id) => Some(registry.resolve_model_entry(model_id)?),
            None => None,
        };
        let requested_backend_id = resolved_model
            .as_ref()
            .map(|entry| entry.backend.clone())
            .or(backend_id);
        let resolved_backend_id = registry.resolve_backend_id(requested_backend_id)?;
        // 新規セッションは検証済み抽象モードを初回保存で確定する。
        // 既定値で save → update_permission_mode の二段階保存を行うと、途中失敗時に
        // 選択値ではない permission_mode で永続化されたセッションが残ってしまうため
        // （Spec issues-947: セッション保存層が permission_mode の正典）、生成 API を一本化する。
        // backend の登録済み初期モデルがあれば selected_model に永続化する（Spec issues-946）。
        crate::usecase::agent_session::session::create_session_with_model_and_plan_mode(
            session_store,
            registry,
            data_dir,
            &worktree_path,
            resolved_backend_id,
            permission_mode,
            resolved_model.map(|entry| entry.model_id),
            plan_mode,
        )?
    };
    let sid = session.id.clone();
    let session_worktree_path = session.worktree_path.clone();
    let session_backend_id = session
        .backend_id
        .clone()
        .unwrap_or_else(|| CLAUDE_BACKEND_ID.to_string());

    // 2. Compute human message parts.
    // 永続化のタイミングは busy 判定後の分岐で決める。キュー投入時は session に
    // 即時追加すると transcript とキューUI に二重表示されるため、ここでは追加せず
    // drain（start_pending_message_turn / prepare_external_pending_message_turn）で追加する。
    let human_parts = if images.is_empty() {
        None
    } else {
        let mut p: Vec<MessagePart> = Vec::new();
        if !content.is_empty() {
            p.push(MessagePart::Text {
                content: content.clone(),
                parent_tool_use_id: None,
            });
        }
        for img in &images {
            p.push(MessagePart::Image {
                data: img.data.clone(),
                media_type: img.media_type.clone(),
            });
        }
        Some(p)
    };
    let human_mentions = if mentions.is_empty() {
        None
    } else {
        Some(mentions.clone())
    };

    // 3. Check turn phase
    let (current_phase, current_state, has_pending_messages) = {
        let map = handles.lock().await;
        map.get(&sid)
            .map(|p| (p.turn_phase, p.state, !p.pending_messages.is_empty()))
            .unwrap_or((TurnPhase::Idle, BridgeState::Ready, false))
    };

    // Initializing だけでは active turn とみなさない。Claude bridge は最初の
    // prompt が渡されるまで session_ready を出さないため、復帰直後の idle な
    // Initializing process には初回発話を直接送る必要がある。
    let initializing_active_turn =
        current_state == BridgeState::Initializing && current_phase != TurnPhase::Idle;
    let active_turn_busy = current_phase == TurnPhase::Streaming
        || current_phase == TurnPhase::WaitingPermission
        || initializing_active_turn;
    let pending_turn_starting = is_pending_turn_starting(&sid).await;
    let pending_queue_busy = has_pending_messages || pending_turn_starting;
    let turn_busy = active_turn_busy || pending_queue_busy;

    let (human_message, agent_message, prepared_input, queued_turn) = if turn_busy {
        let can_steer_active_turn = if active_turn_busy && !pending_turn_starting {
            if let Some(backend) = registry.get(&session_backend_id) {
                let session_handle = crate::infrastructure::agent_session::runtime::SessionHandle {
                    chat_session_id: sid.clone(),
                    backend_id: session_backend_id.clone(),
                };
                backend.active_turn_steering_ready(&session_handle).await
            } else {
                false
            }
        } else {
            false
        };

        if can_steer_active_turn {
            // steer は即座にアクティブターンへ流し込むため、人間メッセージを永続化する。
            let human_message = add_message_internal(
                session_store,
                data_dir,
                &sid,
                MessageRole::Human,
                &content,
                human_parts.clone(),
                human_mentions.clone(),
            )?;
            let resolved_prompt = mention_resolver.resolve_mentions_or_fallback(
                &session_worktree_path,
                &content,
                &mentions,
            );
            let steer = PreparedAgentSteer {
                session_id: sid.clone(),
                backend_id: session_backend_id.clone(),
                permission_mode: pm.clone(),
                plan_mode,
                prompt: resolved_prompt,
                steering_message_id: human_message.id.clone(),
                images: images.clone(),
                editor_context,
            };
            (
                human_message,
                None,
                Some(PreparedAgentRuntimeInput::Steer(steer)),
                None,
            )
        } else {
            // 4a. Queue pending message + interrupt
            // 人間メッセージはここでは永続化しない（transcript とキューUI の二重表示を
            // 避けるため）。drain 時に各 drain 関数が永続化する。response 用には
            // 非永続の ChatMessage を構築して返す。
            let pending = PendingMessage {
                id: uuid::Uuid::new_v4().to_string(),
                content: content.clone(),
                created_at: now_timestamp(),
                client_sent_at_ms,
                request_received_at_ms,
                permission_mode: pm.clone(),
                plan_mode,
                images: images.clone(),
                worktree_path: session_worktree_path.clone(),
                mentions: mentions.clone(),
                editor_context: editor_context.clone(),
                existing_human_message_id: None,
                existing_agent_message_id: None,
            };
            let queued_turn = pending_message_to_queued_turn(&pending);
            let transient_human = ChatMessage {
                id: uuid::Uuid::new_v4().to_string(),
                role: MessageRole::Human,
                content: content.clone(),
                thinking: None,
                activities: None,
                parts: human_parts.clone(),
                streaming_final_seq: 0,
                timestamp: now_timestamp(),
                mentions: None,
            };
            {
                let mut map = handles.lock().await;
                let proc = map
                    .get_mut(&sid)
                    .ok_or_else(|| format!("No active agent process for session {sid}"))?;
                proc.pending_messages.push_back(pending);
            }
            if active_turn_busy && !pending_turn_starting {
                interrupt_active_agent_turn(handles, registry, &sid).await?;
            }
            (transient_human, None, None, Some(queued_turn))
        }
    } else {
        // 4b. Create human + agent message, start turn
        let human_message = add_message_internal(
            session_store,
            data_dir,
            &sid,
            MessageRole::Human,
            &content,
            human_parts.clone(),
            human_mentions.clone(),
        )?;
        let agent_msg = add_message_internal(
            session_store,
            data_dir,
            &sid,
            MessageRole::Agent,
            "",
            None,
            None,
        )?;
        let resolved_prompt = mention_resolver.resolve_mentions_or_fallback(
            &session_worktree_path,
            &content,
            &mentions,
        );
        let system_prompt = build_and_persist_session_system_prompt(
            branch_diff_context.as_deref(),
            session_store,
            data_dir,
            &session,
            &session_backend_id,
            session.selected_model.as_deref(),
            None,
            &mentions,
            editor_context.as_ref(),
        )?;
        let turn = PreparedAgentTurn {
            session_id: sid.clone(),
            backend_id: session
                .backend_id
                .clone()
                .unwrap_or_else(|| CLAUDE_BACKEND_ID.to_string()),
            worktree_path: session_worktree_path.clone(),
            permission_mode: pm.clone(),
            plan_mode,
            prompt: resolved_prompt,
            system_prompt,
            agent_message_id: agent_msg.id.clone(),
            images: images.clone(),
            editor_context,
            branch_diff_context,
        };
        (
            human_message,
            Some(agent_msg),
            Some(PreparedAgentRuntimeInput::Turn(turn)),
            None,
        )
    };

    // 5. Get updated session shell and list. Message bodies are returned through
    // human_message / agent_message and page APIs, not the session envelope.
    let updated_session = session_store
        .get_session_shell(data_dir, &sid)?
        .ok_or_else(|| format!("Session not found: {sid}"))?;
    let sessions = session_store.list_sessions(data_dir, &session_worktree_path)?;
    let pending_queue = {
        let map = handles.lock().await;
        map.get(&sid).map(pending_queue_view).unwrap_or_default()
    };

    Ok((
        SendMessageResponse {
            session: updated_session,
            human_message,
            agent_message,
            queued_turn,
            pending_queue_count: pending_queue.len(),
            pending_queue,
            sessions,
        },
        prepared_input,
    ))
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn start_prepared_agent_turn<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    session_store: &Arc<SessionStore>,
    registry: &Arc<crate::infrastructure::agent_session::runtime::AgentBackendRegistry>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    turn: PreparedAgentTurn,
) -> Result<(), String> {
    if turn.backend_id == CODEX_BACKEND_ID {
        let backend = registry
            .get(&turn.backend_id)
            .ok_or_else(|| format!("Agent backend not found: {}", turn.backend_id))?;
        return backend
            .send_message(
                &crate::infrastructure::agent_session::runtime::SessionHandle {
                    chat_session_id: turn.session_id,
                    backend_id: turn.backend_id,
                },
                crate::infrastructure::agent_session::runtime::AgentMessage {
                    content: turn.prompt,
                    system_prompt: turn.system_prompt,
                    streaming_message_id: turn.agent_message_id,
                    images: turn.images,
                    permission_mode: turn.permission_mode,
                    plan_mode: turn.plan_mode,
                    permission_profile_id: None,
                    editor_context: turn.editor_context,
                },
            )
            .await;
    }

    start_agent_turn(
        app,
        turn.branch_diff_context,
        handles,
        session_store,
        &turn.session_id,
        &turn.worktree_path,
        &turn.permission_mode,
        turn.plan_mode,
        &turn.prompt,
        turn.system_prompt,
        &turn.agent_message_id,
        &turn.images,
    )
    .await
}

fn spawn_prepared_agent_turn_after_response<R: tauri::Runtime + 'static>(
    app: tauri::AppHandle<R>,
    session_store: Arc<SessionStore>,
    registry: Arc<crate::infrastructure::agent_session::runtime::AgentBackendRegistry>,
    handles: Arc<Mutex<AgentProcessMap>>,
    turn: PreparedAgentTurn,
) {
    let session_id = turn.session_id.clone();
    tokio::spawn(async move {
        tokio::task::yield_now().await;
        if let Err(e) =
            start_prepared_agent_turn(&app, &session_store, &registry, &handles, turn).await
        {
            log::error!("Failed to start prepared agent turn for session {session_id}: {e}");
            emit_prepared_agent_turn_start_error(&app, &session_store, &session_id);
        }
    });
}

fn emit_prepared_agent_turn_start_error<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    session_store: &Arc<SessionStore>,
    chat_session_id: &str,
) {
    let session_state = Some(SessionState::Error);
    persist_prepared_agent_turn_start_error_state(app, session_store, chat_session_id);
    emit_session_state_changed(
        app,
        chat_session_id,
        TurnPhase::Idle,
        None,
        false,
        session_state.clone(),
    );
    notify_status_transition(
        app,
        session_store,
        chat_session_id,
        TurnPhase::Idle,
        session_state,
    );
}

fn persist_prepared_agent_turn_start_error_state<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    session_store: &Arc<SessionStore>,
    chat_session_id: &str,
) {
    let Ok(data_dir) = resolve_data_dir(app) else {
        return;
    };
    match session_store.get_session_meta(&data_dir, chat_session_id) {
        Ok(Some(meta)) if meta.state != SessionState::Archived => {
            if let Err(e) =
                session_store.set_session_state(&data_dir, chat_session_id, SessionState::Error)
            {
                log::warn!(
                    "Failed to persist prepared agent turn start error state for session \
                     {chat_session_id}: {e}"
                );
            }
        }
        Ok(_) => {}
        Err(e) => {
            log::warn!(
                "Failed to load session metadata for prepared agent turn start error \
                 (session {chat_session_id}): {e}"
            );
        }
    }
}

pub(super) async fn steer_prepared_agent_turn(
    registry: &Arc<crate::infrastructure::agent_session::runtime::AgentBackendRegistry>,
    steer: PreparedAgentSteer,
) -> Result<(), String> {
    let backend = registry
        .get(&steer.backend_id)
        .ok_or_else(|| format!("Agent backend not found: {}", steer.backend_id))?;
    backend
        .steer_message(
            &crate::infrastructure::agent_session::runtime::SessionHandle {
                chat_session_id: steer.session_id,
                backend_id: steer.backend_id,
            },
            crate::infrastructure::agent_session::runtime::AgentMessage {
                content: steer.prompt,
                system_prompt: None,
                streaming_message_id: steer.steering_message_id,
                images: steer.images,
                permission_mode: steer.permission_mode,
                plan_mode: steer.plan_mode,
                permission_profile_id: None,
                editor_context: steer.editor_context,
            },
        )
        .await
}

/// Unified command to send a message: handles session creation, message persistence,
/// turn phase check (interrupt if streaming, start query if idle), and pending message queuing.
#[allow(clippy::too_many_arguments)]
pub async fn send_agent_message_internal<R: tauri::Runtime + 'static>(
    app: &tauri::AppHandle<R>,
    branch_diff_context: Option<Arc<dyn BranchDiffContextPort>>,
    session_store: &Arc<SessionStore>,
    registry: &Arc<crate::infrastructure::agent_session::runtime::AgentBackendRegistry>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    chat_session_id: Option<String>,
    worktree_path: String,
    content: String,
    permission_mode: crate::domain::agent_session::PermissionMode,
    plan_mode: bool,
    backend_id: Option<String>,
    model_id: Option<String>,
    images: Option<Vec<ImageAttachment>>,
    mentions: Option<Vec<crate::domain::code::MentionReference>>,
    editor_context: Option<AgentEditorContext>,
    client_sent_at_ms: Option<f64>,
    request_received_at_ms: Option<f64>,
) -> Result<SendMessageResponse, String> {
    let lock_key = chat_session_id
        .as_deref()
        .map(str::to_string)
        .unwrap_or_else(|| format!("new-session:{worktree_path}"));
    let data_dir = resolve_data_dir(app)?;
    let mention_resolver = app
        .try_state::<Arc<dyn MentionResolverPort>>()
        .map(|state| state.inner().clone())
        .ok_or_else(|| "MentionResolverPort is not registered".to_string())?;
    let (response, prepared_input) = {
        let _send_guard = acquire_session_runtime_lock(&lock_key).await;
        prepare_send_agent_message_internal(
            None,
            mention_resolver.as_ref(),
            branch_diff_context,
            session_store,
            registry,
            handles,
            &data_dir,
            chat_session_id,
            worktree_path,
            content,
            permission_mode,
            plan_mode,
            backend_id,
            model_id,
            images,
            mentions,
            editor_context,
            client_sent_at_ms,
            request_received_at_ms,
        )
        .await?
    };

    if let Some(PreparedAgentRuntimeInput::Turn(turn)) = prepared_input.as_ref() {
        let resume = if turn.backend_id == CLAUDE_BACKEND_ID {
            ui_to_start_resume_for_session(handles, &turn.session_id, &response.session).await
        } else {
            false
        };
        record_ui_to_start_latency_for_turn(
            &response.session,
            turn,
            resume,
            client_sent_at_ms,
            request_received_at_ms,
        );
    }

    if let Some(input) = prepared_input {
        match input {
            PreparedAgentRuntimeInput::Turn(turn) => {
                emit_agent_turn_prepared(app, &response);
                spawn_prepared_agent_turn_after_response(
                    app.clone(),
                    Arc::clone(session_store),
                    Arc::clone(registry),
                    Arc::clone(handles),
                    turn,
                );
            }
            PreparedAgentRuntimeInput::Steer(steer) => {
                steer_prepared_agent_turn(registry, steer).await?;
            }
        }
    }

    Ok(response)
}

#[allow(clippy::too_many_arguments)]
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InitSessionsResponse {
    pub sessions: Vec<SessionSummary>,
    pub active_session: Option<GetSessionResponse>,
    pub permission_mode: String,
    pub plan_mode: bool,
}

/// Unified command for session initialization: lists sessions, starts Bridge processes,
/// creates a new session if empty, returns sessions + active session.
pub async fn init_agent_sessions(
    app: tauri::AppHandle,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    registry: tauri::State<
        '_,
        Arc<crate::infrastructure::agent_session::runtime::AgentBackendRegistry>,
    >,
    handles: tauri::State<'_, Arc<Mutex<AgentProcessMap>>>,
    open_tabs: tauri::State<'_, Arc<crate::usecase::agent_session::session::OpenTabRegistry>>,
    worktree_path: String,
) -> Result<InitSessionsResponse, String> {
    init_agent_sessions_internal(
        &app,
        session_store.inner(),
        registry.inner(),
        handles.inner(),
        open_tabs.inner(),
        worktree_path,
    )
    .await
}

pub(super) async fn init_agent_sessions_internal<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    session_store: &Arc<SessionStore>,
    registry: &Arc<crate::infrastructure::agent_session::runtime::AgentBackendRegistry>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    open_tabs: &Arc<crate::usecase::agent_session::session::OpenTabRegistry>,
    worktree_path: String,
) -> Result<InitSessionsResponse, String> {
    let data_dir = resolve_data_dir(app)?;

    crate::adaptor::gateway::workflow::hydrate_open_workflow_step_tabs(
        session_store,
        &data_dir,
        &worktree_path,
        open_tabs,
    )?;
    let sessions = session_store.list_sessions(&data_dir, &worktree_path)?;

    if sessions.is_empty() {
        Ok(InitSessionsResponse {
            sessions,
            active_session: None,
            permission_mode: crate::domain::agent_session::PermissionMode::Edit
                .as_str()
                .to_string(),
            plan_mode: false,
        })
    } else {
        // spec issues-1023: workflow step として起動された chat session は free chat
        // tab bar 上に同格に並ばないため、初期 active session 候補からも除外する。
        // 候補が無い場合は active_session を None で返し、UI は空状態を描く。
        let active_candidate = pick_initial_active_session_candidate(&sessions);
        let active = if let Some(candidate) = active_candidate {
            get_session_internal(session_store, handles, Some(registry), app, &candidate.id).await?
        } else {
            None
        };
        let (permission_mode, plan_mode) = active
            .as_ref()
            .map(|response| {
                (
                    response.session.permission_mode.clone(),
                    response.session.plan_mode,
                )
            })
            .unwrap_or_else(|| {
                (
                    crate::domain::agent_session::PermissionMode::Edit
                        .as_str()
                        .to_string(),
                    false,
                )
            });

        Ok(InitSessionsResponse {
            sessions,
            active_session: active,
            permission_mode,
            plan_mode,
        })
    }
}

/// Runtime lock acquired by the caller variant used by workflow step startup.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn start_agent_turn_internal_locked<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    branch_diff_context: Option<Arc<dyn BranchDiffContextPort>>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    session_store: &Arc<SessionStore>,
    chat_session_id: &str,
    cwd: &str,
    permission_mode: &str,
    prompt: &str,
    base_system_prompt: Option<String>,
    workflow_instructions: Vec<String>,
) -> Result<(), crate::infrastructure::agent_session::runtime::AgentRuntimeError> {
    let data_dir = resolve_data_dir(app)
        .map_err(crate::infrastructure::agent_session::runtime::AgentRuntimeError::Other)?;

    // Add human message
    let _human_msg = add_message_internal(
        session_store,
        &data_dir,
        chat_session_id,
        MessageRole::Human,
        prompt,
        None,
        None,
    )
    .map_err(crate::infrastructure::agent_session::runtime::AgentRuntimeError::Other)?;

    // Add empty agent message (will be filled by streaming)
    let agent_msg = add_message_internal(
        session_store,
        &data_dir,
        chat_session_id,
        MessageRole::Agent,
        "",
        None,
        None,
    )
    .map_err(crate::infrastructure::agent_session::runtime::AgentRuntimeError::Other)?;

    let system_prompt = build_internal_turn_system_prompt(
        branch_diff_context.as_deref(),
        session_store,
        &data_dir,
        chat_session_id,
        base_system_prompt,
        workflow_instructions,
    )
    .map_err(crate::infrastructure::agent_session::runtime::AgentRuntimeError::Other)?;

    start_agent_turn_locked_runtime(
        app,
        branch_diff_context,
        handles,
        session_store,
        chat_session_id,
        cwd,
        permission_mode,
        false,
        prompt,
        system_prompt,
        &agent_msg.id,
        &[],
    )
    .await
}
#[cfg(test)]
mod moved_tests {
    use super::super::process_registry::*;

    use super::super::session_lifecycle::*;

    use super::super::shared::test_support::*;
    use super::super::shared::*;

    use crate::adaptor::gateway::agent_session::FileSessionStorage;
    use crate::adaptor::gateway::workflow::state::WorkflowExecutionState;
    use crate::adaptor::gateway::workflow::test_support::TestRuntimeKernel;
    use crate::infrastructure::agent_session::runtime::runtime_coordinator::clear_pending_turn_starting;
    use crate::infrastructure::agent_session::runtime::{
        AgentBackend, AgentBackendRegistry, AgentMessage, ModelInfo, PermissionResponse,
        SessionConfig, SessionHandle,
    };
    use crate::usecase::agent_session::context::{
        BranchDiffContextChangedFile, BranchDiffContextPort, BranchDiffContextStats,
        BranchDiffContextSummary,
    };
    use crate::usecase::agent_session::event_log::{
        PromptInput, TurnEventLog, WorkflowTurnCompleteInput,
    };

    use crate::usecase::agent_session::session::{
        add_message_internal, create_session_internal, AttachmentRef, ChatMessage, ChatSession,
        MessageMention, MessagePart, MessageRole, SessionStore, WorkflowStepContextDto,
    };

    use std::collections::{HashMap, VecDeque};

    use std::sync::{Arc, Mutex as StdMutex};
    use std::time::{Duration, Instant};

    use tauri::Listener;
    use tokio::sync::{Mutex, Semaphore};

    struct FakeBranchDiffContext;

    impl BranchDiffContextPort for FakeBranchDiffContext {
        fn get_branch_diff_context(
            &self,
            _worktree_path: &str,
        ) -> Result<BranchDiffContextSummary, String> {
            Ok(BranchDiffContextSummary {
                base_branch: "main".to_string(),
                changed_files: vec![BranchDiffContextChangedFile {
                    path: "src/lib.rs".to_string(),
                    status: "modified".to_string(),
                    stats: BranchDiffContextStats {
                        additions: 3,
                        deletions: 1,
                    },
                }],
            })
        }
    }

    struct BlockingSendBackend {
        started: Arc<Semaphore>,
        release: Arc<Semaphore>,
        finished: Arc<Semaphore>,
    }

    #[async_trait::async_trait]
    impl AgentBackend for BlockingSendBackend {
        fn id(&self) -> &str {
            CODEX_BACKEND_ID
        }

        fn name(&self) -> &str {
            "BlockingSend"
        }

        async fn start_session(&self, config: SessionConfig) -> Result<SessionHandle, String> {
            Ok(SessionHandle {
                chat_session_id: config.chat_session_id,
                backend_id: CODEX_BACKEND_ID.to_string(),
            })
        }

        async fn send_message(
            &self,
            _session: &SessionHandle,
            _message: AgentMessage,
        ) -> Result<(), String> {
            self.started.add_permits(1);
            let _permit = self
                .release
                .acquire()
                .await
                .map_err(|_| "release semaphore closed".to_string())?;
            self.finished.add_permits(1);
            Ok(())
        }

        async fn interrupt(&self, _session: &SessionHandle) -> Result<(), String> {
            Ok(())
        }

        async fn respond_permission(
            &self,
            _session: &SessionHandle,
            _response: PermissionResponse,
        ) -> Result<(), String> {
            Ok(())
        }

        fn fixed_models(&self) -> Option<Vec<String>> {
            Some(vec!["mock-model".to_string()])
        }
    }

    fn pending_message_for_test(id: &str) -> PendingMessage {
        PendingMessage {
            id: id.to_string(),
            content: format!("pending {id}"),
            created_at: 1.0,
            client_sent_at_ms: None,
            request_received_at_ms: None,
            permission_mode: "edit".to_string(),
            plan_mode: false,
            images: Vec::new(),
            worktree_path: "/repo".to_string(),
            mentions: Vec::new(),
            editor_context: None,
            existing_human_message_id: None,
            existing_agent_message_id: None,
        }
    }

    #[tokio::test]
    async fn send_agent_message_emits_placeholder_event_before_runtime_send_completes() {
        let temp = tempfile::tempdir().unwrap();
        let started = Arc::new(Semaphore::new(0));
        let release = Arc::new(Semaphore::new(0));
        let finished = Arc::new(Semaphore::new(0));

        let mut registry = AgentBackendRegistry::new();
        registry.register(Arc::new(BlockingSendBackend {
            started: Arc::clone(&started),
            release: Arc::clone(&release),
            finished: Arc::clone(&finished),
        }));
        registry.set_default(Some(CODEX_BACKEND_ID.to_string()));
        let registry = Arc::new(registry);
        let session_store = Arc::new(crate::test_support::build_session_store());
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let mention_resolver: Arc<
            dyn crate::infrastructure::agent_session::resolver_ports::MentionResolverPort,
        > = Arc::new(crate::adaptor::controller::wiring::build_code_usecase());
        let app = tauri::test::mock_builder()
            .manage(crate::infrastructure::platform::app_data_dir::TestDataDir(
                temp.path().to_path_buf(),
            ))
            .manage(mention_resolver)
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();

        let (prepared_tx, prepared_rx) = tokio::sync::oneshot::channel();
        let prepared_tx = Arc::new(StdMutex::new(Some(prepared_tx)));
        let prepared_tx_for_listener = Arc::clone(&prepared_tx);
        app.listen("agent-turn-prepared", move |event| {
            let payload = serde_json::from_str::<serde_json::Value>(event.payload())
                .expect("agent-turn-prepared payload must be json");
            if let Some(tx) = prepared_tx_for_listener.lock().unwrap().take() {
                let _ = tx.send(payload);
            }
        });

        let response = tokio::time::timeout(
            Duration::from_millis(200),
            send_agent_message_internal(
                app.handle(),
                None,
                &session_store,
                &registry,
                &handles,
                None,
                "/repo".to_string(),
                "hello".to_string(),
                crate::domain::agent_session::PermissionMode::Edit,
                false,
                Some(CODEX_BACKEND_ID.to_string()),
                None,
                None,
                None,
                None,
                None,
                None,
            ),
        )
        .await
        .expect("send response must not wait for runtime send")
        .unwrap();

        let agent_message = response
            .agent_message
            .expect("new turn response includes agent placeholder");
        assert_eq!(agent_message.role, MessageRole::Agent);
        assert!(agent_message
            .parts
            .as_ref()
            .map(Vec::is_empty)
            .unwrap_or(true));

        let prepared_payload = tokio::time::timeout(Duration::from_secs(1), prepared_rx)
            .await
            .expect("agent-turn-prepared should emit before runtime send completes")
            .expect("agent-turn-prepared channel should stay open");
        assert_eq!(
            prepared_payload
                .get("chat_session_id")
                .and_then(serde_json::Value::as_str),
            Some(response.session.id.as_str())
        );
        assert_eq!(
            prepared_payload
                .get("agent_message")
                .and_then(|message| message.get("id"))
                .and_then(serde_json::Value::as_str),
            Some(agent_message.id.as_str())
        );

        let _started_permit = tokio::time::timeout(Duration::from_secs(1), started.acquire())
            .await
            .expect("background runtime send should start after response")
            .expect("started semaphore should stay open");
        release.add_permits(1);
        let _finished_permit = tokio::time::timeout(Duration::from_secs(1), finished.acquire())
            .await
            .expect("background runtime send should finish after release")
            .expect("finished semaphore should stay open");
    }

    #[tokio::test]
    async fn prepared_turn_start_error_emits_terminal_error_state() {
        let temp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let session = create_session_internal(&session_store, temp.path(), "/repo", None).unwrap();
        session_store
            .set_session_state(temp.path(), &session.id, SessionState::Done)
            .unwrap();
        let center = Arc::new(crate::usecase::agent_session::status::AgentStatusCenter::new());
        let app = tauri::test::mock_builder()
            .manage(crate::infrastructure::platform::app_data_dir::TestDataDir(
                temp.path().to_path_buf(),
            ))
            .manage(Arc::clone(&center))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();

        let (state_tx, state_rx) = tokio::sync::oneshot::channel();
        let state_tx = Arc::new(StdMutex::new(Some(state_tx)));
        let state_tx_for_listener = Arc::clone(&state_tx);
        app.listen("agent-session-state-changed", move |event| {
            let payload = serde_json::from_str::<serde_json::Value>(event.payload())
                .expect("agent-session-state-changed payload must be json");
            if let Some(tx) = state_tx_for_listener.lock().unwrap().take() {
                let _ = tx.send(payload);
            }
        });

        emit_prepared_agent_turn_start_error(app.handle(), &session_store, &session.id);

        let state_payload = tokio::time::timeout(Duration::from_secs(1), state_rx)
            .await
            .expect("terminal state event should be emitted")
            .expect("terminal state channel should stay open");
        assert_eq!(
            state_payload
                .get("chat_session_id")
                .and_then(serde_json::Value::as_str),
            Some(session.id.as_str())
        );
        assert_eq!(
            state_payload
                .get("turn_phase")
                .and_then(serde_json::Value::as_str),
            Some("idle")
        );
        assert_eq!(
            state_payload
                .get("session_state")
                .and_then(serde_json::Value::as_str),
            Some("error")
        );

        let status = center
            .get_session(&session.id)
            .expect("status center receives terminal state");
        assert_eq!(
            status.agent_state,
            crate::usecase::agent_session::status::AgentState::Error
        );
        assert_eq!(
            status.turn_phase,
            crate::usecase::agent_session::status::TurnPhaseRepr::Idle
        );
        assert_eq!(status.session_state, SessionState::Error);
        let meta = session_store
            .get_session_meta(temp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(meta.state, SessionState::Error);
    }

    #[tokio::test]
    async fn complete_streaming_turn_post_lock_uses_projected_state_when_persist_fails() {
        let app = tauri::test::mock_builder()
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));

        let (state_tx, state_rx) = tokio::sync::oneshot::channel();
        let state_tx = Arc::new(StdMutex::new(Some(state_tx)));
        let state_tx_for_listener = Arc::clone(&state_tx);
        app.listen("agent-session-state-changed", move |event| {
            let payload = serde_json::from_str::<serde_json::Value>(event.payload())
                .expect("agent-session-state-changed payload must be json");
            if let Some(tx) = state_tx_for_listener.lock().unwrap().take() {
                let _ = tx.send(payload);
            }
        });

        complete_streaming_turn_post_lock(
            app.handle(),
            None,
            &session_store,
            &handles,
            "missing-session",
            TurnCompleteTransition {
                turn_completed: true,
                workflow_turn_complete: Some(WorkflowTurnCompleteInput {
                    turn_id: 1,
                    exit_code: 1,
                    final_text_parts: Vec::new(),
                    failure_signal: None,
                    token_usage: None,
                    interrupted: false,
                }),
                projected_session_state: Some(SessionState::Error),
                ..Default::default()
            },
            TurnCompletePostOptions {
                consume_pending: false,
            },
        )
        .await;

        let state_payload = tokio::time::timeout(Duration::from_secs(1), state_rx)
            .await
            .expect("completed turn state event should be emitted")
            .expect("completed turn state channel should stay open");
        assert_eq!(
            state_payload
                .get("chat_session_id")
                .and_then(serde_json::Value::as_str),
            Some("missing-session")
        );
        assert_eq!(
            state_payload
                .get("session_state")
                .and_then(serde_json::Value::as_str),
            Some("error")
        );
    }

    #[test]
    fn internal_turn_system_prompt_uses_session_shell_context() {
        let temp = tempfile::tempdir().unwrap();
        let _app = tauri::test::mock_builder()
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
        session.workflow_step_session = true;
        session.selected_model = Some("sonnet".to_string());
        session.workflow_step_context = Some(WorkflowStepContextDto {
            run_id: "run-1".to_string(),
            workflow_name: "workflow".to_string(),
            step_name: "step-a".to_string(),
            run_index: 1,
            parent_step_name: None,
            parent_run_index: None,
            order: 0,
            startup_timeout_secs: None,
            startup_max_retries: None,
            stale_timeout_secs: None,
        });
        store
            .save_full_session_for_migration_or_restore(temp.path(), &session)
            .unwrap();
        store
            .update_system_context_private_meta_if_changed(
                temp.path(),
                &session.id,
                None,
                vec!["private workflow instruction".to_string()],
                None,
            )
            .unwrap();

        let system_prompt = build_internal_turn_system_prompt(
            None,
            &store,
            temp.path(),
            &session.id,
            Some("workflow policy prompt".to_string()),
            Vec::new(),
        )
        .unwrap()
        .expect("system prompt");

        assert!(system_prompt.contains("workflow policy prompt"));
        assert!(system_prompt.contains("private workflow instruction"));
        assert!(system_prompt.contains("<releash_workflow_state>"));
        assert!(system_prompt.contains("<releash_backend_model_identity>"));
        assert!(system_prompt.contains("backend_id: claude"));
        assert!(store
            .get_session_meta(temp.path(), &session.id)
            .unwrap()
            .and_then(|meta| meta.context_epoch)
            .is_some());
    }

    #[test]
    fn saved_read_path_adds_file_neighbor_instruction_to_system_prompt() {
        let temp = tempfile::tempdir().unwrap();
        let store = Arc::new(crate::test_support::build_session_store());
        let worktree = temp.path().join("worktree");
        std::fs::create_dir_all(worktree.join("src/local")).unwrap();
        std::fs::write(worktree.join("AGENTS.md"), "root instruction").unwrap();
        std::fs::write(
            worktree.join("src/local/AGENTS.md"),
            "local file-neighbor instruction",
        )
        .unwrap();
        std::fs::write(worktree.join("src/local/file.rs"), "fn main() {}").unwrap();
        let session = create_session_internal(
            &store,
            temp.path(),
            &worktree.to_string_lossy(),
            Some(CLAUDE_BACKEND_ID.to_string()),
        )
        .unwrap();
        add_message_internal(
            &store,
            temp.path(),
            &session.id,
            MessageRole::Agent,
            "",
            Some(vec![MessagePart::ToolUse {
                tool: "Read".to_string(),
                input: serde_json::json!({"file_path": "src/local/file.rs"}),
                id: "tool-1".to_string(),
                parent_tool_use_id: None,
            }]),
            None,
        )
        .unwrap();
        let shell = store
            .get_session_shell(temp.path(), &session.id)
            .unwrap()
            .expect("session shell");
        assert!(shell.messages.is_empty());

        let system_prompt = build_and_persist_session_system_prompt(
            None,
            &store,
            temp.path(),
            &shell,
            CLAUDE_BACKEND_ID,
            None,
            None,
            &[],
            None,
        )
        .unwrap()
        .expect("system prompt");

        assert!(system_prompt.contains("<releash_project_instructions>"));
        assert!(system_prompt.contains("local file-neighbor instruction"));
    }

    #[test]
    fn prompt_input_from_human_message_preserves_mentions_and_prompt_parts() {
        let attachment = AttachmentRef {
            id: "att-1".to_string(),
            media_type: "image/png".to_string(),
            byte_size: 42,
        };
        let parts = vec![
            MessagePart::Text {
                content: "inspect this".to_string(),
                parent_tool_use_id: None,
            },
            MessagePart::ImageRef {
                attachment: attachment.clone(),
            },
        ];
        let mentions = vec![MessageMention {
            file_path: "src/main.rs".to_string(),
            start_line: Some(7),
            end_line: Some(9),
        }];
        let message = ChatMessage {
            id: "human-1".to_string(),
            role: MessageRole::Human,
            content: "inspect this".to_string(),
            thinking: None,
            activities: None,
            parts: Some(parts.clone()),
            streaming_final_seq: 0,
            timestamp: 1.0,
            mentions: Some(mentions.clone()),
        };

        let prompt = PromptInput::from_human_message(&message);

        assert_eq!(prompt.content, "inspect this");
        assert_eq!(prompt.mentions, mentions);
        assert_eq!(prompt.attachment_refs, vec![attachment]);
        assert_eq!(prompt.parts, parts);
    }

    #[test]
    fn started_turn_prompt_uses_saved_human_message_id_and_parts() {
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
        let parts = vec![MessagePart::Text {
            content: "inspect this".to_string(),
            parent_tool_use_id: None,
        }];
        let mentions = vec![crate::domain::code::MentionReference {
            file_path: "src/main.rs".to_string(),
            start_line: Some(7),
            end_line: Some(9),
        }];
        let human_message = add_message_internal(
            &store,
            temp.path(),
            &session.id,
            MessageRole::Human,
            "inspect this",
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

        let started_turn_prompt = prompt_input_for_started_turn(
            Some(app.handle()),
            Some(&store),
            &session.id,
            &agent_message.id,
            "fallback",
            &[],
        );

        assert_eq!(started_turn_prompt.message_id, human_message.id.as_str());
        assert_eq!(
            started_turn_prompt.prompt,
            PromptInput::from_human_message(&human_message)
        );
    }

    #[test]
    fn started_turn_prompt_selects_previous_human_before_target_agent_with_single_message_read() {
        let temp = tempfile::tempdir().unwrap();
        let app = tauri::test::mock_builder()
            .manage(crate::infrastructure::platform::app_data_dir::TestDataDir(
                temp.path().to_path_buf(),
            ))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let storage = Arc::new(FileSessionStorage::default());
        let store = Arc::new(SessionStore::new(storage.clone()));
        let session = create_session_internal(
            &store,
            temp.path(),
            "/repo",
            Some(CLAUDE_BACKEND_ID.to_string()),
        )
        .unwrap();
        add_message_internal(
            &store,
            temp.path(),
            &session.id,
            MessageRole::Human,
            "first",
            None,
            None,
        )
        .unwrap();
        add_message_internal(
            &store,
            temp.path(),
            &session.id,
            MessageRole::Agent,
            "first answer",
            None,
            None,
        )
        .unwrap();
        let expected_human = add_message_internal(
            &store,
            temp.path(),
            &session.id,
            MessageRole::Human,
            "second",
            None,
            None,
        )
        .unwrap();
        let target_agent = add_message_internal(
            &store,
            temp.path(),
            &session.id,
            MessageRole::Agent,
            "",
            None,
            None,
        )
        .unwrap();

        storage.reset_message_read_count();
        let started_turn_prompt = prompt_input_for_started_turn(
            Some(app.handle()),
            Some(&store),
            &session.id,
            &target_agent.id,
            "fallback",
            &[],
        );

        assert_eq!(storage.message_read_count(), 1);
        assert_eq!(started_turn_prompt.message_id, expected_human.id);
        assert_eq!(started_turn_prompt.prompt.content, "second");
    }

    #[test]
    fn started_turn_prompt_falls_back_when_target_or_previous_human_is_missing() {
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
        let agent_without_human = add_message_internal(
            &store,
            temp.path(),
            &session.id,
            MessageRole::Agent,
            "",
            None,
            None,
        )
        .unwrap();

        for streaming_message_id in [&agent_without_human.id, "missing-agent"] {
            let started_turn_prompt = prompt_input_for_started_turn(
                Some(app.handle()),
                Some(&store),
                &session.id,
                streaming_message_id,
                "fallback",
                &[],
            );

            assert_eq!(
                started_turn_prompt.message_id,
                fallback_prompt_message_id(streaming_message_id)
            );
            assert_eq!(started_turn_prompt.prompt.content, "fallback");
        }
    }

    #[test]
    fn started_turn_prompt_falls_back_when_app_store_or_lightweight_load_fails() {
        let temp = tempfile::tempdir().unwrap();
        let app = tauri::test::mock_builder()
            .manage(crate::infrastructure::platform::app_data_dir::TestDataDir(
                temp.path().to_path_buf(),
            ))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let store = Arc::new(crate::test_support::build_session_store());
        let fallback_images = vec![ImageAttachment {
            data: "encoded".to_string(),
            media_type: "image/png".to_string(),
        }];

        let missing_app = prompt_input_for_started_turn::<tauri::test::MockRuntime>(
            None,
            Some(&store),
            "session-1",
            "agent-1",
            "fallback",
            &fallback_images,
        );
        assert_started_turn_prompt_matches_fallback(
            missing_app,
            "agent-1",
            "fallback",
            &fallback_images,
        );

        let missing_store = prompt_input_for_started_turn(
            Some(app.handle()),
            None,
            "session-1",
            "agent-2",
            "fallback",
            &fallback_images,
        );
        assert_started_turn_prompt_matches_fallback(
            missing_store,
            "agent-2",
            "fallback",
            &fallback_images,
        );

        let broken_data_dir = tempfile::tempdir().unwrap();
        std::fs::write(broken_data_dir.path().join("sessions"), "not a directory").unwrap();
        let broken_app = tauri::test::mock_builder()
            .manage(crate::infrastructure::platform::app_data_dir::TestDataDir(
                broken_data_dir.path().to_path_buf(),
            ))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let broken_store = Arc::new(SessionStore::new(Arc::new(FileSessionStorage::default())));
        let storage_failure = prompt_input_for_started_turn(
            Some(broken_app.handle()),
            Some(&broken_store),
            "session-1",
            "agent-3",
            "fallback",
            &fallback_images,
        );
        assert_started_turn_prompt_matches_fallback(
            storage_failure,
            "agent-3",
            "fallback",
            &fallback_images,
        );
    }

    #[test]
    fn started_turn_prompt_hydrates_saved_human_like_full_restore() {
        let temp = tempfile::tempdir().unwrap();
        let app = tauri::test::mock_builder()
            .manage(crate::infrastructure::platform::app_data_dir::TestDataDir(
                temp.path().to_path_buf(),
            ))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let storage = Arc::new(FileSessionStorage::default());
        let store = Arc::new(SessionStore::new(storage.clone()));
        let session = create_session_internal(
            &store,
            temp.path(),
            "/repo",
            Some(CLAUDE_BACKEND_ID.to_string()),
        )
        .unwrap();
        let parts = vec![
            MessagePart::Text {
                content: "inspect image".to_string(),
                parent_tool_use_id: None,
            },
            MessagePart::Image {
                data: "iVBORw0KGgo=".to_string(),
                media_type: "image/png".to_string(),
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
            "inspect image",
            Some(parts),
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
        let full_session = store
            .load_full_session_for_restore(temp.path(), &session.id)
            .unwrap()
            .unwrap();
        let full_human = full_session
            .messages
            .iter()
            .find(|message| message.id == human_message.id)
            .unwrap();
        let expected_prompt = PromptInput::from_human_message(full_human);

        storage.reset_message_read_count();
        let started_turn_prompt = prompt_input_for_started_turn(
            Some(app.handle()),
            Some(&store),
            &session.id,
            &agent_message.id,
            "fallback",
            &[],
        );

        assert_eq!(storage.message_read_count(), 1);
        assert_eq!(started_turn_prompt.message_id, human_message.id);
        assert_eq!(started_turn_prompt.prompt, expected_prompt);
    }

    #[tokio::test]
    async fn prepared_send_accepts_already_validated_workflow_step_session() {
        let data_dir = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let mut registry = AgentBackendRegistry::new();
        registry.register(Arc::new(MockModelBackend {
            backend_id: "mock".to_string(),
        }));
        let registry = Arc::new(registry);
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let worktree_path = "/repo".to_string();
        let mut step_session = create_session_internal(
            &session_store,
            data_dir.path(),
            &worktree_path,
            Some("mock".to_string()),
        )
        .unwrap();
        step_session.workflow_step_session = true;
        session_store
            .save_full_session_for_migration_or_restore(data_dir.path(), &step_session)
            .unwrap();
        let parent_session = create_session_internal(
            &session_store,
            data_dir.path(),
            &worktree_path,
            Some("mock".to_string()),
        )
        .unwrap();

        session_store
            .save_full_session_for_migration_or_restore(data_dir.path(), &parent_session)
            .unwrap();

        let result = prepare_send_agent_message_internal(
            None,
            &crate::adaptor::controller::wiring::build_code_usecase(),
            None,
            &session_store,
            &registry,
            &handles,
            data_dir.path(),
            Some(step_session.id.clone()),
            "/different-request-worktree".to_string(),
            "continue completed step".to_string(),
            crate::domain::agent_session::PermissionMode::Edit,
            false,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await;

        let (_response, prepared_turn) = result.unwrap();
        let prepared_turn = prepared_turn.expect("workflow step message starts a turn");
        let prepared_turn = expect_prepared_turn(prepared_turn);
        assert_eq!(prepared_turn.worktree_path, "/repo");
        let saved = session_store
            .load_full_session_for_restore(data_dir.path(), &step_session.id)
            .unwrap()
            .unwrap();
        assert!(
            saved
                .messages
                .iter()
                .any(|message| message.content == "continue completed step"),
            "workflow command validation happens before bridge turn preparation"
        );
    }

    #[tokio::test]
    async fn prepared_send_to_regular_session_does_not_change_workflow_step_runtime() {
        let data_dir = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let mut registry = AgentBackendRegistry::new();
        registry.register(Arc::new(MockModelBackend {
            backend_id: "mock".to_string(),
        }));
        let registry = Arc::new(registry);
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let open_tabs = crate::usecase::agent_session::session::OpenTabRegistry::default();
        let worktree_path = "/repo".to_string();
        let regular_session = create_session_internal(
            &session_store,
            data_dir.path(),
            &worktree_path,
            Some("mock".to_string()),
        )
        .unwrap();
        let mut step_session = create_session_internal(
            &session_store,
            data_dir.path(),
            &worktree_path,
            Some("mock".to_string()),
        )
        .unwrap();
        step_session.workflow_step_session = true;
        session_store
            .save_full_session_for_migration_or_restore(data_dir.path(), &step_session)
            .unwrap();
        handles
            .lock()
            .await
            .insert(step_session.id.clone(), make_test_agent_process());

        let before =
            crate::adaptor::gateway::workflow::build_workflow_state_projection_from_snapshot(
                workflow_state_for_runtime_test(&step_session.id),
                Some(&handles),
                Some(&open_tabs),
            )
            .await;

        let (_response, prepared_turn) = prepare_send_agent_message_internal(
            None,
            &crate::adaptor::controller::wiring::build_code_usecase(),
            None,
            &session_store,
            &registry,
            &handles,
            data_dir.path(),
            Some(regular_session.id),
            worktree_path,
            "regular chat".to_string(),
            crate::domain::agent_session::PermissionMode::Edit,
            false,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

        assert!(prepared_turn.is_some());
        assert!(handles.lock().await.contains_key(&step_session.id));
        let after =
            crate::adaptor::gateway::workflow::build_workflow_state_projection_from_snapshot(
                workflow_state_for_runtime_test(&step_session.id),
                Some(&handles),
                Some(&open_tabs),
            )
            .await;
        assert_eq!(
            before.runtime_states[&step_session.id].runtime_active,
            after.runtime_states[&step_session.id].runtime_active
        );
    }

    #[tokio::test]
    async fn prepare_send_persists_selected_permission_mode_for_new_session() {
        // Spec issues-947: 新規セッション作成時、選択された抽象モードがそのまま
        // ChatSession.permission_mode に保存される（PreparedAgentTurn と乖離しない）。
        // モデル未選択状態は廃止されたため、新規セッションは backend の既定モデル解決を
        // 必要とする。fixed_models を持つ実 backend registry を使う。
        let data_dir = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let registry = make_fixed_model_registry();
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let worktree_path = "/repo".to_string();

        let (response, prepared_turn) = prepare_send_agent_message_internal(
            None,
            &crate::adaptor::controller::wiring::build_code_usecase(),
            None,
            &session_store,
            &registry,
            &handles,
            data_dir.path(),
            None,
            worktree_path.clone(),
            "hi".to_string(),
            crate::domain::agent_session::PermissionMode::Ask,
            false,
            Some(CLAUDE_BACKEND_ID.to_string()),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

        let prepared_turn = prepared_turn.expect("new session should start a turn");
        let prepared_turn = expect_prepared_turn(prepared_turn);
        assert_eq!(prepared_turn.permission_mode, "ask");
        assert_eq!(response.session.permission_mode, "ask");
        let saved = session_store
            .load_full_session_for_restore(data_dir.path(), &response.session.id)
            .unwrap()
            .unwrap();
        assert_eq!(saved.permission_mode, "ask");
    }

    #[tokio::test]
    async fn prepare_send_existing_session_uses_meta_and_append_without_hydrating_body() {
        let data_dir = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let mut registry = AgentBackendRegistry::new();
        registry.register(Arc::new(MockModelBackend {
            backend_id: "mock".to_string(),
        }));
        let registry = Arc::new(registry);
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let session_id = uuid::Uuid::new_v4().to_string();
        let session = ChatSession {
            id: session_id.clone(),
            worktree_path: "/repo".to_string(),
            messages: vec![ChatMessage {
                id: "old-message".to_string(),
                role: MessageRole::Human,
                content: "old body".to_string(),
                thinking: None,
                activities: None,
                parts: None,
                streaming_final_seq: 0,
                timestamp: 1000.0,
                mentions: None,
            }],
            state: crate::usecase::agent_session::session::SessionState::Active,
            created_at: 1000.0,
            updated_at: 1000.0,
            agent_session_id: None,
            context_carry: None,
            permission_mode: "edit".to_string(),
            plan_mode: false,
            selected_model: None,
            permission_profile_id: None,
            backend_id: Some("mock".to_string()),
            workflow_step_session: false,
            workflow_step_context: None,
            context_epoch: None,
        };
        session_store
            .save_full_session_for_migration_or_restore(data_dir.path(), &session)
            .unwrap();
        std::fs::write(
            data_dir
                .path()
                .join("sessions")
                .join(&session_id)
                .join("messages")
                .join("1.json"),
            "{",
        )
        .unwrap();

        let (response, prepared_turn) = prepare_send_agent_message_internal(
            None,
            &crate::adaptor::controller::wiring::build_code_usecase(),
            None,
            &session_store,
            &registry,
            &handles,
            data_dir.path(),
            Some(session_id.clone()),
            "/repo".to_string(),
            "new prompt".to_string(),
            crate::domain::agent_session::PermissionMode::Ask,
            true,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

        assert!(prepared_turn.is_some());
        assert!(response.session.messages.is_empty());
        assert_eq!(response.session.permission_mode, "ask");
        assert!(response.session.plan_mode);
        assert_eq!(response.human_message.content, "new prompt");
        assert!(response.agent_message.is_some());
        let page = session_store
            .get_session_page(data_dir.path(), &session_id, None, 2)
            .unwrap()
            .unwrap();
        assert_eq!(
            page.messages
                .iter()
                .map(|message| message.id.as_str())
                .collect::<Vec<_>>(),
            vec![
                response.human_message.id.as_str(),
                response.agent_message.as_ref().unwrap().id.as_str(),
            ]
        );
    }

    #[tokio::test]
    async fn prepared_turn_carries_codex_backend_for_runtime_dispatch() {
        let data_dir = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let registry = make_fixed_model_registry();
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));

        let (_response, prepared_turn) = prepare_send_agent_message_internal(
            None,
            &crate::adaptor::controller::wiring::build_code_usecase(),
            None,
            &session_store,
            &registry,
            &handles,
            data_dir.path(),
            None,
            "/repo".to_string(),
            "hello codex".to_string(),
            crate::domain::agent_session::PermissionMode::Edit,
            false,
            Some(CODEX_BACKEND_ID.to_string()),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

        let prepared_turn = prepared_turn.expect("new codex session should start a turn");
        let prepared_turn = expect_prepared_turn(prepared_turn);
        assert_eq!(prepared_turn.backend_id, CODEX_BACKEND_ID);
        assert_eq!(prepared_turn.prompt, "hello codex");
    }

    #[tokio::test]
    async fn prepared_turn_carries_system_context_outside_user_prompt() {
        let data_dir = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let registry = make_fixed_model_registry();
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));

        let (_response, prepared_turn) = prepare_send_agent_message_internal(
            None,
            &crate::adaptor::controller::wiring::build_code_usecase(),
            None,
            &session_store,
            &registry,
            &handles,
            data_dir.path(),
            None,
            "/repo".to_string(),
            "hello without stale context".to_string(),
            crate::domain::agent_session::PermissionMode::Edit,
            false,
            Some(CLAUDE_BACKEND_ID.to_string()),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

        let prepared_turn = prepared_turn.expect("new session should start a turn");
        let prepared_turn = expect_prepared_turn(prepared_turn);
        let system_prompt = prepared_turn.system_prompt.expect("system context prompt");

        assert_eq!(prepared_turn.prompt, "hello without stale context");
        assert!(!prepared_turn
            .prompt
            .contains("releash_backend_model_identity"));
        assert!(system_prompt.contains("<releash_backend_model_identity>"));
        assert!(system_prompt.contains("backend_id: claude"));
        assert!(!system_prompt.contains("<releash_mentions>"));
        assert!(!system_prompt.contains("<releash_open_editor_selection>"));
    }

    #[tokio::test]
    async fn prepared_turn_carries_mentions_and_editor_context_in_system_prompt() {
        let data_dir = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let registry = make_fixed_model_registry();
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let mentions = vec![crate::domain::code::MentionReference {
            file_path: "src/mentioned.rs".to_string(),
            start_line: Some(12),
            end_line: Some(18),
        }];
        let editor_context = AgentEditorContext {
            active_editor_path: Some("src/active.rs".to_string()),
            open_editor_paths: vec!["src/active.rs".to_string(), "src/other.rs".to_string()],
            selection: Some(
                crate::infrastructure::agent_session::runtime::AgentEditorSelection {
                    file_path: "src/editor.rs".to_string(),
                    start_line: 3,
                    end_line: 8,
                },
            ),
        };

        let (_response, prepared_turn) = prepare_send_agent_message_internal(
            None,
            &crate::adaptor::controller::wiring::build_code_usecase(),
            None,
            &session_store,
            &registry,
            &handles,
            data_dir.path(),
            None,
            "/repo".to_string(),
            "hello with context".to_string(),
            crate::domain::agent_session::PermissionMode::Edit,
            false,
            Some(CLAUDE_BACKEND_ID.to_string()),
            None,
            None,
            Some(mentions),
            Some(editor_context),
            None,
            None,
        )
        .await
        .unwrap();

        let prepared_turn = prepared_turn.expect("new session should start a turn");
        let prepared_turn = expect_prepared_turn(prepared_turn);
        let system_prompt = prepared_turn.system_prompt.expect("system context prompt");

        assert!(system_prompt.contains("<releash_mentions>"));
        assert!(system_prompt.contains("src/mentioned.rs:12-18"));
        assert!(system_prompt.contains("<releash_open_editor_selection>"));
        assert!(system_prompt.contains("src/active.rs"));
        assert!(system_prompt.contains("src/other.rs"));
        assert!(system_prompt.contains("src/editor.rs"));
        assert!(system_prompt.contains("startLine"));
        assert!(system_prompt.contains("endLine"));
    }

    #[tokio::test]
    async fn prepared_turn_uses_injected_branch_diff_context_without_app_state() {
        let data_dir = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let registry = make_fixed_model_registry();
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let branch_diff_context: Arc<dyn BranchDiffContextPort> = Arc::new(FakeBranchDiffContext);

        let (_response, prepared_turn) = prepare_send_agent_message_internal(
            None,
            &crate::adaptor::controller::wiring::build_code_usecase(),
            Some(branch_diff_context),
            &session_store,
            &registry,
            &handles,
            data_dir.path(),
            None,
            "/repo".to_string(),
            "review diff".to_string(),
            crate::domain::agent_session::PermissionMode::Edit,
            false,
            Some(CLAUDE_BACKEND_ID.to_string()),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

        let prepared_turn = expect_prepared_turn(prepared_turn.expect("prepared turn"));
        let system_prompt = prepared_turn.system_prompt.expect("system context prompt");
        assert!(system_prompt.contains("<releash_diff_review_snapshot>"));
        assert!(system_prompt.contains("base_branch: main"));
        assert!(system_prompt.contains("- modified src/lib.rs (+3 -1)"));
    }

    #[tokio::test]
    async fn initializing_idle_runtime_prepares_first_turn_instead_of_queueing() {
        let data_dir = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let mut registry = AgentBackendRegistry::new();
        registry.register(Arc::new(MockModelBackend {
            backend_id: "mock".to_string(),
        }));
        let registry = Arc::new(registry);
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let worktree_path = "/repo".to_string();
        let session = create_session_internal(
            &session_store,
            data_dir.path(),
            &worktree_path,
            Some("mock".to_string()),
        )
        .unwrap();
        let mut proc = make_test_agent_process();
        proc.state = BridgeState::Initializing;
        proc.turn_phase = TurnPhase::Idle;
        handles.lock().await.insert(session.id.clone(), proc);

        let (response, prepared_input) = prepare_send_agent_message_internal(
            None,
            &crate::adaptor::controller::wiring::build_code_usecase(),
            None,
            &session_store,
            &registry,
            &handles,
            data_dir.path(),
            Some(session.id.clone()),
            worktree_path,
            "first restored turn".to_string(),
            crate::domain::agent_session::PermissionMode::Edit,
            false,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

        assert!(response.queued_turn.is_none());
        assert!(response.agent_message.is_some());
        assert!(response.pending_queue.is_empty());
        let prepared_turn = expect_prepared_turn(prepared_input.expect("first turn should start"));
        assert_eq!(prepared_turn.session_id, session.id);
        assert_eq!(prepared_turn.prompt, "first restored turn");
        let mut proc = handles.lock().await.remove(&session.id).unwrap();
        assert!(proc.pending_messages.is_empty());
        let _ = proc.child.kill().await;
    }

    #[tokio::test]
    async fn busy_send_uses_active_turn_steer_when_backend_is_ready() {
        let data_dir = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let mut registry = AgentBackendRegistry::new();
        registry.register(Arc::new(MockSteeringBackend {
            backend_id: "steer".to_string(),
        }));
        let registry = Arc::new(registry);
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let worktree_path = "/repo".to_string();
        let session = create_session_internal(
            &session_store,
            data_dir.path(),
            &worktree_path,
            Some("steer".to_string()),
        )
        .unwrap();
        let mut proc = make_test_agent_process();
        proc.state = BridgeState::Streaming;
        proc.turn_phase = TurnPhase::Streaming;
        handles.lock().await.insert(session.id.clone(), proc);

        let (response, prepared_input) = prepare_send_agent_message_internal(
            None,
            &crate::adaptor::controller::wiring::build_code_usecase(),
            None,
            &session_store,
            &registry,
            &handles,
            data_dir.path(),
            Some(session.id.clone()),
            worktree_path.clone(),
            "/status".to_string(),
            crate::domain::agent_session::PermissionMode::Edit,
            false,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

        assert!(response.agent_message.is_none());
        assert!(response.queued_turn.is_none());
        assert!(response.pending_queue.is_empty());
        let steer = expect_prepared_steer(prepared_input.expect("busy send should steer"));
        assert_eq!(steer.session_id, session.id);
        assert_eq!(steer.backend_id, "steer");
        assert_eq!(steer.prompt, "/status");
        assert_eq!(steer.steering_message_id, response.human_message.id);
        let pending_count = handles
            .lock()
            .await
            .get(&session.id)
            .map(|proc| proc.pending_messages.len())
            .unwrap_or_default();
        assert_eq!(pending_count, 0);
    }

    #[tokio::test]
    async fn workflow_step_send_with_stopped_runtime_prepares_single_resume_turn() {
        let data_dir = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let mut registry = AgentBackendRegistry::new();
        registry.register(Arc::new(MockModelBackend {
            backend_id: "mock".to_string(),
        }));
        let registry = Arc::new(registry);
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let worktree_path = "/repo".to_string();
        let mut step_session = create_session_internal(
            &session_store,
            data_dir.path(),
            &worktree_path,
            Some("mock".to_string()),
        )
        .unwrap();
        step_session.workflow_step_session = true;
        step_session.agent_session_id = Some("sdk-session".to_string());
        session_store
            .save_full_session_for_migration_or_restore(data_dir.path(), &step_session)
            .unwrap();

        let (_response, prepared_turn) = prepare_send_agent_message_internal(
            None,
            &crate::adaptor::controller::wiring::build_code_usecase(),
            None,
            &session_store,
            &registry,
            &handles,
            data_dir.path(),
            Some(step_session.id.clone()),
            worktree_path.clone(),
            "resume step".to_string(),
            crate::domain::agent_session::PermissionMode::Edit,
            false,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

        let prepared_turn = prepared_turn.expect("stopped workflow step should resume on send");
        let prepared_turn = expect_prepared_turn(prepared_turn);
        assert_eq!(prepared_turn.session_id, step_session.id);
        assert_eq!(prepared_turn.worktree_path, worktree_path);
        assert_eq!(prepared_turn.prompt, "resume step");
        assert!(
            handles.lock().await.is_empty(),
            "preparation must not leave a half-started runtime before turn start"
        );
    }

    #[tokio::test]
    async fn turn_start_requires_existing_session_meta() {
        let data_dir = tempfile::tempdir().unwrap();
        let app = tauri::test::mock_builder()
            .manage(crate::infrastructure::platform::app_data_dir::TestDataDir(
                data_dir.path().to_path_buf(),
            ))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));

        let err = start_agent_turn(
            app.handle(),
            None,
            &handles,
            &session_store,
            "missing-turn",
            "/repo",
            "edit",
            false,
            "hello",
            None,
            "agent-msg-1",
            &[],
        )
        .await
        .unwrap_err();
        assert_eq!(err, "Session not found: missing-turn");

        let err = start_agent_turn_locked(
            app.handle(),
            None,
            &handles,
            &session_store,
            "missing-locked-turn",
            "/repo",
            "edit",
            false,
            "hello",
            None,
            "agent-msg-2",
            &[],
        )
        .await
        .unwrap_err();
        assert_eq!(err, "Session not found: missing-locked-turn");
        assert!(handles.lock().await.is_empty());
    }

    #[tokio::test]
    async fn stopped_workflow_step_turn_start_spawns_resume_runtime_once() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let session_id = "stopped-step".to_string();
        let spawn_count = Arc::new(AtomicUsize::new(0));

        start_agent_turn_with_runtime_spawner(
            None::<&tauri::AppHandle>,
            None,
            None,
            &handles,
            &session_id,
            "edit",
            "resume step",
            "agent-msg-1",
            &[],
            {
                let handles = Arc::clone(&handles);
                let session_id = session_id.clone();
                let spawn_count = Arc::clone(&spawn_count);
                move || async move {
                    spawn_count.fetch_add(1, Ordering::SeqCst);
                    handles
                        .lock()
                        .await
                        .insert(session_id, make_test_agent_process());
                    Ok(())
                }
            },
        )
        .await
        .unwrap();

        assert_eq!(spawn_count.load(Ordering::SeqCst), 1);
        let map = handles.lock().await;
        let proc = map.get(&session_id).expect("runtime was started");
        assert_eq!(proc.state, BridgeState::Streaming);
        assert_eq!(proc.turn_phase, TurnPhase::Streaming);
        assert_eq!(proc.streaming_message_id.as_deref(), Some("agent-msg-1"));
    }

    #[tokio::test]
    async fn turn_start_transition_carries_projected_active_session_state() {
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let session_id = "projected-start".to_string();

        let projected_session_state = start_agent_turn_with_runtime_spawner(
            None::<&tauri::AppHandle>,
            None,
            None,
            &handles,
            &session_id,
            "edit",
            "start turn",
            "agent-msg-1",
            &[],
            {
                let handles = Arc::clone(&handles);
                let session_id = session_id.clone();
                move || async move {
                    handles
                        .lock()
                        .await
                        .insert(session_id, make_test_agent_process());
                    Ok(())
                }
            },
        )
        .await
        .unwrap();

        assert_eq!(
            projected_session_state,
            Some(crate::usecase::agent_session::session::SessionState::Active)
        );
        let mut proc = handles.lock().await.remove(&session_id).unwrap();
        let _ = proc.child.kill().await;
    }

    #[tokio::test]
    async fn running_workflow_step_turn_start_reuses_existing_runtime_without_spawn() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let session_id = "running-step".to_string();
        handles
            .lock()
            .await
            .insert(session_id.clone(), make_test_agent_process());
        let spawn_count = Arc::new(AtomicUsize::new(0));

        start_agent_turn_with_runtime_spawner(
            None::<&tauri::AppHandle>,
            None,
            None,
            &handles,
            &session_id,
            "edit",
            "continue step",
            "agent-msg-2",
            &[],
            {
                let spawn_count = Arc::clone(&spawn_count);
                move || async move {
                    spawn_count.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }
            },
        )
        .await
        .unwrap();

        assert_eq!(spawn_count.load(Ordering::SeqCst), 0);
        assert_eq!(handles.lock().await.len(), 1);
    }

    #[tokio::test]
    async fn stale_system_prompt_ready_runtime_is_removed_before_reuse() {
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let session_id = "stale-system-prompt".to_string();
        let mut proc = make_test_agent_process();
        proc.system_prompt_fingerprint = runtime_system_prompt_fingerprint(Some("old context"));
        handles.lock().await.insert(session_id.clone(), proc);

        replace_ready_runtime_if_system_prompt_changed::<tauri::test::MockRuntime>(
            None,
            &handles,
            &session_id,
            runtime_system_prompt_fingerprint(Some("new context")),
        )
        .await
        .unwrap();

        assert!(
            handles.lock().await.is_empty(),
            "stale system prompt runtime must be removed so init carries current context"
        );
    }

    #[tokio::test]
    async fn matching_system_prompt_fingerprint_runtime_is_preserved() {
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let session_id = "matching-system-prompt".to_string();
        let fingerprint = runtime_system_prompt_fingerprint(Some("same context"));
        let mut proc = make_test_agent_process();
        proc.system_prompt_fingerprint = fingerprint.clone();
        handles.lock().await.insert(session_id.clone(), proc);

        let preserved_pending = replace_ready_runtime_if_system_prompt_changed::<
            tauri::test::MockRuntime,
        >(None, &handles, &session_id, fingerprint)
        .await
        .unwrap();

        assert!(preserved_pending.is_empty());
        let mut proc = handles.lock().await.remove(&session_id).unwrap();
        let _ = proc.child.kill().await;
    }

    #[tokio::test]
    async fn in_flight_runtime_is_not_removed_for_system_prompt_change() {
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let session_id = "in-flight-system-prompt".to_string();
        let mut proc = make_test_agent_process();
        proc.state = BridgeState::Streaming;
        proc.turn_phase = TurnPhase::Streaming;
        proc.system_prompt_fingerprint = runtime_system_prompt_fingerprint(Some("old context"));
        handles.lock().await.insert(session_id.clone(), proc);

        let preserved_pending =
            replace_ready_runtime_if_system_prompt_changed::<tauri::test::MockRuntime>(
                None,
                &handles,
                &session_id,
                runtime_system_prompt_fingerprint(Some("new context")),
            )
            .await
            .unwrap();

        assert!(preserved_pending.is_empty());
        let mut proc = handles.lock().await.remove(&session_id).unwrap();
        let _ = proc.child.kill().await;
    }

    #[tokio::test]
    async fn stale_initializing_idle_runtime_is_removed_before_first_turn() {
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let session_id = "stale-initializing-system-prompt".to_string();
        let mut proc = make_test_agent_process();
        proc.state = BridgeState::Initializing;
        proc.turn_phase = TurnPhase::Idle;
        proc.system_prompt_fingerprint = runtime_system_prompt_fingerprint(Some("old context"));
        handles.lock().await.insert(session_id.clone(), proc);

        replace_ready_runtime_if_system_prompt_changed::<tauri::test::MockRuntime>(
            None,
            &handles,
            &session_id,
            runtime_system_prompt_fingerprint(Some("new context")),
        )
        .await
        .unwrap();

        assert!(handles.lock().await.is_empty());
    }

    #[tokio::test]
    async fn stale_ready_idle_runtime_with_pending_preserves_queue_for_replacement() {
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let session_id = "stale-system-prompt-with-pending".to_string();
        let mut proc = make_test_agent_process();
        proc.system_prompt_fingerprint = runtime_system_prompt_fingerprint(Some("old context"));
        proc.pending_messages
            .push_back(pending_message_for_test("queued-1"));
        handles.lock().await.insert(session_id.clone(), proc);

        let preserved_pending =
            replace_ready_runtime_if_system_prompt_changed::<tauri::test::MockRuntime>(
                None,
                &handles,
                &session_id,
                runtime_system_prompt_fingerprint(Some("new context")),
            )
            .await
            .unwrap();

        assert!(handles.lock().await.is_empty());
        assert_eq!(preserved_pending.len(), 1);
        assert_eq!(preserved_pending[0].id, "queued-1");

        handles
            .lock()
            .await
            .insert(session_id.clone(), make_test_agent_process());
        prepend_pending_messages_to_runtime(&handles, &session_id, preserved_pending).await;
        let mut proc = handles.lock().await.remove(&session_id).unwrap();
        assert_eq!(proc.pending_messages.len(), 1);
        assert_eq!(proc.pending_messages.front().unwrap().id, "queued-1");
        let _ = proc.child.kill().await;
    }

    #[tokio::test]
    async fn workflow_step_turn_start_holds_session_runtime_lock_until_message_write() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let session_id = "locked-turn-start".to_string();
        let spawn_count = Arc::new(AtomicUsize::new(0));
        let guard = acquire_session_runtime_lock(&session_id).await;

        let start = {
            let handles = Arc::clone(&handles);
            let session_id = session_id.clone();
            let spawn_count = Arc::clone(&spawn_count);
            tokio::spawn(async move {
                start_agent_turn_with_runtime_spawner(
                    None::<&tauri::AppHandle>,
                    None,
                    None,
                    &handles,
                    &session_id,
                    "edit",
                    "resume step",
                    "agent-msg-locked",
                    &[],
                    {
                        let handles = Arc::clone(&handles);
                        let session_id = session_id.clone();
                        let spawn_count = Arc::clone(&spawn_count);
                        move || async move {
                            spawn_count.fetch_add(1, Ordering::SeqCst);
                            handles
                                .lock()
                                .await
                                .insert(session_id, make_test_agent_process());
                            Ok(())
                        }
                    },
                )
                .await
            })
        };

        tokio::time::sleep(Duration::from_millis(30)).await;
        assert_eq!(spawn_count.load(Ordering::SeqCst), 0);
        assert!(handles.lock().await.is_empty());

        drop(guard);
        start.await.unwrap().unwrap();

        assert_eq!(spawn_count.load(Ordering::SeqCst), 1);
        let map = handles.lock().await;
        let proc = map.get(&session_id).expect("runtime was started");
        assert_eq!(
            proc.streaming_message_id.as_deref(),
            Some("agent-msg-locked")
        );
    }

    #[tokio::test]
    async fn concurrent_workflow_step_turn_start_spawns_runtime_at_most_once() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let session_id = "concurrent-step".to_string();
        let spawn_count = Arc::new(AtomicUsize::new(0));

        let start_one = {
            let handles = Arc::clone(&handles);
            let session_id = session_id.clone();
            let spawn_count = Arc::clone(&spawn_count);
            async move {
                start_agent_turn_with_runtime_spawner(
                    None::<&tauri::AppHandle>,
                    None,
                    None,
                    &handles,
                    &session_id,
                    "edit",
                    "first",
                    "agent-msg-1",
                    &[],
                    {
                        let handles = Arc::clone(&handles);
                        let session_id = session_id.clone();
                        let spawn_count = Arc::clone(&spawn_count);
                        move || async move {
                            spawn_count.fetch_add(1, Ordering::SeqCst);
                            handles
                                .lock()
                                .await
                                .insert(session_id, make_test_agent_process());
                            Ok(())
                        }
                    },
                )
                .await
            }
        };
        let start_two = {
            let handles = Arc::clone(&handles);
            let session_id = session_id.clone();
            let spawn_count = Arc::clone(&spawn_count);
            async move {
                start_agent_turn_with_runtime_spawner(
                    None::<&tauri::AppHandle>,
                    None,
                    None,
                    &handles,
                    &session_id,
                    "edit",
                    "second",
                    "agent-msg-2",
                    &[],
                    {
                        let handles = Arc::clone(&handles);
                        let session_id = session_id.clone();
                        let spawn_count = Arc::clone(&spawn_count);
                        move || async move {
                            spawn_count.fetch_add(1, Ordering::SeqCst);
                            handles
                                .lock()
                                .await
                                .insert(session_id, make_test_agent_process());
                            Ok(())
                        }
                    },
                )
                .await
            }
        };

        let (first, second) = tokio::join!(start_one, start_two);

        first.unwrap();
        second.unwrap();
        assert_eq!(spawn_count.load(Ordering::SeqCst), 1);
        assert_eq!(handles.lock().await.len(), 1);
    }

    #[tokio::test]
    async fn get_session_returns_registry_models_without_process() {
        let temp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let session = create_session_internal(
            &session_store,
            temp.path(),
            "/repo",
            Some(CLAUDE_BACKEND_ID.to_string()),
        )
        .unwrap();

        let mut cfg = crate::adaptor::gateway::app_config::ReleashConfig::default();
        cfg.agents.claude.models = vec!["mock-model".to_string()];
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let config = Arc::new(crate::adaptor::gateway::app_config::AppConfig::new(
            cfg,
            tmp.path().to_path_buf(),
        ));

        let mut registry = AgentBackendRegistry::new();
        registry.register(Arc::new(MockModelBackend {
            backend_id: CLAUDE_BACKEND_ID.to_string(),
        }));
        registry.set_config(config);
        let registry = Arc::new(registry);
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));

        let response = get_session_internal_with_data_dir(
            &session_store,
            &handles,
            Some(&registry),
            temp.path(),
            &session.id,
        )
        .await
        .unwrap()
        .unwrap();

        assert_eq!(response.available_models.len(), 1);
        assert_eq!(response.available_models[0].model_id, "mock-model");
    }

    #[tokio::test]
    async fn set_session_backend_updates_unstarted_session_and_models() {
        let temp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let mut session = create_session_internal(
            &session_store,
            temp.path(),
            "/repo",
            Some(CLAUDE_BACKEND_ID.to_string()),
        )
        .unwrap();
        session.selected_model = Some("old-model".to_string());
        session_store
            .save_full_session_for_migration_or_restore(temp.path(), &session)
            .unwrap();

        let mut cfg = crate::adaptor::gateway::app_config::ReleashConfig::default();
        cfg.agents.claude.models = vec!["a-model".to_string()];
        cfg.agents.codex.models = vec!["b-model".to_string()];
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let config = Arc::new(crate::adaptor::gateway::app_config::AppConfig::new(
            cfg,
            tmp.path().to_path_buf(),
        ));

        let mut registry = AgentBackendRegistry::new();
        registry.register(Arc::new(MockModelBackend {
            backend_id: CLAUDE_BACKEND_ID.to_string(),
        }));
        registry.register(Arc::new(MockModelBackend {
            backend_id: CODEX_BACKEND_ID.to_string(),
        }));
        registry.set_config(config);
        let registry = Arc::new(registry);
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));

        let response = set_session_backend_internal(
            &session_store,
            &registry,
            &handles,
            temp.path(),
            &session.id,
            CODEX_BACKEND_ID.to_string(),
        )
        .await
        .unwrap();

        assert_eq!(
            response.session.backend_id,
            Some(CODEX_BACKEND_ID.to_string())
        );
        // backend 切替後は新 backend の既定モデル（一覧先頭）へ解決される。
        assert_eq!(
            response.session.selected_model,
            Some("codex:b-model".to_string())
        );
        assert_eq!(response.available_models.len(), 1);
        assert_eq!(response.available_models[0].model_id, "b-model");
    }

    #[tokio::test]
    async fn set_session_backend_rejects_session_with_messages() {
        let temp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let session = create_session_internal(
            &session_store,
            temp.path(),
            "/repo",
            Some("mock-a".to_string()),
        )
        .unwrap();
        add_message_internal(
            &session_store,
            temp.path(),
            &session.id,
            MessageRole::Human,
            "hello",
            None,
            None,
        )
        .unwrap();
        let mut registry = AgentBackendRegistry::new();
        registry.register(Arc::new(MockModelBackend {
            backend_id: "mock-a".to_string(),
        }));
        registry.register(Arc::new(MockModelBackend {
            backend_id: "mock-b".to_string(),
        }));
        let registry = Arc::new(registry);
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));

        let result = set_session_backend_internal(
            &session_store,
            &registry,
            &handles,
            temp.path(),
            &session.id,
            "mock-b".to_string(),
        )
        .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn set_session_backend_rejects_session_with_agent_session_id() {
        let temp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let mut session = create_session_internal(
            &session_store,
            temp.path(),
            "/repo",
            Some("mock-a".to_string()),
        )
        .unwrap();
        session.agent_session_id = Some("sdk-session".to_string());
        session_store
            .save_full_session_for_migration_or_restore(temp.path(), &session)
            .unwrap();
        let mut registry = AgentBackendRegistry::new();
        registry.register(Arc::new(MockModelBackend {
            backend_id: "mock-a".to_string(),
        }));
        registry.register(Arc::new(MockModelBackend {
            backend_id: "mock-b".to_string(),
        }));
        let registry = Arc::new(registry);
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));

        let result = set_session_backend_internal(
            &session_store,
            &registry,
            &handles,
            temp.path(),
            &session.id,
            "mock-b".to_string(),
        )
        .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn set_session_backend_rejects_invalid_backend_id() {
        let temp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let session = create_session_internal(
            &session_store,
            temp.path(),
            "/repo",
            Some("mock-a".to_string()),
        )
        .unwrap();
        let mut registry = AgentBackendRegistry::new();
        registry.register(Arc::new(MockModelBackend {
            backend_id: "mock-a".to_string(),
        }));
        let registry = Arc::new(registry);
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));

        let result = set_session_backend_internal(
            &session_store,
            &registry,
            &handles,
            temp.path(),
            &session.id,
            "missing".to_string(),
        )
        .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn approval_chat_adjustment_send_path_keeps_session_state() {
        let worktree = tempfile::tempdir().unwrap();
        let worktree_path = worktree.path().to_string_lossy().to_string();
        let engine = Arc::new(TestRuntimeKernel::new_for_test());
        let data_dir = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let session = create_session_internal(
            &session_store,
            data_dir.path(),
            &worktree_path,
            Some("mock-a".to_string()),
        )
        .unwrap();
        add_message_internal(
            &session_store,
            data_dir.path(),
            &session.id,
            MessageRole::Agent,
            &approved_fix_policy_output("Old policy.", "code_review_parallel"),
            None,
            None,
        )
        .unwrap();
        let before = engine
            .insert_test_approval_execution(
                &worktree_path,
                &session.id,
                WorkflowExecutionState::WaitingApproval,
            )
            .await;

        let mut registry = AgentBackendRegistry::new();
        registry.register(Arc::new(MockModelBackend {
            backend_id: "mock-a".to_string(),
        }));
        let registry = Arc::new(registry);
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let mut command = test_echo_command();
        let mut child = command
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        handles.lock().await.insert(
            session.id.clone(),
            AgentProcess {
                stdin: Arc::new(Mutex::new(stdin)),
                backend_id: "mock-a".to_string(),
                state: BridgeState::Ready,
                turn_phase: TurnPhase::Idle,
                sdk_session_id: None,
                system_prompt_fingerprint: None,
                context_carry_on_ready: None,
                child,
                generation_id: 0,
                #[cfg(unix)]
                pgid: None,
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
                current_permission_mode: "edit".to_string(),
                available_models: Vec::new(),
                selected_model: None,
                stale_timeout: std::time::Duration::from_secs(180),
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
            },
        );

        let (response, prepared_turn) = prepare_send_agent_message_internal(
            None,
            &crate::adaptor::controller::wiring::build_code_usecase(),
            None,
            &session_store,
            &registry,
            &handles,
            data_dir.path(),
            Some(session.id.clone()),
            worktree_path.clone(),
            "Narrow the policy to reviewed findings.".to_string(),
            crate::domain::agent_session::PermissionMode::Edit,
            false,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

        let agent = response.agent_message.unwrap();
        let prepared_turn = prepared_turn.unwrap();
        let prepared_turn = expect_prepared_turn(prepared_turn);
        assert_eq!(prepared_turn.session_id, session.id);
        assert_eq!(
            prepared_turn.prompt,
            "Narrow the policy to reviewed findings."
        );
        {
            let mut map = handles.lock().await;
            let proc = map.get_mut(&session.id).unwrap();
            proc.streaming_parts = vec![MessagePart::Text {
                content: approved_fix_policy_output(
                    "Latest adjusted policy.",
                    "code_review_parallel",
                ),
                parent_tool_use_id: None,
            }];
        }
        {
            let mut saved = session_store
                .load_full_session_for_restore(data_dir.path(), &session.id)
                .unwrap()
                .unwrap();
            let latest_policy =
                approved_fix_policy_output("Latest adjusted policy.", "code_review_parallel");
            let msg = saved
                .messages
                .iter_mut()
                .find(|msg| msg.id == agent.id)
                .unwrap();
            msg.content = latest_policy.clone();
            msg.parts = Some(vec![MessagePart::Text {
                content: latest_policy,
                parent_tool_use_id: None,
            }]);
            session_store
                .save_full_session_for_migration_or_restore(data_dir.path(), &saved)
                .unwrap();
        }

        assert_eq!(response.human_message.role, MessageRole::Human);
        assert_eq!(agent.role, MessageRole::Agent);
        let after_send = engine.get_state(&worktree_path).await.unwrap();
        assert_eq!(after_send.execution_id, before.execution_id);
        assert_eq!(after_send.current_step_name, before.current_step_name);
        assert_eq!(
            after_send.current_session_id.as_deref(),
            Some(session.id.as_str())
        );
        assert_eq!(after_send.state, WorkflowExecutionState::WaitingApproval);

        let saved = session_store
            .load_full_session_for_restore(data_dir.path(), &session.id)
            .unwrap()
            .unwrap();
        let latest_agent = saved
            .messages
            .iter()
            .rev()
            .find(|msg| msg.role == MessageRole::Agent)
            .unwrap();
        // [08] このテストの責務は「send path で session が破壊されず維持されること」のみに限定する。
        // typed structured output の contract 検証（typed な step_outputs 更新の保証）は
        // SubmitOutput を経由する CLI/API 経路でしか発生しないため、本テストでは保証しない。
        // contract 検証経路の回帰テストは domain contract の単体テスト群および
        // SubmitOutput の CLI/API 経路テストで別途カバーする。
        // ここでは「session の最新 Agent メッセージが上書きされて存在すること」のみ確認する。
        assert_eq!(latest_agent.id, agent.id);

        let removed_proc = handles.lock().await.remove(&session.id);
        if let Some(mut proc) = removed_proc {
            let _ = proc.child.kill().await;
        }
    }

    #[test]
    fn ensure_session_backend_selected_saves_default_for_missing_backend_id() {
        let temp = tempfile::tempdir().unwrap();
        let session_store = crate::test_support::build_session_store();
        let session = create_session_internal(&session_store, temp.path(), "/repo", None).unwrap();
        let mut registry = AgentBackendRegistry::new();
        registry.register(Arc::new(MockModelBackend {
            backend_id: "mock-default".to_string(),
        }));

        let updated =
            ensure_session_backend_selected(&session_store, &registry, temp.path(), session)
                .unwrap();

        assert_eq!(updated.backend_id, Some("mock-default".to_string()));
        let persisted = session_store
            .load_full_session_for_restore(temp.path(), &updated.id)
            .unwrap()
            .unwrap();
        assert_eq!(persisted.backend_id, Some("mock-default".to_string()));
    }

    /// spec issues-1023: workflow step として起動された chat session は free chat
    /// tab bar 上に同格に並ばないため、`init_agent_sessions` の active 候補からも
    /// 除外される。本テストは候補選択 helper を 3 シナリオで検証する:
    /// - 先頭が workflow step でも active にならない（free chat があればそれが active）
    /// - 全てが workflow step の場合は active は None
    /// - 通常 chat のみのときは先頭が active になる
    #[test]
    fn pick_initial_active_session_candidate_excludes_workflow_step_sessions() {
        fn make(id: &str, workflow_step: bool) -> SessionSummary {
            SessionSummary {
                id: id.to_string(),
                worktree_path: "/repo".to_string(),
                state: crate::usecase::agent_session::session::SessionState::Idle,
                created_at: 1.0,
                updated_at: 1.0,
                first_message: String::new(),
                message_count: 0,
                agent_session_id: None,
                context_carry: None,
                permission_mode: "edit".to_string(),
                plan_mode: false,
                permission_profile_id: None,
                backend_id: Some("claude".to_string()),
                workflow_step_session: workflow_step,
                workflow_step_context: None,
            }
        }

        // 先頭が workflow step だが後ろに free chat がある: free chat が active になる
        let sessions = vec![make("step-1", true), make("chat-1", false)];
        let picked = pick_initial_active_session_candidate(&sessions);
        assert_eq!(picked.map(|s| s.id.as_str()), Some("chat-1"));

        // 全て workflow step: active 候補 None
        let only_steps = vec![make("step-1", true), make("step-2", true)];
        assert!(pick_initial_active_session_candidate(&only_steps).is_none());

        // 通常 chat のみ: 先頭が active
        let only_chats = vec![make("chat-1", false), make("chat-2", false)];
        assert_eq!(
            pick_initial_active_session_candidate(&only_chats).map(|s| s.id.as_str()),
            Some("chat-1")
        );

        // 空: None
        assert!(pick_initial_active_session_candidate(&[]).is_none());
    }

    #[tokio::test]
    async fn init_agent_sessions_returns_active_latest_page_without_starting_processes() {
        let temp = tempfile::tempdir().unwrap();
        let app = tauri::test::mock_builder()
            .manage(crate::infrastructure::platform::app_data_dir::TestDataDir(
                temp.path().to_path_buf(),
            ))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let session = create_session_internal(
            &session_store,
            temp.path(),
            "/repo",
            Some(CLAUDE_BACKEND_ID.to_string()),
        )
        .unwrap();
        add_message_internal(
            &session_store,
            temp.path(),
            &session.id,
            MessageRole::Human,
            "hello",
            None,
            None,
        )
        .unwrap();

        let registry = make_fixed_model_registry();
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let open_tabs =
            Arc::new(crate::usecase::agent_session::session::OpenTabRegistry::default());

        let response = init_agent_sessions_internal(
            app.handle(),
            &session_store,
            &registry,
            &handles,
            &open_tabs,
            "/repo".to_string(),
        )
        .await
        .unwrap();

        assert_eq!(response.sessions.len(), 1);
        let active = response
            .active_session
            .expect("active shell should be returned");
        assert_eq!(active.session.id, session.id);
        assert_eq!(active.session.messages.len(), 1);
        assert_eq!(active.session.messages[0].content, "hello");
        assert_eq!(
            active.initial_page,
            Some(InitialSessionPage {
                next_cursor: None,
                has_more: false,
                total_count: 1,
            })
        );
        assert!(handles.lock().await.is_empty());
    }

    #[tokio::test]
    async fn pending_turn_requeues_when_system_context_persist_fails() {
        let temp = tempfile::tempdir().unwrap();
        let app = tauri::test::mock_builder()
            .manage(crate::infrastructure::platform::app_data_dir::TestDataDir(
                temp.path().to_path_buf(),
            ))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let session = create_session_internal(
            &session_store,
            temp.path(),
            "/repo",
            Some(CLAUDE_BACKEND_ID.to_string()),
        )
        .unwrap();
        let private_context_path = temp
            .path()
            .join("sessions")
            .join(&session.id)
            .join("private_context.json");
        let _ = std::fs::remove_file(&private_context_path);
        std::fs::create_dir_all(&private_context_path).unwrap();
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        handles
            .lock()
            .await
            .insert(session.id.clone(), make_test_agent_process());

        start_pending_message_turn(
            app.handle(),
            None,
            &handles,
            &session_store,
            &session.id,
            pending_message_for_test("queued-context-error"),
        )
        .await;

        let saved = session_store
            .load_full_session_for_restore(temp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert!(
            saved.messages.is_empty(),
            "pending turn messages must not be consumed when system context persist fails"
        );
        let mut proc = handles.lock().await.remove(&session.id).unwrap();
        assert_eq!(proc.pending_messages.len(), 1);
        assert_eq!(
            proc.pending_messages.front().unwrap().id,
            "queued-context-error"
        );
        let _ = proc.child.kill().await;
    }

    #[tokio::test]
    async fn pending_messages_are_consumed_fifo_without_overwriting_later_turns() {
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let mut proc = make_test_agent_process();
        proc.pending_messages.push_back(PendingMessage {
            id: "queued-1".to_string(),
            content: "first".to_string(),
            created_at: 1.0,
            client_sent_at_ms: None,
            request_received_at_ms: None,
            permission_mode: "edit".to_string(),
            plan_mode: false,
            images: Vec::new(),
            worktree_path: "/repo".to_string(),
            mentions: Vec::new(),
            editor_context: None,
            existing_human_message_id: None,
            existing_agent_message_id: None,
        });
        proc.pending_messages.push_back(PendingMessage {
            id: "queued-2".to_string(),
            content: "second".to_string(),
            created_at: 2.0,
            client_sent_at_ms: None,
            request_received_at_ms: None,
            permission_mode: "ask".to_string(),
            plan_mode: false,
            images: Vec::new(),
            worktree_path: "/repo".to_string(),
            mentions: Vec::new(),
            editor_context: None,
            existing_human_message_id: None,
            existing_agent_message_id: None,
        });
        handles.lock().await.insert("queued".to_string(), proc);

        let first = take_pending_message(&handles, "queued").await.unwrap();
        assert_eq!(first.content, "first");
        assert_eq!(first.permission_mode, "edit");
        assert!(agent_session_has_pending_message(&handles, "queued").await);

        clear_pending_turn_starting("queued").await;
        let second = take_pending_message(&handles, "queued").await.unwrap();
        assert_eq!(second.content, "second");
        assert_eq!(second.permission_mode, "ask");
        assert!(!agent_session_has_pending_message(&handles, "queued").await);

        clear_pending_turn_starting("queued").await;
    }

    #[tokio::test]
    async fn cancel_agent_queued_turn_removes_only_requested_pending_turn() {
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let mut proc = make_test_agent_process();
        proc.pending_messages.push_back(PendingMessage {
            id: "keep".to_string(),
            content: "first".to_string(),
            created_at: 1.0,
            client_sent_at_ms: None,
            request_received_at_ms: None,
            permission_mode: "edit".to_string(),
            plan_mode: false,
            images: Vec::new(),
            worktree_path: "/repo".to_string(),
            mentions: Vec::new(),
            editor_context: None,
            existing_human_message_id: None,
            existing_agent_message_id: None,
        });
        proc.pending_messages.push_back(PendingMessage {
            id: "drop".to_string(),
            content: "second".to_string(),
            created_at: 2.0,
            client_sent_at_ms: None,
            request_received_at_ms: None,
            permission_mode: "ask".to_string(),
            plan_mode: false,
            images: Vec::new(),
            worktree_path: "/repo".to_string(),
            mentions: Vec::new(),
            editor_context: None,
            existing_human_message_id: None,
            existing_agent_message_id: None,
        });
        handles.lock().await.insert("queued".to_string(), proc);

        let response = cancel_agent_queued_turn_internal(&handles, "queued", Some("drop"))
            .await
            .unwrap();

        assert_eq!(response.canceled_count, 1);
        assert_eq!(response.pending_queue_count, 1);
        assert_eq!(response.pending_queue[0].id, "keep");
        assert_eq!(response.pending_queue[0].content_preview, "first");
    }

    #[test]
    fn pending_turn_start_failure_log_is_redacted() {
        let log_line = pending_turn_start_failed_log_message();

        assert!(log_line.contains("code=pending_turn_start_failed"));
        assert!(log_line.contains("message=failed_to_start_pending_turn"));
        assert!(!log_line.contains("agent-session-secret"));
        assert!(!log_line.contains("queued message body"));
        assert!(!log_line.contains("/private/worktree/path"));
    }

    #[tokio::test]
    async fn cleanup_then_pending_resume_leaves_one_runtime_without_double_spawn() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let session_id = "step-cleanup-resume".to_string();
        handles
            .lock()
            .await
            .insert(session_id.clone(), make_test_agent_process());
        mark_pending_turn_starting(&session_id).await;
        let close_count = Arc::new(AtomicUsize::new(0));
        let spawn_count = Arc::new(AtomicUsize::new(0));

        {
            let _guard = acquire_session_runtime_lock(&session_id).await;
            if handles.lock().await.remove(&session_id).is_some() {
                close_count.fetch_add(1, Ordering::SeqCst);
            }
        }
        {
            let _guard = acquire_session_runtime_lock(&session_id).await;
            ensure_runtime_for_turn(&handles, &session_id, {
                let handles = Arc::clone(&handles);
                let session_id = session_id.clone();
                let spawn_count = Arc::clone(&spawn_count);
                move || async move {
                    spawn_count.fetch_add(1, Ordering::SeqCst);
                    handles
                        .lock()
                        .await
                        .insert(session_id, make_test_agent_process());
                    Ok(())
                }
            })
            .await
            .unwrap();
        }
        clear_pending_turn_starting(&session_id).await;

        assert_eq!(close_count.load(Ordering::SeqCst), 1);
        assert_eq!(spawn_count.load(Ordering::SeqCst), 1);
        assert!(handles.lock().await.contains_key(&session_id));
    }

    #[tokio::test]
    async fn session_runtime_lock_is_pruned_after_last_guard_drops() {
        {
            let _guard = acquire_session_runtime_lock("lock-prune-test").await;
            assert!(
                    crate::infrastructure::agent_session::runtime::runtime_coordinator::session_runtime_lock_exists(
                        "lock-prune-test"
                    )
                    .await
                );
        }

        for _ in 0..10 {
            if !crate::infrastructure::agent_session::runtime::runtime_coordinator::session_runtime_lock_exists("lock-prune-test")
                    .await
                {
                    return;
                }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("runtime lock was not pruned after guard drop");
    }

    #[tokio::test]
    async fn session_runtime_lock_serializes_same_step_operations() {
        let guard = acquire_session_runtime_lock("same-step-lock-test").await;
        let waiter = tokio::spawn(async {
            let _guard = acquire_session_runtime_lock("same-step-lock-test").await;
        });

        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(!waiter.is_finished());
        drop(guard);
        waiter.await.unwrap();
    }

    #[tokio::test]
    async fn spawn_session_guard_serializes_same_step_spawns() {
        let guard = acquire_spawn_session_guard("same-step-spawn-test").await;
        let waiter = tokio::spawn(async {
            let _guard = acquire_spawn_session_guard("same-step-spawn-test").await;
        });

        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(!waiter.is_finished());
        drop(guard);
        waiter.await.unwrap();
    }

    #[tokio::test]
    async fn closing_session_marker_blocks_until_close_finishes() {
        mark_session_closing("same-step-close-test").await;
        mark_session_closing("same-step-close-test").await;
        let waiter = tokio::spawn(async {
            wait_until_session_close_finished("same-step-close-test").await;
        });

        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(!waiter.is_finished());
        clear_session_closing("same-step-close-test").await;
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(!waiter.is_finished());
        clear_session_closing("same-step-close-test").await;
        waiter.await.unwrap();
    }

    #[tokio::test]
    async fn prepare_send_persists_selected_modes_for_existing_session() {
        // Spec issues-947: 既存セッションに対する送信時にも、検証済み permission_mode が
        // 異なれば ChatSession.permission_mode に書き戻される（保存層が単一の正典）。
        let data_dir = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let mut registry = AgentBackendRegistry::new();
        registry.register(Arc::new(MockModelBackend {
            backend_id: "mock".to_string(),
        }));
        let registry = Arc::new(registry);
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));

        let session_id = uuid::Uuid::new_v4().to_string();
        let session = chat_session_for_permission_test(&session_id, "edit");
        session_store
            .save_full_session_for_migration_or_restore(data_dir.path(), &session)
            .unwrap();

        let (response, _prepared_turn) = prepare_send_agent_message_internal(
            None,
            &crate::adaptor::controller::wiring::build_code_usecase(),
            None,
            &session_store,
            &registry,
            &handles,
            data_dir.path(),
            Some(session_id.clone()),
            "/repo".to_string(),
            "hello".to_string(),
            crate::domain::agent_session::PermissionMode::Ask,
            true,
            Some("mock".to_string()),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

        assert_eq!(response.session.permission_mode, "ask");
        assert!(response.session.plan_mode);
        let saved = session_store
            .load_full_session_for_restore(data_dir.path(), &session_id)
            .unwrap()
            .unwrap();
        assert_eq!(saved.permission_mode, "ask");
        assert!(saved.plan_mode);
    }

    #[tokio::test]
    async fn get_session_applies_runtime_streaming_overlay_to_latest_page() {
        let temp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let session = create_session_internal(
            &session_store,
            temp.path(),
            "/repo",
            Some(CLAUDE_BACKEND_ID.to_string()),
        )
        .unwrap();
        let agent_message = add_message_internal(
            &session_store,
            temp.path(),
            &session.id,
            MessageRole::Agent,
            "",
            Some(vec![MessagePart::Text {
                content: "persisted".to_string(),
                parent_tool_use_id: None,
            }]),
            None,
        )
        .unwrap();
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        {
            let mut proc = make_test_agent_process();
            proc.state = BridgeState::Streaming;
            proc.turn_phase = TurnPhase::Streaming;
            proc.streaming_message_id = Some(agent_message.id.clone());
            proc.streaming_parts = vec![MessagePart::Text {
                content: "streaming".to_string(),
                parent_tool_use_id: None,
            }];
            handles.lock().await.insert(session.id.clone(), proc);
        }

        let response = get_session_internal_with_data_dir(
            &session_store,
            &handles,
            Some(&make_fixed_model_registry()),
            temp.path(),
            &session.id,
        )
        .await
        .unwrap()
        .unwrap();

        assert_eq!(
            response.session.messages[0].parts,
            Some(vec![MessagePart::Text {
                content: "streaming".to_string(),
                parent_tool_use_id: None,
            }])
        );
        assert_eq!(
            response.initial_page,
            Some(InitialSessionPage {
                next_cursor: None,
                has_more: false,
                total_count: 1,
            })
        );
        let mut map = handles.lock().await;
        force_kill_all_sessions(&mut map).await;
    }

    #[tokio::test]
    async fn get_session_projects_large_tool_output_streaming_overlay() {
        let temp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let session = create_session_internal(
            &session_store,
            temp.path(),
            "/repo",
            Some(CLAUDE_BACKEND_ID.to_string()),
        )
        .unwrap();
        let agent_message = add_message_internal(
            &session_store,
            temp.path(),
            &session.id,
            MessageRole::Agent,
            "",
            Some(vec![MessagePart::Text {
                content: "persisted".to_string(),
                parent_tool_use_id: None,
            }]),
            None,
        )
        .unwrap();
        let full_output = format!(
            "{}GET_SESSION_OVERLAY_SECRET_TAIL",
            "x".repeat(crate::usecase::agent_session::session::MAX_TOOL_OUTPUT_BYTES + 1)
        );
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        {
            let mut proc = make_test_agent_process();
            proc.state = BridgeState::Streaming;
            proc.turn_phase = TurnPhase::Streaming;
            proc.streaming_message_id = Some(agent_message.id.clone());
            proc.streaming_parts = vec![MessagePart::ToolResult {
                content: full_output.clone(),
                is_error: false,
                tool_use_id: Some("tool-1".to_string()),
                parent_tool_use_id: None,
                content_ref: None,
                summary: None,
            }];
            handles.lock().await.insert(session.id.clone(), proc);
        }

        let response = get_session_internal_with_data_dir(
            &session_store,
            &handles,
            Some(&make_fixed_model_registry()),
            temp.path(),
            &session.id,
        )
        .await
        .unwrap()
        .unwrap();

        let MessagePart::ToolResult {
            content,
            content_ref,
            summary,
            ..
        } = &response.session.messages[0].parts.as_ref().unwrap()[0]
        else {
            panic!("expected tool result");
        };
        assert!(!content.contains("GET_SESSION_OVERLAY_SECRET_TAIL"));
        assert!(content.len() <= crate::usecase::agent_session::session::TOOL_OUTPUT_PREVIEW_BYTES);
        assert!(content_ref.is_none());
        assert_eq!(
            summary.as_ref().map(|summary| summary.byte_size),
            Some(full_output.len() as u64)
        );
        let mut map = handles.lock().await;
        force_kill_all_sessions(&mut map).await;
    }

    #[tokio::test]
    async fn get_session_reads_only_latest_page_for_large_sessions() {
        let temp = tempfile::tempdir().unwrap();
        let storage =
            Arc::new(crate::adaptor::gateway::agent_session::FileSessionStorage::default());
        let session_store = Arc::new(SessionStore::new(storage.clone()));
        let total_messages = INITIAL_SESSION_PAGE_LIMIT + 25;
        let session_id = uuid::Uuid::new_v4().to_string();
        let session = ChatSession {
            id: session_id.clone(),
            worktree_path: "/repo".to_string(),
            messages: (0..total_messages)
                .map(|index| ChatMessage {
                    id: format!("m{index}"),
                    role: MessageRole::Human,
                    content: format!("message {index}"),
                    thinking: None,
                    activities: None,
                    parts: None,
                    streaming_final_seq: 0,
                    timestamp: 1000.0 + index as f64,
                    mentions: None,
                })
                .collect(),
            state: crate::usecase::agent_session::session::SessionState::Idle,
            created_at: 1000.0,
            updated_at: 2000.0,
            agent_session_id: None,
            context_carry: None,
            permission_mode: "edit".to_string(),
            plan_mode: false,
            permission_profile_id: None,
            selected_model: Some("selected-model".to_string()),
            backend_id: Some(CLAUDE_BACKEND_ID.to_string()),
            workflow_step_session: false,
            workflow_step_context: None,
            context_epoch: None,
        };
        session_store
            .save_full_session_for_migration_or_restore(temp.path(), &session)
            .unwrap();
        storage.reset_message_read_count();
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));

        let response = get_session_internal_with_data_dir(
            &session_store,
            &handles,
            None,
            temp.path(),
            &session_id,
        )
        .await
        .unwrap()
        .unwrap();

        assert_eq!(response.session.id, session_id);
        assert_eq!(
            response.session.selected_model,
            Some(crate::domain::agent_session::model_entry_id(
                CLAUDE_BACKEND_ID,
                "selected-model",
            ))
        );
        assert_eq!(
            response.turn_phase,
            crate::usecase::agent_session::status::TurnPhase::Idle
        );
        assert_eq!(response.session.messages.len(), INITIAL_SESSION_PAGE_LIMIT);
        assert_eq!(response.session.messages[0].id, "m25");
        assert_eq!(
            response.session.messages[INITIAL_SESSION_PAGE_LIMIT - 1].id,
            format!("m{}", total_messages - 1)
        );
        assert_eq!(
            response.initial_page,
            Some(InitialSessionPage {
                next_cursor: Some(PageCursor(26)),
                has_more: true,
                total_count: total_messages,
            })
        );
        assert_eq!(storage.message_read_count(), INITIAL_SESSION_PAGE_LIMIT);
    }

    #[tokio::test]
    async fn get_session_page_applies_runtime_streaming_overlay() {
        let temp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let session = create_session_internal(
            &session_store,
            temp.path(),
            "/repo",
            Some(CLAUDE_BACKEND_ID.to_string()),
        )
        .unwrap();
        let agent_message = add_message_internal(
            &session_store,
            temp.path(),
            &session.id,
            MessageRole::Agent,
            "",
            Some(vec![MessagePart::Text {
                content: "persisted".to_string(),
                parent_tool_use_id: None,
            }]),
            None,
        )
        .unwrap();
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        {
            let mut proc = make_test_agent_process();
            proc.state = BridgeState::Streaming;
            proc.turn_phase = TurnPhase::Streaming;
            proc.streaming_message_id = Some(agent_message.id.clone());
            proc.streaming_parts = vec![MessagePart::Text {
                content: "streaming".to_string(),
                parent_tool_use_id: None,
            }];
            handles.lock().await.insert(session.id.clone(), proc);
        }

        let page = get_session_page_internal_with_data_dir(
            &session_store,
            &handles,
            temp.path(),
            &session.id,
            None,
            10,
        )
        .await
        .unwrap()
        .unwrap();

        assert_eq!(
            page.messages[0].parts,
            Some(vec![MessagePart::Text {
                content: "streaming".to_string(),
                parent_tool_use_id: None,
            }])
        );
        let mut map = handles.lock().await;
        force_kill_all_sessions(&mut map).await;
    }

    #[tokio::test]
    async fn get_session_page_projects_large_tool_output_streaming_overlay() {
        let temp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let session = create_session_internal(
            &session_store,
            temp.path(),
            "/repo",
            Some(CLAUDE_BACKEND_ID.to_string()),
        )
        .unwrap();
        let agent_message = add_message_internal(
            &session_store,
            temp.path(),
            &session.id,
            MessageRole::Agent,
            "",
            Some(vec![MessagePart::Text {
                content: "persisted".to_string(),
                parent_tool_use_id: None,
            }]),
            None,
        )
        .unwrap();
        let full_output = format!(
            "{}GET_PAGE_OVERLAY_SECRET_TAIL",
            "x".repeat(crate::usecase::agent_session::session::MAX_TOOL_OUTPUT_BYTES + 1)
        );
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        {
            let mut proc = make_test_agent_process();
            proc.state = BridgeState::Streaming;
            proc.turn_phase = TurnPhase::Streaming;
            proc.streaming_message_id = Some(agent_message.id.clone());
            proc.streaming_parts = vec![MessagePart::ToolResult {
                content: full_output.clone(),
                is_error: false,
                tool_use_id: Some("tool-1".to_string()),
                parent_tool_use_id: None,
                content_ref: None,
                summary: None,
            }];
            handles.lock().await.insert(session.id.clone(), proc);
        }

        let page = get_session_page_internal_with_data_dir(
            &session_store,
            &handles,
            temp.path(),
            &session.id,
            None,
            10,
        )
        .await
        .unwrap()
        .unwrap();

        let page_json = serde_json::to_string(&page).unwrap();
        assert!(!page_json.contains("GET_PAGE_OVERLAY_SECRET_TAIL"));
        let MessagePart::ToolResult {
            content,
            content_ref,
            summary,
            ..
        } = &page.messages[0].parts.as_ref().unwrap()[0]
        else {
            panic!("expected tool result");
        };
        assert!(content.len() <= crate::usecase::agent_session::session::TOOL_OUTPUT_PREVIEW_BYTES);
        assert!(content_ref.is_none());
        assert_eq!(
            summary.as_ref().map(|summary| summary.byte_size),
            Some(full_output.len() as u64)
        );
        let mut map = handles.lock().await;
        force_kill_all_sessions(&mut map).await;
    }

    #[test]
    fn token_usage_from_result_message_preserves_context_window_metadata() {
        let usage = token_usage_from_result_message(&serde_json::json!({
            "type": "result",
            "modelUsage": {
                "codex": {
                    "inputTokens": 12,
                    "outputTokens": 34,
                    "totalTokens": 1234,
                    "contextWindowTokens": 200000
                }
            }
        }))
        .expect("usage should be parsed");

        assert_eq!(usage.input_tokens, 12);
        assert_eq!(usage.output_tokens, 34);
        assert_eq!(usage.total_tokens, Some(1234));
        assert_eq!(usage.context_window_tokens, Some(200000));
    }

    // --- get_session: active process が居ても config 由来を返す ---

    #[tokio::test]
    async fn get_session_returns_config_derived_available_models_even_with_active_process() {
        let temp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let session = create_session_internal(
            &session_store,
            temp.path(),
            "/repo",
            Some(CODEX_BACKEND_ID.to_string()),
        )
        .unwrap();
        let registry = make_test_registry_with_models(&[], &["from-config"]);

        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        // active process の stale キャッシュ
        {
            let mut map = handles.lock().await;
            let mut proc = make_test_agent_process();
            proc.backend_id = CODEX_BACKEND_ID.to_string();
            proc.available_models = vec![ModelInfo::new(CODEX_BACKEND_ID, "stale-from-process")];
            proc.latest_token_usage = Some(TokenUsage {
                input_tokens: 1200,
                output_tokens: 34,
                total_tokens: None,
                context_window_tokens: None,
            });
            map.insert(session.id.clone(), proc);
        }

        let response = get_session_internal_with_data_dir(
            &session_store,
            &handles,
            Some(&registry),
            temp.path(),
            &session.id,
        )
        .await
        .unwrap()
        .expect("session should exist");

        let values: Vec<String> = response
            .available_models
            .into_iter()
            .map(|m| m.model_id)
            .collect();
        assert_eq!(values, vec!["from-config".to_string()]);
        assert_eq!(
            response.latest_token_usage,
            Some(TokenUsage {
                input_tokens: 1200,
                output_tokens: 34,
                total_tokens: None,
                context_window_tokens: None,
            })
        );

        let mut map = handles.lock().await;
        force_kill_all_sessions(&mut map).await;
    }

    #[tokio::test]
    async fn get_session_page_returns_latest_token_usage_metadata() {
        let temp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let session = create_session_internal(
            &session_store,
            temp.path(),
            "/repo",
            Some(CLAUDE_BACKEND_ID.to_string()),
        )
        .unwrap();
        add_message_internal(
            &session_store,
            temp.path(),
            &session.id,
            MessageRole::Human,
            "hello",
            None,
            None,
        )
        .unwrap();
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        {
            let mut proc = make_test_agent_process();
            proc.latest_token_usage = Some(TokenUsage {
                input_tokens: 10,
                output_tokens: 5,
                total_tokens: Some(15),
                context_window_tokens: Some(200_000),
            });
            handles.lock().await.insert(session.id.clone(), proc);
        }

        let page = get_session_page_internal_with_data_dir(
            &session_store,
            &handles,
            temp.path(),
            &session.id,
            None,
            10,
        )
        .await
        .unwrap()
        .expect("page should exist");

        assert_eq!(
            page.latest_token_usage,
            Some(TokenUsage {
                input_tokens: 10,
                output_tokens: 5,
                total_tokens: Some(15),
                context_window_tokens: Some(200_000),
            })
        );
        assert_eq!(page.message_metadata[0].message_id, page.messages[0].id);

        let mut map = handles.lock().await;
        force_kill_all_sessions(&mut map).await;
    }

    #[tokio::test]
    async fn get_session_resolves_none_selected_model_to_default() {
        // spec: モデル未選択状態は廃止。selected_model=None の既存セッションを get_session
        // すると、応答の selected_model は backend の既定モデル（固定リスト先頭）へ解決される。
        let temp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let mut session = create_session_internal(
            &session_store,
            temp.path(),
            "/repo",
            Some(CLAUDE_BACKEND_ID.to_string()),
        )
        .unwrap();
        // 旧フォーマット（未選択）を模して None を永続化する。
        session.selected_model = None;
        session_store
            .save_full_session_for_migration_or_restore(temp.path(), &session)
            .unwrap();

        let registry = make_fixed_model_registry();
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));

        let response = get_session_internal_with_data_dir(
            &session_store,
            &handles,
            Some(&registry),
            temp.path(),
            &session.id,
        )
        .await
        .unwrap()
        .expect("session should exist");

        assert_eq!(
            response.session.selected_model,
            Some(crate::domain::agent_session::model_entry_id(
                CLAUDE_BACKEND_ID,
                crate::domain::agent_session::CLAUDE_FIXED_MODELS[0],
            ))
        );
    }

    #[tokio::test]
    async fn get_session_errors_when_default_model_unresolvable() {
        // 契約: 応答の selected_model は常に非 null。registry が在りつつ既定モデルへ解決
        // できない場合（fixed_models 無し + config 空）、フィールド脱落を防ぐため Err を返す。
        let temp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let mut session = create_session_internal(
            &session_store,
            temp.path(),
            "/repo",
            Some(CLAUDE_BACKEND_ID.to_string()),
        )
        .unwrap();
        session.selected_model = None;
        session_store
            .save_full_session_for_migration_or_restore(temp.path(), &session)
            .unwrap();

        // claude/codex とも fixed_models を持たない mock backend + 空 config → 既定モデル無し。
        let registry = make_test_registry_with_models(&[], &[]);
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));

        let result = get_session_internal_with_data_dir(
            &session_store,
            &handles,
            Some(&registry),
            temp.path(),
            &session.id,
        )
        .await;

        assert!(result.is_err());
    }
}
