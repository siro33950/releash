use super::process_registry::{
    AgentProcess, AgentProcessMap, BridgeState, StreamPartRollback, TurnPhase,
};
use super::shared::{
    bridge_permission_fields, notify_status_transition, write_bridge_command, CODEX_BACKEND_ID,
};
use super::stream_emit::{
    emit_session_state_changed, emit_streaming_delta, enqueue_pending_delta_with_rollbacks,
    flush_streaming_before_transition, force_flush_pending_streaming, spawn_streaming_timer,
};
use super::turn_event_log::{
    projected_session_state_for_current_turn, record_permission_resolution_for_current_turn,
};
use crate::infrastructure::agent_session::runtime::turn_latency;
use crate::infrastructure::platform::app_data_dir::resolve_data_dir;
use crate::usecase::agent_session::session::MessagePart;
use crate::usecase::agent_session::session::SessionState;
use crate::usecase::agent_session::session::SessionStore;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;

pub(super) fn emit_permission_mode_changed<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    chat_session_id: &str,
    mode: &str,
) {
    use tauri::Emitter;
    let _ = app.emit(
        "agent-permission-mode-changed",
        serde_json::json!({
            "chat_session_id": chat_session_id,
            "permission_mode": mode,
        }),
    );
}

/// SDK 由来の `permissionMode` 通知を保存値ベースで処理する。
/// Spec issues-947: 保存値の読み取り失敗時は SDK 値に fallback せず、log::error! を残して
/// runtime/UI を更新せずに通知の処理だけスキップする（保存値が edit/full のセッションを
/// 誤って ask に落とさないため）。後段の `write_bridge_command` 失敗も log に記録する。
pub(super) async fn handle_sdk_permission_mode_notification<R: tauri::Runtime>(
    sdk_mode: &str,
    app: &tauri::AppHandle<R>,
    session_store: &std::sync::Arc<crate::usecase::agent_session::session::SessionStore>,
    handles: &std::sync::Arc<
        tokio::sync::Mutex<
            crate::infrastructure::agent_session::runtime::bridge_common::AgentProcessMap,
        >,
    >,
    chat_session_id: &str,
) {
    let sdk_abstract =
        crate::infrastructure::agent_session::runtime::permission_flags::mode_from_claude_flag(
            sdk_mode,
        );
    let data_dir = match crate::infrastructure::platform::app_data_dir::resolve_data_dir(app) {
        Ok(dir) => dir,
        Err(e) => {
            log::error!(
                "Failed to resolve data dir for SDK permissionMode notification \
                 (chat_session_id={chat_session_id}): {e}"
            );
            return;
        }
    };
    let saved_meta = match session_store.get_session_meta(&data_dir, chat_session_id) {
        Ok(meta) => meta,
        Err(e) => {
            log::error!(
                "Failed to read saved session metadata for SDK permissionMode notification \
                 (chat_session_id={chat_session_id}): {e}"
            );
            return;
        }
    };
    let Some(meta) = saved_meta else {
        log::error!(
            "Saved session not found for SDK permissionMode notification \
             (chat_session_id={chat_session_id})"
        );
        return;
    };
    let canonical_mode =
        match crate::domain::agent_session::PermissionMode::parse(&meta.permission_mode) {
            Ok(mode) => mode,
            Err(e) => {
                log::error!(
                    "Saved permission_mode is invalid for SDK permissionMode notification \
                     (chat_session_id={chat_session_id}): {e}"
                );
                return;
            }
        };
    let canonical_str = canonical_mode.as_str();
    let (backend_for_resync, needs_resync) = {
        let mut map = handles.lock().await;
        if let Some(proc) = map.get_mut(chat_session_id) {
            proc.current_permission_mode = canonical_str.to_string();
            let backend_id = proc.backend_id.clone();
            let resync = sdk_abstract.is_some_and(|mode| mode != canonical_mode);
            (Some(backend_id), resync)
        } else {
            (None, false)
        }
    };
    emit_permission_mode_changed(app, chat_session_id, canonical_str);
    if !needs_resync {
        return;
    }
    let Some(backend_id) = backend_for_resync else {
        return;
    };
    let payload = build_set_mode_payload_for_mode(canonical_mode, &backend_id);
    if let Err(e) = write_bridge_command(handles, chat_session_id, payload).await {
        log::error!(
            "Failed to resync permission mode to bridge \
             (chat_session_id={chat_session_id}, backend_id={backend_id}): {e}"
        );
    }
}

/// 抽象モード文字列（"ask"/"edit"/"full"）→ バックエンド固有の setMode コマンドを生成する。
/// 対象外の値が渡された場合はエラー（境界で検証済みを前提）。
pub(super) fn build_set_mode_command_for_backend(
    permission_mode: &str,
    backend_id: &str,
) -> Result<String, String> {
    let pm = crate::domain::agent_session::PermissionMode::parse(permission_mode)
        .map_err(|e| e.to_string())?;
    Ok(build_set_mode_command_for_mode(pm, backend_id))
}

pub(super) fn build_set_mode_payload_for_mode(
    pm: crate::domain::agent_session::PermissionMode,
    backend_id: &str,
) -> serde_json::Value {
    let mut payload = serde_json::json!({ "type": "setMode" });
    let obj = payload
        .as_object_mut()
        .expect("setMode payload is an object");
    for (k, v) in bridge_permission_fields(pm, backend_id, false) {
        obj.insert(k, v);
    }
    payload
}

pub(super) fn build_set_mode_command_for_mode(
    pm: crate::domain::agent_session::PermissionMode,
    backend_id: &str,
) -> String {
    format!("{}\n", build_set_mode_payload_for_mode(pm, backend_id))
}

/// Write setMode commands to the Bridge stdin before a turn starts.
pub(super) async fn sync_pre_turn_settings(
    proc: &mut AgentProcess,
    permission_mode: &str,
) -> Result<(), String> {
    let mode_data = build_set_mode_command_for_backend(permission_mode, &proc.backend_id)?;
    let mut stdin = proc.stdin.lock().await;
    stdin
        .write_all(mode_data.as_bytes())
        .await
        .map_err(|e| format!("Failed to write setMode: {e}"))?;
    stdin
        .flush()
        .await
        .map_err(|e| format!("Failed to flush setMode: {e}"))?;

    Ok(())
}

/// Effect returned by `run_permission_request_transition_locked`.
/// `did_transition` reports whether the process actually moved from
/// `Streaming` to `WaitingPermission`; production still mirrors the pending
/// permission request even when a late/out-of-order request arrives after the
/// process has already left `Streaming`.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct PermissionRequestTransition {
    pub(crate) did_transition: bool,
    pub(crate) projected_session_state: Option<SessionState>,
}

/// Run the in-lock part of the `permission_request` transition: force-flush
/// the pending streaming delta first, then — only when the process was in
/// `Streaming` — promote `turn_phase` to `WaitingPermission`. The flush runs
/// before the state mutation so the frontend never observes a state change
/// ahead of the tail content. The caller is responsible for emitting the
/// pending permission state outside the lock.
pub(super) fn run_permission_request_transition_locked<F>(
    proc: &mut AgentProcess,
    chat_session_id: &str,
    request_id: Option<&str>,
    request_received_at: Instant,
    emit_stream: F,
) -> PermissionRequestTransition
where
    F: FnMut(&str, u64, bool, &[MessagePart]) -> bool,
{
    if proc.state == BridgeState::Streaming {
        if let Some(request_id) = request_id {
            turn_latency::begin_permission_wait_latency(
                &mut proc.turn_latency,
                request_id,
                request_received_at,
            );
        }
    }
    let turn_completed = flush_streaming_before_transition(proc, chat_session_id, emit_stream);
    if turn_completed {
        proc.turn_phase = TurnPhase::WaitingPermission;
        proc.mark_turn_phase_since_now();
    }
    PermissionRequestTransition {
        did_transition: turn_completed,
        projected_session_state: projected_session_state_for_current_turn(proc),
    }
}

/// Effect returned by `apply_respond_permission_locked`. `did_transition` is
/// `true` only when the process was actually in `WaitingPermission`; this
/// gates both the post-lock `agent-session-state-changed(Streaming)`
/// emission and the per-turn auxiliary timer restart.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct PermissionResponseTransition {
    pub(crate) did_transition: bool,
    pub(crate) projected_session_state: Option<SessionState>,
}

/// Run the in-lock part of `respond_agent_permission`: flip `turn_phase`
/// back to `Streaming` (only when actually waiting), patch the matching
/// `Permission` part status in the streaming buffer, enqueue the updated
/// part as a pending delta, and force-flush so the frontend observes the
/// permission decision before the state-change notification. The caller
/// handles stdin write before, and timer restart / state-change emission
/// after.
pub(super) fn apply_respond_permission_locked<F>(
    proc: &mut AgentProcess,
    chat_session_id: &str,
    request_id: &str,
    behavior: &str,
    answers_value: Option<&serde_json::Value>,
    mut emit_stream: F,
) -> PermissionResponseTransition
where
    F: FnMut(&str, u64, bool, &[MessagePart]) -> bool,
{
    let did_transition = proc.turn_phase == TurnPhase::WaitingPermission;
    turn_latency::record_permission_wait_latency(&mut proc.turn_latency, request_id);
    if did_transition {
        proc.turn_phase = TurnPhase::Streaming;
        proc.mark_turn_phase_since_now();
        proc.touch_liveness();
    }
    let new_status = if behavior == "allow" {
        "allowed"
    } else {
        "denied"
    };
    let mut found_part: Option<MessagePart> = None;
    let mut rollbacks = Vec::new();
    for index in 0..proc.streaming_parts.len() {
        let matches_request = matches!(
            &proc.streaming_parts[index],
            MessagePart::Permission { request, .. }
                if request.get("request_id").and_then(|v| v.as_str()) == Some(request_id)
        );
        if matches_request {
            let previous = proc.streaming_parts[index].clone();
            if let MessagePart::Permission {
                status, answers, ..
            } = &mut proc.streaming_parts[index]
            {
                rollbacks.push(StreamPartRollback { index, previous });
                *status = new_status.to_string();
                if let Some(av) = answers_value {
                    *answers = Some(av.clone());
                }
                found_part = Some(proc.streaming_parts[index].clone());
            }
        }
    }
    let emit_msg_id = proc.streaming_message_id.clone();
    if let (Some(mid), Some(part)) = (emit_msg_id, found_part) {
        enqueue_pending_delta_with_rollbacks(proc, std::slice::from_ref(&part), rollbacks);
        force_flush_pending_streaming(proc, chat_session_id, &mid, |seq, snapshot, parts| {
            emit_stream(&mid, seq, snapshot, parts)
        });
    }
    record_permission_resolution_for_current_turn(
        proc,
        request_id,
        behavior,
        answers_value.cloned(),
    );
    PermissionResponseTransition {
        did_transition,
        projected_session_state: projected_session_state_for_current_turn(proc),
    }
}

pub async fn set_agent_permission_mode(
    app: tauri::AppHandle,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    handles: tauri::State<'_, Arc<Mutex<AgentProcessMap>>>,
    chat_session_id: String,
    permission_mode: String,
) -> Result<(), String> {
    let data_dir = resolve_data_dir(&app)?;
    set_agent_permission_mode_internal(
        session_store.inner(),
        handles.inner(),
        &data_dir,
        &chat_session_id,
        &permission_mode,
    )
    .await
}

/// `set_agent_permission_mode` の内部実装。Tauri コマンドから AppHandle 依存を切り離し、
/// 境界での invalid 値拒否（保存値・current_permission_mode・bridge stdin 不変）を
/// テストから直接検証できるようにする（Spec issues-947）。
pub(super) async fn set_agent_permission_mode_internal(
    session_store: &Arc<SessionStore>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    data_dir: &Path,
    chat_session_id: &str,
    permission_mode: &str,
) -> Result<(), String> {
    // 境界で抽象モードを検証。対象外の値はセッション状態を変更せず bridge にも送らない。
    let pm = crate::domain::agent_session::PermissionMode::parse(permission_mode)
        .map_err(|e| e.to_string())?;

    // Persist to SessionStore（検証済みの抽象モード）
    session_store.update_permission_mode(data_dir, chat_session_id, pm.as_str())?;

    {
        let mut map = handles.lock().await;
        if let Some(proc) = map.get_mut(chat_session_id) {
            let data = build_set_mode_command_for_mode(pm, &proc.backend_id);
            let mut stdin = proc.stdin.lock().await;
            stdin
                .write_all(data.as_bytes())
                .await
                .map_err(|e| format!("Failed to write setMode: {e}"))?;
            stdin
                .flush()
                .await
                .map_err(|e| format!("Failed to flush setMode: {e}"))?;
            drop(stdin);
            proc.current_permission_mode = pm.as_str().to_string();
        }
        // If no process exists, silently ignore (process not yet started)
    }

    Ok(())
}

/// `agent-models-updated` イベントの payload を組み立てる。
/// session 単位の available_models / selected_model を frontend へ同期するために使う。
#[allow(clippy::too_many_arguments)]
pub async fn respond_agent_permission(
    app: tauri::AppHandle,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    handles: tauri::State<'_, Arc<Mutex<AgentProcessMap>>>,
    registry: tauri::State<
        '_,
        Arc<crate::infrastructure::agent_session::runtime::AgentBackendRegistry>,
    >,
    chat_session_id: String,
    request_id: String,
    behavior: String,
    message: Option<String>,
    updated_input: Option<String>,
) -> Result<(), String> {
    respond_agent_permission_internal(
        &app,
        session_store.inner(),
        handles.inner(),
        registry.inner(),
        chat_session_id,
        request_id,
        behavior,
        message,
        updated_input,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn respond_agent_permission_internal(
    app: &tauri::AppHandle,
    session_store: &Arc<SessionStore>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    registry: &Arc<crate::infrastructure::agent_session::runtime::AgentBackendRegistry>,
    chat_session_id: String,
    request_id: String,
    behavior: String,
    message: Option<String>,
    updated_input: Option<String>,
) -> Result<(), String> {
    if behavior != "allow" && behavior != "deny" {
        return Err(format!("Invalid behavior: {behavior}"));
    }
    let mut result = serde_json::json!({ "behavior": behavior });
    if let Some(msg) = &message {
        result["message"] = serde_json::Value::String(msg.clone());
    }
    let answers_value =
        apply_updated_input_to_permission_result(&mut result, updated_input.as_deref())?;
    let payload = serde_json::json!({
        "type": "permission_response",
        "request_id": request_id,
        "result": result,
    });
    let data = format!("{}\n", payload);

    let backend_id = {
        let map = handles.lock().await;
        map.get(&chat_session_id)
            .map(|proc| proc.backend_id.clone())
            .ok_or_else(|| format!("No active agent process for session {chat_session_id}"))?
    };

    if backend_id == CODEX_BACKEND_ID {
        let backend = registry
            .get(&backend_id)
            .ok_or_else(|| format!("Agent backend not found: {backend_id}"))?;
        backend
            .respond_permission(
                &crate::infrastructure::agent_session::runtime::SessionHandle {
                    chat_session_id: chat_session_id.clone(),
                    backend_id: backend_id.clone(),
                },
                crate::infrastructure::agent_session::runtime::PermissionResponse {
                    request_id: request_id.clone(),
                    behavior: behavior.clone(),
                    updated_input: updated_input.clone(),
                },
            )
            .await?;
    }

    let permission_transition;
    {
        let mut map = handles.lock().await;
        if let Some(proc) = map.get_mut(&chat_session_id) {
            if backend_id != CODEX_BACKEND_ID {
                let mut stdin = proc.stdin.lock().await;
                stdin
                    .write_all(data.as_bytes())
                    .await
                    .map_err(|e| format!("Failed to write permission response: {e}"))?;
                stdin
                    .flush()
                    .await
                    .map_err(|e| format!("Failed to flush: {e}"))?;
                drop(stdin);
            }

            // Apply the synchronous part of the permission response
            // (phase flip + permission part patch + force flush) via the
            // shared helper so production and unit tests exercise the same
            // ordering: flush must complete before the state-change emit
            // outside the lock.
            let effect = apply_respond_permission_locked(
                proc,
                &chat_session_id,
                &request_id,
                &behavior,
                answers_value.as_ref(),
                |mid, seq, snapshot, parts| {
                    emit_streaming_delta(app, &chat_session_id, mid, seq, snapshot, parts.to_vec())
                },
            );
            permission_transition = effect;

            // Resuming the turn: restart the per-turn auxiliary timer if it
            // has already exited (turn left Streaming when WaitingPermission
            // was entered). Idempotent — no-op if a timer is still alive.
            if permission_transition.did_transition {
                spawn_streaming_timer(app, handles, &chat_session_id, proc);
            }
        } else {
            return Err(format!(
                "No active agent process for session {chat_session_id}"
            ));
        }
    }

    // Emit state change only if we actually transitioned: WaitingPermission → Streaming
    if permission_transition.did_transition {
        emit_session_state_changed(
            app,
            &chat_session_id,
            TurnPhase::Streaming,
            None,
            false,
            None,
        );
        notify_status_transition(
            app,
            session_store,
            &chat_session_id,
            TurnPhase::Streaming,
            permission_transition.projected_session_state,
        );
    }

    Ok(())
}

fn apply_updated_input_to_permission_result(
    result: &mut serde_json::Value,
    updated_input: Option<&str>,
) -> Result<Option<serde_json::Value>, String> {
    if let Some(input_json) = updated_input {
        let parsed = serde_json::from_str::<serde_json::Value>(input_json)
            .map_err(|e| format!("Invalid updated_input JSON: {e}"))?;
        result["updatedInput"] = parsed.clone();
        Ok(parsed.get("answers").cloned())
    } else {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::super::shared::CLAUDE_BACKEND_ID;
    use super::build_set_mode_command_for_backend;

    #[test]
    fn permission_mode_command_rejects_invalid_mode() {
        assert!(build_set_mode_command_for_backend("invalid", CLAUDE_BACKEND_ID).is_err());
    }
}
#[cfg(test)]
mod moved_tests {

    use super::super::permission::*;
    use super::super::process_registry::*;

    use super::super::shared::test_support::*;
    use super::super::shared::*;

    use super::super::turn_event_log::*;

    use crate::usecase::agent_session::session::MessagePart;

    use std::sync::Arc;

    use tokio::sync::Mutex;

    #[tokio::test]
    async fn respond_permission_orders_flush_then_state_change() {
        // Spec (Rule: ターン完了・状態遷移時には未配信バッファを強制配信する):
        //   権限待ち → ストリーミング への遷移時、Permission part 更新を
        //   含む強制 flush が state 通知より前に観測されること。
        let mut proc = make_process_waiting_for_permission("req-1");
        let mut events = Vec::new();
        let transitioned =
            drive_respond_permission_path(&mut proc, "csid", "req-1", "allow", None, &mut events);
        assert!(transitioned);

        assert_eq!(events.len(), 2, "flush emit then state emit");
        match &events[0] {
            RecordedEmit::StreamingFlush { parts_count, .. } => {
                assert!(*parts_count >= 1);
            }
            other => panic!("first emit must be StreamingFlush, got {other:?}"),
        }
        assert_eq!(
            events[1],
            RecordedEmit::StateChanged {
                phase: TurnPhase::Streaming,
                exit_code: None,
            }
        );
        assert_eq!(proc.turn_phase, TurnPhase::Streaming);
        assert!(proc.pending_stream_parts.is_empty());
        // Permission part status was updated in place.
        let updated = proc
            .streaming_parts
            .iter()
            .find_map(|p| match p {
                MessagePart::Permission { status, .. } => Some(status.clone()),
                _ => None,
            })
            .expect("permission part present");
        assert_eq!(updated, "allowed");
    }

    #[tokio::test]
    async fn respond_permission_transition_carries_projected_active_session_state() {
        let mut proc = make_process_waiting_for_permission("req-1");
        begin_test_turn_event_log(&mut proc);
        let permission_part = proc
            .streaming_parts
            .iter()
            .find(|part| matches!(part, MessagePart::Permission { .. }))
            .cloned()
            .expect("permission part");
        record_durable_parts_for_current_turn(
            &mut proc,
            "m1",
            std::slice::from_ref(&permission_part),
        );

        let effect = apply_respond_permission_locked(
            &mut proc,
            "csid",
            "req-1",
            "allow",
            None,
            |_mid, _seq, _snapshot, _parts| true,
        );

        assert!(effect.did_transition);
        assert_eq!(
            effect.projected_session_state,
            Some(crate::usecase::agent_session::session::SessionState::Active)
        );
        assert_eq!(proc.turn_phase, TurnPhase::Streaming);
        let _ = proc.child.kill().await;
    }

    #[tokio::test]
    async fn respond_permission_no_transition_when_not_waiting() {
        // 直前に WaitingPermission でなかった場合、state は変更されず、
        // 後続の state-changed emit も発火しないこと。
        let mut proc = make_process_waiting_for_permission("req-1");
        proc.turn_phase = TurnPhase::Streaming; // not WaitingPermission

        let mut events = Vec::new();
        let transitioned =
            drive_respond_permission_path(&mut proc, "csid", "req-1", "deny", None, &mut events);
        assert!(
            !transitioned,
            "no transition when proc was not in WaitingPermission"
        );

        // StateChanged は events に積まれていないこと。
        assert!(!events
            .iter()
            .any(|e| matches!(e, RecordedEmit::StateChanged { .. })));
        assert_eq!(proc.turn_phase, TurnPhase::Streaming);
    }

    #[tokio::test]
    async fn respond_permission_continues_on_emit_failure() {
        // Spec L157「強制配信が失敗しても後続の状態遷移は続行する」:
        //  emit 失敗でも did_transition は true のまま返り、
        //  呼び出し側 (production: emit_session_state_changed) は続行できる。
        let mut proc = make_process_waiting_for_permission("req-1");

        let effect = apply_respond_permission_locked(
            &mut proc,
            "csid",
            "req-1",
            "allow",
            None,
            |_mid, _seq, _snapshot, _parts| false, // emit failure on both channels
        );
        assert!(
            effect.did_transition,
            "transition must still be reported so caller emits state-change"
        );
        assert_eq!(proc.turn_phase, TurnPhase::Streaming);
        // Retry payload is retained for the next flush.
        assert_eq!(proc.pending_stream_parts.len(), 0);
        assert!(proc.retry_stream_delta.is_some());
        assert!(proc.last_stream_emit_at.is_none());
    }

    #[test]
    fn permission_response_payload_format() {
        let request_id = "req-123";
        let behavior = "allow";
        let message: Option<String> = None;
        let mut result = serde_json::json!({ "behavior": behavior });
        if let Some(msg) = &message {
            result["message"] = serde_json::Value::String(msg.clone());
        }
        let payload = serde_json::json!({
            "type": "permission_response",
            "request_id": request_id,
            "result": result,
        });
        assert_eq!(payload["type"], "permission_response");
        assert_eq!(payload["request_id"], "req-123");
        assert_eq!(payload["result"]["behavior"], "allow");
        assert!(payload["result"].get("message").is_none());
    }

    #[test]
    fn permission_response_payload_with_updated_input() {
        let request_id = "req-789";
        let behavior = "allow";
        let message: Option<String> = None;
        let updated_input = Some(r#"{"questions":[],"answers":{"Q":"A"}}"#.to_string());
        let mut result = serde_json::json!({ "behavior": behavior });
        if let Some(msg) = &message {
            result["message"] = serde_json::Value::String(msg.clone());
        }
        if let Some(input_json) = &updated_input {
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(input_json) {
                result["updatedInput"] = parsed;
            }
        }
        let payload = serde_json::json!({
            "type": "permission_response",
            "request_id": request_id,
            "result": result,
        });
        assert_eq!(payload["result"]["behavior"], "allow");
        assert_eq!(payload["result"]["updatedInput"]["answers"]["Q"], "A");
        assert!(payload["result"].get("message").is_none());
    }

    #[test]
    fn permission_response_rejects_invalid_updated_input_json() {
        let mut result = serde_json::json!({ "behavior": "allow" });

        let err =
            apply_updated_input_to_permission_result(&mut result, Some("{not json")).unwrap_err();

        assert!(err.contains("Invalid updated_input JSON"));
        assert!(result.get("updatedInput").is_none());
    }

    #[test]
    fn behavior_validation_rejects_invalid_values() {
        let valid = ["allow", "deny"];
        let invalid = ["Allow", "ALLOW", "reject", "", "maybe"];
        for v in valid {
            assert!(v == "allow" || v == "deny");
        }
        for v in invalid {
            assert!(v != "allow" && v != "deny");
        }
    }

    #[test]
    fn permission_response_payload_with_deny_message() {
        let request_id = "req-456";
        let behavior = "deny";
        let message = Some("User denied".to_string());
        let mut result = serde_json::json!({ "behavior": behavior });
        if let Some(msg) = &message {
            result["message"] = serde_json::Value::String(msg.clone());
        }
        let payload = serde_json::json!({
            "type": "permission_response",
            "request_id": request_id,
            "result": result,
        });
        assert_eq!(payload["result"]["behavior"], "deny");
        assert_eq!(payload["result"]["message"], "User denied");
    }

    #[test]
    fn claude_flag_round_trip_via_permission_flags_module() {
        use crate::domain::agent_session::PermissionMode;
        use crate::infrastructure::agent_session::runtime::permission_flags::{
            claude_flag_from_mode, mode_from_claude_flag,
        };
        for (abstract_mode, expected_flag) in [
            (PermissionMode::Ask, "default"),
            (PermissionMode::Edit, "acceptEdits"),
            (PermissionMode::Full, "bypassPermissions"),
        ] {
            assert_eq!(claude_flag_from_mode(abstract_mode), expected_flag);
            assert_eq!(mode_from_claude_flag(expected_flag), Some(abstract_mode));
        }
        // "plan" は廃止語彙のため抽象モードに戻せない（None）。
        assert!(mode_from_claude_flag("plan").is_none());
    }

    /// spawn_bridge_process が spawn 前にパーミッションモードを検証することの担保。
    /// 本テストは spawn 前の `PermissionMode::parse` 早期 return を直接利用するためのスモークテスト。
    #[test]
    fn pre_spawn_permission_validation_smoke() {
        // 抽象モード以外は早期に弾かれる契約を確認する。
        for invalid in ["acceptEdits", "bypassPermissions", "plan", "default", ""] {
            assert!(
                crate::domain::agent_session::PermissionMode::parse(invalid).is_err(),
                "spawn 前の検証は '{invalid}' を弾く必要がある"
            );
        }
        for valid in ["ask", "edit", "full"] {
            assert!(crate::domain::agent_session::PermissionMode::parse(valid).is_ok());
        }
    }

    #[test]
    fn build_set_mode_command_emits_claude_flag() {
        let data =
            build_set_mode_command_for_backend("edit", CLAUDE_BACKEND_ID).expect("valid mode");
        let cmd: serde_json::Value = serde_json::from_str(data.trim()).unwrap();
        assert_eq!(cmd["type"], "setMode");
        assert_eq!(cmd["permissionMode"], "acceptEdits");
        assert!(cmd.get("approvalPolicy").is_none());
        assert!(cmd.get("sandboxMode").is_none());
    }

    #[test]
    fn set_mode_command_format() {
        let data =
            build_set_mode_command_for_backend("full", CLAUDE_BACKEND_ID).expect("valid mode");
        let cmd: serde_json::Value = serde_json::from_str(data.trim()).unwrap();
        assert_eq!(cmd["type"], "setMode");
        assert_eq!(cmd["permissionMode"], "bypassPermissions");
    }

    #[test]
    fn build_set_mode_command_emits_codex_flags() {
        let data =
            build_set_mode_command_for_backend("full", CODEX_BACKEND_ID).expect("valid mode");
        let cmd: serde_json::Value = serde_json::from_str(data.trim()).unwrap();
        assert_eq!(cmd["type"], "setMode");
        assert_eq!(cmd["sandboxMode"], "danger-full-access");
        assert_eq!(cmd["approvalPolicy"], "never");
        assert!(cmd.get("permissionMode").is_none());
    }

    #[test]
    fn build_set_mode_command_rejects_legacy_value() {
        for legacy in ["acceptEdits", "bypassPermissions", "plan", "default", ""] {
            assert!(
                build_set_mode_command_for_backend(legacy, CLAUDE_BACKEND_ID).is_err(),
                "legacy '{legacy}' must be rejected"
            );
        }
    }

    #[tokio::test]
    async fn sync_pre_turn_settings_does_not_send_set_model() {
        use std::time::Duration;
        use tokio::io::AsyncReadExt;

        let (mut proc, mut stdout) = make_test_agent_process_with_stdout();
        proc.backend_id = CLAUDE_BACKEND_ID.to_string();
        proc.selected_model = Some("claude-opus".to_string());

        sync_pre_turn_settings(&mut proc, "edit")
            .await
            .expect("pre-turn settings must sync");

        let AgentProcess {
            stdin, mut child, ..
        } = proc;
        drop(stdin);

        let mut output = String::new();
        tokio::time::timeout(Duration::from_secs(5), stdout.read_to_string(&mut output))
            .await
            .expect("stdout read must complete")
            .expect("stdout must be readable");
        let _ = child.wait().await;

        let commands = output
            .lines()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("json command"))
            .collect::<Vec<_>>();

        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0]["type"], "setMode");
        assert_eq!(commands[0]["permissionMode"], "acceptEdits");
        assert!(
            commands
                .iter()
                .all(|cmd| cmd.get("type").and_then(|value| value.as_str()) != Some("setModel")),
            "pre-turn sync must not send setModel: {commands:?}"
        );
    }

    #[tokio::test]
    async fn set_agent_permission_mode_internal_rejects_invalid_without_mutating_state() {
        use tokio::io::AsyncReadExt;

        // Spec issues-947: 外部境界（set_agent_permission_mode 相当）で invalid 値を受けたとき、
        // 保存値・current_permission_mode・bridge stdin のいずれも変化させない。
        // bridge stdin の不変は、stdout を pipe で開いた `cat` を bridge process に見立てて
        // 「invalid を拒否した後で stdin を閉じ、stdout の echo が空である」ことで観測する。
        let data_dir = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let session_id = uuid::Uuid::new_v4().to_string();
        let session = chat_session_for_permission_test(&session_id, "edit");
        session_store
            .save_full_session_for_migration_or_restore(data_dir.path(), &session)
            .unwrap();

        let (proc, mut stdout) = make_test_agent_process_with_stdout();
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        handles.lock().await.insert(session_id.clone(), proc);

        for invalid in [
            "acceptEdits",
            "bypassPermissions",
            "plan",
            "default",
            "unknown",
            "",
        ] {
            let err = set_agent_permission_mode_internal(
                &session_store,
                &handles,
                data_dir.path(),
                &session_id,
                invalid,
            )
            .await
            .err()
            .unwrap_or_else(|| panic!("invalid '{invalid}' must be rejected"));
            assert!(
                err.contains("ask, edit, full"),
                "invalid '{invalid}' must include allowed list, got: {err}"
            );

            // 保存値が変わらない。
            let saved = session_store
                .load_full_session_for_restore(data_dir.path(), &session_id)
                .unwrap()
                .unwrap();
            assert_eq!(
                saved.permission_mode, "edit",
                "persisted permission_mode must remain unchanged for '{invalid}'"
            );

            // current_permission_mode（ランタイム）も変わらない。
            let map = handles.lock().await;
            let proc = map.get(&session_id).expect("agent process retained");
            assert_eq!(
                proc.current_permission_mode, "edit",
                "current_permission_mode must remain unchanged for '{invalid}'"
            );
        }

        // 全ての invalid 入力を試した後、bridge stdin への書き込みなしを直接観測する。
        // Map から AgentProcess を取り出して child を kill し、stdin を drop することで
        // `cat` が EOF を読み取って終了し、stdout 読み取りが完了する。`cat` は受け取った
        // バイトをそのまま echo するため、stdout が空 == bridge stdin 未書き込み。
        let mut proc = handles.lock().await.remove(&session_id).unwrap();
        let _ = proc.child.kill().await;
        drop(proc.stdin);
        let mut buf = Vec::new();
        stdout
            .read_to_end(&mut buf)
            .await
            .expect("read stdout to EOF");
        assert!(
            buf.is_empty(),
            "no bytes must be written to bridge stdin for invalid permission modes, got: {:?}",
            String::from_utf8_lossy(&buf)
        );
    }

    #[tokio::test]
    async fn set_agent_permission_mode_internal_persists_valid_abstract_mode() {
        let data_dir = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let session_id = uuid::Uuid::new_v4().to_string();
        let session = chat_session_for_permission_test(&session_id, "edit");
        session_store
            .save_full_session_for_migration_or_restore(data_dir.path(), &session)
            .unwrap();
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));

        set_agent_permission_mode_internal(
            &session_store,
            &handles,
            data_dir.path(),
            &session_id,
            "ask",
        )
        .await
        .expect("valid abstract mode must be accepted");

        let saved = session_store
            .load_full_session_for_restore(data_dir.path(), &session_id)
            .unwrap()
            .unwrap();
        assert_eq!(saved.permission_mode, "ask");
    }
}
