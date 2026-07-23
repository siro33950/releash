//! Canonical CBOR codec for `AgentSessionDomainEvent` (issues-1499 T-03).
//!
//! The registry owns the persistent identity (`agent_session.session_event`
//! / payload version 1); Rust type names and serde tags never become stored
//! identity. Floats are forbidden by canonical CBOR, so `at` timestamps are
//! stored as shortest-round-trip decimal text.

use crate::adaptor::gateway::local_event_store::canonical_cbor::CborValue;
use crate::adaptor::gateway::local_event_store::envelope::{
    EventCodecError, LocalEventPayloadCodec,
};
use crate::domain::agent_session::entities::{
    Attachment, MessagePart, PermissionAllowedPrompt,
    PermissionDecision as EntityPermissionDecision, PermissionPartStatus, PermissionQuestion,
    PermissionQuestionOption, PermissionRequest, PermissionRequestBody, PermissionRequestStatus,
};
use crate::domain::agent_session::events::{
    AgentSessionDomainEvent, BackendSessionRecoveryReason, GoalReactivationOutcome,
    InterruptReason, ObligationKind, ObligationState, PermissionDecision, PromptInput,
    RecoveryActionKind, RecoveryResultClassification, SendDisposition, SessionLifecycleKind,
    StopResolution, TurnStopReason, TurnTokenUsage,
};
use crate::domain::agent_session::value_objects::{
    JsonPayload, SystemNotificationType, TodoListItem, ToolOutputRef, ToolOutputSummary,
};
use crate::domain::code::MentionReference;
use crate::domain::local_event::LocalDomainEvent;

pub(crate) const AGENT_SESSION_EVENT_TYPE: &str = "agent_session.session_event";
pub(crate) const AGENT_SESSION_PAYLOAD_VERSION: i64 = 1;

pub(crate) struct AgentSessionEventCodec;

fn malformed() -> EventCodecError {
    EventCodecError::MalformedPayload {
        event_type: AGENT_SESSION_EVENT_TYPE.to_string(),
    }
}

// --- Small builders ---------------------------------------------------------

type Entries = Vec<(CborValue, CborValue)>;

fn text_entry(key: &str, value: &str) -> (CborValue, CborValue) {
    (
        CborValue::Text(key.to_string()),
        CborValue::Text(value.to_string()),
    )
}

fn u64_entry(key: &str, value: u64) -> (CborValue, CborValue) {
    (CborValue::Text(key.to_string()), CborValue::Unsigned(value))
}

fn i64_entry(key: &str, value: i64) -> (CborValue, CborValue) {
    (CborValue::Text(key.to_string()), CborValue::int(value))
}

fn bool_entry(key: &str, value: bool) -> (CborValue, CborValue) {
    (CborValue::Text(key.to_string()), CborValue::Bool(value))
}

/// `f64` timestamps round-trip through Rust's shortest decimal display.
fn f64_entry(key: &str, value: f64) -> Result<(CborValue, CborValue), EventCodecError> {
    if !value.is_finite() {
        return Err(malformed());
    }
    Ok((
        CborValue::Text(key.to_string()),
        CborValue::Text(format!("{value}")),
    ))
}

fn push_opt_text(entries: &mut Entries, key: &str, value: &Option<String>) {
    if let Some(value) = value {
        entries.push(text_entry(key, value));
    }
}

fn map_get<'a>(entries: &'a [(CborValue, CborValue)], key: &str) -> Option<&'a CborValue> {
    entries
        .iter()
        .find_map(|(entry_key, value)| match entry_key {
            CborValue::Text(text) if text == key => Some(value),
            _ => None,
        })
}

fn map_text(entries: &[(CborValue, CborValue)], key: &str) -> Result<String, EventCodecError> {
    match map_get(entries, key) {
        Some(CborValue::Text(text)) => Ok(text.clone()),
        _ => Err(malformed()),
    }
}

fn map_opt_text(entries: &[(CborValue, CborValue)], key: &str) -> Option<String> {
    match map_get(entries, key) {
        Some(CborValue::Text(text)) => Some(text.clone()),
        _ => None,
    }
}

fn map_u64(entries: &[(CborValue, CborValue)], key: &str) -> Result<u64, EventCodecError> {
    match map_get(entries, key) {
        Some(CborValue::Unsigned(value)) => Ok(*value),
        _ => Err(malformed()),
    }
}

fn map_opt_u64(entries: &[(CborValue, CborValue)], key: &str) -> Option<u64> {
    match map_get(entries, key) {
        Some(CborValue::Unsigned(value)) => Some(*value),
        _ => None,
    }
}

fn map_i64(entries: &[(CborValue, CborValue)], key: &str) -> Result<i64, EventCodecError> {
    map_get(entries, key)
        .and_then(CborValue::as_i64)
        .ok_or_else(malformed)
}

fn map_bool(entries: &[(CborValue, CborValue)], key: &str) -> Result<bool, EventCodecError> {
    match map_get(entries, key) {
        Some(CborValue::Bool(value)) => Ok(*value),
        _ => Err(malformed()),
    }
}

fn map_f64(entries: &[(CborValue, CborValue)], key: &str) -> Result<f64, EventCodecError> {
    let value = map_text(entries, key)?
        .parse::<f64>()
        .map_err(|_| malformed())?;
    if !value.is_finite() {
        return Err(malformed());
    }
    Ok(value)
}

fn map_array<'a>(
    entries: &'a [(CborValue, CborValue)],
    key: &str,
) -> Result<&'a [CborValue], EventCodecError> {
    match map_get(entries, key) {
        Some(CborValue::Array(items)) => Ok(items),
        _ => Err(malformed()),
    }
}

fn map_map<'a>(
    entries: &'a [(CborValue, CborValue)],
    key: &str,
) -> Result<&'a [(CborValue, CborValue)], EventCodecError> {
    match map_get(entries, key) {
        Some(CborValue::Map(inner)) => Ok(inner),
        _ => Err(malformed()),
    }
}

fn as_map(value: &CborValue) -> Result<&[(CborValue, CborValue)], EventCodecError> {
    match value {
        CborValue::Map(entries) => Ok(entries),
        _ => Err(malformed()),
    }
}

// --- Closed label vocabularies ----------------------------------------------

fn interrupt_reason_label(reason: InterruptReason) -> &'static str {
    reason.label()
}

fn parse_interrupt_reason(raw: &str) -> Result<InterruptReason, EventCodecError> {
    Ok(match raw {
        "abort" => InterruptReason::Abort,
        "timeout" => InterruptReason::Timeout,
        "crash" => InterruptReason::Crash,
        "session_closed" => InterruptReason::SessionClosed,
        _ => return Err(malformed()),
    })
}

fn recovery_reason_label(reason: BackendSessionRecoveryReason) -> &'static str {
    match reason {
        BackendSessionRecoveryReason::ResumeMismatch => "resume_mismatch",
        BackendSessionRecoveryReason::BackendSessionLost => "backend_session_lost",
    }
}

fn parse_recovery_reason(raw: &str) -> Result<BackendSessionRecoveryReason, EventCodecError> {
    Ok(match raw {
        "resume_mismatch" => BackendSessionRecoveryReason::ResumeMismatch,
        "backend_session_lost" => BackendSessionRecoveryReason::BackendSessionLost,
        _ => return Err(malformed()),
    })
}

fn obligation_kind_label(kind: ObligationKind) -> &'static str {
    match kind {
        ObligationKind::ProviderEstablish => "provider_establish",
        ObligationKind::TurnExecution => "turn_execution",
        ObligationKind::PermissionResponse => "permission_response",
        ObligationKind::ProviderInterrupt => "provider_interrupt",
        ObligationKind::SessionClose => "session_close",
        ObligationKind::QueuePause => "queue_pause",
    }
}

fn parse_obligation_kind(raw: &str) -> Result<ObligationKind, EventCodecError> {
    Ok(match raw {
        "provider_establish" => ObligationKind::ProviderEstablish,
        "turn_execution" => ObligationKind::TurnExecution,
        "permission_response" => ObligationKind::PermissionResponse,
        "provider_interrupt" => ObligationKind::ProviderInterrupt,
        "session_close" => ObligationKind::SessionClose,
        "queue_pause" => ObligationKind::QueuePause,
        _ => return Err(malformed()),
    })
}

fn obligation_state_label(state: ObligationState) -> &'static str {
    match state {
        ObligationState::Pending => "pending",
        ObligationState::EffectReserved => "effect_reserved",
        ObligationState::Completed => "completed",
        ObligationState::ReconciliationRequired => "reconciliation_required",
        ObligationState::Cancelled => "cancelled",
    }
}

fn parse_obligation_state(raw: &str) -> Result<ObligationState, EventCodecError> {
    Ok(match raw {
        "pending" => ObligationState::Pending,
        "effect_reserved" => ObligationState::EffectReserved,
        "completed" => ObligationState::Completed,
        "reconciliation_required" => ObligationState::ReconciliationRequired,
        "cancelled" => ObligationState::Cancelled,
        _ => return Err(malformed()),
    })
}

fn lifecycle_kind_label(kind: SessionLifecycleKind) -> &'static str {
    match kind {
        SessionLifecycleKind::Close => "close",
        SessionLifecycleKind::Archive => "archive",
        SessionLifecycleKind::BackendSwitch => "backend_switch",
    }
}

fn parse_lifecycle_kind(raw: &str) -> Result<SessionLifecycleKind, EventCodecError> {
    Ok(match raw {
        "close" => SessionLifecycleKind::Close,
        "archive" => SessionLifecycleKind::Archive,
        "backend_switch" => SessionLifecycleKind::BackendSwitch,
        _ => return Err(malformed()),
    })
}

fn stop_resolution_label(resolution: StopResolution) -> &'static str {
    match resolution {
        StopResolution::Succeeded => "succeeded",
        StopResolution::Superseded => "superseded",
    }
}

fn parse_stop_resolution(raw: &str) -> Result<StopResolution, EventCodecError> {
    Ok(match raw {
        "succeeded" => StopResolution::Succeeded,
        "superseded" => StopResolution::Superseded,
        _ => return Err(malformed()),
    })
}

fn recovery_action_kind_label(kind: RecoveryActionKind) -> &'static str {
    match kind {
        RecoveryActionKind::ReadAgain => "read_again",
        RecoveryActionKind::RetrySameEffect => "retry_same_effect",
        RecoveryActionKind::UseObservedResult => "use_observed_result",
        RecoveryActionKind::CancelIfSafe => "cancel_if_safe",
        RecoveryActionKind::KeepForManualResolution => "keep_for_manual_resolution",
    }
}

fn parse_recovery_action_kind(raw: &str) -> Result<RecoveryActionKind, EventCodecError> {
    Ok(match raw {
        "read_again" => RecoveryActionKind::ReadAgain,
        "retry_same_effect" => RecoveryActionKind::RetrySameEffect,
        "use_observed_result" => RecoveryActionKind::UseObservedResult,
        "cancel_if_safe" => RecoveryActionKind::CancelIfSafe,
        "keep_for_manual_resolution" => RecoveryActionKind::KeepForManualResolution,
        _ => return Err(malformed()),
    })
}

fn classification_label(classification: RecoveryResultClassification) -> &'static str {
    match classification {
        RecoveryResultClassification::Pending => "pending",
        RecoveryResultClassification::Succeeded => "succeeded",
        RecoveryResultClassification::ConfirmedNoEffect => "confirmed_no_effect",
        RecoveryResultClassification::Ambiguous => "ambiguous",
        RecoveryResultClassification::CancelledBeforeEffect => "cancelled_before_effect",
        RecoveryResultClassification::Unchanged => "unchanged",
    }
}

fn parse_classification(raw: &str) -> Result<RecoveryResultClassification, EventCodecError> {
    Ok(match raw {
        "pending" => RecoveryResultClassification::Pending,
        "succeeded" => RecoveryResultClassification::Succeeded,
        "confirmed_no_effect" => RecoveryResultClassification::ConfirmedNoEffect,
        "ambiguous" => RecoveryResultClassification::Ambiguous,
        "cancelled_before_effect" => RecoveryResultClassification::CancelledBeforeEffect,
        "unchanged" => RecoveryResultClassification::Unchanged,
        _ => return Err(malformed()),
    })
}

fn notification_type_label(notification_type: SystemNotificationType) -> &'static str {
    notification_type.as_str()
}

fn parse_notification_type(raw: &str) -> Result<SystemNotificationType, EventCodecError> {
    Ok(match raw {
        "compaction" => SystemNotificationType::Compaction,
        "session_recovery" => SystemNotificationType::SessionRecovery,
        _ => return Err(malformed()),
    })
}

// --- Structured sub-values --------------------------------------------------

fn encode_attachment(attachment: &Attachment) -> CborValue {
    CborValue::Map(vec![
        text_entry("id", &attachment.id),
        text_entry("media_type", &attachment.media_type),
        u64_entry("byte_size", attachment.byte_size),
    ])
}

fn decode_attachment(value: &CborValue) -> Result<Attachment, EventCodecError> {
    let entries = as_map(value)?;
    Ok(Attachment {
        id: map_text(entries, "id")?,
        media_type: map_text(entries, "media_type")?,
        byte_size: map_u64(entries, "byte_size")?,
    })
}

fn encode_mention(mention: &MentionReference) -> CborValue {
    let mut entries = vec![text_entry("file_path", &mention.file_path)];
    if let Some(start_line) = mention.start_line {
        entries.push(u64_entry("start_line", u64::from(start_line)));
    }
    if let Some(end_line) = mention.end_line {
        entries.push(u64_entry("end_line", u64::from(end_line)));
    }
    CborValue::Map(entries)
}

fn decode_mention(value: &CborValue) -> Result<MentionReference, EventCodecError> {
    let entries = as_map(value)?;
    Ok(MentionReference {
        file_path: map_text(entries, "file_path")?,
        start_line: map_opt_u64(entries, "start_line").map(|value| value as u32),
        end_line: map_opt_u64(entries, "end_line").map(|value| value as u32),
    })
}

fn encode_tool_output_ref(content_ref: &ToolOutputRef) -> CborValue {
    CborValue::Map(vec![
        text_entry("id", &content_ref.id),
        u64_entry("byte_size", content_ref.byte_size),
    ])
}

fn decode_tool_output_ref(value: &CborValue) -> Result<ToolOutputRef, EventCodecError> {
    let entries = as_map(value)?;
    Ok(ToolOutputRef {
        id: map_text(entries, "id")?,
        byte_size: map_u64(entries, "byte_size")?,
    })
}

fn encode_tool_output_summary(summary: &ToolOutputSummary) -> CborValue {
    CborValue::Map(vec![
        u64_entry("line_count", summary.line_count),
        u64_entry("byte_size", summary.byte_size),
        bool_entry("is_error", summary.is_error),
        bool_entry("truncated", summary.truncated),
    ])
}

fn decode_tool_output_summary(value: &CborValue) -> Result<ToolOutputSummary, EventCodecError> {
    let entries = as_map(value)?;
    Ok(ToolOutputSummary {
        line_count: map_u64(entries, "line_count")?,
        byte_size: map_u64(entries, "byte_size")?,
        is_error: map_bool(entries, "is_error")?,
        truncated: map_bool(entries, "truncated")?,
    })
}

fn encode_todo_item(item: &TodoListItem) -> CborValue {
    CborValue::Map(vec![
        text_entry("text", &item.text),
        bool_entry("completed", item.completed),
    ])
}

fn decode_todo_item(value: &CborValue) -> Result<TodoListItem, EventCodecError> {
    let entries = as_map(value)?;
    Ok(TodoListItem {
        text: map_text(entries, "text")?,
        completed: map_bool(entries, "completed")?,
    })
}

fn push_opt_json(entries: &mut Entries, key: &str, payload: &Option<JsonPayload>) {
    if let Some(payload) = payload {
        entries.push(text_entry(key, payload.as_str()));
    }
}

fn map_opt_json(entries: &[(CborValue, CborValue)], key: &str) -> Option<JsonPayload> {
    map_opt_text(entries, key).map(JsonPayload::new_unchecked)
}

fn permission_decision_label(decision: EntityPermissionDecision) -> &'static str {
    match decision {
        EntityPermissionDecision::Allowed => "allowed",
        EntityPermissionDecision::Denied => "denied",
        EntityPermissionDecision::Cancelled => "cancelled",
    }
}

fn parse_entity_permission_decision(
    raw: &str,
) -> Result<EntityPermissionDecision, EventCodecError> {
    Ok(match raw {
        "allowed" => EntityPermissionDecision::Allowed,
        "denied" => EntityPermissionDecision::Denied,
        "cancelled" => EntityPermissionDecision::Cancelled,
        _ => return Err(malformed()),
    })
}

fn encode_permission_request(request: &PermissionRequest) -> CborValue {
    let mut entries = vec![
        text_entry("id", &request.id),
        text_entry("tool_name", &request.tool_name),
    ];
    push_opt_text(&mut entries, "tool_use_id", &request.tool_use_id);
    push_opt_text(
        &mut entries,
        "parent_tool_use_id",
        &request.parent_tool_use_id,
    );
    push_opt_text(&mut entries, "title", &request.title);
    push_opt_text(&mut entries, "display_name", &request.display_name);
    push_opt_text(&mut entries, "description", &request.description);
    push_opt_text(&mut entries, "decision_reason", &request.decision_reason);
    let body = match &request.body {
        PermissionRequestBody::ToolApproval { input } => CborValue::Map(vec![
            text_entry("kind", "tool_approval"),
            text_entry("input", input.as_str()),
        ]),
        PermissionRequestBody::PlanApproval {
            plan,
            allowed_prompts,
        } => CborValue::Map(vec![
            text_entry("kind", "plan_approval"),
            text_entry("plan", plan),
            (
                CborValue::Text("allowed_prompts".to_string()),
                CborValue::Array(
                    allowed_prompts
                        .iter()
                        .map(|prompt| {
                            CborValue::Map(vec![
                                text_entry("tool", &prompt.tool),
                                text_entry("prompt", &prompt.prompt),
                            ])
                        })
                        .collect(),
                ),
            ),
        ]),
        PermissionRequestBody::Question { questions } => CborValue::Map(vec![
            text_entry("kind", "question"),
            (
                CborValue::Text("questions".to_string()),
                CborValue::Array(questions.iter().map(encode_permission_question).collect()),
            ),
        ]),
        PermissionRequestBody::PermissionGrant { requested } => CborValue::Map(vec![
            text_entry("kind", "permission_grant"),
            text_entry("requested", requested.as_str()),
        ]),
    };
    entries.push((CborValue::Text("body".to_string()), body));
    let status = match &request.status {
        PermissionRequestStatus::Pending => CborValue::Map(vec![text_entry("kind", "pending")]),
        PermissionRequestStatus::Resolved { decision, answers } => {
            let mut status_entries = vec![
                text_entry("kind", "resolved"),
                text_entry("decision", permission_decision_label(*decision)),
            ];
            push_opt_json(&mut status_entries, "answers", answers);
            CborValue::Map(status_entries)
        }
    };
    entries.push((CborValue::Text("status".to_string()), status));
    CborValue::Map(entries)
}

fn encode_permission_question(question: &PermissionQuestion) -> CborValue {
    let mut entries = vec![
        text_entry("question", &question.question),
        bool_entry("multi_select", question.multi_select),
        (
            CborValue::Text("options".to_string()),
            CborValue::Array(
                question
                    .options
                    .iter()
                    .map(|option| {
                        let mut option_entries = vec![text_entry("label", &option.label)];
                        push_opt_text(&mut option_entries, "description", &option.description);
                        CborValue::Map(option_entries)
                    })
                    .collect(),
            ),
        ),
    ];
    push_opt_text(&mut entries, "header", &question.header);
    CborValue::Map(entries)
}

fn decode_permission_question(value: &CborValue) -> Result<PermissionQuestion, EventCodecError> {
    let entries = as_map(value)?;
    let options = map_array(entries, "options")?
        .iter()
        .map(|option| {
            let option_entries = as_map(option)?;
            Ok(PermissionQuestionOption {
                label: map_text(option_entries, "label")?,
                description: map_opt_text(option_entries, "description"),
            })
        })
        .collect::<Result<Vec<_>, EventCodecError>>()?;
    Ok(PermissionQuestion {
        question: map_text(entries, "question")?,
        header: map_opt_text(entries, "header"),
        options,
        multi_select: map_bool(entries, "multi_select")?,
    })
}

fn decode_permission_request(value: &CborValue) -> Result<PermissionRequest, EventCodecError> {
    let entries = as_map(value)?;
    let body_entries = map_map(entries, "body")?;
    let body = match map_text(body_entries, "kind")?.as_str() {
        "tool_approval" => PermissionRequestBody::ToolApproval {
            input: JsonPayload::new_unchecked(map_text(body_entries, "input")?),
        },
        "plan_approval" => PermissionRequestBody::PlanApproval {
            plan: map_text(body_entries, "plan")?,
            allowed_prompts: map_array(body_entries, "allowed_prompts")?
                .iter()
                .map(|prompt| {
                    let prompt_entries = as_map(prompt)?;
                    Ok(PermissionAllowedPrompt {
                        tool: map_text(prompt_entries, "tool")?,
                        prompt: map_text(prompt_entries, "prompt")?,
                    })
                })
                .collect::<Result<Vec<_>, EventCodecError>>()?,
        },
        "question" => PermissionRequestBody::Question {
            questions: map_array(body_entries, "questions")?
                .iter()
                .map(decode_permission_question)
                .collect::<Result<Vec<_>, EventCodecError>>()?,
        },
        "permission_grant" => PermissionRequestBody::PermissionGrant {
            requested: JsonPayload::new_unchecked(map_text(body_entries, "requested")?),
        },
        _ => return Err(malformed()),
    };
    let status_entries = map_map(entries, "status")?;
    let status = match map_text(status_entries, "kind")?.as_str() {
        "pending" => PermissionRequestStatus::Pending,
        "resolved" => PermissionRequestStatus::Resolved {
            decision: parse_entity_permission_decision(&map_text(status_entries, "decision")?)?,
            answers: map_opt_json(status_entries, "answers"),
        },
        _ => return Err(malformed()),
    };
    Ok(PermissionRequest {
        id: map_text(entries, "id")?,
        tool_use_id: map_opt_text(entries, "tool_use_id"),
        parent_tool_use_id: map_opt_text(entries, "parent_tool_use_id"),
        tool_name: map_text(entries, "tool_name")?,
        body,
        title: map_opt_text(entries, "title"),
        display_name: map_opt_text(entries, "display_name"),
        description: map_opt_text(entries, "description"),
        decision_reason: map_opt_text(entries, "decision_reason"),
        status,
    })
}

fn permission_part_status_label(status: PermissionPartStatus) -> &'static str {
    status.as_str()
}

fn parse_permission_part_status(raw: &str) -> Result<PermissionPartStatus, EventCodecError> {
    PermissionPartStatus::from_wire(raw).ok_or_else(malformed)
}

fn encode_message_part(part: &MessagePart) -> CborValue {
    let entries = match part {
        MessagePart::Thinking {
            content,
            parent_tool_use_id,
        } => {
            let mut entries = vec![
                text_entry("kind", "thinking"),
                text_entry("content", content),
            ];
            push_opt_text(&mut entries, "parent_tool_use_id", parent_tool_use_id);
            entries
        }
        MessagePart::Text {
            content,
            parent_tool_use_id,
        } => {
            let mut entries = vec![text_entry("kind", "text"), text_entry("content", content)];
            push_opt_text(&mut entries, "parent_tool_use_id", parent_tool_use_id);
            entries
        }
        MessagePart::ToolUse {
            id,
            tool,
            input,
            parent_tool_use_id,
        } => {
            let mut entries = vec![
                text_entry("kind", "tool_use"),
                text_entry("id", id),
                text_entry("tool", tool),
                text_entry("input", input.as_str()),
            ];
            push_opt_text(&mut entries, "parent_tool_use_id", parent_tool_use_id);
            entries
        }
        MessagePart::ToolResult {
            content,
            is_error,
            tool_use_id,
            parent_tool_use_id,
            content_ref,
            summary,
        } => {
            let mut entries = vec![
                text_entry("kind", "tool_result"),
                text_entry("content", content),
                bool_entry("is_error", *is_error),
            ];
            push_opt_text(&mut entries, "tool_use_id", tool_use_id);
            push_opt_text(&mut entries, "parent_tool_use_id", parent_tool_use_id);
            if let Some(content_ref) = content_ref {
                entries.push((
                    CborValue::Text("content_ref".to_string()),
                    encode_tool_output_ref(content_ref),
                ));
            }
            if let Some(summary) = summary {
                entries.push((
                    CborValue::Text("summary".to_string()),
                    encode_tool_output_summary(summary),
                ));
            }
            entries
        }
        MessagePart::Error {
            content,
            parent_tool_use_id,
        } => {
            let mut entries = vec![text_entry("kind", "error"), text_entry("content", content)];
            push_opt_text(&mut entries, "parent_tool_use_id", parent_tool_use_id);
            entries
        }
        MessagePart::Permission {
            request,
            status,
            answers,
            parent_tool_use_id,
        } => {
            let mut entries = vec![
                text_entry("kind", "permission"),
                (
                    CborValue::Text("request".to_string()),
                    encode_permission_request(request),
                ),
                text_entry("status", permission_part_status_label(*status)),
            ];
            push_opt_json(&mut entries, "answers", answers);
            push_opt_text(&mut entries, "parent_tool_use_id", parent_tool_use_id);
            entries
        }
        MessagePart::TaskStatus {
            task_tool_use_id,
            status,
            description,
            summary,
        } => {
            let mut entries = vec![
                text_entry("kind", "task_status"),
                text_entry("task_tool_use_id", task_tool_use_id),
                text_entry("status", status),
            ];
            push_opt_text(&mut entries, "description", description);
            push_opt_text(&mut entries, "summary", summary);
            entries
        }
        MessagePart::TodoListSnapshot { items } => vec![
            text_entry("kind", "todo_list_snapshot"),
            (
                CborValue::Text("items".to_string()),
                CborValue::Array(items.iter().map(encode_todo_item).collect()),
            ),
        ],
        MessagePart::SystemNotification {
            notification_type,
            status,
            label,
            detail,
            hook_id,
        } => {
            let mut entries = vec![
                text_entry("kind", "system_notification"),
                text_entry(
                    "notification_type",
                    notification_type_label(*notification_type),
                ),
                text_entry("status", status),
                text_entry("label", label),
            ];
            push_opt_text(&mut entries, "detail", detail);
            push_opt_text(&mut entries, "hook_id", hook_id);
            entries
        }
        MessagePart::Image { data, media_type } => vec![
            text_entry("kind", "image"),
            text_entry("data", data),
            text_entry("media_type", media_type),
        ],
        MessagePart::ImageRef { attachment } => vec![
            text_entry("kind", "image_ref"),
            (
                CborValue::Text("attachment".to_string()),
                encode_attachment(attachment),
            ),
        ],
    };
    CborValue::Map(entries)
}

fn decode_message_part(value: &CborValue) -> Result<MessagePart, EventCodecError> {
    let entries = as_map(value)?;
    Ok(match map_text(entries, "kind")?.as_str() {
        "thinking" => MessagePart::Thinking {
            content: map_text(entries, "content")?,
            parent_tool_use_id: map_opt_text(entries, "parent_tool_use_id"),
        },
        "text" => MessagePart::Text {
            content: map_text(entries, "content")?,
            parent_tool_use_id: map_opt_text(entries, "parent_tool_use_id"),
        },
        "tool_use" => MessagePart::ToolUse {
            id: map_text(entries, "id")?,
            tool: map_text(entries, "tool")?,
            input: JsonPayload::new_unchecked(map_text(entries, "input")?),
            parent_tool_use_id: map_opt_text(entries, "parent_tool_use_id"),
        },
        "tool_result" => MessagePart::ToolResult {
            content: map_text(entries, "content")?,
            is_error: map_bool(entries, "is_error")?,
            tool_use_id: map_opt_text(entries, "tool_use_id"),
            parent_tool_use_id: map_opt_text(entries, "parent_tool_use_id"),
            content_ref: match map_get(entries, "content_ref") {
                Some(value) => Some(decode_tool_output_ref(value)?),
                None => None,
            },
            summary: match map_get(entries, "summary") {
                Some(value) => Some(decode_tool_output_summary(value)?),
                None => None,
            },
        },
        "error" => MessagePart::Error {
            content: map_text(entries, "content")?,
            parent_tool_use_id: map_opt_text(entries, "parent_tool_use_id"),
        },
        "permission" => MessagePart::Permission {
            request: decode_permission_request(map_get(entries, "request").ok_or_else(malformed)?)?,
            status: parse_permission_part_status(&map_text(entries, "status")?)?,
            answers: map_opt_json(entries, "answers"),
            parent_tool_use_id: map_opt_text(entries, "parent_tool_use_id"),
        },
        "task_status" => MessagePart::TaskStatus {
            task_tool_use_id: map_text(entries, "task_tool_use_id")?,
            status: map_text(entries, "status")?,
            description: map_opt_text(entries, "description"),
            summary: map_opt_text(entries, "summary"),
        },
        "todo_list_snapshot" => MessagePart::TodoListSnapshot {
            items: map_array(entries, "items")?
                .iter()
                .map(decode_todo_item)
                .collect::<Result<Vec<_>, EventCodecError>>()?,
        },
        "system_notification" => MessagePart::SystemNotification {
            notification_type: parse_notification_type(&map_text(entries, "notification_type")?)?,
            status: map_text(entries, "status")?,
            label: map_text(entries, "label")?,
            detail: map_opt_text(entries, "detail"),
            hook_id: map_opt_text(entries, "hook_id"),
        },
        "image" => MessagePart::Image {
            data: map_text(entries, "data")?,
            media_type: map_text(entries, "media_type")?,
        },
        "image_ref" => MessagePart::ImageRef {
            attachment: decode_attachment(map_get(entries, "attachment").ok_or_else(malformed)?)?,
        },
        _ => return Err(malformed()),
    })
}

fn encode_prompt_input(prompt: &PromptInput) -> CborValue {
    CborValue::Map(vec![
        text_entry("content", &prompt.content),
        (
            CborValue::Text("mentions".to_string()),
            CborValue::Array(prompt.mentions.iter().map(encode_mention).collect()),
        ),
        (
            CborValue::Text("attachment_refs".to_string()),
            CborValue::Array(
                prompt
                    .attachment_refs
                    .iter()
                    .map(encode_attachment)
                    .collect(),
            ),
        ),
        (
            CborValue::Text("parts".to_string()),
            CborValue::Array(prompt.parts.iter().map(encode_message_part).collect()),
        ),
    ])
}

fn decode_prompt_input(value: &CborValue) -> Result<PromptInput, EventCodecError> {
    let entries = as_map(value)?;
    Ok(PromptInput {
        content: map_text(entries, "content")?,
        mentions: map_array(entries, "mentions")?
            .iter()
            .map(decode_mention)
            .collect::<Result<Vec<_>, EventCodecError>>()?,
        attachment_refs: map_array(entries, "attachment_refs")?
            .iter()
            .map(decode_attachment)
            .collect::<Result<Vec<_>, EventCodecError>>()?,
        parts: map_array(entries, "parts")?
            .iter()
            .map(decode_message_part)
            .collect::<Result<Vec<_>, EventCodecError>>()?,
    })
}

fn encode_goal_outcome(outcome: &GoalReactivationOutcome) -> CborValue {
    match outcome {
        GoalReactivationOutcome::NoCurrentGoal => {
            CborValue::Map(vec![text_entry("kind", "no_current_goal")])
        }
        GoalReactivationOutcome::TerminalGoalUnchanged {
            goal_id,
            goal_revision,
        } => CborValue::Map(vec![
            text_entry("kind", "terminal_goal_unchanged"),
            text_entry("goal_id", goal_id),
            u64_entry("goal_revision", *goal_revision),
        ]),
        GoalReactivationOutcome::Restored {
            goal_id,
            goal_revision,
            provider_goal_ref,
        } => {
            let mut entries = vec![
                text_entry("kind", "restored"),
                text_entry("goal_id", goal_id),
                u64_entry("goal_revision", *goal_revision),
            ];
            push_opt_text(&mut entries, "provider_goal_ref", provider_goal_ref);
            CborValue::Map(entries)
        }
        GoalReactivationOutcome::ObservedUnchanged {
            goal_id,
            goal_revision,
        } => CborValue::Map(vec![
            text_entry("kind", "observed_unchanged"),
            text_entry("goal_id", goal_id),
            u64_entry("goal_revision", *goal_revision),
        ]),
    }
}

fn decode_goal_outcome(value: &CborValue) -> Result<GoalReactivationOutcome, EventCodecError> {
    let entries = as_map(value)?;
    Ok(match map_text(entries, "kind")?.as_str() {
        "no_current_goal" => GoalReactivationOutcome::NoCurrentGoal,
        "terminal_goal_unchanged" => GoalReactivationOutcome::TerminalGoalUnchanged {
            goal_id: map_text(entries, "goal_id")?,
            goal_revision: map_u64(entries, "goal_revision")?,
        },
        "restored" => GoalReactivationOutcome::Restored {
            goal_id: map_text(entries, "goal_id")?,
            goal_revision: map_u64(entries, "goal_revision")?,
            provider_goal_ref: map_opt_text(entries, "provider_goal_ref"),
        },
        "observed_unchanged" => GoalReactivationOutcome::ObservedUnchanged {
            goal_id: map_text(entries, "goal_id")?,
            goal_revision: map_u64(entries, "goal_revision")?,
        },
        _ => return Err(malformed()),
    })
}

fn encode_send_disposition(disposition: &SendDisposition) -> CborValue {
    match disposition {
        SendDisposition::StartedTurn { turn_id } => CborValue::Map(vec![
            text_entry("kind", "started_turn"),
            text_entry("turn_id", turn_id),
        ]),
        SendDisposition::Queued { queue_item_id } => CborValue::Map(vec![
            text_entry("kind", "queued"),
            text_entry("queue_item_id", queue_item_id),
        ]),
    }
}

fn decode_send_disposition(value: &CborValue) -> Result<SendDisposition, EventCodecError> {
    let entries = as_map(value)?;
    Ok(match map_text(entries, "kind")?.as_str() {
        "started_turn" => SendDisposition::StartedTurn {
            turn_id: map_text(entries, "turn_id")?,
        },
        "queued" => SendDisposition::Queued {
            queue_item_id: map_text(entries, "queue_item_id")?,
        },
        _ => return Err(malformed()),
    })
}

// --- Top-level event codec --------------------------------------------------

fn encode_event(event: &AgentSessionDomainEvent) -> Result<CborValue, EventCodecError> {
    use AgentSessionDomainEvent as E;
    let entries: Entries = match event {
        E::BackendSessionRecoveryStarted {
            recovery_id,
            old_provider_session_generation,
            reason,
            at,
        } => vec![
            text_entry("kind", "backend_session_recovery_started"),
            text_entry("recovery_id", recovery_id),
            u64_entry(
                "old_provider_session_generation",
                *old_provider_session_generation,
            ),
            text_entry("reason", recovery_reason_label(*reason)),
            f64_entry("at", *at)?,
        ],
        E::SessionConfigurationReactivated {
            recovery_id,
            provider_session_generation,
            consumed_observation_id,
            at,
        } => {
            let mut entries = vec![
                text_entry("kind", "session_configuration_reactivated"),
                text_entry("recovery_id", recovery_id),
                u64_entry("provider_session_generation", *provider_session_generation),
                f64_entry("at", *at)?,
            ];
            push_opt_text(
                &mut entries,
                "consumed_observation_id",
                consumed_observation_id,
            );
            entries
        }
        E::SessionGoalReactivated {
            recovery_id,
            outcome,
            provider_session_generation,
            restoring_turn_id,
            consumed_observation_id,
            at,
        } => {
            let mut entries = vec![
                text_entry("kind", "session_goal_reactivated"),
                text_entry("recovery_id", recovery_id),
                (
                    CborValue::Text("outcome".to_string()),
                    encode_goal_outcome(outcome),
                ),
                u64_entry("provider_session_generation", *provider_session_generation),
                f64_entry("at", *at)?,
            ];
            push_opt_text(&mut entries, "restoring_turn_id", restoring_turn_id);
            push_opt_text(
                &mut entries,
                "consumed_observation_id",
                consumed_observation_id,
            );
            entries
        }
        E::BackendSessionRecoveryCompleted {
            recovery_id,
            provider_session_generation,
            at,
        } => vec![
            text_entry("kind", "backend_session_recovery_completed"),
            text_entry("recovery_id", recovery_id),
            u64_entry("provider_session_generation", *provider_session_generation),
            f64_entry("at", *at)?,
        ],
        E::BackendSessionRecoveryFailed {
            recovery_id,
            error,
            at,
        } => vec![
            text_entry("kind", "backend_session_recovery_failed"),
            text_entry("recovery_id", recovery_id),
            text_entry("error", error),
            f64_entry("at", *at)?,
        ],
        E::TurnStarted {
            turn_id,
            message_id,
            assistant_message_id,
            prompt,
            at,
        } => {
            let mut entries = vec![
                text_entry("kind", "turn_started"),
                u64_entry("turn_id", *turn_id),
                text_entry("message_id", message_id),
                (
                    CborValue::Text("prompt".to_string()),
                    encode_prompt_input(prompt),
                ),
                f64_entry("at", *at)?,
            ];
            push_opt_text(&mut entries, "assistant_message_id", assistant_message_id);
            entries
        }
        E::TurnInterruptRequested { turn_id, at } => vec![
            text_entry("kind", "turn_interrupt_requested"),
            u64_entry("turn_id", *turn_id),
            f64_entry("at", *at)?,
        ],
        E::QueuePaused { at } => vec![text_entry("kind", "queue_paused"), f64_entry("at", *at)?],
        E::QueueResumed {
            expected_paused_at,
            at,
        } => vec![
            text_entry("kind", "queue_resumed"),
            f64_entry("expected_paused_at", *expected_paused_at)?,
            f64_entry("at", *at)?,
        ],
        E::TextRecorded {
            turn_id,
            message_id,
            content,
            parent_tool_use_id,
        } => {
            let mut entries = vec![
                text_entry("kind", "text_recorded"),
                u64_entry("turn_id", *turn_id),
                text_entry("message_id", message_id),
                text_entry("content", content),
            ];
            push_opt_text(&mut entries, "parent_tool_use_id", parent_tool_use_id);
            entries
        }
        E::ReasoningRecorded {
            turn_id,
            message_id,
            content,
            parent_tool_use_id,
        } => {
            let mut entries = vec![
                text_entry("kind", "reasoning_recorded"),
                u64_entry("turn_id", *turn_id),
                text_entry("message_id", message_id),
                text_entry("content", content),
            ];
            push_opt_text(&mut entries, "parent_tool_use_id", parent_tool_use_id);
            entries
        }
        E::ErrorRecorded {
            turn_id,
            message_id,
            content,
            parent_tool_use_id,
        } => {
            let mut entries = vec![
                text_entry("kind", "error_recorded"),
                u64_entry("turn_id", *turn_id),
                text_entry("message_id", message_id),
                text_entry("content", content),
            ];
            push_opt_text(&mut entries, "parent_tool_use_id", parent_tool_use_id);
            entries
        }
        E::ToolCallStarted {
            turn_id,
            tool_use_id,
            tool,
            input,
            parent_tool_use_id,
        } => {
            let mut entries = vec![
                text_entry("kind", "tool_call_started"),
                u64_entry("turn_id", *turn_id),
                text_entry("tool_use_id", tool_use_id),
                text_entry("tool", tool),
                text_entry("input", input.as_str()),
            ];
            push_opt_text(&mut entries, "parent_tool_use_id", parent_tool_use_id);
            entries
        }
        E::ToolCallSucceeded {
            turn_id,
            tool_use_id,
            content,
            content_ref,
            summary,
        } => {
            let mut entries = vec![
                text_entry("kind", "tool_call_succeeded"),
                u64_entry("turn_id", *turn_id),
                text_entry("tool_use_id", tool_use_id),
                text_entry("content", content),
            ];
            if let Some(content_ref) = content_ref {
                entries.push((
                    CborValue::Text("content_ref".to_string()),
                    encode_tool_output_ref(content_ref),
                ));
            }
            if let Some(summary) = summary {
                entries.push((
                    CborValue::Text("summary".to_string()),
                    encode_tool_output_summary(summary),
                ));
            }
            entries
        }
        E::ToolCallFailed {
            turn_id,
            tool_use_id,
            content,
            content_ref,
            summary,
        } => {
            let mut entries = vec![
                text_entry("kind", "tool_call_failed"),
                u64_entry("turn_id", *turn_id),
                text_entry("tool_use_id", tool_use_id),
                text_entry("content", content),
            ];
            if let Some(content_ref) = content_ref {
                entries.push((
                    CborValue::Text("content_ref".to_string()),
                    encode_tool_output_ref(content_ref),
                ));
            }
            if let Some(summary) = summary {
                entries.push((
                    CborValue::Text("summary".to_string()),
                    encode_tool_output_summary(summary),
                ));
            }
            entries
        }
        E::ToolResultRecorded {
            turn_id,
            message_id,
            content,
            is_error,
            content_ref,
            summary,
            tool_use_id,
            parent_tool_use_id,
        } => {
            let mut entries = vec![
                text_entry("kind", "tool_result_recorded"),
                u64_entry("turn_id", *turn_id),
                text_entry("message_id", message_id),
                text_entry("content", content),
                bool_entry("is_error", *is_error),
            ];
            if let Some(content_ref) = content_ref {
                entries.push((
                    CborValue::Text("content_ref".to_string()),
                    encode_tool_output_ref(content_ref),
                ));
            }
            if let Some(summary) = summary {
                entries.push((
                    CborValue::Text("summary".to_string()),
                    encode_tool_output_summary(summary),
                ));
            }
            push_opt_text(&mut entries, "tool_use_id", tool_use_id);
            push_opt_text(&mut entries, "parent_tool_use_id", parent_tool_use_id);
            entries
        }
        E::ToolCallRetried {
            turn_id,
            tool_use_id,
            attempt,
        } => vec![
            text_entry("kind", "tool_call_retried"),
            u64_entry("turn_id", *turn_id),
            text_entry("tool_use_id", tool_use_id),
            u64_entry("attempt", u64::from(*attempt)),
        ],
        E::PermissionRequested {
            turn_id,
            tool_use_id,
            request,
        } => {
            let mut entries = vec![
                text_entry("kind", "permission_requested"),
                u64_entry("turn_id", *turn_id),
                (
                    CborValue::Text("request".to_string()),
                    encode_permission_request(request),
                ),
            ];
            push_opt_text(&mut entries, "tool_use_id", tool_use_id);
            entries
        }
        E::PermissionResolved {
            turn_id,
            tool_use_id,
            request_id,
            decision,
            answers,
        } => {
            let decision_label = match decision {
                PermissionDecision::Allowed => "allowed",
                PermissionDecision::Denied => "denied",
                PermissionDecision::Cancelled => "cancelled",
            };
            let mut entries = vec![
                text_entry("kind", "permission_resolved"),
                u64_entry("turn_id", *turn_id),
                text_entry("decision", decision_label),
            ];
            push_opt_text(&mut entries, "tool_use_id", tool_use_id);
            push_opt_text(&mut entries, "request_id", request_id);
            push_opt_json(&mut entries, "answers", answers);
            entries
        }
        E::TaskStatusChanged {
            turn_id,
            message_id,
            task_tool_use_id,
            status,
            description,
            summary,
        } => {
            let mut entries = vec![
                text_entry("kind", "task_status_changed"),
                u64_entry("turn_id", *turn_id),
                text_entry("message_id", message_id),
                text_entry("task_tool_use_id", task_tool_use_id),
                text_entry("status", status),
            ];
            push_opt_text(&mut entries, "description", description);
            push_opt_text(&mut entries, "summary", summary);
            entries
        }
        E::TodoListSnapshotRecorded {
            turn_id,
            message_id,
            items,
        } => vec![
            text_entry("kind", "todo_list_snapshot_recorded"),
            u64_entry("turn_id", *turn_id),
            text_entry("message_id", message_id),
            (
                CborValue::Text("items".to_string()),
                CborValue::Array(items.iter().map(encode_todo_item).collect()),
            ),
        ],
        E::SystemNotificationRecorded {
            turn_id,
            message_id,
            notification_type,
            status,
            label,
            detail,
            hook_id,
        } => {
            let mut entries = vec![
                text_entry("kind", "system_notification_recorded"),
                u64_entry("turn_id", *turn_id),
                text_entry("message_id", message_id),
                text_entry(
                    "notification_type",
                    notification_type_label(*notification_type),
                ),
                text_entry("status", status),
                text_entry("label", label),
            ];
            push_opt_text(&mut entries, "detail", detail);
            push_opt_text(&mut entries, "hook_id", hook_id);
            entries
        }
        E::ImageRecorded {
            turn_id,
            message_id,
            data,
            media_type,
        } => vec![
            text_entry("kind", "image_recorded"),
            u64_entry("turn_id", *turn_id),
            text_entry("message_id", message_id),
            text_entry("data", data),
            text_entry("media_type", media_type),
        ],
        E::ImageRefRecorded {
            turn_id,
            message_id,
            attachment,
        } => vec![
            text_entry("kind", "image_ref_recorded"),
            u64_entry("turn_id", *turn_id),
            text_entry("message_id", message_id),
            (
                CborValue::Text("attachment".to_string()),
                encode_attachment(attachment),
            ),
        ],
        E::FinalPartsRecorded {
            turn_id,
            message_id,
            parts,
        } => vec![
            text_entry("kind", "final_parts_recorded"),
            u64_entry("turn_id", *turn_id),
            text_entry("message_id", message_id),
            (
                CborValue::Text("parts".to_string()),
                CborValue::Array(parts.iter().map(encode_message_part).collect()),
            ),
        ],
        E::TurnCompleted {
            turn_id,
            exit_code,
            stop_reason,
            token_usage,
        } => {
            let mut entries = vec![
                text_entry("kind", "turn_completed"),
                u64_entry("turn_id", *turn_id),
                i64_entry("exit_code", *exit_code),
            ];
            if matches!(stop_reason, Some(TurnStopReason::Refusal)) {
                entries.push(text_entry("stop_reason", "refusal"));
            }
            if let Some(usage) = token_usage {
                entries.push((
                    CborValue::Text("token_usage".to_string()),
                    CborValue::Map(vec![
                        u64_entry("input_tokens", usage.input_tokens),
                        u64_entry("output_tokens", usage.output_tokens),
                    ]),
                ));
            }
            entries
        }
        E::TurnInterrupted {
            turn_id,
            reason,
            exit_code,
            error,
        } => {
            let mut entries = vec![
                text_entry("kind", "turn_interrupted"),
                u64_entry("turn_id", *turn_id),
                text_entry("reason", interrupt_reason_label(*reason)),
                i64_entry("exit_code", *exit_code),
            ];
            push_opt_text(&mut entries, "error", error);
            entries
        }
        E::SessionErrored {
            message_id,
            reason,
            at,
        } => vec![
            text_entry("kind", "session_errored"),
            text_entry("message_id", message_id),
            text_entry("reason", reason),
            f64_entry("at", *at)?,
        ],
        E::SessionClosed { at } => {
            vec![text_entry("kind", "session_closed"), f64_entry("at", *at)?]
        }
        E::SendOperationAccepted {
            operation_id,
            disposition,
            human_message_id,
            prompt,
            reserved_turn_id,
            at,
        } => {
            let mut entries = vec![
                text_entry("kind", "send_operation_accepted"),
                text_entry("operation_id", operation_id),
                (
                    CborValue::Text("disposition".to_string()),
                    encode_send_disposition(disposition),
                ),
                f64_entry("at", *at)?,
            ];
            push_opt_text(&mut entries, "human_message_id", human_message_id);
            if let Some(prompt) = prompt {
                entries.push((
                    CborValue::Text("prompt".to_string()),
                    encode_prompt_input(prompt),
                ));
            }
            push_opt_text(&mut entries, "reserved_turn_id", reserved_turn_id);
            entries
        }
        E::StopOperationAccepted {
            operation_id,
            target_turn_id,
            at,
        } => vec![
            text_entry("kind", "stop_operation_accepted"),
            text_entry("operation_id", operation_id),
            u64_entry("target_turn_id", *target_turn_id),
            f64_entry("at", *at)?,
        ],
        E::SessionLifecycleOperationAccepted {
            operation_id,
            kind,
            at,
        } => vec![
            text_entry("kind", "session_lifecycle_operation_accepted"),
            text_entry("operation_id", operation_id),
            text_entry("lifecycle_kind", lifecycle_kind_label(*kind)),
            f64_entry("at", *at)?,
        ],
        E::ObligationRecorded {
            obligation_id,
            kind,
            state,
            at,
        } => vec![
            text_entry("kind", "obligation_recorded"),
            text_entry("obligation_id", obligation_id),
            text_entry("obligation_kind", obligation_kind_label(*kind)),
            text_entry("state", obligation_state_label(*state)),
            f64_entry("at", *at)?,
        ],
        E::StopResolutionRecorded {
            operation_id,
            turn_id,
            resolution,
            at,
        } => vec![
            text_entry("kind", "stop_resolution_recorded"),
            text_entry("operation_id", operation_id),
            u64_entry("turn_id", *turn_id),
            text_entry("resolution", stop_resolution_label(*resolution)),
            f64_entry("at", *at)?,
        ],
        E::PendingRecoveryPublished {
            obligation_id,
            kind,
            at,
        } => vec![
            text_entry("kind", "pending_recovery_published"),
            text_entry("obligation_id", obligation_id),
            text_entry("obligation_kind", obligation_kind_label(*kind)),
            f64_entry("at", *at)?,
        ],
        E::RecoveryActionResolved {
            action_id,
            obligation_id,
            kind,
            classification,
            at,
        } => vec![
            text_entry("kind", "recovery_action_resolved"),
            text_entry("action_id", action_id),
            text_entry("obligation_id", obligation_id),
            text_entry("action_kind", recovery_action_kind_label(*kind)),
            text_entry("classification", classification_label(*classification)),
            f64_entry("at", *at)?,
        ],
    };
    Ok(CborValue::Map(entries))
}

fn decode_event(value: &CborValue) -> Result<Option<AgentSessionDomainEvent>, EventCodecError> {
    use AgentSessionDomainEvent as E;
    let entries = as_map(value)?;
    let kind = map_text(entries, "kind")?;
    let event = match kind.as_str() {
        "backend_session_recovery_started" => E::BackendSessionRecoveryStarted {
            recovery_id: map_text(entries, "recovery_id")?,
            old_provider_session_generation: map_u64(entries, "old_provider_session_generation")?,
            reason: parse_recovery_reason(&map_text(entries, "reason")?)?,
            at: map_f64(entries, "at")?,
        },
        "session_configuration_reactivated" => E::SessionConfigurationReactivated {
            recovery_id: map_text(entries, "recovery_id")?,
            provider_session_generation: map_u64(entries, "provider_session_generation")?,
            consumed_observation_id: map_opt_text(entries, "consumed_observation_id"),
            at: map_f64(entries, "at")?,
        },
        "session_goal_reactivated" => E::SessionGoalReactivated {
            recovery_id: map_text(entries, "recovery_id")?,
            outcome: decode_goal_outcome(map_get(entries, "outcome").ok_or_else(malformed)?)?,
            provider_session_generation: map_u64(entries, "provider_session_generation")?,
            restoring_turn_id: map_opt_text(entries, "restoring_turn_id"),
            consumed_observation_id: map_opt_text(entries, "consumed_observation_id"),
            at: map_f64(entries, "at")?,
        },
        "backend_session_recovery_completed" => E::BackendSessionRecoveryCompleted {
            recovery_id: map_text(entries, "recovery_id")?,
            provider_session_generation: map_u64(entries, "provider_session_generation")?,
            at: map_f64(entries, "at")?,
        },
        "backend_session_recovery_failed" => E::BackendSessionRecoveryFailed {
            recovery_id: map_text(entries, "recovery_id")?,
            error: map_text(entries, "error")?,
            at: map_f64(entries, "at")?,
        },
        "turn_started" => E::TurnStarted {
            turn_id: map_u64(entries, "turn_id")?,
            message_id: map_text(entries, "message_id")?,
            assistant_message_id: map_opt_text(entries, "assistant_message_id"),
            prompt: decode_prompt_input(map_get(entries, "prompt").ok_or_else(malformed)?)?,
            at: map_f64(entries, "at")?,
        },
        "turn_interrupt_requested" => E::TurnInterruptRequested {
            turn_id: map_u64(entries, "turn_id")?,
            at: map_f64(entries, "at")?,
        },
        "queue_paused" => E::QueuePaused {
            at: map_f64(entries, "at")?,
        },
        "queue_resumed" => E::QueueResumed {
            expected_paused_at: map_f64(entries, "expected_paused_at")?,
            at: map_f64(entries, "at")?,
        },
        "text_recorded" => E::TextRecorded {
            turn_id: map_u64(entries, "turn_id")?,
            message_id: map_text(entries, "message_id")?,
            content: map_text(entries, "content")?,
            parent_tool_use_id: map_opt_text(entries, "parent_tool_use_id"),
        },
        "reasoning_recorded" => E::ReasoningRecorded {
            turn_id: map_u64(entries, "turn_id")?,
            message_id: map_text(entries, "message_id")?,
            content: map_text(entries, "content")?,
            parent_tool_use_id: map_opt_text(entries, "parent_tool_use_id"),
        },
        "error_recorded" => E::ErrorRecorded {
            turn_id: map_u64(entries, "turn_id")?,
            message_id: map_text(entries, "message_id")?,
            content: map_text(entries, "content")?,
            parent_tool_use_id: map_opt_text(entries, "parent_tool_use_id"),
        },
        "tool_call_started" => E::ToolCallStarted {
            turn_id: map_u64(entries, "turn_id")?,
            tool_use_id: map_text(entries, "tool_use_id")?,
            tool: map_text(entries, "tool")?,
            input: JsonPayload::new_unchecked(map_text(entries, "input")?),
            parent_tool_use_id: map_opt_text(entries, "parent_tool_use_id"),
        },
        "tool_call_succeeded" => E::ToolCallSucceeded {
            turn_id: map_u64(entries, "turn_id")?,
            tool_use_id: map_text(entries, "tool_use_id")?,
            content: map_text(entries, "content")?,
            content_ref: match map_get(entries, "content_ref") {
                Some(value) => Some(decode_tool_output_ref(value)?),
                None => None,
            },
            summary: match map_get(entries, "summary") {
                Some(value) => Some(decode_tool_output_summary(value)?),
                None => None,
            },
        },
        "tool_call_failed" => E::ToolCallFailed {
            turn_id: map_u64(entries, "turn_id")?,
            tool_use_id: map_text(entries, "tool_use_id")?,
            content: map_text(entries, "content")?,
            content_ref: match map_get(entries, "content_ref") {
                Some(value) => Some(decode_tool_output_ref(value)?),
                None => None,
            },
            summary: match map_get(entries, "summary") {
                Some(value) => Some(decode_tool_output_summary(value)?),
                None => None,
            },
        },
        "tool_result_recorded" => E::ToolResultRecorded {
            turn_id: map_u64(entries, "turn_id")?,
            message_id: map_text(entries, "message_id")?,
            content: map_text(entries, "content")?,
            is_error: map_bool(entries, "is_error")?,
            content_ref: match map_get(entries, "content_ref") {
                Some(value) => Some(decode_tool_output_ref(value)?),
                None => None,
            },
            summary: match map_get(entries, "summary") {
                Some(value) => Some(decode_tool_output_summary(value)?),
                None => None,
            },
            tool_use_id: map_opt_text(entries, "tool_use_id"),
            parent_tool_use_id: map_opt_text(entries, "parent_tool_use_id"),
        },
        "tool_call_retried" => E::ToolCallRetried {
            turn_id: map_u64(entries, "turn_id")?,
            tool_use_id: map_text(entries, "tool_use_id")?,
            attempt: u32::try_from(map_u64(entries, "attempt")?).map_err(|_| malformed())?,
        },
        "permission_requested" => E::PermissionRequested {
            turn_id: map_u64(entries, "turn_id")?,
            tool_use_id: map_opt_text(entries, "tool_use_id"),
            request: decode_permission_request(map_get(entries, "request").ok_or_else(malformed)?)?,
        },
        "permission_resolved" => E::PermissionResolved {
            turn_id: map_u64(entries, "turn_id")?,
            tool_use_id: map_opt_text(entries, "tool_use_id"),
            request_id: map_opt_text(entries, "request_id"),
            decision: match map_text(entries, "decision")?.as_str() {
                "allowed" => PermissionDecision::Allowed,
                "denied" => PermissionDecision::Denied,
                "cancelled" => PermissionDecision::Cancelled,
                _ => return Err(malformed()),
            },
            answers: map_opt_json(entries, "answers"),
        },
        "task_status_changed" => E::TaskStatusChanged {
            turn_id: map_u64(entries, "turn_id")?,
            message_id: map_text(entries, "message_id")?,
            task_tool_use_id: map_text(entries, "task_tool_use_id")?,
            status: map_text(entries, "status")?,
            description: map_opt_text(entries, "description"),
            summary: map_opt_text(entries, "summary"),
        },
        "todo_list_snapshot_recorded" => E::TodoListSnapshotRecorded {
            turn_id: map_u64(entries, "turn_id")?,
            message_id: map_text(entries, "message_id")?,
            items: map_array(entries, "items")?
                .iter()
                .map(decode_todo_item)
                .collect::<Result<Vec<_>, EventCodecError>>()?,
        },
        "system_notification_recorded" => E::SystemNotificationRecorded {
            turn_id: map_u64(entries, "turn_id")?,
            message_id: map_text(entries, "message_id")?,
            notification_type: parse_notification_type(&map_text(entries, "notification_type")?)?,
            status: map_text(entries, "status")?,
            label: map_text(entries, "label")?,
            detail: map_opt_text(entries, "detail"),
            hook_id: map_opt_text(entries, "hook_id"),
        },
        "image_recorded" => E::ImageRecorded {
            turn_id: map_u64(entries, "turn_id")?,
            message_id: map_text(entries, "message_id")?,
            data: map_text(entries, "data")?,
            media_type: map_text(entries, "media_type")?,
        },
        "image_ref_recorded" => E::ImageRefRecorded {
            turn_id: map_u64(entries, "turn_id")?,
            message_id: map_text(entries, "message_id")?,
            attachment: decode_attachment(map_get(entries, "attachment").ok_or_else(malformed)?)?,
        },
        "final_parts_recorded" => E::FinalPartsRecorded {
            turn_id: map_u64(entries, "turn_id")?,
            message_id: map_text(entries, "message_id")?,
            parts: map_array(entries, "parts")?
                .iter()
                .map(decode_message_part)
                .collect::<Result<Vec<_>, EventCodecError>>()?,
        },
        "turn_completed" => E::TurnCompleted {
            turn_id: map_u64(entries, "turn_id")?,
            exit_code: map_i64(entries, "exit_code")?,
            stop_reason: match map_opt_text(entries, "stop_reason").as_deref() {
                Some("refusal") => Some(TurnStopReason::Refusal),
                Some(_) => return Err(malformed()),
                None => None,
            },
            token_usage: match map_get(entries, "token_usage") {
                Some(value) => {
                    let usage_entries = as_map(value)?;
                    Some(TurnTokenUsage {
                        input_tokens: map_u64(usage_entries, "input_tokens")?,
                        output_tokens: map_u64(usage_entries, "output_tokens")?,
                    })
                }
                None => None,
            },
        },
        "turn_interrupted" => E::TurnInterrupted {
            turn_id: map_u64(entries, "turn_id")?,
            reason: parse_interrupt_reason(&map_text(entries, "reason")?)?,
            exit_code: map_i64(entries, "exit_code")?,
            error: map_opt_text(entries, "error"),
        },
        "session_errored" => E::SessionErrored {
            message_id: map_text(entries, "message_id")?,
            reason: map_text(entries, "reason")?,
            at: map_f64(entries, "at")?,
        },
        "session_closed" => E::SessionClosed {
            at: map_f64(entries, "at")?,
        },
        "send_operation_accepted" => E::SendOperationAccepted {
            operation_id: map_text(entries, "operation_id")?,
            disposition: decode_send_disposition(
                map_get(entries, "disposition").ok_or_else(malformed)?,
            )?,
            human_message_id: map_opt_text(entries, "human_message_id"),
            prompt: map_get(entries, "prompt")
                .map(decode_prompt_input)
                .transpose()?,
            reserved_turn_id: map_opt_text(entries, "reserved_turn_id"),
            at: map_f64(entries, "at")?,
        },
        "stop_operation_accepted" => E::StopOperationAccepted {
            operation_id: map_text(entries, "operation_id")?,
            target_turn_id: map_u64(entries, "target_turn_id")?,
            at: map_f64(entries, "at")?,
        },
        "session_lifecycle_operation_accepted" => E::SessionLifecycleOperationAccepted {
            operation_id: map_text(entries, "operation_id")?,
            kind: parse_lifecycle_kind(&map_text(entries, "lifecycle_kind")?)?,
            at: map_f64(entries, "at")?,
        },
        "obligation_recorded" => E::ObligationRecorded {
            obligation_id: map_text(entries, "obligation_id")?,
            kind: parse_obligation_kind(&map_text(entries, "obligation_kind")?)?,
            state: parse_obligation_state(&map_text(entries, "state")?)?,
            at: map_f64(entries, "at")?,
        },
        "stop_resolution_recorded" => E::StopResolutionRecorded {
            operation_id: map_text(entries, "operation_id")?,
            turn_id: map_u64(entries, "turn_id")?,
            resolution: parse_stop_resolution(&map_text(entries, "resolution")?)?,
            at: map_f64(entries, "at")?,
        },
        "pending_recovery_published" => E::PendingRecoveryPublished {
            obligation_id: map_text(entries, "obligation_id")?,
            kind: parse_obligation_kind(&map_text(entries, "obligation_kind")?)?,
            at: map_f64(entries, "at")?,
        },
        "recovery_action_resolved" => E::RecoveryActionResolved {
            action_id: map_text(entries, "action_id")?,
            obligation_id: map_text(entries, "obligation_id")?,
            kind: parse_recovery_action_kind(&map_text(entries, "action_kind")?)?,
            classification: parse_classification(&map_text(entries, "classification")?)?,
            at: map_f64(entries, "at")?,
        },
        // Unknown kinds within a known payload version are malformed; new
        // kinds must bump the payload version instead.
        _ => return Ok(None),
    };
    Ok(Some(event))
}

impl LocalEventPayloadCodec for AgentSessionEventCodec {
    fn event_type(&self) -> &'static str {
        AGENT_SESSION_EVENT_TYPE
    }

    fn payload_version(&self) -> i64 {
        AGENT_SESSION_PAYLOAD_VERSION
    }

    fn handles(&self, event: &LocalDomainEvent) -> bool {
        matches!(event, LocalDomainEvent::AgentSession(_))
    }

    fn encode(&self, event: &LocalDomainEvent) -> Result<CborValue, EventCodecError> {
        let LocalDomainEvent::AgentSession(event) = event else {
            return Err(EventCodecError::UnregisteredEvent {
                description: "non-agent-session event given to agent-session codec".to_string(),
            });
        };
        encode_event(event)
    }

    fn decode(
        &self,
        payload_version: i64,
        value: &CborValue,
    ) -> Result<Option<LocalDomainEvent>, EventCodecError> {
        if payload_version != AGENT_SESSION_PAYLOAD_VERSION {
            return Ok(None);
        }
        Ok(decode_event(value)?.map(LocalDomainEvent::AgentSession))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adaptor::gateway::local_event_store::envelope::{
        DecodedStoredEvent, EventCodecRegistry,
    };

    fn sample_permission_request() -> PermissionRequest {
        PermissionRequest {
            id: "perm-1".to_string(),
            tool_use_id: Some("tool-1".to_string()),
            parent_tool_use_id: None,
            tool_name: "Bash".to_string(),
            body: PermissionRequestBody::Question {
                questions: vec![PermissionQuestion {
                    question: "Run this?".to_string(),
                    header: Some("Confirm".to_string()),
                    options: vec![PermissionQuestionOption {
                        label: "Yes".to_string(),
                        description: None,
                    }],
                    multi_select: false,
                }],
            },
            title: None,
            display_name: Some("Bash".to_string()),
            description: None,
            decision_reason: None,
            status: PermissionRequestStatus::Resolved {
                decision: EntityPermissionDecision::Allowed,
                answers: Some(JsonPayload::new_unchecked("{\"a\":1}".to_string())),
            },
        }
    }

    fn representative_events() -> Vec<AgentSessionDomainEvent> {
        use AgentSessionDomainEvent as E;
        vec![
            E::BackendSessionRecoveryStarted {
                recovery_id: "rec-1".to_string(),
                old_provider_session_generation: 3,
                reason: BackendSessionRecoveryReason::ResumeMismatch,
                at: 1700000000123.5,
            },
            E::SessionGoalReactivated {
                recovery_id: "rec-1".to_string(),
                outcome: GoalReactivationOutcome::Restored {
                    goal_id: "goal-1".to_string(),
                    goal_revision: 2,
                    provider_goal_ref: None,
                },
                provider_session_generation: 4,
                restoring_turn_id: Some("7".to_string()),
                consumed_observation_id: None,
                at: 2.0,
            },
            E::TurnStarted {
                turn_id: 7,
                message_id: "m-1".to_string(),
                assistant_message_id: Some("m-2".to_string()),
                prompt: PromptInput {
                    content: "hello".to_string(),
                    mentions: vec![MentionReference {
                        file_path: "src/main.rs".to_string(),
                        start_line: Some(1),
                        end_line: None,
                    }],
                    attachment_refs: vec![Attachment {
                        id: "att-1".to_string(),
                        media_type: "image/png".to_string(),
                        byte_size: 128,
                    }],
                    parts: vec![MessagePart::Text {
                        content: "hello".to_string(),
                        parent_tool_use_id: None,
                    }],
                },
                at: 3.25,
            },
            E::QueuePaused { at: 4.0 },
            E::QueueResumed {
                expected_paused_at: 4.0,
                at: 5.0,
            },
            E::ToolCallStarted {
                turn_id: 7,
                tool_use_id: "tool-1".to_string(),
                tool: "Bash".to_string(),
                input: JsonPayload::new_unchecked("{\"command\":\"ls\"}".to_string()),
                parent_tool_use_id: None,
            },
            E::ToolResultRecorded {
                turn_id: 7,
                message_id: "m-2".to_string(),
                content: "ok".to_string(),
                is_error: false,
                content_ref: Some(ToolOutputRef {
                    id: "out-1".to_string(),
                    byte_size: 2,
                }),
                summary: Some(ToolOutputSummary {
                    line_count: 1,
                    byte_size: 2,
                    is_error: false,
                    truncated: false,
                }),
                tool_use_id: Some("tool-1".to_string()),
                parent_tool_use_id: None,
            },
            E::PermissionRequested {
                turn_id: 7,
                tool_use_id: Some("tool-1".to_string()),
                request: sample_permission_request(),
            },
            E::PermissionResolved {
                turn_id: 7,
                tool_use_id: Some("tool-1".to_string()),
                request_id: Some("perm-1".to_string()),
                decision: PermissionDecision::Allowed,
                answers: Some(JsonPayload::new_unchecked("{\"a\":1}".to_string())),
            },
            E::FinalPartsRecorded {
                turn_id: 7,
                message_id: "m-2".to_string(),
                parts: vec![
                    MessagePart::Permission {
                        request: sample_permission_request(),
                        status: PermissionPartStatus::Allowed,
                        answers: Some(JsonPayload::new_unchecked("{\"a\":1}".to_string())),
                        parent_tool_use_id: None,
                    },
                    MessagePart::TodoListSnapshot {
                        items: vec![TodoListItem {
                            text: "step".to_string(),
                            completed: true,
                        }],
                    },
                    MessagePart::SystemNotification {
                        notification_type: SystemNotificationType::Compaction,
                        status: "done".to_string(),
                        label: "Compacted".to_string(),
                        detail: None,
                        hook_id: None,
                    },
                    MessagePart::ImageRef {
                        attachment: Attachment {
                            id: "att-1".to_string(),
                            media_type: "image/png".to_string(),
                            byte_size: 128,
                        },
                    },
                ],
            },
            E::TurnCompleted {
                turn_id: 7,
                exit_code: 0,
                stop_reason: Some(TurnStopReason::Refusal),
                token_usage: Some(TurnTokenUsage {
                    input_tokens: 10,
                    output_tokens: 20,
                }),
            },
            E::TurnInterrupted {
                turn_id: 8,
                reason: InterruptReason::Timeout,
                exit_code: -1,
                error: Some("timed out".to_string()),
            },
            E::SessionClosed { at: 6.0 },
            E::SendOperationAccepted {
                operation_id: "op-1".to_string(),
                disposition: SendDisposition::Queued {
                    queue_item_id: "q-1".to_string(),
                },
                human_message_id: None,
                prompt: None,
                reserved_turn_id: None,
                at: 7.0,
            },
            E::StopOperationAccepted {
                operation_id: "stop-1".to_string(),
                target_turn_id: 7,
                at: 8.0,
            },
            E::SessionLifecycleOperationAccepted {
                operation_id: "slc-1".to_string(),
                kind: SessionLifecycleKind::Archive,
                at: 9.0,
            },
            E::ObligationRecorded {
                obligation_id: "ob-1".to_string(),
                kind: ObligationKind::ProviderEstablish,
                state: ObligationState::EffectReserved,
                at: 10.0,
            },
            E::StopResolutionRecorded {
                operation_id: "stop-1".to_string(),
                turn_id: 7,
                resolution: StopResolution::Superseded,
                at: 11.0,
            },
            E::PendingRecoveryPublished {
                obligation_id: "ob-1".to_string(),
                kind: ObligationKind::TurnExecution,
                at: 12.0,
            },
            E::RecoveryActionResolved {
                action_id: "act-1".to_string(),
                obligation_id: "ob-1".to_string(),
                kind: RecoveryActionKind::RetrySameEffect,
                classification: RecoveryResultClassification::Succeeded,
                at: 13.0,
            },
        ]
    }

    #[test]
    fn agent_session_events_round_trip_canonically() {
        let registry = EventCodecRegistry::new();
        for event in representative_events() {
            let domain = LocalDomainEvent::AgentSession(event);
            let encoded = registry.encode(&domain).expect("encode");
            assert_eq!(encoded.event_type, AGENT_SESSION_EVENT_TYPE);
            assert_eq!(encoded.payload_version, AGENT_SESSION_PAYLOAD_VERSION);
            let decoded = registry
                .decode(
                    &encoded.event_type,
                    encoded.payload_version,
                    &encoded.payload,
                )
                .expect("decode");
            assert_eq!(decoded, DecodedStoredEvent::Known(Box::new(domain)));
        }
    }

    #[test]
    fn unknown_payload_version_is_preserved_raw() {
        let registry = EventCodecRegistry::new();
        let domain =
            LocalDomainEvent::AgentSession(AgentSessionDomainEvent::SessionClosed { at: 1.0 });
        let encoded = registry.encode(&domain).unwrap();
        assert_eq!(
            registry
                .decode(&encoded.event_type, 999, &encoded.payload)
                .unwrap(),
            DecodedStoredEvent::Unknown
        );
    }

    #[test]
    fn timestamps_round_trip_losslessly_through_decimal_text() {
        let registry = EventCodecRegistry::new();
        for at in [0.0, 0.5, 1700000000123.456, f64::from(i32::MAX)] {
            let domain =
                LocalDomainEvent::AgentSession(AgentSessionDomainEvent::SessionClosed { at });
            let encoded = registry.encode(&domain).unwrap();
            let decoded = registry
                .decode(
                    &encoded.event_type,
                    encoded.payload_version,
                    &encoded.payload,
                )
                .unwrap();
            assert_eq!(decoded, DecodedStoredEvent::Known(Box::new(domain)));
        }
    }

    #[test]
    fn non_finite_timestamps_fail_closed_before_persistence() {
        let registry = EventCodecRegistry::new();
        for at in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let domain =
                LocalDomainEvent::AgentSession(AgentSessionDomainEvent::SessionClosed { at });
            assert!(matches!(
                registry.encode(&domain),
                Err(EventCodecError::MalformedPayload { .. })
            ));
        }
    }
}
