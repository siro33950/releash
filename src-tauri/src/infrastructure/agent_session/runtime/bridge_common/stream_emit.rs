use super::process_registry::{
    AgentProcess, AgentProcessMap, BridgeState, PendingStreamDelta, StreamPartRollback, TurnPhase,
};
use super::shared::{
    append_display_delta_parts, apply_stream_delta_to_parts, canonical_stream_parts_from_slice,
    pending_delta_parts, StreamDeltaApplyResult,
};
use crate::usecase::agent_session::session::now_timestamp;
use crate::usecase::agent_session::session::MessagePart;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;
use tokio::sync::Mutex;

pub(super) const STREAMING_EMIT_INTERVAL_MS: u64 = 33;

/// Maximum number of pending delta parts before we flush early.
/// Acts as a flush threshold in normal operation, and as a soft cap
/// (we keep accepting parts even past this) while delivery is failing.
pub(super) const STREAMING_PENDING_PART_LIMIT: usize = 1000;

/// Maximum cumulative byte size of pending delta payloads before we flush early.
/// Same semantics as `STREAMING_PENDING_PART_LIMIT`: flush threshold in normal
/// operation, soft cap (allowed to overflow) while delivery is failing.
pub(super) const STREAMING_PENDING_BYTE_LIMIT: usize = 256 * 1024;

pub(super) fn release_completed_turn_streaming_buffer(proc: &mut AgentProcess) -> Vec<MessagePart> {
    let released_parts = std::mem::take(&mut proc.streaming_parts);
    proc.confirmed_stream_part_len = 0;
    proc.pending_stream_parts.clear();
    proc.pending_stream_part_rollbacks.clear();
    proc.retry_stream_delta = None;
    proc.pending_stream_bytes = 0;
    proc.last_stream_emit_at = None;
    released_parts
}

pub(super) fn emit_session_state_changed<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    chat_session_id: &str,
    turn_phase: TurnPhase,
    exit_code: Option<i64>,
    interrupted: bool,
) {
    use tauri::Emitter;
    let completed_at = exit_code.map(|_| now_timestamp());
    let _ = app.emit(
        "agent-session-state-changed",
        serde_json::json!({
            "chat_session_id": chat_session_id,
            "turn_phase": turn_phase,
            "exit_code": exit_code,
            "completed_at": completed_at,
            "interrupted": interrupted,
        }),
    );
}

/// Returns `(tauri_ok, ws_ok)`. `tauri_ok` reflects whether the Tauri event
/// dispatcher accepted the delta payload. `ws_ok` is always `true` on the
/// production broadcaster path because enqueueing is best effort; unit tests
/// that need a WS-side failure drive `apply_streaming_emit_result` directly.
pub(super) fn emit_streaming_delta<R, F>(
    app: &tauri::AppHandle<R>,
    chat_session_id: &str,
    message_id: &str,
    seq: u64,
    parts: Vec<MessagePart>,
    snapshot_parts: F,
) -> (bool, bool)
where
    R: tauri::Runtime,
    F: FnOnce() -> Vec<MessagePart>,
{
    use tauri::{Emitter, Manager};
    let payload = serde_json::json!({
        "chat_session_id": chat_session_id,
        "message_id": message_id,
        "seq": seq,
        "parts": parts,
    });
    crate::other::telemetry::record_payload_size(
        crate::other::telemetry::Payload::TauriEvent,
        || {
            serde_json::to_vec(&payload)
                .map(|body| body.len())
                .unwrap_or(0)
        },
    );
    let tauri_ok = app.emit("agent-streaming-delta", &payload).is_ok();
    if let Some(broadcaster) = app.try_state::<Arc<crate::ws_bridge::WsBroadcaster>>() {
        let session_id = chat_session_id.to_string();
        let message_id = message_id.to_string();
        broadcaster.send_stream_delta(
            crate::protocol::AgentStreamDeltaMsg {
                session_id: session_id.clone(),
                message_id: message_id.clone(),
                seq,
                parts: parts.into_iter().map(to_agent_stream_part_msg).collect(),
            },
            || crate::protocol::AgentStreamSync {
                session_id,
                message_id,
                seq,
                parts: snapshot_parts()
                    .into_iter()
                    .map(to_agent_stream_part_msg)
                    .collect(),
            },
        );
    }
    (tauri_ok, true)
}

fn to_agent_stream_part_msg(part: MessagePart) -> crate::protocol::AgentStreamPartMsg {
    part.into()
}

/// Estimate the wire byte size contributed by one delta part. Used to decide
/// whether the pending buffer has crossed the byte cap. Exact values aren't
/// required — only proportional growth matters.
pub(super) fn part_byte_size(part: &MessagePart) -> usize {
    match part {
        MessagePart::Text { content, .. }
        | MessagePart::Thinking { content, .. }
        | MessagePart::Error { content, .. }
        | MessagePart::ToolResult { content, .. } => content.len(),
        MessagePart::ToolUse {
            tool, input, id, ..
        } => tool.len() + id.len() + serde_json::to_string(input).map(|s| s.len()).unwrap_or(0),
        MessagePart::Permission {
            request,
            status,
            answers,
            ..
        } => {
            status.len()
                + serde_json::to_string(request).map(|s| s.len()).unwrap_or(0)
                + answers
                    .as_ref()
                    .and_then(|a| serde_json::to_string(a).ok())
                    .map(|s| s.len())
                    .unwrap_or(0)
        }
        MessagePart::TaskStatus {
            task_tool_use_id,
            status,
            description,
            summary,
        } => {
            task_tool_use_id.len()
                + status.len()
                + description.as_ref().map(|s| s.len()).unwrap_or(0)
                + summary.as_ref().map(|s| s.len()).unwrap_or(0)
        }
        MessagePart::TodoListSnapshot { items } => {
            items.iter().map(|item| item.text.len() + 1).sum()
        }
        MessagePart::SystemNotification {
            notification_type,
            status,
            label,
            detail,
            hook_id,
        } => {
            notification_type.as_str().len()
                + status.len()
                + label.len()
                + detail.as_ref().map(|s| s.len()).unwrap_or(0)
                + hook_id.as_ref().map(|s| s.len()).unwrap_or(0)
        }
        MessagePart::Image { data, media_type } => data.len() + media_type.len(),
        MessagePart::ImageRef { attachment } => {
            attachment.id.len() + attachment.media_type.len() + std::mem::size_of::<u64>()
        }
    }
}

/// Record that delta parts have been queued for the next coalescing flush.
/// `pending_stream_parts` owns the concrete delta payload until a successful
/// emit; its length is the pending count and dirty signal. `pending_stream_bytes`
/// is tracked separately for the byte-cap flush trigger.
pub(super) fn enqueue_pending_delta(proc: &mut AgentProcess, delta: &[MessagePart]) {
    enqueue_pending_delta_with_rollbacks(proc, delta, Vec::new());
}

pub(super) fn enqueue_pending_delta_with_rollbacks(
    proc: &mut AgentProcess,
    delta: &[MessagePart],
    rollbacks: Vec<StreamPartRollback>,
) {
    for p in delta {
        proc.pending_stream_bytes = proc.pending_stream_bytes.saturating_add(part_byte_size(p));
    }
    proc.pending_stream_parts.extend_from_slice(delta);
    proc.pending_stream_part_rollbacks.extend(rollbacks);
}

/// True when the pending buffer has crossed either the count or byte threshold.
/// While delivery is succeeding this triggers an immediate flush; while
/// delivery is failing the buffer continues to grow past these thresholds.
pub(super) fn pending_exceeds_threshold(proc: &AgentProcess) -> bool {
    proc.pending_stream_parts.len() >= STREAMING_PENDING_PART_LIMIT
        || proc.pending_stream_bytes >= STREAMING_PENDING_BYTE_LIMIT
}

pub(super) fn has_pending_stream_flush(proc: &AgentProcess) -> bool {
    !proc.pending_stream_parts.is_empty() || proc.retry_stream_delta.is_some()
}

/// True when enough time has elapsed since the last successful emit for the
/// next-delta flush trigger to fire. First emit (no `last_stream_emit_at`)
/// always returns true so the initial chunk reaches the UI without delay.
pub(super) fn streaming_interval_elapsed(proc: &AgentProcess) -> bool {
    match proc.last_stream_emit_at {
        None => true,
        Some(t) => t.elapsed() >= Duration::from_millis(STREAMING_EMIT_INTERVAL_MS),
    }
}

/// Snapshot of pending-flush bookkeeping captured before an emit attempt.
/// Holds enough metadata to build a failure log without re-reading the
/// process state, and is the source of `apply_streaming_emit_result`.
#[derive(Debug, Clone)]
pub(super) struct StreamingFlushSnapshot {
    pub(crate) seq: u64,
    pub(crate) parts: Vec<MessagePart>,
    pub(crate) part_count: usize,
    pub(crate) buffer_len: usize,
    pub(crate) pending_bytes: usize,
    rollbacks: Vec<StreamPartRollback>,
    confirmed_stream_part_len_after_success: usize,
    source: StreamingFlushSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StreamingFlushSource {
    Pending,
    Retry,
    OneShot,
}

/// Prepare a streaming flush: snapshot the pending delta payload and buffer
/// metadata after passing the update through the Rust canonical stream apply
/// rules. Append-only deltas are emitted directly; in-place updates that cannot
/// be represented by frontend append/merge are sent as an intentional seq gap
/// so the existing resync read model supplies the canonical snapshot.
pub(super) fn prepare_streaming_flush(proc: &AgentProcess) -> Option<StreamingFlushSnapshot> {
    if let Some(retry) = &proc.retry_stream_delta {
        return Some(StreamingFlushSnapshot {
            seq: retry.seq,
            part_count: retry.part_count,
            buffer_len: retry.part_count,
            pending_bytes: retry.pending_bytes,
            parts: retry.parts.clone(),
            rollbacks: retry.rollbacks.clone(),
            confirmed_stream_part_len_after_success: retry.confirmed_stream_part_len_after_success,
            source: StreamingFlushSource::Retry,
        });
    }
    if proc.pending_stream_parts.is_empty() {
        return None;
    }
    let next_seq = proc.streaming_delta_seq.saturating_add(1);
    let mut canonical_parts = confirmed_streaming_parts(proc);
    let mut canonical_seq = proc.streaming_delta_seq;
    let apply_result = apply_stream_delta_to_parts(
        &mut canonical_parts,
        &mut canonical_seq,
        next_seq,
        &proc.pending_stream_parts,
    );
    debug_assert_eq!(apply_result, StreamDeltaApplyResult::Applied);

    let parts = pending_delta_parts(&proc.pending_stream_parts, proc.pending_stream_parts.len());
    let mut display_projected_parts = confirmed_streaming_parts(proc);
    append_display_delta_parts(&mut display_projected_parts, &parts);
    let append_only_delta = display_projected_parts == canonical_parts;
    let seq = if append_only_delta {
        next_seq
    } else {
        next_seq.saturating_add(1)
    };
    let parts = if append_only_delta { parts } else { Vec::new() };
    Some(StreamingFlushSnapshot {
        seq,
        part_count: parts.len(),
        buffer_len: proc.pending_stream_parts.len(),
        pending_bytes: proc.pending_stream_bytes,
        parts,
        rollbacks: proc.pending_stream_part_rollbacks.clone(),
        confirmed_stream_part_len_after_success: proc.streaming_parts.len(),
        source: StreamingFlushSource::Pending,
    })
}

fn apply_rollbacks(parts: &mut [MessagePart], rollbacks: &[StreamPartRollback]) {
    for rollback in rollbacks.iter().rev() {
        if let Some(part) = parts.get_mut(rollback.index) {
            *part = rollback.previous.clone();
        }
    }
}

pub(super) fn confirmed_streaming_parts(proc: &AgentProcess) -> Vec<MessagePart> {
    let mut parts = proc.streaming_parts.clone();
    apply_rollbacks(&mut parts, &proc.pending_stream_part_rollbacks);
    if let Some(retry) = &proc.retry_stream_delta {
        apply_rollbacks(&mut parts, &retry.rollbacks);
    }
    parts.truncate(proc.confirmed_stream_part_len.min(parts.len()));
    canonical_stream_parts_from_slice(&parts)
}

fn snapshot_parts_after_success(
    proc: &AgentProcess,
    snapshot: &StreamingFlushSnapshot,
) -> Vec<MessagePart> {
    let mut parts = proc.streaming_parts.clone();
    if snapshot.source == StreamingFlushSource::Retry {
        apply_rollbacks(&mut parts, &proc.pending_stream_part_rollbacks);
    }
    parts.truncate(
        snapshot
            .confirmed_stream_part_len_after_success
            .min(parts.len()),
    );
    canonical_stream_parts_from_slice(&parts)
}

/// Apply the emit result to the coalescing state. On success clears the
/// pending buffer, commits the delta seq, and bumps `last_stream_emit_at`; on
/// failure retains both (so the next flush retries the same delta seq) and emits a warning
/// log containing only non-body metadata. Returns whether the emit succeeded.
pub(super) fn apply_streaming_emit_result(
    proc: &mut AgentProcess,
    chat_session_id: &str,
    message_id: &str,
    snapshot: &StreamingFlushSnapshot,
    tauri_ok: bool,
    ws_ok: bool,
) -> bool {
    if tauri_ok && ws_ok {
        match snapshot.source {
            StreamingFlushSource::Pending => {
                proc.pending_stream_parts.clear();
                proc.pending_stream_part_rollbacks.clear();
                proc.pending_stream_bytes = 0;
            }
            StreamingFlushSource::Retry => {
                proc.retry_stream_delta = None;
            }
            StreamingFlushSource::OneShot => {}
        }
        proc.streaming_delta_seq_by_message
            .insert(message_id.to_string(), snapshot.seq);
        let targets_live_counter = snapshot.source != StreamingFlushSource::OneShot
            || proc.streaming_message_id.as_deref() == Some(message_id)
            || proc.last_message_id.as_deref() == Some(message_id);
        if targets_live_counter {
            proc.streaming_delta_seq = snapshot.seq;
            proc.confirmed_stream_part_len = snapshot.confirmed_stream_part_len_after_success;
            let now = Instant::now();
            if let Some(previous) = proc.last_stream_emit_at {
                crate::other::telemetry::record_emit_interval(now.duration_since(previous));
            }
            proc.last_stream_emit_at = Some(now);
        }
        true
    } else {
        // NB: deliberately exclude payload content / tool I/O / mentions —
        // those are external user data and must not appear in logs.
        log::warn!(
            "agent-streaming-delta emit failure: chat_session={} message_id={} \
             part_count={} buffer_len={} pending_bytes={} tauri_ok={} ws_ok={}",
            chat_session_id,
            message_id,
            snapshot.part_count,
            snapshot.buffer_len,
            snapshot.pending_bytes,
            tauri_ok,
            ws_ok
        );
        if snapshot.source == StreamingFlushSource::Pending && proc.retry_stream_delta.is_none() {
            proc.retry_stream_delta = Some(PendingStreamDelta {
                seq: snapshot.seq,
                parts: snapshot.parts.clone(),
                part_count: snapshot.part_count,
                pending_bytes: snapshot.pending_bytes,
                rollbacks: snapshot.rollbacks.clone(),
                confirmed_stream_part_len_after_success: snapshot
                    .confirmed_stream_part_len_after_success,
            });
            proc.pending_stream_parts.clear();
            proc.pending_stream_part_rollbacks.clear();
            proc.pending_stream_bytes = 0;
        }
        false
    }
}

/// Run the prepare → emit → apply sequence with a caller-supplied emit
/// function. Extracting this lets unit tests drive the production flush
/// pipeline with a recording emit closure, instead of mirroring the prepare
/// / apply calls inline (which used to drift from the production path).
///
/// The closure receives the delta seq and `MessagePart` slice destined for the
/// frontend, plus a lazy cumulative snapshot factory for WS overflow fallback,
/// and returns `(tauri_ok, ws_ok)` matching
/// `emit_streaming_delta`.
pub(super) fn force_flush_pending_streaming<F>(
    proc: &mut AgentProcess,
    chat_session_id: &str,
    message_id: &str,
    mut emit: F,
) -> bool
where
    F: FnMut(u64, &[MessagePart], &dyn Fn() -> Vec<MessagePart>) -> (bool, bool),
{
    let Some(snapshot) = prepare_streaming_flush(proc) else {
        return true;
    };
    let overflow_snapshot_parts = || snapshot_parts_after_success(proc, &snapshot);
    let (tauri_ok, ws_ok) = emit(snapshot.seq, &snapshot.parts, &overflow_snapshot_parts);
    apply_streaming_emit_result(
        proc,
        chat_session_id,
        message_id,
        &snapshot,
        tauri_ok,
        ws_ok,
    )
}

pub(super) fn emit_one_shot_streaming_delta<F>(
    proc: &mut AgentProcess,
    chat_session_id: &str,
    message_id: &str,
    seq: u64,
    parts: Vec<MessagePart>,
    snapshot_parts: Vec<MessagePart>,
    mut emit: F,
) -> bool
where
    F: FnMut(u64, &[MessagePart], &dyn Fn() -> Vec<MessagePart>) -> (bool, bool),
{
    let snapshot = StreamingFlushSnapshot {
        seq,
        part_count: parts.len(),
        buffer_len: parts.len(),
        pending_bytes: parts.iter().map(part_byte_size).sum(),
        parts,
        rollbacks: Vec::new(),
        confirmed_stream_part_len_after_success: proc.confirmed_stream_part_len,
        source: StreamingFlushSource::OneShot,
    };
    let overflow_snapshot_parts = || snapshot_parts.clone();
    let (tauri_ok, ws_ok) = emit(snapshot.seq, &snapshot.parts, &overflow_snapshot_parts);
    apply_streaming_emit_result(
        proc,
        chat_session_id,
        message_id,
        &snapshot,
        tauri_ok,
        ws_ok,
    )
}

/// Force-flush pending streaming delta before a turn-phase transition
/// (permission_request, turn_complete, tool boundary, error). Returns
/// `true` when the process was in `Streaming` state so the caller knows to
/// emit a `agent-session-state-changed` notification after releasing the
/// lock. The flush runs FIRST so the frontend never observes a state
/// transition ahead of the tail content for the current message.
///
/// The emit closure mirrors `emit_streaming_delta`: it receives the message
/// id, seq, delta parts, and lazy snapshot fallback parts and returns
/// `(tauri_ok, ws_ok)`.
/// Production callers pass a closure that delegates to `emit_streaming_delta`;
/// unit tests pass a recording closure to verify the ordering invariant.
pub(super) fn flush_streaming_before_transition<F>(
    proc: &mut AgentProcess,
    chat_session_id: &str,
    mut emit_stream: F,
) -> bool
where
    F: FnMut(&str, u64, &[MessagePart], &dyn Fn() -> Vec<MessagePart>) -> (bool, bool),
{
    let turn_completed = proc.state == BridgeState::Streaming;
    let Some(mid) = proc.streaming_message_id.clone() else {
        return turn_completed;
    };
    let _ =
        force_flush_pending_streaming(proc, chat_session_id, &mid, |seq, parts, snapshot_parts| {
            emit_stream(&mid, seq, parts, snapshot_parts)
        });
    turn_completed
}

/// Per-delta flush decision used by the stdout reader. `post_turn` is true
/// when the delta is arriving after `turn_complete` (background-task events
/// piggy-backed on the closed turn) — those are always force-flushed so the
/// post-turn UI does not stall on the throttle.
pub(super) fn should_flush_per_delta(
    proc: &AgentProcess,
    delta: &[MessagePart],
    post_turn: bool,
) -> bool {
    let force = post_turn || delta_has_tool_event(delta) || pending_exceeds_threshold(proc);
    force || streaming_interval_elapsed(proc)
}

/// One iteration of the auxiliary timer loop. Bound to a single process by
/// the caller (generation_id / state checks happen above this helper). The
/// emit closure mirrors `force_flush_pending_streaming` so tests can drive
/// the same code path the production timer uses.
///
#[derive(Debug, Default)]
pub(super) struct StreamingTimerTickEffect {
    pub(crate) keep_running: bool,
    pub(crate) released_streaming_parts: Vec<MessagePart>,
}

/// `keep_running` is `true` when the timer should continue running this turn,
/// and `false` when the loop should exit (turn is over and the buffer has been
/// fully drained).
pub(super) fn run_streaming_timer_tick<F>(
    proc: &mut AgentProcess,
    chat_session_id: &str,
    mut emit: F,
) -> StreamingTimerTickEffect
where
    F: FnMut(&str, u64, &[MessagePart], &dyn Fn() -> Vec<MessagePart>) -> (bool, bool),
{
    let pending = has_pending_stream_flush(proc);
    let streaming = proc.state == BridgeState::Streaming;
    if !pending && !streaming {
        // Turn ended and the buffer is empty — timer has nothing left to do.
        return StreamingTimerTickEffect {
            keep_running: false,
            released_streaming_parts: release_completed_turn_streaming_buffer(proc),
        };
    }
    if !pending || !streaming_interval_elapsed(proc) {
        return StreamingTimerTickEffect {
            keep_running: true,
            ..StreamingTimerTickEffect::default()
        };
    }
    let Some(mid) = proc
        .streaming_message_id
        .clone()
        .or_else(|| proc.last_message_id.clone())
    else {
        return StreamingTimerTickEffect {
            keep_running: true,
            ..StreamingTimerTickEffect::default()
        };
    };
    let flushed =
        force_flush_pending_streaming(proc, chat_session_id, &mid, |seq, parts, snapshot_parts| {
            emit(&mid, seq, parts, snapshot_parts)
        });
    if !streaming
        && flushed
        && proc.pending_stream_parts.is_empty()
        && proc.retry_stream_delta.is_none()
    {
        return StreamingTimerTickEffect {
            keep_running: false,
            released_streaming_parts: release_completed_turn_streaming_buffer(proc),
        };
    }
    StreamingTimerTickEffect {
        keep_running: true,
        ..StreamingTimerTickEffect::default()
    }
}

/// Returns `true` when any delta part represents a tool invocation boundary.
/// Used to force a flush around tool start/end so the UI never shows a stale
/// frame across these transitions.
pub(super) fn delta_has_tool_event(delta: &[MessagePart]) -> bool {
    delta.iter().any(|p| {
        matches!(
            p,
            MessagePart::ToolUse { .. } | MessagePart::ToolResult { .. }
        )
    })
}

/// Loop control decision for the per-turn streaming timer. Extracted as a
/// pure function so the spawn loop's exit/flag-management semantics are
/// covered by unit tests, instead of relying on a tokio task to be observable
/// from the test harness.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum TimerDecision {
    /// Generation matches and process is still streaming — run the tick.
    Continue,
    /// Generation matches and process has crashed — exit and release the
    /// active flag so a future turn can spawn a fresh timer.
    BreakClearFlag,
    /// Generation no longer matches: a newer process owns the slot, and its
    /// own timer is responsible for the flag. Exit without touching it.
    BreakKeepFlag,
}

/// Decide what the streaming timer should do at the top of each tick.
pub(super) fn streaming_timer_decision(proc: &AgentProcess, captured_gen_id: u64) -> TimerDecision {
    if proc.generation_id != captured_gen_id {
        return TimerDecision::BreakKeepFlag;
    }
    if proc.state == BridgeState::Crashed {
        return TimerDecision::BreakClearFlag;
    }
    TimerDecision::Continue
}

/// Idempotency gate for `spawn_streaming_timer`. Marks the timer slot active
/// and returns `true` when the caller should spawn; returns `false` when a
/// timer is already running for this process (duplicate spawn no-op).
pub(super) fn try_mark_streaming_timer_active(proc: &mut AgentProcess) -> bool {
    if proc.streaming_timer_active {
        return false;
    }
    proc.streaming_timer_active = true;
    true
}

/// Spawn the per-turn auxiliary streaming-flush timer. Ticks every
/// `STREAMING_EMIT_INTERVAL_MS` and drains the pending coalescing buffer so
/// silent gaps between deltas (e.g. SDK ingesting a tool result) still
/// surface buffered content within one interval. Exits when the turn ends
/// and the buffer is fully drained, on generation mismatch, or on crash.
/// Idempotent: a second call while a timer is already alive is a no-op.
pub(super) fn spawn_streaming_timer<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    chat_session_id: &str,
    proc: &mut AgentProcess,
) {
    if !try_mark_streaming_timer_active(proc) {
        return;
    }
    let handles_timer = Arc::clone(handles);
    let app_timer = app.clone();
    let csid_timer = chat_session_id.to_string();
    let captured_gen_id_timer = proc.generation_id;
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(STREAMING_EMIT_INTERVAL_MS));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        // Skip the immediate first tick — the stdout reader's per-delta path
        // handles the very first emit.
        interval.tick().await;
        loop {
            interval.tick().await;
            let tick_effect = {
                let mut map = handles_timer.lock().await;
                let Some(proc) = map.get_mut(&csid_timer) else {
                    // Process removed — no flag to clear.
                    break;
                };
                match streaming_timer_decision(proc, captured_gen_id_timer) {
                    TimerDecision::BreakKeepFlag => break,
                    TimerDecision::BreakClearFlag => {
                        proc.streaming_timer_active = false;
                        break;
                    }
                    TimerDecision::Continue => {}
                }
                let tick_effect = run_streaming_timer_tick(
                    proc,
                    &csid_timer,
                    |mid, seq, parts, snapshot_parts| {
                        emit_streaming_delta(
                            &app_timer,
                            &csid_timer,
                            mid,
                            seq,
                            parts.to_vec(),
                            snapshot_parts,
                        )
                    },
                );
                if !tick_effect.keep_running {
                    proc.streaming_timer_active = false;
                }
                tick_effect
            };
            let StreamingTimerTickEffect {
                keep_running,
                released_streaming_parts,
            } = tick_effect;
            drop(released_streaming_parts);
            if !keep_running {
                break;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::part_byte_size;
    use crate::usecase::agent_session::session::MessagePart;

    #[test]
    fn stream_emit_estimates_text_delta_size() {
        let part = MessagePart::Text {
            content: "hello".to_string(),
            parent_tool_use_id: None,
        };

        assert_eq!(part_byte_size(&part), 5);
    }
}
#[cfg(test)]
mod moved_tests {
    use super::super::external_agent::*;

    use super::super::permission::*;
    use super::super::process_registry::*;
    use super::super::recovery::*;

    use super::super::sdk_message::*;
    use super::super::session_lifecycle::*;

    use super::super::session_persistence::*;
    use super::super::shared::test_support::*;
    use super::super::shared::*;

    use super::super::stream_emit::*;
    use super::super::turn_event_log::*;

    use crate::usecase::agent_session::event_log::{TurnTokenUsage, WorkflowTurnCompleteInput};

    use crate::usecase::agent_session::session::{
        create_session_internal, parts_to_legacy, ChatMessage, MessagePart, MessageRole,
        SystemNotificationType,
    };

    use std::collections::HashMap;

    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use tokio::sync::Mutex;

    fn stream_task_status(
        task_tool_use_id: &str,
        status: &str,
        summary: Option<&str>,
    ) -> MessagePart {
        MessagePart::TaskStatus {
            task_tool_use_id: task_tool_use_id.to_string(),
            status: status.to_string(),
            description: Some(status.to_string()),
            summary: summary.map(str::to_string),
        }
    }

    fn stream_permission(request_id: &str, tool_use_id: &str, status: &str) -> MessagePart {
        MessagePart::Permission {
            request: serde_json::json!({
                "request_id": request_id,
                "tool_use_id": tool_use_id,
                "tool_name": "Bash",
                "input": {},
            }),
            status: status.to_string(),
            answers: None,
            parent_tool_use_id: None,
        }
    }

    #[tokio::test]
    async fn permission_request_token_fence_rejects_stale_and_accepts_active_turn() {
        let temp = tempfile::tempdir().unwrap();
        let app = tauri::test::mock_builder()
            .manage(crate::app_data_dir::TestDataDir(temp.path().to_path_buf()))
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
                "type": "permission_request",
                "request_id": "stale-permission",
                "tool_name": "Edit",
                "input": {},
                "tool_use_id": "toolu_stale",
                "turn_token": "old-agent-message",
            }),
            &mut state,
        )
        .await;

        {
            let map = handles.lock().await;
            let proc = map.get(&session.id).unwrap();
            assert_eq!(proc.turn_phase, TurnPhase::Streaming);
            assert_eq!(proc.active_turn_token.as_deref(), Some("new-agent-message"));
            assert!(
                !proc.streaming_parts.iter().any(|part| matches!(
                    part,
                    MessagePart::Permission {
                        status,
                        request,
                        ..
                    } if status == "pending"
                        && request.get("request_id").and_then(|v| v.as_str())
                            == Some("stale-permission")
                )),
                "stale permission_request must not create pending permission state"
            );
        }

        handle_external_bridge_message(
            &app.handle(),
            &store,
            &handles,
            &session.id,
            serde_json::json!({
                "type": "permission_request",
                "request_id": "active-permission",
                "tool_name": "Edit",
                "input": {},
                "tool_use_id": "toolu_active",
                "turn_token": "new-agent-message",
            }),
            &mut state,
        )
        .await;

        let removed = handles.lock().await.remove(&session.id);
        if let Some(mut proc) = removed {
            assert_eq!(proc.turn_phase, TurnPhase::WaitingPermission);
            assert_eq!(proc.active_turn_token.as_deref(), Some("new-agent-message"));
            assert!(
                proc.streaming_parts.iter().any(|part| matches!(
                    part,
                    MessagePart::Permission {
                        status,
                        request,
                        ..
                    } if status == "pending"
                        && request.get("request_id").and_then(|v| v.as_str())
                            == Some("active-permission")
                )),
                "matching permission_request should keep entering WaitingPermission"
            );
            let _ = proc.child.kill().await;
        }
    }

    #[tokio::test]
    async fn enqueue_pending_delta_accumulates_parts_and_bytes() {
        let mut proc = make_streaming_test_process();
        let delta = vec![
            MessagePart::Text {
                content: "abcde".to_string(),
                parent_tool_use_id: None,
            },
            MessagePart::Thinking {
                content: "fg".to_string(),
                parent_tool_use_id: None,
            },
        ];
        enqueue_pending_delta(&mut proc, &delta);
        assert_eq!(proc.pending_stream_parts.len(), 2);
        assert_eq!(proc.pending_stream_bytes, "abcde".len() + "fg".len());
    }

    #[tokio::test]
    async fn streaming_interval_elapsed_is_true_before_any_emit() {
        let proc = make_streaming_test_process();
        assert!(
            streaming_interval_elapsed(&proc),
            "first emit must not wait for an interval"
        );
    }

    #[tokio::test]
    async fn streaming_interval_elapsed_is_false_within_interval() {
        let mut proc = make_streaming_test_process();
        proc.last_stream_emit_at = Some(Instant::now());
        assert!(
            !streaming_interval_elapsed(&proc),
            "successive emit within {}ms must wait",
            STREAMING_EMIT_INTERVAL_MS
        );
    }

    #[tokio::test]
    async fn pending_exceeds_threshold_triggers_on_part_count() {
        let mut proc = make_streaming_test_process();
        for _ in 0..STREAMING_PENDING_PART_LIMIT {
            enqueue_pending_delta(
                &mut proc,
                &[MessagePart::Text {
                    content: "x".to_string(),
                    parent_tool_use_id: None,
                }],
            );
        }
        assert!(pending_exceeds_threshold(&proc));
    }

    #[tokio::test]
    async fn pending_exceeds_threshold_triggers_on_byte_count() {
        let mut proc = make_streaming_test_process();
        proc.pending_stream_bytes = STREAMING_PENDING_BYTE_LIMIT;
        assert!(pending_exceeds_threshold(&proc));
    }

    #[tokio::test]
    async fn pending_exceeds_threshold_returns_false_when_below_both_caps() {
        let mut proc = make_streaming_test_process();
        proc.pending_stream_bytes = STREAMING_PENDING_BYTE_LIMIT - 1;
        proc.pending_stream_parts.push(MessagePart::Text {
            content: "x".to_string(),
            parent_tool_use_id: None,
        });
        assert!(!pending_exceeds_threshold(&proc));
    }

    #[tokio::test]
    async fn streaming_interval_elapsed_is_true_after_interval() {
        let mut proc = make_streaming_test_process();
        proc.last_stream_emit_at =
            Some(Instant::now() - Duration::from_millis(STREAMING_EMIT_INTERVAL_MS + 5));
        assert!(streaming_interval_elapsed(&proc));
    }

    #[tokio::test]
    async fn prepare_streaming_flush_is_none_when_buffer_is_empty() {
        let proc = make_streaming_test_process();
        // 空バッファでは flush 準備が None になり、emit を発火しない。
        assert!(prepare_streaming_flush(&proc).is_none());
    }

    #[tokio::test]
    async fn prepare_streaming_flush_returns_pending_delta_parts() {
        let mut proc = make_streaming_test_process();
        proc.streaming_parts.push(MessagePart::Text {
            content: "Hel".to_string(),
            parent_tool_use_id: None,
        });
        proc.streaming_parts.push(MessagePart::Text {
            content: "lo".to_string(),
            parent_tool_use_id: None,
        });
        enqueue_pending_delta(
            &mut proc,
            &[MessagePart::Text {
                content: "lo".to_string(),
                parent_tool_use_id: None,
            }],
        );
        let snapshot = prepare_streaming_flush(&proc).expect("snapshot");
        assert_eq!(snapshot.seq, 1);
        assert_eq!(snapshot.parts.len(), 1);
        match &snapshot.parts[0] {
            MessagePart::Text { content, .. } => assert_eq!(content, "lo"),
            _ => panic!("expected pending Text delta part"),
        }
        assert_eq!(snapshot.buffer_len, 1);
        assert_eq!(snapshot.pending_bytes, "lo".len());
    }

    #[tokio::test]
    async fn prepare_streaming_flush_uses_actual_pending_update_parts() {
        let mut proc = make_streaming_test_process();
        proc.streaming_parts = vec![
            MessagePart::TaskStatus {
                task_tool_use_id: "tool-1".to_string(),
                status: "completed".to_string(),
                description: Some("done".to_string()),
                summary: None,
            },
            MessagePart::Text {
                content: "tail".to_string(),
                parent_tool_use_id: None,
            },
        ];
        let updated = MessagePart::TaskStatus {
            task_tool_use_id: "tool-1".to_string(),
            status: "completed".to_string(),
            description: Some("done".to_string()),
            summary: Some("summary".to_string()),
        };
        enqueue_pending_delta(&mut proc, std::slice::from_ref(&updated));

        let snapshot = prepare_streaming_flush(&proc).expect("snapshot");

        assert_eq!(snapshot.parts, vec![updated]);
    }

    #[tokio::test]
    async fn prepare_streaming_flush_routes_non_appendable_update_through_resync_gap() {
        let mut proc = make_streaming_test_process();
        let previous = MessagePart::TaskStatus {
            task_tool_use_id: "tool-1".to_string(),
            status: "started".to_string(),
            description: Some("old".to_string()),
            summary: None,
        };
        let updated = MessagePart::TaskStatus {
            task_tool_use_id: "tool-1".to_string(),
            status: "completed".to_string(),
            description: Some("done".to_string()),
            summary: Some("summary".to_string()),
        };
        let tail = MessagePart::Text {
            content: "tail".to_string(),
            parent_tool_use_id: None,
        };
        proc.streaming_delta_seq = 1;
        proc.streaming_parts = vec![updated.clone(), tail.clone()];
        proc.confirmed_stream_part_len = 2;
        enqueue_pending_delta_with_rollbacks(
            &mut proc,
            std::slice::from_ref(&updated),
            vec![StreamPartRollback { index: 0, previous }],
        );

        let snapshot = prepare_streaming_flush(&proc).expect("snapshot");

        assert_eq!(snapshot.seq, 3);
        assert!(
            snapshot.parts.is_empty(),
            "non-tail replacement must trigger resync instead of append-only delta"
        );
        assert_eq!(
            snapshot_parts_after_success(&proc, &snapshot),
            vec![updated, tail]
        );
    }

    #[tokio::test]
    async fn success_snapshot_and_confirmed_view_canonicalize_appended_identity_parts() {
        let mut proc = make_streaming_test_process();
        let old_status = stream_task_status("tool-1", "started", None);
        let old_permission = stream_permission("req-1", "tool-1", "pending");
        let updated_status = stream_task_status("tool-1", "completed", Some("done"));
        let updated_permission = stream_permission("req-1", "tool-1", "allowed");
        proc.streaming_delta_seq = 1;
        proc.confirmed_stream_part_len = 2;
        proc.streaming_parts = vec![
            old_status,
            old_permission,
            updated_status.clone(),
            updated_permission.clone(),
        ];
        enqueue_pending_delta(
            &mut proc,
            &[updated_status.clone(), updated_permission.clone()],
        );

        let snapshot = prepare_streaming_flush(&proc).expect("snapshot");

        assert_eq!(snapshot.seq, 3);
        assert!(
            snapshot.parts.is_empty(),
            "appended identity replacements must force resync instead of duplicate append"
        );
        let expected = vec![updated_status.clone(), updated_permission.clone()];
        assert_eq!(snapshot_parts_after_success(&proc, &snapshot), expected);

        let ok = apply_streaming_emit_result(&mut proc, "csid", "mid", &snapshot, true, true);

        assert!(ok);
        assert_eq!(confirmed_streaming_parts(&proc), expected);
    }

    #[tokio::test]
    async fn apply_streaming_emit_result_clears_pending_on_success() {
        let mut proc = make_streaming_test_process();
        enqueue_pending_delta(
            &mut proc,
            &[MessagePart::Text {
                content: "abc".to_string(),
                parent_tool_use_id: None,
            }],
        );
        let snapshot = prepare_streaming_flush(&proc).expect("snapshot");
        let ok = apply_streaming_emit_result(&mut proc, "csid", "mid", &snapshot, true, true);
        assert!(ok);
        assert_eq!(proc.pending_stream_parts.len(), 0);
        assert_eq!(proc.pending_stream_bytes, 0);
        assert!(proc.last_stream_emit_at.is_some());
    }

    #[tokio::test]
    async fn apply_streaming_emit_result_updates_confirmed_snapshot_on_success() {
        let mut proc = make_streaming_test_process();
        let previous = MessagePart::TaskStatus {
            task_tool_use_id: "tool-1".to_string(),
            status: "started".to_string(),
            description: None,
            summary: None,
        };
        let updated = MessagePart::TaskStatus {
            task_tool_use_id: "tool-1".to_string(),
            status: "completed".to_string(),
            description: None,
            summary: Some("done".to_string()),
        };
        proc.streaming_parts = vec![updated.clone()];
        proc.confirmed_stream_part_len = 1;
        enqueue_pending_delta_with_rollbacks(
            &mut proc,
            std::slice::from_ref(&updated),
            vec![StreamPartRollback { index: 0, previous }],
        );
        let snapshot = prepare_streaming_flush(&proc).expect("snapshot");

        let ok = apply_streaming_emit_result(&mut proc, "csid", "mid", &snapshot, true, true);

        assert!(ok);
        assert_eq!(confirmed_streaming_parts(&proc), vec![updated]);
    }

    #[tokio::test]
    async fn apply_streaming_emit_result_records_interval_on_second_success() {
        let _guard = crate::other::telemetry::lock_test_telemetry();
        crate::other::telemetry::reset_test_metrics();
        crate::other::telemetry::set_performance_configured(true);
        crate::other::telemetry::set_performance_enabled(true);
        let mut proc = make_streaming_test_process();
        proc.last_stream_emit_at = Some(Instant::now() - Duration::from_millis(25));
        enqueue_pending_delta(
            &mut proc,
            &[MessagePart::Text {
                content: "abc".to_string(),
                parent_tool_use_id: None,
            }],
        );
        let snapshot = prepare_streaming_flush(&proc).expect("snapshot");

        let ok = apply_streaming_emit_result(&mut proc, "csid", "mid", &snapshot, true, true);

        assert!(ok);
        let records = crate::other::telemetry::test_metric_records();
        assert!(records.iter().any(|record| {
            record.name == "releash.agent_stream.emit_interval_ms" && record.value >= 25.0
        }));
        crate::other::telemetry::reset_test_metrics();
    }

    #[tokio::test]
    async fn apply_streaming_emit_result_does_not_record_interval_on_first_success() {
        let _guard = crate::other::telemetry::lock_test_telemetry();
        crate::other::telemetry::reset_test_metrics();
        crate::other::telemetry::set_performance_configured(true);
        crate::other::telemetry::set_performance_enabled(true);
        let mut proc = make_streaming_test_process();
        enqueue_pending_delta(
            &mut proc,
            &[MessagePart::Text {
                content: "abc".to_string(),
                parent_tool_use_id: None,
            }],
        );
        let snapshot = prepare_streaming_flush(&proc).expect("snapshot");

        let ok = apply_streaming_emit_result(&mut proc, "csid", "mid", &snapshot, true, true);

        assert!(ok);
        assert!(!crate::other::telemetry::test_metric_records()
            .iter()
            .any(|record| record.name == "releash.agent_stream.emit_interval_ms"));
        crate::other::telemetry::reset_test_metrics();
    }

    #[tokio::test]
    async fn apply_streaming_emit_result_retains_pending_on_failure() {
        let mut proc = make_streaming_test_process();
        enqueue_pending_delta(
            &mut proc,
            &[MessagePart::Text {
                content: "abc".to_string(),
                parent_tool_use_id: None,
            }],
        );
        let snapshot = prepare_streaming_flush(&proc).expect("snapshot");
        // Tauri failed / WS ok
        let ok = apply_streaming_emit_result(&mut proc, "csid", "mid", &snapshot, false, true);
        assert!(!ok);
        assert_eq!(proc.pending_stream_parts.len(), 0);
        assert_eq!(proc.pending_stream_bytes, 0);
        let retry = proc.retry_stream_delta.as_ref().expect("retry delta");
        assert_eq!(retry.seq, 1);
        assert_eq!(retry.pending_bytes, "abc".len());
        assert!(
            proc.last_stream_emit_at.is_none(),
            "last_emit_at must not advance on failure"
        );
    }

    #[tokio::test]
    async fn apply_streaming_emit_result_does_not_record_interval_on_failure() {
        let _guard = crate::other::telemetry::lock_test_telemetry();
        crate::other::telemetry::reset_test_metrics();
        crate::other::telemetry::set_performance_configured(true);
        crate::other::telemetry::set_performance_enabled(true);
        let mut proc = make_streaming_test_process();
        proc.last_stream_emit_at = Some(Instant::now() - Duration::from_millis(25));
        enqueue_pending_delta(
            &mut proc,
            &[MessagePart::Text {
                content: "abc".to_string(),
                parent_tool_use_id: None,
            }],
        );
        let snapshot = prepare_streaming_flush(&proc).expect("snapshot");
        let records_before = crate::other::telemetry::test_metric_records();

        let ok = apply_streaming_emit_result(&mut proc, "csid", "mid", &snapshot, false, true);

        assert!(!ok);
        assert_eq!(
            crate::other::telemetry::test_metric_records(),
            records_before
        );
        crate::other::telemetry::reset_test_metrics();
    }

    #[tokio::test]
    async fn apply_streaming_emit_result_retains_when_both_channels_fail() {
        let mut proc = make_streaming_test_process();
        enqueue_pending_delta(
            &mut proc,
            &[MessagePart::Text {
                content: "abc".to_string(),
                parent_tool_use_id: None,
            }],
        );
        let snapshot = prepare_streaming_flush(&proc).expect("snapshot");
        let ok = apply_streaming_emit_result(&mut proc, "csid", "mid", &snapshot, false, false);
        assert!(!ok);
        assert_eq!(proc.pending_stream_parts.len(), 0);
        assert!(proc.retry_stream_delta.is_some());
        assert!(proc.last_stream_emit_at.is_none());
    }

    #[tokio::test]
    async fn next_flush_after_partial_failure_re_sends_same_seq_same_payload() {
        // 片方の transport だけに seq=1 が届いた後に新規 delta が来ても、
        // seq=1 retry payload は拡大せず、新規 delta は seq=2 に回す。
        let mut proc = make_streaming_test_process();
        proc.streaming_parts.push(MessagePart::Text {
            content: "Hel".to_string(),
            parent_tool_use_id: None,
        });
        enqueue_pending_delta(
            &mut proc,
            &[MessagePart::Text {
                content: "Hel".to_string(),
                parent_tool_use_id: None,
            }],
        );
        let first = prepare_streaming_flush(&proc).expect("first snapshot");
        apply_streaming_emit_result(&mut proc, "csid", "mid", &first, true, false);

        // 失敗後に次 delta が到着。
        proc.streaming_parts.push(MessagePart::Text {
            content: "lo".to_string(),
            parent_tool_use_id: None,
        });
        enqueue_pending_delta(
            &mut proc,
            &[MessagePart::Text {
                content: "lo".to_string(),
                parent_tool_use_id: None,
            }],
        );
        let second = prepare_streaming_flush(&proc).expect("second snapshot");
        assert_eq!(second.seq, 1);
        assert_eq!(second.parts.len(), 1);
        match &second.parts[0] {
            MessagePart::Text { content, .. } => assert_eq!(content, "Hel"),
            _ => panic!("expected consolidated Text"),
        }

        apply_streaming_emit_result(&mut proc, "csid", "mid", &second, true, true);
        let third = prepare_streaming_flush(&proc).expect("new pending delta");
        assert_eq!(third.seq, 2);
        match &third.parts[0] {
            MessagePart::Text { content, .. } => assert_eq!(content, "lo"),
            _ => panic!("expected Text"),
        }
    }

    #[tokio::test]
    async fn pending_can_overflow_thresholds_while_delivery_fails() {
        // 上限到達状態で配信失敗しても、追加 delta はバッファに保持される（ソフト上限）。
        let mut proc = make_streaming_test_process();
        // streaming_parts にも同等の cumulative を入れて prepare_streaming_flush が
        // snapshot を返せる状態にする。
        for _ in 0..STREAMING_PENDING_PART_LIMIT {
            let part = MessagePart::Text {
                content: "x".to_string(),
                parent_tool_use_id: None,
            };
            proc.streaming_parts.push(part.clone());
            enqueue_pending_delta(&mut proc, std::slice::from_ref(&part));
        }
        assert!(pending_exceeds_threshold(&proc));

        let snapshot = prepare_streaming_flush(&proc).expect("snapshot");
        apply_streaming_emit_result(&mut proc, "csid", "mid", &snapshot, false, true);

        enqueue_pending_delta(
            &mut proc,
            &[MessagePart::Text {
                content: "extra".to_string(),
                parent_tool_use_id: None,
            }],
        );
        assert!(proc.retry_stream_delta.is_some());
        assert_eq!(proc.pending_stream_parts.len(), 1);
    }

    #[tokio::test]
    async fn reset_streaming_state_for_new_turn_clears_all_coalescing_state() {
        // 前ターン残骸 (pending / last_emit_at / streaming_parts / last_message_id)
        // が新ターン開始時に確実にクリアされる。
        let mut proc = make_streaming_test_process();
        proc.streaming_parts.push(MessagePart::Text {
            content: "old".to_string(),
            parent_tool_use_id: None,
        });
        proc.confirmed_stream_part_len = 1;
        proc.pending_stream_parts.push(MessagePart::Text {
            content: "pending".to_string(),
            parent_tool_use_id: None,
        });
        proc.pending_stream_part_rollbacks.push(StreamPartRollback {
            index: 0,
            previous: MessagePart::Text {
                content: "old".to_string(),
                parent_tool_use_id: None,
            },
        });
        proc.retry_stream_delta = Some(PendingStreamDelta {
            seq: 1,
            parts: vec![MessagePart::Text {
                content: "retry".to_string(),
                parent_tool_use_id: None,
            }],
            part_count: 1,
            pending_bytes: "retry".len(),
            rollbacks: Vec::new(),
            confirmed_stream_part_len_after_success: 1,
        });
        proc.pending_stream_bytes = 32;
        proc.last_stream_emit_at = Some(Instant::now());
        proc.last_message_id = Some("old".to_string());
        proc.post_turn_base_untrusted_message_id = Some("old".to_string());
        proc.task_id_map
            .insert("task".to_string(), "tool".to_string());

        proc.reset_streaming_state_for_new_turn();

        assert!(proc.streaming_parts.is_empty());
        assert_eq!(proc.confirmed_stream_part_len, 0);
        assert!(proc.pending_stream_parts.is_empty());
        assert!(proc.pending_stream_part_rollbacks.is_empty());
        assert!(proc.retry_stream_delta.is_none());
        assert_eq!(proc.pending_stream_parts.len(), 0);
        assert_eq!(proc.pending_stream_bytes, 0);
        assert!(proc.last_stream_emit_at.is_none());
        assert!(proc.last_message_id.is_none());
        assert!(proc.post_turn_base_untrusted_message_id.is_none());
        assert!(proc.task_id_map.is_empty());

        // 新ターン直後は最初の emit が即時 flush される (= interval elapsed).
        assert!(streaming_interval_elapsed(&proc));
    }

    #[tokio::test]
    async fn second_flush_after_success_is_noop_until_new_delta() {
        // 強制 flush が同じ契機で連続呼ばれても、二重配信は起きない。
        let mut proc = make_streaming_test_process();
        proc.streaming_parts.push(MessagePart::Text {
            content: "Hello".to_string(),
            parent_tool_use_id: None,
        });
        enqueue_pending_delta(
            &mut proc,
            &[MessagePart::Text {
                content: "Hello".to_string(),
                parent_tool_use_id: None,
            }],
        );
        let snapshot = prepare_streaming_flush(&proc).expect("snapshot");
        assert!(apply_streaming_emit_result(
            &mut proc, "csid", "mid", &snapshot, true, true,
        ));

        assert!(prepare_streaming_flush(&proc).is_none(), "no double emit");
    }

    #[tokio::test]
    async fn forced_flush_continues_after_failure_for_state_transition() {
        // Spec: 強制配信が失敗しても後続の状態遷移は続行する。
        // apply_streaming_emit_result は false を返すだけで panic / abort せず、
        // 呼び出し元は戻り値を見ずに後続処理へ進められる。
        let mut proc = make_streaming_test_process();
        enqueue_pending_delta(
            &mut proc,
            &[MessagePart::Text {
                content: "tail".to_string(),
                parent_tool_use_id: None,
            }],
        );
        let snapshot = prepare_streaming_flush(&proc).expect("snapshot");
        // 失敗を返してもパニックしない（= 状態遷移を続行できる）。
        let _ = apply_streaming_emit_result(&mut proc, "csid", "mid", &snapshot, false, false);
        // retry payload は保持され、次の契機で再試行可能。
        assert_eq!(proc.pending_stream_parts.len(), 0);
        assert!(proc.retry_stream_delta.is_some());
    }

    #[tokio::test]
    async fn coalescing_first_delta_flushes_immediately() {
        // 初回 delta: last_stream_emit_at が None なので interval elapsed=true、
        // should_flush=true、flush_streaming で pending がクリアされる。
        let mut proc = make_streaming_test_process();
        let delta = vec![MessagePart::Text {
            content: "first".to_string(),
            parent_tool_use_id: None,
        }];
        proc.streaming_parts.extend(delta.clone());
        enqueue_pending_delta(&mut proc, &delta);

        assert!(should_flush_per_delta(&proc, &delta, false));
        let snapshot = prepare_streaming_flush(&proc).expect("first emit must flush");
        apply_streaming_emit_result(&mut proc, "csid", "mid", &snapshot, true, true);

        assert!(proc.pending_stream_parts.len() == 0);
        assert!(proc.last_stream_emit_at.is_some());
    }

    #[tokio::test]
    async fn coalescing_within_interval_does_not_flush() {
        // 配信直後（last_stream_emit_at=now）で続く delta が来ても、
        // 件数・byte 上限・tool event のいずれも当たらなければ flush しない。
        let mut proc = make_streaming_test_process();
        proc.last_stream_emit_at = Some(Instant::now());
        let delta = vec![MessagePart::Text {
            content: "tick".to_string(),
            parent_tool_use_id: None,
        }];
        proc.streaming_parts.extend(delta.clone());
        enqueue_pending_delta(&mut proc, &delta);

        assert!(!should_flush_per_delta(&proc, &delta, false));
        // pending は保持されたまま（次の契機まで蓄積される）。
        assert_eq!(proc.pending_stream_parts.len(), 1);
    }

    #[tokio::test]
    async fn coalescing_after_interval_flushes_accumulated_buffer() {
        // 直前配信から interval を超えて経過した状態で次 delta が来ると、
        // 既に溜まっている pending と新規 delta をまとめて flush する。
        let mut proc = make_streaming_test_process();
        proc.last_stream_emit_at =
            Some(Instant::now() - Duration::from_millis(STREAMING_EMIT_INTERVAL_MS + 5));
        let earlier = MessagePart::Text {
            content: "ear".to_string(),
            parent_tool_use_id: None,
        };
        proc.streaming_parts.push(earlier.clone());
        enqueue_pending_delta(&mut proc, std::slice::from_ref(&earlier));

        let new_delta = vec![MessagePart::Text {
            content: "lier".to_string(),
            parent_tool_use_id: None,
        }];
        proc.streaming_parts.extend(new_delta.clone());
        enqueue_pending_delta(&mut proc, &new_delta);

        assert!(should_flush_per_delta(&proc, &new_delta, false));
        let snapshot = prepare_streaming_flush(&proc).expect("must flush");
        // consolidated 後は 1 個の Text に統合される。
        assert_eq!(snapshot.parts.len(), 1);
        match &snapshot.parts[0] {
            MessagePart::Text { content, .. } => assert_eq!(content, "earlier"),
            _ => panic!("expected consolidated Text"),
        }
        apply_streaming_emit_result(&mut proc, "csid", "mid", &snapshot, true, true);
        assert!(proc.pending_stream_parts.len() == 0);
    }

    #[tokio::test]
    async fn coalescing_count_limit_forces_flush_within_interval() {
        // pending parts が件数上限に達していれば、interval 未経過でも force flush。
        // production 経路と同じ流れを踏ませる: enqueue_pending_delta で上限まで
        // 蓄積 → 新規 delta が到着 → flush snapshot に新規 delta が含まれる →
        // apply 成功で pending が空に戻る。
        let mut proc = make_streaming_test_process();
        proc.last_stream_emit_at = Some(Instant::now());
        for _ in 0..STREAMING_PENDING_PART_LIMIT {
            let part = MessagePart::Text {
                content: "x".to_string(),
                parent_tool_use_id: None,
            };
            proc.streaming_parts.push(part.clone());
            enqueue_pending_delta(&mut proc, std::slice::from_ref(&part));
        }
        assert!(pending_exceeds_threshold(&proc));

        // 新規 delta を production と同じ手順で蓄積する。
        let next = vec![MessagePart::Text {
            content: "y".to_string(),
            parent_tool_use_id: None,
        }];
        proc.streaming_parts.extend(next.clone());
        enqueue_pending_delta(&mut proc, &next);

        assert!(!streaming_interval_elapsed(&proc));
        assert!(should_flush_per_delta(&proc, &next, false));

        let snapshot =
            prepare_streaming_flush(&proc).expect("count-limit flush must produce snapshot");
        // consolidate 後は全 Text が 1 個に統合され、末尾は新規 delta の "y"。
        assert_eq!(snapshot.parts.len(), 1);
        match snapshot
            .parts
            .last()
            .expect("snapshot has at least one part")
        {
            MessagePart::Text { content, .. } => {
                assert!(
                    content.ends_with('y'),
                    "consolidated tail should be the new delta"
                );
            }
            _ => panic!("expected consolidated Text part"),
        }
        let ok = apply_streaming_emit_result(&mut proc, "csid", "mid", &snapshot, true, true);
        assert!(ok);
        assert!(proc.pending_stream_parts.len() == 0);
        assert_eq!(proc.pending_stream_bytes, 0);
    }

    #[tokio::test]
    async fn coalescing_byte_limit_forces_flush_within_interval() {
        // pending bytes が byte 上限に達していれば、interval 未経過でも force flush。
        // ハードコード値ではなく実装定数 STREAMING_PENDING_BYTE_LIMIT から算出する。
        // production 経路と同じ流れ: 上限相当の chunk を enqueue → 新規 delta
        // 到着 → flush snapshot に新規 delta が含まれる → apply 成功で pending 空。
        let mut proc = make_streaming_test_process();
        proc.last_stream_emit_at = Some(Instant::now());
        let chunk = "z".repeat(STREAMING_PENDING_BYTE_LIMIT);
        let part = MessagePart::Text {
            content: chunk,
            parent_tool_use_id: None,
        };
        proc.streaming_parts.push(part.clone());
        enqueue_pending_delta(&mut proc, std::slice::from_ref(&part));
        assert!(pending_exceeds_threshold(&proc));

        let next = vec![MessagePart::Text {
            content: "n".to_string(),
            parent_tool_use_id: None,
        }];
        proc.streaming_parts.extend(next.clone());
        enqueue_pending_delta(&mut proc, &next);

        assert!(!streaming_interval_elapsed(&proc));
        assert!(should_flush_per_delta(&proc, &next, false));

        let snapshot =
            prepare_streaming_flush(&proc).expect("byte-limit flush must produce snapshot");
        assert_eq!(snapshot.parts.len(), 1);
        match snapshot
            .parts
            .last()
            .expect("snapshot has at least one part")
        {
            MessagePart::Text { content, .. } => {
                assert!(
                    content.ends_with('n'),
                    "consolidated tail should be the new delta"
                );
            }
            _ => panic!("expected consolidated Text part"),
        }
        let ok = apply_streaming_emit_result(&mut proc, "csid", "mid", &snapshot, true, true);
        assert!(ok);
        assert!(proc.pending_stream_parts.len() == 0);
        assert_eq!(proc.pending_stream_bytes, 0);
    }

    #[tokio::test]
    async fn coalescing_tool_event_forces_flush_within_interval() {
        // tool start / end は interval 未経過でも即 flush（UI に古いフレームを残さない）。
        let mut proc = make_streaming_test_process();
        proc.last_stream_emit_at = Some(Instant::now());

        let delta_tool_use = vec![MessagePart::ToolUse {
            id: "tool-1".to_string(),
            tool: "Bash".to_string(),
            input: serde_json::json!({}),
            parent_tool_use_id: None,
        }];
        assert!(!streaming_interval_elapsed(&proc));
        assert!(should_flush_per_delta(&proc, &delta_tool_use, false));

        let delta_tool_result = vec![MessagePart::ToolResult {
            tool_use_id: Some("tool-1".to_string()),
            content: "ok".to_string(),
            is_error: false,
            parent_tool_use_id: None,
        }];
        assert!(should_flush_per_delta(&proc, &delta_tool_result, false));
    }

    #[tokio::test]
    async fn tool_event_flushes_pending_text_through_production_path() {
        // Spec (Rule: ターン完了・状態遷移時には未配信バッファを強制配信する,
        //  Examples ツール実行の開始 / 終了):
        //   未配信 text が pending に積まれている状態で ToolUse / ToolResult
        //   delta が到着すると、interval 未経過でも force flush され、
        //   pending text + tool event が同一の delta payload として
        //   emit され、emit 成功で pending が clear される。
        //
        // 本テストは production 経路 (enqueue_pending_delta →
        // prepare_streaming_flush → apply_streaming_emit_result) を最初から
        // 最後まで通し、ToolUse / ToolResult 双方について同じ流れを検証する。
        let mut proc = make_streaming_test_process();
        proc.last_stream_emit_at = Some(Instant::now());

        // 1) interval 未経過で未配信 text を pending に蓄積する。
        let pending_text = MessagePart::Text {
            content: "before-tool".to_string(),
            parent_tool_use_id: None,
        };
        proc.streaming_parts.push(pending_text.clone());
        enqueue_pending_delta(&mut proc, std::slice::from_ref(&pending_text));
        assert!(!streaming_interval_elapsed(&proc));
        assert_eq!(proc.pending_stream_parts.len(), 1);

        // 2) ToolUse delta が到着 → production と同じ手順で enqueue。
        let tool_use_delta = vec![MessagePart::ToolUse {
            id: "tool-1".to_string(),
            tool: "Bash".to_string(),
            input: serde_json::json!({"cmd": "ls"}),
            parent_tool_use_id: None,
        }];
        proc.streaming_parts.extend(tool_use_delta.clone());
        enqueue_pending_delta(&mut proc, &tool_use_delta);

        // tool event は interval 未経過でも force flush。
        assert!(!streaming_interval_elapsed(&proc));
        assert!(should_flush_per_delta(&proc, &tool_use_delta, false));

        // 3) prepare → emit (success) → apply で pending が clear される。
        let snapshot = prepare_streaming_flush(&proc).expect("tool start must produce snapshot");
        assert_eq!(snapshot.seq, 1);
        // delta payload には pending text + ToolUse が同一 emit で含まれる。
        assert_eq!(snapshot.parts.len(), 2);
        match &snapshot.parts[0] {
            MessagePart::Text { content, .. } => assert_eq!(content, "before-tool"),
            other => panic!("first delta part must be pending Text, got {other:?}"),
        }
        match &snapshot.parts[1] {
            MessagePart::ToolUse { id, tool, .. } => {
                assert_eq!(id, "tool-1");
                assert_eq!(tool, "Bash");
            }
            other => panic!("second delta part must be ToolUse, got {other:?}"),
        }
        let ok = apply_streaming_emit_result(&mut proc, "csid", "mid", &snapshot, true, true);
        assert!(ok, "tool start emit must succeed → pending cleared");
        assert_eq!(proc.pending_stream_parts.len(), 0);
        assert_eq!(proc.pending_stream_bytes, 0);

        // 4) 続いて ToolResult delta が到着 → 同じく force flush。
        //    last_stream_emit_at は直前の apply で now() に更新されている。
        let tool_result_delta = vec![MessagePart::ToolResult {
            tool_use_id: Some("tool-1".to_string()),
            content: "ok".to_string(),
            is_error: false,
            parent_tool_use_id: None,
        }];
        proc.streaming_parts.extend(tool_result_delta.clone());
        enqueue_pending_delta(&mut proc, &tool_result_delta);

        assert!(!streaming_interval_elapsed(&proc));
        assert!(should_flush_per_delta(&proc, &tool_result_delta, false));

        let snapshot2 = prepare_streaming_flush(&proc).expect("tool end must produce snapshot");
        assert_eq!(snapshot2.seq, 2);
        // 2 回目の payload は、新しく pending になった ToolResult delta のみ。
        assert_eq!(snapshot2.parts.len(), 1);
        assert!(matches!(
            snapshot2.parts.last(),
            Some(MessagePart::ToolResult { content, .. }) if content == "ok"
        ));
        let ok = apply_streaming_emit_result(&mut proc, "csid", "mid", &snapshot2, true, true);
        assert!(ok, "tool end emit must succeed → pending cleared");
        assert_eq!(proc.pending_stream_parts.len(), 0);
        assert_eq!(proc.pending_stream_bytes, 0);
    }

    #[tokio::test]
    async fn timer_flushes_when_pending_and_interval_elapsed() {
        // 本番の補助 timer (`spawn_streaming_timer`) は `run_streaming_timer_tick`
        // を毎 tick 呼ぶ。テストも同じ helper を直接呼び、pending と
        // last_stream_emit_at の更新まで含めた挙動を検証する。
        let mut proc = make_streaming_test_process();
        proc.last_stream_emit_at =
            Some(Instant::now() - Duration::from_millis(STREAMING_EMIT_INTERVAL_MS + 5));
        let part = MessagePart::Text {
            content: "silent".to_string(),
            parent_tool_use_id: None,
        };
        proc.streaming_parts.push(part.clone());
        enqueue_pending_delta(&mut proc, std::slice::from_ref(&part));

        let mut emitted = Vec::new();
        let tick_effect =
            run_streaming_timer_tick(&mut proc, "csid", |mid, _seq, parts, _snapshot_parts| {
                emitted.push((mid.to_string(), parts.to_vec()));
                (true, true)
            });

        assert!(
            tick_effect.keep_running,
            "still streaming → timer continues"
        );
        assert!(tick_effect.released_streaming_parts.is_empty());
        assert_eq!(emitted.len(), 1, "timer must call emit exactly once");
        assert_eq!(emitted[0].0, "m1");
        assert_eq!(emitted[0].1.len(), 1);
        assert_eq!(
            proc.pending_stream_parts.len(),
            0,
            "pending cleared on success"
        );
        assert!(
            proc.last_stream_emit_at.is_some(),
            "last_stream_emit_at updated on success"
        );
    }

    #[tokio::test]
    async fn timer_skips_when_pending_but_interval_not_elapsed() {
        let mut proc = make_streaming_test_process();
        proc.last_stream_emit_at = Some(Instant::now());
        let part = MessagePart::Text {
            content: "fresh".to_string(),
            parent_tool_use_id: None,
        };
        proc.streaming_parts.push(part.clone());
        enqueue_pending_delta(&mut proc, std::slice::from_ref(&part));

        let mut emitted = false;
        let tick_effect =
            run_streaming_timer_tick(&mut proc, "csid", |_mid, _seq, _parts, _snapshot_parts| {
                emitted = true;
                (true, true)
            });
        assert!(tick_effect.keep_running);
        assert!(tick_effect.released_streaming_parts.is_empty());
        assert!(!emitted, "interval not elapsed → timer must not flush");
        assert_eq!(proc.pending_stream_parts.len(), 1);
    }

    #[tokio::test]
    async fn timer_skips_when_pending_empty_even_if_interval_elapsed() {
        let mut proc = make_streaming_test_process();
        proc.last_stream_emit_at =
            Some(Instant::now() - Duration::from_millis(STREAMING_EMIT_INTERVAL_MS + 5));
        assert!(streaming_interval_elapsed(&proc));
        assert_eq!(proc.pending_stream_parts.len(), 0);

        let mut emitted = false;
        let tick_effect =
            run_streaming_timer_tick(&mut proc, "csid", |_mid, _seq, _parts, _snapshot_parts| {
                emitted = true;
                (true, true)
            });
        // pending=0 & still Streaming → continue running but no flush this tick.
        assert!(tick_effect.keep_running);
        assert!(tick_effect.released_streaming_parts.is_empty());
        assert!(!emitted);
    }

    #[tokio::test]
    async fn timer_exits_when_turn_ended_and_buffer_empty() {
        // turn 終了 (state != Streaming) かつ pending が空になった時点で timer は
        // ループを終了させるべき。これを `run_streaming_timer_tick` の戻り値で表現する。
        let mut proc = make_streaming_test_process();
        proc.state = BridgeState::Ready;
        proc.last_stream_emit_at =
            Some(Instant::now() - Duration::from_millis(STREAMING_EMIT_INTERVAL_MS + 5));
        assert_eq!(proc.pending_stream_parts.len(), 0);

        let tick_effect =
            run_streaming_timer_tick(&mut proc, "csid", |_mid, _seq, _parts, _snapshot_parts| {
                (true, true)
            });
        assert!(
            !tick_effect.keep_running,
            "turn ended (state != Streaming) and buffer empty → timer must exit"
        );
        assert!(tick_effect.released_streaming_parts.is_empty());
    }

    #[tokio::test]
    async fn timer_drains_pending_even_after_turn_ended() {
        // turn 終了直後でも pending が残っていれば drain し、成功時は
        // 完了済み streaming buffer を解放して timer を終了する。
        let mut proc = make_streaming_test_process();
        proc.state = BridgeState::Ready;
        proc.last_stream_emit_at =
            Some(Instant::now() - Duration::from_millis(STREAMING_EMIT_INTERVAL_MS + 5));
        let part = MessagePart::Text {
            content: "tail".to_string(),
            parent_tool_use_id: None,
        };
        proc.streaming_parts.push(part.clone());
        enqueue_pending_delta(&mut proc, std::slice::from_ref(&part));

        let mut emitted = 0usize;
        let tick_effect =
            run_streaming_timer_tick(&mut proc, "csid", |_mid, _seq, _parts, _snapshot_parts| {
                emitted += 1;
                (true, true)
            });
        assert!(
            !tick_effect.keep_running,
            "turn ended and pending drained → timer exits immediately"
        );
        assert_eq!(tick_effect.released_streaming_parts, vec![part]);
        assert_eq!(emitted, 1, "tail content flushed before exit");
        assert_eq!(proc.pending_stream_parts.len(), 0);
        assert!(proc.streaming_parts.is_empty());
    }

    #[tokio::test]
    async fn streaming_timer_decision_continue_when_generation_matches_and_streaming() {
        let proc = make_streaming_test_process();
        assert_eq!(
            streaming_timer_decision(&proc, proc.generation_id),
            TimerDecision::Continue
        );
    }

    #[tokio::test]
    async fn streaming_timer_decision_break_keep_flag_on_generation_mismatch() {
        // 新しい turn (generation_id 更新) が同じ csid を再利用したケース。
        // 既存 timer は自分の captured generation と一致しないので flag を残して
        // 終了する (新 timer が flag を所有しているため触らない)。
        let mut proc = make_streaming_test_process();
        let captured = proc.generation_id;
        proc.generation_id = captured.wrapping_add(1);
        proc.streaming_timer_active = true;
        assert_eq!(
            streaming_timer_decision(&proc, captured),
            TimerDecision::BreakKeepFlag
        );
    }

    #[tokio::test]
    async fn streaming_timer_decision_break_clear_flag_on_crash() {
        // 同一 generation で Crashed に遷移したら drain 不要なので flag を解放して
        // 終了する。
        let mut proc = make_streaming_test_process();
        proc.state = BridgeState::Crashed;
        assert_eq!(
            streaming_timer_decision(&proc, proc.generation_id),
            TimerDecision::BreakClearFlag
        );
    }

    #[tokio::test]
    async fn try_mark_streaming_timer_active_marks_idle_and_returns_true() {
        let mut proc = make_streaming_test_process();
        assert!(!proc.streaming_timer_active);
        assert!(try_mark_streaming_timer_active(&mut proc));
        assert!(proc.streaming_timer_active);
    }

    #[tokio::test]
    async fn try_mark_streaming_timer_active_returns_false_when_already_active() {
        // Duplicate spawn no-op: 同じ turn で 2 回目の spawn_streaming_timer を
        // 呼んでも flag は既に true なので false を返し新 task を起こさない。
        let mut proc = make_streaming_test_process();
        proc.streaming_timer_active = true;
        assert!(!try_mark_streaming_timer_active(&mut proc));
        assert!(proc.streaming_timer_active, "flag must remain set");
    }

    #[tokio::test]
    async fn forced_flush_emits_pending_before_state_transition_inputs() {
        // 強制 flush の呼び出し元（turn_complete / permission_request / error /
        // tool start/end）は、まず flush_streaming で pending を排出してから
        // state 遷移用の値（emit_session_state_changed の引数等）を組み立てる。
        // 本テストは「flush 完了後に pending が空になっている」ことを通じて、
        // 後続の状態通知が flush 済みデータの後で発火することを担保する。
        let mut proc = make_streaming_test_process();
        let delta = vec![MessagePart::Text {
            content: "tail-before-state".to_string(),
            parent_tool_use_id: None,
        }];
        proc.streaming_parts.extend(delta.clone());
        enqueue_pending_delta(&mut proc, &delta);

        // forced flush の中身: snapshot → emit (mocked success) → apply
        let snapshot = prepare_streaming_flush(&proc).expect("pending must yield snapshot");
        let ok = apply_streaming_emit_result(&mut proc, "csid", "mid", &snapshot, true, true);
        assert!(
            ok,
            "forced flush succeeded → pending cleared before state emit"
        );
        assert!(proc.pending_stream_parts.len() == 0);

        // この時点で呼び出し元が emit_session_state_changed を発火する。pending は
        // 既にクリアされているので、状態通知より前にストリーム emit が完了している。
    }

    #[tokio::test]
    async fn forced_flush_is_noop_when_no_pending_avoiding_double_delivery() {
        // 直前の強制 flush で pending を空にしている状態で再度同じ契機 (e.g. error 経路
        // と直後の EOF 経路) が forced flush を呼んでも、prepare_streaming_flush が
        // None を返すため二重配信は発生しない。
        let mut proc = make_streaming_test_process();
        proc.streaming_parts.push(MessagePart::Text {
            content: "once".to_string(),
            parent_tool_use_id: None,
        });
        enqueue_pending_delta(
            &mut proc,
            &[MessagePart::Text {
                content: "once".to_string(),
                parent_tool_use_id: None,
            }],
        );
        let snapshot = prepare_streaming_flush(&proc).expect("first snapshot");
        assert!(apply_streaming_emit_result(
            &mut proc, "csid", "mid", &snapshot, true, true,
        ));

        // 二度目の forced flush は no-op になる。
        assert!(prepare_streaming_flush(&proc).is_none());
    }

    #[tokio::test]
    async fn forced_flush_failure_does_not_block_followup_processing() {
        // Spec: 強制配信が失敗しても後続の状態遷移は続行する。失敗時 pending と
        // last_stream_emit_at は保持され、apply は false を返すのみ（panic しない）。
        let mut proc = make_streaming_test_process();
        let delta = vec![MessagePart::Text {
            content: "kept".to_string(),
            parent_tool_use_id: None,
        }];
        proc.streaming_parts.extend(delta.clone());
        enqueue_pending_delta(&mut proc, &delta);
        let snapshot = prepare_streaming_flush(&proc).expect("snapshot");
        let ok = apply_streaming_emit_result(&mut proc, "csid", "mid", &snapshot, false, false);
        assert!(!ok);
        // 呼び出し元はここから後続 (emit_session_state_changed 等) に進める。
        // retry payload と last_stream_emit_at は次の契機での再試行のため保持される。
        assert_eq!(proc.pending_stream_parts.len(), 0);
        assert!(proc.retry_stream_delta.is_some());
        assert!(proc.last_stream_emit_at.is_none());
    }

    #[tokio::test]
    async fn normal_turn_complete_allows_same_token_post_turn_updates() {
        let mut proc = make_streaming_test_process();
        proc.streaming_message_id = Some("agent-message-1".to_string());
        proc.active_turn_token = Some("agent-message-1".to_string());

        let mut events = Vec::new();
        drive_turn_complete_path(&mut proc, "csid", 0, &mut events);

        assert!(proc.active_turn_token.is_none());
        assert_eq!(
            proc.post_turn_message_token.as_deref(),
            Some("agent-message-1")
        );
        assert_eq!(proc.last_message_id.as_deref(), Some("agent-message-1"));

        let post_turn_msg = serde_json::json!({
            "type": "stream_event",
            "turn_token": "agent-message-1",
            "event": {
                "type": "content_block_delta",
                "delta": {"type": "text_delta", "text": "background task update"}
            }
        });
        assert!(!bridge_message_is_stale_for_active_turn(
            &proc,
            &post_turn_msg
        ));
    }

    #[tokio::test]
    async fn nonzero_turn_complete_does_not_allow_post_turn_token() {
        let mut proc = make_streaming_test_process();
        proc.streaming_message_id = Some("agent-message-1".to_string());
        proc.active_turn_token = Some("agent-message-1".to_string());

        let mut events = Vec::new();
        drive_turn_complete_path(&mut proc, "csid", 1, &mut events);

        assert!(proc.active_turn_token.is_none());
        assert!(proc.post_turn_message_token.is_none());

        let late_msg = serde_json::json!({
            "type": "stream_event",
            "turn_token": "agent-message-1",
        });
        assert!(bridge_message_is_stale_for_active_turn(&proc, &late_msg));
    }

    #[tokio::test]
    async fn permission_request_emits_pending_before_state_change() {
        // Spec (Rule: ターン完了・状態遷移時には未配信バッファを強制配信する):
        //   ストリーミング → 権限待ち の遷移時、未配信 delta が state 通知より
        //   前にフロントエンドへ配信されること。
        let mut proc = make_streaming_test_process();
        let delta = vec![MessagePart::Text {
            content: "tail-before-perm".to_string(),
            parent_tool_use_id: None,
        }];
        proc.streaming_parts.extend(delta.clone());
        enqueue_pending_delta(&mut proc, &delta);

        let mut events = Vec::new();
        let transitioned = drive_permission_request_path(&mut proc, "csid", &mut events);
        assert!(transitioned);

        assert_eq!(events.len(), 2, "both emits must fire");
        match &events[0] {
            RecordedEmit::StreamingFlush {
                parts_count,
                tail_text,
            } => {
                assert_eq!(*parts_count, 1);
                assert_eq!(tail_text.as_deref(), Some("tail-before-perm"));
            }
            other => panic!("first emit must be StreamingFlush, got {other:?}"),
        }
        assert_eq!(
            events[1],
            RecordedEmit::StateChanged {
                phase: TurnPhase::WaitingPermission,
                exit_code: None,
            }
        );
        assert!(proc.pending_stream_parts.len() == 0);
        assert_eq!(proc.turn_phase, TurnPhase::WaitingPermission);
    }

    #[tokio::test]
    async fn permission_request_without_pending_skips_streaming_emit() {
        // pending が空のとき、state 通知のみが発火し、ストリーム emit は
        // 起きない (prepare_streaming_flush が None → no-op)。
        let mut proc = make_streaming_test_process();

        let mut events = Vec::new();
        assert!(drive_permission_request_path(
            &mut proc,
            "csid",
            &mut events,
        ));

        assert_eq!(
            events,
            vec![RecordedEmit::StateChanged {
                phase: TurnPhase::WaitingPermission,
                exit_code: None,
            }]
        );
    }

    #[tokio::test]
    async fn turn_complete_emits_pending_before_state_change() {
        // Spec: ターン完了時に未配信バッファを強制配信する。
        // streaming emit が state emit (Idle) より前に観測される。
        let mut proc = make_streaming_test_process();
        begin_test_turn_event_log(&mut proc);
        let delta = vec![MessagePart::Text {
            content: "tail-before-idle".to_string(),
            parent_tool_use_id: None,
        }];
        proc.streaming_parts.extend(delta.clone());
        enqueue_pending_delta(&mut proc, &delta);

        let mut events = Vec::new();
        drive_turn_complete_path(&mut proc, "csid", 0, &mut events);

        assert_eq!(events.len(), 2);
        match &events[0] {
            RecordedEmit::StreamingFlush {
                parts_count,
                tail_text,
            } => {
                assert_eq!(*parts_count, 1);
                assert_eq!(tail_text.as_deref(), Some("tail-before-idle"));
            }
            other => panic!("first emit must be StreamingFlush, got {other:?}"),
        }
        assert_eq!(
            events[1],
            RecordedEmit::StateChanged {
                phase: TurnPhase::Idle,
                exit_code: Some(0),
            }
        );
        assert!(proc.pending_stream_parts.len() == 0);
        assert_eq!(proc.turn_phase, TurnPhase::Idle);
        assert_eq!(proc.state, BridgeState::Ready);
    }

    #[tokio::test]
    async fn turn_complete_with_nonzero_exit_code_still_flushes_before_state() {
        // 失敗終了 (exit_code != 0) でも emit 順序は同じ: streaming → state。
        let mut proc = make_streaming_test_process();
        begin_test_turn_event_log(&mut proc);
        let delta = vec![MessagePart::Text {
            content: "tail-error".to_string(),
            parent_tool_use_id: None,
        }];
        proc.streaming_parts.extend(delta.clone());
        enqueue_pending_delta(&mut proc, &delta);

        let mut events = Vec::new();
        drive_turn_complete_path(&mut proc, "csid", 1, &mut events);

        assert!(matches!(events[0], RecordedEmit::StreamingFlush { .. }));
        assert_eq!(
            events[1],
            RecordedEmit::StateChanged {
                phase: TurnPhase::Idle,
                exit_code: Some(1),
            }
        );
        assert_eq!(proc.state, BridgeState::Crashed);
    }

    #[tokio::test]
    async fn turn_complete_without_turn_started_flushes_but_skips_projection_followup_gate() {
        for backend_id in [CLAUDE_BACKEND_ID, CODEX_BACKEND_ID] {
            let mut proc = make_streaming_test_process();
            proc.backend_id = backend_id.to_string();
            let delta = vec![MessagePart::Text {
                content: format!("tail-{backend_id}"),
                parent_tool_use_id: None,
            }];
            proc.streaming_parts.extend(delta.clone());
            enqueue_pending_delta(&mut proc, &delta);

            let mut events = Vec::new();
            let effect = run_turn_complete_transition_locked(
                &mut proc,
                "csid",
                0,
                recording_emit(&mut events),
            );

            assert!(!effect.turn_completed);
            assert!(effect.workflow_turn_complete.is_none());
            assert!(effect.projected_session_state.is_none());
            assert_eq!(
                events.len(),
                1,
                "stream snapshot still flushes for {backend_id}"
            );
            assert!(matches!(events[0], RecordedEmit::StreamingFlush { .. }));
            assert_eq!(proc.turn_event_log.current_turn_id(), None);
            assert!(proc
                .turn_event_log
                .project()
                .workflow_turn_complete
                .is_none());
            assert_eq!(proc.state, BridgeState::Ready);
            assert_eq!(proc.turn_phase, TurnPhase::Idle);
            let _ = proc.child.kill().await;
        }
    }

    #[tokio::test]
    async fn turn_complete_releases_streaming_parts_after_final_snapshot() {
        let mut proc = make_streaming_test_process();
        begin_test_turn_event_log(&mut proc);
        proc.task_id_map
            .insert("background-1".to_string(), "tool-1".to_string());
        let raw_parts = vec![
            MessagePart::Text {
                content: "hello".to_string(),
                parent_tool_use_id: None,
            },
            MessagePart::Text {
                content: " world".to_string(),
                parent_tool_use_id: None,
            },
            MessagePart::ToolUse {
                tool: "Bash".to_string(),
                input: serde_json::json!({ "cmd": "echo ok" }),
                id: "tool-1".to_string(),
                parent_tool_use_id: None,
            },
        ];
        proc.streaming_parts.extend(raw_parts.clone());
        enqueue_pending_delta(&mut proc, &raw_parts);

        let effect = run_turn_complete_transition_locked(
            &mut proc,
            "csid",
            0,
            |_mid, _seq, _parts, _snapshot_parts| (true, true),
        );

        assert!(effect.turn_completed);
        assert_eq!(effect.final_msg_id.as_deref(), Some("m1"));
        assert_eq!(effect.final_parts, consolidate_parts_from_slice(&raw_parts));
        assert_eq!(effect.released_streaming_parts, raw_parts);
        assert_eq!(proc.state, BridgeState::Ready);
        assert_eq!(proc.turn_phase, TurnPhase::Idle);
        assert_eq!(proc.last_message_id.as_deref(), Some("m1"));
        assert_eq!(
            proc.post_turn_base_untrusted_message_id.as_deref(),
            Some("m1")
        );
        clear_post_turn_store_base_untrusted_after_persist_success(&mut proc, "m1");
        assert!(proc.post_turn_base_untrusted_message_id.is_none());
        assert_eq!(
            proc.task_id_map.get("background-1").map(String::as_str),
            Some("tool-1")
        );
        assert!(proc.streaming_parts.is_empty());
        assert_eq!(proc.pending_stream_parts.len(), 0);
        assert_eq!(proc.pending_stream_bytes, 0);
        assert!(proc.last_stream_emit_at.is_none());
        let _ = proc.child.kill().await;
    }

    #[tokio::test]
    async fn permission_request_transition_carries_projected_active_session_state() {
        let mut proc = make_streaming_test_process();
        begin_test_turn_event_log(&mut proc);
        let permission_part = MessagePart::Permission {
            request: serde_json::json!({ "request_id": "req-1" }),
            status: "pending".to_string(),
            answers: None,
            parent_tool_use_id: None,
        };
        proc.streaming_parts.push(permission_part.clone());
        enqueue_pending_delta(&mut proc, std::slice::from_ref(&permission_part));
        record_durable_parts_for_current_turn(
            &mut proc,
            "m1",
            std::slice::from_ref(&permission_part),
        );

        let effect = run_permission_request_transition_locked(
            &mut proc,
            "csid",
            None,
            Instant::now(),
            |_mid, _seq, _parts, _snapshot_parts| (true, true),
        );

        assert!(effect.did_transition);
        assert_eq!(
            effect.projected_session_state,
            Some(crate::usecase::agent_session::session::SessionState::Active)
        );
        assert_eq!(proc.turn_phase, TurnPhase::WaitingPermission);
        let _ = proc.child.kill().await;
    }

    #[tokio::test]
    async fn post_turn_skips_stale_store_base_when_final_persist_failed() {
        let mut proc = make_streaming_test_process();
        begin_test_turn_event_log(&mut proc);
        let fresh_parts = vec![
            MessagePart::Text {
                content: "fresh base".to_string(),
                parent_tool_use_id: None,
            },
            MessagePart::ToolUse {
                tool: "Bash".to_string(),
                input: serde_json::json!({ "cmd": "date" }),
                id: "tool-1".to_string(),
                parent_tool_use_id: None,
            },
        ];
        proc.streaming_parts.extend(fresh_parts.clone());
        enqueue_pending_delta(&mut proc, &fresh_parts);

        let complete_effect = run_turn_complete_transition_locked(
            &mut proc,
            "csid",
            0,
            |_mid, _seq, _parts, _snapshot_parts| (true, true),
        );

        assert_eq!(complete_effect.final_msg_id.as_deref(), Some("m1"));
        assert_eq!(
            proc.post_turn_base_untrusted_message_id.as_deref(),
            Some("m1"),
            "simulates the final persist failing and leaving the store base stale"
        );
        assert!(proc.streaming_parts.is_empty());

        let stale_store_base = vec![
            MessagePart::Text {
                content: "stale base".to_string(),
                parent_tool_use_id: None,
            },
            MessagePart::ToolUse {
                tool: "Bash".to_string(),
                input: serde_json::json!({ "cmd": "date" }),
                id: "tool-1".to_string(),
                parent_tool_use_id: None,
            },
        ];
        let msg = post_turn_tool_result_message("tool-1", "must-not-overwrite");
        let mut emitted = false;

        let post_turn_effect = accumulate_stream_or_post_turn_message_locked(
            &mut proc,
            "csid",
            &msg,
            0,
            |_mid, _seq, _parts, _snapshot_parts| {
                emitted = true;
                (true, true)
            },
            Some(("m1".to_string(), stale_store_base)),
        );

        assert!(post_turn_effect.accumulated);
        assert!(post_turn_effect.emit_msg_id.is_none());
        assert!(!post_turn_effect.should_persist);
        assert!(post_turn_effect.persist_parts.is_empty());
        assert!(!emitted);
        assert!(proc.streaming_parts.is_empty());
        assert_eq!(
            proc.post_turn_base_untrusted_message_id.as_deref(),
            Some("m1")
        );
        let _ = proc.child.kill().await;
    }

    #[tokio::test]
    async fn turn_complete_emit_failure_retains_retry_state_until_timer_drains() {
        let mut proc = make_streaming_test_process();
        begin_test_turn_event_log(&mut proc);
        let raw_parts = vec![MessagePart::Text {
            content: "tail".to_string(),
            parent_tool_use_id: None,
        }];
        proc.streaming_parts.extend(raw_parts.clone());
        enqueue_pending_delta(&mut proc, &raw_parts);

        let effect = run_turn_complete_transition_locked(
            &mut proc,
            "csid",
            0,
            |_mid, _seq, _parts, _snapshot_parts| (false, true),
        );

        assert!(effect.turn_completed);
        assert_eq!(effect.final_msg_id.as_deref(), Some("m1"));
        assert_eq!(effect.final_parts, consolidate_parts_from_slice(&raw_parts));
        assert!(effect.released_streaming_parts.is_empty());
        assert_eq!(proc.state, BridgeState::Ready);
        assert_eq!(proc.last_message_id.as_deref(), Some("m1"));
        assert_eq!(proc.pending_stream_parts.len(), 0);
        assert!(proc.retry_stream_delta.is_some());
        assert_eq!(proc.streaming_parts, raw_parts);
        assert!(
            proc.last_stream_emit_at.is_none(),
            "failed emit must remain retryable"
        );

        let mut emitted: Vec<(String, Vec<MessagePart>)> = Vec::new();
        let tick_effect =
            run_streaming_timer_tick(&mut proc, "csid", |mid, _seq, parts, _snapshot_parts| {
                emitted.push((mid.to_string(), parts.to_vec()));
                (true, true)
            });

        assert!(!tick_effect.keep_running);
        assert_eq!(tick_effect.released_streaming_parts, raw_parts.clone());
        assert_eq!(emitted.len(), 1);
        assert_eq!(emitted[0].0, "m1");
        assert_eq!(emitted[0].1, consolidate_parts_from_slice(&raw_parts));
        assert!(proc.streaming_parts.is_empty());
        assert_eq!(proc.pending_stream_parts.len(), 0);
        assert_eq!(proc.pending_stream_bytes, 0);
        let _ = proc.child.kill().await;
    }

    #[tokio::test]
    async fn turn_complete_nonzero_exit_code_emit_failure_releases_after_final_snapshot() {
        let mut proc = make_streaming_test_process();
        begin_test_turn_event_log(&mut proc);
        let raw_parts = vec![MessagePart::Text {
            content: "tail-before-crash".to_string(),
            parent_tool_use_id: None,
        }];
        proc.streaming_parts.extend(raw_parts.clone());
        enqueue_pending_delta(&mut proc, &raw_parts);
        let mut emit_attempts = 0usize;

        let effect = run_turn_complete_transition_locked(
            &mut proc,
            "csid",
            1,
            |_mid, _seq, _parts, _snapshot_parts| {
                emit_attempts += 1;
                (false, true)
            },
        );

        assert!(effect.turn_completed);
        assert_eq!(effect.final_msg_id.as_deref(), Some("m1"));
        assert_eq!(effect.final_parts, consolidate_parts_from_slice(&raw_parts));
        assert_eq!(effect.released_streaming_parts, raw_parts);
        assert_eq!(emit_attempts, 1);
        assert_eq!(proc.state, BridgeState::Crashed);
        assert_eq!(proc.turn_phase, TurnPhase::Idle);
        assert_eq!(proc.last_message_id.as_deref(), Some("m1"));
        assert!(proc.streaming_parts.is_empty());
        assert_eq!(proc.pending_stream_parts.len(), 0);
        assert_eq!(proc.pending_stream_bytes, 0);
        let _ = proc.child.kill().await;
    }

    #[tokio::test]
    async fn post_turn_permission_mode_notification_skips_reseed_and_persist() {
        let mut proc = make_test_agent_process();
        proc.state = BridgeState::Ready;
        proc.turn_phase = TurnPhase::Idle;
        proc.last_message_id = Some("m1".to_string());
        let msg = serde_json::json!({
            "type": "system",
            "permissionMode": "acceptEdits"
        });
        let mut emitted = false;

        let effect = accumulate_stream_or_post_turn_message_locked(
            &mut proc,
            "csid",
            &msg,
            0,
            |_mid, _seq, _parts, _snapshot_parts| {
                emitted = true;
                (true, true)
            },
            None,
        );

        assert!(!effect.accumulated);
        assert!(effect.post_turn_reseed_message_id.is_none());
        assert!(effect.emit_msg_id.is_none());
        assert!(!effect.should_persist);
        assert!(effect.persist_parts.is_empty());
        assert!(!emitted);
        assert!(proc.streaming_parts.is_empty());
        assert_eq!(proc.pending_stream_parts.len(), 0);
        assert!(should_forward_sdk_message(effect.accumulated, "system"));
        let _ = proc.child.kill().await;
    }

    #[tokio::test]
    async fn post_turn_partless_system_notification_skips_reseed_emit_and_persist() {
        let mut proc = make_test_agent_process();
        proc.state = BridgeState::Ready;
        proc.turn_phase = TurnPhase::Idle;
        proc.last_message_id = Some("m1".to_string());
        let msg = serde_json::json!({
            "type": "system",
            "subtype": "hook_started",
            "hook_name": "SessionEnd",
            "hook_event": "StopSession",
            "hook_id": "hook-001"
        });
        let mut emitted = false;

        let effect = accumulate_stream_or_post_turn_message_locked(
            &mut proc,
            "csid",
            &msg,
            0,
            |_mid, _seq, _parts, _snapshot_parts| {
                emitted = true;
                (true, true)
            },
            None,
        );

        assert!(effect.accumulated);
        assert!(effect.post_turn_reseed_message_id.is_none());
        assert!(effect.emit_msg_id.is_none());
        assert!(!effect.should_persist);
        assert!(effect.persist_parts.is_empty());
        assert!(!emitted);
        assert!(proc.streaming_parts.is_empty());
        assert_eq!(proc.pending_stream_parts.len(), 0);
        let _ = proc.child.kill().await;
    }

    #[tokio::test]
    async fn post_turn_reseed_preserves_cumulative_payload_and_releases_again() {
        let mut proc = make_test_agent_process();
        proc.state = BridgeState::Ready;
        proc.turn_phase = TurnPhase::Idle;
        proc.last_message_id = Some("m1".to_string());
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
        let delta_part = MessagePart::ToolResult {
            content: "done".to_string(),
            is_error: false,
            tool_use_id: Some("tool-1".to_string()),
            parent_tool_use_id: None,
        };
        let expected_delta_parts = vec![delta_part.clone()];
        let expected_parts = consolidate_parts_from_slice(
            &base_parts
                .iter()
                .cloned()
                .chain(std::iter::once(delta_part.clone()))
                .collect::<Vec<_>>(),
        );
        let msg = post_turn_tool_result_message("tool-1", "done");
        let mut emitted: Vec<Vec<MessagePart>> = Vec::new();

        let effect = accumulate_stream_or_post_turn_message_locked(
            &mut proc,
            "csid",
            &msg,
            0,
            |_mid, _seq, parts, _snapshot_parts| {
                emitted.push(parts.to_vec());
                (true, true)
            },
            Some(("m1".to_string(), base_parts.clone())),
        );

        assert!(effect.accumulated);
        assert_eq!(effect.emit_msg_id.as_deref(), Some("m1"));
        assert!(effect.should_persist);
        assert_eq!(effect.persist_parts, expected_parts);
        assert_eq!(effect.released_streaming_parts, expected_parts.clone());
        assert_eq!(emitted, vec![expected_delta_parts]);
        assert!(proc.streaming_parts.is_empty());
        assert_eq!(proc.pending_stream_parts.len(), 0);
        assert_eq!(proc.pending_stream_bytes, 0);
        let _ = proc.child.kill().await;
    }

    #[tokio::test]
    async fn post_turn_emit_failure_requests_timer_restart_when_idle() {
        let mut proc = make_test_agent_process();
        proc.state = BridgeState::Ready;
        proc.turn_phase = TurnPhase::Idle;
        proc.last_message_id = Some("m1".to_string());
        assert!(!proc.streaming_timer_active);
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
        let delta_part = MessagePart::ToolResult {
            content: "done".to_string(),
            is_error: false,
            tool_use_id: Some("tool-1".to_string()),
            parent_tool_use_id: None,
        };
        let expected_delta_parts = vec![delta_part.clone()];
        let expected_parts = consolidate_parts_from_slice(
            &base_parts
                .iter()
                .cloned()
                .chain(std::iter::once(delta_part.clone()))
                .collect::<Vec<_>>(),
        );
        let msg = post_turn_tool_result_message("tool-1", "done");
        let mut emitted_attempts = 0;

        let effect = accumulate_stream_or_post_turn_message_locked(
            &mut proc,
            "csid",
            &msg,
            0,
            |_mid, _seq, parts, _snapshot_parts| {
                emitted_attempts += 1;
                assert_eq!(parts, expected_delta_parts.as_slice());
                (false, true)
            },
            Some(("m1".to_string(), base_parts)),
        );

        assert!(effect.accumulated);
        assert_eq!(effect.emit_msg_id.as_deref(), Some("m1"));
        assert!(effect.should_persist);
        assert_eq!(effect.persist_parts, expected_parts);
        assert!(effect.start_streaming_timer);
        assert!(effect.released_streaming_parts.is_empty());
        assert_eq!(emitted_attempts, 1);
        assert_eq!(proc.pending_stream_parts.len(), 0);
        assert!(proc.retry_stream_delta.is_some());
        assert!(!proc.streaming_parts.is_empty());
        assert!(
            proc.last_stream_emit_at.is_none(),
            "failed post-turn emit must remain retryable"
        );

        let mut retry_payloads: Vec<(String, Vec<MessagePart>)> = Vec::new();
        let tick_effect =
            run_streaming_timer_tick(&mut proc, "csid", |mid, _seq, parts, _snapshot_parts| {
                retry_payloads.push((mid.to_string(), parts.to_vec()));
                (true, true)
            });

        assert!(!tick_effect.keep_running);
        assert_eq!(tick_effect.released_streaming_parts, expected_parts.clone());
        assert_eq!(
            retry_payloads,
            vec![("m1".to_string(), expected_delta_parts)]
        );
        assert!(proc.streaming_parts.is_empty());
        assert_eq!(proc.pending_stream_parts.len(), 0);
        assert_eq!(proc.pending_stream_bytes, 0);
        let _ = proc.child.kill().await;
    }

    #[tokio::test]
    async fn post_turn_reseed_retry_persists_old_message_when_new_turn_started_after_base_load() {
        let mut proc = make_test_agent_process();
        proc.state = BridgeState::Ready;
        proc.turn_phase = TurnPhase::Idle;
        proc.last_message_id = Some("old-message".to_string());
        proc.streaming_delta_seq = 4;
        proc.streaming_delta_seq_by_message
            .insert("old-message".to_string(), 4);
        let msg = post_turn_tool_result_message("tool-1", "late");
        let mut emitted: Vec<(String, u64, Vec<MessagePart>)> = Vec::new();

        let first_effect = accumulate_stream_or_post_turn_message_locked(
            &mut proc,
            "csid",
            &msg,
            0,
            |_mid, _seq, _parts, _snapshot_parts| {
                panic!("first pass must request a store reseed before emitting")
            },
            None,
        );

        assert!(!first_effect.accumulated);
        assert_eq!(
            first_effect.post_turn_reseed_message_id.as_deref(),
            Some("old-message")
        );
        assert!(emitted.is_empty());

        proc.state = BridgeState::Streaming;
        proc.turn_phase = TurnPhase::Streaming;
        proc.streaming_message_id = Some("new-message".to_string());
        proc.reset_streaming_state_for_new_turn();
        proc.begin_turn_liveness();
        begin_turn_event_log(
            &mut proc,
            "new-human",
            test_prompt_input("new prompt"),
            "new-message",
            2.0,
        );
        let new_turn_parts = vec![MessagePart::Text {
            content: "new turn".to_string(),
            parent_tool_use_id: None,
        }];
        proc.streaming_parts = new_turn_parts.clone();
        proc.task_id_map
            .insert("new-task".to_string(), "new-tool".to_string());

        let stale_base_parts = vec![
            MessagePart::Text {
                content: "old base".to_string(),
                parent_tool_use_id: None,
            },
            MessagePart::ToolUse {
                tool: "Bash".to_string(),
                input: serde_json::json!({ "cmd": "date" }),
                id: "tool-1".to_string(),
                parent_tool_use_id: None,
            },
        ];
        let expected_delta_parts = vec![MessagePart::ToolResult {
            content: "late".to_string(),
            is_error: false,
            tool_use_id: Some("tool-1".to_string()),
            parent_tool_use_id: None,
        }];
        let expected_parts = consolidate_parts_from_slice(
            &stale_base_parts
                .iter()
                .cloned()
                .chain(expected_delta_parts.iter().cloned())
                .collect::<Vec<_>>(),
        );
        let second_effect = accumulate_stream_or_post_turn_message_locked(
            &mut proc,
            "csid",
            &msg,
            0,
            |mid, seq, parts, _snapshot_parts| {
                emitted.push((mid.to_string(), seq, parts.to_vec()));
                (true, true)
            },
            Some(("old-message".to_string(), stale_base_parts)),
        );

        assert!(second_effect.accumulated);
        assert_eq!(second_effect.emit_msg_id.as_deref(), Some("old-message"));
        assert!(second_effect.should_persist);
        assert_eq!(second_effect.persist_parts, expected_parts);
        assert!(!second_effect.start_streaming_timer);
        assert_eq!(
            emitted,
            vec![("old-message".to_string(), 5, expected_delta_parts)]
        );
        assert_eq!(proc.state, BridgeState::Streaming);
        assert_eq!(proc.streaming_message_id.as_deref(), Some("new-message"));
        assert!(proc.last_message_id.is_none());
        assert_eq!(
            proc.streaming_delta_seq, 0,
            "stale old-message delta must not advance the active new-message seq"
        );
        assert_eq!(
            proc.streaming_delta_seq_by_message.get("old-message"),
            Some(&5)
        );
        assert_eq!(proc.streaming_parts, new_turn_parts);
        assert_eq!(proc.pending_stream_parts.len(), 0);
        assert_eq!(
            proc.task_id_map.get("new-task").map(String::as_str),
            Some("new-tool")
        );
        assert!(
            proc.turn_event_log
                .project()
                .agent_parts_for_message("new-message")
                .iter()
                .all(|part| !matches!(
                    part,
                    MessagePart::ToolResult {
                        content,
                        tool_use_id: Some(id),
                        ..
                    } if id == "tool-1" && content == "late"
                )),
            "stale post-turn delta must not be appended to the active turn event log"
        );
        let _ = proc.child.kill().await;
    }

    #[tokio::test]
    async fn post_turn_loaded_base_preserves_projected_todo_text_without_reprojection_duplication()
    {
        let mut proc = make_test_agent_process();
        proc.state = BridgeState::Streaming;
        proc.turn_phase = TurnPhase::Streaming;
        proc.streaming_message_id = Some("new-message".to_string());
        proc.begin_turn_liveness();
        begin_turn_event_log(
            &mut proc,
            "new-human",
            test_prompt_input("new prompt"),
            "new-message",
            2.0,
        );

        let items = vec![crate::usecase::agent_session::session::TodoListItem {
            text: "ship fix".to_string(),
            completed: true,
        }];
        let todo_text = MessagePart::Text {
            content: todo_update_log(&items),
            parent_tool_use_id: None,
        };
        let todo_snapshot = MessagePart::TodoListSnapshot {
            items: items.clone(),
        };
        let tool_use = MessagePart::ToolUse {
            tool: "Bash".to_string(),
            input: serde_json::json!({ "cmd": "date" }),
            id: "tool-1".to_string(),
            parent_tool_use_id: None,
        };
        let base_parts = vec![todo_text.clone(), todo_snapshot.clone(), tool_use.clone()];
        let expected_parts = vec![
            todo_text.clone(),
            todo_snapshot,
            tool_use,
            MessagePart::ToolResult {
                content: "done".to_string(),
                is_error: false,
                tool_use_id: Some("tool-1".to_string()),
                parent_tool_use_id: None,
            },
        ];
        let expected_delta_parts = vec![MessagePart::ToolResult {
            content: "done".to_string(),
            is_error: false,
            tool_use_id: Some("tool-1".to_string()),
            parent_tool_use_id: None,
        }];
        let mut emitted = Vec::new();

        let effect = accumulate_stream_or_post_turn_message_locked(
            &mut proc,
            "csid",
            &post_turn_tool_result_message("tool-1", "done"),
            0,
            |mid, _seq, parts, _snapshot_parts| {
                emitted.push((mid.to_string(), parts.to_vec()));
                (true, true)
            },
            Some(("old-message".to_string(), base_parts)),
        );

        assert!(effect.accumulated);
        assert_eq!(effect.emit_msg_id.as_deref(), Some("old-message"));
        assert!(effect.should_persist);
        assert_eq!(effect.persist_parts, expected_parts);
        assert_eq!(
            effect
                .persist_parts
                .iter()
                .filter(|part| matches!(
                    part,
                    MessagePart::Text { content, .. } if content == &todo_update_log(&items)
                ))
                .count(),
            1
        );
        assert_eq!(
            emitted,
            vec![("old-message".to_string(), expected_delta_parts)]
        );
        assert!(
            proc.turn_event_log
                .project()
                .agent_parts_for_message("new-message")
                .is_empty(),
            "loaded old-message parts must not be reprojected into the active turn"
        );
        let _ = proc.child.kill().await;
    }

    #[tokio::test]
    async fn post_turn_reseed_retry_applies_status_to_old_message_from_loaded_task_map() {
        let mut proc = make_test_agent_process();
        proc.state = BridgeState::Streaming;
        proc.turn_phase = TurnPhase::Streaming;
        proc.streaming_message_id = Some("new-message".to_string());
        proc.begin_turn_liveness();
        begin_turn_event_log(
            &mut proc,
            "new-human",
            test_prompt_input("new prompt"),
            "new-message",
            2.0,
        );
        let new_turn_parts = vec![MessagePart::Text {
            content: "new turn".to_string(),
            parent_tool_use_id: None,
        }];
        proc.streaming_parts = new_turn_parts.clone();
        proc.task_id_map
            .insert("new-task".to_string(), "new-tool".to_string());

        let base_parts = vec![MessagePart::ToolResult {
            content: "Async agent launched successfully.\nagentId: old-task (internal ID)"
                .to_string(),
            is_error: false,
            tool_use_id: Some("old-tool".to_string()),
            parent_tool_use_id: None,
        }];
        let msg = serde_json::json!({
            "type": "system",
            "subtype": "task_updated",
            "task_id": "old-task",
            "patch": {"status": "completed", "summary": "done"}
        });

        let effect = accumulate_stream_or_post_turn_message_locked(
            &mut proc,
            "csid",
            &msg,
            0,
            |_mid, _seq, _parts, _snapshot_parts| (true, true),
            Some(("old-message".to_string(), base_parts.clone())),
        );

        assert!(effect.accumulated);
        assert_eq!(effect.emit_msg_id.as_deref(), Some("old-message"));
        assert!(effect.should_persist);
        assert!(matches!(
            effect.persist_parts.last(),
            Some(MessagePart::TaskStatus {
                task_tool_use_id,
                status,
                summary,
                ..
            }) if task_tool_use_id == "old-tool"
                && status == "completed"
                && summary.as_deref() == Some("done")
        ));
        assert_eq!(proc.streaming_message_id.as_deref(), Some("new-message"));
        assert_eq!(proc.streaming_parts, new_turn_parts);
        assert_eq!(
            proc.task_id_map.get("new-task").map(String::as_str),
            Some("new-tool")
        );
        assert!(!proc.task_id_map.contains_key("old-task"));
        assert!(
            proc.turn_event_log
                .project()
                .agent_parts_for_message("new-message")
                .iter()
                .all(|part| !matches!(
                    part,
                    MessagePart::TaskStatus {
                        task_tool_use_id,
                        ..
                    } if task_tool_use_id == "old-tool"
                )),
            "old task status must not be appended to the active turn event log"
        );
        let _ = proc.child.kill().await;
    }

    #[tokio::test]
    async fn post_turn_reseed_retry_persist_payload_restores_old_message_without_duplication() {
        let temp = tempfile::tempdir().unwrap();
        let app = tauri::test::mock_builder()
            .manage(crate::app_data_dir::TestDataDir(temp.path().to_path_buf()))
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
        let message_id = "old-message";
        let base_parts = vec![
            MessagePart::Text {
                content: "old base".to_string(),
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

        let mut proc = make_test_agent_process();
        proc.state = BridgeState::Streaming;
        proc.turn_phase = TurnPhase::Streaming;
        proc.streaming_message_id = Some("new-message".to_string());
        let new_turn_parts = vec![MessagePart::Text {
            content: "new turn".to_string(),
            parent_tool_use_id: None,
        }];
        proc.streaming_parts = new_turn_parts.clone();
        let msg = post_turn_tool_result_message("tool-1", "late");
        let mut emitted = Vec::new();

        let effect = accumulate_stream_or_post_turn_message_locked(
            &mut proc,
            "csid",
            &msg,
            0,
            |mid, _seq, parts, _snapshot_parts| {
                emitted.push((mid.to_string(), parts.to_vec()));
                (true, true)
            },
            Some((message_id.to_string(), base_parts.clone())),
        );

        assert!(effect.should_persist);
        let persisted = persist_streaming_parts(
            &store,
            &app.handle(),
            &session.id,
            message_id,
            &effect.persist_parts,
            0,
            None,
        );
        assert!(persisted);

        let expected_delta_parts = vec![MessagePart::ToolResult {
            content: "late".to_string(),
            is_error: false,
            tool_use_id: Some("tool-1".to_string()),
            parent_tool_use_id: None,
        }];
        let expected_parts = vec![
            base_parts[0].clone(),
            base_parts[1].clone(),
            expected_delta_parts[0].clone(),
        ];
        let loaded = store
            .load_full_session_for_restore(temp.path(), &session.id)
            .unwrap()
            .unwrap();
        let loaded_message = loaded
            .messages
            .iter()
            .find(|message| message.id == message_id)
            .expect("old agent message persisted");
        assert_eq!(
            loaded_message.parts.as_deref(),
            Some(expected_parts.as_slice())
        );
        assert_eq!(proc.streaming_message_id.as_deref(), Some("new-message"));
        assert_eq!(proc.streaming_parts, new_turn_parts);
        assert_eq!(
            emitted,
            vec![(message_id.to_string(), expected_delta_parts)]
        );
        let _ = proc.child.kill().await;
    }

    #[tokio::test]
    async fn post_turn_reseed_failure_skips_accumulate_and_persist_payload() {
        let mut proc = make_test_agent_process();
        proc.state = BridgeState::Ready;
        proc.turn_phase = TurnPhase::Idle;
        proc.last_message_id = Some("m1".to_string());
        let msg = post_turn_tool_result_message("tool-1", "should-not-write");
        let mut emitted = false;

        let effect = accumulate_stream_or_post_turn_message_locked(
            &mut proc,
            "csid",
            &msg,
            0,
            |_mid, _seq, _parts, _snapshot_parts| {
                emitted = true;
                (true, true)
            },
            None,
        );

        assert!(!effect.accumulated);
        assert_eq!(effect.post_turn_reseed_message_id.as_deref(), Some("m1"));
        assert!(effect.emit_msg_id.is_none());
        assert!(!effect.should_persist);
        assert!(effect.persist_parts.is_empty());
        assert!(!emitted);
        assert!(proc.streaming_parts.is_empty());
        assert_eq!(proc.pending_stream_parts.len(), 0);
        let _ = proc.child.kill().await;
    }

    #[test]
    fn consolidated_post_turn_base_matches_raw_retained_payload() {
        let raw_base = vec![
            MessagePart::Text {
                content: "hel".to_string(),
                parent_tool_use_id: None,
            },
            MessagePart::Text {
                content: "lo".to_string(),
                parent_tool_use_id: None,
            },
            MessagePart::ToolUse {
                tool: "Bash".to_string(),
                input: serde_json::json!({}),
                id: "tool-1".to_string(),
                parent_tool_use_id: None,
            },
            MessagePart::Thinking {
                content: "thin".to_string(),
                parent_tool_use_id: Some("tool-1".to_string()),
            },
            MessagePart::Thinking {
                content: "king".to_string(),
                parent_tool_use_id: Some("tool-1".to_string()),
            },
        ];
        let post_turn_delta = vec![
            MessagePart::Text {
                content: " done".to_string(),
                parent_tool_use_id: None,
            },
            MessagePart::ToolResult {
                content: "ok".to_string(),
                is_error: false,
                tool_use_id: Some("tool-1".to_string()),
                parent_tool_use_id: None,
            },
        ];

        let old_payload = consolidate_parts_from_slice(
            &raw_base
                .iter()
                .cloned()
                .chain(post_turn_delta.iter().cloned())
                .collect::<Vec<_>>(),
        );
        let new_payload = consolidate_parts_from_slice(
            &consolidate_parts_from_slice(&raw_base)
                .into_iter()
                .chain(post_turn_delta)
                .collect::<Vec<_>>(),
        );

        assert_eq!(new_payload, old_payload);
    }

    #[tokio::test]
    async fn bridge_error_persist_success_clears_untrusted_and_allows_post_turn_update() {
        let temp = tempfile::tempdir().unwrap();
        let app = tauri::test::mock_builder()
            .manage(crate::app_data_dir::TestDataDir(temp.path().to_path_buf()))
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
        let message_id = "m1";
        let empty_parts: Vec<MessagePart> = Vec::new();
        let (content, thinking, activities) = parts_to_legacy(&empty_parts);
        session.messages.push(ChatMessage {
            id: message_id.to_string(),
            role: MessageRole::Agent,
            content,
            thinking,
            activities,
            parts: Some(empty_parts),
            streaming_final_seq: 0,
            timestamp: 10.0,
            mentions: None,
        });
        store
            .save_full_session_for_migration_or_restore(temp.path(), &session)
            .unwrap();

        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let mut proc = make_streaming_test_process();
        proc.backend_id = CODEX_BACKEND_ID.to_string();
        let streaming_parts = vec![
            MessagePart::Text {
                content: "before error".to_string(),
                parent_tool_use_id: None,
            },
            MessagePart::ToolUse {
                tool: "Bash".to_string(),
                input: serde_json::json!({ "cmd": "date" }),
                id: "tool-1".to_string(),
                parent_tool_use_id: None,
            },
        ];
        proc.streaming_parts.extend(streaming_parts.clone());
        enqueue_pending_delta(&mut proc, &streaming_parts);
        handles.lock().await.insert(session.id.clone(), proc);
        let mut state = ExternalBridgeMessageState::default();

        handle_external_bridge_message(
            &app.handle(),
            &store,
            &handles,
            &session.id,
            serde_json::json!({
                "type": "error",
                "message": "bridge reported failure",
            }),
            &mut state,
        )
        .await;

        {
            let map = handles.lock().await;
            let proc = map.get(&session.id).unwrap();
            assert!(proc.post_turn_base_untrusted_message_id.is_none());
            assert_eq!(proc.last_message_id.as_deref(), Some(message_id));
        }

        handle_external_bridge_message(
            &app.handle(),
            &store,
            &handles,
            &session.id,
            post_turn_tool_result_message("tool-1", "post-turn result"),
            &mut state,
        )
        .await;

        let loaded = store
            .load_full_session_for_restore(temp.path(), &session.id)
            .unwrap()
            .unwrap();
        let loaded_message = loaded
            .messages
            .iter()
            .find(|message| message.id == message_id)
            .unwrap();
        assert!(loaded_message
            .parts
            .as_deref()
            .unwrap()
            .iter()
            .any(|part| matches!(
                part,
                MessagePart::ToolResult { content, .. } if content == "post-turn result"
            )));

        let removed_proc = handles.lock().await.remove(&session.id);
        if let Some(mut proc) = removed_proc {
            let _ = proc.child.kill().await;
        }
    }

    #[tokio::test]
    async fn bridge_error_persist_failure_keeps_untrusted() {
        let temp = tempfile::tempdir().unwrap();
        let app = tauri::test::mock_builder()
            .manage(crate::app_data_dir::TestDataDir(temp.path().to_path_buf()))
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
        let mut proc = make_streaming_test_process();
        proc.backend_id = CODEX_BACKEND_ID.to_string();
        begin_test_turn_event_log(&mut proc);
        let streaming_parts = vec![MessagePart::Text {
            content: "before error".to_string(),
            parent_tool_use_id: None,
        }];
        proc.streaming_parts.extend(streaming_parts.clone());
        enqueue_pending_delta(&mut proc, &streaming_parts);
        handles.lock().await.insert(session.id.clone(), proc);
        let mut state = ExternalBridgeMessageState::default();

        handle_external_bridge_message(
            &app.handle(),
            &store,
            &handles,
            &session.id,
            serde_json::json!({
                "type": "error",
                "message": "bridge reported failure",
            }),
            &mut state,
        )
        .await;

        {
            let map = handles.lock().await;
            let proc = map.get(&session.id).unwrap();
            assert_eq!(
                proc.post_turn_base_untrusted_message_id.as_deref(),
                Some("m1")
            );
        }

        let removed_proc = handles.lock().await.remove(&session.id);
        if let Some(mut proc) = removed_proc {
            let _ = proc.child.kill().await;
        }
    }

    #[tokio::test]
    async fn bridge_error_emits_pending_before_state_change() {
        // Spec (Rule: ターン完了・状態遷移時には未配信バッファを強制配信する,
        //  Examples ストリーミング → クラッシュ):
        //   Bridge から error メッセージを受信したクラッシュ経路では、
        //   未配信 delta + 合成 error part が同一 cumulative payload として
        //   state 通知 (Idle) より前にフロントエンドへ配信されること。
        let mut proc = make_streaming_test_process();
        begin_test_turn_event_log(&mut proc);
        // 未配信 text が pending に残っている状態でクラッシュが起こる。
        let pending_text = MessagePart::Text {
            content: "tail-before-crash".to_string(),
            parent_tool_use_id: None,
        };
        proc.streaming_parts.push(pending_text.clone());
        enqueue_pending_delta(&mut proc, std::slice::from_ref(&pending_text));

        let mut events = Vec::new();
        let transition =
            drive_bridge_error_path(&mut proc, "csid", "bridge reported failure", &mut events);

        assert_eq!(events.len(), 2, "flush emit then state emit");
        assert!(transition.turn_complete.turn_completed);
        assert!(transition
            .turn_complete
            .final_parts
            .iter()
            .any(|part| matches!(
                part,
                MessagePart::Error { content, .. } if content == "Error: bridge reported failure"
            )));
        match &events[0] {
            RecordedEmit::StreamingFlush {
                parts_count,
                tail_text,
            } => {
                // cumulative: pending Text + 合成 Error
                assert_eq!(*parts_count, 2);
                assert_eq!(
                    tail_text.as_deref(),
                    Some("Error: bridge reported failure"),
                    "tail must be the synthetic error part"
                );
            }
            other => panic!("first emit must be StreamingFlush, got {other:?}"),
        }
        assert_eq!(
            events[1],
            RecordedEmit::StateChanged {
                phase: TurnPhase::Idle,
                exit_code: Some(1),
            }
        );
        assert_eq!(proc.state, BridgeState::Crashed);
        assert_eq!(proc.turn_phase, TurnPhase::Idle);
        assert_eq!(proc.pending_stream_parts.len(), 0);
    }

    #[tokio::test]
    async fn accumulate_records_durable_events_but_keeps_text_delta_live_only() {
        let mut proc = make_streaming_test_process();
        proc.begin_turn_liveness();
        begin_turn_event_log(&mut proc, "human-1", test_prompt_input("prompt"), "m1", 1.0);

        let text_delta = serde_json::json!({
            "type": "stream_event",
            "event": {
                "type": "content_block_delta",
                "delta": {"type": "text_delta", "text": "hello"}
            }
        });
        let effect = accumulate_stream_or_post_turn_message_locked(
            &mut proc,
            "csid",
            &text_delta,
            0,
            |_mid, _seq, _parts, _snapshot_parts| (true, true),
            None,
        );

        assert!(effect.accumulated);
        assert!(
            proc.turn_event_log
                .project()
                .agent_parts_for_message("m1")
                .is_empty(),
            "text_delta must remain live-only until the terminal flush records the final block"
        );

        let tool_use = serde_json::json!({
            "type": "assistant",
            "message": {
                "content": [{
                    "type": "tool_use",
                    "name": "Read",
                    "input": {"file_path": "src/lib.rs"},
                    "id": "tool-1"
                }]
            }
        });
        let effect = accumulate_stream_or_post_turn_message_locked(
            &mut proc,
            "csid",
            &tool_use,
            0,
            |_mid, _seq, _parts, _snapshot_parts| (true, true),
            None,
        );

        assert!(effect.accumulated);
        let projected_parts = proc.turn_event_log.project().agent_parts_for_message("m1");
        assert!(projected_parts.iter().any(|part| matches!(
            part,
            MessagePart::ToolUse { id, tool, .. } if id == "tool-1" && tool == "Read"
        )));

        let retried_tool_use = serde_json::json!({
            "type": "assistant",
            "message": {
                "content": [{
                    "type": "tool_use",
                    "name": "Read",
                    "input": {"file_path": "src/main.rs"},
                    "id": "tool-1"
                }]
            }
        });
        let effect = accumulate_stream_or_post_turn_message_locked(
            &mut proc,
            "csid",
            &retried_tool_use,
            0,
            |_mid, _seq, _parts, _snapshot_parts| (true, true),
            None,
        );

        assert!(effect.accumulated);
        let projected = proc.turn_event_log.project();
        assert_eq!(projected.tool_retries.len(), 1);
        assert_eq!(projected.tool_retries[0].tool_use_id, "tool-1");
        assert_eq!(projected.tool_retries[0].attempt, 2);
    }

    #[tokio::test]
    async fn live_only_durable_record_skips_projection_and_durable_part_updates_phase() {
        let mut proc = make_streaming_test_process();
        proc.begin_turn_liveness();
        begin_turn_event_log(&mut proc, "human-1", test_prompt_input("prompt"), "m1", 1.0);
        proc.turn_phase = TurnPhase::WaitingPermission;

        record_durable_parts_for_current_turn(
            &mut proc,
            "m1",
            &[MessagePart::Text {
                content: "live".to_string(),
                parent_tool_use_id: None,
            }],
        );

        assert_eq!(proc.turn_phase, TurnPhase::WaitingPermission);
        assert!(proc
            .turn_event_log
            .project()
            .agent_parts_for_message("m1")
            .is_empty());

        record_durable_parts_for_current_turn(
            &mut proc,
            "m1",
            &[MessagePart::ToolUse {
                tool: "Read".to_string(),
                input: serde_json::json!({}),
                id: "tool-1".to_string(),
                parent_tool_use_id: None,
            }],
        );

        assert_eq!(proc.turn_phase, TurnPhase::Streaming);
        assert!(proc
            .turn_event_log
            .project()
            .agent_parts_for_message("m1")
            .iter()
            .any(|part| matches!(part, MessagePart::ToolUse { id, .. } if id == "tool-1")));
    }

    #[tokio::test]
    async fn turn_complete_appends_terminal_event_and_projects_workflow_input() {
        let mut proc = make_streaming_test_process();
        proc.begin_turn_liveness();
        begin_turn_event_log(&mut proc, "human-1", test_prompt_input("prompt"), "m1", 1.0);
        proc.last_result_token_usage = Some((11, 13));
        let final_text = MessagePart::Text {
            content: "final text".to_string(),
            parent_tool_use_id: None,
        };
        proc.streaming_parts.push(final_text.clone());
        enqueue_pending_delta(&mut proc, std::slice::from_ref(&final_text));

        let effect = run_turn_complete_transition_locked(
            &mut proc,
            "csid",
            0,
            |_mid, _seq, _parts, _snapshot_parts| (true, true),
        );

        assert!(effect.turn_completed);
        assert_eq!(effect.final_parts, vec![final_text]);
        assert_eq!(
            effect.workflow_turn_complete,
            Some(WorkflowTurnCompleteInput {
                turn_id: 1,
                exit_code: 0,
                final_text_parts: vec!["final text".to_string()],
                token_usage: Some(TurnTokenUsage {
                    input_tokens: 11,
                    output_tokens: 13,
                }),
                interrupted: false,
            })
        );
        assert_eq!(
            proc.turn_event_log.project().status.turn_phase,
            crate::usecase::agent_session::status::TurnPhase::Idle
        );
    }

    #[tokio::test]
    async fn nonzero_turn_complete_without_interrupt_projects_completed_workflow_input() {
        let mut proc = make_streaming_test_process();
        proc.begin_turn_liveness();
        begin_turn_event_log(&mut proc, "human-1", test_prompt_input("prompt"), "m1", 1.0);
        let final_text = MessagePart::Text {
            content: "failed but complete".to_string(),
            parent_tool_use_id: None,
        };
        proc.streaming_parts.push(final_text.clone());
        enqueue_pending_delta(&mut proc, std::slice::from_ref(&final_text));

        let effect = run_turn_complete_transition_locked(
            &mut proc,
            "csid",
            7,
            |_mid, _seq, _parts, _snapshot_parts| (true, true),
        );
        let projected = proc.turn_event_log.project();

        assert!(effect.turn_completed);
        assert_eq!(
            effect.workflow_turn_complete,
            Some(WorkflowTurnCompleteInput {
                turn_id: 1,
                exit_code: 7,
                final_text_parts: vec!["failed but complete".to_string()],
                token_usage: None,
                interrupted: false,
            })
        );
        assert_eq!(
            projected.status.session_state,
            crate::usecase::agent_session::session::SessionState::Error
        );
    }

    #[tokio::test]
    async fn turn_complete_does_not_record_terminal_text_already_projected_from_durable_event() {
        let mut proc = make_streaming_test_process();
        proc.begin_turn_liveness();
        begin_turn_event_log(&mut proc, "human-1", test_prompt_input("prompt"), "m1", 1.0);
        let items = vec![crate::usecase::agent_session::session::TodoListItem {
            text: "ship fix".to_string(),
            completed: true,
        }];
        let todo_text = MessagePart::Text {
            content: "TODO を更新しました（1/1 完了）".to_string(),
            parent_tool_use_id: None,
        };
        let todo_snapshot = MessagePart::TodoListSnapshot {
            items: items.clone(),
        };
        record_durable_parts_for_current_turn(
            &mut proc,
            "m1",
            std::slice::from_ref(&todo_snapshot),
        );
        proc.streaming_parts
            .extend([todo_text.clone(), todo_snapshot.clone()]);
        enqueue_pending_delta(&mut proc, &[todo_text, todo_snapshot]);

        let effect = run_turn_complete_transition_locked(
            &mut proc,
            "csid",
            0,
            |_mid, _seq, _parts, _snapshot_parts| (true, true),
        );

        assert_eq!(
            effect
                .final_parts
                .iter()
                .filter(|part| matches!(
                    part,
                    MessagePart::Text { content, .. }
                        if content == "TODO を更新しました（1/1 完了）"
                ))
                .count(),
            1
        );
        assert_eq!(
            proc.turn_event_log
                .project()
                .agent_parts_for_message("m1")
                .iter()
                .filter(|part| matches!(
                    part,
                    MessagePart::Text { content, .. }
                        if content == "TODO を更新しました（1/1 完了）"
                ))
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn turn_complete_preserves_non_adjacent_duplicate_part_occurrences() {
        let mut proc = make_streaming_test_process();
        proc.begin_turn_liveness();
        begin_turn_event_log(&mut proc, "human-1", test_prompt_input("prompt"), "m1", 1.0);
        let final_parts = vec![
            MessagePart::Text {
                content: "ok".to_string(),
                parent_tool_use_id: None,
            },
            MessagePart::ToolUse {
                tool: "Read".to_string(),
                input: serde_json::json!({"file_path": "a"}),
                id: "tool-1".to_string(),
                parent_tool_use_id: None,
            },
            MessagePart::Text {
                content: "ok".to_string(),
                parent_tool_use_id: None,
            },
        ];
        proc.streaming_parts = final_parts.clone();
        enqueue_pending_delta(&mut proc, &final_parts);

        let effect = run_turn_complete_transition_locked(
            &mut proc,
            "csid",
            0,
            |_mid, _seq, _parts, _snapshot_parts| (true, true),
        );
        let projected_parts = proc.turn_event_log.project().agent_parts_for_message("m1");

        assert_eq!(effect.final_parts, final_parts);
        assert_eq!(projected_parts, final_parts);
    }

    #[tokio::test]
    async fn turn_complete_does_not_append_preexisting_projected_occurrence_twice() {
        let mut proc = make_streaming_test_process();
        proc.begin_turn_liveness();
        begin_turn_event_log(&mut proc, "human-1", test_prompt_input("prompt"), "m1", 1.0);
        let tool_use = MessagePart::ToolUse {
            tool: "Read".to_string(),
            input: serde_json::json!({"file_path": "a"}),
            id: "tool-1".to_string(),
            parent_tool_use_id: None,
        };
        record_durable_parts_for_current_turn(&mut proc, "m1", std::slice::from_ref(&tool_use));
        proc.streaming_parts = vec![tool_use.clone()];
        enqueue_pending_delta(&mut proc, std::slice::from_ref(&tool_use));

        let effect = run_turn_complete_transition_locked(
            &mut proc,
            "csid",
            0,
            |_mid, _seq, _parts, _snapshot_parts| (true, true),
        );
        let projected_parts = proc.turn_event_log.project().agent_parts_for_message("m1");

        assert_eq!(effect.final_parts, vec![tool_use.clone()]);
        assert_eq!(projected_parts, vec![tool_use]);
    }

    #[tokio::test]
    async fn streaming_periodic_persist_uses_durable_projection_not_live_text_buffer() {
        let mut proc = make_streaming_test_process();
        proc.begin_turn_liveness();
        begin_turn_event_log(&mut proc, "human-1", test_prompt_input("prompt"), "m1", 1.0);
        proc.turn_event_log.reset_project_call_count();
        proc.turn_phase = TurnPhase::WaitingPermission;

        let text_delta = serde_json::json!({
            "type": "stream_event",
            "event": {
                "type": "content_block_delta",
                "delta": {"type": "text_delta", "text": "live only"}
            }
        });
        let effect = accumulate_stream_or_post_turn_message_locked(
            &mut proc,
            "csid",
            &text_delta,
            PERSIST_INTERVAL_MS,
            |_mid, _seq, _parts, _snapshot_parts| (true, true),
            None,
        );
        assert!(effect.accumulated);
        assert!(!effect.should_persist);
        assert!(effect.persist_parts.is_empty());
        assert_eq!(
            proc.turn_event_log.project_call_count(),
            0,
            "live-only periodic persist must not project the durable event log"
        );
        assert_eq!(
            proc.turn_phase,
            TurnPhase::WaitingPermission,
            "skipping live-only projection must preserve the current turn phase"
        );

        let tool_use = serde_json::json!({
            "type": "assistant",
            "message": {
                "content": [{
                    "type": "tool_use",
                    "name": "Read",
                    "input": {"file_path": "src/lib.rs"},
                    "id": "tool-1"
                }]
            }
        });
        let effect = accumulate_stream_or_post_turn_message_locked(
            &mut proc,
            "csid",
            &tool_use,
            PERSIST_INTERVAL_MS,
            |_mid, _seq, _parts, _snapshot_parts| (true, true),
            None,
        );

        assert!(effect.should_persist);
        assert!(
            proc.turn_event_log.project_call_count() > 0,
            "durable deltas still project for phase and persist parts"
        );
        assert_eq!(effect.persist_parts.len(), 1);
        assert!(matches!(
            effect.persist_parts.as_slice(),
            [MessagePart::ToolUse { id, tool, .. }] if id == "tool-1" && tool == "Read"
        ));
    }

    #[tokio::test]
    async fn bridge_error_finalizes_partial_tool_and_permission_events() {
        let mut proc = make_streaming_test_process();
        proc.begin_turn_liveness();
        begin_turn_event_log(&mut proc, "human-1", test_prompt_input("prompt"), "m1", 1.0);
        record_durable_parts_for_current_turn(
            &mut proc,
            "m1",
            &[
                MessagePart::ToolUse {
                    tool: "Edit".to_string(),
                    input: serde_json::json!({}),
                    id: "tool-1".to_string(),
                    parent_tool_use_id: None,
                },
                MessagePart::Permission {
                    request: serde_json::json!({
                        "request_id": "req-1",
                        "tool_use_id": "tool-1",
                        "tool_name": "Edit",
                    }),
                    status: "pending".to_string(),
                    answers: None,
                    parent_tool_use_id: None,
                },
            ],
        );

        let transition = run_bridge_error_transition_locked(
            &mut proc,
            "csid",
            &serde_json::json!({"type": "error", "message": "bridge failed"}),
            |_mid, _seq, _parts, _snapshot_parts| (true, true),
        );
        let projected = proc.turn_event_log.project();
        let projected_parts = projected.agent_parts_for_message("m1");

        assert!(transition.turn_complete.turn_completed);
        assert!(projected_parts.iter().any(|part| matches!(
            part,
            MessagePart::ToolResult {
                tool_use_id: Some(id),
                is_error: true,
                content,
                ..
            } if id == "tool-1" && content == "Error: bridge failed により中断"
        )));
        assert!(projected_parts.iter().any(|part| matches!(
            part,
            MessagePart::Permission { status, .. } if status == "cancelled"
        )));
        assert_eq!(
            projected.status.session_state,
            crate::usecase::agent_session::session::SessionState::Error
        );
        assert_eq!(
            projected.status.turn_phase,
            crate::usecase::agent_session::status::TurnPhase::Idle
        );
        assert_eq!(
            transition.turn_complete.workflow_turn_complete,
            Some(WorkflowTurnCompleteInput {
                turn_id: 1,
                exit_code: 1,
                final_text_parts: Vec::new(),
                token_usage: None,
                interrupted: true,
            })
        );
    }

    #[test]
    fn delta_has_tool_event_detects_tool_use_and_tool_result() {
        assert!(delta_has_tool_event(&[MessagePart::ToolUse {
            id: "1".to_string(),
            tool: "Bash".to_string(),
            input: serde_json::json!({}),
            parent_tool_use_id: None,
        }]));
        assert!(delta_has_tool_event(&[MessagePart::ToolResult {
            tool_use_id: Some("1".to_string()),
            content: "ok".to_string(),
            is_error: false,
            parent_tool_use_id: None,
        }]));
        assert!(!delta_has_tool_event(&[MessagePart::Text {
            content: "plain".to_string(),
            parent_tool_use_id: None,
        }]));
        assert!(!delta_has_tool_event(&[]));
    }

    #[tokio::test]
    async fn permission_request_message_is_parseable() {
        let json_str = r#"{"type":"permission_request","request_id":"abc-123","tool_name":"Edit","input":{},"tool_use_id":"toolu_001"}"#;
        let msg: serde_json::Value = serde_json::from_str(json_str).unwrap();
        assert_eq!(
            msg.get("type").and_then(|v| v.as_str()),
            Some("permission_request")
        );
        assert_eq!(
            msg.get("request_id").and_then(|v| v.as_str()),
            Some("abc-123")
        );
        assert_eq!(msg.get("tool_name").and_then(|v| v.as_str()), Some("Edit"));
    }

    #[test]
    fn turn_complete_message_parsing() {
        let msg_str = r#"{"type":"turn_complete","session_id":"sess-123","exit_code":0}"#;
        let msg: serde_json::Value = serde_json::from_str(msg_str).unwrap();
        assert_eq!(msg["type"], "turn_complete");
        assert_eq!(msg["exit_code"], 0);
        assert_eq!(msg["session_id"], "sess-123");
    }

    #[test]
    fn turn_complete_with_error() {
        let msg_str = r#"{"type":"turn_complete","session_id":"sess-123","exit_code":1}"#;
        let msg: serde_json::Value = serde_json::from_str(msg_str).unwrap();
        assert_eq!(msg["exit_code"], 1);
    }

    // --- accumulate_sdk_message tests ---

    #[test]
    fn post_turn_base_requirement_matches_accumulate_sdk_message() {
        struct Case {
            name: &'static str,
            msg: serde_json::Value,
            initial_parts: Vec<MessagePart>,
            task_id_map: HashMap<String, String>,
            expected: PostTurnBaseRequirement,
        }

        let mut mapped_task_ids = HashMap::new();
        mapped_task_ids.insert("task-1".to_string(), "tool-1".to_string());

        let compaction_in_progress = vec![MessagePart::SystemNotification {
            notification_type: SystemNotificationType::Compaction,
            status: "in_progress".to_string(),
            label: "Compacting conversation...".to_string(),
            detail: None,
            hook_id: None,
        }];

        let cases = vec![
            Case {
                name: "stream_event text_delta",
                msg: serde_json::json!({
                    "type": "stream_event",
                    "event": {
                        "type": "content_block_delta",
                        "delta": {"type": "text_delta", "text": "hello"}
                    }
                }),
                initial_parts: vec![],
                task_id_map: HashMap::new(),
                expected: PostTurnBaseRequirement::RequiresBase,
            },
            Case {
                name: "stream_event thinking_delta",
                msg: serde_json::json!({
                    "type": "stream_event",
                    "event": {
                        "type": "content_block_delta",
                        "delta": {"type": "thinking_delta", "thinking": "thinking"}
                    }
                }),
                initial_parts: vec![],
                task_id_map: HashMap::new(),
                expected: PostTurnBaseRequirement::RequiresBase,
            },
            Case {
                name: "stream_event unsupported delta",
                msg: serde_json::json!({
                    "type": "stream_event",
                    "event": {
                        "type": "content_block_delta",
                        "delta": {"type": "input_json_delta", "partial_json": "{}"}
                    }
                }),
                initial_parts: vec![],
                task_id_map: HashMap::new(),
                expected: PostTurnBaseRequirement::NotAccumulated,
            },
            Case {
                name: "assistant tool_use",
                msg: serde_json::json!({
                    "type": "assistant",
                    "message": {
                        "content": [{
                            "type": "tool_use",
                            "name": "Read",
                            "input": {"file_path": "/tmp/file"},
                            "id": "tool-1"
                        }]
                    }
                }),
                initial_parts: vec![],
                task_id_map: HashMap::new(),
                expected: PostTurnBaseRequirement::RequiresBase,
            },
            Case {
                name: "assistant TodoWrite snapshot",
                msg: serde_json::json!({
                    "type": "assistant",
                    "message": {
                        "content": [{
                            "type": "tool_use",
                            "name": "TodoWrite",
                            "input": {"todos": [{"content": "ship", "status": "pending"}]},
                            "id": "tool-1"
                        }]
                    }
                }),
                initial_parts: vec![],
                task_id_map: HashMap::new(),
                expected: PostTurnBaseRequirement::RequiresBase,
            },
            Case {
                name: "assistant TodoWrite without items",
                msg: serde_json::json!({
                    "type": "assistant",
                    "message": {
                        "content": [{
                            "type": "tool_use",
                            "name": "TodoWrite",
                            "input": {},
                            "id": "tool-1"
                        }]
                    }
                }),
                initial_parts: vec![],
                task_id_map: HashMap::new(),
                expected: PostTurnBaseRequirement::AccumulatedWithoutParts,
            },
            Case {
                name: "assistant without tool_use",
                msg: serde_json::json!({
                    "type": "assistant",
                    "message": {"content": [{"type": "text", "text": "ignored"}]}
                }),
                initial_parts: vec![],
                task_id_map: HashMap::new(),
                expected: PostTurnBaseRequirement::AccumulatedWithoutParts,
            },
            Case {
                name: "user tool_result",
                msg: post_turn_tool_result_message("tool-1", "done"),
                initial_parts: vec![],
                task_id_map: HashMap::new(),
                expected: PostTurnBaseRequirement::RequiresBase,
            },
            Case {
                name: "user without tool_result",
                msg: serde_json::json!({
                    "type": "user",
                    "message": {"content": [{"type": "text", "text": "ignored"}]}
                }),
                initial_parts: vec![],
                task_id_map: HashMap::new(),
                expected: PostTurnBaseRequirement::AccumulatedWithoutParts,
            },
            Case {
                name: "todo_list_snapshot with items",
                msg: serde_json::json!({
                    "type": "todo_list_snapshot",
                    "items": [{"text": "ship", "completed": false}]
                }),
                initial_parts: vec![],
                task_id_map: HashMap::new(),
                expected: PostTurnBaseRequirement::RequiresBase,
            },
            Case {
                name: "todo_list_snapshot without items",
                msg: serde_json::json!({"type": "todo_list_snapshot"}),
                initial_parts: vec![],
                task_id_map: HashMap::new(),
                expected: PostTurnBaseRequirement::AccumulatedWithoutParts,
            },
            Case {
                name: "permission_denied",
                msg: serde_json::json!({
                    "type": "permission_denied",
                    "tool_name": "Edit",
                    "tool_use_id": "tool-1",
                    "request_id": "req-1"
                }),
                initial_parts: vec![],
                task_id_map: HashMap::new(),
                expected: PostTurnBaseRequirement::RequiresBase,
            },
            Case {
                name: "permission_request",
                msg: serde_json::json!({
                    "type": "permission_request",
                    "request_id": "req-1",
                    "tool_name": "Edit"
                }),
                initial_parts: vec![],
                task_id_map: HashMap::new(),
                expected: PostTurnBaseRequirement::RequiresBase,
            },
            Case {
                name: "system task_started",
                msg: serde_json::json!({
                    "type": "system",
                    "subtype": "task_started",
                    "tool_use_id": "tool-1",
                    "description": "start"
                }),
                initial_parts: vec![],
                task_id_map: HashMap::new(),
                expected: PostTurnBaseRequirement::RequiresBase,
            },
            Case {
                name: "system task_notification",
                msg: serde_json::json!({
                    "type": "system",
                    "subtype": "task_notification",
                    "tool_use_id": "tool-1",
                    "status": "completed"
                }),
                initial_parts: vec![],
                task_id_map: HashMap::new(),
                expected: PostTurnBaseRequirement::RequiresBase,
            },
            Case {
                name: "system task_progress",
                msg: serde_json::json!({
                    "type": "system",
                    "subtype": "task_progress",
                    "tool_use_id": "tool-1",
                    "description": "progress"
                }),
                initial_parts: vec![],
                task_id_map: HashMap::new(),
                expected: PostTurnBaseRequirement::RequiresBase,
            },
            Case {
                name: "system task_updated mapped",
                msg: serde_json::json!({
                    "type": "system",
                    "subtype": "task_updated",
                    "task_id": "task-1",
                    "patch": {"status": "completed"}
                }),
                initial_parts: vec![],
                task_id_map: mapped_task_ids.clone(),
                expected: PostTurnBaseRequirement::RequiresBase,
            },
            Case {
                name: "system task_updated without mapping",
                msg: serde_json::json!({
                    "type": "system",
                    "subtype": "task_updated",
                    "task_id": "missing",
                    "patch": {"status": "completed"}
                }),
                initial_parts: vec![],
                task_id_map: HashMap::new(),
                expected: PostTurnBaseRequirement::AccumulatedWithoutParts,
            },
            Case {
                name: "system init",
                msg: serde_json::json!({
                    "type": "system",
                    "subtype": "init",
                    "session_id": "session-1"
                }),
                initial_parts: vec![],
                task_id_map: HashMap::new(),
                expected: PostTurnBaseRequirement::NotAccumulated,
            },
            Case {
                name: "system compact_boundary new part",
                msg: serde_json::json!({
                    "type": "system",
                    "subtype": "compact_boundary",
                    "compact_metadata": {"trigger": "manual", "pre_summary_token_count": 10}
                }),
                initial_parts: vec![],
                task_id_map: HashMap::new(),
                expected: PostTurnBaseRequirement::RequiresBase,
            },
            Case {
                name: "system compact_boundary update",
                msg: serde_json::json!({
                    "type": "system",
                    "subtype": "compact_boundary",
                    "compact_metadata": {"trigger": "auto", "pre_summary_token_count": 20}
                }),
                initial_parts: compaction_in_progress,
                task_id_map: HashMap::new(),
                expected: PostTurnBaseRequirement::RequiresBase,
            },
            Case {
                name: "system hook_started",
                msg: serde_json::json!({"type": "system", "subtype": "hook_started"}),
                initial_parts: vec![],
                task_id_map: HashMap::new(),
                expected: PostTurnBaseRequirement::AccumulatedWithoutParts,
            },
            Case {
                name: "system hook_progress",
                msg: serde_json::json!({"type": "system", "subtype": "hook_progress"}),
                initial_parts: vec![],
                task_id_map: HashMap::new(),
                expected: PostTurnBaseRequirement::AccumulatedWithoutParts,
            },
            Case {
                name: "system hook_response",
                msg: serde_json::json!({"type": "system", "subtype": "hook_response"}),
                initial_parts: vec![],
                task_id_map: HashMap::new(),
                expected: PostTurnBaseRequirement::AccumulatedWithoutParts,
            },
            Case {
                name: "system files_persisted",
                msg: serde_json::json!({"type": "system", "subtype": "files_persisted"}),
                initial_parts: vec![],
                task_id_map: HashMap::new(),
                expected: PostTurnBaseRequirement::AccumulatedWithoutParts,
            },
            Case {
                name: "system local_command_output",
                msg: serde_json::json!({"type": "system", "subtype": "local_command_output"}),
                initial_parts: vec![],
                task_id_map: HashMap::new(),
                expected: PostTurnBaseRequirement::AccumulatedWithoutParts,
            },
            Case {
                name: "system codex_realtime",
                msg: serde_json::json!({"type": "system", "subtype": "codex_realtime"}),
                initial_parts: vec![],
                task_id_map: HashMap::new(),
                expected: PostTurnBaseRequirement::AccumulatedWithoutParts,
            },
            Case {
                name: "system status compacting",
                msg: serde_json::json!({"type": "system", "status": "compacting"}),
                initial_parts: vec![],
                task_id_map: HashMap::new(),
                expected: PostTurnBaseRequirement::RequiresBase,
            },
            Case {
                name: "system unknown",
                msg: serde_json::json!({"type": "system", "subtype": "unknown"}),
                initial_parts: vec![],
                task_id_map: HashMap::new(),
                expected: PostTurnBaseRequirement::NotAccumulated,
            },
            Case {
                name: "error",
                msg: serde_json::json!({"type": "error", "message": "boom"}),
                initial_parts: vec![],
                task_id_map: HashMap::new(),
                expected: PostTurnBaseRequirement::NotAccumulated,
            },
            Case {
                name: "unknown type",
                msg: serde_json::json!({"type": "unknown"}),
                initial_parts: vec![],
                task_id_map: HashMap::new(),
                expected: PostTurnBaseRequirement::NotAccumulated,
            },
        ];

        for case in cases {
            let requirement =
                post_turn_base_requirement_for_empty_buffer(&case.msg, &case.task_id_map);
            assert_eq!(requirement, case.expected, "classifier: {}", case.name);

            let mut parts = case.initial_parts.clone();
            let before_parts = parts.clone();
            let mut task_id_map = case.task_id_map.clone();
            let (accumulated, _updated_parts) =
                accumulate_sdk_message(&case.msg, &mut parts, &mut task_id_map);
            let parts_changed = parts != before_parts;
            let expected_shape = match requirement {
                PostTurnBaseRequirement::RequiresBase => (true, true),
                PostTurnBaseRequirement::AccumulatedWithoutParts => (true, false),
                PostTurnBaseRequirement::NotAccumulated => (false, false),
            };
            assert_eq!(
                (accumulated, parts_changed),
                expected_shape,
                "accumulate shape: {}",
                case.name
            );
        }
    }
}
