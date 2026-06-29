use super::model_selection::resolve_selected_model;
use super::process_registry::{AgentProcess, AgentProcessMap, BridgeState, PendingMessage};
use super::shared::{
    parts_have_tool_output_ref, CLAUDE_BACKEND_ID, DEFER_AGENT_SESSION_ID_PERSIST_ON_READY,
};
use crate::infrastructure::agent_session::runtime::context_restore::context_restore_plan_for_session;
use crate::infrastructure::agent_session::runtime::context_restore::context_restore_plan_for_session_before_turn;
use crate::infrastructure::agent_session::runtime::context_restore::context_restore_plan_from_meta;
use crate::infrastructure::agent_session::runtime::context_restore::ContextRestorePlan;
use crate::infrastructure::agent_session::runtime::ImageAttachment;
use crate::infrastructure::platform::app_data_dir::resolve_data_dir;
use crate::usecase::agent_session::session::parts_to_legacy;
use crate::usecase::agent_session::session::ChatMessage;
use crate::usecase::agent_session::session::ChatSession;
use crate::usecase::agent_session::session::ContextCarryState;
use crate::usecase::agent_session::session::MessagePart;
use crate::usecase::agent_session::session::MessageRole;
use crate::usecase::agent_session::session::SessionMeta;
use crate::usecase::agent_session::session::SessionStore;
use std::path::Path;
use std::sync::Arc;
use tauri::Manager;
use tokio::sync::Mutex;

pub(super) const PERSIST_INTERVAL_MS: u64 = 1000;

#[derive(Debug, Clone, PartialEq)]
pub(super) struct PersistedStreamingParts {
    pub parts: Vec<MessagePart>,
    pub has_tool_output_ref: bool,
}

impl PersistedStreamingParts {
    fn from_parts(parts: Vec<MessagePart>) -> Self {
        let has_tool_output_ref = parts_have_tool_output_ref(&parts);
        Self {
            parts,
            has_tool_output_ref,
        }
    }
}

pub(super) fn persist_streaming_parts<R: tauri::Runtime>(
    session_store: &SessionStore,
    app: &tauri::AppHandle<R>,
    chat_session_id: &str,
    message_id: &str,
    parts: &[MessagePart],
    streaming_final_seq: u64,
    completed_at: Option<f64>,
) -> Option<PersistedStreamingParts> {
    let data_dir = match resolve_data_dir(app) {
        Ok(d) => d,
        Err(e) => {
            log::warn!(
                "Failed to resolve data dir for streaming persist (session {chat_session_id}): {e}"
            );
            return None;
        }
    };
    match session_store.persist_message_parts(
        &data_dir,
        chat_session_id,
        message_id,
        parts,
        streaming_final_seq,
        completed_at,
    ) {
        Ok(parts) => Some(PersistedStreamingParts::from_parts(parts)),
        Err(e) => {
            log::warn!("Failed to persist streaming parts for session {chat_session_id}: {e}");
            None
        }
    }
}

pub(super) fn load_post_turn_base_parts_from_store<R: tauri::Runtime>(
    session_store: &SessionStore,
    app: &tauri::AppHandle<R>,
    chat_session_id: &str,
    message_id: &str,
) -> Option<Vec<MessagePart>> {
    let data_dir = match resolve_data_dir(app) {
        Ok(d) => d,
        Err(e) => {
            log::warn!(
                "Failed to resolve data dir for post-turn streaming reseed \
                 (session {chat_session_id}, message {message_id}): {e}"
            );
            return None;
        }
    };
    let session = match session_store.load_full_session_for_restore(&data_dir, chat_session_id) {
        Ok(Some(s)) => s,
        Ok(None) => {
            log::warn!(
                "Session not found for post-turn streaming reseed: \
                 session {chat_session_id}, message {message_id}"
            );
            return None;
        }
        Err(e) => {
            log::warn!(
                "Failed to get session for post-turn streaming reseed \
                 (session {chat_session_id}, message {message_id}): {e}"
            );
            return None;
        }
    };
    let Some(message) = session.messages.iter().find(|m| m.id == message_id) else {
        log::warn!(
            "Message not found for post-turn streaming reseed: \
             session {chat_session_id}, message {message_id}"
        );
        return None;
    };
    if let Some(parts) = message.parts.clone() {
        return Some(parts);
    }
    if !message.content.is_empty() {
        return Some(vec![MessagePart::Text {
            content: message.content.clone(),
            parent_tool_use_id: None,
        }]);
    }
    Some(Vec::new())
}

pub(super) struct PersistedSpawnInfo {
    pub resume_sid: Option<String>,
    pub has_session: bool,
    pub selected_model: Option<String>,
    pub backend_id: String,
    pub permission_profile_id: Option<String>,
    pub context_restore_plan: ContextRestorePlan,
}

/// Retrieve persisted session fields needed for spawning a Bridge process.
///
/// モデル「未選択（None）」状態は廃止されたが、`ChatSession.selected_model` の永続化型は
/// 既存 JSON 互換のため `Option<String>` のまま。spawn 経路では `None` を backend の
/// 既定モデル（[`crate::infrastructure::agent_session::runtime::AgentBackendRegistry::default_model_for`]）へ lazy 解決して
/// から Bridge へ渡す。
pub(super) fn get_persisted_spawn_info<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    session_store: &SessionStore,
    chat_session_id: &str,
) -> Result<PersistedSpawnInfo, String> {
    let data_dir = resolve_data_dir(app)?;
    let registry =
        app.try_state::<Arc<crate::infrastructure::agent_session::runtime::AgentBackendRegistry>>();
    let meta = session_store.get_session_meta(&data_dir, chat_session_id)?;
    resolve_spawn_info_from_meta_or_full(
        session_store,
        &data_dir,
        chat_session_id,
        meta,
        registry.as_deref(),
        None,
    )
}

pub(super) fn require_session_meta_for_turn(
    session_store: &SessionStore,
    data_dir: &Path,
    chat_session_id: &str,
) -> Result<SessionMeta, String> {
    session_store
        .get_session_meta(data_dir, chat_session_id)?
        .ok_or_else(|| format!("Session not found: {chat_session_id}"))
}

pub(super) fn get_required_persisted_spawn_info_for_turn<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    session_store: &SessionStore,
    chat_session_id: &str,
) -> Result<PersistedSpawnInfo, String> {
    let data_dir = resolve_data_dir(app)?;
    let registry =
        app.try_state::<Arc<crate::infrastructure::agent_session::runtime::AgentBackendRegistry>>();
    let meta = require_session_meta_for_turn(session_store, &data_dir, chat_session_id)?;
    resolve_spawn_info_from_meta_or_full(
        session_store,
        &data_dir,
        chat_session_id,
        Some(meta),
        registry.as_deref(),
        None,
    )
}

pub(super) fn get_required_persisted_spawn_info_before_turn<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    session_store: &SessionStore,
    chat_session_id: &str,
    streaming_agent_message_id: &str,
) -> Result<PersistedSpawnInfo, String> {
    let data_dir = resolve_data_dir(app)?;
    let registry =
        app.try_state::<Arc<crate::infrastructure::agent_session::runtime::AgentBackendRegistry>>();
    let meta = require_session_meta_for_turn(session_store, &data_dir, chat_session_id)?;
    resolve_spawn_info_from_meta_or_full(
        session_store,
        &data_dir,
        chat_session_id,
        Some(meta),
        registry.as_deref(),
        Some(streaming_agent_message_id),
    )
}

pub(super) fn resolve_spawn_info_from_meta(
    meta: SessionMeta,
    registry: Option<&Arc<crate::infrastructure::agent_session::runtime::AgentBackendRegistry>>,
    context_restore_plan: ContextRestorePlan,
) -> PersistedSpawnInfo {
    let has_session = meta
        .agent_session_id
        .as_deref()
        .map(str::trim)
        .is_some_and(|session_id| !session_id.is_empty());
    let backend_id = meta
        .backend_id
        .unwrap_or_else(|| CLAUDE_BACKEND_ID.to_string());
    let selected_model = resolve_selected_model(meta.selected_model, &backend_id, registry);
    PersistedSpawnInfo {
        resume_sid: context_restore_plan
            .resume_session_id()
            .map(ToString::to_string),
        has_session,
        selected_model,
        backend_id,
        permission_profile_id: meta.permission_profile_id,
        context_restore_plan,
    }
}

pub(super) fn resolve_spawn_info_from_meta_or_full(
    session_store: &SessionStore,
    data_dir: &Path,
    chat_session_id: &str,
    meta: Option<SessionMeta>,
    registry: Option<&Arc<crate::infrastructure::agent_session::runtime::AgentBackendRegistry>>,
    before_turn_message_id: Option<&str>,
) -> Result<PersistedSpawnInfo, String> {
    let Some(meta) = meta else {
        return Ok(resolve_spawn_info_with_plan(
            None,
            registry,
            ContextRestorePlan::NoContext,
        ));
    };
    if let Some(plan) = context_restore_plan_from_meta(&meta) {
        return Ok(resolve_spawn_info_from_meta(meta, registry, plan));
    }
    let persisted = session_store.load_full_session_for_restore(data_dir, chat_session_id)?;
    let context_restore_plan = match before_turn_message_id {
        Some(message_id) => {
            context_restore_plan_for_session_before_turn(persisted.as_ref(), message_id)
        }
        None => context_restore_plan_for_session(persisted.as_ref()),
    };
    Ok(resolve_spawn_info_with_plan(
        persisted,
        registry,
        context_restore_plan,
    ))
}

/// 永続化セッションから spawn 情報を組み立てる純粋関数。
///
/// `selected_model == None` は registry の既定モデルへ解決する（モデル未選択状態は廃止）。
/// registry 未指定（テスト等）では `None` のままとする。
#[cfg(test)]
pub(super) fn resolve_spawn_info(
    persisted: Option<ChatSession>,
    registry: Option<&Arc<crate::infrastructure::agent_session::runtime::AgentBackendRegistry>>,
) -> PersistedSpawnInfo {
    let context_restore_plan = context_restore_plan_for_session(persisted.as_ref());
    resolve_spawn_info_with_plan(persisted, registry, context_restore_plan)
}

pub(super) fn resolve_spawn_info_with_plan(
    persisted: Option<ChatSession>,
    registry: Option<&Arc<crate::infrastructure::agent_session::runtime::AgentBackendRegistry>>,
    context_restore_plan: ContextRestorePlan,
) -> PersistedSpawnInfo {
    let (
        resume_sid,
        has_session,
        selected_model,
        backend_id,
        permission_profile_id,
        context_restore_plan,
    ) = persisted_spawn_info_from_session(persisted, context_restore_plan);
    let selected_model = resolve_selected_model(selected_model, &backend_id, registry);
    PersistedSpawnInfo {
        resume_sid,
        has_session,
        selected_model,
        backend_id,
        permission_profile_id,
        context_restore_plan,
    }
}

pub(super) fn persisted_spawn_info_from_session(
    session: Option<ChatSession>,
    context_restore_plan: ContextRestorePlan,
) -> (
    Option<String>,
    bool,
    Option<String>,
    String,
    Option<String>,
    ContextRestorePlan,
) {
    session
        .map(|s| {
            let has_session = s
                .agent_session_id
                .as_deref()
                .map(str::trim)
                .is_some_and(|session_id| !session_id.is_empty());
            (
                context_restore_plan
                    .resume_session_id()
                    .map(ToString::to_string),
                has_session,
                s.selected_model,
                s.backend_id
                    .unwrap_or_else(|| CLAUDE_BACKEND_ID.to_string()),
                s.permission_profile_id,
                context_restore_plan.clone(),
            )
        })
        .unwrap_or((
            None,
            false,
            None,
            CLAUDE_BACKEND_ID.to_string(),
            None,
            context_restore_plan,
        ))
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct SessionContextCarryUpdate {
    pub(crate) chat_session_id: String,
    pub(crate) agent_session_id: Option<String>,
    pub(crate) context_carry: Option<ContextCarryState>,
    pub(crate) updated_at: f64,
}

impl SessionContextCarryUpdate {
    fn from_meta(meta: &crate::usecase::agent_session::session::SessionMeta) -> Self {
        Self {
            chat_session_id: meta.id.clone(),
            agent_session_id: meta.agent_session_id.clone(),
            context_carry: meta.context_carry.clone(),
            updated_at: meta.updated_at,
        }
    }

    fn to_protocol(&self) -> crate::adaptor::protocol::AgentSessionContextCarryUpdated {
        crate::adaptor::protocol::AgentSessionContextCarryUpdated {
            chat_session_id: self.chat_session_id.clone(),
            agent_session_id: self.agent_session_id.clone(),
            context_carry: self.context_carry.clone(),
            updated_at: self.updated_at,
        }
    }
}

pub(super) fn emit_session_context_carry_update<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    update: SessionContextCarryUpdate,
) {
    use tauri::Emitter;

    let payload = update.to_protocol();
    let _ = app.emit("agent-session-context-carry-updated", &payload);
}

pub(super) fn session_ready_resume_mismatch(
    context_carry_on_ready: Option<&ContextCarryState>,
    requested_resume_id: Option<&str>,
    ready_session_id: Option<&str>,
) -> bool {
    if context_carry_on_ready != Some(&ContextCarryState::Resumed) {
        return false;
    }
    let Some(requested_resume_id) = requested_resume_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return false;
    };
    ready_session_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        != Some(requested_resume_id)
}

#[derive(Debug, Clone)]
pub(super) struct StreamingTurnRequeueCandidate {
    pub(crate) streaming_message_id: String,
    pub(crate) permission_mode: String,
}

pub(super) fn streaming_turn_requeue_candidate(
    proc: &AgentProcess,
) -> Option<StreamingTurnRequeueCandidate> {
    if proc.state != BridgeState::Streaming {
        return None;
    }
    Some(StreamingTurnRequeueCandidate {
        streaming_message_id: proc.streaming_message_id.clone()?,
        permission_mode: proc.current_permission_mode.clone(),
    })
}

pub(super) fn pending_content_from_human_message(message: &ChatMessage) -> String {
    if !message.content.is_empty() {
        return message.content.clone();
    }
    message.parts.as_deref().map_or_else(String::new, |parts| {
        let (content, _, _) = parts_to_legacy(parts);
        content
    })
}

pub(super) fn pending_images_from_human_message(message: &ChatMessage) -> Vec<ImageAttachment> {
    message
        .parts
        .as_deref()
        .unwrap_or_default()
        .iter()
        .filter_map(|part| match part {
            MessagePart::Image { data, media_type } => Some(ImageAttachment {
                data: data.clone(),
                media_type: media_type.clone(),
            }),
            MessagePart::ImageRef { .. } => None,
            _ => None,
        })
        .collect()
}

pub(super) fn pending_mentions_from_human_message(
    message: &ChatMessage,
) -> Vec<crate::domain::code::MentionReference> {
    message
        .mentions
        .clone()
        .unwrap_or_default()
        .into_iter()
        .map(crate::usecase::agent_session::session::MessageMention::into_domain)
        .collect()
}

pub(super) fn pending_message_from_streaming_turn(
    session: &ChatSession,
    candidate: &StreamingTurnRequeueCandidate,
) -> Option<PendingMessage> {
    let agent_index = session
        .messages
        .iter()
        .position(|message| message.id == candidate.streaming_message_id)?;
    if session.messages.get(agent_index)?.role != MessageRole::Agent {
        return None;
    }
    let human_index = agent_index.checked_sub(1)?;
    let human_message = session.messages.get(human_index)?;
    if human_message.role != MessageRole::Human {
        return None;
    }
    Some(PendingMessage {
        id: uuid::Uuid::new_v4().to_string(),
        content: pending_content_from_human_message(human_message),
        created_at: human_message.timestamp,
        client_sent_at_ms: None,
        request_received_at_ms: None,
        permission_mode: candidate.permission_mode.clone(),
        plan_mode: session.plan_mode,
        images: pending_images_from_human_message(human_message),
        worktree_path: session.worktree_path.clone(),
        mentions: pending_mentions_from_human_message(human_message),
        editor_context: None,
        existing_human_message_id: Some(human_message.id.clone()),
        existing_agent_message_id: Some(candidate.streaming_message_id.clone()),
    })
}

pub(super) async fn requeue_streaming_turn_for_resume_mismatch<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    session_store: &SessionStore,
    chat_session_id: &str,
    candidate: StreamingTurnRequeueCandidate,
) -> bool {
    let data_dir = match resolve_data_dir(app) {
        Ok(data_dir) => data_dir,
        Err(e) => {
            log::warn!("Failed to resolve data dir for resume mismatch requeue: {e}");
            return false;
        }
    };
    let session = match session_store.load_full_session_for_restore(&data_dir, chat_session_id) {
        Ok(Some(session)) => session,
        Ok(None) => {
            log::warn!("Session not found for resume mismatch requeue: {chat_session_id}");
            return false;
        }
        Err(e) => {
            log::warn!("Failed to load session for resume mismatch requeue: {e}");
            return false;
        }
    };
    let Some(pending) = pending_message_from_streaming_turn(&session, &candidate) else {
        log::warn!("Streaming turn not found for resume mismatch requeue: {chat_session_id}");
        return false;
    };

    let mut map = handles.lock().await;
    let Some(proc) = map.get_mut(chat_session_id) else {
        return false;
    };
    if proc.state != BridgeState::Streaming
        || proc.streaming_message_id.as_deref() != Some(candidate.streaming_message_id.as_str())
    {
        return false;
    }
    proc.pending_messages.push_front(pending);
    true
}

pub(super) fn take_defer_agent_session_id_persist_on_ready(msg: &mut serde_json::Value) -> bool {
    msg.as_object_mut()
        .and_then(|object| object.remove(DEFER_AGENT_SESSION_ID_PERSIST_ON_READY))
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
}

pub(super) fn save_session_context_carry(
    session_store: &SessionStore,
    data_dir: &Path,
    chat_session_id: &str,
    context_carry: ContextCarryState,
) -> Result<Option<SessionContextCarryUpdate>, String> {
    session_store
        .update_context_carry_if_changed(data_dir, chat_session_id, Some(context_carry))
        .map(|updated| updated.as_ref().map(SessionContextCarryUpdate::from_meta))
}

pub(super) fn save_resume_mismatch_for_reinject(
    session_store: &SessionStore,
    data_dir: &Path,
    chat_session_id: &str,
) -> Result<Option<SessionContextCarryUpdate>, String> {
    session_store
        .update_resume_metadata_if_changed(data_dir, chat_session_id, None, None)
        .map(|updated| updated.as_ref().map(SessionContextCarryUpdate::from_meta))
}

pub(super) fn persist_resume_mismatch_for_reinject<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    session_store: &SessionStore,
    chat_session_id: &str,
) {
    match resolve_data_dir(app).and_then(|data_dir| {
        save_resume_mismatch_for_reinject(session_store, &data_dir, chat_session_id)
    }) {
        Ok(Some(update)) => emit_session_context_carry_update(app, update),
        Ok(None) => {}
        Err(e) => {
            log::warn!("Failed to prepare context reinjection for {chat_session_id}: {e}");
        }
    }
}

pub(crate) fn persist_context_carry_state<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    session_store: &SessionStore,
    chat_session_id: &str,
    context_carry: ContextCarryState,
) {
    match resolve_data_dir(app).and_then(|data_dir| {
        save_session_context_carry(session_store, &data_dir, chat_session_id, context_carry)
    }) {
        Ok(Some(update)) => emit_session_context_carry_update(app, update),
        Ok(None) => {}
        Err(e) => {
            log::warn!("Failed to persist context carry state for {chat_session_id}: {e}");
        }
    }
}

pub(super) fn persist_agent_session_id<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    session_store: &SessionStore,
    chat_session_id: &str,
    agent_session_id: &str,
) {
    let agent_session_id = agent_session_id.trim();
    if agent_session_id.is_empty() {
        return;
    }
    let result = resolve_data_dir(app).and_then(|data_dir| {
        session_store
            .update_agent_session_id_if_changed(
                &data_dir,
                chat_session_id,
                Some(agent_session_id.to_string()),
            )
            .map(|_| ())
    });
    if let Err(e) = result {
        log::warn!("Failed to persist agent session id for {chat_session_id}: {e}");
    }
}

pub(super) fn load_persisted_agent_session_id_for_resume<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    session_store: &SessionStore,
    chat_session_id: &str,
) -> Option<String> {
    let data_dir = match resolve_data_dir(app) {
        Ok(data_dir) => data_dir,
        Err(e) => {
            log::warn!(
                "Failed to resolve data dir for interrupted resume rollback \
                 (session {chat_session_id}): {e}"
            );
            return None;
        }
    };
    let session = match session_store.load_full_session_for_restore(&data_dir, chat_session_id) {
        Ok(Some(session)) => session,
        Ok(None) => {
            log::warn!(
                "Session not found for interrupted resume rollback: session {chat_session_id}"
            );
            return None;
        }
        Err(e) => {
            log::warn!(
                "Failed to load session for interrupted resume rollback \
                 (session {chat_session_id}): {e}"
            );
            return None;
        }
    };
    session.agent_session_id.and_then(|sid| match sid.trim() {
        "" => None,
        trimmed => Some(trimmed.to_string()),
    })
}

pub(super) fn should_mark_context_carry_failed_after_init_error(
    context_carry: Option<&ContextCarryState>,
    force_context_carry_failed: bool,
) -> bool {
    if force_context_carry_failed
        || matches!(
            context_carry,
            Some(ContextCarryState::Resumed | ContextCarryState::Reinjected)
        )
    {
        return context_carry != Some(&ContextCarryState::Failed);
    }
    false
}

pub(crate) fn persist_context_carry_failed_after_init_error<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    session_store: &SessionStore,
    chat_session_id: &str,
    clear_agent_session_id: bool,
    force_context_carry_failed: bool,
) {
    let result = resolve_data_dir(app).and_then(|data_dir| {
        let Some(meta) = session_store.get_session_meta(&data_dir, chat_session_id)? else {
            return Ok(None);
        };
        let next_agent_session_id = if clear_agent_session_id {
            None
        } else {
            meta.agent_session_id.clone()
        };
        let next_context_carry = if should_mark_context_carry_failed_after_init_error(
            meta.context_carry.as_ref(),
            force_context_carry_failed,
        ) {
            Some(ContextCarryState::Failed)
        } else {
            meta.context_carry.clone()
        };
        session_store
            .update_resume_metadata_if_changed(
                &data_dir,
                chat_session_id,
                next_agent_session_id,
                next_context_carry,
            )
            .map(|updated| updated.as_ref().map(SessionContextCarryUpdate::from_meta))
    });
    match result {
        Ok(Some(update)) => emit_session_context_carry_update(app, update),
        Ok(None) => {}
        Err(e) => {
            log::warn!("Failed to persist context carry failure for {chat_session_id}: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::PERSIST_INTERVAL_MS;

    #[test]
    fn session_persistence_interval_matches_existing_contract() {
        assert_eq!(PERSIST_INTERVAL_MS, 1000);
    }
}
#[cfg(test)]
mod moved_tests {
    use super::super::external_agent::*;

    use super::super::process_registry::*;
    use super::super::recovery::*;
    use super::super::sdk_message::*;
    use super::super::session_lifecycle::*;
    use super::super::session_persistence::*;
    use super::super::shared::test_support::*;
    use super::super::shared::*;

    use super::super::turn_event_log::*;

    use crate::infrastructure::agent_session::runtime::runtime_coordinator::clear_pending_turn_starting;
    use crate::usecase::agent_session::session::{
        add_message_internal, create_session_internal, ChatMessage, ContextCarryState, MessagePart,
        MessageRole,
    };

    use std::sync::Arc;

    use tokio::sync::Mutex;

    #[test]
    fn persisted_spawn_info_uses_step_agent_session_id_for_resume() {
        let info = resolve_spawn_info(Some(chat_session_for_spawn_info("step")), None);

        assert_eq!(info.resume_sid.as_deref(), Some("sdk-resume-id"));
        assert_eq!(info.selected_model.as_deref(), Some("sonnet"));
        assert_eq!(info.backend_id, "mock");
        assert!(matches!(
            info.context_restore_plan,
            ContextRestorePlan::Resume { .. }
        ));
    }

    #[test]
    fn persist_agent_session_id_updates_session_store_on_ready() {
        let temp = tempfile::tempdir().unwrap();
        let app = tauri::test::mock_builder()
            .manage(crate::infrastructure::platform::app_data_dir::TestDataDir(
                temp.path().to_path_buf(),
            ))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let store = crate::test_support::build_session_store();
        let session = create_session_internal(
            &store,
            temp.path(),
            "/repo",
            Some(CLAUDE_BACKEND_ID.to_string()),
        )
        .unwrap();

        persist_agent_session_id(app.handle(), &store, &session.id, "sdk-ready");

        let loaded = store
            .load_full_session_for_restore(temp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(loaded.agent_session_id.as_deref(), Some("sdk-ready"));
    }

    #[test]
    fn load_post_turn_base_parts_preserves_legacy_content_without_parts() {
        let temp = tempfile::tempdir().unwrap();
        let app = tauri::test::mock_builder()
            .manage(crate::infrastructure::platform::app_data_dir::TestDataDir(
                temp.path().to_path_buf(),
            ))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let store = crate::test_support::build_session_store();
        let session = create_session_internal(
            &store,
            temp.path(),
            "/repo",
            Some(CLAUDE_BACKEND_ID.to_string()),
        )
        .unwrap();
        let message = add_message_internal(
            &store,
            temp.path(),
            &session.id,
            MessageRole::Agent,
            "legacy body",
            None,
            None,
        )
        .unwrap();
        let mut loaded = store
            .load_full_session_for_restore(temp.path(), &session.id)
            .unwrap()
            .unwrap();
        let stored_message = loaded
            .messages
            .iter_mut()
            .find(|stored| stored.id == message.id)
            .expect("message must be present");
        stored_message.parts = None;
        stored_message.content = "legacy body".to_string();
        store
            .save_full_session_for_migration_or_restore(temp.path(), &loaded)
            .unwrap();

        let parts =
            load_post_turn_base_parts_from_store(&store, app.handle(), &session.id, &message.id)
                .unwrap();

        assert_eq!(parts.len(), 1);
        assert!(matches!(
            &parts[0],
            MessagePart::Text { content, parent_tool_use_id: None } if content == "legacy body"
        ));
    }

    #[test]
    fn save_session_context_carry_returns_update_payload_when_state_changes() {
        let temp = tempfile::tempdir().unwrap();
        let store = crate::test_support::build_session_store();
        let mut session = create_session_internal(
            &store,
            temp.path(),
            "/repo",
            Some(CLAUDE_BACKEND_ID.to_string()),
        )
        .unwrap();
        session.agent_session_id = Some("sdk-session".to_string());
        store
            .save_full_session_for_migration_or_restore(temp.path(), &session)
            .unwrap();

        let update =
            save_session_context_carry(&store, temp.path(), &session.id, ContextCarryState::Failed)
                .unwrap()
                .unwrap();

        assert_eq!(update.chat_session_id, session.id);
        assert_eq!(update.agent_session_id.as_deref(), Some("sdk-session"));
        assert_eq!(update.context_carry, Some(ContextCarryState::Failed));
        assert!(update.updated_at >= session.updated_at);
        assert!(save_session_context_carry(
            &store,
            temp.path(),
            &session.id,
            ContextCarryState::Failed
        )
        .unwrap()
        .is_none());
    }

    #[test]
    fn context_carry_persistence_does_not_read_message_chunks() {
        let temp = tempfile::tempdir().unwrap();
        let store = crate::test_support::build_session_store();
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
            "hello",
            None,
            None,
        )
        .unwrap();
        let chunk = temp
            .path()
            .join("sessions")
            .join(&session.id)
            .join("messages")
            .join("1.json");
        std::fs::write(chunk, "{not valid json").unwrap();

        let update =
            save_session_context_carry(&store, temp.path(), &session.id, ContextCarryState::Failed)
                .unwrap()
                .unwrap();

        assert_eq!(update.context_carry, Some(ContextCarryState::Failed));
        let meta = store
            .get_session_meta(temp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(meta.context_carry, Some(ContextCarryState::Failed));
    }

    #[test]
    fn session_ready_resume_mismatch_requires_requested_id_to_match_ready_id() {
        assert!(!session_ready_resume_mismatch(
            Some(&ContextCarryState::Reinjected),
            Some("resume-1"),
            Some("new-1")
        ));
        assert!(!session_ready_resume_mismatch(
            Some(&ContextCarryState::Resumed),
            Some("resume-1"),
            Some("resume-1")
        ));
        assert!(session_ready_resume_mismatch(
            Some(&ContextCarryState::Resumed),
            Some("resume-1"),
            Some("new-1")
        ));
        assert!(session_ready_resume_mismatch(
            Some(&ContextCarryState::Resumed),
            Some("resume-1"),
            None
        ));
    }

    #[test]
    fn defer_agent_session_id_persist_flag_is_internal() {
        let mut msg = serde_json::json!({
            "type": "session_ready",
            "session_id": "sdk-session",
        });
        msg[DEFER_AGENT_SESSION_ID_PERSIST_ON_READY] = serde_json::Value::Bool(true);

        assert!(take_defer_agent_session_id_persist_on_ready(&mut msg));
        assert!(msg.get(DEFER_AGENT_SESSION_ID_PERSIST_ON_READY).is_none());

        let mut msg_without_flag = serde_json::json!({ "type": "session_ready" });
        assert!(!take_defer_agent_session_id_persist_on_ready(
            &mut msg_without_flag
        ));
    }

    #[test]
    fn persisted_streaming_parts_reports_tool_output_ref_without_seq_policy() {
        let ref_part = MessagePart::ToolResult {
            content: "preview".to_string(),
            is_error: false,
            tool_use_id: Some("tool-1".to_string()),
            parent_tool_use_id: None,
            content_ref: Some(crate::usecase::agent_session::session::ToolOutputRef {
                id: "a".repeat(64),
                byte_size: 123,
            }),
            summary: Some(crate::usecase::agent_session::session::ToolOutputSummary {
                line_count: 1,
                byte_size: 123,
                is_error: false,
                truncated: true,
            }),
        };
        let inline_part = MessagePart::ToolResult {
            content: "inline".to_string(),
            is_error: false,
            tool_use_id: Some("tool-2".to_string()),
            parent_tool_use_id: None,
            content_ref: None,
            summary: None,
        };

        let persisted = PersistedStreamingParts::from_parts(vec![ref_part]);
        assert!(persisted.has_tool_output_ref);
        let persisted = PersistedStreamingParts::from_parts(vec![inline_part]);
        assert!(!persisted.has_tool_output_ref);
    }

    #[tokio::test]
    async fn external_session_ready_can_defer_agent_session_id_persistence() {
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
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let mut proc = make_test_agent_process();
        proc.backend_id = CODEX_BACKEND_ID.to_string();
        proc.state = BridgeState::Initializing;
        handles.lock().await.insert(session.id.clone(), proc);
        let mut state = ExternalBridgeMessageState::default();

        let mut ready_message = serde_json::json!({
            "type": "session_ready",
            "session_id": "new-codex-thread",
        });
        ready_message[DEFER_AGENT_SESSION_ID_PERSIST_ON_READY] = serde_json::Value::Bool(true);

        handle_external_bridge_message(
            app.handle(),
            &store,
            &handles,
            &session.id,
            ready_message,
            &mut state,
        )
        .await;

        let loaded = store
            .load_full_session_for_restore(temp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(loaded.agent_session_id, None);
        let removed = handles.lock().await.remove(&session.id);
        if let Some(mut proc) = removed {
            assert_eq!(proc.sdk_session_id.as_deref(), Some("new-codex-thread"));
            assert_eq!(proc.state, BridgeState::Ready);
            let _ = proc.child.kill().await;
        }
    }

    #[tokio::test]
    async fn stale_session_ready_token_does_not_update_resume_state() {
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
        session.context_carry = Some(ContextCarryState::Resumed);
        store
            .save_full_session_for_migration_or_restore(temp.path(), &session)
            .unwrap();

        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let mut proc = make_test_agent_process();
        proc.backend_id = CLAUDE_BACKEND_ID.to_string();
        proc.state = BridgeState::Streaming;
        proc.turn_phase = TurnPhase::Streaming;
        proc.active_turn_token = Some("new-agent-message".to_string());
        proc.sdk_session_id = Some("new-sdk-session".to_string());
        proc.context_carry_on_ready = Some(ContextCarryState::Resumed);
        handles.lock().await.insert(session.id.clone(), proc);
        let mut state = ExternalBridgeMessageState::default();

        handle_external_bridge_message(
            app.handle(),
            &store,
            &handles,
            &session.id,
            serde_json::json!({
                "type": "session_ready",
                "session_id": "old-sdk-session",
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
        assert_eq!(loaded.context_carry, Some(ContextCarryState::Resumed));
        let removed = handles.lock().await.remove(&session.id);
        if let Some(mut proc) = removed {
            assert_eq!(proc.state, BridgeState::Streaming);
            assert_eq!(proc.turn_phase, TurnPhase::Streaming);
            assert_eq!(proc.sdk_session_id.as_deref(), Some("new-sdk-session"));
            assert_eq!(proc.active_turn_token.as_deref(), Some("new-agent-message"));
            assert_eq!(
                proc.context_carry_on_ready,
                Some(ContextCarryState::Resumed)
            );
            let _ = proc.child.kill().await;
        }
    }

    #[tokio::test]
    async fn external_turn_complete_persists_successful_session_id() {
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
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let mut proc = make_test_agent_process();
        proc.backend_id = CODEX_BACKEND_ID.to_string();
        proc.state = BridgeState::Streaming;
        proc.turn_phase = TurnPhase::Streaming;
        proc.streaming_message_id = Some("agent-message-1".to_string());
        proc.active_turn_token = Some("agent-message-1".to_string());
        begin_test_turn_event_log(&mut proc);
        handles.lock().await.insert(session.id.clone(), proc);
        let mut state = ExternalBridgeMessageState::default();

        handle_external_bridge_message(
            app.handle(),
            &store,
            &handles,
            &session.id,
            serde_json::json!({
                "type": "turn_complete",
                "session_id": "new-codex-thread",
                "exit_code": 0,
                "turn_token": "agent-message-1",
            }),
            &mut state,
        )
        .await;

        let loaded = store
            .load_full_session_for_restore(temp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(loaded.agent_session_id.as_deref(), Some("new-codex-thread"));
        let removed = handles.lock().await.remove(&session.id);
        if let Some(mut proc) = removed {
            assert_eq!(proc.state, BridgeState::Ready);
            let _ = proc.child.kill().await;
        }
    }

    #[tokio::test]
    async fn interrupted_turn_complete_does_not_persist_session_id_or_overwrite_done_state() {
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
        session.state = crate::usecase::agent_session::session::SessionState::Done;
        store
            .save_full_session_for_migration_or_restore(temp.path(), &session)
            .unwrap();

        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let mut proc = make_test_agent_process();
        proc.backend_id = CLAUDE_BACKEND_ID.to_string();
        proc.state = BridgeState::Streaming;
        proc.turn_phase = TurnPhase::Streaming;
        proc.streaming_message_id = Some("agent-message-1".to_string());
        proc.active_turn_token = Some("agent-message-1".to_string());
        proc.sdk_session_id = Some("interrupted-sdk-session".to_string());
        handles.lock().await.insert(session.id.clone(), proc);
        let mut state = ExternalBridgeMessageState::default();

        handle_external_bridge_message(
            app.handle(),
            &store,
            &handles,
            &session.id,
            serde_json::json!({
                "type": "turn_complete",
                "session_id": "interrupted-sdk-session",
                "exit_code": 0,
                "interrupted": true,
                "turn_token": "agent-message-1",
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
        assert_eq!(
            loaded.state,
            crate::usecase::agent_session::session::SessionState::Done
        );
        let removed = handles.lock().await.remove(&session.id);
        if let Some(mut proc) = removed {
            assert_eq!(proc.state, BridgeState::Ready);
            assert_eq!(proc.turn_phase, TurnPhase::Idle);
            assert_eq!(
                proc.sdk_session_id.as_deref(),
                Some("previous-good-session")
            );
            assert!(proc.active_turn_token.is_none());
            assert!(proc.post_turn_message_token.is_none());
            let _ = proc.child.kill().await;
        }
    }

    #[tokio::test]
    async fn interrupted_turn_complete_drains_pending_starts_new_turn_and_fences_old_token() {
        use std::sync::atomic::{AtomicUsize, Ordering};

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
        let old_turn_token = "old-agent-message";
        let mut proc = make_test_agent_process();
        proc.backend_id = CLAUDE_BACKEND_ID.to_string();
        proc.state = BridgeState::Streaming;
        proc.turn_phase = TurnPhase::Streaming;
        proc.streaming_message_id = Some(old_turn_token.to_string());
        proc.active_turn_token = Some(old_turn_token.to_string());
        proc.sdk_session_id = Some("interrupted-sdk-session".to_string());
        proc.streaming_parts.push(MessagePart::Text {
            content: "old response".to_string(),
            parent_tool_use_id: None,
        });
        begin_turn_event_log(
            &mut proc,
            "human-old",
            test_prompt_input("old prompt"),
            old_turn_token,
            1.0,
        );
        proc.pending_messages
            .push_back(test_pending_message("queued-followup", "next prompt"));
        handles.lock().await.insert(session.id.clone(), proc);

        let interrupted = true;
        let (final_parts, workflow_turn_complete) = {
            let mut map = handles.lock().await;
            let proc = map.get_mut(&session.id).unwrap();
            let effect = run_turn_complete_transition_locked(
                proc,
                &session.id,
                0,
                |_mid, _seq, _snapshot, _parts| true,
            );
            proc.sdk_session_id = Some("previous-good-session".to_string());
            proc.post_turn_message_token = None;
            assert!(effect.turn_completed);
            assert_eq!(proc.state, BridgeState::Ready);
            assert_eq!(proc.turn_phase, TurnPhase::Idle);
            assert!(proc.active_turn_token.is_none());
            (effect.final_parts, effect.workflow_turn_complete)
        };

        let loaded = store
            .load_full_session_for_restore(temp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(
            loaded.agent_session_id.as_deref(),
            Some("previous-good-session")
        );

        let pending = take_pending_message(&handles, &session.id)
            .await
            .expect("pending follow-up turn should be drained");
        assert!(!agent_session_has_pending_message(&handles, &session.id).await);

        let (_human_msg, agent_msg, emit_consumed_messages) =
            prepare_pending_turn_messages(&store, temp.path(), &session.id, &pending).unwrap();
        assert!(emit_consumed_messages);
        assert_ne!(agent_msg.id, old_turn_token);

        let spawn_count = Arc::new(AtomicUsize::new(0));
        start_agent_turn_with_runtime_spawner(
            None::<&tauri::AppHandle>,
            None,
            None,
            &handles,
            &session.id,
            &pending.permission_mode,
            &pending.content,
            &agent_msg.id,
            &pending.images,
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

        assert_eq!(
            spawn_count.load(Ordering::SeqCst),
            0,
            "existing bridge runtime should be reused for the pending turn"
        );
        {
            let map = handles.lock().await;
            let proc = map.get(&session.id).unwrap();
            assert_eq!(proc.state, BridgeState::Streaming);
            assert_eq!(proc.turn_phase, TurnPhase::Streaming);
            assert_eq!(
                proc.streaming_message_id.as_deref(),
                Some(agent_msg.id.as_str())
            );
            assert_eq!(
                proc.active_turn_token.as_deref(),
                Some(agent_msg.id.as_str())
            );
            assert!(proc.streaming_parts.is_empty());
        }

        let mut state = ExternalBridgeMessageState::default();
        handle_external_bridge_message(
            app.handle(),
            &store,
            &handles,
            &session.id,
            serde_json::json!({
                "type": "stream_event",
                "turn_token": old_turn_token,
                "event": {
                    "type": "content_block_delta",
                    "delta": {"type": "text_delta", "text": " stale tail"}
                }
            }),
            &mut state,
        )
        .await;
        {
            let map = handles.lock().await;
            let proc = map.get(&session.id).unwrap();
            assert!(
                proc.streaming_parts.is_empty(),
                "old token stream must not append to the new turn"
            );
            assert_eq!(
                proc.active_turn_token.as_deref(),
                Some(agent_msg.id.as_str())
            );
        }

        handle_external_bridge_message(
            app.handle(),
            &store,
            &handles,
            &session.id,
            serde_json::json!({
                "type": "error",
                "turn_token": old_turn_token,
                "message": "stale error"
            }),
            &mut state,
        )
        .await;
        {
            let map = handles.lock().await;
            let proc = map.get(&session.id).unwrap();
            assert_eq!(proc.state, BridgeState::Streaming);
            assert_eq!(proc.turn_phase, TurnPhase::Streaming);
            assert!(
                proc.streaming_parts.is_empty(),
                "old token error must not append to or complete the new turn"
            );
            assert_eq!(
                proc.active_turn_token.as_deref(),
                Some(agent_msg.id.as_str())
            );
        }

        handle_external_bridge_message(
            app.handle(),
            &store,
            &handles,
            &session.id,
            serde_json::json!({
                "type": "turn_complete",
                "session_id": "old-sdk-session",
                "exit_code": 0,
                "turn_token": old_turn_token,
            }),
            &mut state,
        )
        .await;
        {
            let map = handles.lock().await;
            let proc = map.get(&session.id).unwrap();
            assert_eq!(proc.state, BridgeState::Streaming);
            assert_eq!(proc.turn_phase, TurnPhase::Streaming);
            assert_eq!(
                proc.active_turn_token.as_deref(),
                Some(agent_msg.id.as_str())
            );
        }

        let workflow_gateway = Arc::new(RecordingWorkflowTurnCompleteGateway {
            session_running: true,
            ..Default::default()
        });
        let workflow = crate::usecase::workflow::turn_complete::WorkflowTurnCompleteUsecase::new(
            workflow_gateway.clone(),
        );
        workflow
            .complete_turn(
                crate::usecase::workflow::ports::WorkflowTurnCompleteNotification {
                    chat_session_id: session.id.clone(),
                    exit_code: 0,
                    final_text_parts: workflow_final_text_parts(&final_parts),
                    failure_signal: workflow_turn_complete
                        .as_ref()
                        .and_then(|input| input.failure_signal)
                        .map(|signal| match signal {
                            crate::usecase::agent_session::event_log::AgentTurnFailureSignal::ModelRefusal => {
                                crate::usecase::workflow::ports::WorkflowTurnFailureSignal::ModelRefusal
                            }
                        }),
                    token_usage: workflow_turn_complete.as_ref().and_then(|input| {
                        input.token_usage.map(|usage| {
                            crate::usecase::workflow::ports::WorkflowTurnTokenUsage {
                                input_tokens: usage.input_tokens,
                                output_tokens: usage.output_tokens,
                            }
                        })
                    }),
                    interrupted,
                },
            )
            .await
            .unwrap();
        assert_eq!(
            workflow_gateway.calls.lock().unwrap().as_slice(),
            ["is_running"],
            "interrupted workflow notification must not complete the workflow turn"
        );

        clear_pending_turn_starting(&session.id).await;
        let removed = handles.lock().await.remove(&session.id);
        if let Some(mut proc) = removed {
            let _ = proc.child.kill().await;
        }
    }

    #[tokio::test]
    async fn external_session_ready_mismatch_prepares_reinject_and_crashes_process() {
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
        session.agent_session_id = Some("stale-sdk-session".to_string());
        session.context_carry = Some(ContextCarryState::Resumed);
        store
            .save_full_session_for_migration_or_restore(temp.path(), &session)
            .unwrap();
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let mut proc = make_test_agent_process();
        proc.state = BridgeState::Initializing;
        proc.sdk_session_id = Some("stale-sdk-session".to_string());
        proc.context_carry_on_ready = Some(ContextCarryState::Resumed);
        handles.lock().await.insert(session.id.clone(), proc);
        let mut state = ExternalBridgeMessageState::default();

        handle_external_bridge_message(
            app.handle(),
            &store,
            &handles,
            &session.id,
            serde_json::json!({
                "type": "session_ready",
                "session_id": "new-sdk-session"
            }),
            &mut state,
        )
        .await;

        let loaded = store
            .load_full_session_for_restore(temp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(loaded.agent_session_id, None);
        assert_eq!(loaded.context_carry, None);
        let removed = handles.lock().await.remove(&session.id);
        if let Some(mut proc) = removed {
            assert_eq!(proc.sdk_session_id, None);
            assert_eq!(proc.context_carry_on_ready, None);
            assert_eq!(proc.state, BridgeState::Crashed);
            let _ = proc.child.kill().await;
        }
    }

    #[tokio::test]
    async fn session_ready_streaming_resume_mismatch_requeues_current_turn_for_reinject() {
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
        session.agent_session_id = Some("stale-sdk-session".to_string());
        session.context_carry = Some(ContextCarryState::Resumed);
        session.messages = vec![
            ChatMessage {
                id: "prior-human".to_string(),
                role: MessageRole::Human,
                content: "remember alpha".to_string(),
                thinking: None,
                activities: None,
                parts: None,
                streaming_final_seq: 0,
                timestamp: 1.0,
                mentions: None,
            },
            ChatMessage {
                id: "prior-agent".to_string(),
                role: MessageRole::Agent,
                content: "alpha is set".to_string(),
                thinking: None,
                activities: None,
                parts: None,
                streaming_final_seq: 0,
                timestamp: 2.0,
                mentions: None,
            },
            ChatMessage {
                id: "current-human".to_string(),
                role: MessageRole::Human,
                content: "what was it?".to_string(),
                thinking: None,
                activities: None,
                parts: None,
                streaming_final_seq: 0,
                timestamp: 3.0,
                mentions: None,
            },
            ChatMessage {
                id: "current-agent".to_string(),
                role: MessageRole::Agent,
                content: String::new(),
                thinking: None,
                activities: None,
                parts: None,
                streaming_final_seq: 0,
                timestamp: 4.0,
                mentions: None,
            },
        ];
        store
            .save_full_session_for_migration_or_restore(temp.path(), &session)
            .unwrap();
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let mut proc = make_test_agent_process();
        proc.state = BridgeState::Streaming;
        proc.turn_phase = TurnPhase::Streaming;
        proc.sdk_session_id = Some("stale-sdk-session".to_string());
        proc.context_carry_on_ready = Some(ContextCarryState::Resumed);
        proc.streaming_message_id = Some("current-agent".to_string());
        handles.lock().await.insert(session.id.clone(), proc);

        let candidate = {
            let mut map = handles.lock().await;
            let proc = map.get_mut(&session.id).unwrap();
            let context_carry_on_ready = proc.context_carry_on_ready.take();
            assert!(session_ready_resume_mismatch(
                context_carry_on_ready.as_ref(),
                proc.sdk_session_id.as_deref(),
                Some("new-sdk-session"),
            ));
            streaming_turn_requeue_candidate(proc).expect("streaming candidate")
        };
        assert!(
            requeue_streaming_turn_for_resume_mismatch(
                app.handle(),
                &handles,
                &store,
                &session.id,
                candidate,
            )
            .await
        );
        persist_resume_mismatch_for_reinject(app.handle(), &store, &session.id);
        crash_agent_process_for_context_reinject(app.handle(), &handles, &session.id).await;

        let loaded = store
            .load_full_session_for_restore(temp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(loaded.agent_session_id, None);
        assert_eq!(loaded.context_carry, None);
        let ContextRestorePlan::Reinject { payload } =
            context_restore_plan_for_session_before_turn(Some(&loaded), "current-agent")
        else {
            panic!("expected reinject plan before current turn");
        };
        assert!(payload.prompt_prefix.contains("remember alpha"));
        assert!(!payload.prompt_prefix.contains("what was it?"));

        let mut proc = handles.lock().await.remove(&session.id).unwrap();
        assert_eq!(proc.state, BridgeState::Crashed);
        assert_eq!(proc.turn_phase, TurnPhase::Idle);
        assert_eq!(proc.sdk_session_id, None);
        assert_eq!(proc.pending_messages.len(), 1);
        let pending = proc.pending_messages.pop_front().unwrap();
        assert_eq!(pending.content, "what was it?");
        assert_eq!(
            pending.existing_human_message_id.as_deref(),
            Some("current-human")
        );
        assert_eq!(
            pending.existing_agent_message_id.as_deref(),
            Some("current-agent")
        );
        let _ = proc.child.kill().await;
    }

    #[tokio::test]
    async fn initializing_resume_mismatch_has_no_streaming_requeue_candidate() {
        let mut proc = make_test_agent_process();
        proc.state = BridgeState::Initializing;
        proc.sdk_session_id = Some("stale-sdk-session".to_string());
        proc.context_carry_on_ready = Some(ContextCarryState::Resumed);

        assert!(session_ready_resume_mismatch(
            proc.context_carry_on_ready.as_ref(),
            proc.sdk_session_id.as_deref(),
            Some("new-sdk-session"),
        ));
        assert!(streaming_turn_requeue_candidate(&proc).is_none());
        let _ = proc.child.kill().await;
    }

    #[test]
    fn persist_context_carry_failed_can_force_failed_before_success_state() {
        let temp = tempfile::tempdir().unwrap();
        let app = tauri::test::mock_builder()
            .manage(crate::infrastructure::platform::app_data_dir::TestDataDir(
                temp.path().to_path_buf(),
            ))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let store = crate::test_support::build_session_store();
        let mut session = create_session_internal(
            &store,
            temp.path(),
            "/repo",
            Some(CLAUDE_BACKEND_ID.to_string()),
        )
        .unwrap();
        session.agent_session_id = Some("stale-sdk-session".to_string());
        store
            .save_full_session_for_migration_or_restore(temp.path(), &session)
            .unwrap();

        persist_context_carry_failed_after_init_error(
            app.handle(),
            &store,
            &session.id,
            true,
            true,
        );

        let loaded = store
            .load_full_session_for_restore(temp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(loaded.agent_session_id, None);
        assert_eq!(
            loaded.context_carry,
            Some(crate::usecase::agent_session::session::ContextCarryState::Failed)
        );
    }

    #[tokio::test]
    async fn bridge_eof_initializing_pending_context_carry_persists_failed() {
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
        session.agent_session_id = Some("stale-sdk-session".to_string());
        session.context_carry = Some(ContextCarryState::Resumed);
        store
            .save_full_session_for_migration_or_restore(temp.path(), &session)
            .unwrap();
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let mut proc = make_test_agent_process();
        proc.state = BridgeState::Initializing;
        proc.generation_id = 42;
        proc.sdk_session_id = Some("stale-sdk-session".to_string());
        proc.context_carry_on_ready = Some(ContextCarryState::Resumed);
        handles.lock().await.insert(session.id.clone(), proc);

        let (was_initializing, context_carry_failed_after_init_error) = {
            let mut map = handles.lock().await;
            let proc = map.get_mut(&session.id).unwrap();
            let generation_matches = proc.generation_id == 42;
            let transition = run_bridge_eof_crash_transition_locked(
                generation_matches,
                proc,
                &session.id,
                |_mid, _seq, _snapshot, _parts| true,
            );
            (
                transition.was_initializing,
                transition.context_restore_failed_on_init,
            )
        };
        assert!(was_initializing);
        assert!(context_carry_failed_after_init_error);
        persist_context_carry_failed_after_init_error(
            app.handle(),
            &store,
            &session.id,
            true,
            true,
        );

        let loaded = store
            .load_full_session_for_restore(temp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(loaded.agent_session_id, None);
        assert_eq!(loaded.context_carry, Some(ContextCarryState::Failed));
        let mut proc = handles.lock().await.remove(&session.id).unwrap();
        assert_eq!(proc.context_carry_on_ready, None);
        assert_eq!(proc.state, BridgeState::Crashed);
        assert_eq!(proc.turn_phase, TurnPhase::Idle);
        let _ = proc.child.kill().await;
    }

    #[tokio::test]
    async fn bridge_eof_initializing_without_pending_context_carry_does_not_persist_failed() {
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
        session.agent_session_id = Some("existing-sdk-session".to_string());
        session.context_carry = None;
        store
            .save_full_session_for_migration_or_restore(temp.path(), &session)
            .unwrap();
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let mut proc = make_test_agent_process();
        proc.state = BridgeState::Initializing;
        proc.generation_id = 7;
        proc.sdk_session_id = Some("existing-sdk-session".to_string());
        proc.context_carry_on_ready = None;
        handles.lock().await.insert(session.id.clone(), proc);

        let context_carry_failed_after_init_error = {
            let mut map = handles.lock().await;
            let proc = map.get_mut(&session.id).unwrap();
            let generation_matches = proc.generation_id == 7;
            let transition = run_bridge_eof_crash_transition_locked(
                generation_matches,
                proc,
                &session.id,
                |_mid, _seq, _snapshot, _parts| true,
            );
            assert!(transition.was_initializing);
            transition.context_restore_failed_on_init
        };
        assert!(!context_carry_failed_after_init_error);
        if context_carry_failed_after_init_error {
            persist_context_carry_failed_after_init_error(
                app.handle(),
                &store,
                &session.id,
                true,
                true,
            );
        }

        let loaded = store
            .load_full_session_for_restore(temp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(
            loaded.agent_session_id.as_deref(),
            Some("existing-sdk-session")
        );
        assert_eq!(loaded.context_carry, None);
        let mut proc = handles.lock().await.remove(&session.id).unwrap();
        let _ = proc.child.kill().await;
    }

    #[test]
    fn resolve_spawn_info_without_registry_keeps_none() {
        // registry 未指定（テスト等）では selected_model=None は None のまま。
        let session = make_chat_session_for_spawn(None, None, CODEX_BACKEND_ID);
        let info = resolve_spawn_info(Some(session), None);
        assert_eq!(info.resume_sid, None);
        assert_eq!(info.selected_model, None);
        assert_eq!(info.backend_id, CODEX_BACKEND_ID.to_string());
    }

    #[test]
    fn resolve_spawn_info_resolves_none_to_default_with_registry() {
        // モデル未選択状態は廃止。selected_model=None は registry の既定モデルへ解決する。
        let registry = make_fixed_model_registry();
        let session = make_chat_session_for_spawn(None, None, CODEX_BACKEND_ID);
        let info = resolve_spawn_info(Some(session), Some(&registry));
        assert_eq!(
            info.selected_model,
            Some(crate::domain::agent_session::CODEX_FIXED_MODELS[0].to_string())
        );
    }

    #[test]
    fn resolve_spawn_info_preserves_existing_selected_model() {
        // 永続化済みの selected_model はそのまま採用する（既定で上書きしない）。
        let registry = make_fixed_model_registry();
        let session = make_chat_session_for_spawn(
            None,
            Some(crate::domain::agent_session::CODEX_FIXED_MODELS[1].to_string()),
            CODEX_BACKEND_ID,
        );
        let info = resolve_spawn_info(Some(session), Some(&registry));
        assert_eq!(
            info.selected_model,
            Some(crate::domain::agent_session::CODEX_FIXED_MODELS[1].to_string())
        );
    }

    #[test]
    fn resolve_spawn_info_uses_default_backend_when_session_missing() {
        // 永続化セッションが存在しない場合は新規セッション扱い。
        // backend_id は claude（既定）にフォールバックし、selected_model も claude の既定へ解決する。
        let registry = make_fixed_model_registry();
        let info = resolve_spawn_info(None, Some(&registry));
        assert_eq!(
            info.selected_model,
            Some(crate::domain::agent_session::CLAUDE_FIXED_MODELS[0].to_string())
        );
        assert_eq!(info.backend_id, CLAUDE_BACKEND_ID.to_string());
    }
}
