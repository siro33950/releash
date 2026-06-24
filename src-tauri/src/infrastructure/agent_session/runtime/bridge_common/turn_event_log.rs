use super::process_registry::{AgentProcess, AgentProcessMap, TurnPhase};
use super::sdk_message::todo_update_log;
use super::shared::{consolidate_parts_from_slice, fallback_prompt_message_id};
use crate::usecase::agent_session::event_log::AgentSessionEvent;
use crate::usecase::agent_session::event_log::InterruptReason;
use crate::usecase::agent_session::event_log::PartEventMode;
use crate::usecase::agent_session::event_log::PermissionDecision;
use crate::usecase::agent_session::event_log::PromptInput;
use crate::usecase::agent_session::event_log::TurnEventLog;
use crate::usecase::agent_session::event_log::TurnTokenUsage;
use crate::usecase::agent_session::event_log::WorkflowTurnCompleteInput;
use crate::usecase::agent_session::session::now_timestamp;
use crate::usecase::agent_session::session::MessagePart;
use crate::usecase::agent_session::session::SessionState;
use std::sync::Arc;
use tokio::sync::Mutex;

pub(super) fn from_projected_turn_phase(
    phase: crate::usecase::agent_session::status::TurnPhase,
) -> TurnPhase {
    match phase {
        crate::usecase::agent_session::status::TurnPhase::Idle => TurnPhase::Idle,
        crate::usecase::agent_session::status::TurnPhase::Streaming => TurnPhase::Streaming,
        crate::usecase::agent_session::status::TurnPhase::WaitingPermission => {
            TurnPhase::WaitingPermission
        }
    }
}

pub(super) fn event_turn_id(proc: &AgentProcess) -> u64 {
    proc.turn_event_log
        .current_turn_id()
        .unwrap_or(proc.turn_seq)
}

pub(super) fn begin_turn_event_log(
    proc: &mut AgentProcess,
    prompt_message_id: &str,
    prompt: PromptInput,
    assistant_message_id: &str,
    at: f64,
) {
    proc.turn_event_log.begin_turn(
        proc.turn_seq,
        prompt_message_id.to_string(),
        assistant_message_id.to_string(),
        prompt,
        at,
    );
    proc.turn_phase = from_projected_turn_phase(proc.turn_event_log.project().status.turn_phase);
}

pub(super) fn projected_session_state_for_current_turn(
    proc: &AgentProcess,
) -> Option<SessionState> {
    proc.turn_event_log.current_turn_id()?;
    Some(proc.turn_event_log.project().status.session_state)
}

pub(super) fn ensure_turn_event_log_started(proc: &mut AgentProcess) {
    if proc.turn_event_log.current_turn_id().is_some() {
        return;
    }
    let Some(message_id) = proc
        .streaming_message_id
        .clone()
        .or_else(|| proc.last_message_id.clone())
    else {
        return;
    };
    let prompt_message_id = fallback_prompt_message_id(&message_id);
    begin_turn_event_log(
        proc,
        &prompt_message_id,
        PromptInput::default(),
        &message_id,
        now_timestamp(),
    );
}

pub(super) fn record_durable_parts_for_current_turn(
    proc: &mut AgentProcess,
    message_id: &str,
    parts: &[MessagePart],
) -> usize {
    ensure_turn_event_log_started(proc);
    let turn_id = event_turn_id(proc);
    let appended = proc.turn_event_log.append_part_events(
        turn_id,
        message_id,
        parts,
        PartEventMode::DurableOnly,
    );
    if appended == 0 {
        return 0;
    }
    let read_model = proc.turn_event_log.project();
    if parts
        .iter()
        .any(|part| matches!(part, MessagePart::ToolUse { .. }))
    {
        if let Some(retry) = read_model.tool_retries.last() {
            log::trace!(
                "agent session event log recorded tool retry: turn={} tool_use_id={} attempt={}",
                retry.turn_id,
                retry.tool_use_id,
                retry.attempt
            );
        }
    }
    proc.turn_phase = from_projected_turn_phase(read_model.status.turn_phase);
    appended
}

pub(super) fn record_permission_resolution_for_current_turn(
    proc: &mut AgentProcess,
    request_id: &str,
    behavior: &str,
    answers: Option<serde_json::Value>,
) {
    ensure_turn_event_log_started(proc);
    let decision = if behavior == "allow" {
        PermissionDecision::Allowed
    } else {
        PermissionDecision::Denied
    };
    let turn_id = event_turn_id(proc);
    proc.turn_event_log
        .append(AgentSessionEvent::PermissionResolved {
            turn_id,
            tool_use_id: None,
            request_id: Some(request_id.to_string()),
            decision,
            answers,
        });
    proc.turn_phase = from_projected_turn_phase(proc.turn_event_log.project().status.turn_phase);
}

pub(super) fn workflow_token_usage(token_usage: Option<(u64, u64)>) -> Option<TurnTokenUsage> {
    token_usage.map(|(input_tokens, output_tokens)| TurnTokenUsage {
        input_tokens,
        output_tokens,
    })
}

#[derive(Debug, Clone)]
pub(super) struct ProjectedTurnTerminal {
    pub(crate) final_parts: Vec<MessagePart>,
    pub(crate) workflow_turn_complete: Option<WorkflowTurnCompleteInput>,
    pub(crate) session_state: SessionState,
    pub(crate) turn_completed: bool,
}

pub(super) fn append_terminal_events_and_project(
    proc: &mut AgentProcess,
    message_id: &str,
    exit_code: i64,
    interrupt: Option<(InterruptReason, Option<String>)>,
    token_usage: Option<(u64, u64)>,
) -> Option<ProjectedTurnTerminal> {
    let turn_id = proc.turn_event_log.current_turn_id()?;
    let final_live_parts = consolidate_parts_from_slice(&proc.streaming_parts);
    append_terminal_part_events_in_order(proc, turn_id, message_id, &final_live_parts);
    if interrupt.is_none() && !final_live_parts.is_empty() {
        proc.turn_event_log
            .append(AgentSessionEvent::FinalPartsRecorded {
                turn_id,
                message_id: message_id.to_string(),
                parts: final_live_parts.clone(),
            });
    }

    match interrupt {
        Some((reason, error)) => proc
            .turn_event_log
            .finalize(turn_id, reason, error, exit_code),
        None => proc
            .turn_event_log
            .append(AgentSessionEvent::TurnCompleted {
                turn_id,
                exit_code,
                token_usage: workflow_token_usage(token_usage),
            }),
    }

    let read_model = proc.turn_event_log.project();
    proc.turn_phase = from_projected_turn_phase(read_model.status.turn_phase);
    let final_parts = read_model.agent_parts_for_message(message_id);
    let session_state = read_model.status.session_state;
    let workflow_turn_complete = read_model.workflow_turn_complete;
    let turn_completed = workflow_turn_complete
        .as_ref()
        .is_some_and(|input| input.turn_id == turn_id);
    Some(ProjectedTurnTerminal {
        final_parts,
        workflow_turn_complete,
        session_state,
        turn_completed,
    })
}

pub(super) fn append_terminal_part_events_in_order(
    proc: &mut AgentProcess,
    turn_id: u64,
    message_id: &str,
    final_parts: &[MessagePart],
) {
    let projected_parts = proc
        .turn_event_log
        .project()
        .agent_parts_for_message(message_id);
    let mut consumed_projected = vec![false; projected_parts.len()];
    for part in final_parts {
        if let Some(index) =
            projected_parts
                .iter()
                .enumerate()
                .find_map(|(index, projected_part)| {
                    (!consumed_projected[index] && projected_part == part).then_some(index)
                })
        {
            consumed_projected[index] = true;
            continue;
        }

        let mode = if terminal_part_has_live_final_event(part) {
            PartEventMode::FinalLiveBlocks
        } else {
            PartEventMode::DurableOnly
        };
        proc.turn_event_log.append_part_events(
            turn_id,
            message_id,
            std::slice::from_ref(part),
            mode,
        );
    }
}

pub(super) fn terminal_part_has_live_final_event(part: &MessagePart) -> bool {
    matches!(
        part,
        MessagePart::Text { .. } | MessagePart::Thinking { .. } | MessagePart::Error { .. }
    )
}

pub(super) fn mark_post_turn_store_base_untrusted(proc: &mut AgentProcess, message_id: &str) {
    proc.post_turn_base_untrusted_message_id = Some(message_id.to_string());
}

pub(super) fn append_parts_to_event_log_in_order(
    log: &mut TurnEventLog,
    turn_id: u64,
    message_id: &str,
    parts: &[MessagePart],
) {
    for (index, part) in parts.iter().enumerate() {
        if text_is_projected_by_following_todo_snapshot(part, parts.get(index + 1)) {
            continue;
        }
        let mode = if terminal_part_has_live_final_event(part) {
            PartEventMode::FinalLiveBlocks
        } else {
            PartEventMode::DurableOnly
        };
        log.append_part_events(turn_id, message_id, std::slice::from_ref(part), mode);
    }
}

pub(super) fn text_is_projected_by_following_todo_snapshot(
    part: &MessagePart,
    next: Option<&MessagePart>,
) -> bool {
    let MessagePart::Text {
        content,
        parent_tool_use_id: None,
    } = part
    else {
        return false;
    };
    let Some(MessagePart::TodoListSnapshot { items }) = next else {
        return false;
    };
    content == &todo_update_log(items)
}

pub(super) fn clear_post_turn_store_base_untrusted_after_persist_success(
    proc: &mut AgentProcess,
    message_id: &str,
) {
    if proc.post_turn_base_untrusted_message_id.as_deref() == Some(message_id) {
        proc.post_turn_base_untrusted_message_id = None;
    }
}

pub(super) async fn clear_post_turn_store_base_untrusted_for_message(
    handles: &Arc<Mutex<AgentProcessMap>>,
    chat_session_id: &str,
    message_id: &str,
) {
    let mut map = handles.lock().await;
    if let Some(proc) = map.get_mut(chat_session_id) {
        clear_post_turn_store_base_untrusted_after_persist_success(proc, message_id);
    }
}
#[cfg(test)]
mod moved_tests {

    use super::super::sdk_message::*;
    use super::super::session_lifecycle::*;

    use super::super::shared::test_support::*;

    use super::super::turn_event_log::*;

    use crate::usecase::agent_session::event_log::WorkflowTurnCompleteInput;

    use std::collections::HashMap;

    #[tokio::test]
    async fn explicit_interrupt_paths_project_interrupted_workflow_input() {
        for reason in [
            InterruptReason::Abort,
            InterruptReason::Timeout,
            InterruptReason::BridgeCrash,
        ] {
            let mut proc = make_streaming_test_process();
            proc.begin_turn_liveness();
            begin_turn_event_log(&mut proc, "human-1", test_prompt_input("prompt"), "m1", 1.0);

            let effect = run_turn_complete_transition_locked_with_interrupt(
                &mut proc,
                "csid",
                1,
                Some(reason),
                Some("stopped".to_string()),
                |_mid, _parts| (true, true),
            );

            assert_eq!(
                effect.workflow_turn_complete,
                Some(WorkflowTurnCompleteInput {
                    turn_id: 1,
                    exit_code: 1,
                    final_text_parts: Vec::new(),
                    token_usage: None,
                    interrupted: true,
                })
            );
        }
    }

    #[tokio::test]
    async fn representative_sdk_sequence_projects_same_parts_as_legacy_consolidation() {
        let sdk_messages = vec![
            serde_json::json!({
                "type": "stream_event",
                "event": {
                    "type": "content_block_delta",
                    "delta": {"type": "text_delta", "text": "hello "}
                }
            }),
            serde_json::json!({
                "type": "stream_event",
                "event": {
                    "type": "content_block_delta",
                    "delta": {"type": "thinking_delta", "thinking": "think"}
                }
            }),
            serde_json::json!({
                "type": "assistant",
                "message": {
                    "content": [{
                        "type": "tool_use",
                        "name": "Read",
                        "input": {"file_path": "src/lib.rs"},
                        "id": "tool-1"
                    }]
                }
            }),
            serde_json::json!({
                "type": "user",
                "message": {
                    "content": [{
                        "type": "tool_result",
                        "tool_use_id": "tool-1",
                        "content": "contents agentId: task-1",
                        "is_error": false
                    }]
                }
            }),
            serde_json::json!({
                "type": "user",
                "message": {
                    "content": [{
                        "type": "tool_result",
                        "content": "standalone",
                        "is_error": true
                    }]
                }
            }),
            serde_json::json!({
                "type": "permission_request",
                "request_id": "req-1",
                "tool_use_id": "tool-1",
                "tool_name": "Edit"
            }),
            serde_json::json!({
                "type": "permission_denied",
                "request_id": "req-1",
                "tool_use_id": "tool-1",
                "tool_name": "Edit",
                "message": "denied"
            }),
            serde_json::json!({
                "type": "todo_list_snapshot",
                "items": [{"text": "ship", "completed": true}]
            }),
            serde_json::json!({
                "type": "system",
                "status": "compacting"
            }),
            serde_json::json!({
                "type": "system",
                "subtype": "compact_boundary",
                "compact_metadata": {
                    "trigger": "manual",
                    "pre_summary_token_count": 123
                }
            }),
            serde_json::json!({
                "type": "stream_event",
                "event": {
                    "type": "content_block_delta",
                    "delta": {"type": "text_delta", "text": "done"}
                }
            }),
        ];

        let mut legacy_parts = Vec::new();
        let mut legacy_task_id_map = HashMap::new();
        for message in &sdk_messages {
            let _ = accumulate_sdk_message(message, &mut legacy_parts, &mut legacy_task_id_map);
        }
        let expected = consolidate_parts_from_slice(&legacy_parts);

        let mut proc = make_streaming_test_process();
        proc.begin_turn_liveness();
        begin_turn_event_log(&mut proc, "human-1", test_prompt_input("prompt"), "m1", 1.0);
        for message in &sdk_messages {
            let prev_len = proc.streaming_parts.len();
            let accumulation = accumulate_sdk_message_with_liveness(
                message,
                &mut proc.streaming_parts,
                &mut proc.task_id_map,
            );
            assert!(accumulation.handled);
            let mut delta = proc.streaming_parts[prev_len..].to_vec();
            if let Some(updated) = accumulation.updated_parts {
                delta.extend(updated);
            }
            record_durable_parts_for_current_turn(&mut proc, "m1", &delta);
        }
        let terminal =
            append_terminal_events_and_project(&mut proc, "m1", 0, None, None).expect("terminal");
        let projected = terminal.final_parts;

        assert_eq!(projected, expected);
    }
}
