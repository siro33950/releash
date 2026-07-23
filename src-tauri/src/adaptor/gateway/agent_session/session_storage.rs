use std::collections::HashMap;
#[cfg(test)]
use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::AtomicBool;

use parking_lot::RwLock;
use sha2::{Digest, Sha256};

use crate::domain::agent_session::events::SendDisposition;
use crate::usecase::agent_session::event_log::{
    latest_turn_interruption, latest_unresolved_permission_request, AgentSessionEvent,
    BackendSessionRecoveryProjection, TurnEventLog,
};
use crate::usecase::agent_session::session::{
    AgentSessionProjectionCodec, CanonicalAgentSessionProjection, CanonicalQueuedSend, ChatMessage,
    ChatSession, MessagePart, PageCursor, SessionAttachment, SessionEventLogRecoverySignal,
    SessionMeta, SessionPage, SessionQueuePauseReader, SessionReviewContext,
    SessionReviewContextReader, SessionState, SessionToolOutput, TokenUsage,
};

mod attachment_blob;
mod event_store;
mod fork_copier;
#[cfg(test)]
mod gc;
mod layout;
mod message_store;
mod meta_repository;
mod private_context;
#[cfg(test)]
mod projection_commit;
mod session_projection_v1;
mod stored_event_v1;
mod stored_message_part_v1;
mod stored_session_v1;
mod titles;
mod tool_output_blob;
#[cfg(test)]
mod transaction;

#[cfg(test)]
mod tests;

pub(crate) use session_projection_v1::{
    decode_agent_content_blob_record_v1, decode_agent_message_projection_record_v1,
    decode_agent_session_projection_record_v1, encode_agent_content_blob_record_v1,
    encode_agent_message_projection_record_v1, encode_agent_session_projection_record_v1,
    AgentSessionProjectionCodecV1,
};
#[cfg(test)]
pub(crate) use stored_session_v1::StoredChatMessageV1;

#[cfg(test)]
pub(crate) fn decode_legacy_chat_message_for_gc(
    raw: &[u8],
    source_id: String,
) -> Result<ChatMessage, String> {
    stored_session_v1::decode_chat_message_v1(
        raw,
        stored_message_part_v1::StoredPayloadSource {
            source_id,
            record_ordinal: None,
        },
    )
    .map(|record| record.message)
    .map_err(|error| error.to_string())
}

/// Known semantic projections decoded during the one-shot Legacy -> SQLite
/// import.  Unknown/additive files stay in `legacy_raw_records`; a file that
/// claims one of these known shapes but cannot be decoded fails migration
/// closed instead of silently cutting over with raw bytes only.
pub(crate) enum LegacySessionProjectionV1 {
    Session {
        session_id: String,
        projection: String,
        messages: Vec<(String, String)>,
    },
    Message {
        session_id: String,
        message_id: String,
        projection: String,
    },
}

pub(crate) type LegacyAgentEventSourceIdentityV1 = (String, (u8, u64));
pub(crate) type LegacyMessageSourceIdentityV1 = String;

pub(crate) fn legacy_agent_event_source_identity_v1(
    relative_path: &str,
) -> Result<Option<LegacyAgentEventSourceIdentityV1>, String> {
    let components = relative_path.split('/').collect::<Vec<_>>();
    if components.len() < 3
        || components[0] != "sessions"
        || !layout::UUID_RE.is_match(components[1])
    {
        return Ok(None);
    }
    let order = match components.as_slice() {
        ["sessions", _, "events.json"] => (0, 0),
        ["sessions", _, "event_batches", file] if file.ends_with(".json") => {
            let ordinal = file
                .trim_end_matches(".json")
                .parse::<u64>()
                .map_err(|_| "known legacy event batch has an invalid ordinal".to_string())?;
            (1, ordinal)
        }
        ["sessions", _, "events_tail.json"] => (2, 0),
        _ => return Ok(None),
    };
    Ok(Some((components[1].to_string(), order)))
}

pub(crate) fn legacy_message_source_identity_v1(
    relative_path: &str,
) -> Result<Option<LegacyMessageSourceIdentityV1>, String> {
    let components = relative_path.split('/').collect::<Vec<_>>();
    let ["sessions", session_id, "messages", file_name] = components.as_slice() else {
        return Ok(None);
    };
    if !layout::UUID_RE.is_match(session_id) || !file_name.ends_with(".json") {
        return Ok(None);
    }
    let raw_ordinal = file_name.trim_end_matches(".json");
    let ordinal = raw_ordinal
        .parse::<u64>()
        .ok()
        .filter(|ordinal| *ordinal > 0)
        .filter(|ordinal| raw_ordinal == ordinal.to_string())
        .ok_or_else(|| "known legacy message has an invalid ordinal".to_string())?;
    let _ = ordinal;
    Ok(Some((*session_id).to_string()))
}

pub(crate) fn decode_streaming_legacy_message_projection_v1<R: std::io::Read>(
    relative_path: &str,
    reader: R,
) -> Result<Option<LegacySessionProjectionV1>, String> {
    let Some(session_id) = legacy_message_source_identity_v1(relative_path)? else {
        return Ok(None);
    };
    let message = stored_session_v1::decode_streaming_chat_message_v1(reader)
        .map_err(|error| format!("known legacy message is incompatible: {error}"))?;
    let projection = stored_session_v1::encode_chat_message_v1(&message)
        .map_err(|_| "known legacy message could not be canonicalized".to_string())?;
    if projection.is_empty() || projection.len() > 16 * 1024 * 1024 {
        return Err("known legacy message semantic record exceeds 16 MiB".to_string());
    }
    let projection = String::from_utf8(projection)
        .map_err(|_| "known legacy message canonical form is not UTF-8".to_string())?;
    Ok(Some(LegacySessionProjectionV1::Message {
        session_id,
        message_id: message.id,
        projection,
    }))
}

pub(crate) fn decode_legacy_agent_event_record_v1(
    raw: &[u8],
    source_id: &str,
    record_ordinal: u64,
) -> Result<Vec<AgentSessionEvent>, String> {
    event_store::decode_legacy_event_record_v1(raw, source_id, record_ordinal)
}

/// Fully derived SQLite participants for one legacy session.  These values
/// are deterministic functions of the fixed legacy inventory, so replaying a
/// migration checkpoint cannot duplicate a terminal or pending obligation.
#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LegacySessionSemanticProjectionV1 {
    pub projection: String,
    pub terminals: Vec<LegacyTurnTerminalV1>,
    pub stop_resolutions: Vec<LegacyStopResolutionV1>,
    pub pending_obligations: Vec<LegacyAgentObligationV1>,
    pub pending_queue_count: usize,
    pub pending_permission_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LegacyTurnTerminalV1 {
    pub session_id: String,
    pub turn_id: String,
    pub terminal_identity: String,
    pub result: String,
    pub participant_digest: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LegacyStopResolutionV1 {
    pub stop_operation_id: String,
    pub resolution: &'static str,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LegacyAgentObligationV1 {
    pub obligation_id: String,
    pub record: String,
    pub ordered_key: String,
    pub owner: String,
    pub partition: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LegacySessionSemanticParticipantV1 {
    Terminal(LegacyTurnTerminalV1),
    StopResolution(LegacyStopResolutionV1),
    PendingObligation(LegacyAgentObligationV1),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LegacySessionSemanticFinalV1 {
    pub projection: String,
    pub pending_obligations: Vec<LegacyAgentObligationV1>,
    pub pending_queue_count: usize,
    pub pending_permission_count: usize,
}

pub(crate) fn decode_legacy_session_titles_v1(
    relative_path: &str,
    raw: &[u8],
) -> Result<Option<HashMap<String, String>>, String> {
    if relative_path != "session_titles.json" {
        return Ok(None);
    }
    let titles = serde_json::from_slice::<HashMap<String, String>>(raw)
        .map_err(|_| "known legacy session title catalog is incompatible".to_string())?;
    Ok(Some(titles))
}

pub(crate) fn decode_legacy_session_projection_v1(
    relative_path: &str,
    raw: &[u8],
) -> Result<Option<LegacySessionProjectionV1>, String> {
    let components = relative_path.split('/').collect::<Vec<_>>();
    if components.len() == 2
        && components[0] == "sessions"
        && components[1].ends_with(".json")
        && !components[1].ends_with(".meta.json")
    {
        let expected_id = components[1].trim_end_matches(".json");
        if !layout::UUID_RE.is_match(expected_id) {
            return Ok(None);
        }
        let session = stored_session_v1::decode_chat_session_v1(raw)
            .map_err(|error| format!("known legacy session is incompatible: {error}"))?;
        if session.id != expected_id {
            return Err("known legacy session identity does not match its path".to_string());
        }
        let mut meta = SessionMeta::from_session(&session);
        meta.message_count = session.messages.len();
        let projection = encode_sqlite_session_projection(meta)?;
        let messages = session
            .messages
            .iter()
            .map(|message| {
                let projection = stored_session_v1::encode_chat_message_v1(message)
                    .map_err(|_| "known legacy message could not be canonicalized".to_string())?;
                Ok((
                    message.id.clone(),
                    String::from_utf8(projection).map_err(|_| {
                        "known legacy message canonical form is not UTF-8".to_string()
                    })?,
                ))
            })
            .collect::<Result<Vec<_>, String>>()?;
        return Ok(Some(LegacySessionProjectionV1::Session {
            session_id: session.id,
            projection,
            messages,
        }));
    }
    if components.len() == 3
        && components[0] == "sessions"
        && components[2] == "meta.json"
        && layout::UUID_RE.is_match(components[1])
    {
        let meta: SessionMeta = serde_json::from_slice(raw)
            .map_err(|_| "known legacy session meta is incompatible".to_string())?;
        let meta = layout::validate_meta(meta, components[1])?;
        let projection = encode_sqlite_session_projection(meta.clone())?;
        return Ok(Some(LegacySessionProjectionV1::Session {
            session_id: meta.id,
            projection,
            messages: Vec::new(),
        }));
    }
    if components.len() == 4
        && components[0] == "sessions"
        && components[2] == "messages"
        && components[3].ends_with(".json")
        && layout::UUID_RE.is_match(components[1])
    {
        let decoded = stored_session_v1::decode_chat_message_v1(
            raw,
            stored_message_part_v1::StoredPayloadSource {
                source_id: relative_path.to_string(),
                record_ordinal: None,
            },
        )
        .map_err(|error| format!("known legacy message is incompatible: {error}"))?;
        let projection = String::from_utf8(
            stored_session_v1::encode_chat_message_v1(&decoded.message)
                .map_err(|_| "known legacy message could not be canonicalized".to_string())?,
        )
        .map_err(|_| "known legacy message canonical form is not UTF-8".to_string())?;
        return Ok(Some(LegacySessionProjectionV1::Message {
            session_id: components[1].to_string(),
            message_id: decoded.message.id,
            projection,
        }));
    }
    Ok(None)
}

pub(crate) fn merge_legacy_private_context_projection_v1(
    relative_path: &str,
    raw: &[u8],
    projection: &str,
) -> Result<Option<(String, String)>, String> {
    let components = relative_path.split('/').collect::<Vec<_>>();
    let ["sessions", session_id, "private_context.json"] = components.as_slice() else {
        return Ok(None);
    };
    if !layout::UUID_RE.is_match(session_id) {
        return Ok(None);
    }
    let payload = crate::domain::local_event::SessionProjectionRecord::AgentSession(Box::new(
        decode_agent_session_projection_record_v1(projection)?,
    ));
    let mut canonical = AgentSessionProjectionCodecV1.decode(&payload)?;
    private_context::hydrate_meta_private_context_bytes(raw, &mut canonical.meta)?;
    let crate::domain::local_event::SessionProjectionRecord::AgentSession(encoded) =
        AgentSessionProjectionCodecV1.encode(&canonical)?
    else {
        return Err("legacy session projection codec returned the wrong kind".to_string());
    };
    let encoded = encode_agent_session_projection_record_v1(&encoded)?;
    Ok(Some(((*session_id).to_string(), encoded)))
}

fn validate_legacy_obligation_record_bound(raw: &str) -> Result<(), String> {
    if raw.is_empty() || raw.len() > 16 * 1024 * 1024 {
        return Err("known legacy obligation exceeds its bounded record".to_string());
    }
    Ok(())
}

fn encode_sqlite_session_projection(meta: SessionMeta) -> Result<String, String> {
    let payload = AgentSessionProjectionCodecV1.encode(&CanonicalAgentSessionProjection {
        meta,
        title: None,
        messages: Vec::new(),
        reducer_events: Vec::new(),
        queue_paused_at: None,
        latest_token_usage: None,
        pending_send_queue: Vec::new(),
    })?;
    let crate::domain::local_event::SessionProjectionRecord::AgentSession(payload) = payload else {
        return Err("agent projection codec returned the wrong kind".to_string());
    };
    encode_agent_session_projection_record_v1(&payload)
}

#[cfg(test)]
fn bounded_legacy_reducer_events(events: &[AgentSessionEvent]) -> Vec<AgentSessionEvent> {
    let Some(turn_start) = events
        .iter()
        .rposition(|event| matches!(event, AgentSessionEvent::TurnStarted { .. }))
    else {
        return events.to_vec();
    };

    // Match the canonical runtime projection policy: retain the current turn
    // plus the latest session-wide latches.  Migration must not reintroduce a
    // full-history projection that makes every future append history-sized.
    let mut retained = Vec::new();
    if let Some(event) = events[..turn_start].iter().rev().find(|event| {
        matches!(
            event,
            AgentSessionEvent::QueuePaused { .. } | AgentSessionEvent::QueueResumed { .. }
        )
    }) {
        retained.push(event.clone());
    }
    if let Some(recovery_start) = events[..turn_start].iter().rposition(|event| {
        matches!(
            event,
            AgentSessionEvent::BackendSessionRecoveryStarted { .. }
        )
    }) {
        retained.extend(
            events[recovery_start..turn_start]
                .iter()
                .filter(|event| {
                    matches!(
                        event,
                        AgentSessionEvent::BackendSessionRecoveryStarted { .. }
                            | AgentSessionEvent::SessionConfigurationReactivated { .. }
                            | AgentSessionEvent::SessionGoalReactivated { .. }
                            | AgentSessionEvent::BackendSessionRecoveryCompleted { .. }
                            | AgentSessionEvent::BackendSessionRecoveryFailed { .. }
                    )
                })
                .cloned(),
        );
    }
    if let Some(event) = events[..turn_start]
        .iter()
        .rev()
        .find(|event| matches!(event, AgentSessionEvent::SessionClosed { .. }))
    {
        retained.push(event.clone());
    }
    retained.extend_from_slice(&events[turn_start..]);
    retained
}

#[cfg(test)]
fn legacy_pending_queue(events: &[AgentSessionEvent]) -> Result<Vec<CanonicalQueuedSend>, String> {
    let mut pending = Vec::<CanonicalQueuedSend>::new();
    for event in events {
        match event {
            AgentSessionEvent::SendOperationAccepted {
                operation_id,
                disposition: SendDisposition::Queued { queue_item_id },
                human_message_id,
                reserved_turn_id,
                ..
            } => {
                let human_message_id = human_message_id.clone().ok_or_else(|| {
                    "known legacy queued send lacks its human message identity".to_string()
                })?;
                let reserved_turn_id = reserved_turn_id.clone().ok_or_else(|| {
                    "known legacy queued send lacks its reserved turn identity".to_string()
                })?;
                if let Some(existing) = pending
                    .iter()
                    .find(|entry| entry.queue_item_id == *queue_item_id)
                {
                    if existing.human_message_id != human_message_id
                        || existing.reserved_turn_id != reserved_turn_id
                    {
                        return Err("known legacy queued send identity has conflicting payloads"
                            .to_string());
                    }
                    continue;
                }
                pending.push(CanonicalQueuedSend {
                    queue_item_id: queue_item_id.clone(),
                    human_message_id,
                    reserved_turn_id,
                    input_ref: format!("legacy-send:{operation_id}"),
                });
            }
            AgentSessionEvent::TurnStarted {
                turn_id,
                message_id,
                ..
            } => {
                let turn_id = turn_id.to_string();
                pending.retain(|entry| {
                    entry.reserved_turn_id != turn_id && entry.human_message_id != *message_id
                });
            }
            _ => {}
        }
    }
    Ok(pending)
}

#[cfg(test)]
fn legacy_terminal_participants(
    session_id: &str,
    events: &[AgentSessionEvent],
    completed_at: f64,
) -> Result<Vec<LegacyTurnTerminalV1>, String> {
    let mut message_ids = HashMap::<u64, String>::new();
    let mut terminals = Vec::<LegacyTurnTerminalV1>::new();
    let mut terminal_by_turn = HashMap::<u64, String>::new();
    for event in events {
        if let AgentSessionEvent::TurnStarted {
            turn_id,
            message_id,
            assistant_message_id,
            ..
        } = event
        {
            message_ids.insert(
                *turn_id,
                assistant_message_id
                    .clone()
                    .unwrap_or_else(|| format!("{message_id}:agent")),
            );
            continue;
        }
        let (turn_id, terminal_kind, exit_code, stop_reason, token_usage) = match event {
            AgentSessionEvent::TurnCompleted {
                turn_id,
                exit_code,
                stop_reason,
                token_usage,
            } => (
                *turn_id,
                "completed",
                *exit_code,
                stop_reason.map(|reason| format!("{reason:?}")),
                token_usage.map(|usage| {
                    serde_json::json!({
                        "input_tokens": usage.input_tokens.to_string(),
                        "output_tokens": usage.output_tokens.to_string(),
                    })
                }),
            ),
            AgentSessionEvent::TurnInterrupted {
                turn_id,
                reason,
                exit_code,
                error,
            } => (*turn_id, reason.label(), *exit_code, error.clone(), None),
            _ => continue,
        };
        let encoded =
            stored_event_v1::encode_agent_session_events_v1(std::slice::from_ref(event), false)
                .map_err(|error| {
                    format!("known legacy terminal cannot be canonicalized: {error}")
                })?;
        let mut digest = Sha256::new();
        digest.update(b"legacy-agent-terminal/v1\0");
        digest.update(session_id.as_bytes());
        digest.update(turn_id.to_be_bytes());
        digest.update(&encoded);
        let participant_digest: [u8; 32] = digest.finalize().into();
        let terminal_identity = format!("legacy-terminal-{}", hex::encode(participant_digest));
        if let Some(existing) = terminal_by_turn.insert(turn_id, terminal_identity.clone()) {
            if existing == terminal_identity {
                continue;
            }
            return Err("known legacy turn has conflicting terminal results".to_string());
        }
        let message_id = message_ids
            .get(&turn_id)
            .cloned()
            .unwrap_or_else(|| format!("legacy-turn-{turn_id}:agent"));
        let result = serde_json::json!({
            "schema": "agent_turn_terminal_v1",
            "terminal_kind": terminal_kind,
            "session_id": session_id,
            "turn_id": turn_id.to_string(),
            "message_id": message_id,
            "streaming_final_seq": "0",
            "completed_at_bits": completed_at.to_bits().to_string(),
            "legacy_result": {
                "exit_code": exit_code,
                "reason": stop_reason,
                "token_usage": token_usage,
            },
        })
        .to_string();
        terminals.push(LegacyTurnTerminalV1 {
            session_id: session_id.to_string(),
            turn_id: turn_id.to_string(),
            terminal_identity,
            result,
            participant_digest,
        });
    }
    Ok(terminals)
}

fn legacy_event_observation(event: &AgentSessionEvent) -> Result<serde_json::Value, String> {
    let encoded =
        stored_event_v1::encode_agent_session_events_v1(std::slice::from_ref(event), false)
            .map_err(|error| format!("known legacy operation cannot be canonicalized: {error}"))?;
    serde_json::from_slice(&encoded)
        .map_err(|_| "known legacy operation observation is incompatible".to_string())
}

#[cfg(test)]
fn legacy_stop_resolutions(
    session_id: &str,
    events: &[AgentSessionEvent],
) -> Result<Vec<LegacyStopResolutionV1>, String> {
    let mut resolutions = Vec::<LegacyStopResolutionV1>::new();
    for event in events {
        let AgentSessionEvent::StopResolutionRecorded {
            operation_id,
            turn_id,
            resolution,
            ..
        } = event
        else {
            continue;
        };
        if operation_id.is_empty() {
            return Err("known legacy Stop resolution has no operation identity".to_string());
        }
        let resolution = match resolution {
            crate::domain::agent_session::events::StopResolution::Succeeded => "succeeded",
            crate::domain::agent_session::events::StopResolution::Superseded => "superseded",
        };
        let detail = serde_json::json!({
            "schema": "legacy_stop_resolution_v1",
            "operation_id": operation_id,
            "session_id": session_id,
            "turn_id": turn_id.to_string(),
            "resolution": resolution,
            "known_observation": legacy_event_observation(event)?,
        })
        .to_string();
        if let Some(existing) = resolutions
            .iter()
            .find(|candidate| candidate.stop_operation_id == *operation_id)
        {
            if existing.resolution == resolution && existing.detail == detail {
                continue;
            }
            return Err("known legacy Stop operation has conflicting resolutions".to_string());
        }
        resolutions.push(LegacyStopResolutionV1 {
            stop_operation_id: operation_id.clone(),
            resolution,
            detail,
        });
    }
    Ok(resolutions)
}

fn legacy_obligation_identity(session_id: &str, kind: &str, identity: &str) -> String {
    let digest = Sha256::digest(
        format!("legacy-agent-obligation/v1\0{session_id}\0{kind}\0{identity}").as_bytes(),
    );
    format!("legacy-{kind}-{}", hex::encode(digest))
}

fn legacy_partition(state: &SessionState) -> &'static str {
    match state {
        SessionState::Closed => "closed_session",
        SessionState::Archived => "archived_session",
        _ => "owner",
    }
}

fn push_legacy_obligation(
    obligations: &mut Vec<LegacyAgentObligationV1>,
    session_id: &str,
    state: &SessionState,
    kind: &str,
    identity: &str,
    record: serde_json::Value,
) {
    let obligation_id = legacy_obligation_identity(session_id, kind, identity);
    obligations.push(LegacyAgentObligationV1 {
        ordered_key: format!("legacy:{session_id}:{kind}:{obligation_id}"),
        obligation_id,
        record: record.to_string(),
        owner: session_id.to_string(),
        partition: legacy_partition(state),
    });
}

fn canonical_legacy_permission_request(
    request: &crate::usecase::agent_session::session::PermissionRequestMsg,
) -> Result<serde_json::Value, String> {
    let mut value = serde_json::to_value(request)
        .map_err(|_| "known legacy permission cannot be canonicalized".to_string())?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| "known legacy permission is not an object".to_string())?;
    for (legacy, canonical) in [
        ("toolUseId", "tool_use_id"),
        ("toolName", "tool_name"),
        ("allowedPrompts", "allowed_prompts"),
        ("displayName", "display_name"),
        ("decisionReason", "decision_reason"),
    ] {
        if let Some(value) = object.remove(legacy) {
            object.insert(canonical.to_string(), value);
        }
    }
    if let Some(questions) = object
        .get_mut("questions")
        .and_then(serde_json::Value::as_array_mut)
    {
        for question in questions {
            let Some(question) = question.as_object_mut() else {
                return Err("known legacy permission question is not an object".to_string());
            };
            if let Some(value) = question.remove("multiSelect") {
                question.insert("multi_select".to_string(), value);
            }
        }
    }
    Ok(value)
}

const LEGACY_SEMANTIC_RETAINED_EVENT_LIMIT: usize = 4096;

fn legacy_terminal_participant(
    session_id: &str,
    event: &AgentSessionEvent,
    assistant_message_ids: &HashMap<u64, String>,
    completed_at: f64,
) -> Result<Option<LegacyTurnTerminalV1>, String> {
    let (turn_id, terminal_kind, exit_code, stop_reason, token_usage) = match event {
        AgentSessionEvent::TurnCompleted {
            turn_id,
            exit_code,
            stop_reason,
            token_usage,
        } => (
            *turn_id,
            "completed",
            *exit_code,
            stop_reason.map(|reason| format!("{reason:?}")),
            token_usage.map(|usage| {
                serde_json::json!({
                    "input_tokens": usage.input_tokens.to_string(),
                    "output_tokens": usage.output_tokens.to_string(),
                })
            }),
        ),
        AgentSessionEvent::TurnInterrupted {
            turn_id,
            reason,
            exit_code,
            error,
        } => (*turn_id, reason.label(), *exit_code, error.clone(), None),
        _ => return Ok(None),
    };
    let encoded =
        stored_event_v1::encode_agent_session_events_v1(std::slice::from_ref(event), false)
            .map_err(|error| format!("known legacy terminal cannot be canonicalized: {error}"))?;
    let mut digest = Sha256::new();
    digest.update(b"legacy-agent-terminal/v1\0");
    digest.update(session_id.as_bytes());
    digest.update(turn_id.to_be_bytes());
    digest.update(&encoded);
    let participant_digest: [u8; 32] = digest.finalize().into();
    let terminal_identity = format!("legacy-terminal-{}", hex::encode(participant_digest));
    let message_id = assistant_message_ids
        .get(&turn_id)
        .cloned()
        .unwrap_or_else(|| format!("legacy-turn-{turn_id}:agent"));
    let result = serde_json::json!({
        "schema": "agent_turn_terminal_v1",
        "terminal_kind": terminal_kind,
        "session_id": session_id,
        "turn_id": turn_id.to_string(),
        "message_id": message_id,
        "streaming_final_seq": "0",
        "completed_at_bits": completed_at.to_bits().to_string(),
        "legacy_result": {
            "exit_code": exit_code,
            "reason": stop_reason,
            "token_usage": token_usage,
        },
    })
    .to_string();
    Ok(Some(LegacyTurnTerminalV1 {
        session_id: session_id.to_string(),
        turn_id: turn_id.to_string(),
        terminal_identity,
        result,
        participant_digest,
    }))
}

fn legacy_stop_resolution_participant(
    session_id: &str,
    event: &AgentSessionEvent,
) -> Result<Option<LegacyStopResolutionV1>, String> {
    let AgentSessionEvent::StopResolutionRecorded {
        operation_id,
        turn_id,
        resolution,
        ..
    } = event
    else {
        return Ok(None);
    };
    if operation_id.is_empty() {
        return Err("known legacy Stop resolution has no operation identity".to_string());
    }
    let resolution = match resolution {
        crate::domain::agent_session::events::StopResolution::Succeeded => "succeeded",
        crate::domain::agent_session::events::StopResolution::Superseded => "superseded",
    };
    let detail = serde_json::json!({
        "schema": "legacy_stop_resolution_v1",
        "operation_id": operation_id,
        "session_id": session_id,
        "turn_id": turn_id.to_string(),
        "resolution": resolution,
        "known_observation": legacy_event_observation(event)?,
    })
    .to_string();
    Ok(Some(LegacyStopResolutionV1 {
        stop_operation_id: operation_id.clone(),
        resolution,
        detail,
    }))
}

fn legacy_operation_participant(
    session_id: &str,
    source_state: &SessionState,
    event: &AgentSessionEvent,
) -> Result<Option<LegacyAgentObligationV1>, String> {
    let (operation_kind, operation_id) = match event {
        AgentSessionEvent::SendOperationAccepted { operation_id, .. } => ("send", operation_id),
        AgentSessionEvent::StopOperationAccepted { operation_id, .. } => ("stop", operation_id),
        AgentSessionEvent::SessionLifecycleOperationAccepted { operation_id, .. } => {
            ("session_lifecycle", operation_id)
        }
        _ => return Ok(None),
    };
    if operation_id.is_empty() {
        return Err("known legacy acceptance has no operation identity".to_string());
    }
    let mut obligation = Vec::with_capacity(1);
    push_legacy_obligation(
        &mut obligation,
        session_id,
        source_state,
        "operation",
        &format!("{operation_kind}:{operation_id}"),
        serde_json::json!({
            "schema": "legacy_agent_reconciliation_obligation_v1",
            "state": "reconciliation_required",
            "kind": "operation_binding",
            "operation_kind": operation_kind,
            "operation_id": operation_id,
            "session_id": session_id,
            "known_observation": legacy_event_observation(event)?,
            "missing_evidence": ["principal", "exact_caller_binding", "immutable_receipt"],
            "safe_actions": ["read_again", "keep_for_manual_resolution"],
        }),
    );
    Ok(obligation.pop())
}

/// Incremental semantic fold used by the production migration. It retains at
/// most the current reducer window plus bounded latches; immutable terminal,
/// Stop, and accepted-operation participants are emitted one event at a time.
pub(crate) struct LegacySessionSemanticAccumulatorV1 {
    session_id: String,
    canonical: CanonicalAgentSessionProjection,
    source_state: SessionState,
    observed_turn: bool,
    latest_turn: Option<u64>,
    latest_turn_is_terminal: bool,
    latest_token_usage: Option<TokenUsage>,
    reducer_events: Vec<AgentSessionEvent>,
    latest_queue_latch: Option<AgentSessionEvent>,
    latest_recovery_sequence: Vec<AgentSessionEvent>,
    latest_session_closed: Option<AgentSessionEvent>,
    pending_send_queue: Vec<CanonicalQueuedSend>,
    assistant_message_ids: HashMap<u64, String>,
}

impl LegacySessionSemanticAccumulatorV1 {
    pub(crate) fn new(
        session_id: &str,
        base_projection: &str,
        title: Option<&str>,
    ) -> Result<Self, String> {
        let payload = crate::domain::local_event::SessionProjectionRecord::AgentSession(Box::new(
            decode_agent_session_projection_record_v1(base_projection)?,
        ));
        let mut canonical = AgentSessionProjectionCodecV1.decode(&payload)?;
        if canonical.meta.id != session_id {
            return Err("known legacy semantic projection identity changed".to_string());
        }
        canonical.title = title.map(str::to_string).or(canonical.title);
        let source_state = canonical.meta.state.clone();
        Ok(Self {
            session_id: session_id.to_string(),
            canonical,
            source_state,
            observed_turn: false,
            latest_turn: None,
            latest_turn_is_terminal: false,
            latest_token_usage: None,
            reducer_events: Vec::new(),
            latest_queue_latch: None,
            latest_recovery_sequence: Vec::new(),
            latest_session_closed: None,
            pending_send_queue: Vec::new(),
            assistant_message_ids: HashMap::new(),
        })
    }

    fn retain_for_new_turn(&mut self) {
        self.reducer_events.clear();
        if let Some(event) = &self.latest_queue_latch {
            self.reducer_events.push(event.clone());
        }
        self.reducer_events
            .extend(self.latest_recovery_sequence.iter().cloned());
        if let Some(event) = &self.latest_session_closed {
            self.reducer_events.push(event.clone());
        }
    }

    fn update_pending_queue(&mut self, event: &AgentSessionEvent) -> Result<(), String> {
        match event {
            AgentSessionEvent::SendOperationAccepted {
                operation_id,
                disposition: SendDisposition::Queued { queue_item_id },
                human_message_id,
                reserved_turn_id,
                ..
            } => {
                let human_message_id = human_message_id.clone().ok_or_else(|| {
                    "known legacy queued send lacks its human message identity".to_string()
                })?;
                let reserved_turn_id = reserved_turn_id.clone().ok_or_else(|| {
                    "known legacy queued send lacks its reserved turn identity".to_string()
                })?;
                if let Some(existing) = self
                    .pending_send_queue
                    .iter()
                    .find(|entry| entry.queue_item_id == *queue_item_id)
                {
                    if existing.human_message_id != human_message_id
                        || existing.reserved_turn_id != reserved_turn_id
                    {
                        return Err("known legacy queued send identity has conflicting payloads"
                            .to_string());
                    }
                    return Ok(());
                }
                self.pending_send_queue.push(CanonicalQueuedSend {
                    queue_item_id: queue_item_id.clone(),
                    human_message_id,
                    reserved_turn_id,
                    input_ref: format!("legacy-send:{operation_id}"),
                });
            }
            AgentSessionEvent::TurnStarted {
                turn_id,
                message_id,
                ..
            } => {
                let turn_id = turn_id.to_string();
                self.pending_send_queue.retain(|entry| {
                    entry.reserved_turn_id != turn_id && entry.human_message_id != *message_id
                });
            }
            _ => {}
        }
        if self.pending_send_queue.len() > LEGACY_SEMANTIC_RETAINED_EVENT_LIMIT {
            return Err("known legacy pending queue exceeds its bounded projection".to_string());
        }
        Ok(())
    }

    fn update_latches(&mut self, event: &AgentSessionEvent) -> Result<(), String> {
        if matches!(
            event,
            AgentSessionEvent::QueuePaused { .. } | AgentSessionEvent::QueueResumed { .. }
        ) {
            self.latest_queue_latch = Some(event.clone());
        }
        if matches!(
            event,
            AgentSessionEvent::BackendSessionRecoveryStarted { .. }
        ) {
            self.latest_recovery_sequence.clear();
        }
        if (!self.latest_recovery_sequence.is_empty()
            || matches!(
                event,
                AgentSessionEvent::BackendSessionRecoveryStarted { .. }
            ))
            && matches!(
                event,
                AgentSessionEvent::BackendSessionRecoveryStarted { .. }
                    | AgentSessionEvent::SessionConfigurationReactivated { .. }
                    | AgentSessionEvent::SessionGoalReactivated { .. }
                    | AgentSessionEvent::BackendSessionRecoveryCompleted { .. }
                    | AgentSessionEvent::BackendSessionRecoveryFailed { .. }
            )
        {
            self.latest_recovery_sequence.push(event.clone());
        }
        if self.latest_recovery_sequence.len() > LEGACY_SEMANTIC_RETAINED_EVENT_LIMIT {
            return Err("known legacy backend recovery exceeds its bounded projection".to_string());
        }
        if matches!(event, AgentSessionEvent::SessionClosed { .. }) {
            self.latest_session_closed = Some(event.clone());
        }
        Ok(())
    }

    pub(crate) fn push(
        &mut self,
        event: AgentSessionEvent,
    ) -> Result<Option<LegacySessionSemanticParticipantV1>, String> {
        self.update_pending_queue(&event)?;
        if let AgentSessionEvent::TurnStarted {
            turn_id,
            message_id,
            assistant_message_id,
            ..
        } = &event
        {
            self.retain_for_new_turn();
            self.observed_turn = true;
            self.latest_turn = Some(*turn_id);
            self.latest_turn_is_terminal = false;
            self.assistant_message_ids.insert(
                *turn_id,
                assistant_message_id
                    .clone()
                    .unwrap_or_else(|| format!("{message_id}:agent")),
            );
            if self.assistant_message_ids.len() > LEGACY_SEMANTIC_RETAINED_EVENT_LIMIT {
                return Err(
                    "known legacy turn identity set exceeds its bounded projection".to_string(),
                );
            }
        }
        self.reducer_events.push(event.clone());
        if self.reducer_events.len() > LEGACY_SEMANTIC_RETAINED_EVENT_LIMIT {
            return Err("known legacy reducer window exceeds its bounded projection".to_string());
        }

        if let AgentSessionEvent::TurnCompleted {
            turn_id,
            token_usage,
            ..
        } = &event
        {
            if self.latest_turn == Some(*turn_id) {
                self.latest_turn_is_terminal = true;
            }
            if let Some(usage) = token_usage {
                self.latest_token_usage = Some(TokenUsage {
                    input_tokens: usage.input_tokens,
                    output_tokens: usage.output_tokens,
                    total_tokens: usage.input_tokens.checked_add(usage.output_tokens),
                    context_window_tokens: None,
                });
            }
        } else if let AgentSessionEvent::TurnInterrupted { turn_id, .. } = &event {
            if self.latest_turn == Some(*turn_id) {
                self.latest_turn_is_terminal = true;
            }
        }

        let terminal = legacy_terminal_participant(
            &self.session_id,
            &event,
            &self.assistant_message_ids,
            self.canonical.meta.updated_at,
        )?;
        let participant = if let Some(terminal) = terminal {
            let terminal_turn_id = match &event {
                AgentSessionEvent::TurnCompleted { turn_id, .. }
                | AgentSessionEvent::TurnInterrupted { turn_id, .. } => Some(*turn_id),
                _ => None,
            };
            if let Some(turn_id) = terminal_turn_id {
                self.assistant_message_ids.remove(&turn_id);
            }
            Some(LegacySessionSemanticParticipantV1::Terminal(terminal))
        } else if let Some(resolution) =
            legacy_stop_resolution_participant(&self.session_id, &event)?
        {
            Some(LegacySessionSemanticParticipantV1::StopResolution(
                resolution,
            ))
        } else {
            legacy_operation_participant(&self.session_id, &self.source_state, &event)?
                .map(LegacySessionSemanticParticipantV1::PendingObligation)
        };
        self.update_latches(&event)?;
        Ok(participant)
    }

    pub(crate) fn finish(mut self) -> Result<LegacySessionSemanticFinalV1, String> {
        const ACTIVE_UPGRADE_FAILURE: &str =
            "This session was active during upgrade and requires reconciliation before it can continue.";
        let projected = TurnEventLog::from_events(self.reducer_events.clone()).project();
        let unresolved_permission = latest_unresolved_permission_request(&self.reducer_events);
        let unresolved_active_turn = self.latest_turn.filter(|_| !self.latest_turn_is_terminal);
        let was_conservatively_paused =
            self.canonical.meta.error_reason.as_deref() == Some(ACTIVE_UPGRADE_FAILURE);
        let ambiguous_active_session = (self.source_state == SessionState::Active
            || was_conservatively_paused)
            && !self.observed_turn;

        self.canonical.reducer_events = self.reducer_events;
        self.canonical.pending_send_queue = self.pending_send_queue;
        self.canonical.queue_paused_at = projected.queue_paused_at;
        if (unresolved_active_turn.is_some()
            || ambiguous_active_session
            || !self.canonical.pending_send_queue.is_empty()
            || unresolved_permission.is_some())
            && self.canonical.queue_paused_at.is_none()
        {
            self.canonical.queue_paused_at = Some(self.canonical.meta.updated_at);
        }
        self.canonical.latest_token_usage = self
            .latest_token_usage
            .or(self.canonical.latest_token_usage);
        self.canonical.meta.last_turn_id = self.latest_turn.or(self.canonical.meta.last_turn_id);
        self.canonical.meta.last_turn_interruption =
            latest_turn_interruption(&self.canonical.reducer_events)
                .or(self.canonical.meta.last_turn_interruption);
        if !matches!(
            self.source_state,
            SessionState::Closed | SessionState::Archived
        ) {
            if unresolved_active_turn.is_some() || ambiguous_active_session {
                self.canonical.meta.state = SessionState::Error;
                self.canonical
                    .meta
                    .error_reason
                    .get_or_insert_with(|| ACTIVE_UPGRADE_FAILURE.to_string());
            } else if self.observed_turn {
                self.canonical.meta.state = projected.status.session_state.clone();
                self.canonical.meta.error_reason = projected.error_reason.clone();
            }
        }

        let mut pending_obligations = Vec::new();
        if let Some(turn_id) = unresolved_active_turn {
            push_legacy_obligation(
                &mut pending_obligations,
                &self.session_id,
                &self.source_state,
                "turn",
                &turn_id.to_string(),
                serde_json::json!({
                    "schema": "legacy_agent_reconciliation_obligation_v1",
                    "state": "reconciliation_required",
                    "kind": "turn_execution",
                    "session_id": self.session_id,
                    "turn_id": turn_id.to_string(),
                    "safe_actions": ["read_again", "keep_for_manual_resolution"],
                }),
            );
        } else if ambiguous_active_session {
            push_legacy_obligation(
                &mut pending_obligations,
                &self.session_id,
                &self.source_state,
                "session",
                &self.session_id,
                serde_json::json!({
                    "schema": "legacy_agent_reconciliation_obligation_v1",
                    "state": "reconciliation_required",
                    "kind": "provider_session",
                    "session_id": self.session_id,
                    "safe_actions": ["read_again", "keep_for_manual_resolution"],
                }),
            );
        }
        if let Some(permission) = &unresolved_permission {
            let request = canonical_legacy_permission_request(&permission.request)?;
            let identity = if permission.request.id.is_empty() {
                format!("turn-{}", permission.turn_id)
            } else {
                permission.request.id.clone()
            };
            push_legacy_obligation(
                &mut pending_obligations,
                &self.session_id,
                &self.source_state,
                "permission",
                &identity,
                serde_json::json!({
                    "schema": "legacy_agent_reconciliation_obligation_v1",
                    "state": "reconciliation_required",
                    "kind": "permission",
                    "session_id": self.session_id,
                    "turn_id": permission.turn_id.to_string(),
                    "request": request,
                    "safe_actions": ["read_again", "keep_for_manual_resolution"],
                }),
            );
        }
        for queued in &self.canonical.pending_send_queue {
            push_legacy_obligation(
                &mut pending_obligations,
                &self.session_id,
                &self.source_state,
                "queue",
                &queued.queue_item_id,
                serde_json::json!({
                    "schema": "legacy_agent_reconciliation_obligation_v1",
                    "state": "reconciliation_required",
                    "kind": "queued_send",
                    "session_id": self.session_id,
                    "queue_item_id": queued.queue_item_id,
                    "human_message_id": queued.human_message_id,
                    "reserved_turn_id": queued.reserved_turn_id,
                    "input_ref": queued.input_ref,
                    "safe_actions": ["read_again", "cancel_if_safe", "keep_for_manual_resolution"],
                }),
            );
        }
        match &projected.backend_recovery {
            Some(BackendSessionRecoveryProjection::Recovering { recovery_id, .. })
            | Some(BackendSessionRecoveryProjection::ReconciliationRequired {
                recovery_id, ..
            }) => push_legacy_obligation(
                &mut pending_obligations,
                &self.session_id,
                &self.source_state,
                "recovery",
                recovery_id,
                serde_json::json!({
                    "schema": "legacy_agent_reconciliation_obligation_v1",
                    "state": "reconciliation_required",
                    "kind": "backend_recovery",
                    "session_id": self.session_id,
                    "recovery_id": recovery_id,
                    "safe_actions": ["read_again", "keep_for_manual_resolution"],
                }),
            ),
            None => {}
        }
        if let Some(pending) = &self.canonical.meta.pending_recovery_message {
            push_legacy_obligation(
                &mut pending_obligations,
                &self.session_id,
                &self.source_state,
                "publication",
                &self.session_id,
                serde_json::json!({
                    "schema": "legacy_agent_reconciliation_obligation_v1",
                    "state": "reconciliation_required",
                    "kind": "recovery_publication",
                    "session_id": self.session_id,
                    "pending_message": pending,
                    "safe_actions": ["read_again", "keep_for_manual_resolution"],
                }),
            );
        }
        for obligation in &pending_obligations {
            validate_legacy_obligation_record_bound(&obligation.record)?;
        }
        let pending_queue_count = self.canonical.pending_send_queue.len();
        let pending_permission_count = usize::from(unresolved_permission.is_some());
        let crate::domain::local_event::SessionProjectionRecord::AgentSession(projection) =
            AgentSessionProjectionCodecV1.encode(&self.canonical)?
        else {
            return Err("legacy session projection codec returned the wrong kind".to_string());
        };
        let projection = encode_agent_session_projection_record_v1(&projection)?;
        Ok(LegacySessionSemanticFinalV1 {
            projection,
            pending_obligations,
            pending_queue_count,
            pending_permission_count,
        })
    }
}

/// Fold the complete, stable legacy event sequence into the bounded SQLite
/// projection and the direct terminal/recovery indexes.  Ambiguous active
/// work is deliberately represented as reconciliation-required and never as
/// a provider effect that startup may blindly resume.
#[cfg(test)]
pub(crate) fn materialize_legacy_session_semantics_v1(
    session_id: &str,
    base_projection: &str,
    title: Option<&str>,
    events: &[AgentSessionEvent],
) -> Result<LegacySessionSemanticProjectionV1, String> {
    const ACTIVE_UPGRADE_FAILURE: &str =
        "This session was active during upgrade and requires reconciliation before it can continue.";
    let payload = crate::domain::local_event::SessionProjectionRecord::AgentSession(Box::new(
        decode_agent_session_projection_record_v1(base_projection)?,
    ));
    let mut canonical = AgentSessionProjectionCodecV1.decode(&payload)?;
    if canonical.meta.id != session_id {
        return Err("known legacy semantic projection identity changed".to_string());
    }
    let source_state = canonical.meta.state.clone();
    let reducer_events = bounded_legacy_reducer_events(events);
    let projected = TurnEventLog::from_events(reducer_events.clone()).project();
    let pending_send_queue = legacy_pending_queue(events)?;
    let latest_turn = events.iter().rev().find_map(|event| match event {
        AgentSessionEvent::TurnStarted { turn_id, .. } => Some(*turn_id),
        _ => None,
    });
    let latest_turn_is_terminal = latest_turn.is_some_and(|turn_id| {
        events.iter().any(|event| {
            matches!(
                event,
                AgentSessionEvent::TurnCompleted { turn_id: candidate, .. }
                    | AgentSessionEvent::TurnInterrupted { turn_id: candidate, .. }
                    if *candidate == turn_id
            )
        })
    });
    let unresolved_permission = latest_unresolved_permission_request(&reducer_events);
    let unresolved_active_turn = latest_turn.filter(|_| !latest_turn_is_terminal);
    let was_conservatively_paused =
        canonical.meta.error_reason.as_deref() == Some(ACTIVE_UPGRADE_FAILURE);
    let ambiguous_active_session = (source_state == SessionState::Active
        || was_conservatively_paused)
        && latest_turn.is_none();

    canonical.title = title.map(str::to_string).or(canonical.title);
    canonical.reducer_events = reducer_events;
    canonical.pending_send_queue = pending_send_queue;
    canonical.queue_paused_at = projected.queue_paused_at;
    if (unresolved_active_turn.is_some()
        || ambiguous_active_session
        || !canonical.pending_send_queue.is_empty()
        || unresolved_permission.is_some())
        && canonical.queue_paused_at.is_none()
    {
        canonical.queue_paused_at = Some(canonical.meta.updated_at);
    }
    canonical.latest_token_usage = events
        .iter()
        .rev()
        .find_map(|event| match event {
            AgentSessionEvent::TurnCompleted {
                token_usage: Some(usage),
                ..
            } => Some(TokenUsage {
                input_tokens: usage.input_tokens,
                output_tokens: usage.output_tokens,
                total_tokens: usage.input_tokens.checked_add(usage.output_tokens),
                context_window_tokens: None,
            }),
            _ => None,
        })
        .or(canonical.latest_token_usage);
    canonical.meta.last_turn_id = latest_turn.or(canonical.meta.last_turn_id);
    canonical.meta.last_turn_interruption = latest_turn_interruption(&canonical.reducer_events)
        .or(canonical.meta.last_turn_interruption);
    if !matches!(source_state, SessionState::Closed | SessionState::Archived) {
        if unresolved_active_turn.is_some() || ambiguous_active_session {
            canonical.meta.state = SessionState::Error;
            canonical
                .meta
                .error_reason
                .get_or_insert_with(|| ACTIVE_UPGRADE_FAILURE.to_string());
        } else if latest_turn.is_some() {
            canonical.meta.state = projected.status.session_state.clone();
            canonical.meta.error_reason = projected.error_reason.clone();
        }
    }

    let mut pending_obligations = Vec::new();
    if let Some(turn_id) = unresolved_active_turn {
        push_legacy_obligation(
            &mut pending_obligations,
            session_id,
            &source_state,
            "turn",
            &turn_id.to_string(),
            serde_json::json!({
                "schema": "legacy_agent_reconciliation_obligation_v1",
                "state": "reconciliation_required",
                "kind": "turn_execution",
                "session_id": session_id,
                "turn_id": turn_id.to_string(),
                "safe_actions": ["read_again", "keep_for_manual_resolution"],
            }),
        );
    } else if ambiguous_active_session {
        push_legacy_obligation(
            &mut pending_obligations,
            session_id,
            &source_state,
            "session",
            session_id,
            serde_json::json!({
                "schema": "legacy_agent_reconciliation_obligation_v1",
                "state": "reconciliation_required",
                "kind": "provider_session",
                "session_id": session_id,
                "safe_actions": ["read_again", "keep_for_manual_resolution"],
            }),
        );
    }
    if let Some(permission) = &unresolved_permission {
        let request = serde_json::to_value(&permission.request)
            .map_err(|_| "known legacy permission cannot be canonicalized".to_string())?;
        let identity = if permission.request.id.is_empty() {
            format!("turn-{}", permission.turn_id)
        } else {
            permission.request.id.clone()
        };
        push_legacy_obligation(
            &mut pending_obligations,
            session_id,
            &source_state,
            "permission",
            &identity,
            serde_json::json!({
                "schema": "legacy_agent_reconciliation_obligation_v1",
                "state": "reconciliation_required",
                "kind": "permission",
                "session_id": session_id,
                "turn_id": permission.turn_id.to_string(),
                "request": request,
                "safe_actions": ["read_again", "keep_for_manual_resolution"],
            }),
        );
    }
    for queued in &canonical.pending_send_queue {
        push_legacy_obligation(
            &mut pending_obligations,
            session_id,
            &source_state,
            "queue",
            &queued.queue_item_id,
            serde_json::json!({
                "schema": "legacy_agent_reconciliation_obligation_v1",
                "state": "reconciliation_required",
                "kind": "queued_send",
                "session_id": session_id,
                "queue_item_id": queued.queue_item_id,
                "human_message_id": queued.human_message_id,
                "reserved_turn_id": queued.reserved_turn_id,
                "input_ref": queued.input_ref,
                "safe_actions": ["read_again", "cancel_if_safe", "keep_for_manual_resolution"],
            }),
        );
    }
    match &projected.backend_recovery {
        Some(BackendSessionRecoveryProjection::Recovering { recovery_id, .. })
        | Some(BackendSessionRecoveryProjection::ReconciliationRequired { recovery_id, .. }) => {
            push_legacy_obligation(
                &mut pending_obligations,
                session_id,
                &source_state,
                "recovery",
                recovery_id,
                serde_json::json!({
                    "schema": "legacy_agent_reconciliation_obligation_v1",
                    "state": "reconciliation_required",
                    "kind": "backend_recovery",
                    "session_id": session_id,
                    "recovery_id": recovery_id,
                    "safe_actions": ["read_again", "keep_for_manual_resolution"],
                }),
            )
        }
        None => {}
    }
    if let Some(pending) = &canonical.meta.pending_recovery_message {
        push_legacy_obligation(
            &mut pending_obligations,
            session_id,
            &source_state,
            "publication",
            session_id,
            serde_json::json!({
                "schema": "legacy_agent_reconciliation_obligation_v1",
                "state": "reconciliation_required",
                "kind": "recovery_publication",
                "session_id": session_id,
                "pending_message": pending,
                "safe_actions": ["read_again", "keep_for_manual_resolution"],
            }),
        );
    }

    // Acceptance events preserve a backend operation identity, but the
    // legacy file store did not persist the principal MAC, exact caller
    // binding, or immutable public receipt required by the SQLite operation
    // record. Exposing a fabricated Accepted record would let an unrelated
    // caller claim it. Preserve the exact known event as supervised pending
    // work instead and deliberately leave the public operation binding empty.
    for event in events {
        let (operation_kind, operation_id) = match event {
            AgentSessionEvent::SendOperationAccepted { operation_id, .. } => ("send", operation_id),
            AgentSessionEvent::StopOperationAccepted { operation_id, .. } => ("stop", operation_id),
            AgentSessionEvent::SessionLifecycleOperationAccepted { operation_id, .. } => {
                ("session_lifecycle", operation_id)
            }
            _ => continue,
        };
        if operation_id.is_empty() {
            return Err("known legacy acceptance has no operation identity".to_string());
        }
        push_legacy_obligation(
            &mut pending_obligations,
            session_id,
            &source_state,
            "operation",
            &format!("{operation_kind}:{operation_id}"),
            serde_json::json!({
                "schema": "legacy_agent_reconciliation_obligation_v1",
                "state": "reconciliation_required",
                "kind": "operation_binding",
                "operation_kind": operation_kind,
                "operation_id": operation_id,
                "session_id": session_id,
                "known_observation": legacy_event_observation(event)?,
                "missing_evidence": ["principal", "exact_caller_binding", "immutable_receipt"],
                "safe_actions": ["read_again", "keep_for_manual_resolution"],
            }),
        );
    }

    let mut unique_obligations = Vec::with_capacity(pending_obligations.len());
    let mut obligation_indices = HashMap::<String, usize>::new();
    for obligation in pending_obligations {
        if let Some(index) = obligation_indices.get(&obligation.obligation_id).copied() {
            if unique_obligations[index] == obligation {
                continue;
            }
            return Err(
                "known legacy obligation identity has conflicting observations".to_string(),
            );
        }
        validate_legacy_obligation_record_bound(&obligation.record)?;
        obligation_indices.insert(obligation.obligation_id.clone(), unique_obligations.len());
        unique_obligations.push(obligation);
    }
    let pending_obligations = unique_obligations;

    let terminals = legacy_terminal_participants(session_id, events, canonical.meta.updated_at)?;
    let stop_resolutions = legacy_stop_resolutions(session_id, events)?;
    let pending_queue_count = canonical.pending_send_queue.len();
    let pending_permission_count = usize::from(unresolved_permission.is_some());
    let crate::domain::local_event::SessionProjectionRecord::AgentSession(projection) =
        AgentSessionProjectionCodecV1.encode(&canonical)?
    else {
        return Err("legacy session projection codec returned the wrong kind".to_string());
    };
    let projection = encode_agent_session_projection_record_v1(&projection)?;
    Ok(LegacySessionSemanticProjectionV1 {
        projection,
        terminals,
        stop_resolutions,
        pending_obligations,
        pending_queue_count,
        pending_permission_count,
    })
}

#[cfg(test)]
pub(crate) use projection_commit::ProjectionCommitStage;
pub(crate) use stored_event_v1::{decode_agent_session_events_v1, encode_agent_session_events_v1};
#[cfg(test)]
pub(crate) use stored_message_part_v1::{
    decode_stored_message_parts_v1, encode_stored_message_parts_v1,
};
#[cfg(test)]
pub(crate) use stored_session_v1::{
    decode_activity_entry_v1, decode_chat_session_v1, encode_activity_entry_v1,
    encode_chat_message_pretty_v1, encode_chat_message_v1, encode_chat_session_v1,
    write_message_index_v1,
};

#[cfg(test)]
pub(crate) type ProjectionCommitHook =
    std::sync::Arc<dyn Fn(ProjectionCommitStage) -> Result<(), String> + Send + Sync>;

pub struct FileSessionStorage {
    pub(super) cache: RwLock<HashMap<String, SessionMeta>>,
    /// 壊れた / 旧形式の session JSON を session_id 単位で隔離する。
    /// Spec issues-947: 1つの不正セッションで全体ロードを Err にせず、無関係な正常セッションの
    /// 一覧取得・取得は素通しさせる。値は API に返す汎化済みエラー文言（フルパス・serde 生メッセージは含まない）。
    pub(super) invalid_sessions: RwLock<HashMap<String, String>>,
    /// Durable commit 済みだが meta/events への反映が完了していない session。
    /// clean session の read path で transaction marker を毎回確認しないため、
    /// process 内の reconciliation 対象を session id 単位で限定する。
    #[cfg(test)]
    pub(super) materialization_pending_sessions: RwLock<HashSet<String>>,
    pub(super) file_lock: parking_lot::Mutex<()>,
    pub(super) loaded: AtomicBool,
    /// One-way production cutover latch. Once SQLite migration starts, the
    /// legacy source may still be read but must never be repaired or mutated.
    pub(super) mutation_admission_closed: AtomicBool,
    #[cfg(test)]
    pub(super) recovered_event_logs: RwLock<HashSet<String>>,
    #[cfg(test)]
    pub(super) message_read_count: std::sync::atomic::AtomicUsize,
    #[cfg(test)]
    pub(super) meta_read_count: std::sync::atomic::AtomicUsize,
    #[cfg(test)]
    transaction_apply_hook: RwLock<Option<transaction::TransactionApplyHook>>,
    #[cfg(test)]
    pub(super) projection_commit_hook: RwLock<Option<ProjectionCommitHook>>,
    #[cfg(test)]
    pub(super) event_read_count: std::sync::atomic::AtomicUsize,
    #[cfg(test)]
    pub(super) event_batch_directory_scan_count: std::sync::atomic::AtomicUsize,
}

impl Default for FileSessionStorage {
    fn default() -> Self {
        Self {
            cache: RwLock::new(HashMap::new()),
            invalid_sessions: RwLock::new(HashMap::new()),
            #[cfg(test)]
            materialization_pending_sessions: RwLock::new(HashSet::new()),
            file_lock: parking_lot::Mutex::new(()),
            loaded: AtomicBool::new(false),
            mutation_admission_closed: AtomicBool::new(!cfg!(test)),
            #[cfg(test)]
            recovered_event_logs: RwLock::new(HashSet::new()),
            #[cfg(test)]
            message_read_count: std::sync::atomic::AtomicUsize::new(0),
            #[cfg(test)]
            meta_read_count: std::sync::atomic::AtomicUsize::new(0),
            #[cfg(test)]
            transaction_apply_hook: RwLock::new(None),
            #[cfg(test)]
            projection_commit_hook: RwLock::new(None),
            #[cfg(test)]
            event_read_count: std::sync::atomic::AtomicUsize::new(0),
            #[cfg(test)]
            event_batch_directory_scan_count: std::sync::atomic::AtomicUsize::new(0),
        }
    }
}

impl FileSessionStorage {
    #[cfg(test)]
    pub(super) fn legacy_mutation_admission_closed(&self) -> bool {
        self.mutation_admission_closed
            .load(std::sync::atomic::Ordering::Acquire)
    }

    #[cfg(test)]
    pub(super) fn ensure_legacy_mutation_admitted(&self) -> Result<(), String> {
        if self.legacy_mutation_admission_closed() {
            Err("legacy session mutation admission is closed".to_string())
        } else {
            Ok(())
        }
    }
}

impl crate::domain::agent_session::AgentSessionStorageTypes for FileSessionStorage {
    type Session = ChatSession;
    type Meta = SessionMeta;
    type PageCursor = PageCursor;
    type Page = SessionPage;
    type Message = ChatMessage;
    type MessagePart = MessagePart;
    type Attachment = SessionAttachment;
    type ToolOutput = SessionToolOutput;
    type Event = AgentSessionEvent;
}

impl crate::domain::agent_session::AgentSessionReader for FileSessionStorage {
    fn list_metas(&self, app_data_dir: &Path) -> Result<Vec<Self::Meta>, String> {
        FileSessionStorage::list_metas(self, app_data_dir)
    }

    fn session_title(
        &self,
        app_data_dir: &Path,
        session_id: &str,
    ) -> Result<Option<String>, String> {
        FileSessionStorage::session_title(self, app_data_dir, session_id)
    }

    fn session_titles(&self, app_data_dir: &Path) -> Result<HashMap<String, String>, String> {
        FileSessionStorage::session_titles(self, app_data_dir)
    }

    fn get_session_meta(
        &self,
        app_data_dir: &Path,
        session_id: &str,
    ) -> Result<Option<Self::Meta>, String> {
        FileSessionStorage::get_session_meta(self, app_data_dir, session_id)
    }

    fn load_full_session_for_restore(
        &self,
        app_data_dir: &Path,
        session_id: &str,
    ) -> Result<Option<Self::Session>, String> {
        FileSessionStorage::load_full_session_for_restore(self, app_data_dir, session_id)
    }

    fn load_previous_human_message_before_agent(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        agent_message_id: &str,
    ) -> Result<Option<Self::Message>, String> {
        FileSessionStorage::load_previous_human_message_before_agent(
            self,
            app_data_dir,
            session_id,
            agent_message_id,
        )
    }

    fn get_session_page(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        cursor: Option<Self::PageCursor>,
        limit: usize,
    ) -> Result<Option<Self::Page>, String> {
        FileSessionStorage::get_session_page(self, app_data_dir, session_id, cursor, limit)
    }

    fn get_session_attachment(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        attachment_id: &str,
    ) -> Result<Option<Self::Attachment>, String> {
        FileSessionStorage::get_session_attachment(self, app_data_dir, session_id, attachment_id)
    }

    fn get_session_tool_output(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        tool_output_id: &str,
    ) -> Result<Option<Self::ToolOutput>, String> {
        FileSessionStorage::get_session_tool_output(self, app_data_dir, session_id, tool_output_id)
    }

    fn load_session_events(
        &self,
        app_data_dir: &Path,
        session_id: &str,
    ) -> Result<Vec<Self::Event>, String> {
        FileSessionStorage::load_session_events(self, app_data_dir, session_id)
    }
}

impl SessionReviewContextReader for FileSessionStorage {
    fn get_session_review_context(
        &self,
        app_data_dir: &Path,
        session_id: &str,
    ) -> Result<Option<SessionReviewContext>, String> {
        FileSessionStorage::get_session_review_context(self, app_data_dir, session_id)
    }
}

impl SessionEventLogRecoverySignal for FileSessionStorage {
    #[cfg(test)]
    fn take_event_log_recovered(&self, session_id: &str) -> bool {
        self.recovered_event_logs.write().remove(session_id)
    }
}

impl SessionQueuePauseReader for FileSessionStorage {
    fn load_queue_paused_at(
        &self,
        app_data_dir: &Path,
        session_id: &str,
    ) -> Result<Option<f64>, String> {
        FileSessionStorage::load_queue_paused_at(self, app_data_dir, session_id)
    }
}

impl crate::domain::agent_session::AgentSessionWriter for FileSessionStorage {
    fn close_mutation_admission(&self) {
        self.mutation_admission_closed
            .store(true, std::sync::atomic::Ordering::Release);
    }

    #[cfg(test)]
    fn write_session_title(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        title: Option<&str>,
    ) -> Result<(), String> {
        FileSessionStorage::write_session_title(self, app_data_dir, session_id, title)
    }

    #[cfg(test)]
    fn fork_session_layout(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        forked_meta: &Self::Meta,
    ) -> Result<(), String> {
        FileSessionStorage::fork_session_layout(self, app_data_dir, session_id, forked_meta)
    }

    #[cfg(test)]
    fn remove_session(&self, app_data_dir: &Path, session_id: &str) {
        FileSessionStorage::remove_session(self, app_data_dir, session_id);
    }

    #[cfg(test)]
    fn update_session_meta(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        update: &mut dyn FnMut(&mut Self::Meta) -> Result<(), String>,
    ) -> Result<Self::Meta, String> {
        FileSessionStorage::update_session_meta(self, app_data_dir, session_id, update)
    }

    #[cfg(test)]
    fn update_session_meta_and_append_session_events(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        update: &mut dyn FnMut(&mut Self::Meta) -> Result<(), String>,
        events: &[Self::Event],
    ) -> Result<Self::Meta, String> {
        FileSessionStorage::update_session_meta_and_append_session_events(
            self,
            app_data_dir,
            session_id,
            update,
            events,
        )
    }

    #[cfg(test)]
    fn save_full_session_for_migration_or_restore(
        &self,
        app_data_dir: &Path,
        session: &Self::Session,
    ) -> Result<(), String> {
        FileSessionStorage::save_full_session_for_migration_or_restore(self, app_data_dir, session)
    }

    #[cfg(test)]
    fn append_message(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        message: &Self::Message,
    ) -> Result<Self::Meta, String> {
        FileSessionStorage::append_message(self, app_data_dir, session_id, message)
    }

    #[cfg(test)]
    fn persist_message_parts(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        message_id: &str,
        parts: &[Self::MessagePart],
        streaming_final_seq: u64,
        completed_at: Option<f64>,
    ) -> Result<Vec<Self::MessagePart>, String> {
        FileSessionStorage::persist_message_parts(
            self,
            app_data_dir,
            session_id,
            message_id,
            parts,
            streaming_final_seq,
            completed_at,
        )
    }

    #[cfg(test)]
    fn append_session_event_without_projection(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        event: &Self::Event,
    ) -> Result<(), String> {
        FileSessionStorage::append_session_event_without_projection(
            self,
            app_data_dir,
            session_id,
            event,
        )
    }

    #[cfg(test)]
    fn commit_session_projection(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        events: &[Self::Event],
        prepare: &mut dyn crate::domain::agent_session::AgentSessionProjectionPreparer<
            Self::Event,
            Self::Meta,
            Self::Message,
            Self::MessagePart,
        >,
    ) -> Result<Vec<Self::MessagePart>, String> {
        FileSessionStorage::commit_session_projection(
            self,
            app_data_dir,
            session_id,
            events,
            prepare,
        )
    }

    #[cfg(test)]
    fn append_session_events(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        events: &[Self::Event],
    ) -> Result<(), String> {
        FileSessionStorage::append_session_events(self, app_data_dir, session_id, events)
    }
}
