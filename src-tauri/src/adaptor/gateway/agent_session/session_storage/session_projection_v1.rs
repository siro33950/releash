//! Strict SQLite read-model envelope for an agent session.
//!
//! This is deliberately distinct from every legacy JSON document.  The
//! semantic values cross the usecase-owned codec port and only this gateway
//! knows the persisted V1 shape.

use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::domain::agent_session::aggregates::backend_recovery_projection::BackendRecoveryProjection;
use crate::domain::agent_session::aggregates::session::{
    QueueItem, QueueState, Session, SessionRestore,
};
use crate::domain::agent_session::entities::TokenUsage as DomainTokenUsage;
use crate::domain::agent_session::services::{
    classify_recovery_fact, detect_image_mime, DefaultToolOutputExternalizationPolicy,
    RecoveryPublicationDecision, ToolOutputExternalizationPolicy,
};
use crate::domain::agent_session::value_objects::{ContextRevision, JsonPayload};
use crate::domain::local_event::{
    AgentContentBlobRecord, AgentContextCarryStateRecord, AgentContextEpochRecord,
    AgentContextSourceRecord, AgentMessageActivityRecord, AgentMessageProjectionRecord,
    AgentMessageRoleRecord, AgentPendingRecoveryMessageRecord, AgentQueuedSendRecord,
    AgentRecoveryPublicationClassificationRecord, AgentRecoveryPublicationListRecord,
    AgentRecoveryPublicationSnapshotRecord, AgentRecoveryPublicationWorkflowOwnerRecord,
    AgentSessionMetadataRecord, AgentSessionProjectionRecord, AgentSessionStateRecord,
    AgentSessionSummaryRecord, AgentTurnInterruptionRecord, MessageProjectionRecord,
    SessionProjectionRecord, ValidatedPendingWorkflowTurnCompletion,
};
use crate::usecase::agent_session::context_meta::{
    context_source_kind_from_key, context_source_kind_key, ContextEpochMeta,
    ContextSourcePayloadCache, ContextSourceRevisionMeta,
};
use crate::usecase::agent_session::event_log::{AgentTurnFailureSignal, WorkflowTurnCompleteInput};
use crate::usecase::agent_session::session::{
    session_summary_from_record, workflow_node_context_mapper, ActivityEntry,
    AgentSessionProjectionCodec, AttachmentRef, CanonicalAgentSessionProjection,
    CanonicalContentBlob, CanonicalQueuedSend, ChatMessage, ContextCarryState,
    EventProjectionMetaPatch, MessageMention, MessagePart, MessageRole, PendingRecoveryMessage,
    RecoveryPublicationClassification, RecoveryPublicationList, RecoveryPublicationSnapshot,
    RecoveryPublicationWorkflowOwner, SessionMeta, SessionState, SessionSummary,
    TerminalMessageProjectionPatch, TokenUsage, ToolOutputRef, ToolOutputSummary, TurnInterruption,
    TurnInterruptionReason,
};

use super::stored_event_v1::{decode_agent_session_events_v1, encode_agent_session_events_v1};
use super::stored_message_part_v1::{encode_stored_message_parts_v1, StoredPayloadSource};
use super::stored_session_v1::{decode_chat_message_v1, encode_chat_message_v1};

const SCHEMA: &str = "agent_session_projection_v1";

#[derive(Debug, Serialize, Deserialize)]
struct StoredAgentSessionProjectionV1 {
    schema: String,
    meta: SessionMeta,
    #[serde(default)]
    title: Option<String>,
    workflow_instructions: Vec<String>,
    agent_read_paths: Option<Vec<std::path::PathBuf>>,
    #[serde(default)]
    context_epoch_payloads: Vec<ContextSourcePayloadCache>,
    #[serde(default)]
    reducer_events: Vec<serde_json::Value>,
    queue_paused_at: Option<f64>,
    latest_token_usage: Option<TokenUsage>,
    #[serde(default)]
    pending_send_queue: Vec<StoredCanonicalQueuedSendV1>,
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredCanonicalQueuedSendV1 {
    queue_item_id: String,
    human_message_id: String,
    reserved_turn_id: String,
    input_ref: String,
}

#[derive(Debug, Default)]
pub(crate) struct AgentSessionProjectionCodecV1;

impl AgentSessionProjectionCodec for AgentSessionProjectionCodecV1 {
    fn encode(
        &self,
        projection: &CanonicalAgentSessionProjection,
    ) -> Result<SessionProjectionRecord, String> {
        Ok(SessionProjectionRecord::AgentSession(Box::new(
            agent_projection_record_from_canonical(projection)?,
        )))
    }

    fn decode(
        &self,
        payload: &SessionProjectionRecord,
    ) -> Result<CanonicalAgentSessionProjection, String> {
        let SessionProjectionRecord::AgentSession(projection) = payload else {
            return Err("projection is not an agent session".to_string());
        };
        canonical_from_agent_projection_record(projection)
    }

    fn restore_session_aggregate(
        &self,
        projection: &CanonicalAgentSessionProjection,
        pending_obligations: &[(String, crate::domain::local_event::ObligationRecord)],
    ) -> Result<Session, String> {
        let state = projection.meta.state;
        let current_turn =
            Session::current_turn_from_events(&projection.reducer_events, state.is_closed());
        let queue = projection
            .pending_send_queue
            .iter()
            .map(|item| QueueItem {
                id: item.queue_item_id.clone(),
                operation_id: item.input_ref.clone(),
                reserved_turn_id: Some(item.reserved_turn_id.clone()),
                human_message_id: Some(item.human_message_id.clone()),
            })
            .collect();
        Session::restore(SessionRestore {
            id: projection.meta.id.clone(),
            revision: projection.meta.state_revision,
            state,
            has_messages: projection.meta.message_count != 0,
            has_provider_session: projection.meta.agent_session_id.is_some(),
            current_turn,
            last_terminal: None,
            queue: QueueState::restore(queue, projection.queue_paused_at.is_some()),
            recovery_fact: classify_recovery_fact(
                projection.meta.pending_recovery_message.is_some(),
                pending_obligations
                    .iter()
                    .map(|(identity, record)| (identity.as_str(), record)),
            ),
        })
        .map_err(|error| format!("invalid canonical Session aggregate: {error:?}"))
    }

    fn encode_message(&self, message: &ChatMessage) -> Result<MessageProjectionRecord, String> {
        Ok(MessageProjectionRecord::AgentMessage(
            agent_message_record_from_chat_message(message)?,
        ))
    }

    fn decode_message(&self, payload: &MessageProjectionRecord) -> Result<ChatMessage, String> {
        let MessageProjectionRecord::AgentMessage(message) = payload else {
            return Err("projection is not an agent message".to_string());
        };
        chat_message_from_agent_message_record(message)
    }

    fn externalize_message_content(
        &self,
        messages: &mut [ChatMessage],
    ) -> Result<Vec<CanonicalContentBlob>, String> {
        let mut blobs = Vec::new();
        let tool_output_policy = DefaultToolOutputExternalizationPolicy;
        for message in messages {
            let Some(parts) = message.parts.as_mut() else {
                continue;
            };
            for part in parts {
                match part {
                    MessagePart::Image { data, media_type } => {
                        let data = data.clone();
                        let media_type = media_type.clone();
                        let bytes = base64::engine::general_purpose::STANDARD
                            .decode(data.as_bytes())
                            .map_err(|_| "canonical attachment is not valid base64".to_string())?;
                        let detected = detect_image_mime(&bytes).ok_or_else(|| {
                            "canonical attachment is not a supported image".to_string()
                        })?;
                        if detected != media_type.as_str() {
                            return Err(
                                "canonical attachment media type does not match bytes".to_string()
                            );
                        }
                        let mut hasher = Sha256::new();
                        hasher.update(media_type.as_bytes());
                        hasher.update([0]);
                        hasher.update(&bytes);
                        let id = hex::encode(hasher.finalize());
                        let byte_size = bytes.len() as u64;
                        blobs.push(CanonicalContentBlob {
                            identity: format!("attachment:{id}"),
                            projection: AgentContentBlobRecord::Attachment {
                                id: id.clone(),
                                media_type: media_type.clone(),
                                bytes,
                            },
                        });
                        *part = MessagePart::ImageRef {
                            attachment: AttachmentRef {
                                id,
                                media_type,
                                byte_size,
                            },
                        };
                    }
                    MessagePart::ToolResult {
                        content,
                        is_error,
                        tool_use_id,
                        parent_tool_use_id,
                        content_ref,
                        summary,
                    } if content_ref.is_none()
                        && tool_output_policy.should_externalize_tool_output(content) =>
                    {
                        let content = content.clone();
                        let id = hex::encode(Sha256::digest(content.as_bytes()));
                        blobs.push(CanonicalContentBlob {
                            identity: format!("tool_output:{id}"),
                            projection: AgentContentBlobRecord::ToolOutput {
                                id: id.clone(),
                                content: content.clone(),
                            },
                        });
                        let projected_summary = summary.clone().unwrap_or_else(|| {
                            let summary =
                                tool_output_policy.tool_output_summary(&content, *is_error, true);
                            ToolOutputSummary {
                                line_count: summary.line_count,
                                byte_size: summary.byte_size,
                                is_error: summary.is_error,
                                truncated: summary.truncated,
                            }
                        });
                        *part = MessagePart::ToolResult {
                            content: tool_output_policy.tool_output_preview(&content),
                            is_error: *is_error,
                            tool_use_id: tool_use_id.clone(),
                            parent_tool_use_id: parent_tool_use_id.clone(),
                            content_ref: Some(ToolOutputRef {
                                id,
                                byte_size: content.len() as u64,
                            }),
                            summary: Some(projected_summary),
                        };
                    }
                    _ => {}
                }
            }
        }
        Ok(blobs)
    }

    fn backend_recovery_from_projection(
        &self,
        projection: &CanonicalAgentSessionProjection,
    ) -> BackendRecoveryProjection {
        backend_recovery_from_meta(&projection.meta, projection.queue_paused_at.is_some())
    }

    fn backend_recovery_from_meta(
        &self,
        meta: &SessionMeta,
        queue_paused: bool,
    ) -> BackendRecoveryProjection {
        backend_recovery_from_meta(meta, queue_paused)
    }

    fn apply_backend_recovery_to_projection(
        &self,
        projection: &mut CanonicalAgentSessionProjection,
        state: BackendRecoveryProjection,
    ) {
        apply_backend_recovery_to_meta(&mut projection.meta, state);
    }

    fn apply_backend_recovery_to_meta(
        &self,
        meta: &mut SessionMeta,
        state: BackendRecoveryProjection,
    ) {
        apply_backend_recovery_to_meta(meta, state);
    }

    fn recovery_publication_snapshot(
        &self,
        recovery_id: &str,
        meta: &SessionMeta,
        decision: RecoveryPublicationDecision,
    ) -> RecoveryPublicationSnapshot {
        let list = match decision.list {
            crate::domain::agent_session::services::RecoveryPublicationListDecision::Sessions => {
                RecoveryPublicationList::SessionList
            }
            crate::domain::agent_session::services::RecoveryPublicationListDecision::ClosedHistory => {
                RecoveryPublicationList::ClosedHistory
            }
            crate::domain::agent_session::services::RecoveryPublicationListDecision::ArchivedHistory => {
                RecoveryPublicationList::ArchivedHistory
            }
        };
        let workflow_owner =
            meta.is_workflow_node_session()
                .then(|| RecoveryPublicationWorkflowOwner {
                    execution_id: meta
                        .workflow_node_context
                        .as_ref()
                        .map(|context| context.execution_id.clone()),
                    node_execution_id: meta
                        .workflow_node_context
                        .as_ref()
                        .map(|context| context.node_execution_id.clone()),
                });
        let mut summary = meta.to_summary();
        summary.state = decision.published_state;
        RecoveryPublicationSnapshot {
            recovery_id: recovery_id.to_string(),
            summary,
            classification: RecoveryPublicationClassification {
                list,
                workflow_owner,
            },
        }
    }

    fn recovery_publication_message_record(
        &self,
        message: &PendingRecoveryMessage,
    ) -> crate::domain::local_event::RecoveryPublicationMessageRecord {
        match message {
            PendingRecoveryMessage::Notice {
                recovery_id,
                message_id,
            } => crate::domain::local_event::RecoveryPublicationMessageRecord {
                kind: crate::domain::local_event::RecoveryPublicationMessageKindRecord::Notice,
                recovery_id: recovery_id.clone(),
                message_id: message_id.clone(),
                error: None,
            },
            PendingRecoveryMessage::Error {
                recovery_id,
                message_id,
                error,
            } => crate::domain::local_event::RecoveryPublicationMessageRecord {
                kind: crate::domain::local_event::RecoveryPublicationMessageKindRecord::Error,
                recovery_id: recovery_id.clone(),
                message_id: message_id.clone(),
                error: Some(error.clone()),
            },
        }
    }

    fn workflow_context(
        &self,
        context: &crate::usecase::agent_session::session::WorkflowNodeContextDto,
    ) -> crate::domain::workflow::WorkflowNodeContext {
        workflow_node_context_mapper::to_domain(context.clone())
    }

    fn workflow_failure_signal(
        &self,
        signal: Option<AgentTurnFailureSignal>,
    ) -> Option<crate::domain::local_event::WorkflowTurnFailureSignalRecord> {
        signal.map(|signal| match signal {
            AgentTurnFailureSignal::ModelRefusal => {
                crate::domain::local_event::WorkflowTurnFailureSignalRecord::ModelRefusal
            }
        })
    }

    fn workflow_turn_complete_input(
        &self,
        pending: &ValidatedPendingWorkflowTurnCompletion,
        final_text_parts: Vec<String>,
    ) -> WorkflowTurnCompleteInput {
        WorkflowTurnCompleteInput {
            turn_id: pending.turn_id,
            exit_code: pending.exit_code,
            final_text_parts,
            failure_signal: pending.failure_signal.map(|signal| match signal {
                crate::domain::local_event::WorkflowTurnFailureSignalRecord::ModelRefusal => {
                    AgentTurnFailureSignal::ModelRefusal
                }
            }),
            token_usage: pending.token_usage,
            interrupted: pending.interrupted,
        }
    }

    fn workflow_final_text_parts(
        &self,
        message: &ChatMessage,
        expected_message_id: &str,
    ) -> Result<Vec<String>, String> {
        if message.id != expected_message_id || message.role != MessageRole::Agent {
            return Err("workflow turn-completion message projection is inconsistent".to_string());
        }
        Ok(message
            .parts
            .as_deref()
            .unwrap_or_default()
            .iter()
            .filter_map(|part| match part {
                MessagePart::Text { content, .. } => Some(content.clone()),
                _ => None,
            })
            .collect())
    }

    fn context_restore_completion_facts(
        &self,
        meta: &SessionMeta,
    ) -> crate::domain::agent_session::services::ContextRestoreCompletionFacts {
        crate::domain::agent_session::services::ContextRestoreCompletionFacts {
            session_state: meta.state,
            pending_recovery_failure: matches!(
                meta.pending_recovery_message,
                Some(PendingRecoveryMessage::Error { .. })
            ),
            has_recovery_publication_snapshot: meta.recovery_publication_snapshot.is_some(),
            provider_session_generation: meta.provider_session_generation,
            context_reinjection_generation: meta.context_reinjection_generation,
            last_turn_id: meta.last_turn_id,
            backend_recovery_observation: meta
                .provider_session_observation_id
                .as_deref()
                .is_some_and(|identity| identity.starts_with("backend-recovery/v1:")),
            has_pending_recovery_message: meta.pending_recovery_message.is_some(),
            context_carry: meta.context_carry,
        }
    }

    fn apply_context_restore_completion_decision(
        &self,
        meta: &mut SessionMeta,
        decision: crate::domain::agent_session::services::ContextRestoreCompletionDecision,
        at: f64,
    ) {
        if decision.clear_context_reinjection_generation {
            meta.context_reinjection_generation = None;
        }
        if let crate::domain::agent_session::services::ContextCarryChange::Replace(context_carry) =
            decision.context_carry
        {
            meta.context_carry = context_carry;
        }
        meta.updated_at = at;
    }

    fn encode_session_identity_v1(
        &self,
        payload: &SessionProjectionRecord,
    ) -> Result<Vec<u8>, String> {
        let SessionProjectionRecord::AgentSession(projection) = payload else {
            return Err("projection is not an agent session".to_string());
        };
        Ok(encode_agent_session_projection_record_v1(projection)?.into_bytes())
    }

    fn encode_message_identity_v1(
        &self,
        payload: &MessageProjectionRecord,
    ) -> Result<Vec<u8>, String> {
        let MessageProjectionRecord::AgentMessage(message) = payload else {
            return Err("projection is not an agent message".to_string());
        };
        Ok(encode_agent_message_projection_record_v1(message)?.into_bytes())
    }

    fn encode_events_for_identity(
        &self,
        events: &[crate::usecase::agent_session::event_log::AgentSessionEvent],
    ) -> Result<Vec<u8>, String> {
        encode_agent_session_events_v1(events, false)
            .map_err(|error| format!("agent identity event encode failed: {error}"))
    }

    fn encode_parts_for_identity(
        &self,
        parts: &[crate::usecase::agent_session::session::MessagePart],
    ) -> Result<Vec<u8>, String> {
        encode_stored_message_parts_v1(parts)
            .map_err(|error| format!("agent identity parts encode failed: {error}"))
    }

    fn hash_terminal_message_projection_patch(
        &self,
        identity: &mut crate::domain::local_event::DurableIdentityBuilder,
        patch: &TerminalMessageProjectionPatch,
    ) -> Result<(), String> {
        let encoded_parts = patch
            .parts
            .as_deref()
            .map(|parts| self.encode_parts_for_identity(parts))
            .transpose()?;
        crate::domain::local_event::hash_terminal_message_projection_patch(
            identity,
            &patch.message_id,
            patch.streaming_final_seq,
            patch.timestamp.map(f64::to_bits),
            encoded_parts.as_deref(),
        );
        Ok(())
    }

    fn hash_event_projection_meta_patch(
        &self,
        identity: &mut crate::domain::local_event::DurableIdentityBuilder,
        patch: &EventProjectionMetaPatch,
    ) -> Result<(), String> {
        use crate::domain::local_event::EventProjectionMetaIdentityFacts;

        let encoded_snapshot;
        let facts = match patch {
            EventProjectionMetaPatch::Started {
                expected_generation,
                publication_snapshot,
                at,
            } => {
                encoded_snapshot = serde_json::to_vec(publication_snapshot).map_err(|error| {
                    format!("recovery publication snapshot encode failed: {error}")
                })?;
                EventProjectionMetaIdentityFacts::RecoveryStarted {
                    expected_generation: *expected_generation,
                    publication_snapshot: &encoded_snapshot,
                    at_bits: at.to_bits(),
                }
            }
            EventProjectionMetaPatch::Completed {
                expected_generation,
                provider_session_generation,
                backend_session_id,
                pending_recovery_message,
                at,
            } => EventProjectionMetaIdentityFacts::RecoveryCompleted {
                expected_generation: *expected_generation,
                provider_session_generation: *provider_session_generation,
                backend_session_id,
                pending_message: recovery_message_identity(pending_recovery_message),
                at_bits: at.to_bits(),
            },
            EventProjectionMetaPatch::ReadbackCompleted {
                old_provider_session_generation,
                provider_session_generation,
                backend_session_id,
                pending_recovery_message,
                at,
            } => EventProjectionMetaIdentityFacts::RecoveryReadbackCompleted {
                old_provider_session_generation: *old_provider_session_generation,
                provider_session_generation: *provider_session_generation,
                backend_session_id,
                pending_message: recovery_message_identity(pending_recovery_message),
                at_bits: at.to_bits(),
            },
            EventProjectionMetaPatch::Failed {
                pending_recovery_message,
                at,
            } => EventProjectionMetaIdentityFacts::RecoveryFailed {
                pending_message: recovery_message_identity(pending_recovery_message),
                at_bits: at.to_bits(),
            },
            EventProjectionMetaPatch::ContextRestoreCompleted {
                expected_provider_session_generation,
                expected_turn_id,
                reinjected,
                clear_context_carry,
                recovery_restore_required,
                at,
            } => EventProjectionMetaIdentityFacts::ContextRestoreCompleted {
                expected_provider_session_generation: *expected_provider_session_generation,
                expected_turn_id: *expected_turn_id,
                reinjected: *reinjected,
                clear_context_carry: *clear_context_carry,
                recovery_restore_required: *recovery_restore_required,
                at_bits: at.to_bits(),
            },
        };
        crate::domain::local_event::hash_event_projection_meta_patch(identity, facts);
        Ok(())
    }
}

fn recovery_message_identity(
    message: &PendingRecoveryMessage,
) -> crate::domain::local_event::RecoveryPublicationMessageIdentityFacts<'_> {
    match message {
        PendingRecoveryMessage::Notice {
            recovery_id,
            message_id,
        } => crate::domain::local_event::RecoveryPublicationMessageIdentityFacts::Notice {
            recovery_id,
            message_id,
        },
        PendingRecoveryMessage::Error {
            recovery_id,
            message_id,
            error,
        } => crate::domain::local_event::RecoveryPublicationMessageIdentityFacts::Error {
            recovery_id,
            message_id,
            error,
        },
    }
}

fn backend_recovery_from_meta(meta: &SessionMeta, queue_paused: bool) -> BackendRecoveryProjection {
    BackendRecoveryProjection {
        session_state: meta.state,
        error_reason: meta.error_reason.clone(),
        queue_paused,
        provider_session_id: meta.agent_session_id.clone(),
        provider_session_generation: meta.provider_session_generation,
        provider_session_observation_id: meta.provider_session_observation_id.clone(),
        context_reinjection_generation: meta.context_reinjection_generation,
        context_carry: meta.context_carry,
        has_recovery_publication_snapshot: meta.recovery_publication_snapshot.is_some(),
        has_pending_recovery_message: meta.pending_recovery_message.is_some(),
        pending_recovery_failure: matches!(
            meta.pending_recovery_message,
            Some(PendingRecoveryMessage::Error { .. })
        ),
    }
}

fn apply_backend_recovery_to_meta(meta: &mut SessionMeta, state: BackendRecoveryProjection) {
    meta.state = state.session_state;
    meta.error_reason = state.error_reason;
    meta.agent_session_id = state.provider_session_id;
    meta.provider_session_generation = state.provider_session_generation;
    meta.provider_session_observation_id = state.provider_session_observation_id;
    meta.context_reinjection_generation = state.context_reinjection_generation;
    meta.context_carry = state.context_carry;
}

pub(crate) fn encode_agent_session_projection_record_v1(
    projection: &AgentSessionProjectionRecord,
) -> Result<String, String> {
    let canonical = canonical_from_agent_projection_record(projection)?;
    serde_json::to_string(&stored_projection_from_canonical(&canonical)?)
        .map_err(|error| format!("agent projection encode failed: {error}"))
}

pub(crate) fn decode_agent_session_projection_record_v1(
    raw: &str,
) -> Result<AgentSessionProjectionRecord, String> {
    let stored: StoredAgentSessionProjectionV1 = serde_json::from_str(raw)
        .map_err(|error| format!("agent projection decode failed: {error}"))?;
    let canonical = canonical_projection_from_stored(stored)?;
    agent_projection_record_from_canonical(&canonical)
}

pub(crate) fn encode_agent_message_projection_record_v1(
    message: &AgentMessageProjectionRecord,
) -> Result<String, String> {
    let message = chat_message_from_agent_message_record(message)?;
    let encoded = encode_chat_message_v1(&message)
        .map_err(|error| format!("agent message projection encode failed: {error}"))?;
    String::from_utf8(encoded).map_err(|_| "agent message projection is not UTF-8".to_string())
}

pub(crate) fn decode_agent_message_projection_record_v1(
    raw: &str,
) -> Result<AgentMessageProjectionRecord, String> {
    let message = decode_chat_message_v1(
        raw.as_bytes(),
        StoredPayloadSource {
            source_id: "sqlite-message-projection".to_string(),
            record_ordinal: None,
        },
    )
    .map(|decoded| decoded.message)
    .map_err(|error| error.to_string())?;
    agent_message_record_from_chat_message(&message)
}

pub(crate) fn encode_agent_content_blob_record_v1(
    blob: &AgentContentBlobRecord,
) -> Result<String, String> {
    let value = match blob {
        AgentContentBlobRecord::Attachment {
            id,
            media_type,
            bytes,
        } => serde_json::json!({
            "schema": "agent_content_blob_v1",
            "kind": "attachment",
            "id": id,
            "media_type": media_type,
            "byte_size": bytes.len().to_string(),
            "data_base64": base64::engine::general_purpose::STANDARD.encode(bytes),
            "content": null,
        }),
        AgentContentBlobRecord::ToolOutput { id, content } => serde_json::json!({
            "schema": "agent_content_blob_v1",
            "kind": "tool_output",
            "id": id,
            "media_type": null,
            "byte_size": content.len().to_string(),
            "data_base64": null,
            "content": content,
        }),
    };
    serde_json::to_string(&value)
        .map_err(|error| format!("agent content projection encode failed: {error}"))
}

pub(crate) fn decode_agent_content_blob_record_v1(
    raw: &str,
) -> Result<AgentContentBlobRecord, String> {
    let value: serde_json::Value = serde_json::from_str(raw)
        .map_err(|error| format!("agent content projection decode failed: {error}"))?;
    if value.get("schema").and_then(serde_json::Value::as_str) != Some("agent_content_blob_v1") {
        return Err("agent content projection schema is unknown".to_string());
    }
    let id = value
        .get("id")
        .and_then(serde_json::Value::as_str)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| "agent content projection identity is missing".to_string())?
        .to_string();
    let stored_byte_size = value
        .get("byte_size")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "agent content projection byte size is missing".to_string())?
        .parse::<u64>()
        .map_err(|_| "agent content projection byte size is invalid".to_string())?;
    match value
        .get("kind")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "agent content projection kind is missing".to_string())?
    {
        "attachment" => {
            let media_type = value
                .get("media_type")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| "agent content projection media type is missing".to_string())?
                .to_string();
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(
                    value
                        .get("data_base64")
                        .and_then(serde_json::Value::as_str)
                        .ok_or_else(|| {
                            "agent content projection attachment data is missing".to_string()
                        })?,
                )
                .map_err(|_| "agent content projection attachment data is invalid".to_string())?;
            if u64::try_from(bytes.len()).ok() != Some(stored_byte_size)
                || !value.get("content").is_none_or(serde_json::Value::is_null)
            {
                return Err("agent content projection attachment is inconsistent".to_string());
            }
            Ok(AgentContentBlobRecord::Attachment {
                id,
                media_type,
                bytes,
            })
        }
        "tool_output" => {
            let content = value
                .get("content")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| "agent content projection output is missing".to_string())?
                .to_string();
            if u64::try_from(content.len()).ok() != Some(stored_byte_size)
                || !value
                    .get("media_type")
                    .is_none_or(serde_json::Value::is_null)
                || !value
                    .get("data_base64")
                    .is_none_or(serde_json::Value::is_null)
            {
                return Err("agent content projection output is inconsistent".to_string());
            }
            Ok(AgentContentBlobRecord::ToolOutput { id, content })
        }
        _ => Err("agent content projection kind is unknown".to_string()),
    }
}

fn stored_projection_from_canonical(
    projection: &CanonicalAgentSessionProjection,
) -> Result<StoredAgentSessionProjectionV1, String> {
    validate_meta(&projection.meta)?;
    Ok(StoredAgentSessionProjectionV1 {
        schema: SCHEMA.to_string(),
        meta: projection.meta.clone(),
        title: projection.title.clone(),
        workflow_instructions: projection.meta.workflow_instructions.clone(),
        agent_read_paths: projection.meta.agent_read_paths.clone(),
        context_epoch_payloads: projection
            .meta
            .context_epoch
            .as_ref()
            .map(|epoch| epoch.payload_cache_entries())
            .unwrap_or_default(),
        reducer_events: serde_json::from_slice(
            &encode_agent_session_events_v1(&projection.reducer_events, false)
                .map_err(|error| format!("agent reducer encode failed: {error}"))?,
        )
        .map_err(|error| format!("agent reducer JSON conversion failed: {error}"))?,
        queue_paused_at: projection.queue_paused_at,
        latest_token_usage: projection.latest_token_usage,
        pending_send_queue: projection
            .pending_send_queue
            .iter()
            .map(|entry| StoredCanonicalQueuedSendV1 {
                queue_item_id: entry.queue_item_id.clone(),
                human_message_id: entry.human_message_id.clone(),
                reserved_turn_id: entry.reserved_turn_id.clone(),
                input_ref: entry.input_ref.clone(),
            })
            .collect(),
    })
}

fn canonical_projection_from_stored(
    stored: StoredAgentSessionProjectionV1,
) -> Result<CanonicalAgentSessionProjection, String> {
    if stored.schema != SCHEMA {
        return Err("agent projection schema is unknown".to_string());
    }
    validate_meta(&stored.meta)?;
    if stored
        .queue_paused_at
        .is_some_and(|value| !value.is_finite())
    {
        return Err("agent projection queue timestamp is non-finite".to_string());
    }
    let mut meta = stored.meta;
    meta.workflow_instructions = stored.workflow_instructions;
    meta.agent_read_paths = stored.agent_read_paths;
    if let Some(context_epoch) = meta.context_epoch.as_mut() {
        context_epoch.hydrate_payload_cache(&stored.context_epoch_payloads);
    }
    let reducer_events = decode_agent_session_events_v1(
        &serde_json::to_vec(&stored.reducer_events)
            .map_err(|error| format!("agent reducer JSON conversion failed: {error}"))?,
    )
    .map_err(|error| format!("agent reducer decode failed: {error}"))?;
    Ok(CanonicalAgentSessionProjection {
        meta,
        title: stored.title,
        messages: Vec::new(),
        reducer_events,
        queue_paused_at: stored.queue_paused_at,
        latest_token_usage: stored.latest_token_usage,
        pending_send_queue: stored
            .pending_send_queue
            .into_iter()
            .map(|entry| CanonicalQueuedSend {
                queue_item_id: entry.queue_item_id,
                human_message_id: entry.human_message_id,
                reserved_turn_id: entry.reserved_turn_id,
                input_ref: entry.input_ref,
            })
            .collect(),
    })
}

fn agent_projection_record_from_canonical(
    projection: &CanonicalAgentSessionProjection,
) -> Result<AgentSessionProjectionRecord, String> {
    validate_meta(&projection.meta)?;
    if projection
        .queue_paused_at
        .is_some_and(|value| !value.is_finite())
    {
        return Err("agent projection queue timestamp is non-finite".to_string());
    }
    Ok(AgentSessionProjectionRecord {
        meta: metadata_record_from_session_meta(&projection.meta)?,
        title: projection.title.clone(),
        reducer_events: projection.reducer_events.clone(),
        queue_paused_at_bits: projection.queue_paused_at.map(f64::to_bits),
        latest_token_usage: projection.latest_token_usage.map(domain_token_usage),
        pending_send_queue: projection
            .pending_send_queue
            .iter()
            .map(|entry| AgentQueuedSendRecord {
                queue_item_id: entry.queue_item_id.clone(),
                human_message_id: entry.human_message_id.clone(),
                reserved_turn_id: entry.reserved_turn_id.clone(),
                input_ref: entry.input_ref.clone(),
            })
            .collect(),
    })
}

fn canonical_from_agent_projection_record(
    projection: &AgentSessionProjectionRecord,
) -> Result<CanonicalAgentSessionProjection, String> {
    let queue_paused_at = projection.queue_paused_at_bits.map(f64::from_bits);
    if queue_paused_at.is_some_and(|value| !value.is_finite()) {
        return Err("agent projection queue timestamp is non-finite".to_string());
    }
    let meta = session_meta_from_metadata_record(&projection.meta)?;
    validate_meta(&meta)?;
    Ok(CanonicalAgentSessionProjection {
        meta,
        title: projection.title.clone(),
        messages: Vec::new(),
        reducer_events: projection.reducer_events.clone(),
        queue_paused_at,
        latest_token_usage: projection.latest_token_usage.map(usecase_token_usage),
        pending_send_queue: projection
            .pending_send_queue
            .iter()
            .map(|entry| CanonicalQueuedSend {
                queue_item_id: entry.queue_item_id.clone(),
                human_message_id: entry.human_message_id.clone(),
                reserved_turn_id: entry.reserved_turn_id.clone(),
                input_ref: entry.input_ref.clone(),
            })
            .collect(),
    })
}

fn metadata_record_from_session_meta(
    meta: &SessionMeta,
) -> Result<AgentSessionMetadataRecord, String> {
    Ok(AgentSessionMetadataRecord {
        id: meta.id.clone(),
        worktree_path: meta.worktree_path.clone(),
        state: session_state_record(&meta.state),
        error_reason: meta.error_reason.clone(),
        state_revision: meta.state_revision,
        created_at_bits: finite_bits(meta.created_at, "created timestamp")?,
        updated_at_bits: finite_bits(meta.updated_at, "updated timestamp")?,
        agent_session_id: meta.agent_session_id.clone(),
        provider_session_generation: meta.provider_session_generation,
        provider_session_observation_id: meta.provider_session_observation_id.clone(),
        context_reinjection_generation: meta.context_reinjection_generation,
        context_carry: meta.context_carry.as_ref().map(context_carry_record),
        pending_recovery_message: meta
            .pending_recovery_message
            .as_ref()
            .map(pending_recovery_message_record),
        recovery_publication_snapshot: meta
            .recovery_publication_snapshot
            .as_ref()
            .map(recovery_publication_snapshot_record)
            .transpose()?,
        permission_mode: meta.permission_mode.clone(),
        plan_mode: meta.plan_mode,
        selected_model: meta.selected_model.clone(),
        permission_profile_id: meta.permission_profile_id.clone(),
        backend_id: meta.backend_id.clone(),
        workflow_node_session: meta.workflow_node_session,
        workflow_node_context: meta
            .workflow_node_context
            .clone()
            .map(workflow_node_context_mapper::to_domain),
        workflow_instructions: meta.workflow_instructions.clone(),
        agent_read_paths: meta.agent_read_paths.clone(),
        context_epoch: meta
            .context_epoch
            .as_ref()
            .map(context_epoch_record)
            .transpose()?,
        last_turn_interruption: meta
            .last_turn_interruption
            .as_ref()
            .map(turn_interruption_record),
        last_turn_id: meta.last_turn_id,
        first_message_preview: meta.first_message_preview.clone(),
        message_count: u64::try_from(meta.message_count)
            .map_err(|_| "agent projection message count exceeds u64".to_string())?,
        body_format_version: meta.body_format_version,
    })
}

fn session_meta_from_metadata_record(
    meta: &AgentSessionMetadataRecord,
) -> Result<SessionMeta, String> {
    let created_at = finite_from_bits(meta.created_at_bits, "created timestamp")?;
    let updated_at = finite_from_bits(meta.updated_at_bits, "updated timestamp")?;
    Ok(SessionMeta {
        id: meta.id.clone(),
        worktree_path: meta.worktree_path.clone(),
        state: session_state(&meta.state),
        error_reason: meta.error_reason.clone(),
        state_revision: meta.state_revision,
        created_at,
        updated_at,
        agent_session_id: meta.agent_session_id.clone(),
        provider_session_generation: meta.provider_session_generation,
        provider_session_observation_id: meta.provider_session_observation_id.clone(),
        context_reinjection_generation: meta.context_reinjection_generation,
        context_carry: meta.context_carry.as_ref().map(context_carry),
        pending_recovery_message: meta
            .pending_recovery_message
            .as_ref()
            .map(pending_recovery_message),
        recovery_publication_snapshot: meta
            .recovery_publication_snapshot
            .as_ref()
            .map(recovery_publication_snapshot)
            .transpose()?,
        permission_mode: meta.permission_mode.clone(),
        plan_mode: meta.plan_mode,
        selected_model: meta.selected_model.clone(),
        permission_profile_id: meta.permission_profile_id.clone(),
        backend_id: meta.backend_id.clone(),
        workflow_node_session: meta.workflow_node_session,
        workflow_node_context: meta
            .workflow_node_context
            .clone()
            .map(workflow_node_context_mapper::to_dto),
        workflow_instructions: meta.workflow_instructions.clone(),
        agent_read_paths: meta.agent_read_paths.clone(),
        context_epoch: meta.context_epoch.as_ref().map(context_epoch_meta),
        last_turn_interruption: meta.last_turn_interruption.as_ref().map(turn_interruption),
        last_turn_id: meta.last_turn_id,
        first_message_preview: meta.first_message_preview.clone(),
        message_count: usize::try_from(meta.message_count)
            .map_err(|_| "agent projection message count exceeds usize".to_string())?,
        body_format_version: meta.body_format_version,
    })
}

fn context_epoch_record(meta: &ContextEpochMeta) -> Result<AgentContextEpochRecord, String> {
    let sources = meta
        .source_revisions
        .iter()
        .map(|source| {
            let kind = context_source_kind_from_key(&source.kind)
                .ok_or_else(|| "agent projection context source kind is unknown".to_string())?;
            Ok(AgentContextSourceRecord {
                kind,
                revision: ContextRevision(source.revision),
                fingerprint: source.fingerprint.clone(),
                payload: source.payload.clone(),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(AgentContextEpochRecord {
        epoch: meta.epoch(),
        sources,
    })
}

fn context_epoch_meta(record: &AgentContextEpochRecord) -> ContextEpochMeta {
    ContextEpochMeta {
        epoch_id: record.epoch.id.0,
        backend_id: record.epoch.backend_id.clone(),
        model_id: record.epoch.model_id.clone(),
        worktree_path: record.epoch.worktree_path.clone(),
        source_revisions: record
            .sources
            .iter()
            .map(|source| ContextSourceRevisionMeta {
                kind: context_source_kind_key(source.kind).to_string(),
                revision: source.revision.0,
                fingerprint: source.fingerprint.clone(),
                payload: source.payload.clone(),
            })
            .collect(),
    }
}

fn session_state_record(state: &SessionState) -> AgentSessionStateRecord {
    match state {
        SessionState::Active => AgentSessionStateRecord::Active,
        SessionState::Idle => AgentSessionStateRecord::Idle,
        SessionState::Done => AgentSessionStateRecord::Done,
        SessionState::Error => AgentSessionStateRecord::Error,
        SessionState::Closed => AgentSessionStateRecord::Closed,
        SessionState::Archived => AgentSessionStateRecord::Archived,
    }
}

fn session_state(state: &AgentSessionStateRecord) -> SessionState {
    match state {
        AgentSessionStateRecord::Active => SessionState::Active,
        AgentSessionStateRecord::Idle => SessionState::Idle,
        AgentSessionStateRecord::Done => SessionState::Done,
        AgentSessionStateRecord::Error => SessionState::Error,
        AgentSessionStateRecord::Closed => SessionState::Closed,
        AgentSessionStateRecord::Archived => SessionState::Archived,
    }
}

fn context_carry_record(state: &ContextCarryState) -> AgentContextCarryStateRecord {
    match state {
        ContextCarryState::Resumed => AgentContextCarryStateRecord::Resumed,
        ContextCarryState::Reinjected => AgentContextCarryStateRecord::Reinjected,
        ContextCarryState::Failed => AgentContextCarryStateRecord::Failed,
    }
}

fn context_carry(state: &AgentContextCarryStateRecord) -> ContextCarryState {
    match state {
        AgentContextCarryStateRecord::Resumed => ContextCarryState::Resumed,
        AgentContextCarryStateRecord::Reinjected => ContextCarryState::Reinjected,
        AgentContextCarryStateRecord::Failed => ContextCarryState::Failed,
    }
}

fn pending_recovery_message_record(
    message: &PendingRecoveryMessage,
) -> AgentPendingRecoveryMessageRecord {
    match message {
        PendingRecoveryMessage::Notice {
            recovery_id,
            message_id,
        } => AgentPendingRecoveryMessageRecord::Notice {
            recovery_id: recovery_id.clone(),
            message_id: message_id.clone(),
        },
        PendingRecoveryMessage::Error {
            recovery_id,
            message_id,
            error,
        } => AgentPendingRecoveryMessageRecord::Error {
            recovery_id: recovery_id.clone(),
            message_id: message_id.clone(),
            error: error.clone(),
        },
    }
}

fn pending_recovery_message(message: &AgentPendingRecoveryMessageRecord) -> PendingRecoveryMessage {
    match message {
        AgentPendingRecoveryMessageRecord::Notice {
            recovery_id,
            message_id,
        } => PendingRecoveryMessage::Notice {
            recovery_id: recovery_id.clone(),
            message_id: message_id.clone(),
        },
        AgentPendingRecoveryMessageRecord::Error {
            recovery_id,
            message_id,
            error,
        } => PendingRecoveryMessage::Error {
            recovery_id: recovery_id.clone(),
            message_id: message_id.clone(),
            error: error.clone(),
        },
    }
}

fn recovery_publication_snapshot_record(
    snapshot: &RecoveryPublicationSnapshot,
) -> Result<AgentRecoveryPublicationSnapshotRecord, String> {
    Ok(AgentRecoveryPublicationSnapshotRecord {
        recovery_id: snapshot.recovery_id.clone(),
        summary: session_summary_record(&snapshot.summary)?,
        classification: recovery_publication_classification_record(&snapshot.classification),
    })
}

fn recovery_publication_snapshot(
    snapshot: &AgentRecoveryPublicationSnapshotRecord,
) -> Result<RecoveryPublicationSnapshot, String> {
    Ok(RecoveryPublicationSnapshot {
        recovery_id: snapshot.recovery_id.clone(),
        summary: session_summary_from_record(&snapshot.summary)?,
        classification: recovery_publication_classification(&snapshot.classification),
    })
}

fn recovery_publication_classification_record(
    classification: &RecoveryPublicationClassification,
) -> AgentRecoveryPublicationClassificationRecord {
    AgentRecoveryPublicationClassificationRecord {
        list: match classification.list {
            RecoveryPublicationList::SessionList => AgentRecoveryPublicationListRecord::SessionList,
            RecoveryPublicationList::ClosedHistory => {
                AgentRecoveryPublicationListRecord::ClosedHistory
            }
            RecoveryPublicationList::ArchivedHistory => {
                AgentRecoveryPublicationListRecord::ArchivedHistory
            }
        },
        workflow_owner: classification.workflow_owner.as_ref().map(|owner| {
            AgentRecoveryPublicationWorkflowOwnerRecord {
                execution_id: owner.execution_id.clone(),
                node_execution_id: owner.node_execution_id.clone(),
            }
        }),
    }
}

fn recovery_publication_classification(
    classification: &AgentRecoveryPublicationClassificationRecord,
) -> RecoveryPublicationClassification {
    RecoveryPublicationClassification {
        list: match classification.list {
            AgentRecoveryPublicationListRecord::SessionList => RecoveryPublicationList::SessionList,
            AgentRecoveryPublicationListRecord::ClosedHistory => {
                RecoveryPublicationList::ClosedHistory
            }
            AgentRecoveryPublicationListRecord::ArchivedHistory => {
                RecoveryPublicationList::ArchivedHistory
            }
        },
        workflow_owner: classification.workflow_owner.as_ref().map(|owner| {
            RecoveryPublicationWorkflowOwner {
                execution_id: owner.execution_id.clone(),
                node_execution_id: owner.node_execution_id.clone(),
            }
        }),
    }
}

fn session_summary_record(summary: &SessionSummary) -> Result<AgentSessionSummaryRecord, String> {
    Ok(AgentSessionSummaryRecord {
        id: summary.id.clone(),
        worktree_path: summary.worktree_path.clone(),
        state: session_state_record(&summary.state),
        error_reason: summary.error_reason.clone(),
        created_at_bits: finite_bits(summary.created_at, "summary created timestamp")?,
        updated_at_bits: finite_bits(summary.updated_at, "summary updated timestamp")?,
        first_message: summary.first_message.clone(),
        message_count: u64::try_from(summary.message_count)
            .map_err(|_| "agent projection summary count exceeds u64".to_string())?,
        agent_session_id: summary.agent_session_id.clone(),
        context_carry: summary.context_carry.as_ref().map(context_carry_record),
        permission_mode: summary.permission_mode.clone(),
        plan_mode: summary.plan_mode,
        permission_profile_id: summary.permission_profile_id.clone(),
        backend_id: summary.backend_id.clone(),
        workflow_node_session: summary.workflow_node_session,
        workflow_node_context: summary
            .workflow_node_context
            .clone()
            .map(workflow_node_context_mapper::to_domain),
    })
}

fn turn_interruption_record(interruption: &TurnInterruption) -> AgentTurnInterruptionRecord {
    AgentTurnInterruptionRecord {
        message_id: interruption.message_id.clone(),
        reason: match interruption.reason {
            TurnInterruptionReason::Abort => {
                crate::domain::agent_session::events::InterruptReason::Abort
            }
            TurnInterruptionReason::Timeout => {
                crate::domain::agent_session::events::InterruptReason::Timeout
            }
            TurnInterruptionReason::Crash => {
                crate::domain::agent_session::events::InterruptReason::Crash
            }
            TurnInterruptionReason::SessionClosed => {
                crate::domain::agent_session::events::InterruptReason::SessionClosed
            }
        },
    }
}

fn turn_interruption(interruption: &AgentTurnInterruptionRecord) -> TurnInterruption {
    TurnInterruption {
        message_id: interruption.message_id.clone(),
        reason: match interruption.reason {
            crate::domain::agent_session::events::InterruptReason::Abort => {
                TurnInterruptionReason::Abort
            }
            crate::domain::agent_session::events::InterruptReason::Timeout => {
                TurnInterruptionReason::Timeout
            }
            crate::domain::agent_session::events::InterruptReason::Crash => {
                TurnInterruptionReason::Crash
            }
            crate::domain::agent_session::events::InterruptReason::SessionClosed => {
                TurnInterruptionReason::SessionClosed
            }
        },
    }
}

fn agent_message_record_from_chat_message(
    message: &ChatMessage,
) -> Result<AgentMessageProjectionRecord, String> {
    Ok(AgentMessageProjectionRecord {
        id: message.id.clone(),
        role: match message.role {
            MessageRole::Human => AgentMessageRoleRecord::Human,
            MessageRole::Agent => AgentMessageRoleRecord::Agent,
            MessageRole::System => AgentMessageRoleRecord::System,
        },
        content: message.content.clone(),
        thinking: message.thinking.clone(),
        activities: message
            .activities
            .as_ref()
            .map(|activities| {
                activities
                    .iter()
                    .map(agent_message_activity_record)
                    .collect::<Result<Vec<_>, String>>()
            })
            .transpose()?,
        parts: message.parts.clone(),
        streaming_final_seq: message.streaming_final_seq,
        timestamp_bits: finite_bits(message.timestamp, "message timestamp")?,
        mentions: message.mentions.as_ref().map(|mentions| {
            mentions
                .iter()
                .cloned()
                .map(MessageMention::into_domain)
                .collect()
        }),
    })
}

fn chat_message_from_agent_message_record(
    message: &AgentMessageProjectionRecord,
) -> Result<ChatMessage, String> {
    Ok(ChatMessage {
        id: message.id.clone(),
        role: match message.role {
            AgentMessageRoleRecord::Human => MessageRole::Human,
            AgentMessageRoleRecord::Agent => MessageRole::Agent,
            AgentMessageRoleRecord::System => MessageRole::System,
        },
        content: message.content.clone(),
        thinking: message.thinking.clone(),
        activities: message
            .activities
            .as_ref()
            .map(|activities| {
                activities
                    .iter()
                    .map(agent_message_activity)
                    .collect::<Result<Vec<_>, String>>()
            })
            .transpose()?,
        parts: message.parts.clone(),
        streaming_final_seq: message.streaming_final_seq,
        timestamp: finite_from_bits(message.timestamp_bits, "message timestamp")?,
        mentions: message.mentions.as_ref().map(|mentions| {
            mentions
                .iter()
                .cloned()
                .map(MessageMention::from_domain)
                .collect()
        }),
    })
}

fn agent_message_activity_record(
    activity: &ActivityEntry,
) -> Result<AgentMessageActivityRecord, String> {
    match activity {
        ActivityEntry::ToolUse { tool, input, id } => Ok(AgentMessageActivityRecord::ToolUse {
            tool: tool.clone(),
            input: JsonPayload::new_unchecked(
                serde_json::to_string(input)
                    .map_err(|error| format!("agent activity input encode failed: {error}"))?,
            ),
            id: id.clone(),
        }),
        ActivityEntry::ToolResult {
            content,
            is_error,
            tool_use_id,
            content_ref,
            summary,
        } => Ok(AgentMessageActivityRecord::ToolResult {
            content: content.clone(),
            is_error: *is_error,
            tool_use_id: tool_use_id.clone(),
            content_ref: content_ref.clone(),
            summary: summary.clone(),
        }),
        ActivityEntry::PermissionResult {
            tool_name,
            status,
            summary,
        } => Ok(AgentMessageActivityRecord::PermissionResult {
            tool_name: tool_name.clone(),
            status: status.clone(),
            summary: summary.clone(),
        }),
    }
}

fn agent_message_activity(activity: &AgentMessageActivityRecord) -> Result<ActivityEntry, String> {
    match activity {
        AgentMessageActivityRecord::ToolUse { tool, input, id } => Ok(ActivityEntry::ToolUse {
            tool: tool.clone(),
            input: serde_json::from_str(input.as_str())
                .map_err(|error| format!("agent activity input decode failed: {error}"))?,
            id: id.clone(),
        }),
        AgentMessageActivityRecord::ToolResult {
            content,
            is_error,
            tool_use_id,
            content_ref,
            summary,
        } => Ok(ActivityEntry::ToolResult {
            content: content.clone(),
            is_error: *is_error,
            tool_use_id: tool_use_id.clone(),
            content_ref: content_ref.clone(),
            summary: summary.clone(),
        }),
        AgentMessageActivityRecord::PermissionResult {
            tool_name,
            status,
            summary,
        } => Ok(ActivityEntry::PermissionResult {
            tool_name: tool_name.clone(),
            status: status.clone(),
            summary: summary.clone(),
        }),
    }
}

fn domain_token_usage(usage: TokenUsage) -> DomainTokenUsage {
    DomainTokenUsage {
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
        total_tokens: usage.total_tokens,
        context_window_tokens: usage.context_window_tokens,
    }
}

fn usecase_token_usage(usage: DomainTokenUsage) -> TokenUsage {
    TokenUsage {
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
        total_tokens: usage.total_tokens,
        context_window_tokens: usage.context_window_tokens,
    }
}

fn finite_bits(value: f64, field: &str) -> Result<u64, String> {
    value
        .is_finite()
        .then_some(value.to_bits())
        .ok_or_else(|| format!("agent projection {field} is non-finite"))
}

fn finite_from_bits(bits: u64, field: &str) -> Result<f64, String> {
    let value = f64::from_bits(bits);
    value
        .is_finite()
        .then_some(value)
        .ok_or_else(|| format!("agent projection {field} is non-finite"))
}

fn validate_meta(meta: &SessionMeta) -> Result<(), String> {
    if meta.id.is_empty()
        || !meta.created_at.is_finite()
        || !meta.updated_at.is_finite()
        || meta.state_revision > i64::MAX as u64
        || meta.provider_session_generation > i64::MAX as u64
        || meta.message_count > i64::MAX as usize
        || meta
            .last_turn_id
            .is_some_and(|turn_id| turn_id > i64::MAX as u64)
    {
        return Err("agent projection metadata is inconsistent".to_string());
    }
    Ok(())
}
