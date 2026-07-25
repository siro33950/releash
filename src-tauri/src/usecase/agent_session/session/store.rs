use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use base64::Engine as _;
use parking_lot::RwLock;
use sha2::{Digest, Sha256};

#[cfg(test)]
use crate::domain::agent_session::AgentSessionStorage;
use crate::domain::agent_session::{
    AgentSessionProjectedMessage, AgentSessionProjectionCommit, AgentSessionReader,
    AgentSessionStorageTypes,
};
use crate::domain::path::same_worktree_path;
use crate::usecase::agent_session::context_meta::ContextEpochMeta;
use crate::usecase::agent_session::event_log::{
    finalize_turn, latest_turn_interruption, AgentSessionEvent, AgentTurnFailureSignal,
    BackendSessionRecoveryProjection, BackendSessionRecoveryReason, GoalReactivationOutcome,
    SessionReadModel, TurnEventLog, WorkflowTurnCompleteInput,
};

use super::{
    error_reason_for_state, now_timestamp, ChatMessage, ChatSession, ContextCarryState,
    MessagePageMetadata, MessagePart, MessageRole, PageCursor, PendingRecoveryMessage,
    RecoveryPublicationClassification, RecoveryPublicationList, RecoveryPublicationSnapshot,
    RecoveryPublicationWorkflowOwner, SessionAttachment, SessionMeta, SessionPage,
    SessionReviewContext, SessionState, SessionSummary, SessionToolOutput, TokenUsage,
    TurnInterruption, WorkflowNodeContextDto,
};

/// `SessionState` の遷移を観測する購読者向けコールバック。
/// 引数は `(session_id, worktree_path, new_state, state_revision)`。
pub type SessionStateChangeListener =
    Arc<dyn Fn(&str, &str, &SessionState, u64) + Send + Sync + 'static>;
pub type SessionEventLogRecoveryListener = Arc<dyn Fn(&str) + Send + Sync + 'static>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NextTurnIdError {
    CapacityExceeded,
    Unavailable(String),
}

impl std::fmt::Display for NextTurnIdError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CapacityExceeded => formatter.write_str("turn identity capacity is exhausted"),
            Self::Unavailable(message) => formatter.write_str(message),
        }
    }
}

impl From<String> for NextTurnIdError {
    fn from(value: String) -> Self {
        Self::Unavailable(value)
    }
}

fn next_sqlite_counter(value: u64, label: &str) -> Result<u64, String> {
    let next = value
        .checked_add(1)
        .ok_or_else(|| format!("{label} capacity is exhausted"))?;
    if next > i64::MAX as u64 {
        return Err(format!("{label} capacity is exhausted"));
    }
    Ok(next)
}

fn add_sqlite_count(value: usize, delta: usize, label: &str) -> Result<usize, String> {
    let next = value
        .checked_add(delta)
        .ok_or_else(|| format!("{label} capacity is exhausted"))?;
    if u64::try_from(next).map_or(true, |next| next > i64::MAX as u64) {
        return Err(format!("{label} capacity is exhausted"));
    }
    Ok(next)
}

#[async_trait::async_trait]
pub(crate) trait RuntimeTerminalParticipantProvider: Send + Sync {
    async fn prepare(
        &self,
        terminal: &crate::domain::local_event::TerminalRecordMutation,
    ) -> Result<RuntimeTerminalParticipants, String>;
}

#[derive(Debug, Clone, Default)]
pub(crate) struct RuntimeTerminalParticipants {
    pub events: Vec<AgentSessionEvent>,
    pub mutations: Vec<crate::domain::local_event::LocalStateMutation>,
}

#[derive(Debug, Clone)]
pub(crate) struct BackendRecoveryReadbackParticipants {
    pub expected_heads: Vec<crate::domain::local_event::ExpectedStreamHead>,
    pub events: Vec<crate::domain::local_event::UncommittedDomainEvent>,
    pub canonical_events: Vec<u8>,
    pub mutations: Vec<crate::domain::local_event::LocalStateMutation>,
    pub participant_digest: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PendingWorkflowTurnCompletion {
    pub obligation_id: String,
    pub revision: crate::domain::local_event::Revision,
    pub session_id: String,
    pub workflow_context: WorkflowNodeContextDto,
    pub input: WorkflowTurnCompleteInput,
    terminal_identity: String,
    message_id: String,
    notification_sha256: String,
}

#[derive(Debug)]
pub(crate) struct PendingWorkflowTurnCompletionPage {
    pub entries: Vec<PendingWorkflowTurnCompletion>,
    pub next_cursor: Option<crate::domain::local_event::QueryCursor>,
}

#[derive(Debug, Clone)]
struct CanonicalContentBlob {
    identity: String,
    projection: crate::domain::local_event::AgentContentBlobRecord,
}

fn externalize_canonical_message_content(
    messages: &mut [ChatMessage],
) -> Result<Vec<CanonicalContentBlob>, String> {
    let mut blobs = Vec::new();
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
                    let detected =
                        crate::domain::agent_session::services::detect_image_mime(&bytes)
                            .ok_or_else(|| {
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
                        projection:
                            crate::domain::local_event::AgentContentBlobRecord::Attachment {
                                id: id.clone(),
                                media_type: media_type.clone(),
                                bytes,
                            },
                    });
                    *part = MessagePart::ImageRef {
                        attachment: super::AttachmentRef {
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
                } if content_ref.is_none() && super::should_externalize_tool_output(content) => {
                    let content = content.clone();
                    let id = hex::encode(Sha256::digest(content.as_bytes()));
                    blobs.push(CanonicalContentBlob {
                        identity: format!("tool_output:{id}"),
                        projection:
                            crate::domain::local_event::AgentContentBlobRecord::ToolOutput {
                                id: id.clone(),
                                content: content.clone(),
                            },
                    });
                    let projected_summary = summary
                        .clone()
                        .unwrap_or_else(|| super::tool_output_summary(&content, *is_error, true));
                    *part = MessagePart::ToolResult {
                        content: super::tool_output_preview(&content),
                        is_error: *is_error,
                        tool_use_id: tool_use_id.clone(),
                        parent_tool_use_id: parent_tool_use_id.clone(),
                        content_ref: Some(super::ToolOutputRef {
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

async fn prepare_canonical_content_blob_mutations(
    repository: &Arc<dyn crate::domain::local_event::LocalEventTransactionRepository>,
    session_id: &str,
    blobs: Vec<CanonicalContentBlob>,
) -> Result<Vec<crate::domain::local_event::LocalStateMutation>, String> {
    let blob_session_id = format!("blob:{session_id}");
    let mut mutations = Vec::new();
    for blob in blobs {
        let projection =
            crate::domain::local_event::MessageProjectionRecord::AgentContentBlob(blob.projection);
        let current = repository
            .query(
                crate::domain::local_event::LocalEventQuery::MessageProjectionByIdentity {
                    session_id: blob_session_id.clone(),
                    message_id: blob.identity.clone(),
                },
            )
            .await
            .map_err(|error| format!("canonical content blob lookup failed: {error}"))?;
        let crate::domain::local_event::LocalEventQueryResult::MessageProjectionByIdentity(current) =
            current
        else {
            return Err("canonical content blob lookup returned the wrong shape".to_string());
        };
        let (expected, revision) = match current {
            Some(current) if current.projection == projection => continue,
            Some(_) => return Err("canonical content blob identity collision".to_string()),
            None => (
                crate::domain::local_event::RevisionGuard::Absent,
                crate::domain::local_event::Revision::new(0).expect("zero revision"),
            ),
        };
        mutations.push(
            crate::domain::local_event::LocalStateMutation::MessageProjection(
                crate::domain::local_event::MessageProjectionMutation {
                    session_id: blob_session_id.clone(),
                    message_id: blob.identity,
                    projection,
                    expected,
                    revision,
                },
            ),
        );
    }
    Ok(mutations)
}

pub(crate) struct ErrorEpisodeInput {
    pub message_id: String,
    pub reason: String,
    pub at: f64,
}

struct PreviousSessionProjection {
    state: SessionState,
    error_reason: Option<String>,
    worktree_path: String,
    state_revision: u64,
}

#[derive(Clone)]
struct AgentSessionEventAuthority {
    repository: Arc<dyn crate::domain::local_event::LocalEventTransactionRepository>,
    installation_id: String,
    projection_codec: Arc<dyn AgentSessionProjectionCodec>,
}

#[derive(Debug, Clone)]
pub(crate) struct CanonicalAgentSessionProjection {
    pub meta: SessionMeta,
    pub title: Option<String>,
    pub messages: Vec<ChatMessage>,
    /// Bounded reducer state for the current turn and session-wide latches.
    /// Historical messages live in `message_projection`; normal mutations
    /// must never rebuild this state by folding the event stream.
    pub reducer_events: Vec<AgentSessionEvent>,
    pub queue_paused_at: Option<f64>,
    pub latest_token_usage: Option<TokenUsage>,
    /// Immutable queue dispositions accepted by the durable send command.
    /// Exact retry payload remains in the encrypted operation obligation;
    /// this bounded projection owns queue identity/readback only.
    pub pending_send_queue: Vec<CanonicalQueuedSend>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CanonicalQueuedSend {
    pub queue_item_id: String,
    pub human_message_id: String,
    pub reserved_turn_id: String,
    pub input_ref: String,
}

pub(crate) struct SendAcceptanceProjectionInput<'a> {
    pub session_id: &'a str,
    pub initial_session: Option<&'a ChatSession>,
    /// The exact canonical session projection revision from which the turn
    /// identity and queue disposition were allocated. Acceptance must use
    /// this same guard instead of rebasing a stale allocation onto a newer
    /// queue projection.
    pub session_projection_guard: crate::domain::local_event::RevisionGuard,
    pub human_message_id: &'a str,
    pub prompt: &'a crate::domain::agent_session::events::PromptInput,
    pub disposition: &'a crate::domain::agent_session::events::SendDisposition,
    pub reserved_turn_id: Option<&'a str>,
    pub input_ref: &'a str,
    pub events: &'a [AgentSessionEvent],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SendAcceptanceAllocation {
    pub next_turn_id: u64,
    /// The canonical projection already owns an active turn. This must travel
    /// with the same revision guard as `next_turn_id`; process-local runtime
    /// hydration is deliberately not an admission authority after restart.
    pub has_active_turn: bool,
    pub has_pending_queue: bool,
    pub session_projection_guard: crate::domain::local_event::RevisionGuard,
}

#[derive(Debug, Clone)]
struct ExpectedAcceptedQueueFront {
    queue_item_id: String,
}

const ACCEPTED_QUEUE_START_BLOCKED: &str = "accepted queued turn is currently blocked";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AcceptedQueuedTurnStartCommitOutcome {
    Committed,
    Blocked,
}

fn bounded_reducer_events(
    mut previous: Vec<AgentSessionEvent>,
    appended: &[AgentSessionEvent],
) -> Vec<AgentSessionEvent> {
    previous.extend_from_slice(appended);
    let Some(turn_start) = previous
        .iter()
        .rposition(|event| matches!(event, AgentSessionEvent::TurnStarted { .. }))
    else {
        return previous;
    };

    // A new turn makes all earlier turn-local events irrelevant to the
    // reducer. Preserve only session-wide latches whose current value is
    // still needed, then retain the complete current turn. This keeps normal
    // mutation cost independent of unrelated historical turns.
    let mut retained = Vec::new();
    if let Some((_, event)) = previous[..turn_start]
        .iter()
        .enumerate()
        .rev()
        .find(|(_, event)| {
            matches!(
                event,
                AgentSessionEvent::QueuePaused { .. } | AgentSessionEvent::QueueResumed { .. }
            )
        })
    {
        retained.push(event.clone());
    }
    if let Some(recovery_start) = previous[..turn_start].iter().rposition(|event| {
        matches!(
            event,
            AgentSessionEvent::BackendSessionRecoveryStarted { .. }
        )
    }) {
        retained.extend(
            previous[recovery_start..turn_start]
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
    if let Some(event) = previous[..turn_start]
        .iter()
        .rev()
        .find(|event| matches!(event, AgentSessionEvent::SessionClosed { .. }))
    {
        retained.push(event.clone());
    }
    retained.extend_from_slice(&previous[turn_start..]);
    retained
}

pub(crate) trait AgentSessionProjectionCodec: Send + Sync {
    fn encode(
        &self,
        projection: &CanonicalAgentSessionProjection,
    ) -> Result<crate::domain::local_event::SessionProjectionRecord, String>;

    fn decode(
        &self,
        payload: &crate::domain::local_event::SessionProjectionRecord,
    ) -> Result<CanonicalAgentSessionProjection, String>;

    fn encode_message(
        &self,
        message: &ChatMessage,
    ) -> Result<crate::domain::local_event::MessageProjectionRecord, String>;

    fn decode_message(
        &self,
        payload: &crate::domain::local_event::MessageProjectionRecord,
    ) -> Result<ChatMessage, String>;

    /// Exact gateway-owned Stored V1 bytes used by existing replay identities.
    fn encode_session_identity_v1(
        &self,
        payload: &crate::domain::local_event::SessionProjectionRecord,
    ) -> Result<Vec<u8>, String>;

    /// Exact gateway-owned Stored V1 bytes used by existing replay identities.
    fn encode_message_identity_v1(
        &self,
        payload: &crate::domain::local_event::MessageProjectionRecord,
    ) -> Result<Vec<u8>, String>;

    fn encode_events_for_identity(&self, events: &[AgentSessionEvent]) -> Result<Vec<u8>, String>;

    fn encode_parts_for_identity(&self, parts: &[MessagePart]) -> Result<Vec<u8>, String>;
}

#[derive(Debug, Clone)]
struct TerminalMessageProjectionPatch {
    message_id: String,
    streaming_final_seq: u64,
    timestamp: Option<f64>,
    parts: Option<Vec<MessagePart>>,
}

fn drop_redundant_terminal_queue_pause(
    previous: &[AgentSessionEvent],
    mut candidate: Vec<AgentSessionEvent>,
) -> Vec<AgentSessionEvent> {
    if TurnEventLog::from_events(previous.to_vec())
        .project()
        .queue_paused_at
        .is_some()
    {
        candidate.retain(|event| !matches!(event, AgentSessionEvent::QueuePaused { .. }));
    }
    candidate
}

fn terminal_requires_queue_pause(events: &[AgentSessionEvent]) -> bool {
    events.iter().any(|event| {
        matches!(event, AgentSessionEvent::TurnInterrupted { .. })
            || matches!(
                event,
                AgentSessionEvent::TurnCompleted { exit_code, .. } if *exit_code != 0
            )
    })
}

fn complete_terminal_projection_events(
    previous: &[AgentSessionEvent],
    supplied: &[AgentSessionEvent],
) -> Vec<AgentSessionEvent> {
    let Some((terminal_index, turn_id)) =
        supplied
            .iter()
            .enumerate()
            .find_map(|(index, event)| match event {
                AgentSessionEvent::TurnCompleted { turn_id, .. }
                | AgentSessionEvent::TurnInterrupted { turn_id, .. } => Some((index, *turn_id)),
                _ => None,
            })
    else {
        return supplied.to_vec();
    };
    let current_turn_id = previous.iter().rev().find_map(|event| match event {
        AgentSessionEvent::TurnStarted { turn_id, .. } => Some(*turn_id),
        _ => None,
    });
    let terminal_already_committed = previous.iter().any(|event| {
        matches!(
            event,
            AgentSessionEvent::TurnCompleted { turn_id: id, .. }
                | AgentSessionEvent::TurnInterrupted { turn_id: id, .. } if *id == turn_id
        )
    });
    if current_turn_id != Some(turn_id) || terminal_already_committed {
        return supplied
            .iter()
            .filter(|event| {
                !matches!(
                    event,
                    AgentSessionEvent::TurnInterruptRequested { turn_id: id, .. }
                        | AgentSessionEvent::FinalPartsRecorded { turn_id: id, .. }
                        | AgentSessionEvent::ToolCallFailed { turn_id: id, .. }
                        | AgentSessionEvent::PermissionResolved { turn_id: id, .. }
                        | AgentSessionEvent::TurnCompleted { turn_id: id, .. }
                        | AgentSessionEvent::TurnInterrupted { turn_id: id, .. }
                        if *id == turn_id
                ) && !matches!(event, AgentSessionEvent::QueuePaused { .. })
            })
            .cloned()
            .collect();
    }

    let AgentSessionEvent::TurnInterrupted {
        reason,
        error,
        exit_code,
        ..
    } = &supplied[terminal_index]
    else {
        return drop_redundant_terminal_queue_pause(previous, supplied.to_vec());
    };

    let mut full = previous.to_vec();
    full.extend_from_slice(&supplied[..terminal_index]);
    let delta_start = previous.len();
    let has_final_parts = full.iter().any(|event| {
        matches!(
            event,
            AgentSessionEvent::FinalPartsRecorded { turn_id: id, .. } if *id == turn_id
        )
    });
    if !has_final_parts {
        let assistant_message_id = full.iter().rev().find_map(|event| match event {
            AgentSessionEvent::TurnStarted {
                turn_id: id,
                message_id,
                assistant_message_id,
                ..
            } if *id == turn_id => Some(
                assistant_message_id
                    .clone()
                    .unwrap_or_else(|| format!("{message_id}:agent")),
            ),
            _ => None,
        });
        if let Some(message_id) = assistant_message_id {
            let projected = TurnEventLog::from_events(full.clone()).project();
            full.push(AgentSessionEvent::FinalPartsRecorded {
                turn_id,
                parts: projected.agent_parts_for_message(&message_id),
                message_id,
            });
        }
    }
    finalize_turn(&mut full, turn_id, *reason, error.clone(), *exit_code);
    let mut completed = full.into_iter().skip(delta_start).collect::<Vec<_>>();
    completed.extend_from_slice(&supplied[terminal_index.saturating_add(1)..]);
    drop_redundant_terminal_queue_pause(previous, completed)
}

fn recovery_publication_snapshot(
    recovery_id: &str,
    meta: &SessionMeta,
) -> RecoveryPublicationSnapshot {
    let list = match meta.state {
        SessionState::Closed => RecoveryPublicationList::ClosedHistory,
        SessionState::Archived => RecoveryPublicationList::ArchivedHistory,
        _ => RecoveryPublicationList::SessionList,
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
    RecoveryPublicationSnapshot {
        recovery_id: recovery_id.to_string(),
        summary: meta.to_summary(),
        classification: RecoveryPublicationClassification {
            list,
            workflow_owner,
        },
    }
}

fn recovery_publication_owner_matches(snapshot: &RecoveryPublicationSnapshot) -> bool {
    let summary = &snapshot.summary;
    match &snapshot.classification.workflow_owner {
        None => !summary.workflow_node_session && summary.workflow_node_context.is_none(),
        Some(owner) => {
            if !summary.workflow_node_session && summary.workflow_node_context.is_none() {
                return false;
            }
            match &summary.workflow_node_context {
                Some(context) => {
                    owner.execution_id.as_deref() == Some(context.execution_id.as_str())
                        && owner.node_execution_id.as_deref()
                            == Some(context.node_execution_id.as_str())
                }
                None => owner.execution_id.is_none() && owner.node_execution_id.is_none(),
            }
        }
    }
}

pub(crate) enum BackendSessionRecoveryStartOutcome {
    Started(Box<SessionMeta>),
    SuppressedByQueuePause,
}

pub(crate) enum ProviderSessionEstablishmentOutcome {
    Settled(Box<SessionMeta>),
    Missing,
    Fenced,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ContextRestoreCompletionRequest {
    expected_provider_session_generation: u64,
    expected_turn_id: Option<u64>,
    reinjected: bool,
    clear_context_carry: bool,
    recovery_restore_required: bool,
}

impl ContextRestoreCompletionRequest {
    pub(crate) fn after_started_turn(
        expected_provider_session_generation: u64,
        expected_turn_id: u64,
        reinjected: bool,
        clear_context_carry: bool,
        recovery_restore_required: bool,
    ) -> Self {
        Self {
            expected_provider_session_generation,
            expected_turn_id: Some(expected_turn_id),
            reinjected,
            clear_context_carry,
            recovery_restore_required,
        }
    }
}

const BACKEND_RECOVERY_START_SUPPRESSED_BY_QUEUE_PAUSE: &str =
    "backend recovery start was suppressed by a durable queue pause";
const CONTEXT_RESTORE_COMPLETION_FENCED: &str =
    "context restore completion was fenced by newer durable session state";
const CONTEXT_RESTORE_COMPLETION_UNCHANGED: &str =
    "context restore completion is already reflected in durable session state";

#[derive(Debug, Clone)]
enum EventProjectionMetaPatch {
    Started {
        expected_generation: u64,
        publication_snapshot: Box<RecoveryPublicationSnapshot>,
        at: f64,
    },
    Completed {
        expected_generation: u64,
        provider_session_generation: u64,
        backend_session_id: String,
        pending_recovery_message: PendingRecoveryMessage,
        at: f64,
    },
    ReadbackCompleted {
        old_provider_session_generation: u64,
        provider_session_generation: u64,
        backend_session_id: String,
        pending_recovery_message: PendingRecoveryMessage,
        at: f64,
    },
    Failed {
        pending_recovery_message: PendingRecoveryMessage,
        at: f64,
    },
    ContextRestoreCompleted {
        expected_provider_session_generation: u64,
        expected_turn_id: Option<u64>,
        reinjected: bool,
        clear_context_carry: bool,
        recovery_restore_required: bool,
        at: f64,
    },
}

fn hash_identity_field(hasher: &mut Sha256, value: &[u8]) {
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value);
}

fn apply_context_restore_completion_to_meta(
    meta: &mut SessionMeta,
    expected_provider_session_generation: u64,
    expected_turn_id: Option<u64>,
    reinjected: bool,
    clear_context_carry: bool,
    recovery_restore_required: bool,
    at: f64,
) -> Result<bool, String> {
    let lifecycle_fenced = matches!(
        meta.state,
        SessionState::Error | SessionState::Closed | SessionState::Archived
    );
    if recovery_restore_required {
        let pending_failure = matches!(
            meta.pending_recovery_message,
            Some(PendingRecoveryMessage::Error { .. })
        );
        if lifecycle_fenced
            || pending_failure
            || meta.recovery_publication_snapshot.is_some()
            || meta.provider_session_generation != expected_provider_session_generation
            || meta.context_reinjection_generation != Some(expected_provider_session_generation)
        {
            return Err(CONTEXT_RESTORE_COMPLETION_FENCED.to_string());
        }
        meta.context_reinjection_generation = None;
        if reinjected {
            meta.context_carry = Some(ContextCarryState::Reinjected);
        }
    } else {
        let next_generation = expected_provider_session_generation.checked_add(1);
        let generation_matches = meta.provider_session_generation
            == expected_provider_session_generation
            || next_generation == Some(meta.provider_session_generation);
        let backend_recovery_observation = meta
            .provider_session_observation_id
            .as_deref()
            .is_some_and(|identity| identity.starts_with("backend-recovery/v1:"));
        if lifecycle_fenced
            || !generation_matches
            || expected_turn_id.is_none()
            || meta.last_turn_id != expected_turn_id
            || backend_recovery_observation
            || meta.recovery_publication_snapshot.is_some()
            || meta.pending_recovery_message.is_some()
            || meta.context_reinjection_generation.is_some()
            || meta.context_carry == Some(ContextCarryState::Failed)
        {
            return Err(CONTEXT_RESTORE_COMPLETION_FENCED.to_string());
        }
        if !reinjected && !clear_context_carry {
            return Err(CONTEXT_RESTORE_COMPLETION_UNCHANGED.to_string());
        }
        let context_carry = reinjected.then_some(ContextCarryState::Reinjected);
        if meta.context_carry == context_carry {
            return Err(CONTEXT_RESTORE_COMPLETION_UNCHANGED.to_string());
        }
        meta.context_carry = context_carry;
    }
    meta.updated_at = at;
    Ok(true)
}

fn hash_pending_recovery_message(hasher: &mut Sha256, pending: &PendingRecoveryMessage) {
    match pending {
        PendingRecoveryMessage::Notice {
            recovery_id,
            message_id,
        } => {
            hash_identity_field(hasher, b"notice");
            hash_identity_field(hasher, recovery_id.as_bytes());
            hash_identity_field(hasher, message_id.as_bytes());
        }
        PendingRecoveryMessage::Error {
            recovery_id,
            message_id,
            error,
        } => {
            hash_identity_field(hasher, b"error");
            hash_identity_field(hasher, recovery_id.as_bytes());
            hash_identity_field(hasher, message_id.as_bytes());
            hash_identity_field(hasher, error.as_bytes());
        }
    }
}

fn hash_terminal_message_patch(
    hasher: &mut Sha256,
    codec: &dyn AgentSessionProjectionCodec,
    patch: &TerminalMessageProjectionPatch,
) -> Result<(), String> {
    hash_identity_field(hasher, b"terminal_message_patch_v1");
    hash_identity_field(hasher, patch.message_id.as_bytes());
    hasher.update(patch.streaming_final_seq.to_be_bytes());
    match patch.timestamp {
        Some(timestamp) => {
            hasher.update([1]);
            hasher.update(timestamp.to_bits().to_be_bytes());
        }
        None => hasher.update([0]),
    }
    match &patch.parts {
        Some(parts) => {
            hasher.update([1]);
            let encoded = codec.encode_parts_for_identity(parts)?;
            hash_identity_field(hasher, &encoded);
        }
        None => hasher.update([0]),
    }
    Ok(())
}

fn hash_projection_meta_patch(
    hasher: &mut Sha256,
    patch: &EventProjectionMetaPatch,
) -> Result<(), String> {
    hash_identity_field(hasher, b"event_projection_meta_patch_v1");
    match patch {
        EventProjectionMetaPatch::Started {
            expected_generation,
            publication_snapshot,
            at,
        } => {
            hash_identity_field(hasher, b"recovery_started");
            hasher.update(expected_generation.to_be_bytes());
            let encoded = serde_json::to_vec(publication_snapshot)
                .map_err(|error| format!("recovery publication snapshot encode failed: {error}"))?;
            hash_identity_field(hasher, &encoded);
            hasher.update(at.to_bits().to_be_bytes());
        }
        EventProjectionMetaPatch::Completed {
            expected_generation,
            provider_session_generation,
            backend_session_id,
            pending_recovery_message,
            at,
        } => {
            hash_identity_field(hasher, b"recovery_completed");
            hasher.update(expected_generation.to_be_bytes());
            hasher.update(provider_session_generation.to_be_bytes());
            hash_identity_field(hasher, backend_session_id.as_bytes());
            hash_pending_recovery_message(hasher, pending_recovery_message);
            hasher.update(at.to_bits().to_be_bytes());
        }
        EventProjectionMetaPatch::ReadbackCompleted {
            old_provider_session_generation,
            provider_session_generation,
            backend_session_id,
            pending_recovery_message,
            at,
        } => {
            hash_identity_field(hasher, b"recovery_readback_completed");
            hasher.update(old_provider_session_generation.to_be_bytes());
            hasher.update(provider_session_generation.to_be_bytes());
            hash_identity_field(hasher, backend_session_id.as_bytes());
            hash_pending_recovery_message(hasher, pending_recovery_message);
            hasher.update(at.to_bits().to_be_bytes());
        }
        EventProjectionMetaPatch::Failed {
            pending_recovery_message,
            at,
        } => {
            hash_identity_field(hasher, b"recovery_failed");
            hash_pending_recovery_message(hasher, pending_recovery_message);
            hasher.update(at.to_bits().to_be_bytes());
        }
        EventProjectionMetaPatch::ContextRestoreCompleted {
            expected_provider_session_generation,
            expected_turn_id,
            reinjected,
            clear_context_carry,
            recovery_restore_required,
            at,
        } => {
            hash_identity_field(hasher, b"context_restore_completed");
            hasher.update(expected_provider_session_generation.to_be_bytes());
            match expected_turn_id {
                Some(turn_id) => {
                    hasher.update([1]);
                    hasher.update(turn_id.to_be_bytes());
                }
                None => hasher.update([0]),
            }
            hasher.update([
                u8::from(*reinjected),
                u8::from(*clear_context_carry),
                u8::from(*recovery_restore_required),
            ]);
            hasher.update(at.to_bits().to_be_bytes());
        }
    }
    Ok(())
}

fn patch_event_projection_meta(
    codec: &dyn AgentSessionProjectionCodec,
    mutations: &mut [crate::domain::local_event::LocalStateMutation],
    patch: &EventProjectionMetaPatch,
) -> Result<(), String> {
    let projection = mutations
        .iter_mut()
        .find_map(|mutation| match mutation {
            crate::domain::local_event::LocalStateMutation::SessionProjection(projection) => {
                Some(projection)
            }
            _ => None,
        })
        .ok_or_else(|| "agent event batch omitted its session projection".to_string())?;
    let mut decoded = codec.decode(&projection.projection)?;
    match patch {
        EventProjectionMetaPatch::Started {
            expected_generation,
            publication_snapshot,
            at,
        } => {
            if decoded.queue_paused_at.is_some() {
                return Err(BACKEND_RECOVERY_START_SUPPRESSED_BY_QUEUE_PAUSE.to_string());
            }
            if decoded.meta.provider_session_generation != *expected_generation {
                return Err(format!(
                    "Backend session generation changed while starting recovery: expected {expected_generation}, actual {}",
                    decoded.meta.provider_session_generation
                ));
            }
            decoded.meta.agent_session_id = None;
            decoded.meta.provider_session_observation_id = None;
            decoded.meta.context_reinjection_generation = None;
            decoded.meta.context_carry = Some(ContextCarryState::Failed);
            if matches!(
                publication_snapshot.summary.state,
                SessionState::Closed | SessionState::Archived
            ) {
                // Closed and archived are explicit lifecycle decisions, not
                // states derived by the turn event projector. A recovery-only
                // event has no turn and would otherwise infer Idle, silently
                // reopening the session in canonical public lists.
                decoded.meta.state = publication_snapshot.summary.state.clone();
                decoded.meta.error_reason = publication_snapshot.summary.error_reason.clone();
            }
            decoded.meta.recovery_publication_snapshot =
                Some(publication_snapshot.as_ref().clone());
            decoded.meta.updated_at = *at;
        }
        EventProjectionMetaPatch::Completed {
            expected_generation,
            provider_session_generation,
            backend_session_id,
            pending_recovery_message,
            at,
        } => {
            if decoded.meta.provider_session_generation != *expected_generation {
                return Err(format!(
                    "Backend session generation changed while completing recovery: expected {expected_generation}, actual {}",
                    decoded.meta.provider_session_generation
                ));
            }
            decoded.meta.agent_session_id = Some(backend_session_id.clone());
            decoded.meta.provider_session_generation = *provider_session_generation;
            let (recovery_id, _) = pending_recovery_message_identity(pending_recovery_message);
            decoded.meta.provider_session_observation_id =
                Some(backend_recovery_provider_observation_id(recovery_id));
            decoded.meta.context_reinjection_generation = Some(*provider_session_generation);
            decoded.meta.pending_recovery_message = Some(pending_recovery_message.clone());
            decoded.meta.recovery_publication_snapshot = None;
            decoded.meta.updated_at = *at;
        }
        EventProjectionMetaPatch::ReadbackCompleted {
            old_provider_session_generation,
            provider_session_generation,
            backend_session_id,
            pending_recovery_message,
            at,
        } => {
            let expected_provider_session_generation = old_provider_session_generation
                .checked_add(1)
                .ok_or_else(|| "provider session generation is exhausted".to_string())?;
            if *provider_session_generation != expected_provider_session_generation
                || decoded.meta.provider_session_generation != *provider_session_generation
                || decoded.meta.agent_session_id.as_deref() != Some(backend_session_id.as_str())
                || decoded.meta.context_reinjection_generation != Some(*provider_session_generation)
            {
                return Err(
                    "durable backend recovery owner evidence no longer matches its reservation"
                        .to_string(),
                );
            }
            let (recovery_id, _) = pending_recovery_message_identity(pending_recovery_message);
            decoded.meta.provider_session_observation_id =
                Some(backend_recovery_provider_observation_id(recovery_id));
            decoded.meta.pending_recovery_message = Some(pending_recovery_message.clone());
            decoded.meta.recovery_publication_snapshot = None;
            decoded.meta.updated_at = *at;
        }
        EventProjectionMetaPatch::Failed {
            pending_recovery_message,
            at,
        } => {
            decoded.meta.state = SessionState::Error;
            decoded.meta.provider_session_observation_id = None;
            decoded.meta.error_reason = match pending_recovery_message {
                PendingRecoveryMessage::Error { error, .. } => Some(error.clone()),
                PendingRecoveryMessage::Notice { .. } => decoded.meta.error_reason,
            };
            decoded.meta.pending_recovery_message = Some(pending_recovery_message.clone());
            decoded.meta.recovery_publication_snapshot = None;
            decoded.meta.updated_at = *at;
        }
        EventProjectionMetaPatch::ContextRestoreCompleted {
            expected_provider_session_generation,
            expected_turn_id,
            reinjected,
            clear_context_carry,
            recovery_restore_required,
            at,
        } => apply_context_restore_completion_to_meta(
            &mut decoded.meta,
            *expected_provider_session_generation,
            *expected_turn_id,
            *reinjected,
            *clear_context_carry,
            *recovery_restore_required,
            *at,
        )
        .map(|_| ())?,
    }
    projection.projection = codec.encode(&decoded)?;
    Ok(())
}

async fn prepare_canonical_event_projection_mutations(
    authority: &AgentSessionEventAuthority,
    session_id: &str,
    events: &[AgentSessionEvent],
    fallback_meta: Option<SessionMeta>,
    terminal_message_patch: Option<&TerminalMessageProjectionPatch>,
    expected_terminal_queue_paused: Option<bool>,
) -> Result<Vec<crate::domain::local_event::LocalStateMutation>, String> {
    let codec = authority.projection_codec.as_ref();
    let stored = match authority
        .repository
        .query(
            crate::domain::local_event::LocalEventQuery::SessionProjectionByIdentity {
                session_id: session_id.to_string(),
            },
        )
        .await
        .map_err(|error| format!("agent SQLite projection read failed: {error}"))?
    {
        crate::domain::local_event::LocalEventQueryResult::SessionProjectionByIdentity(stored) => {
            stored
        }
        _ => return Err("agent SQLite projection query returned the wrong shape".to_string()),
    };
    let (
        mut meta,
        title,
        reducer_events,
        mut pending_send_queue,
        queue_paused_at,
        expected,
        revision,
    ) = match stored {
        Some(stored) => {
            let decoded = codec.decode(&stored.projection)?;
            let next = stored
                .revision
                .next()
                .ok_or_else(|| "agent projection revision exhausted".to_string())?;
            (
                decoded.meta,
                decoded.title,
                decoded.reducer_events,
                decoded.pending_send_queue,
                decoded.queue_paused_at,
                crate::domain::local_event::RevisionGuard::Expected(stored.revision),
                next,
            )
        }
        None => (
            fallback_meta
                .ok_or_else(|| "agent projection has no initialization metadata".to_string())?,
            None,
            Vec::new(),
            Vec::new(),
            None,
            crate::domain::local_event::RevisionGuard::Absent,
            crate::domain::local_event::Revision::new(0).expect("zero revision"),
        ),
    };
    if expected_terminal_queue_paused.is_some_and(|expected| expected != queue_paused_at.is_some())
    {
        return Err(
            "terminal queue-pause authority changed before projection preparation; retry"
                .to_string(),
        );
    }
    let reducer_events = bounded_reducer_events(reducer_events, events);
    if let Some(terminal_turn_id) = events.iter().rev().find_map(|event| match event {
        AgentSessionEvent::TurnCompleted { turn_id, .. }
        | AgentSessionEvent::TurnInterrupted { turn_id, .. } => Some(*turn_id),
        _ => None,
    }) {
        pending_send_queue
            .retain(|entry| entry.reserved_turn_id.parse::<u64>().ok() != Some(terminal_turn_id));
    }
    let last_turn_interruption = latest_turn_interruption(&reducer_events);
    let last_turn_id = reducer_events.iter().rev().find_map(|event| match event {
        AgentSessionEvent::TurnStarted { turn_id, .. } => Some(*turn_id),
        _ => None,
    });
    let mut touched_message_ids = reducer_events
        .iter()
        .rev()
        .find_map(|event| match event {
            AgentSessionEvent::TurnStarted {
                message_id,
                assistant_message_id,
                ..
            } => Some([
                message_id.clone(),
                assistant_message_id
                    .clone()
                    .unwrap_or_else(|| format!("{message_id}:agent")),
            ]),
            _ => None,
        })
        .into_iter()
        .flatten()
        .collect::<HashSet<_>>();
    for event in events {
        if let AgentSessionEvent::SessionErrored { message_id, .. } = event {
            touched_message_ids.insert(message_id.clone());
        }
    }
    let projected = TurnEventLog::from_events(reducer_events.clone()).project();
    meta.state = projected.status.session_state.clone();
    meta.error_reason = error_reason_for_state(&meta.state, &projected.error_reason);
    meta.state_revision = next_sqlite_counter(meta.state_revision, "session state revision")?;
    meta.last_turn_interruption = last_turn_interruption;
    meta.last_turn_id = last_turn_id;
    let latest_token_usage = projected
        .workflow_turn_complete
        .as_ref()
        .and_then(|turn| turn.token_usage)
        .map(|usage| TokenUsage {
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            total_tokens: usage.input_tokens.checked_add(usage.output_tokens),
            context_window_tokens: None,
        });
    let mut projected_messages = projected
        .messages
        .iter()
        .filter(|message| touched_message_ids.contains(&message.id))
        .cloned()
        .collect::<Vec<_>>();
    if let Some(patch) = terminal_message_patch {
        let message = projected_messages
            .iter_mut()
            .find(|message| message.id == patch.message_id)
            .ok_or_else(|| {
                format!(
                    "Turn projection omitted message {} for {session_id}",
                    patch.message_id
                )
            })?;
        message.streaming_final_seq = patch.streaming_final_seq;
        if let Some(timestamp) = patch.timestamp {
            message.timestamp = timestamp;
        }
        if let Some(parts) = &patch.parts {
            message.parts = Some(parts.clone());
        }
    }
    let content_blobs = externalize_canonical_message_content(&mut projected_messages)?;
    let mut state_mutations =
        prepare_canonical_content_blob_mutations(&authority.repository, session_id, content_blobs)
            .await?;
    let mut inserted_messages = Vec::new();
    for message in projected_messages {
        let encoded_message = codec.encode_message(&message)?;
        let stored = match authority
            .repository
            .query(
                crate::domain::local_event::LocalEventQuery::MessageProjectionByIdentity {
                    session_id: session_id.to_string(),
                    message_id: message.id.clone(),
                },
            )
            .await
            .map_err(|error| format!("agent SQLite message projection read failed: {error}"))?
        {
            crate::domain::local_event::LocalEventQueryResult::MessageProjectionByIdentity(
                stored,
            ) => stored,
            _ => {
                return Err(
                    "agent SQLite message projection query returned the wrong shape".to_string(),
                );
            }
        };
        if stored
            .as_ref()
            .is_some_and(|stored| stored.projection == encoded_message)
        {
            continue;
        }
        let (expected, revision) = match stored {
            Some(stored) => (
                crate::domain::local_event::RevisionGuard::Expected(stored.revision),
                stored
                    .revision
                    .next()
                    .ok_or_else(|| "agent message projection revision exhausted".to_string())?,
            ),
            None => {
                inserted_messages.push(message.clone());
                (
                    crate::domain::local_event::RevisionGuard::Absent,
                    crate::domain::local_event::Revision::new(0).expect("zero revision"),
                )
            }
        };
        state_mutations.push(
            crate::domain::local_event::LocalStateMutation::MessageProjection(
                crate::domain::local_event::MessageProjectionMutation {
                    session_id: session_id.to_string(),
                    message_id: message.id,
                    projection: encoded_message,
                    expected,
                    revision,
                },
            ),
        );
    }
    if !inserted_messages.is_empty() {
        meta.message_count = add_sqlite_count(
            meta.message_count,
            inserted_messages.len(),
            "session message count",
        )?;
        if meta.first_message_preview.is_empty() {
            meta.first_message_preview = super::first_message_preview(&inserted_messages);
        }
    }
    let projection = codec.encode(&CanonicalAgentSessionProjection {
        meta,
        title,
        messages: Vec::new(),
        reducer_events,
        queue_paused_at: projected.queue_paused_at,
        latest_token_usage,
        pending_send_queue,
    })?;
    state_mutations.insert(
        0,
        crate::domain::local_event::LocalStateMutation::SessionProjection(
            crate::domain::local_event::SessionProjectionMutation {
                session_id: session_id.to_string(),
                projection,
                expected,
                revision,
            },
        ),
    );
    Ok(state_mutations)
}

fn runtime_terminal_record_mutation(
    session_id: &str,
    events: &[AgentSessionEvent],
    message_id: &str,
    streaming_final_seq: u64,
    completed_at: f64,
    turn_result: &crate::domain::agent_session::entities::TurnResult,
    encoded_events: &[u8],
) -> Result<crate::domain::local_event::LocalStateMutation, String> {
    let turn_id = events
        .iter()
        .rev()
        .find_map(|event| match event {
            AgentSessionEvent::TurnCompleted { turn_id, .. }
            | AgentSessionEvent::TurnInterrupted { turn_id, .. } => Some(*turn_id),
            _ => None,
        })
        .ok_or_else(|| "terminal event batch is missing its terminal fact".to_string())?;
    let terminal_kind = match turn_result {
        crate::domain::agent_session::entities::TurnResult::Completed { .. } => {
            crate::domain::local_event::AgentTerminalKind::Completed
        }
        crate::domain::agent_session::entities::TurnResult::Failed { .. } => {
            crate::domain::local_event::AgentTerminalKind::Crash
        }
        crate::domain::agent_session::entities::TurnResult::Interrupted { reason, .. } => {
            match reason {
                crate::domain::agent_session::entities::InterruptReason::Abort => {
                    crate::domain::local_event::AgentTerminalKind::Abort
                }
                crate::domain::agent_session::entities::InterruptReason::Timeout => {
                    crate::domain::local_event::AgentTerminalKind::Timeout
                }
                crate::domain::agent_session::entities::InterruptReason::Crash => {
                    crate::domain::local_event::AgentTerminalKind::Crash
                }
                crate::domain::agent_session::entities::InterruptReason::SessionClosed => {
                    crate::domain::local_event::AgentTerminalKind::SessionClosed
                }
            }
        }
    };
    let mut participant = Sha256::new();
    participant.update(session_id.as_bytes());
    participant.update(turn_id.to_be_bytes());
    participant.update(message_id.as_bytes());
    participant.update(streaming_final_seq.to_be_bytes());
    participant.update(completed_at.to_bits().to_be_bytes());
    hash_identity_field(&mut participant, encoded_events);
    let participant_digest: [u8; 32] = participant.finalize().into();
    let terminal_identity = format!("runtime-terminal-{}", hex::encode(participant_digest));
    Ok(
        crate::domain::local_event::LocalStateMutation::TerminalRecord(
            crate::domain::local_event::TerminalRecordMutation {
                session_id: session_id.to_string(),
                turn_id: turn_id.to_string(),
                terminal_identity,
                result: crate::domain::local_event::TerminalResultRecord::AgentTurn {
                    kind: terminal_kind,
                    session_id: session_id.to_string(),
                    turn_id: turn_id.to_string(),
                    message_id: message_id.to_string(),
                    streaming_final_sequence: streaming_final_seq,
                    completed_at_bits: completed_at.to_bits(),
                    result: crate::domain::local_event::AgentTurnTerminalResultRecord::Current(
                        turn_result.clone(),
                    ),
                },
                participant_digest,
            },
        ),
    )
}

fn backend_recovery_obligation_id(session_id: &str, recovery_id: &str) -> String {
    format!("backend-recovery:{session_id}:{recovery_id}")
}

fn backend_recovery_provider_observation_id(recovery_id: &str) -> String {
    format!("backend-recovery/v1:{recovery_id}")
}

fn pending_recovery_message_identity(pending: &PendingRecoveryMessage) -> (&str, &str) {
    match pending {
        PendingRecoveryMessage::Notice {
            recovery_id,
            message_id,
        }
        | PendingRecoveryMessage::Error {
            recovery_id,
            message_id,
            ..
        } => (recovery_id, message_id),
    }
}

fn recovery_publication_obligation_id(
    session_id: &str,
    recovery_id: &str,
    message_id: &str,
) -> String {
    let digest = Sha256::digest(
        format!("recovery-publication/v1\0{session_id}\0{recovery_id}\0{message_id}").as_bytes(),
    );
    format!("recovery-publication-{}", hex::encode(digest))
}

const WORKFLOW_TURN_COMPLETION_ORDERED_KEY_PREFIX: &str = "workflow_turn_complete:";

fn workflow_context_to_domain(
    context: &WorkflowNodeContextDto,
) -> crate::domain::workflow::WorkflowNodeContext {
    crate::domain::workflow::WorkflowNodeContext {
        execution_id: context.execution_id.clone(),
        node_execution_id: context.node_execution_id.clone(),
        workflow_name: context.workflow_name.clone(),
        node_name: context.node_name.clone(),
        attempt: context.attempt,
        parent_node_name: context.parent_node_name.clone(),
        parent_attempt: context.parent_attempt,
        order: context.order,
        startup_timeout_secs: context.startup_timeout_secs,
        startup_max_retries: context.startup_max_retries,
        stale_timeout_secs: context.stale_timeout_secs,
    }
}

fn workflow_context_to_dto(
    context: &crate::domain::workflow::WorkflowNodeContext,
) -> WorkflowNodeContextDto {
    WorkflowNodeContextDto {
        execution_id: context.execution_id.clone(),
        node_execution_id: context.node_execution_id.clone(),
        workflow_name: context.workflow_name.clone(),
        node_name: context.node_name.clone(),
        attempt: context.attempt,
        parent_node_name: context.parent_node_name.clone(),
        parent_attempt: context.parent_attempt,
        order: context.order,
        startup_timeout_secs: context.startup_timeout_secs,
        startup_max_retries: context.startup_max_retries,
        stale_timeout_secs: context.stale_timeout_secs,
    }
}

fn hash_optional_text(hasher: &mut Sha256, value: Option<&str>) {
    match value {
        Some(value) => {
            hasher.update([1]);
            hash_identity_field(hasher, value.as_bytes());
        }
        None => hasher.update([0]),
    }
}

fn hash_optional_u32(hasher: &mut Sha256, value: Option<u32>) {
    match value {
        Some(value) => {
            hasher.update([1]);
            hasher.update(value.to_be_bytes());
        }
        None => hasher.update([0]),
    }
}

fn hash_optional_u64(hasher: &mut Sha256, value: Option<u64>) {
    match value {
        Some(value) => {
            hasher.update([1]);
            hasher.update(value.to_be_bytes());
        }
        None => hasher.update([0]),
    }
}

fn workflow_turn_completion_notification_sha256(
    session_id: &str,
    workflow_context: &WorkflowNodeContextDto,
    terminal_identity: &str,
    message_id: &str,
    input: &WorkflowTurnCompleteInput,
) -> String {
    let mut hasher = Sha256::new();
    hash_identity_field(
        &mut hasher,
        b"workflow_turn_completion_notification_identity_v1",
    );
    hash_identity_field(&mut hasher, session_id.as_bytes());
    hash_identity_field(&mut hasher, workflow_context.execution_id.as_bytes());
    hash_identity_field(&mut hasher, workflow_context.node_execution_id.as_bytes());
    hash_identity_field(&mut hasher, workflow_context.workflow_name.as_bytes());
    hash_identity_field(&mut hasher, workflow_context.node_name.as_bytes());
    hasher.update(workflow_context.attempt.to_be_bytes());
    hash_optional_text(&mut hasher, workflow_context.parent_node_name.as_deref());
    hash_optional_u32(&mut hasher, workflow_context.parent_attempt);
    hasher.update(workflow_context.order.to_be_bytes());
    hash_optional_u64(&mut hasher, workflow_context.startup_timeout_secs);
    hash_optional_u32(&mut hasher, workflow_context.startup_max_retries);
    hash_optional_u64(&mut hasher, workflow_context.stale_timeout_secs);
    hash_identity_field(&mut hasher, terminal_identity.as_bytes());
    hash_identity_field(&mut hasher, message_id.as_bytes());
    hasher.update(input.turn_id.to_be_bytes());
    hasher.update(input.exit_code.to_be_bytes());
    hasher.update((input.final_text_parts.len() as u64).to_be_bytes());
    for part in &input.final_text_parts {
        hash_identity_field(&mut hasher, part.as_bytes());
    }
    match input.failure_signal {
        Some(AgentTurnFailureSignal::ModelRefusal) => {
            hasher.update([1]);
            hash_identity_field(&mut hasher, b"model_refusal");
        }
        None => hasher.update([0]),
    }
    match input.token_usage {
        Some(usage) => {
            hasher.update([1]);
            hasher.update(usage.input_tokens.to_be_bytes());
            hasher.update(usage.output_tokens.to_be_bytes());
        }
        None => hasher.update([0]),
    }
    hasher.update([u8::from(input.interrupted)]);
    hex::encode(hasher.finalize())
}

fn workflow_turn_completion_obligation_id(notification_sha256: &str) -> String {
    format!("workflow-turn-complete:{notification_sha256}")
}

fn workflow_turn_completion_ordered_key(turn_id: u64, notification_sha256: &str) -> String {
    format!("{WORKFLOW_TURN_COMPLETION_ORDERED_KEY_PREFIX}{turn_id:020}:{notification_sha256}")
}

fn workflow_turn_completion_pending_mutation(
    session_id: &str,
    workflow_context: &WorkflowNodeContextDto,
    terminal: &crate::domain::local_event::TerminalRecordMutation,
    message_id: &str,
    input: &WorkflowTurnCompleteInput,
) -> Result<crate::domain::local_event::LocalStateMutation, String> {
    if terminal.session_id != session_id
        || terminal.turn_id.parse::<u64>().ok() != Some(input.turn_id)
    {
        return Err("workflow turn-completion terminal identity is inconsistent".to_string());
    }
    let notification_sha256 = workflow_turn_completion_notification_sha256(
        session_id,
        workflow_context,
        &terminal.terminal_identity,
        message_id,
        input,
    );
    let obligation_id = workflow_turn_completion_obligation_id(&notification_sha256);
    let notification_digest: [u8; 32] = hex::decode(&notification_sha256)
        .map_err(|_| "workflow turn-completion digest is invalid".to_string())?
        .try_into()
        .map_err(|_| "workflow turn-completion digest has an invalid length".to_string())?;
    let record = crate::domain::local_event::ObligationRecord::WorkflowTurnCompletion {
        session_id: session_id.to_string(),
        turn_id: input.turn_id.to_string(),
        terminal_identity: terminal.terminal_identity.clone(),
        notification_sha256: notification_digest,
        detail: crate::domain::local_event::WorkflowTurnCompletionObligationRecord::Pending {
            workflow_context: Box::new(workflow_context_to_domain(workflow_context)),
            message_id: message_id.to_string(),
            exit_code: input.exit_code,
            failure_signal: input.failure_signal.map(|signal| match signal {
                AgentTurnFailureSignal::ModelRefusal => {
                    crate::domain::local_event::WorkflowTurnFailureSignalRecord::ModelRefusal
                }
            }),
            token_usage: input.token_usage,
            interrupted: input.interrupted,
        },
        state: crate::domain::local_event::ObligationStateRecord::Pending,
    };
    Ok(crate::domain::local_event::LocalStateMutation::Obligation(
        crate::domain::local_event::ObligationMutation {
            obligation_id,
            record,
            pending: Some(crate::domain::local_event::PendingIndexEntry {
                ordered_key: workflow_turn_completion_ordered_key(
                    input.turn_id,
                    &notification_sha256,
                ),
                owner: session_id.to_string(),
                partition: crate::domain::local_event::PendingPartition::Owner,
                shutdown_plan: None,
            }),
            expected: crate::domain::local_event::RevisionGuard::Absent,
            revision: crate::domain::local_event::Revision::new(0).expect("zero revision"),
        },
    ))
}

fn workflow_final_text_parts(message: &ChatMessage) -> Vec<String> {
    message
        .parts
        .as_deref()
        .unwrap_or_default()
        .iter()
        .filter_map(|part| match part {
            MessagePart::Text { content, .. } => Some(content.clone()),
            _ => None,
        })
        .collect()
}

fn recovery_publication_message_record(
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

fn recovery_publication_message_matches(
    stored: &crate::domain::local_event::RecoveryPublicationMessageRecord,
    expected: &PendingRecoveryMessage,
) -> bool {
    stored == &recovery_publication_message_record(expected)
}

fn backend_recovery_obligation_mutation(
    obligation_id: String,
    record: crate::domain::local_event::ObligationRecord,
    pending: Option<crate::domain::local_event::PendingIndexEntry>,
    current: Option<&crate::domain::local_event::ObligationView>,
) -> Result<crate::domain::local_event::LocalStateMutation, String> {
    let (expected, revision) = match current {
        Some(current) => (
            crate::domain::local_event::RevisionGuard::Expected(current.revision),
            current
                .revision
                .next()
                .ok_or_else(|| "backend recovery obligation revision exhausted".to_string())?,
        ),
        None => (
            crate::domain::local_event::RevisionGuard::Absent,
            crate::domain::local_event::Revision::new(0).expect("zero revision"),
        ),
    };
    Ok(crate::domain::local_event::LocalStateMutation::Obligation(
        crate::domain::local_event::ObligationMutation {
            obligation_id,
            record,
            pending,
            expected,
            revision,
        },
    ))
}

fn recovery_publication_obligation_mutation(
    obligation_id: String,
    record: crate::domain::local_event::ObligationRecord,
    pending: Option<crate::domain::local_event::PendingIndexEntry>,
    current: Option<&crate::domain::local_event::ObligationView>,
) -> Result<crate::domain::local_event::LocalStateMutation, String> {
    let (expected, revision) = match current {
        Some(current) => (
            crate::domain::local_event::RevisionGuard::Expected(current.revision),
            current
                .revision
                .next()
                .ok_or_else(|| "recovery publication obligation revision exhausted".to_string())?,
        ),
        None => (
            crate::domain::local_event::RevisionGuard::Absent,
            crate::domain::local_event::Revision::new(0).expect("zero revision"),
        ),
    };
    Ok(crate::domain::local_event::LocalStateMutation::Obligation(
        crate::domain::local_event::ObligationMutation {
            obligation_id,
            record,
            pending,
            expected,
            revision,
        },
    ))
}

#[cfg(test)]
pub trait SessionReviewContextReader: Send + Sync {
    fn get_session_review_context(
        &self,
        _app_data_dir: &Path,
        session_id: &str,
    ) -> Result<Option<SessionReviewContext>, String>;
}

/// Gateway が物理 event log を修復した事実を usecase へ一度だけ伝える signal。
/// 修復方式や storage format は domain の writer API へ露出させない。
#[cfg(test)]
pub(crate) trait SessionEventLogRecoverySignal: Send + Sync {
    #[cfg(test)]
    fn take_event_log_recovered(&self, session_id: &str) -> bool;
}

/// queue pause の小さい durable projection を読む port。
/// transcript 全体の read model を構築せず runtime/query を hydrate するために分離する。
#[cfg(test)]
pub trait SessionQueuePauseReader: Send + Sync {
    fn load_queue_paused_at(
        &self,
        app_data_dir: &Path,
        session_id: &str,
    ) -> Result<Option<f64>, String>;
}

#[cfg(test)]
pub trait SessionStoragePort:
    AgentSessionStorage<
        Session = ChatSession,
        Meta = SessionMeta,
        PageCursor = PageCursor,
        Page = SessionPage,
        Message = ChatMessage,
        MessagePart = MessagePart,
        Attachment = SessionAttachment,
        ToolOutput = SessionToolOutput,
        Event = AgentSessionEvent,
    > + SessionReviewContextReader
    + SessionEventLogRecoverySignal
    + SessionQueuePauseReader
    + Send
    + Sync
{
}

#[cfg(test)]
impl<T> SessionStoragePort for T where
    T: AgentSessionStorage<
            Session = ChatSession,
            Meta = SessionMeta,
            PageCursor = PageCursor,
            Page = SessionPage,
            Message = ChatMessage,
            MessagePart = MessagePart,
            Attachment = SessionAttachment,
            ToolOutput = SessionToolOutput,
            Event = AgentSessionEvent,
        > + SessionReviewContextReader
        + SessionEventLogRecoverySignal
        + SessionQueuePauseReader
        + Send
        + Sync
{
}

pub type SessionReaderPort = dyn AgentSessionReader<
        Session = ChatSession,
        Meta = SessionMeta,
        PageCursor = PageCursor,
        Page = SessionPage,
        Message = ChatMessage,
        MessagePart = MessagePart,
        Attachment = SessionAttachment,
        ToolOutput = SessionToolOutput,
        Event = AgentSessionEvent,
    > + Send
    + Sync;

/// テストで session 保存パスへ失敗を注入するためのフック。
/// workflow node session の作成ロールバック経路（fanout child node の save 失敗等）を
/// 検証するために用いる。
#[cfg(test)]
pub(crate) type SessionSaveHook = Arc<dyn Fn(&ChatSession) -> Result<(), String> + Send + Sync>;
#[cfg(test)]
pub(crate) type SessionAppendMessageHook =
    Arc<dyn Fn(&str, &ChatMessage) -> Result<(), String> + Send + Sync>;
#[cfg(test)]
pub(crate) type SessionPersistPartsHook =
    Arc<dyn Fn(&str, &str, &[MessagePart]) -> Result<(), String> + Send + Sync>;
#[cfg(test)]
pub(crate) type SessionAppendEventHook =
    Arc<dyn Fn(&str, &AgentSessionEvent) -> Result<(), String> + Send + Sync>;
#[cfg(test)]
pub(crate) type SessionSetStateHook =
    Arc<dyn Fn(&str, &SessionState) -> Result<(), String> + Send + Sync>;
#[cfg(test)]
pub(crate) type SessionProjectionHook =
    Arc<dyn Fn(&str, &SessionState, Option<&str>) -> Result<(), String> + Send + Sync>;
#[cfg(test)]
pub(crate) type SessionAppendedEventHook = Arc<dyn Fn(&str, &AgentSessionEvent) + Send + Sync>;
#[cfg(test)]
pub(crate) type SessionEventProjectionHook =
    Arc<dyn Fn(&str, Option<u64>) -> Result<(), String> + Send + Sync>;
#[cfg(test)]
pub(crate) type SessionAtomicEventCommitHook = Arc<
    dyn Fn(crate::domain::local_event::CommitOperationKind) -> Result<(), String> + Send + Sync,
>;
#[cfg(test)]
pub(crate) type SessionBackendEstablishedHook =
    Arc<dyn Fn(&str, &str) -> Result<(), String> + Send + Sync>;
#[cfg(test)]
pub(crate) type SessionProjectedReadModelHook = Arc<dyn Fn(&str, &SessionReadModel) + Send + Sync>;

pub struct SessionStore {
    #[cfg(test)]
    storage: Option<Arc<dyn SessionStoragePort>>,
    event_authority: RwLock<Option<AgentSessionEventAuthority>>,
    state_change_listeners: RwLock<Vec<SessionStateChangeListener>>,
    event_log_recovery_listeners: RwLock<Vec<SessionEventLogRecoveryListener>>,
    runtime_terminal_participant_provider:
        RwLock<Option<Arc<dyn RuntimeTerminalParticipantProvider>>>,
    #[cfg(test)]
    permission_response_reservations:
        RwLock<HashMap<String, crate::domain::local_event::ObligationRecord>>,
    #[cfg(test)]
    save_hook: RwLock<Option<SessionSaveHook>>,
    #[cfg(test)]
    append_message_hook: RwLock<Option<SessionAppendMessageHook>>,
    #[cfg(test)]
    persist_parts_hook: RwLock<Option<SessionPersistPartsHook>>,
    #[cfg(test)]
    append_event_hook: RwLock<Option<SessionAppendEventHook>>,
    #[cfg(test)]
    set_state_hook: RwLock<Option<SessionSetStateHook>>,
    #[cfg(test)]
    projection_hook: RwLock<Option<SessionProjectionHook>>,
    #[cfg(test)]
    appended_event_hook: RwLock<Option<SessionAppendedEventHook>>,
    #[cfg(test)]
    event_projection_hook: RwLock<Option<SessionEventProjectionHook>>,
    #[cfg(test)]
    atomic_event_commit_hook: RwLock<Option<SessionAtomicEventCommitHook>>,
    #[cfg(test)]
    backend_established_hook: RwLock<Option<SessionBackendEstablishedHook>>,
    #[cfg(test)]
    projected_read_model_hook: RwLock<Option<SessionProjectedReadModelHook>>,
}

fn compact_session_title(title: &str) -> String {
    let compact = title.split_whitespace().collect::<Vec<_>>().join(" ");
    match compact.char_indices().nth(100) {
        Some((byte_pos, _)) => format!("{}…", &compact[..byte_pos]),
        None => compact,
    }
}

impl AgentSessionStorageTypes for SessionStore {
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

impl AgentSessionReader for SessionStore {
    fn list_metas(&self, app_data_dir: &Path) -> Result<Vec<Self::Meta>, String> {
        self.list_metas_for_active_read_authority(app_data_dir)
    }

    fn session_title(
        &self,
        app_data_dir: &Path,
        session_id: &str,
    ) -> Result<Option<String>, String> {
        SessionStore::session_title(self, app_data_dir, session_id)
    }

    fn session_titles(&self, app_data_dir: &Path) -> Result<HashMap<String, String>, String> {
        SessionStore::session_titles(self, app_data_dir)
    }

    fn get_session_meta(
        &self,
        app_data_dir: &Path,
        session_id: &str,
    ) -> Result<Option<Self::Meta>, String> {
        SessionStore::get_session_meta(self, app_data_dir, session_id)
    }

    fn load_full_session_for_restore(
        &self,
        app_data_dir: &Path,
        session_id: &str,
    ) -> Result<Option<Self::Session>, String> {
        SessionStore::load_full_session_for_restore(self, app_data_dir, session_id)
    }

    fn load_previous_human_message_before_agent(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        agent_message_id: &str,
    ) -> Result<Option<Self::Message>, String> {
        SessionStore::load_previous_human_message_before_agent(
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
        SessionStore::get_session_page(self, app_data_dir, session_id, cursor, limit)
    }

    fn get_session_attachment(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        attachment_id: &str,
    ) -> Result<Option<Self::Attachment>, String> {
        SessionStore::get_session_attachment(self, app_data_dir, session_id, attachment_id)
    }

    fn get_session_tool_output(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        tool_output_id: &str,
    ) -> Result<Option<Self::ToolOutput>, String> {
        SessionStore::get_session_tool_output(self, app_data_dir, session_id, tool_output_id)
    }

    fn load_session_events(
        &self,
        app_data_dir: &Path,
        session_id: &str,
    ) -> Result<Vec<Self::Event>, String> {
        SessionStore::load_session_events(self, app_data_dir, session_id)
    }
}

impl SessionStore {
    fn list_metas_for_active_read_authority(
        &self,
        app_data_dir: &Path,
    ) -> Result<Vec<SessionMeta>, String> {
        #[cfg(test)]
        if !self.canonical_authority_active() {
            return self.test_storage().list_metas(app_data_dir);
        }
        self.list_metas_canonical(app_data_dir)
    }

    fn ensure_canonical_mutation_admission(&self) -> Result<(), String> {
        match self.event_authority.read().as_ref() {
            Some(_) => Ok(()),
            None => {
                #[cfg(test)]
                return Ok(());
                #[cfg(not(test))]
                return Err("agent-session SQLite event authority is not configured".to_string());
            }
        }
    }

    fn canonical_authority_active(&self) -> bool {
        self.event_authority.read().is_some()
    }

    #[cfg(test)]
    pub fn new(storage: Arc<dyn SessionStoragePort>) -> Self {
        Self {
            storage: Some(storage),
            event_authority: RwLock::new(None),
            state_change_listeners: RwLock::new(Vec::new()),
            event_log_recovery_listeners: RwLock::new(Vec::new()),
            runtime_terminal_participant_provider: RwLock::new(None),
            #[cfg(test)]
            permission_response_reservations: RwLock::new(HashMap::new()),
            #[cfg(test)]
            save_hook: RwLock::new(None),
            #[cfg(test)]
            append_message_hook: RwLock::new(None),
            #[cfg(test)]
            persist_parts_hook: RwLock::new(None),
            #[cfg(test)]
            append_event_hook: RwLock::new(None),
            #[cfg(test)]
            set_state_hook: RwLock::new(None),
            #[cfg(test)]
            projection_hook: RwLock::new(None),
            #[cfg(test)]
            appended_event_hook: RwLock::new(None),
            #[cfg(test)]
            event_projection_hook: RwLock::new(None),
            #[cfg(test)]
            atomic_event_commit_hook: RwLock::new(None),
            #[cfg(test)]
            backend_established_hook: RwLock::new(None),
            #[cfg(test)]
            projected_read_model_hook: RwLock::new(None),
        }
    }

    pub(crate) fn new_canonical(
        repository: Arc<dyn crate::domain::local_event::LocalEventTransactionRepository>,
        installation_id: String,
        projection_codec: Arc<dyn AgentSessionProjectionCodec>,
    ) -> Self {
        Self {
            #[cfg(test)]
            storage: None,
            event_authority: RwLock::new(Some(AgentSessionEventAuthority {
                repository,
                installation_id,
                projection_codec,
            })),
            state_change_listeners: RwLock::new(Vec::new()),
            event_log_recovery_listeners: RwLock::new(Vec::new()),
            runtime_terminal_participant_provider: RwLock::new(None),
            #[cfg(test)]
            permission_response_reservations: RwLock::new(HashMap::new()),
            #[cfg(test)]
            save_hook: RwLock::new(None),
            #[cfg(test)]
            append_message_hook: RwLock::new(None),
            #[cfg(test)]
            persist_parts_hook: RwLock::new(None),
            #[cfg(test)]
            append_event_hook: RwLock::new(None),
            #[cfg(test)]
            set_state_hook: RwLock::new(None),
            #[cfg(test)]
            projection_hook: RwLock::new(None),
            #[cfg(test)]
            appended_event_hook: RwLock::new(None),
            #[cfg(test)]
            event_projection_hook: RwLock::new(None),
            #[cfg(test)]
            atomic_event_commit_hook: RwLock::new(None),
            #[cfg(test)]
            backend_established_hook: RwLock::new(None),
            #[cfg(test)]
            projected_read_model_hook: RwLock::new(None),
        }
    }

    #[cfg(test)]
    fn test_storage(&self) -> &dyn SessionStoragePort {
        self.storage
            .as_deref()
            .expect("test file-session storage is not configured")
    }

    #[cfg(test)]
    pub(crate) fn set_local_event_repository(
        &self,
        repository: Arc<dyn crate::domain::local_event::LocalEventTransactionRepository>,
        installation_id: String,
        projection_codec: Arc<dyn AgentSessionProjectionCodec>,
    ) {
        *self.event_authority.write() = Some(AgentSessionEventAuthority {
            repository,
            installation_id,
            projection_codec,
        });
    }

    pub(crate) fn set_runtime_terminal_participant_provider(
        &self,
        provider: Arc<dyn RuntimeTerminalParticipantProvider>,
    ) {
        *self.runtime_terminal_participant_provider.write() = Some(provider);
    }

    /// Fail-closed admission fence for mutations that could start or drain a
    /// provider/workflow effect. The owner secondary index is the authority;
    /// this does not hydrate a session or infer recovery from live state.
    pub(crate) async fn ensure_no_unresolved_recovery(
        &self,
        owner: &str,
    ) -> Result<(), crate::domain::local_event::SafeOperationFailure> {
        use crate::domain::local_event::{
            LocalEventQuery, LocalEventQueryResult, SessionOperationFailureKind,
        };

        if owner.is_empty() {
            return Err(crate::domain::local_event::SafeOperationFailure::new(
                SessionOperationFailureKind::Internal,
                false,
                "The recovery owner identity is invalid.",
                "recovery-owner-invalid",
            ));
        }
        let authority = match self.event_authority.read().clone() {
            Some(authority) => authority,
            None => {
                // Legacy unit fixtures have no #1499 authority. Never extend
                // this bypass to a production build: missing canonical
                // recovery authority must close mutation admission.
                #[cfg(test)]
                return Ok(());
                #[cfg(not(test))]
                return Err(crate::domain::local_event::SafeOperationFailure::new(
                    SessionOperationFailureKind::StorageUnavailable,
                    true,
                    "The pending recovery authority is unavailable.",
                    format!("recovery-authority-{owner}"),
                ));
            }
        };
        let owner = owner.to_string();
        const PAGE_LIMIT: usize = 200;
        let mut cursor = None;
        loop {
            let result = authority
                .repository
                .query(LocalEventQuery::PendingRecoveryPage {
                    limit: PAGE_LIMIT,
                    partition: None,
                    owner: Some(owner.clone()),
                    ordered_key_prefix: None,
                    shutdown_plan: None,
                    cursor,
                })
                .await
                .map_err(|_| {
                    crate::domain::local_event::SafeOperationFailure::new(
                        SessionOperationFailureKind::StorageUnavailable,
                        true,
                        "The pending recovery inventory is unavailable.",
                        format!("recovery-inventory-{owner}"),
                    )
                })?;
            let LocalEventQueryResult::PendingRecoveryPage(page) = result else {
                return Err(crate::domain::local_event::SafeOperationFailure::new(
                    SessionOperationFailureKind::StorageCorrupt,
                    false,
                    "The pending recovery inventory is incompatible.",
                    format!("recovery-inventory-{owner}"),
                ));
            };
            for entry in page.entries {
                if entry.owner != owner {
                    return Err(crate::domain::local_event::SafeOperationFailure::new(
                        SessionOperationFailureKind::StorageCorrupt,
                        false,
                        "The pending recovery owner index is inconsistent.",
                        entry.obligation_id,
                    ));
                }
                if let Some(identity) = crate::usecase::agent_session::operation::recovery::unresolved_recovery_original_identity(
                    &entry.obligation_id,
					&entry.record,
                ) {
                    return Err(crate::domain::local_event::SafeOperationFailure::new(
                        SessionOperationFailureKind::OutcomeUnknown,
                        true,
                        "Unresolved recovery must be resolved before this operation.",
                        identity.clone(),
                    )
                    .with_detail(&format!("Pending recovery identity: {identity}")));
                }
            }
            cursor = page.next_cursor;
            if cursor.is_none() {
                return Ok(());
            }
        }
    }

    fn canonical_session_projection(
        &self,
        session_id: &str,
    ) -> Result<Option<CanonicalAgentSessionProjection>, String> {
        self.canonical_session_projection_with_revision(session_id)
            .map(|projection| projection.map(|(projection, _)| projection))
    }

    fn canonical_session_projection_with_revision(
        &self,
        session_id: &str,
    ) -> Result<
        Option<(
            CanonicalAgentSessionProjection,
            crate::domain::local_event::Revision,
        )>,
        String,
    > {
        let Some(authority) = self.event_authority.read().clone() else {
            return Ok(None);
        };
        let codec = authority.projection_codec.clone();
        let session_id = session_id.to_string();
        std::thread::scope(|scope| {
            scope
                .spawn(move || {
                    tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|error| {
                            format!("failed to create agent projection read runtime: {error}")
                        })?
                        .block_on(async move {
                            let result = authority
                                .repository
                                .query(crate::domain::local_event::LocalEventQuery::SessionProjectionByIdentity {
                                    session_id: session_id.clone(),
                                })
                                .await
                                .map_err(|error| {
                                    format!("agent SQLite projection read failed: {error}")
                                })?;
                            let crate::domain::local_event::LocalEventQueryResult::SessionProjectionByIdentity(
                                projection,
                            ) = result
                            else {
                                return Err("agent SQLite projection query returned the wrong shape".to_string());
                            };
                            projection
                                .map(|projection| {
                                    codec
                                        .decode(&projection.projection)
                                        .map(|decoded| (decoded, projection.revision))
                                })
                                .transpose()
                        })
                })
                .join()
                .map_err(|_| "agent SQLite projection read worker panicked".to_string())?
        })
    }

    /// Read the bounded durable queue identity projection in its canonical
    /// execution order. Retry payloads remain obligation-owned and are not
    /// retained here.
    pub(crate) fn canonical_pending_send_queue(
        &self,
        session_id: &str,
    ) -> Result<Vec<CanonicalQueuedSend>, String> {
        self.canonical_session_projection(session_id)?
            .map(|projection| projection.pending_send_queue)
            .ok_or_else(|| format!("Session projection not found: {session_id}"))
    }

    /// Check that a queued effect still names one exact durable queue entry.
    /// `input_ref` is optional because older accepted-effect DTOs do not carry
    /// it; callers that have the receipt should supply it for the full match.
    pub(crate) fn canonical_queue_contains_exact(
        &self,
        session_id: &str,
        queue_item_id: &str,
        human_message_id: &str,
        reserved_turn_id: &str,
        input_ref: Option<&str>,
    ) -> Result<bool, String> {
        Ok(self
            .canonical_pending_send_queue(session_id)?
            .iter()
            .any(|entry| {
                entry.queue_item_id == queue_item_id
                    && entry.human_message_id == human_message_id
                    && entry.reserved_turn_id == reserved_turn_id
                    && input_ref.is_none_or(|input_ref| entry.input_ref == input_ref)
            }))
    }

    fn canonical_obligation(
        &self,
        obligation_id: &str,
    ) -> Result<Option<crate::domain::local_event::ObligationView>, String> {
        let authority = self
            .event_authority
            .read()
            .clone()
            .ok_or_else(|| "agent-session SQLite authority is unavailable".to_string())?;
        let obligation_id = obligation_id.to_string();
        std::thread::scope(|scope| {
            scope
                .spawn(move || {
                    tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|error| {
                            format!("failed to create obligation read runtime: {error}")
                        })?
                        .block_on(async move {
                            match authority
                                .repository
                                .query(
                                    crate::domain::local_event::LocalEventQuery::ObligationByIdentity {
                                        obligation_id,
                                    },
                                )
                                .await
                                .map_err(|error| {
                                    format!("agent SQLite obligation read failed: {error}")
                                })?
                            {
                                crate::domain::local_event::LocalEventQueryResult::ObligationByIdentity(obligation) => Ok(obligation),
                                _ => Err("agent SQLite obligation query returned the wrong shape".to_string()),
                            }
                        })
                })
                .join()
                .map_err(|_| "obligation read worker panicked".to_string())?
        })
    }

    fn canonical_terminal(
        &self,
        session_id: &str,
        turn_id: u64,
    ) -> Result<Option<crate::domain::local_event::TerminalRecordView>, String> {
        let authority = self
            .event_authority
            .read()
            .clone()
            .ok_or_else(|| "agent-session SQLite authority is unavailable".to_string())?;
        let session_id = session_id.to_string();
        let turn_id = turn_id.to_string();
        std::thread::scope(|scope| {
            scope
                .spawn(move || {
                    tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|error| {
                            format!("failed to create terminal read runtime: {error}")
                        })?
                        .block_on(async move {
                            match authority
                                .repository
                                .query(crate::domain::local_event::LocalEventQuery::TerminalByTurn {
                                    session_id,
                                    turn_id,
                                })
                                .await
                                .map_err(|error| {
                                    format!("agent SQLite terminal read failed: {error}")
                                })?
                            {
                                crate::domain::local_event::LocalEventQueryResult::TerminalByTurn(terminal) => Ok(terminal),
                                _ => Err("agent SQLite terminal query returned the wrong shape".to_string()),
                            }
                        })
                })
                .join()
                .map_err(|_| "terminal read worker panicked".to_string())?
        })
    }

    pub(crate) fn canonical_message_projection(
        &self,
        session_id: &str,
        message_id: &str,
    ) -> Result<Option<ChatMessage>, String> {
        let Some(authority) = self.event_authority.read().clone() else {
            return Ok(None);
        };
        let codec = authority.projection_codec.clone();
        let session_id = session_id.to_string();
        let message_id = message_id.to_string();
        std::thread::scope(|scope| {
            scope
                .spawn(move || {
                    tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|error| {
                            format!("failed to create agent message read runtime: {error}")
                        })?
                        .block_on(async move {
                            let result = authority
                                .repository
                                .query(crate::domain::local_event::LocalEventQuery::MessageProjectionByIdentity {
                                    session_id: session_id.clone(),
                                    message_id: message_id.clone(),
                                })
                                .await
                                .map_err(|error| {
                                    format!("agent SQLite message projection read failed: {error}")
                                })?;
                            let crate::domain::local_event::LocalEventQueryResult::MessageProjectionByIdentity(
                                projection,
                            ) = result
                            else {
                                return Err("agent SQLite message projection query returned the wrong shape".to_string());
                            };
                            projection
                                .map(|projection| codec.decode_message(&projection.projection))
                                .transpose()
                        })
                })
                .join()
                .map_err(|_| "agent SQLite message read worker panicked".to_string())?
        })
    }

    /// Reads only the workflow turn-completion namespace from the pending
    /// index. Callers retain the returned cursor and decide how many bounded
    /// pages to replay; this method never falls back to a full inventory scan.
    pub(crate) fn pending_workflow_turn_completion_page(
        &self,
        owner: Option<&str>,
        turn_id: Option<u64>,
        limit: usize,
        cursor: Option<crate::domain::local_event::QueryCursor>,
    ) -> Result<PendingWorkflowTurnCompletionPage, String> {
        const MAX_PAGE: usize = 128;
        if limit == 0 || limit > MAX_PAGE || owner.is_some_and(str::is_empty) {
            return Err("workflow turn-completion page request is invalid".to_string());
        }
        if turn_id.is_some() && owner.is_none() {
            return Err(
                "workflow turn-completion turn lookup requires a session owner".to_string(),
            );
        }
        let authority = self
            .event_authority
            .read()
            .clone()
            .ok_or_else(|| "agent-session SQLite authority is unavailable".to_string())?;
        let codec = authority.projection_codec.clone();
        let owner = owner.map(str::to_string);
        let ordered_key_prefix = turn_id.map_or_else(
            || WORKFLOW_TURN_COMPLETION_ORDERED_KEY_PREFIX.to_string(),
            |turn_id| format!("{WORKFLOW_TURN_COMPLETION_ORDERED_KEY_PREFIX}{turn_id:020}:"),
        );
        std::thread::scope(|scope| {
            scope
                .spawn(move || {
                    tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|error| {
                            format!(
                                "failed to create workflow turn-completion read runtime: {error}"
                            )
                        })?
                        .block_on(async move {
                            let result = authority
                                .repository
                                .query(
                                    crate::domain::local_event::LocalEventQuery::PendingRecoveryPage {
                                        limit,
                                        // Prefix queries are already an exact
                                        // bounded namespace and the closed
                                        // query contract treats partition and
                                        // owner as mutually exclusive. Each
                                        // decoded row is still required to be
                                        // in Owner below.
                                        partition: None,
                                        owner,
                                        ordered_key_prefix: Some(ordered_key_prefix),
                                        shutdown_plan: None,
                                        cursor,
                                    },
                                )
                                .await
                                .map_err(|error| {
                                    format!(
                                        "workflow turn-completion pending read failed: {error}"
                                    )
                                })?;
                            let crate::domain::local_event::LocalEventQueryResult::PendingRecoveryPage(
                                page,
                            ) = result
                            else {
                                return Err(
                                    "workflow turn-completion pending query returned the wrong shape"
                                        .to_string(),
                                );
                            };
                            let mut entries = Vec::with_capacity(page.entries.len());
                            for stored in page.entries {
                                let crate::domain::local_event::ObligationRecord::WorkflowTurnCompletion {
                                    session_id,
                                    turn_id,
                                    terminal_identity,
                                    notification_sha256,
                                    detail:
                                        crate::domain::local_event::WorkflowTurnCompletionObligationRecord::Pending {
                                            workflow_context,
                                            message_id,
                                            exit_code,
                                            failure_signal,
                                            token_usage,
                                            interrupted,
                                        },
                                    state: crate::domain::local_event::ObligationStateRecord::Pending,
                                } = stored.record
                                else {
                                    return Err(
                                        "completed workflow turn-completion obligation remained pending"
                                            .to_string(),
                                    );
                                };
                                let parsed_turn_id = turn_id.parse::<u64>().map_err(|_| {
                                    "workflow turn-completion turn identity is invalid".to_string()
                                })?;
                                let notification_sha256 = hex::encode(notification_sha256);
                                let workflow_context = workflow_context_to_dto(&workflow_context);
                                if stored.owner != session_id
                                    || stored.partition
                                        != crate::domain::local_event::PendingPartition::Owner
                                    || stored.shutdown_plan.is_some()
                                    || stored.obligation_id
                                        != workflow_turn_completion_obligation_id(
                                            &notification_sha256,
                                        )
                                    || stored.ordered_key
                                        != workflow_turn_completion_ordered_key(
                                            parsed_turn_id,
                                            &notification_sha256,
                                        )
                                {
                                    return Err(
                                        "workflow turn-completion obligation identity is inconsistent"
                                            .to_string(),
                                    );
                                }
                                let terminal = authority
                                    .repository
                                    .query(
                                        crate::domain::local_event::LocalEventQuery::TerminalByTurn {
                                            session_id: session_id.clone(),
                                            turn_id: turn_id.clone(),
                                        },
                                    )
                                    .await
                                    .map_err(|error| {
                                        format!(
                                            "workflow turn-completion terminal read failed: {error}"
                                        )
                                    })?;
                                let crate::domain::local_event::LocalEventQueryResult::TerminalByTurn(
                                    Some(terminal),
                                ) = terminal
                                else {
                                    return Err(
                                        "workflow turn-completion terminal record is missing"
                                            .to_string(),
                                    );
                                };
                                if terminal.session_id != session_id
                                    || terminal.turn_id != turn_id
                                    || terminal.terminal_identity != terminal_identity
                                {
                                    return Err(
                                        "workflow turn-completion terminal record is inconsistent"
                                            .to_string(),
                                    );
                                }
                                if !matches!(
                                    &terminal.result,
                                    crate::domain::local_event::TerminalResultRecord::AgentTurn {
                                        session_id: terminal_session_id,
                                        turn_id: terminal_turn_id,
                                        message_id: terminal_message_id,
                                        ..
                                    } if terminal_session_id == &session_id
                                        && terminal_turn_id == &turn_id
                                        && terminal_message_id == &message_id
                                ) {
                                    return Err(
                                        "workflow turn-completion message reference is inconsistent"
                                            .to_string(),
                                    );
                                }
                                let message = authority
                                    .repository
                                    .query(
                                        crate::domain::local_event::LocalEventQuery::MessageProjectionByIdentity {
                                            session_id: session_id.clone(),
                                            message_id: message_id.clone(),
                                        },
                                    )
                                    .await
                                    .map_err(|error| {
                                        format!(
                                            "workflow turn-completion message read failed: {error}"
                                        )
                                    })?;
                                let crate::domain::local_event::LocalEventQueryResult::MessageProjectionByIdentity(
                                    Some(message),
                                ) = message
                                else {
                                    return Err(
                                        "workflow turn-completion message projection is missing"
                                            .to_string(),
                                    );
                                };
                                let message = codec.decode_message(&message.projection)?;
                                if message.id != message_id || message.role != MessageRole::Agent {
                                    return Err(
                                        "workflow turn-completion message projection is inconsistent"
                                            .to_string(),
                                    );
                                }
                                let input = WorkflowTurnCompleteInput {
                                    turn_id: parsed_turn_id,
                                    exit_code,
                                    final_text_parts: workflow_final_text_parts(&message),
                                    failure_signal: failure_signal.map(|signal| match signal {
                                        crate::domain::local_event::WorkflowTurnFailureSignalRecord::ModelRefusal => {
                                            AgentTurnFailureSignal::ModelRefusal
                                        }
                                    }),
                                    token_usage,
                                    interrupted,
                                };
                                let expected_sha256 =
                                    workflow_turn_completion_notification_sha256(
                                        &session_id,
                                        &workflow_context,
                                        &terminal_identity,
                                        &message_id,
                                        &input,
                                    );
                                if expected_sha256 != notification_sha256 {
                                    return Err(
                                        "workflow turn-completion notification binding is inconsistent"
                                            .to_string(),
                                    );
                                }
                                entries.push(PendingWorkflowTurnCompletion {
                                    obligation_id: stored.obligation_id,
                                    revision: stored.revision,
                                    session_id,
                                    workflow_context,
                                    input,
                                    terminal_identity,
                                    message_id,
                                    notification_sha256,
                                });
                            }
                            Ok(PendingWorkflowTurnCompletionPage {
                                entries,
                                next_cursor: page.next_cursor,
                            })
                        })
                })
                .join()
                .map_err(|_| {
                    "workflow turn-completion read worker panicked".to_string()
                })?
        })
    }

    #[cfg(test)]
    pub(crate) fn pending_workflow_turn_completion(
        &self,
        session_id: &str,
        turn_id: u64,
    ) -> Result<Option<PendingWorkflowTurnCompletion>, String> {
        let page =
            self.pending_workflow_turn_completion_page(Some(session_id), Some(turn_id), 2, None)?;
        if page.next_cursor.is_some() || page.entries.len() > 1 {
            return Err(
                "multiple pending workflow turn-completions exist for one session turn".to_string(),
            );
        }
        Ok(page.entries.into_iter().next())
    }

    /// Removes the pending membership only after the workflow side has
    /// durably accepted this exact notification. Replays of the same consume
    /// are successful; a different binding fails closed.
    pub(crate) fn complete_workflow_turn_completion(
        &self,
        entry: &PendingWorkflowTurnCompletion,
    ) -> Result<(), String> {
        let expected_sha256 = workflow_turn_completion_notification_sha256(
            &entry.session_id,
            &entry.workflow_context,
            &entry.terminal_identity,
            &entry.message_id,
            &entry.input,
        );
        if expected_sha256 != entry.notification_sha256
            || entry.obligation_id
                != workflow_turn_completion_obligation_id(&entry.notification_sha256)
        {
            return Err("workflow turn-completion consume binding is inconsistent".to_string());
        }
        let current = self
            .canonical_obligation(&entry.obligation_id)?
            .ok_or_else(|| {
                "workflow turn-completion obligation disappeared before consume".to_string()
            })?;
        let notification_digest: [u8; 32] = hex::decode(&entry.notification_sha256)
            .map_err(|_| "workflow turn-completion digest is invalid".to_string())?
            .try_into()
            .map_err(|_| "workflow turn-completion digest has an invalid length".to_string())?;
        match &current.record {
            crate::domain::local_event::ObligationRecord::WorkflowTurnCompletion {
                session_id,
                turn_id,
                terminal_identity,
                notification_sha256,
                detail:
                    crate::domain::local_event::WorkflowTurnCompletionObligationRecord::Completed {
                        ..
                    },
                state: crate::domain::local_event::ObligationStateRecord::Completed,
            } => {
                if session_id == &entry.session_id
                    && turn_id.parse::<u64>().ok() == Some(entry.input.turn_id)
                    && terminal_identity == &entry.terminal_identity
                    && notification_sha256 == &notification_digest
                    && current.pending.is_none()
                {
                    return Ok(());
                }
                return Err(
                    "completed workflow turn-completion obligation is inconsistent".to_string(),
                );
            }
            crate::domain::local_event::ObligationRecord::WorkflowTurnCompletion {
                session_id,
                turn_id,
                terminal_identity,
                notification_sha256,
                detail:
                    crate::domain::local_event::WorkflowTurnCompletionObligationRecord::Pending {
                        workflow_context,
                        message_id,
                        exit_code,
                        failure_signal,
                        token_usage,
                        interrupted,
                    },
                state: crate::domain::local_event::ObligationStateRecord::Pending,
            } => {
                let stored_failure_signal = failure_signal.map(|signal| match signal {
                    crate::domain::local_event::WorkflowTurnFailureSignalRecord::ModelRefusal => {
                        AgentTurnFailureSignal::ModelRefusal
                    }
                });
                if session_id != &entry.session_id
                    || workflow_context.as_ref()
                        != &workflow_context_to_domain(&entry.workflow_context)
                    || turn_id.parse::<u64>().ok() != Some(entry.input.turn_id)
                    || terminal_identity != &entry.terminal_identity
                    || message_id != &entry.message_id
                    || *exit_code != entry.input.exit_code
                    || stored_failure_signal != entry.input.failure_signal
                    || *token_usage != entry.input.token_usage
                    || *interrupted != entry.input.interrupted
                    || notification_sha256 != &notification_digest
                    || current.pending.is_none()
                    || current.revision != entry.revision
                {
                    return Err(
                        "pending workflow turn-completion obligation is inconsistent".to_string(),
                    );
                }
            }
            _ => {
                return Err(
                    "workflow turn-completion obligation has an incompatible kind or state"
                        .to_string(),
                );
            }
        }
        let record = crate::domain::local_event::ObligationRecord::WorkflowTurnCompletion {
            session_id: entry.session_id.clone(),
            turn_id: entry.input.turn_id.to_string(),
            terminal_identity: entry.terminal_identity.clone(),
            notification_sha256: notification_digest,
            detail: crate::domain::local_event::WorkflowTurnCompletionObligationRecord::Completed {
                completed_at_bits: now_timestamp().to_bits(),
            },
            state: crate::domain::local_event::ObligationStateRecord::Completed,
        };
        let mutation = crate::domain::local_event::LocalStateMutation::Obligation(
            crate::domain::local_event::ObligationMutation {
                obligation_id: entry.obligation_id.clone(),
                record,
                pending: None,
                expected: crate::domain::local_event::RevisionGuard::Expected(current.revision),
                revision: current.revision.next().ok_or_else(|| {
                    "workflow turn-completion obligation revision exhausted".to_string()
                })?,
            },
        );
        let mutation_identity = mutation.canonical_identity_v1().map_err(str::to_string)?;
        let payload_hash: [u8; 32] = Sha256::digest(&mutation_identity).into();
        let mut commit_hasher = Sha256::new();
        hash_identity_field(
            &mut commit_hasher,
            b"workflow_turn_completion_consume_commit_v1",
        );
        hash_identity_field(&mut commit_hasher, entry.obligation_id.as_bytes());
        commit_hasher.update(current.revision.value().to_be_bytes());
        commit_hasher.update(payload_hash);
        let commit_identity = hex::encode(commit_hasher.finalize());
        let authority = self
            .event_authority
            .read()
            .clone()
            .ok_or_else(|| "agent-session SQLite authority is unavailable".to_string())?;
        let idempotency_key = format!(
            "workflow-turn-complete.consume:{}",
            entry.notification_sha256
        );
        let obligation_id = entry.obligation_id.clone();
        let commit_result = std::thread::scope(|scope| {
            scope
                .spawn(move || {
                    tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|error| {
                            format!(
                                "failed to create workflow turn-completion consume runtime: {error}"
                            )
                        })?
                        .block_on(async move {
                            let commit_id =
                                crate::domain::local_event::CommitIdentity::parse(&commit_identity)
                                    .map_err(|_| {
                                        "workflow turn-completion consume identity is invalid"
                                            .to_string()
                                    })?;
                            let batch = crate::domain::local_event::LocalAtomicBatch {
                                commit_id: commit_id.clone(),
                                idempotency: crate::domain::local_event::IdempotencyBinding {
                                    installation_id: authority.installation_id.clone(),
                                    operation_kind:
                                        crate::domain::local_event::CommitOperationKind::Recovery,
                                    idempotency_key,
                                    payload_hash,
                                },
                                expected_heads: Vec::new(),
                                events: Vec::new(),
                                state_mutations: vec![mutation],
                            };
                            match authority.repository.commit_batch(batch).await {
                                Ok(_) => Ok(()),
                                Err(
                                    crate::domain::local_event::CommitBatchError::OutcomeUnknown {
                                        identity,
                                    },
                                ) => match authority.repository.resolve_commit(identity).await {
                                    Ok(crate::domain::local_event::CommitResolution::Committed(_)) => {
                                        Ok(())
                                    }
                                    Ok(crate::domain::local_event::CommitResolution::NotCommitted) => {
                                        Err("workflow turn-completion consume was not committed"
                                            .to_string())
                                    }
                                    Err(error) => Err(format!(
                                        "workflow turn-completion consume outcome could not be resolved: {error}"
                                    )),
                                },
                                Err(error) => Err(format!(
                                    "workflow turn-completion consume failed: {error}"
                                )),
                            }
                        })
                })
                .join()
                .map_err(|_| {
                    "workflow turn-completion consume worker panicked".to_string()
                })?
        });
        if let Err(error) = commit_result {
            if let Some(current) = self.canonical_obligation(&obligation_id)? {
                if let crate::domain::local_event::ObligationRecord::WorkflowTurnCompletion {
                    session_id,
                    turn_id,
                    terminal_identity,
                    notification_sha256,
                    detail:
                        crate::domain::local_event::WorkflowTurnCompletionObligationRecord::Completed {
                            ..
                        },
                    state: crate::domain::local_event::ObligationStateRecord::Completed,
                } = current.record
                {
                    if session_id == entry.session_id
                        && turn_id.parse::<u64>().ok() == Some(entry.input.turn_id)
                        && terminal_identity == entry.terminal_identity
                        && notification_sha256 == notification_digest
                        && current.pending.is_none()
                    {
                        return Ok(());
                    }
                }
            }
            return Err(error);
        }
        Ok(())
    }

    fn canonical_content_blob(
        &self,
        session_id: &str,
        identity: String,
    ) -> Result<Option<crate::domain::local_event::AgentContentBlobRecord>, String> {
        let authority = self
            .event_authority
            .read()
            .clone()
            .ok_or_else(|| "agent-session SQLite authority is unavailable".to_string())?;
        let blob_session_id = format!("blob:{session_id}");
        std::thread::scope(|scope| {
            scope
                .spawn(move || {
                    tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|error| {
                            format!("failed to create content blob read runtime: {error}")
                        })?
                        .block_on(async move {
                            let result = authority
                                .repository
                                .query(crate::domain::local_event::LocalEventQuery::MessageProjectionByIdentity {
                                    session_id: blob_session_id,
                                    message_id: identity,
                                })
                                .await
                                .map_err(|error| format!("SQLite content blob read failed: {error}"))?;
                            let crate::domain::local_event::LocalEventQueryResult::MessageProjectionByIdentity(record) = result else {
                                return Err("SQLite content blob query returned the wrong shape".to_string());
                            };
                            record
                                .map(|record| match record.projection {
                                    crate::domain::local_event::MessageProjectionRecord::AgentContentBlob(blob) => Ok(blob),
                                    _ => Err("SQLite content blob is incompatible".to_string()),
                                })
                                .transpose()
                        })
                })
                .join()
                .map_err(|_| "SQLite content blob read worker panicked".to_string())?
        })
    }

    fn list_metas_canonical(&self, _app_data_dir: &Path) -> Result<Vec<SessionMeta>, String> {
        let authority = self.event_authority.read().clone().ok_or_else(|| {
            "agent-session SQLite projection authority is unavailable".to_string()
        })?;
        let codec = authority.projection_codec.clone();
        std::thread::scope(|scope| {
            scope
                .spawn(move || {
                    tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|error| {
                            format!("failed to create agent projection page runtime: {error}")
                        })?
                        .block_on(async move {
                            let mut after_session_id = None;
                            let mut metas = Vec::new();
                            loop {
                                let result = authority
                                    .repository
                                    .query(crate::domain::local_event::LocalEventQuery::SessionProjectionPage {
                                        limit: 200,
                                        after_session_id: after_session_id.clone(),
                                    })
                                    .await
                                    .map_err(|error| {
                                        format!("agent SQLite projection page read failed: {error}")
                                    })?;
                                let crate::domain::local_event::LocalEventQueryResult::SessionProjectionPage(
                                    page,
                                ) = result
                                else {
                                    return Err("agent SQLite projection page query returned the wrong shape".to_string());
                                };
                                let page_len = page.len();
                                for projection in page {
                                    after_session_id = Some(projection.session_id);
                                    if !matches!(
                                        &projection.projection,
                                        crate::domain::local_event::SessionProjectionRecord::AgentSession(_)
                                    ) {
                                        continue;
                                    }
                                    metas.push(codec.decode(&projection.projection)?.meta);
                                }
                                if page_len < 200 {
                                    break;
                                }
                            }
                            Ok(metas)
                        })
                })
                .join()
                .map_err(|_| "agent SQLite projection page worker panicked".to_string())?
        })
    }

    fn canonical_message_page(
        &self,
        session_id: &str,
        cursor: Option<PageCursor>,
        limit: usize,
    ) -> Result<SessionPage, String> {
        let authority = self.event_authority.read().clone().ok_or_else(|| {
            "agent-session SQLite projection authority is unavailable".to_string()
        })?;
        let codec = authority.projection_codec.clone();
        let session_id = session_id.to_string();
        let before_position = cursor
            .map(|cursor| i64::try_from(cursor.0))
            .transpose()
            .map_err(|_| "agent message page cursor exceeds i64::MAX".to_string())?;
        std::thread::scope(|scope| {
            scope
                .spawn(move || {
                    tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|error| {
                            format!("failed to create agent message page runtime: {error}")
                        })?
                        .block_on(async move {
                            let result = authority
                                .repository
                                .query(crate::domain::local_event::LocalEventQuery::MessageProjectionPage {
                                    session_id: session_id.clone(),
                                    before_position,
                                    limit,
                                })
                                .await
                                .map_err(|error| {
                                    format!("agent SQLite message page read failed: {error}")
                                })?;
                            let crate::domain::local_event::LocalEventQueryResult::MessageProjectionPage(
                                page,
                            ) = result
                            else {
                                return Err("agent SQLite message page query returned the wrong shape".to_string());
                            };
                            let mut messages = Vec::with_capacity(page.entries.len());
                            let mut message_metadata = Vec::with_capacity(page.entries.len());
                            for entry in page.entries {
                                let message = codec.decode_message(&entry.message.projection)?;
                                message_metadata.push(MessagePageMetadata {
                                    message_id: message.id.clone(),
                                    token_meta: None,
                                    run_meta: None,
                                });
                                messages.push(message);
                            }
                            let latest_token_usage = match authority
                                .repository
                                .query(crate::domain::local_event::LocalEventQuery::SessionProjectionByIdentity {
                                    session_id,
                                })
                                .await
                                .map_err(|error| {
                                    format!("agent SQLite projection read failed: {error}")
                                })?
                            {
                                crate::domain::local_event::LocalEventQueryResult::SessionProjectionByIdentity(Some(projection)) => {
                                    codec.decode(&projection.projection)?.latest_token_usage
                                }
                                crate::domain::local_event::LocalEventQueryResult::SessionProjectionByIdentity(None) => None,
                                _ => return Err("agent SQLite projection query returned the wrong shape".to_string()),
                            };
                            let next_cursor = page
                                .next_before_position
                                .map(|position| {
                                    u64::try_from(position)
                                        .map(PageCursor)
                                        .map_err(|_| "agent message page cursor is invalid".to_string())
                                })
                                .transpose()?;
                            Ok(SessionPage {
                                messages,
                                message_metadata,
                                has_more: next_cursor.is_some(),
                                next_cursor,
                                total_count: page.total_count,
                                latest_token_usage,
                            })
                        })
                })
                .join()
                .map_err(|_| "agent SQLite message page worker panicked".to_string())?
        })
    }

    fn canonical_all_messages(&self, session_id: &str) -> Result<Vec<ChatMessage>, String> {
        let mut cursor = None;
        let mut chunks = Vec::new();
        loop {
            let page = self.canonical_message_page(session_id, cursor, 200)?;
            let next = page.next_cursor;
            chunks.push(page.messages);
            let Some(next) = next else {
                break;
            };
            cursor = Some(next);
        }
        chunks.reverse();
        Ok(chunks.into_iter().flatten().collect())
    }

    fn commit_agent_events(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        events: &[AgentSessionEvent],
    ) -> Result<(), String> {
        self.commit_agent_events_with_kind_and_queue_front(
            app_data_dir,
            session_id,
            events,
            crate::domain::local_event::CommitOperationKind::Projection,
            None,
            Vec::new(),
            None,
        )
    }

    fn commit_agent_events_with_queue_pause_guard(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        events: &[AgentSessionEvent],
        expected_queue_paused: bool,
    ) -> Result<(), String> {
        self.commit_agent_events_with_kind_and_queue_front(
            app_data_dir,
            session_id,
            events,
            crate::domain::local_event::CommitOperationKind::Projection,
            None,
            Vec::new(),
            Some(expected_queue_paused),
        )
    }

    fn commit_agent_events_with_kind(
        &self,
        _app_data_dir: &Path,
        session_id: &str,
        events: &[AgentSessionEvent],
        operation_kind: crate::domain::local_event::CommitOperationKind,
    ) -> Result<(), String> {
        self.commit_agent_events_with_kind_and_queue_front(
            _app_data_dir,
            session_id,
            events,
            operation_kind,
            None,
            Vec::new(),
            None,
        )
    }

    #[allow(clippy::too_many_arguments)] // One atomic projection boundary receives each guard and participant explicitly.
    fn commit_agent_events_with_kind_and_queue_front(
        &self,
        _app_data_dir: &Path,
        session_id: &str,
        events: &[AgentSessionEvent],
        operation_kind: crate::domain::local_event::CommitOperationKind,
        expected_queue_front: Option<ExpectedAcceptedQueueFront>,
        additional_mutations: Vec<crate::domain::local_event::LocalStateMutation>,
        expected_queue_paused: Option<bool>,
    ) -> Result<(), String> {
        if events.is_empty() {
            return Ok(());
        }
        let Some(authority) = self.event_authority.read().clone() else {
            #[cfg(test)]
            return Ok(());
            #[cfg(not(test))]
            return Err("agent-session SQLite event authority is not configured".to_string());
        };
        let fallback_meta = Some(
            self.canonical_session_projection(session_id)?
                .ok_or_else(|| format!("Session projection not found: {session_id}"))?
                .meta,
        );
        #[cfg(test)]
        let atomic_event_commit_hook = self.atomic_event_commit_hook.read().clone();
        let session_id = session_id.to_string();
        let events = events.to_vec();
        let expected_queue_front = expected_queue_front.clone();
        let additional_mutations = additional_mutations.clone();
        std::thread::scope(|scope| {
            scope
                .spawn(move || {
                    tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|error| {
                            format!("failed to create agent event commit runtime: {error}")
                        })?
                        .block_on(async move {
                            let stream_id = crate::domain::local_event::StreamId::agent_session(
                                &session_id,
                            )
                            .map_err(|_| "agent session stream identity is invalid".to_string())?;
                            let mut exact = Sha256::new();
                            hash_identity_field(&mut exact, b"agent_event_commit_identity_v1");
                            hash_identity_field(&mut exact, session_id.as_bytes());
                            let codec = authority.projection_codec.as_ref();
                            let encoded_events = codec.encode_events_for_identity(&events)?;
                            hash_identity_field(&mut exact, &encoded_events);
                            for mutation in &additional_mutations {
                                let encoded = authority
                                    .repository
                                    .canonical_mutation_identity_v1(mutation)?;
                                hash_identity_field(&mut exact, &encoded);
                            }
                            let payload_hash: [u8; 32] = exact.finalize().into();
                            for _ in 0..4 {
                                // Stream head metadata is returned with every page. A one-row
                                // read is sufficient for optimistic append; normal mutation state
                                // comes from the point-addressed session projection below.
                                let head = authority
                                    .repository
                                    .load_stream(crate::domain::local_event::LoadStreamRequest {
                                        stream_id: stream_id.clone(),
                                        after: None,
                                        limit: 1,
                                    })
                                    .await
                                    .map_err(|error| {
                                        format!("agent SQLite head read failed: {error}")
                                    })?
                                    .head;
                                let mut state_mutations = Vec::new();
                                {
                                    let codec = authority.projection_codec.as_ref();
                                    let result = authority
                                        .repository
                                        .query(crate::domain::local_event::LocalEventQuery::SessionProjectionByIdentity {
                                            session_id: session_id.clone(),
                                        })
                                        .await
                                        .map_err(|error| {
                                            format!("agent SQLite projection read failed: {error}")
                                        })?;
                                    let crate::domain::local_event::LocalEventQueryResult::SessionProjectionByIdentity(
                                        stored,
                                    ) = result
                                    else {
                                        return Err("agent SQLite projection query returned the wrong shape".to_string());
                                    };
                                    let (
                                        mut meta,
                                        title,
                                        mut reducer_events,
                                        queue_paused_at,
                                        mut pending_send_queue,
                                        expected,
                                        revision,
                                    ) =
                                        match stored {
                                        Some(stored) => {
                                            let decoded = codec.decode(&stored.projection)?;
                                            let next = stored.revision.next().ok_or_else(|| {
                                                "agent projection revision exhausted".to_string()
                                            })?;
                                            (
                                                decoded.meta,
                                                decoded.title,
                                                decoded.reducer_events,
                                                decoded.queue_paused_at,
                                                decoded.pending_send_queue,
                                                crate::domain::local_event::RevisionGuard::Expected(stored.revision),
                                                next,
                                            )
                                        }
                                        None => (
                                            fallback_meta.clone().ok_or_else(|| {
                                                "agent projection has no initialization metadata".to_string()
                                            })?,
                                            None,
                                            Vec::new(),
                                            None,
                                            Vec::new(),
                                            crate::domain::local_event::RevisionGuard::Absent,
                                            crate::domain::local_event::Revision::new(0)
                                                .expect("zero revision"),
                                        ),
                                    };
                                    if expected_queue_paused.is_some_and(|expected| {
                                        expected != queue_paused_at.is_some()
                                    }) {
                                        return Err(
                                            "queue-pause authority changed before guarded event commit; retry"
                                                .to_string(),
                                        );
                                    }
                                    if expected_queue_front.is_some() {
                                        if queue_paused_at.is_some() {
                                            return Err(
                                                format!(
                                                    "{ACCEPTED_QUEUE_START_BLOCKED}: canonical queue is paused"
                                                ),
                                            );
                                        }
                                        if !matches!(
                                            &meta.state,
                                            SessionState::Idle
                                                | SessionState::Done
                                                | SessionState::Error
                                        ) {
                                            return Err(format!(
                                                "{ACCEPTED_QUEUE_START_BLOCKED}: canonical session state is {:?}",
                                                meta.state
                                            ));
                                        }
                                        if TurnEventLog::from_events(reducer_events.clone())
                                            .project()
                                            .backend_recovery
                                            .is_some()
                                        {
                                            return Err(
                                                format!(
                                                    "{ACCEPTED_QUEUE_START_BLOCKED}: canonical backend recovery is active"
                                                ),
                                            );
                                        }
                                    }
                                    let mut previous_turn_id = meta.last_turn_id.unwrap_or(0);
                                    let mut consumed_expected_queue_front = false;
                                    for event in &events {
                                        if let AgentSessionEvent::TurnStarted {
                                            turn_id,
                                            message_id,
                                            ..
                                        } = event
                                        {
                                            if *turn_id <= previous_turn_id {
                                                return Err(format!(
                                                    "turn identity {turn_id} does not advance durable turn {previous_turn_id}"
                                                ));
                                            }
                                            match expected_queue_front.as_ref() {
                                                Some(expected_front) => {
                                                    if pending_send_queue
                                                        .iter()
                                                        .position(|entry| {
                                                            entry.queue_item_id
                                                                == expected_front.queue_item_id
                                                        })
                                                        .is_some_and(|position| position != 0)
                                                    {
                                                        return Err(format!(
                                                            "{ACCEPTED_QUEUE_START_BLOCKED}: another canonical queue item is first"
                                                        ));
                                                    }
                                                    let front = pending_send_queue.first().ok_or_else(
                                                        || {
                                                            "accepted queued turn has no canonical queue front"
                                                                .to_string()
                                                        },
                                                    )?;
                                                    let front_turn_id = front
                                                        .reserved_turn_id
                                                        .parse::<u64>()
                                                        .map_err(|_| {
                                                            "canonical queue front has an invalid turn identity"
                                                                .to_string()
                                                        })?;
                                                    if front.queue_item_id
                                                        != expected_front.queue_item_id
                                                        || front.human_message_id != *message_id
                                                        || front_turn_id != *turn_id
                                                    {
                                                        return Err(
                                                            "accepted queued turn does not match the canonical queue front"
                                                                .to_string(),
                                                        );
                                                    }
                                                    pending_send_queue.remove(0);
                                                    consumed_expected_queue_front = true;
                                                }
                                                None if !pending_send_queue.is_empty() => {
                                                    return Err(
                                                        "turn start cannot bypass the canonical queue front"
                                                            .to_string(),
                                                    );
                                                }
                                                None => {}
                                            }
                                            previous_turn_id = *turn_id;
                                        }
                                    }
                                    if expected_queue_front.is_some()
                                        && !consumed_expected_queue_front
                                    {
                                        return Err(
                                            "accepted queued turn commit omitted TurnStarted"
                                                .to_string(),
                                        );
                                    }
                                    reducer_events =
                                        bounded_reducer_events(reducer_events, &events);
                                    let last_turn_interruption =
                                        latest_turn_interruption(&reducer_events);
                                    let last_turn_id = reducer_events.iter().rev().find_map(
                                        |event| match event {
                                            AgentSessionEvent::TurnStarted { turn_id, .. } => {
                                                Some(*turn_id)
                                            }
                                            _ => None,
                                        },
                                    );
                                    let mut touched_message_ids = reducer_events
                                        .iter()
                                        .rev()
                                        .find_map(|event| match event {
                                            AgentSessionEvent::TurnStarted {
                                                message_id,
                                                assistant_message_id,
                                                ..
                                            } => Some([
                                                message_id.clone(),
                                                assistant_message_id.clone().unwrap_or_else(|| {
                                                    format!("{message_id}:agent")
                                                }),
                                            ]),
                                            _ => None,
                                        })
                                        .into_iter()
                                        .flatten()
                                        .collect::<HashSet<_>>();
                                    for event in &events {
                                        if let AgentSessionEvent::SessionErrored {
                                            message_id,
                                            ..
                                        } = event
                                        {
                                            touched_message_ids.insert(message_id.clone());
                                        }
                                    }
                                    let projected =
                                        TurnEventLog::from_events(reducer_events.clone()).project();
                                    meta.state = projected.status.session_state.clone();
                                    meta.error_reason = error_reason_for_state(
                                        &meta.state,
                                        &projected.error_reason,
                                    );
                                    meta.state_revision = next_sqlite_counter(
                                        meta.state_revision,
                                        "session state revision",
                                    )?;
                                    meta.last_turn_interruption = last_turn_interruption;
                                    meta.last_turn_id = last_turn_id;
                                    let latest_token_usage = projected
                                        .workflow_turn_complete
                                        .as_ref()
                                        .and_then(|turn| turn.token_usage)
                                        .map(|usage| TokenUsage {
                                            input_tokens: usage.input_tokens,
                                            output_tokens: usage.output_tokens,
                                            total_tokens: usage
                                                .input_tokens
                                                .checked_add(usage.output_tokens),
                                            context_window_tokens: None,
                                        });
                                    let mut inserted_messages = Vec::new();
                                    for message in projected.messages.iter().filter(|message| {
                                        touched_message_ids.contains(&message.id)
                                    }) {
                                        let encoded_message = codec.encode_message(message)?;
                                        let result = authority
                                            .repository
                                            .query(crate::domain::local_event::LocalEventQuery::MessageProjectionByIdentity {
                                                session_id: session_id.clone(),
                                                message_id: message.id.clone(),
                                            })
                                            .await
                                            .map_err(|error| {
                                                format!("agent SQLite message projection read failed: {error}")
                                            })?;
                                        let crate::domain::local_event::LocalEventQueryResult::MessageProjectionByIdentity(
                                            stored,
                                        ) = result
                                        else {
                                            return Err("agent SQLite message projection query returned the wrong shape".to_string());
                                        };
                                        if stored.as_ref().is_some_and(|stored| {
                                            stored.projection == encoded_message
                                        }) {
                                            continue;
                                        }
                                        let (expected, revision) = match stored {
                                            Some(stored) => (
                                                crate::domain::local_event::RevisionGuard::Expected(stored.revision),
                                                stored.revision.next().ok_or_else(|| {
                                                    "agent message projection revision exhausted".to_string()
                                                })?,
                                            ),
                                            None => {
                                                inserted_messages.push(message.clone());
                                                (
                                                    crate::domain::local_event::RevisionGuard::Absent,
                                                    crate::domain::local_event::Revision::new(0)
                                                        .expect("zero revision"),
                                                )
                                            }
                                        };
                                        state_mutations.push(
                                            crate::domain::local_event::LocalStateMutation::MessageProjection(
                                                crate::domain::local_event::MessageProjectionMutation {
                                                    session_id: session_id.clone(),
                                                    message_id: message.id.clone(),
                                                    projection: encoded_message,
                                                    expected,
                                                    revision,
                                                },
                                            ),
                                        );
                                    }
                                    if !inserted_messages.is_empty() {
                                        meta.message_count = add_sqlite_count(
                                            meta.message_count,
                                            inserted_messages.len(),
                                            "session message count",
                                        )?;
                                        if meta.first_message_preview.is_empty() {
                                            meta.first_message_preview =
                                                super::first_message_preview(&inserted_messages);
                                        }
                                    }
                                    let projection = codec.encode(
                                        &CanonicalAgentSessionProjection {
                                            meta,
                                            title,
                                            messages: Vec::new(),
                                            reducer_events,
                                            queue_paused_at: projected.queue_paused_at,
                                            latest_token_usage,
                                            pending_send_queue,
                                        },
                                    )?;
                                    state_mutations.insert(
                                        0,
                                        crate::domain::local_event::LocalStateMutation::SessionProjection(
                                            crate::domain::local_event::SessionProjectionMutation {
                                                session_id: session_id.clone(),
                                                projection,
                                                expected,
                                                revision,
                                            },
                                        ),
                                    );
                                }
                                state_mutations.extend(additional_mutations.clone());
                                let identity = format!(
                                    "session-event-{}",
                                    hex::encode(payload_hash)
                                );
                                let occurred_at_ms = (now_timestamp() * 1000.0).round() as i64;
                                #[cfg(test)]
                                if let Some(hook) = &atomic_event_commit_hook {
                                    hook(operation_kind)?;
                                }
                                let batch = crate::domain::local_event::LocalAtomicBatch {
                                    commit_id: crate::domain::local_event::CommitIdentity::parse(
                                        &identity,
                                    )
                                    .map_err(|_| {
                                        "agent event commit identity is invalid".to_string()
                                    })?,
                                    idempotency: crate::domain::local_event::IdempotencyBinding {
                                        installation_id: authority.installation_id.clone(),
                                        operation_kind,
                                        idempotency_key: hex::encode(payload_hash),
                                        payload_hash,
                                    },
                                    expected_heads: vec![
                                        crate::domain::local_event::ExpectedStreamHead {
                                            stream_id: stream_id.clone(),
                                            expected: head,
                                        },
                                    ],
                                    events: events
                                        .iter()
                                        .cloned()
                                        .map(|event| {
                                            crate::domain::local_event::UncommittedDomainEvent {
                                                stream_id: stream_id.clone(),
                                                event: crate::domain::local_event::LocalDomainEvent::AgentSession(event),
                                                occurred_at_ms,
                                            }
                                        })
                                        .collect(),
                                    state_mutations,
                                };
                                match authority.repository.commit_batch(batch).await {
                                    Ok(_) => return Ok(()),
                                    Err(crate::domain::local_event::CommitBatchError::EffectAdmissionBlocked)
                                        if expected_queue_front.is_some() =>
                                    {
                                        return Err(format!(
                                            "{ACCEPTED_QUEUE_START_BLOCKED}: unresolved owner recovery is active"
                                        ));
                                    }
                                    Err(crate::domain::local_event::CommitBatchError::StreamHeadConflict { .. })
                                        if expected_queue_paused.is_some() =>
                                    {
                                        return Err(
                                            "queue-pause authority changed during guarded event commit; retry"
                                                .to_string(),
                                        );
                                    }
                                    Err(crate::domain::local_event::CommitBatchError::StreamHeadConflict { .. }) => continue,
                                    Err(crate::domain::local_event::CommitBatchError::PayloadConflict)
                                        if expected_queue_paused.is_some() =>
                                    {
                                        return Err(
                                            "queue-pause authority changed during guarded event commit; retry"
                                                .to_string(),
                                        );
                                    }
                                    Err(crate::domain::local_event::CommitBatchError::PayloadConflict)
                                        if expected_queue_front.is_some() => continue,
                                    Err(crate::domain::local_event::CommitBatchError::OutcomeUnknown { identity }) => {
                                        match authority.repository.resolve_commit(identity).await {
                                            Ok(crate::domain::local_event::CommitResolution::Committed(_)) => return Ok(()),
                                            Ok(crate::domain::local_event::CommitResolution::NotCommitted) => continue,
                                            Err(error) => {
                                                return Err(format!(
                                                    "accepted queued turn commit outcome could not be resolved: {error}"
                                                ));
                                            }
                                        }
                                    }
                                    Err(error) => {
                                        return Err(format!(
                                            "agent SQLite event commit failed: {error}"
                                        ));
                                    }
                                }
                            }
                            Err("agent SQLite event commit remained contended".to_string())
                        })
                })
                .join()
                .map_err(|_| "agent SQLite event commit worker panicked".to_string())?
        })
    }

    #[allow(clippy::too_many_arguments)] // One transaction boundary receives every atomic participant explicitly.
    fn commit_agent_events_with_additional_mutations(
        &self,
        session_id: &str,
        events: &[AgentSessionEvent],
        additional_mutations: Vec<crate::domain::local_event::LocalStateMutation>,
        terminal_message_patch: Option<TerminalMessageProjectionPatch>,
        projection_meta_patch: Option<EventProjectionMetaPatch>,
        terminal_participant: Option<(
            Arc<dyn RuntimeTerminalParticipantProvider>,
            crate::domain::local_event::TerminalRecordMutation,
        )>,
        operation_kind: crate::domain::local_event::CommitOperationKind,
    ) -> Result<(), String> {
        if events.is_empty()
            && additional_mutations.is_empty()
            && terminal_message_patch.is_none()
            && projection_meta_patch.is_none()
            && terminal_participant.is_none()
        {
            return Ok(());
        }
        let authority = self
            .event_authority
            .read()
            .clone()
            .ok_or_else(|| "agent-session SQLite authority is unavailable".to_string())?;
        let current_projection = self.canonical_session_projection(session_id)?;
        let fallback_meta = current_projection
            .as_ref()
            .map(|projection| projection.meta.clone());
        let terminal_pause_retry_required = terminal_requires_queue_pause(events);
        let supplies_queue_pause = events
            .iter()
            .any(|event| matches!(event, AgentSessionEvent::QueuePaused { .. }));
        let expected_terminal_queue_paused =
            terminal_pause_retry_required.then_some(!supplies_queue_pause);
        if let Some(expected_queue_paused) = expected_terminal_queue_paused {
            let queue_is_paused = current_projection
                .as_ref()
                .is_some_and(|projection| projection.queue_paused_at.is_some());
            if expected_queue_paused != queue_is_paused {
                return Err(
                    "terminal queue-pause authority changed before atomic commit; retry"
                        .to_string(),
                );
            }
        }
        #[cfg(test)]
        let atomic_event_commit_hook = self.atomic_event_commit_hook.read().clone();
        let session_id = session_id.to_string();
        let events = events.to_vec();
        std::thread::scope(|scope| {
            scope
                .spawn(move || {
                    tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|error| {
                            format!("failed to create atomic agent event runtime: {error}")
                        })?
                        .block_on(async move {
                            let stream_id = crate::domain::local_event::StreamId::agent_session(
                                &session_id,
                            )
                            .map_err(|_| "agent session stream identity is invalid".to_string())?;
                            let mut exact = Sha256::new();
                            hash_identity_field(
                                &mut exact,
                                b"agent_atomic_event_commit_identity_v1",
                            );
                            hash_identity_field(&mut exact, session_id.as_bytes());
                            hash_identity_field(
                                &mut exact,
                                operation_kind.label().as_bytes(),
                            );
                            let codec = authority.projection_codec.as_ref();
                            let encoded_events = codec.encode_events_for_identity(&events)?;
                            hash_identity_field(&mut exact, &encoded_events);
                            for mutation in &additional_mutations {
                                let encoded = authority
                                    .repository
                                    .canonical_mutation_identity_v1(mutation)?;
                                hash_identity_field(&mut exact, &encoded);
                            }
                            if let Some(patch) = &terminal_message_patch {
                                hash_terminal_message_patch(&mut exact, codec, patch)?;
                            }
                            if let Some(patch) = &projection_meta_patch {
                                hash_projection_meta_patch(&mut exact, patch)?;
                            }
                            let payload_hash: [u8; 32] = exact.finalize().into();
                            let identity = format!(
                                "session-atomic-event-{}",
                                hex::encode(payload_hash)
                            );
                            let commit_id =
                                crate::domain::local_event::CommitIdentity::parse(&identity)
                                    .map_err(|_| {
                                        "atomic agent event commit identity is invalid".to_string()
                                    })?;
                            let occurred_at_ms = (now_timestamp() * 1000.0).round() as i64;
                            for _ in 0..4 {
                                let head = authority
                                    .repository
                                    .load_stream(crate::domain::local_event::LoadStreamRequest {
                                        stream_id: stream_id.clone(),
                                        after: None,
                                        limit: 1,
                                    })
                                    .await
                                    .map_err(|error| {
                                        format!("agent SQLite head read failed: {error}")
                                    })?
                                    .head;
                                let mut state_mutations =
                                    prepare_canonical_event_projection_mutations(
                                        &authority,
                                        &session_id,
                                        &events,
                                        fallback_meta.clone(),
                                        terminal_message_patch.as_ref(),
                                        expected_terminal_queue_paused,
                                    )
                                    .await?;
                                if let Some(patch) = &projection_meta_patch {
                                    let codec = authority.projection_codec.as_ref();
                                    patch_event_projection_meta(
                                        codec,
                                        &mut state_mutations,
                                        patch,
                                    )?;
                                }
                                state_mutations.extend(additional_mutations.clone());
                                let participant_events =
                                    if let Some((provider, terminal)) = &terminal_participant {
                                    // This point-query must happen after the session projection
                                    // read above. Stop acceptance mutates that same projection,
                                    // so an acceptance racing after a `none` answer makes this
                                    // batch conflict and the next loop re-queries participants.
                                        let participants = provider.prepare(terminal).await?;
                                        state_mutations.extend(participants.mutations);
                                        participants.events
                                    } else {
                                        Vec::new()
                                    };
                                #[cfg(test)]
                                if let Some(hook) = &atomic_event_commit_hook {
                                    hook(operation_kind)?;
                                }
                                let batch = crate::domain::local_event::LocalAtomicBatch {
                                    commit_id: commit_id.clone(),
                                    idempotency: crate::domain::local_event::IdempotencyBinding {
                                        installation_id: authority.installation_id.clone(),
                                        operation_kind,
                                        idempotency_key: hex::encode(payload_hash),
                                        payload_hash,
                                    },
                                    expected_heads: vec![
                                        crate::domain::local_event::ExpectedStreamHead {
                                            stream_id: stream_id.clone(),
                                            expected: head,
                                        },
                                    ],
                                    events: events
                                        .iter()
                                        .chain(participant_events.iter())
                                        .cloned()
                                        .map(|event| {
                                            crate::domain::local_event::UncommittedDomainEvent {
                                                stream_id: stream_id.clone(),
                                                event: crate::domain::local_event::LocalDomainEvent::AgentSession(event),
                                                occurred_at_ms,
                                            }
                                        })
                                        .collect(),
                                    state_mutations,
                                };
                                match authority.repository.commit_batch(batch).await {
                                    Ok(_) => return Ok(()),
                                    Err(crate::domain::local_event::CommitBatchError::StreamHeadConflict { .. })
                                        if terminal_pause_retry_required =>
                                    {
                                        return Err(
                                            "terminal queue-pause authority changed during atomic commit; retry"
                                                .to_string(),
                                        );
                                    }
                                    Err(crate::domain::local_event::CommitBatchError::StreamHeadConflict { .. }) => continue,
                                    Err(crate::domain::local_event::CommitBatchError::PayloadConflict)
                                        if terminal_pause_retry_required =>
                                    {
                                        return Err(
                                            "terminal queue-pause authority changed during atomic commit; retry"
                                                .to_string(),
                                        );
                                    }
                                    Err(crate::domain::local_event::CommitBatchError::PayloadConflict)
                                        if terminal_participant.is_some() => continue,
                                    Err(crate::domain::local_event::CommitBatchError::PayloadConflict)
                                        if projection_meta_patch.is_some() => continue,
                                    Err(crate::domain::local_event::CommitBatchError::OutcomeUnknown { identity }) => {
                                        match authority.repository.resolve_commit(identity).await {
                                            Ok(crate::domain::local_event::CommitResolution::Committed(_)) => return Ok(()),
                                            Ok(crate::domain::local_event::CommitResolution::NotCommitted) => continue,
                                            Err(error) => return Err(format!("atomic agent event commit outcome could not be resolved: {error}")),
                                        }
                                    }
                                    Err(error) => {
                                        return Err(format!(
                                            "atomic agent event commit failed: {error}"
                                        ));
                                    }
                                }
                            }
                            Err("atomic agent event commit remained contended".to_string())
                        })
                })
                .join()
                .map_err(|_| "atomic agent event commit worker panicked".to_string())?
        })
    }

    fn commit_session_projection_snapshot(
        &self,
        projection: CanonicalAgentSessionProjection,
    ) -> Result<(), String> {
        self.commit_session_projection_snapshot_with_kind(
            projection,
            crate::domain::local_event::CommitOperationKind::Projection,
        )
    }

    fn commit_user_session_projection_snapshot(
        &self,
        projection: CanonicalAgentSessionProjection,
    ) -> Result<(), String> {
        self.commit_session_projection_snapshot_with_kind(
            projection,
            crate::domain::local_event::CommitOperationKind::UserMutation,
        )
    }

    fn commit_session_projection_snapshot_with_kind(
        &self,
        projection: CanonicalAgentSessionProjection,
        operation_kind: crate::domain::local_event::CommitOperationKind,
    ) -> Result<(), String> {
        self.commit_session_projection_snapshot_with_kind_and_mutations(
            projection,
            operation_kind,
            Vec::new(),
        )
    }

    fn commit_session_projection_snapshot_with_kind_and_mutations(
        &self,
        mut projection: CanonicalAgentSessionProjection,
        operation_kind: crate::domain::local_event::CommitOperationKind,
        additional_mutations: Vec<crate::domain::local_event::LocalStateMutation>,
    ) -> Result<(), String> {
        let Some(authority) = self.event_authority.read().clone() else {
            #[cfg(test)]
            return Ok(());
            #[cfg(not(test))]
            return Err("agent-session SQLite event authority is not configured".to_string());
        };
        let codec = authority.projection_codec.clone();
        let session_id = projection.meta.id.clone();
        let content_blobs = externalize_canonical_message_content(&mut projection.messages)?;
        let encoded_messages = projection
            .messages
            .iter()
            .map(|message| {
                codec
                    .encode_message(message)
                    .map(|encoded| (message.id.clone(), encoded))
            })
            .collect::<Result<Vec<_>, String>>()?;
        let encoded = codec.encode(&projection)?;
        let encoded_identity_v1 = codec.encode_session_identity_v1(&encoded)?;
        let payload_hash: [u8; 32] = Sha256::digest(&encoded_identity_v1).into();
        std::thread::scope(|scope| {
            scope
                .spawn(move || {
                    tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|error| {
                            format!("failed to create agent projection commit runtime: {error}")
                        })?
                        .block_on(async move {
                            let result = authority
                                .repository
                                .query(crate::domain::local_event::LocalEventQuery::SessionProjectionByIdentity {
                                    session_id: session_id.clone(),
                                })
                                .await
                                .map_err(|error| {
                                    format!("agent SQLite projection read failed: {error}")
                                })?;
                            let crate::domain::local_event::LocalEventQueryResult::SessionProjectionByIdentity(
                                stored,
                            ) = result
                            else {
                                return Err("agent SQLite projection query returned the wrong shape".to_string());
                            };
                            if encoded_messages.is_empty()
                                && additional_mutations.is_empty()
                                && stored.as_ref().is_some_and(|stored| {
                                    stored.projection == encoded
                                })
                            {
                                return Ok(());
                            }
                            let (expected, revision) = match stored {
                                Some(stored) => (
                                    crate::domain::local_event::RevisionGuard::Expected(stored.revision),
                                    stored.revision.next().ok_or_else(|| {
                                        "agent projection revision exhausted".to_string()
                                    })?,
                                ),
                                None => (
                                    crate::domain::local_event::RevisionGuard::Absent,
                                    crate::domain::local_event::Revision::new(0)
                                        .expect("zero revision"),
                                ),
                            };
                            let mut binding = Sha256::new();
                            binding.update(payload_hash);
                            binding.update(revision.value().to_be_bytes());
                            for (message_id, message) in &encoded_messages {
                                binding.update((message_id.len() as u64).to_be_bytes());
                                binding.update(message_id.as_bytes());
                                let message_identity_v1 =
                                    codec.encode_message_identity_v1(message)?;
                                binding.update(
                                    (message_identity_v1.len() as u64).to_be_bytes(),
                                );
                                binding.update(&message_identity_v1);
                            }
                            for mutation in &additional_mutations {
                                let encoded = authority
                                    .repository
                                    .canonical_mutation_identity_v1(mutation)?;
                                hash_identity_field(&mut binding, &encoded);
                            }
                            let binding_hash: [u8; 32] = binding.finalize().into();
                            let identity =
                                format!("session-projection-{}", hex::encode(binding_hash));
                            let mut state_mutations = vec![
                                crate::domain::local_event::LocalStateMutation::SessionProjection(
                                    crate::domain::local_event::SessionProjectionMutation {
                                        session_id: session_id.clone(),
                                        projection: encoded,
                                        expected,
                                        revision,
                                    },
                                ),
                            ];
                            state_mutations.extend(
                                prepare_canonical_content_blob_mutations(
                                    &authority.repository,
                                    &session_id,
                                    content_blobs,
                                )
                                .await?,
                            );
                            for (message_id, encoded_message) in encoded_messages {
                                let result = authority
                                    .repository
                                    .query(crate::domain::local_event::LocalEventQuery::MessageProjectionByIdentity {
                                        session_id: session_id.clone(),
                                        message_id: message_id.clone(),
                                    })
                                    .await
                                    .map_err(|error| {
                                        format!("agent SQLite message projection read failed: {error}")
                                    })?;
                                let crate::domain::local_event::LocalEventQueryResult::MessageProjectionByIdentity(
                                    stored,
                                ) = result
                                else {
                                    return Err("agent SQLite message projection query returned the wrong shape".to_string());
                                };
                                if stored.as_ref().is_some_and(|stored| {
                                    stored.projection == encoded_message
                                }) {
                                    continue;
                                }
                                let (expected, revision) = match stored {
                                    Some(stored) => (
                                        crate::domain::local_event::RevisionGuard::Expected(stored.revision),
                                        stored.revision.next().ok_or_else(|| {
                                            "agent message projection revision exhausted".to_string()
                                        })?,
                                    ),
                                    None => (
                                        crate::domain::local_event::RevisionGuard::Absent,
                                        crate::domain::local_event::Revision::new(0)
                                            .expect("zero revision"),
                                    ),
                                };
                                state_mutations.push(
                                    crate::domain::local_event::LocalStateMutation::MessageProjection(
                                        crate::domain::local_event::MessageProjectionMutation {
                                            session_id: session_id.clone(),
                                            message_id,
                                            projection: encoded_message,
                                            expected,
                                            revision,
                                        },
                                    ),
                                );
                            }
                            state_mutations.extend(additional_mutations);
                            let batch = crate::domain::local_event::LocalAtomicBatch {
                                commit_id: crate::domain::local_event::CommitIdentity::parse(
                                    &identity,
                                )
                                .map_err(|_| {
                                    "agent projection commit identity is invalid".to_string()
                                })?,
                                idempotency: crate::domain::local_event::IdempotencyBinding {
                                    installation_id: authority.installation_id.clone(),
                                    operation_kind,
                                    idempotency_key: hex::encode(binding_hash),
                                    payload_hash: binding_hash,
                                },
                                expected_heads: Vec::new(),
                                events: Vec::new(),
                                state_mutations,
                            };
                            authority
                                .repository
                                .commit_batch(batch)
                                .await
                                .map(|_| ())
                                .map_err(|error| {
                                    format!("agent SQLite projection commit failed: {error}")
                                })
                        })
                })
                .join()
                .map_err(|_| "agent SQLite projection commit worker panicked".to_string())?
        })
    }

    /// Prepare the bounded canonical session/message projection mutations for
    /// an arbitrary agent-event slice without committing them. Operation
    /// admission paths use this to include their domain events and read-model
    /// changes in the same compare-and-swap batch.
    pub(crate) fn prepare_event_projection_mutations(
        &self,
        session_id: &str,
        events: &[AgentSessionEvent],
    ) -> Result<Vec<crate::domain::local_event::LocalStateMutation>, String> {
        let authority = self
            .event_authority
            .read()
            .clone()
            .ok_or_else(|| "agent-session SQLite authority is unavailable".to_string())?;
        let current_projection = self.canonical_session_projection(session_id)?;
        let fallback_meta = current_projection
            .as_ref()
            .map(|projection| projection.meta.clone());
        let session_id = session_id.to_string();
        let events = complete_terminal_projection_events(
            current_projection
                .as_ref()
                .map(|projection| projection.reducer_events.as_slice())
                .unwrap_or_default(),
            events,
        );
        std::thread::scope(|scope| {
            scope
                .spawn(move || {
                    tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|error| {
                            format!("failed to create event projection runtime: {error}")
                        })?
                        .block_on(prepare_canonical_event_projection_mutations(
                            &authority,
                            &session_id,
                            &events,
                            fallback_meta,
                            None,
                            None,
                        ))
                })
                .join()
                .map_err(|_| "event projection worker panicked".to_string())?
        })
    }

    /// Prepare an event projection only when the mutation was derived from
    /// the caller's exact public session revision. The returned projection
    /// retains its own SQLite revision guard, so a change after preparation
    /// still conflicts at commit.
    pub(crate) fn prepare_event_projection_mutations_if_current_revision(
        &self,
        session_id: &str,
        expected_state_revision: u64,
        events: &[AgentSessionEvent],
    ) -> Result<Option<Vec<crate::domain::local_event::LocalStateMutation>>, String> {
        let mutations = self.prepare_event_projection_mutations(session_id, events)?;
        let authority = self
            .event_authority
            .read()
            .clone()
            .ok_or_else(|| "agent-session SQLite authority is unavailable".to_string())?;
        let projection = mutations
            .iter()
            .find_map(|mutation| match mutation {
                crate::domain::local_event::LocalStateMutation::SessionProjection(projection) => {
                    Some(projection)
                }
                _ => None,
            })
            .ok_or_else(|| "agent event batch omitted its session projection".to_string())?;
        let projected = authority.projection_codec.decode(&projection.projection)?;
        let expected_projected_revision =
            next_sqlite_counter(expected_state_revision, "guarded session state revision")?;
        if projected.meta.state_revision != expected_projected_revision {
            return Ok(None);
        }
        Ok(Some(mutations))
    }

    /// Prepare the owner-side closure for a backend recovery whose provider
    /// effect survived a process crash but whose completion batch did not.
    ///
    /// The durable provider identity/generation is the only success evidence.
    /// This method does not commit: RecoveryActionUsecase appends the returned
    /// events, projection and publication obligation beside its action/source
    /// obligation CAS in one LocalAtomicBatch.
    pub(crate) fn prepare_backend_recovery_readback_completion(
        &self,
        session_id: &str,
        recovery_id: &str,
    ) -> Result<Option<BackendRecoveryReadbackParticipants>, String> {
        let source_obligation_id = backend_recovery_obligation_id(session_id, recovery_id);
        let source = self
            .canonical_obligation(&source_obligation_id)?
            .ok_or_else(|| "backend recovery readback has no durable reservation".to_string())?;
        fn original(
            record: &crate::domain::local_event::ObligationRecord,
        ) -> &crate::domain::local_event::ObligationRecord {
            match record {
                crate::domain::local_event::ObligationRecord::RecoveryTransition {
                    original: nested,
                    ..
                }
                | crate::domain::local_event::ObligationRecord::Observed {
                    original: nested, ..
                } => original(nested),
                record => record,
            }
        }
        let (old_provider_session_generation, reserved_at_bits) = match original(&source.record) {
            crate::domain::local_event::ObligationRecord::BackendSessionRecovery {
                session_id: stored_session_id,
                recovery_id: stored_recovery_id,
                detail:
                    crate::domain::local_event::BackendSessionRecoveryObligationRecord::EffectReserved {
                        old_provider_session_generation,
                        reserved_at_bits,
                        ..
                    },
                state: crate::domain::local_event::ObligationStateRecord::EffectReserved,
            } if stored_session_id == session_id && stored_recovery_id == recovery_id => {
                (*old_provider_session_generation, *reserved_at_bits)
            }
            _ => {
                return Err(
                    "backend recovery readback reservation identity is inconsistent".to_string(),
                )
            }
        };
        let provider_session_generation = next_sqlite_counter(
            old_provider_session_generation,
            "provider session generation",
        )?;
        let current_projection = self
            .canonical_session_projection(session_id)?
            .ok_or_else(|| "backend recovery owner projection is unavailable".to_string())?;
        let Some(backend_session_id) = current_projection
            .meta
            .agent_session_id
            .clone()
            .filter(|value| !value.is_empty())
        else {
            return Ok(None);
        };
        if current_projection.meta.provider_session_generation != provider_session_generation
            || current_projection.meta.context_reinjection_generation
                != Some(provider_session_generation)
            || current_projection
                .meta
                .recovery_publication_snapshot
                .as_ref()
                .is_none_or(|snapshot| {
                    snapshot.recovery_id != recovery_id || snapshot.summary.id != session_id
                })
        {
            return Ok(None);
        }
        let reserved_at = f64::from_bits(reserved_at_bits);
        let at = current_projection.meta.updated_at;
        if !reserved_at.is_finite() || !at.is_finite() || at < reserved_at {
            return Err("backend recovery durable completion timestamp is invalid".to_string());
        }
        let message_digest = Sha256::digest(
            format!(
                "backend-recovery-readback-publication/v1\0{session_id}\0{recovery_id}\0{provider_session_generation}\0{backend_session_id}"
            )
            .as_bytes(),
        );
        let pending_recovery_message = PendingRecoveryMessage::Notice {
            recovery_id: recovery_id.to_string(),
            message_id: format!("backend-recovery-readback-{}", hex::encode(message_digest)),
        };
        let source_completion = backend_recovery_obligation_mutation(
            source_obligation_id.clone(),
            crate::domain::local_event::ObligationRecord::BackendSessionRecovery {
                session_id: session_id.to_string(),
                recovery_id: recovery_id.to_string(),
                detail:
                    crate::domain::local_event::BackendSessionRecoveryObligationRecord::Completed {
                        old_provider_session_generation,
                        provider_session_generation,
                        backend_session_id: backend_session_id.clone(),
                        completed_at_bits: at.to_bits(),
                    },
                state: crate::domain::local_event::ObligationStateRecord::Completed,
            },
            None,
            Some(&source),
        )?;
        let (_, publication_message_id) =
            pending_recovery_message_identity(&pending_recovery_message);
        let publication_obligation_id =
            recovery_publication_obligation_id(session_id, recovery_id, publication_message_id);
        if self
            .canonical_obligation(&publication_obligation_id)?
            .is_some()
        {
            return Err("backend recovery publication identity was already reserved".to_string());
        }
        let publication = recovery_publication_obligation_mutation(
            publication_obligation_id.clone(),
            crate::domain::local_event::ObligationRecord::RecoveryPublication {
                session_id: session_id.to_string(),
                recovery_id: recovery_id.to_string(),
                message_id: publication_message_id.to_string(),
                source_obligation_id,
                detail: crate::domain::local_event::RecoveryPublicationObligationRecord::Pending {
                    pending_message: recovery_publication_message_record(&pending_recovery_message),
                },
                state: crate::domain::local_event::ObligationStateRecord::Pending,
            },
            Some(crate::domain::local_event::PendingIndexEntry {
                ordered_key: format!(
                    "{:020}:{publication_obligation_id}",
                    (at * 1000.0).round() as i64
                ),
                owner: session_id.to_string(),
                partition: crate::domain::local_event::PendingPartition::Owner,
                shutdown_plan: None,
            }),
            None,
        )?;
        let events = vec![
            AgentSessionEvent::SessionConfigurationReactivated {
                recovery_id: recovery_id.to_string(),
                provider_session_generation,
                consumed_observation_id: None,
                at,
            },
            AgentSessionEvent::SessionGoalReactivated {
                recovery_id: recovery_id.to_string(),
                outcome: GoalReactivationOutcome::NoCurrentGoal,
                provider_session_generation,
                restoring_turn_id: None,
                consumed_observation_id: None,
                at,
            },
            AgentSessionEvent::BackendSessionRecoveryCompleted {
                recovery_id: recovery_id.to_string(),
                provider_session_generation,
                at,
            },
        ];
        let authority = self
            .event_authority
            .read()
            .clone()
            .ok_or_else(|| "agent-session SQLite authority is unavailable".to_string())?;
        let fallback_meta = Some(current_projection.meta);
        let session_id = session_id.to_string();
        std::thread::scope(|scope| {
            scope
                .spawn(move || {
                    tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|error| {
                            format!("failed to create backend readback runtime: {error}")
                        })?
                        .block_on(async move {
                            let codec = authority.projection_codec.as_ref();
                            let stream_id =
                                crate::domain::local_event::StreamId::agent_session(&session_id)
                                    .map_err(|_| {
                                        "backend recovery stream identity is invalid".to_string()
                                    })?;
                            let head = authority
                                .repository
                                .load_stream(crate::domain::local_event::LoadStreamRequest {
                                    stream_id: stream_id.clone(),
                                    after: None,
                                    limit: 1,
                                })
                                .await
                                .map_err(|error| {
                                    format!("backend recovery stream head read failed: {error}")
                                })?
                                .head;
                            let mut mutations = prepare_canonical_event_projection_mutations(
                                &authority,
                                &session_id,
                                &events,
                                fallback_meta,
                                None,
                                None,
                            )
                            .await?;
                            patch_event_projection_meta(
                                codec,
                                &mut mutations,
                                &EventProjectionMetaPatch::ReadbackCompleted {
                                    old_provider_session_generation,
                                    provider_session_generation,
                                    backend_session_id,
                                    pending_recovery_message,
                                    at,
                                },
                            )?;
                            mutations.push(publication);
                            let occurred_at_ms = (at * 1000.0).round() as i64;
                            let uncommitted_events = events
                                .into_iter()
                                .map(|event| crate::domain::local_event::UncommittedDomainEvent {
                                    stream_id: stream_id.clone(),
                                    event:
                                        crate::domain::local_event::LocalDomainEvent::AgentSession(
                                            event,
                                        ),
                                    occurred_at_ms,
                                })
                                .collect::<Vec<_>>();
                            let encoded_events = authority
                                .repository
                                .canonical_event_batch_identity_v1(&uncommitted_events)?;
                            let mut participant = Sha256::new();
                            hash_identity_field(
                                &mut participant,
                                b"backend_recovery_readback_participants_v1",
                            );
                            hash_identity_field(&mut participant, stream_id.as_str().as_bytes());
                            participant.update(head.value().to_be_bytes());
                            hash_identity_field(&mut participant, &encoded_events);
                            let mut mutation_identities = Vec::with_capacity(mutations.len());
                            for mutation in &mutations {
                                let encoded = authority
                                    .repository
                                    .canonical_mutation_identity_v1(mutation)?;
                                mutation_identities.push(encoded);
                            }
                            mutation_identities.sort();
                            for encoded in mutation_identities {
                                hash_identity_field(&mut participant, &encoded);
                            }
                            let participant_digest: [u8; 32] = participant.finalize().into();
                            // RecoveryActionUsecase validates and merges this
                            // exact source closure into its single wrapped
                            // source mutation; it is deliberately excluded
                            // from the owner-batch digest after normalization.
                            mutations.push(source_completion);
                            Ok(Some(BackendRecoveryReadbackParticipants {
                                expected_heads: vec![
                                    crate::domain::local_event::ExpectedStreamHead {
                                        stream_id: stream_id.clone(),
                                        expected: head,
                                    },
                                ],
                                events: uncommitted_events,
                                canonical_events: encoded_events,
                                mutations,
                                participant_digest,
                            }))
                        })
                })
                .join()
                .map_err(|_| "backend recovery readback worker panicked".to_string())?
        })
    }

    /// Prepare the canonical lifecycle projection as a participant of the
    /// lifecycle acceptance transaction. Runtime teardown happens only after
    /// this mutation commits; it must never perform a second canonical state
    /// write for close, archive, or backend selection.
    pub(crate) fn prepare_lifecycle_acceptance_mutations(
        &self,
        session_id: &str,
        events: &[AgentSessionEvent],
        lifecycle_state: SessionState,
        backend_selection: Option<(&str, &str)>,
    ) -> Result<Vec<crate::domain::local_event::LocalStateMutation>, String> {
        let mut mutations = self.prepare_event_projection_mutations(session_id, events)?;
        let authority = self
            .event_authority
            .read()
            .clone()
            .ok_or_else(|| "agent-session SQLite authority is unavailable".to_string())?;
        let codec = authority.projection_codec.as_ref();
        let projection = mutations
            .iter_mut()
            .find_map(|mutation| match mutation {
                crate::domain::local_event::LocalStateMutation::SessionProjection(projection) => {
                    Some(projection)
                }
                _ => None,
            })
            .ok_or_else(|| "lifecycle batch omitted its session projection".to_string())?;
        let mut decoded = codec.decode(&projection.projection)?;
        decoded.meta.state = lifecycle_state;
        if decoded.meta.state != SessionState::Error {
            decoded.meta.error_reason = None;
        }
        if let Some((backend_id, selected_model)) = backend_selection {
            decoded.meta.backend_id = backend_id.to_string();
            decoded.meta.selected_model = Some(selected_model.to_string());
        }
        decoded.meta.updated_at = now_timestamp();
        projection.projection = codec.encode(&decoded)?;
        Ok(mutations)
    }

    pub(crate) fn prepare_send_acceptance_mutations(
        &self,
        input: SendAcceptanceProjectionInput<'_>,
    ) -> Result<Vec<crate::domain::local_event::LocalStateMutation>, String> {
        let SendAcceptanceProjectionInput {
            session_id,
            initial_session,
            session_projection_guard,
            human_message_id,
            prompt,
            disposition,
            reserved_turn_id,
            input_ref,
            events,
        } = input;
        let authority = self
            .event_authority
            .read()
            .clone()
            .ok_or_else(|| "agent-session SQLite authority is unavailable".to_string())?;
        let codec = authority.projection_codec.clone();
        let session_id = session_id.to_string();
        let human_message_id = human_message_id.to_string();
        let prompt = prompt.clone();
        let disposition = disposition.clone();
        let reserved_turn_id = reserved_turn_id.map(str::to_string);
        let input_ref = input_ref.to_string();
        let initial_meta = initial_session.map(SessionMeta::from_session);
        let events = events.to_vec();
        std::thread::scope(|scope| {
            scope
                .spawn(move || {
                    tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|error| format!("failed to create send projection runtime: {error}"))?
                        .block_on(async move {
                            let stored = match authority
                                .repository
                                .query(crate::domain::local_event::LocalEventQuery::SessionProjectionByIdentity {
                                    session_id: session_id.clone(),
                                })
                                .await
                                .map_err(|error| format!("send projection read failed: {error}"))?
                            {
                                crate::domain::local_event::LocalEventQueryResult::SessionProjectionByIdentity(stored) => stored,
                                _ => return Err("send projection query returned the wrong shape".to_string()),
                            };
                            let (
                                mut meta,
                                title,
                                reducer_events,
                                mut pending_send_queue,
                                expected,
                                revision,
                            ) = match stored {
                                Some(stored) => {
                                    if initial_meta.is_some() {
                                        return Err("new send target already exists".to_string());
                                    }
                                    if session_projection_guard
                                        != crate::domain::local_event::RevisionGuard::Expected(
                                            stored.revision,
                                        )
                                    {
                                        return Err(
                                            "send allocation projection changed before acceptance"
                                                .to_string(),
                                        );
                                    }
                                    let decoded = codec.decode(&stored.projection)?;
                                    (
                                        decoded.meta,
                                        decoded.title,
                                        decoded.reducer_events,
                                        decoded.pending_send_queue,
                                        session_projection_guard,
                                        stored.revision.next().ok_or_else(|| "send projection revision exhausted".to_string())?,
                                    )
                                }
                                None => {
                                    if session_projection_guard
                                        != crate::domain::local_event::RevisionGuard::Absent
                                    {
                                        return Err(
                                            "send allocation projection changed before acceptance"
                                                .to_string(),
                                        );
                                    }
                                    (
                                        initial_meta.ok_or_else(|| "send target projection is missing".to_string())?,
                                        None,
                                        Vec::new(),
                                        Vec::new(),
                                        session_projection_guard,
                                        crate::domain::local_event::Revision::new(0).expect("zero revision"),
                                    )
                                }
                            };
                            let reducer_events = bounded_reducer_events(reducer_events, &events);
                            let projected = TurnEventLog::from_events(reducer_events.clone()).project();
                            let started_turn_id = events.iter().find_map(|event| match event {
                                AgentSessionEvent::TurnStarted { turn_id, .. } => Some(*turn_id),
                                _ => None,
                            });
                            if let Some(turn_id) = started_turn_id {
                                if !pending_send_queue.is_empty() {
                                    return Err(
                                        "started send cannot bypass the canonical queue front"
                                            .to_string(),
                                    );
                                }
                                if meta
                                    .last_turn_id
                                    .is_some_and(|last_turn_id| turn_id <= last_turn_id)
                                {
                                    return Err(
                                        "started send turn identity does not advance".to_string()
                                    );
                                }
                                meta.state = projected.status.session_state.clone();
                                meta.error_reason = error_reason_for_state(
                                    &meta.state,
                                    &projected.error_reason,
                                );
                                meta.last_turn_id = reducer_events.iter().rev().find_map(|event| {
                                    match event {
                                        AgentSessionEvent::TurnStarted { turn_id, .. } => Some(*turn_id),
                                        _ => None,
                                    }
                                });
                            }
                            let started_turn = started_turn_id.is_some();
                            meta.state_revision = next_sqlite_counter(
                                meta.state_revision,
                                "session state revision",
                            )?;

                            let mut messages = projected
                                .messages
                                .into_iter()
                                .filter(|message| {
                                    message.id == human_message_id
                                        || (started_turn && message.role == MessageRole::Agent)
                                })
                                .collect::<Vec<_>>();
                            if !messages.iter().any(|message| message.id == human_message_id) {
                                messages.push(ChatMessage {
                                    id: human_message_id.clone(),
                                    role: MessageRole::Human,
                                    content: prompt.content.clone(),
                                    thinking: None,
                                    activities: None,
                                    parts: (!prompt.parts.is_empty()).then(|| prompt.parts.clone()),
                                    streaming_final_seq: 0,
                                    timestamp: now_timestamp(),
                                    mentions: (!prompt.mentions.is_empty()).then(|| {
                                        prompt
                                            .mentions
                                            .clone()
                                            .into_iter()
                                            .map(super::MessageMention::from_domain)
                                            .collect()
                                    }),
                                });
                            }

                            let content_blobs =
                                externalize_canonical_message_content(&mut messages)?;

                            let mut mutations = prepare_canonical_content_blob_mutations(
                                &authority.repository,
                                &session_id,
                                content_blobs,
                            )
                            .await?;
                            let mut inserted = Vec::new();
                            for message in messages {
                                let encoded = codec.encode_message(&message)?;
                                let stored = match authority
                                    .repository
                                    .query(crate::domain::local_event::LocalEventQuery::MessageProjectionByIdentity {
                                        session_id: session_id.clone(),
                                        message_id: message.id.clone(),
                                    })
                                    .await
                                    .map_err(|error| format!("send message projection read failed: {error}"))?
                                {
                                    crate::domain::local_event::LocalEventQueryResult::MessageProjectionByIdentity(stored) => stored,
                                    _ => return Err("send message query returned the wrong shape".to_string()),
                                };
                                let (message_expected, message_revision) = match stored {
                                    Some(stored) if stored.projection == encoded => continue,
                                    Some(stored) => (
                                        crate::domain::local_event::RevisionGuard::Expected(stored.revision),
                                        stored.revision.next().ok_or_else(|| "send message revision exhausted".to_string())?,
                                    ),
                                    None => {
                                        inserted.push(message.clone());
                                        (
                                            crate::domain::local_event::RevisionGuard::Absent,
                                            crate::domain::local_event::Revision::new(0).expect("zero revision"),
                                        )
                                    }
                                };
                                mutations.push(crate::domain::local_event::LocalStateMutation::MessageProjection(
                                    crate::domain::local_event::MessageProjectionMutation {
                                        session_id: session_id.clone(),
                                        message_id: message.id,
                                        projection: encoded,
                                        expected: message_expected,
                                        revision: message_revision,
                                    },
                                ));
                            }
                            meta.message_count = add_sqlite_count(
                                meta.message_count,
                                inserted.len(),
                                "session message count",
                            )?;
                            if meta.first_message_preview.is_empty() {
                                meta.first_message_preview = super::first_message_preview(&inserted);
                            }
                            meta.updated_at = now_timestamp();
                            if let crate::domain::agent_session::events::SendDisposition::Queued {
                                queue_item_id,
                            } = &disposition
                            {
                                let reserved_turn_id = reserved_turn_id.clone().ok_or_else(|| {
                                    "queued send is missing its reserved turn identity".to_string()
                                })?;
                                if !pending_send_queue
                                    .iter()
                                    .any(|entry| entry.queue_item_id == *queue_item_id)
                                {
                                    pending_send_queue.push(CanonicalQueuedSend {
                                        queue_item_id: queue_item_id.clone(),
                                        human_message_id: human_message_id.clone(),
                                        reserved_turn_id,
                                        input_ref: input_ref.clone(),
                                    });
                                }
                            }
                            let projection = codec.encode(&CanonicalAgentSessionProjection {
                                meta,
                                title,
                                messages: Vec::new(),
                                reducer_events,
                                queue_paused_at: projected.queue_paused_at,
                                latest_token_usage: None,
                                pending_send_queue,
                            })?;
                            mutations.insert(0, crate::domain::local_event::LocalStateMutation::SessionProjection(
                                crate::domain::local_event::SessionProjectionMutation {
                                    session_id,
                                    projection,
                                    expected,
                                    revision,
                                },
                            ));
                            Ok(mutations)
                        })
                })
                .join()
                .map_err(|_| "send projection worker panicked".to_string())?
        })
    }

    fn commit_meta_projection_snapshot(&self, meta: SessionMeta) -> Result<(), String> {
        self.commit_meta_projection_snapshot_with_kind(
            meta,
            crate::domain::local_event::CommitOperationKind::Projection,
        )
    }

    fn commit_user_meta_projection_snapshot(&self, meta: SessionMeta) -> Result<(), String> {
        self.commit_meta_projection_snapshot_with_kind(
            meta,
            crate::domain::local_event::CommitOperationKind::UserMutation,
        )
    }

    fn commit_meta_projection_snapshot_with_kind(
        &self,
        meta: SessionMeta,
        operation_kind: crate::domain::local_event::CommitOperationKind,
    ) -> Result<(), String> {
        let current = self.canonical_session_projection(&meta.id)?;
        let queue_paused_at = current
            .as_ref()
            .and_then(|projection| projection.queue_paused_at);
        let title = current
            .as_ref()
            .and_then(|projection| projection.title.clone());
        let reducer_events = current
            .as_ref()
            .map(|projection| projection.reducer_events.clone())
            .unwrap_or_default();
        let pending_send_queue = current
            .map(|projection| projection.pending_send_queue)
            .unwrap_or_default();
        self.commit_session_projection_snapshot_with_kind(
            CanonicalAgentSessionProjection {
                meta,
                title,
                messages: Vec::new(),
                reducer_events,
                queue_paused_at,
                latest_token_usage: None,
                pending_send_queue,
            },
            operation_kind,
        )
    }

    fn remove_canonical_session_projection(&self, session_id: &str) -> Result<(), String> {
        let Some(authority) = self.event_authority.read().clone() else {
            #[cfg(test)]
            return Ok(());
            #[cfg(not(test))]
            return Err("agent-session SQLite event authority is not configured".to_string());
        };
        let session_id = session_id.to_string();
        std::thread::scope(|scope| {
            scope
                .spawn(move || {
                    tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|error| {
                            format!("failed to create agent rollback runtime: {error}")
                        })?
                        .block_on(async move {
                            let result = authority
                                .repository
                                .query(crate::domain::local_event::LocalEventQuery::SessionProjectionByIdentity {
                                    session_id: session_id.clone(),
                                })
                                .await
                                .map_err(|error| {
                                    format!("agent rollback projection read failed: {error}")
                                })?;
                            let crate::domain::local_event::LocalEventQueryResult::SessionProjectionByIdentity(
                                stored,
                            ) = result
                            else {
                                return Err("agent rollback projection query returned the wrong shape".to_string());
                            };
                            let Some(stored) = stored else {
                                return Ok(());
                            };
                            let mut binding = Sha256::new();
                            binding.update(b"agent-session-projection-rollback/v1");
                            binding.update((session_id.len() as u64).to_be_bytes());
                            binding.update(session_id.as_bytes());
                            binding.update(stored.revision.value().to_be_bytes());
                            let binding_hash: [u8; 32] = binding.finalize().into();
                            let identity = format!(
                                "session-projection-rollback-{}",
                                hex::encode(binding_hash)
                            );
                            let commit_identity =
                                crate::domain::local_event::CommitIdentity::parse(&identity)
                                    .map_err(|_| {
                                        "agent rollback commit identity is invalid".to_string()
                                    })?;
                            let batch = crate::domain::local_event::LocalAtomicBatch {
                                commit_id: commit_identity.clone(),
                                idempotency: crate::domain::local_event::IdempotencyBinding {
                                    installation_id: authority.installation_id.clone(),
                                    operation_kind: crate::domain::local_event::CommitOperationKind::Projection,
                                    idempotency_key: hex::encode(binding_hash),
                                    payload_hash: binding_hash,
                                },
                                expected_heads: Vec::new(),
                                events: Vec::new(),
                                state_mutations: vec![
                                    crate::domain::local_event::LocalStateMutation::SessionProjectionRemoval(
                                        crate::domain::local_event::SessionProjectionRemovalMutation {
                                            session_id,
                                            expected: crate::domain::local_event::RevisionGuard::Expected(
                                                stored.revision,
                                            ),
                                        },
                                    ),
                                ],
                            };
                            match authority.repository.commit_batch(batch).await {
                                Ok(_) => Ok(()),
                                Err(crate::domain::local_event::CommitBatchError::OutcomeUnknown {
                                    ..
                                }) => match authority
                                    .repository
                                    .resolve_commit(commit_identity)
                                    .await
                                    .map_err(|error| {
                                        format!("agent rollback readback failed: {error}")
                                    })?
                                {
                                    crate::domain::local_event::CommitResolution::Committed(_) => {
                                        Ok(())
                                    }
                                    crate::domain::local_event::CommitResolution::NotCommitted => {
                                        Err("agent rollback was not committed".to_string())
                                    }
                                },
                                Err(error) => {
                                    Err(format!("agent rollback commit failed: {error}"))
                                }
                            }
                        })
                })
                .join()
                .map_err(|_| "agent rollback worker panicked".to_string())?
        })
    }

    #[cfg(test)]
    pub(crate) fn set_save_hook_for_test(&self, hook: SessionSaveHook) {
        *self.save_hook.write() = Some(hook);
    }

    #[cfg(test)]
    pub(crate) fn set_append_message_hook_for_test(&self, hook: SessionAppendMessageHook) {
        *self.append_message_hook.write() = Some(hook);
    }

    #[cfg(test)]
    pub(crate) fn set_persist_parts_hook_for_test(&self, hook: SessionPersistPartsHook) {
        *self.persist_parts_hook.write() = Some(hook);
    }

    #[cfg(test)]
    pub(crate) fn set_append_event_hook_for_test(&self, hook: SessionAppendEventHook) {
        *self.append_event_hook.write() = Some(hook);
    }

    #[cfg(test)]
    pub(crate) fn set_state_hook_for_test(&self, hook: SessionSetStateHook) {
        *self.set_state_hook.write() = Some(hook);
    }

    #[cfg(test)]
    pub(crate) fn set_projection_hook_for_test(&self, hook: SessionProjectionHook) {
        *self.projection_hook.write() = Some(hook);
    }

    #[cfg(test)]
    pub(crate) fn set_appended_event_hook_for_test(&self, hook: SessionAppendedEventHook) {
        *self.appended_event_hook.write() = Some(hook);
    }

    #[cfg(test)]
    pub(crate) fn set_event_projection_hook_for_test(&self, hook: SessionEventProjectionHook) {
        *self.event_projection_hook.write() = Some(hook);
    }

    #[cfg(test)]
    pub(crate) fn set_atomic_event_commit_hook_for_test(&self, hook: SessionAtomicEventCommitHook) {
        *self.atomic_event_commit_hook.write() = Some(hook);
    }

    #[cfg(test)]
    pub(crate) fn set_backend_established_hook_for_test(
        &self,
        hook: SessionBackendEstablishedHook,
    ) {
        *self.backend_established_hook.write() = Some(hook);
    }

    #[cfg(test)]
    pub(crate) fn set_projected_read_model_hook_for_test(
        &self,
        hook: SessionProjectedReadModelHook,
    ) {
        *self.projected_read_model_hook.write() = Some(hook);
    }

    pub fn list_sessions(
        &self,
        app_data_dir: &Path,
        worktree_path: &str,
    ) -> Result<Vec<SessionSummary>, String> {
        self.list_sessions_filtered(app_data_dir, worktree_path, |s| {
            s.state != SessionState::Closed && s.state != SessionState::Archived
        })
    }

    pub fn list_closed_sessions(
        &self,
        app_data_dir: &Path,
        worktree_path: &str,
    ) -> Result<Vec<SessionSummary>, String> {
        self.list_sessions_filtered(app_data_dir, worktree_path, |s| {
            s.state == SessionState::Closed
        })
    }

    /// Fixed application-shutdown inventory. Workflow-owned child sessions
    /// are represented by their workflow owner target and therefore are not
    /// emitted as a second shutdown target here.
    pub(crate) fn application_shutdown_target_session_ids(
        &self,
        app_data_dir: &Path,
    ) -> Result<Vec<String>, String> {
        let mut ids = self
            .list_metas_for_active_read_authority(app_data_dir)?
            .into_iter()
            .filter(|meta| {
                !meta.workflow_node_session
                    && meta.state != SessionState::Closed
                    && meta.state != SessionState::Archived
            })
            .map(|meta| meta.id)
            .collect::<Vec<_>>();
        ids.sort();
        ids.dedup();
        Ok(ids)
    }

    pub fn list_published_sessions(
        &self,
        app_data_dir: &Path,
        worktree_path: &str,
    ) -> Result<Vec<SessionSummary>, String> {
        let summaries = self.list_sessions(app_data_dir, worktree_path)?;
        self.overlay_recovery_publication_snapshots(
            app_data_dir,
            summaries,
            RecoveryPublicationList::SessionList,
        )
    }

    pub fn list_published_closed_sessions(
        &self,
        app_data_dir: &Path,
        worktree_path: &str,
    ) -> Result<Vec<SessionSummary>, String> {
        let summaries = self.list_closed_sessions(app_data_dir, worktree_path)?;
        self.overlay_recovery_publication_snapshots(
            app_data_dir,
            summaries,
            RecoveryPublicationList::ClosedHistory,
        )
    }

    fn overlay_recovery_publication_snapshots(
        &self,
        _app_data_dir: &Path,
        summaries: Vec<SessionSummary>,
        expected_list: RecoveryPublicationList,
    ) -> Result<Vec<SessionSummary>, String> {
        let mut published = Vec::with_capacity(summaries.len());
        for mut summary in summaries {
            let (events, publication_snapshot) =
                if let Some(projection) = self.canonical_session_projection(&summary.id)? {
                    (
                        projection.reducer_events,
                        projection.meta.recovery_publication_snapshot,
                    )
                } else if self.canonical_authority_active() {
                    (Vec::new(), None)
                } else {
                    #[cfg(test)]
                    {
                        (
                            self.test_storage()
                                .load_session_events(_app_data_dir, &summary.id)?,
                            self.test_storage()
                                .get_session_meta(_app_data_dir, &summary.id)?
                                .and_then(|meta| meta.recovery_publication_snapshot),
                        )
                    }
                    #[cfg(not(test))]
                    unreachable!("production always has a SQLite event authority")
                };
            let recovery = TurnEventLog::from_events(events).project().backend_recovery;
            if let Some(BackendSessionRecoveryProjection::Recovering { recovery_id, .. }) = recovery
            {
                let Some(snapshot) = publication_snapshot.filter(|snapshot| {
                    snapshot.recovery_id == recovery_id
                        && snapshot.classification.list == expected_list
                        && recovery_publication_owner_matches(snapshot)
                }) else {
                    // Recovery records written before durable publication
                    // snapshots existed remain suppressed rather than being
                    // reclassified from mutable current state.
                    continue;
                };
                let published_title = summary.first_message.clone();
                summary = snapshot.summary;
                summary.first_message = published_title;
            }
            published.push(summary);
        }
        published.sort_by(|a, b| {
            b.updated_at
                .partial_cmp(&a.updated_at)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Ok(published)
    }

    fn list_sessions_filtered(
        &self,
        app_data_dir: &Path,
        worktree_path: &str,
        predicate: impl Fn(&SessionMeta) -> bool,
    ) -> Result<Vec<SessionSummary>, String> {
        let mut summaries = self
            .list_metas_for_active_read_authority(app_data_dir)?
            .into_iter()
            .filter(|s| same_worktree_path(&s.worktree_path, worktree_path) && predicate(s))
            .map(|meta| meta.to_summary())
            .collect::<Vec<_>>();
        summaries.sort_by(|a, b| {
            b.updated_at
                .partial_cmp(&a.updated_at)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        for summary in &mut summaries {
            if let Some(title) = self.session_title(app_data_dir, &summary.id)? {
                summary.first_message = title;
            }
        }
        Ok(summaries)
    }

    #[cfg(test)]
    pub fn archive_session(&self, app_data_dir: &Path, session_id: &str) -> Result<(), String> {
        let meta = self.require_meta(app_data_dir, session_id)?;
        if meta.state != SessionState::Closed {
            return Err("Only closed sessions can be archived".to_string());
        }
        self.set_session_state(app_data_dir, session_id, SessionState::Archived)
    }

    #[cfg(test)]
    pub fn archive_open_session(
        &self,
        app_data_dir: &Path,
        session_id: &str,
    ) -> Result<(), String> {
        let meta = self.require_meta(app_data_dir, session_id)?;
        if meta.workflow_node_session {
            return Err("Workflow node sessions cannot be archived".to_string());
        }
        self.set_session_state(app_data_dir, session_id, SessionState::Archived)
    }

    pub fn session_title(
        &self,
        _app_data_dir: &Path,
        session_id: &str,
    ) -> Result<Option<String>, String> {
        if let Some(projection) = self.canonical_session_projection(session_id)? {
            return Ok(projection.title);
        }
        if self.canonical_authority_active() {
            return Ok(None);
        }
        #[cfg(test)]
        return self.test_storage().session_title(_app_data_dir, session_id);
        #[cfg(not(test))]
        unreachable!("production always has a SQLite event authority")
    }

    pub fn session_titles(&self, app_data_dir: &Path) -> Result<HashMap<String, String>, String> {
        if self.canonical_authority_active() {
            let mut titles = HashMap::new();
            for meta in self.list_metas_canonical(app_data_dir)? {
                if let Some(title) = self.session_title(app_data_dir, &meta.id)? {
                    titles.insert(meta.id, title);
                }
            }
            return Ok(titles);
        }
        #[cfg(test)]
        return self.test_storage().session_titles(app_data_dir);
        #[cfg(not(test))]
        unreachable!("production always has a SQLite event authority")
    }

    pub fn set_session_title(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        title: Option<&str>,
    ) -> Result<SessionSummary, String> {
        let meta = self.require_meta(app_data_dir, session_id)?;
        if meta.workflow_node_session {
            return Err("Workflow node sessions cannot be renamed".to_string());
        }

        let title_for_summary = title
            .map(compact_session_title)
            .filter(|title| !title.is_empty());
        if !self.canonical_authority_active() {
            #[cfg(test)]
            {
                self.test_storage().write_session_title(
                    app_data_dir,
                    session_id,
                    title_for_summary.as_deref(),
                )?;
                let mut summary = meta.to_summary();
                if let Some(title) = title_for_summary {
                    summary.first_message = title;
                }
                return Ok(summary);
            }
            #[cfg(not(test))]
            return Err("agent-session SQLite event authority is not configured".to_string());
        }
        let current = self
            .canonical_session_projection(session_id)?
            .ok_or_else(|| format!("Session projection not found: {session_id}"))?;
        self.commit_user_session_projection_snapshot(CanonicalAgentSessionProjection {
            meta: current.meta.clone(),
            title: title_for_summary.clone(),
            messages: Vec::new(),
            reducer_events: current.reducer_events,
            queue_paused_at: current.queue_paused_at,
            latest_token_usage: current.latest_token_usage,
            pending_send_queue: current.pending_send_queue,
        })?;

        let mut summary = meta.to_summary();
        if let Some(title) = title_for_summary {
            summary.first_message = title;
        }
        Ok(summary)
    }

    pub fn fork_session(
        &self,
        app_data_dir: &Path,
        session_id: &str,
    ) -> Result<ChatSession, String> {
        self.ensure_canonical_mutation_admission()?;
        let parent_meta = self.require_meta(app_data_dir, session_id)?;
        if parent_meta.workflow_node_session {
            return Err("Workflow node sessions cannot be forked".to_string());
        }

        let now = now_timestamp();
        let mut forked_meta = parent_meta.clone();
        forked_meta.id = uuid::Uuid::new_v4().to_string();
        forked_meta.state = SessionState::Idle;
        forked_meta.error_reason = None;
        forked_meta.created_at = now;
        forked_meta.updated_at = now;
        forked_meta.agent_session_id = None;
        forked_meta.provider_session_generation = 0;
        forked_meta.provider_session_observation_id = None;
        forked_meta.context_reinjection_generation = None;
        forked_meta.context_carry = None;
        forked_meta.recovery_publication_snapshot = None;
        forked_meta.last_turn_interruption = None;
        forked_meta.last_turn_id = Some(0);
        forked_meta.workflow_node_session = false;

        if !self.canonical_authority_active() {
            #[cfg(test)]
            {
                let title = self
                    .test_storage()
                    .session_title(app_data_dir, session_id)?;
                self.test_storage()
                    .fork_session_layout(app_data_dir, session_id, &forked_meta)?;
                if let Some(title) = title.as_deref() {
                    self.test_storage().write_session_title(
                        app_data_dir,
                        &forked_meta.id,
                        Some(title),
                    )?;
                }
                return Ok(forked_meta.to_session(Vec::new()));
            }
            #[cfg(not(test))]
            return Err("agent-session SQLite event authority is not configured".to_string());
        }

        let title = self.session_title(app_data_dir, session_id)?;
        let messages = self.canonical_all_messages(session_id)?;
        self.commit_user_session_projection_snapshot(CanonicalAgentSessionProjection {
            meta: forked_meta.clone(),
            title,
            messages,
            reducer_events: Vec::new(),
            queue_paused_at: None,
            latest_token_usage: None,
            pending_send_queue: Vec::new(),
        })?;
        Ok(forked_meta.to_session(Vec::new()))
    }

    pub fn get_session_shell(
        &self,
        app_data_dir: &Path,
        session_id: &str,
    ) -> Result<Option<ChatSession>, String> {
        Ok(self
            .get_session_meta(app_data_dir, session_id)?
            .map(|meta| meta.to_session(Vec::new())))
    }

    pub fn get_session_with_latest_page(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        limit: usize,
    ) -> Result<Option<(ChatSession, SessionPage, Option<TurnInterruption>)>, String> {
        let Some(meta) = self.get_session_meta(app_data_dir, session_id)? else {
            return Ok(None);
        };
        let page = self
            .get_session_page(app_data_dir, session_id, None, limit)?
            .unwrap_or(SessionPage {
                messages: Vec::new(),
                message_metadata: Vec::new(),
                next_cursor: None,
                has_more: false,
                total_count: meta.message_count,
                latest_token_usage: None,
            });
        let session = meta.to_session(page.messages.clone());
        Ok(Some((session, page, meta.last_turn_interruption)))
    }

    pub fn get_session_meta(
        &self,
        _app_data_dir: &Path,
        session_id: &str,
    ) -> Result<Option<SessionMeta>, String> {
        if let Some(projection) = self.canonical_session_projection(session_id)? {
            return Ok(Some(projection.meta));
        }
        if self.canonical_authority_active() {
            return Ok(None);
        }
        #[cfg(test)]
        return self
            .test_storage()
            .get_session_meta(_app_data_dir, session_id);
        #[cfg(not(test))]
        unreachable!("production always has a SQLite event authority")
    }

    pub fn get_session_review_context(
        &self,
        _app_data_dir: &Path,
        session_id: &str,
    ) -> Result<Option<SessionReviewContext>, String> {
        if let Some(projection) = self.canonical_session_projection(session_id)? {
            return Ok(Some(projection.meta.into()));
        }
        if self.canonical_authority_active() {
            return Ok(None);
        }
        #[cfg(test)]
        return self
            .test_storage()
            .get_session_review_context(_app_data_dir, session_id);
        #[cfg(not(test))]
        unreachable!("production always has a SQLite event authority")
    }

    pub fn load_full_session_for_restore(
        &self,
        _app_data_dir: &Path,
        session_id: &str,
    ) -> Result<Option<ChatSession>, String> {
        if let Some(projection) = self.canonical_session_projection(session_id)? {
            let messages = self.canonical_all_messages(session_id)?;
            return Ok(Some(projection.meta.to_session(messages)));
        }
        if self.canonical_authority_active() {
            return Ok(None);
        }
        #[cfg(test)]
        return self
            .test_storage()
            .load_full_session_for_restore(_app_data_dir, session_id);
        #[cfg(not(test))]
        unreachable!("production always has a SQLite event authority")
    }

    pub fn load_session_events(
        &self,
        _app_data_dir: &Path,
        session_id: &str,
    ) -> Result<Vec<AgentSessionEvent>, String> {
        if let Some(projection) = self.canonical_session_projection(session_id)? {
            return Ok(projection.reducer_events);
        }
        if self.canonical_authority_active() {
            return Ok(Vec::new());
        }
        #[cfg(test)]
        return self
            .test_storage()
            .load_session_events(_app_data_dir, session_id);
        #[cfg(not(test))]
        unreachable!("production always has a SQLite event authority")
    }

    /// Read only the bounded reducer input owned by the current session
    /// projection. Terminal arbitration and operation CAS paths must use this
    /// instead of replaying historical turns.
    pub(crate) fn load_current_reducer_events(
        &self,
        _app_data_dir: &Path,
        session_id: &str,
    ) -> Result<Vec<AgentSessionEvent>, String> {
        if let Some(projection) = self.canonical_session_projection(session_id)? {
            return Ok(projection.reducer_events);
        }
        if self.canonical_authority_active() {
            return Ok(Vec::new());
        }
        #[cfg(test)]
        {
            let events = self
                .test_storage()
                .load_session_events(_app_data_dir, session_id)?;
            Ok(bounded_reducer_events(Vec::new(), &events))
        }
        #[cfg(not(test))]
        unreachable!("production always has a SQLite event authority")
    }

    pub fn load_queue_paused_at(
        &self,
        _app_data_dir: &Path,
        session_id: &str,
    ) -> Result<Option<f64>, String> {
        if let Some(projection) = self.canonical_session_projection(session_id)? {
            return Ok(projection.queue_paused_at);
        }
        if self.canonical_authority_active() {
            return Ok(None);
        }
        #[cfg(test)]
        return self
            .test_storage()
            .load_queue_paused_at(_app_data_dir, session_id);
        #[cfg(not(test))]
        unreachable!("production always has a SQLite event authority")
    }

    #[cfg(test)]
    pub fn append_session_event_and_project_state(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        event: AgentSessionEvent,
    ) -> Result<SessionState, String> {
        self.append_session_event_and_project_read_model(app_data_dir, session_id, event)
            .map(|projected| projected.status.session_state)
    }

    #[cfg(test)]
    pub fn append_session_event_and_project(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        event: AgentSessionEvent,
    ) -> Result<SessionState, String> {
        #[cfg(test)]
        if let Some(hook) = self.append_event_hook.read().clone() {
            hook(session_id, &event)?;
        }
        self.commit_agent_events(app_data_dir, session_id, std::slice::from_ref(&event))?;
        if !self.canonical_authority_active() {
            #[cfg(test)]
            {
                self.test_storage().append_session_events(
                    app_data_dir,
                    session_id,
                    std::slice::from_ref(&event),
                )?;
                let events = self
                    .test_storage()
                    .load_session_events(app_data_dir, session_id)?;
                if self.test_storage().take_event_log_recovered(session_id) {
                    self.notify_event_log_recovered(session_id);
                }
                return self.project_session_events(app_data_dir, session_id, &events);
            }
            #[cfg(not(test))]
            return Err("agent-session SQLite event authority is not configured".to_string());
        }
        let events = self
            .canonical_session_projection(session_id)?
            .map(|projection| projection.reducer_events)
            .unwrap_or_default();
        #[cfg(test)]
        if let Some(hook) = self.appended_event_hook.read().clone() {
            hook(session_id, &event);
        }
        self.project_session_events(app_data_dir, session_id, &events)
    }

    #[cfg(test)]
    pub(crate) fn append_session_event_and_project_read_model(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        event: AgentSessionEvent,
    ) -> Result<SessionReadModel, String> {
        #[cfg(test)]
        if let Some(hook) = self.append_event_hook.read().clone() {
            hook(session_id, &event)?;
        }
        self.commit_agent_events(app_data_dir, session_id, std::slice::from_ref(&event))?;
        if !self.canonical_authority_active() {
            #[cfg(test)]
            {
                self.test_storage().append_session_events(
                    app_data_dir,
                    session_id,
                    std::slice::from_ref(&event),
                )?;
                let events = self
                    .test_storage()
                    .load_session_events(app_data_dir, session_id)?;
                if self.test_storage().take_event_log_recovered(session_id) {
                    self.notify_event_log_recovered(session_id);
                }
                let projected = TurnEventLog::from_events(events.clone()).project();
                self.project_session_events(app_data_dir, session_id, &events)?;
                return Ok(projected);
            }
            #[cfg(not(test))]
            return Err("agent-session SQLite event authority is not configured".to_string());
        }
        let events = self
            .canonical_session_projection(session_id)?
            .map(|projection| projection.reducer_events)
            .ok_or_else(|| format!("Session projection not found: {session_id}"))?;
        let projected = TurnEventLog::from_events(events.clone()).project();
        Ok(projected)
    }

    #[cfg(test)]
    pub(crate) fn append_error_episode_and_materialize(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        input: ErrorEpisodeInput,
    ) -> Result<(SessionReadModel, ChatMessage), String> {
        self.append_error_episode_with_queue_policy_and_materialize(
            app_data_dir,
            session_id,
            input,
            false,
        )
    }

    pub(crate) fn append_error_episode_and_pause_queue(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        input: ErrorEpisodeInput,
    ) -> Result<(SessionReadModel, ChatMessage), String> {
        self.append_error_episode_with_queue_policy_and_materialize(
            app_data_dir,
            session_id,
            input,
            true,
        )
    }

    fn append_error_episode_with_queue_policy_and_materialize(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        input: ErrorEpisodeInput,
        pause_queue: bool,
    ) -> Result<(SessionReadModel, ChatMessage), String> {
        let message_id = input.message_id;
        let event = AgentSessionEvent::SessionErrored {
            message_id: message_id.clone(),
            reason: input.reason,
            at: input.at,
        };
        let queue_was_paused = pause_queue
            && self
                .load_queue_paused_at(app_data_dir, session_id)?
                .is_some();
        #[cfg(test)]
        if let Some(hook) = self.append_event_hook.read().clone() {
            hook(session_id, &event)?;
        }
        let mut events = vec![event];
        if pause_queue && !queue_was_paused {
            events.push(AgentSessionEvent::QueuePaused { at: input.at });
        }
        let (projected, message, _) = self.commit_projection_and_notify_with_queue_guard(
            app_data_dir,
            session_id,
            &events,
            pause_queue.then_some(queue_was_paused),
            |projected, projected_meta| {
                let message = projected
                    .message_for_id(&message_id)
                    .cloned()
                    .ok_or_else(|| {
                        format!("Error projection omitted message {message_id} for {session_id}")
                    })?;
                #[cfg(test)]
                if let Some(hook) = self.append_message_hook.read().clone() {
                    hook(session_id, &message)?;
                }
                Ok((
                    AgentSessionProjectionCommit {
                        meta: projected_meta,
                        message: AgentSessionProjectedMessage::Append(message.clone()),
                    },
                    message,
                ))
            },
        )?;
        Ok((projected, message))
    }

    #[allow(clippy::too_many_arguments)] // Terminal identity, projection patch, and result must enter one commit together.
    pub(crate) fn append_terminal_events_and_materialize(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        events: &[AgentSessionEvent],
        message_id: &str,
        streaming_final_seq: u64,
        completed_at: f64,
        turn_result: &crate::domain::agent_session::entities::TurnResult,
    ) -> Result<(SessionReadModel, Vec<MessagePart>), String> {
        #[cfg(test)]
        if let Some(hook) = self.append_event_hook.read().clone() {
            for event in events {
                hook(session_id, event)?;
            }
        }
        if self.canonical_authority_active() {
            let codec = self
                .event_authority
                .read()
                .as_ref()
                .map(|authority| authority.projection_codec.clone())
                .ok_or_else(|| "agent-session projection codec is unavailable".to_string())?;
            let previous = self
                .canonical_session_projection(session_id)?
                .ok_or_else(|| format!("Session projection not found: {session_id}"))?;
            let candidate_turn_id = events.iter().rev().find_map(|event| match event {
                AgentSessionEvent::TurnCompleted { turn_id, .. }
                | AgentSessionEvent::TurnInterrupted { turn_id, .. } => Some(*turn_id),
                _ => None,
            });
            let current_turn_id =
                previous
                    .reducer_events
                    .iter()
                    .rev()
                    .find_map(|event| match event {
                        AgentSessionEvent::TurnStarted { turn_id, .. } => Some(*turn_id),
                        _ => None,
                    });
            let durable_winner = candidate_turn_id
                .map(|turn_id| self.canonical_terminal(session_id, turn_id))
                .transpose()?
                .flatten();
            if candidate_turn_id.is_some()
                && (candidate_turn_id != current_turn_id || durable_winner.is_some())
            {
                let projected =
                    TurnEventLog::from_events(previous.reducer_events.clone()).project();
                let persisted_parts = self
                    .canonical_message_projection(session_id, message_id)?
                    .and_then(|message| message.parts)
                    .unwrap_or_else(|| projected.agent_parts_for_message(message_id));
                return Ok((projected, persisted_parts));
            }
            let events = complete_terminal_projection_events(&previous.reducer_events, events);
            if !events.is_empty() {
                let encoded_events = codec.encode_events_for_identity(&events)?;
                let terminal_mutation = runtime_terminal_record_mutation(
                    session_id,
                    &events,
                    message_id,
                    streaming_final_seq,
                    completed_at,
                    turn_result,
                    &encoded_events,
                )?;
                let mut additional_mutations = vec![terminal_mutation];
                let terminal_record = match &additional_mutations[0] {
                    crate::domain::local_event::LocalStateMutation::TerminalRecord(record) => {
                        record.clone()
                    }
                    _ => unreachable!("runtime terminal builder always returns a terminal row"),
                };
                if let Some(workflow_context) = previous.meta.workflow_node_context.as_ref() {
                    let mut candidate_events = previous.reducer_events.clone();
                    candidate_events.extend(events.iter().cloned());
                    let workflow_input = TurnEventLog::from_events(candidate_events)
                        .project()
                        .workflow_turn_complete
                        .ok_or_else(|| {
                            "workflow-owned terminal omitted its turn-completion input".to_string()
                        })?;
                    if candidate_turn_id != Some(workflow_input.turn_id) {
                        return Err(
                            "workflow-owned terminal projected a different turn identity"
                                .to_string(),
                        );
                    }
                    // The workflow turn-complete usecase intentionally treats
                    // a clean interruption as a no-op. Do not create an
                    // impossible completion obligation that would otherwise
                    // block orphan recovery forever waiting for a workflow
                    // commit which is not meant to exist.
                    if !(workflow_input.interrupted
                        && workflow_input.exit_code == 0
                        && workflow_input.failure_signal.is_none())
                    {
                        additional_mutations.push(workflow_turn_completion_pending_mutation(
                            session_id,
                            workflow_context,
                            &terminal_record,
                            message_id,
                            &workflow_input,
                        )?);
                    }
                }
                let participant_provider =
                    self.runtime_terminal_participant_provider.read().clone();
                if participant_provider.is_none() {
                    #[cfg(not(test))]
                    return Err(
                        "runtime terminal participant provider is not configured".to_string()
                    );
                }
                let terminal_participant =
                    participant_provider.map(|provider| (provider, terminal_record));
                if let Err(error) = self.commit_agent_events_with_additional_mutations(
                    session_id,
                    &events,
                    additional_mutations,
                    Some(TerminalMessageProjectionPatch {
                        message_id: message_id.to_string(),
                        streaming_final_seq,
                        timestamp: Some(completed_at),
                        parts: None,
                    }),
                    None,
                    terminal_participant,
                    crate::domain::local_event::CommitOperationKind::Projection,
                ) {
                    let converged = candidate_turn_id
                        .map(|turn_id| self.canonical_terminal(session_id, turn_id))
                        .transpose()?
                        .flatten()
                        .is_some();
                    if !converged {
                        return Err(error);
                    }
                }
            }
            let canonical = self
                .canonical_session_projection(session_id)?
                .ok_or_else(|| format!("Session projection not found: {session_id}"))?;
            let projected = TurnEventLog::from_events(canonical.reducer_events.clone()).project();
            self.projected_meta_for_commit(
                session_id,
                &canonical.meta,
                &canonical.reducer_events,
                &projected,
            )?;
            projected
                .message_for_id(message_id)
                .filter(|message| message.role == MessageRole::Agent)
                .ok_or_else(|| {
                    format!("Turn projection omitted message {message_id} for {session_id}")
                })?;
            let persisted_parts = projected.agent_parts_for_message(message_id);
            #[cfg(test)]
            if let Some(hook) = self.persist_parts_hook.read().clone() {
                hook(session_id, message_id, &persisted_parts)?;
            }
            self.notify_projected_commit(
                session_id,
                Some(PreviousSessionProjection {
                    state: previous.meta.state,
                    error_reason: previous.meta.error_reason,
                    worktree_path: previous.meta.worktree_path,
                    state_revision: canonical.meta.state_revision,
                }),
                &projected,
            );
            return Ok((projected, persisted_parts));
        }
        let (projected, (), persisted_parts) = self.commit_projection_and_notify(
            app_data_dir,
            session_id,
            events,
            |projected, projected_meta| {
                projected
                    .message_for_id(message_id)
                    .filter(|message| message.role == MessageRole::Agent)
                    .ok_or_else(|| {
                        format!("Turn projection omitted message {message_id} for {session_id}")
                    })?;
                let parts = projected.agent_parts_for_message(message_id);
                #[cfg(test)]
                if let Some(hook) = self.persist_parts_hook.read().clone() {
                    hook(session_id, message_id, &parts)?;
                }
                Ok((
                    AgentSessionProjectionCommit {
                        meta: projected_meta,
                        message: AgentSessionProjectedMessage::PersistParts {
                            message_id: message_id.to_string(),
                            parts,
                            streaming_final_seq,
                            completed_at,
                        },
                    },
                    (),
                ))
            },
        )?;
        Ok((projected, persisted_parts))
    }

    fn commit_projection_and_notify<Output>(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        events: &[AgentSessionEvent],
        build_commit: impl FnMut(
            &SessionReadModel,
            SessionMeta,
        ) -> Result<
            (
                AgentSessionProjectionCommit<SessionMeta, ChatMessage, MessagePart>,
                Output,
            ),
            String,
        >,
    ) -> Result<(SessionReadModel, Output, Vec<MessagePart>), String> {
        self.commit_projection_and_notify_with_queue_guard(
            app_data_dir,
            session_id,
            events,
            None,
            build_commit,
        )
    }

    fn commit_projection_and_notify_with_queue_guard<Output>(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        events: &[AgentSessionEvent],
        expected_queue_paused: Option<bool>,
        mut build_commit: impl FnMut(
            &SessionReadModel,
            SessionMeta,
        ) -> Result<
            (
                AgentSessionProjectionCommit<SessionMeta, ChatMessage, MessagePart>,
                Output,
            ),
            String,
        >,
    ) -> Result<(SessionReadModel, Output, Vec<MessagePart>), String> {
        self.ensure_canonical_mutation_admission()?;
        if !self.canonical_authority_active() {
            #[cfg(test)]
            {
                let mut projected_result = None;
                let mut previous_projection = None;
                let persisted_parts = {
                    let mut prepare = |all_events: &[AgentSessionEvent], meta: &SessionMeta| {
                        let projected = TurnEventLog::from_events(all_events.to_vec()).project();
                        let mut projected_meta = self
                            .projected_meta_for_commit(session_id, meta, all_events, &projected)?;
                        projected_meta.state_revision =
                            next_sqlite_counter(meta.state_revision, "session state revision")?;
                        previous_projection = Some(PreviousSessionProjection {
                            state: meta.state.clone(),
                            error_reason: meta.error_reason.clone(),
                            worktree_path: meta.worktree_path.clone(),
                            state_revision: projected_meta.state_revision,
                        });
                        let (commit, output) = build_commit(&projected, projected_meta)?;
                        projected_result = Some((projected, output));
                        Ok(commit)
                    };
                    self.test_storage().commit_session_projection(
                        app_data_dir,
                        session_id,
                        events,
                        &mut prepare,
                    )?
                };
                if self.test_storage().take_event_log_recovered(session_id) {
                    self.notify_event_log_recovered(session_id);
                }
                let (projected, output) = projected_result
                    .expect("commit_session_projection must invoke prepare before returning Ok");
                self.notify_projected_commit(session_id, previous_projection, &projected);
                return Ok((projected, output, persisted_parts));
            }
            #[cfg(not(test))]
            return Err("agent-session SQLite event authority is not configured".to_string());
        }

        let previous_canonical = self
            .canonical_session_projection(session_id)?
            .ok_or_else(|| format!("Session projection not found: {session_id}"))?;
        let previous_projection = Some(PreviousSessionProjection {
            state: previous_canonical.meta.state.clone(),
            error_reason: previous_canonical.meta.error_reason.clone(),
            worktree_path: previous_canonical.meta.worktree_path.clone(),
            state_revision: previous_canonical.meta.state_revision,
        });
        match expected_queue_paused {
            Some(expected_queue_paused) => self.commit_agent_events_with_queue_pause_guard(
                app_data_dir,
                session_id,
                events,
                expected_queue_paused,
            )?,
            None => self.commit_agent_events(app_data_dir, session_id, events)?,
        }
        let canonical = self
            .canonical_session_projection(session_id)?
            .ok_or_else(|| format!("Session projection not found: {session_id}"))?;
        let projected = TurnEventLog::from_events(canonical.reducer_events.clone()).project();
        let mut projected_meta = self.projected_meta_for_commit(
            session_id,
            &canonical.meta,
            &canonical.reducer_events,
            &projected,
        )?;
        projected_meta.state_revision = canonical.meta.state_revision;
        let (derived_commit, output) = build_commit(&projected, projected_meta)?;
        let persisted_parts = match &derived_commit.message {
            AgentSessionProjectedMessage::PersistParts { parts, .. } => parts.clone(),
            AgentSessionProjectedMessage::Append(_) => Vec::new(),
        };
        self.notify_projected_commit(session_id, previous_projection, &projected);
        Ok((projected, output, persisted_parts))
    }

    fn projected_meta_for_commit(
        &self,
        _session_id: &str,
        meta: &SessionMeta,
        events: &[AgentSessionEvent],
        projected: &SessionReadModel,
    ) -> Result<SessionMeta, String> {
        #[cfg(test)]
        if let Some(hook) = self.projection_hook.read().clone() {
            hook(
                _session_id,
                &projected.status.session_state,
                projected.error_reason.as_deref(),
            )?;
        }
        let mut projected_meta = meta.clone();
        projected_meta.state = projected.status.session_state.clone();
        projected_meta.error_reason =
            error_reason_for_state(&projected_meta.state, &projected.error_reason);
        projected_meta.last_turn_interruption = latest_turn_interruption(events);
        projected_meta.last_turn_id = events.iter().rev().find_map(|event| match event {
            AgentSessionEvent::TurnStarted { turn_id, .. } => Some(*turn_id),
            _ => None,
        });
        #[cfg(test)]
        if let Some(hook) = self.event_projection_hook.read().clone() {
            hook(_session_id, projected_meta.last_turn_id)?;
        }
        Ok(projected_meta)
    }

    fn notify_projected_commit(
        &self,
        session_id: &str,
        previous_projection: Option<PreviousSessionProjection>,
        projected: &SessionReadModel,
    ) {
        #[cfg(test)]
        if let Some(hook) = self.projected_read_model_hook.read().clone() {
            hook(session_id, projected);
        }
        let previous = previous_projection
            .expect("commit_session_projection must invoke prepare before returning Ok");
        let projected_reason =
            error_reason_for_state(&projected.status.session_state, &projected.error_reason);
        if previous.state != projected.status.session_state
            || previous.error_reason != projected_reason
        {
            self.notify_state_change(
                session_id,
                &previous.worktree_path,
                &projected.status.session_state,
                previous.state_revision,
            );
        }
    }

    pub fn append_turn_started_and_project_state(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        event: AgentSessionEvent,
    ) -> Result<(), String> {
        let _turn_id = match &event {
            AgentSessionEvent::TurnStarted { turn_id, .. } => *turn_id,
            _ => return Err("Turn start projection requires a TurnStarted event".to_string()),
        };
        self.append_session_event_without_projection(app_data_dir, session_id, event.clone())?;
        #[cfg(test)]
        if !self.canonical_authority_active() {
            if let Err(projection_error) = self.set_event_projection(
                app_data_dir,
                session_id,
                SessionState::Active,
                None,
                None,
                Some(_turn_id),
            ) {
                let recovery =
                    self.load_session_events(app_data_dir, session_id)
                        .and_then(|events| {
                            self.project_session_events(app_data_dir, session_id, &events)
                                .map(|_| ())
                        });
                return match recovery {
                    Ok(()) => Err(projection_error),
                    Err(recovery_error) => Err(format!(
                        "{projection_error}; failed to recover committed turn projection: {recovery_error}"
                    )),
                };
            }
        }
        #[cfg(test)]
        if let Some(hook) = self.appended_event_hook.read().clone() {
            hook(session_id, &event);
        }
        Ok(())
    }

    /// Commit the dequeue boundary for a durably accepted queued send. The
    /// transaction verifies and removes only the exact canonical queue front;
    /// a recovery worker restoring a later item cannot advance it first.
    #[cfg(test)]
    pub(crate) fn append_accepted_queued_turn_started_and_project_state(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        queue_item_id: &str,
        event: AgentSessionEvent,
    ) -> Result<(), String> {
        if !matches!(&event, AgentSessionEvent::TurnStarted { .. }) {
            return Err("Accepted queue projection requires a TurnStarted event".to_string());
        }
        if self.canonical_authority_active() {
            return self.commit_agent_events_with_kind_and_queue_front(
                app_data_dir,
                session_id,
                std::slice::from_ref(&event),
                crate::domain::local_event::CommitOperationKind::Projection,
                Some(ExpectedAcceptedQueueFront {
                    queue_item_id: queue_item_id.to_string(),
                }),
                Vec::new(),
                None,
            );
        }
        #[cfg(test)]
        {
            self.append_turn_started_and_project_state(app_data_dir, session_id, event)
        }
        #[cfg(not(test))]
        {
            Err("agent-session SQLite event authority is not configured".to_string())
        }
    }

    /// Atomically claim a queued send and materialize its canonical
    /// `TurnStarted` boundary. Lifecycle/recovery winners leave every supplied
    /// operation participant untouched and retain the exact queue item.
    pub(crate) fn commit_accepted_queued_turn_start_with_participants(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        queue_item_id: &str,
        event: AgentSessionEvent,
        additional_mutations: Vec<crate::domain::local_event::LocalStateMutation>,
    ) -> Result<AcceptedQueuedTurnStartCommitOutcome, String> {
        if !matches!(&event, AgentSessionEvent::TurnStarted { .. }) {
            return Err("Accepted queue projection requires a TurnStarted event".to_string());
        }
        if !self.canonical_authority_active() {
            return Err("agent-session SQLite event authority is not configured".to_string());
        }
        let result = self.commit_agent_events_with_kind_and_queue_front(
            app_data_dir,
            session_id,
            std::slice::from_ref(&event),
            crate::domain::local_event::CommitOperationKind::Send,
            Some(ExpectedAcceptedQueueFront {
                queue_item_id: queue_item_id.to_string(),
            }),
            additional_mutations,
            None,
        );
        match result {
            Ok(()) => {
                #[cfg(test)]
                if let Some(hook) = self.appended_event_hook.read().clone() {
                    hook(session_id, &event);
                }
                Ok(AcceptedQueuedTurnStartCommitOutcome::Committed)
            }
            Err(error) if error.starts_with(ACCEPTED_QUEUE_START_BLOCKED) => {
                Ok(AcceptedQueuedTurnStartCommitOutcome::Blocked)
            }
            Err(error) => Err(error),
        }
    }

    #[cfg(test)]
    pub fn project_session_events(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        events: &[AgentSessionEvent],
    ) -> Result<SessionState, String> {
        let last_turn_interruption = latest_turn_interruption(events);
        let last_turn_id = events.iter().rev().find_map(|event| match event {
            AgentSessionEvent::TurnStarted { turn_id, .. } => Some(*turn_id),
            _ => None,
        });
        let projected = TurnEventLog::from_events(events.to_vec()).project();
        let projected_state = projected.status.session_state.clone();
        self.set_event_projection(
            app_data_dir,
            session_id,
            projected_state.clone(),
            projected.error_reason,
            last_turn_interruption,
            last_turn_id,
        )?;
        Ok(projected_state)
    }

    pub(crate) fn next_turn_id(
        &self,
        app_data_dir: &Path,
        session_id: &str,
    ) -> Result<u64, NextTurnIdError> {
        if self.canonical_authority_active() {
            return self
                .send_acceptance_allocation(session_id)
                .map(|allocation| allocation.next_turn_id);
        }

        let meta = self.require_meta(app_data_dir, session_id)?;
        #[cfg(test)]
        let last_turn_id = match meta.last_turn_id {
            Some(turn_id) => turn_id,
            None => self
                .test_storage()
                .load_session_events(app_data_dir, session_id)?
                .iter()
                .rev()
                .find_map(|event| match event {
                    AgentSessionEvent::TurnStarted { turn_id, .. } => Some(*turn_id),
                    _ => None,
                })
                .unwrap_or(0),
        };
        #[cfg(not(test))]
        let last_turn_id = meta.last_turn_id.unwrap_or(0);
        if last_turn_id >= i64::MAX as u64 {
            return Err(NextTurnIdError::CapacityExceeded);
        }
        last_turn_id
            .checked_add(1)
            .ok_or(NextTurnIdError::CapacityExceeded)
    }

    /// Allocate from one canonical projection snapshot and return the exact
    /// revision that must guard the later acceptance mutation. Queue identity
    /// is strictly increasing in canonical order; accepting new work on a
    /// malformed queue would otherwise make turn reuse permanent.
    pub(crate) fn send_acceptance_allocation(
        &self,
        session_id: &str,
    ) -> Result<SendAcceptanceAllocation, NextTurnIdError> {
        let (projection, revision) = self
            .canonical_session_projection_with_revision(session_id)?
            .ok_or_else(|| format!("Session projection not found: {session_id}"))?;
        let mut last_turn_id = projection.meta.last_turn_id.unwrap_or_else(|| {
            projection
                .reducer_events
                .iter()
                .rev()
                .find_map(|event| match event {
                    AgentSessionEvent::TurnStarted { turn_id, .. } => Some(*turn_id),
                    _ => None,
                })
                .unwrap_or(0)
        });
        for pending in &projection.pending_send_queue {
            let reserved_turn_id = pending.reserved_turn_id.parse::<u64>().map_err(|_| {
                format!(
                    "queued send {} has an invalid reserved turn identity",
                    pending.queue_item_id
                )
            })?;
            if reserved_turn_id <= last_turn_id {
                return Err(format!(
                    "queued send {} does not advance the canonical turn identity",
                    pending.queue_item_id
                )
                .into());
            }
            last_turn_id = reserved_turn_id;
        }
        if last_turn_id >= i64::MAX as u64 {
            return Err(NextTurnIdError::CapacityExceeded);
        }
        Ok(SendAcceptanceAllocation {
            next_turn_id: last_turn_id
                .checked_add(1)
                .ok_or(NextTurnIdError::CapacityExceeded)?,
            has_active_turn: projection.meta.state == SessionState::Active,
            has_pending_queue: !projection.pending_send_queue.is_empty(),
            session_projection_guard: crate::domain::local_event::RevisionGuard::Expected(revision),
        })
    }

    /// Read the canonical owner of an immediately accepted provider turn.
    ///
    /// This is a preflight optimization for the runtime adapter. The SQLite
    /// writer repeats the same check in the claim transaction, which closes
    /// the read/claim race.
    pub(crate) fn canonical_active_turn_matches(
        &self,
        session_id: &str,
        turn_id: u64,
    ) -> Result<bool, String> {
        Ok(self
            .canonical_session_projection(session_id)?
            .is_some_and(|projection| {
                projection.meta.state == SessionState::Active
                    && projection.meta.last_turn_id == Some(turn_id)
            }))
    }

    pub fn append_session_event_without_projection(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        event: AgentSessionEvent,
    ) -> Result<(), String> {
        #[cfg(test)]
        if let Some(hook) = self.append_event_hook.read().clone() {
            hook(session_id, &event)?;
        }
        self.commit_agent_events(app_data_dir, session_id, std::slice::from_ref(&event))?;
        if !self.canonical_authority_active() {
            #[cfg(test)]
            {
                self.test_storage()
                    .append_session_event_without_projection(app_data_dir, session_id, &event)?;
                if self.test_storage().take_event_log_recovered(session_id) {
                    self.notify_event_log_recovered(session_id);
                }
                return Ok(());
            }
            #[cfg(not(test))]
            return Err("agent-session SQLite event authority is not configured".to_string());
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn reserve_permission_response(
        &self,
        _app_data_dir: &Path,
        session_id: &str,
        turn_id: u64,
        request_id: &str,
        exact_response: crate::domain::agent_session::entities::PermissionResponse,
    ) -> Result<String, String> {
        let obligation_id = format!("permission-response:{session_id}:{turn_id}:{request_id}");
        let turn_id_text = turn_id.to_string();
        let at = now_timestamp();
        if exact_response.request_id != request_id {
            return Err("permission response request identity is inconsistent".to_string());
        }
        let record = crate::domain::local_event::ObligationRecord::PermissionResponse {
            operation_id: obligation_id.clone(),
            effect_identity: obligation_id.clone(),
            session_id: session_id.to_string(),
            turn_id: turn_id_text.clone(),
            response: exact_response,
            owner_access: true,
            from_runtime_state: true,
            state: crate::domain::local_event::ObligationStateRecord::Pending,
        };

        let matches_pending = |stored: &crate::domain::local_event::ObligationRecord| {
            matches!(
                stored,
                crate::domain::local_event::ObligationRecord::PermissionResponse {
                    operation_id,
                    effect_identity,
                    session_id: stored_session_id,
                    turn_id: stored_turn_id,
                    response,
                    owner_access: true,
                    from_runtime_state: true,
                    state: crate::domain::local_event::ObligationStateRecord::Pending,
                } if operation_id == &obligation_id
                    && effect_identity == &obligation_id
                    && stored_session_id == session_id
                    && stored_turn_id == &turn_id_text
                    && response == match &record {
                        crate::domain::local_event::ObligationRecord::PermissionResponse { response, .. } => response,
                        _ => unreachable!(),
                    }
            )
        };

        if !self.canonical_authority_active() {
            #[cfg(test)]
            {
                let mut reservations = self.permission_response_reservations.write();
                if let Some(current) = reservations.get(&obligation_id) {
                    if matches_pending(current) {
                        return Ok(obligation_id);
                    }
                    return Err(format!(
                        "permission response {request_id} already has a claimed effect and requires reconciliation"
                    ));
                }
                reservations.insert(obligation_id.clone(), record);
                return Ok(obligation_id);
            }
            #[cfg(not(test))]
            return Err(
                "permission responses require the canonical local-event authority".to_string(),
            );
        }
        if let Some(current) = self.canonical_obligation(&obligation_id)? {
            if matches_pending(&current.record) {
                return Ok(obligation_id);
            }
            return Err(format!(
                "permission response {request_id} already has a claimed effect and requires reconciliation"
            ));
        }
        let obligation = backend_recovery_obligation_mutation(
            obligation_id.clone(),
            record.clone(),
            Some(crate::domain::local_event::PendingIndexEntry {
                ordered_key: format!(
                    "permission-response:{:020}:{obligation_id}",
                    (at * 1000.0).round() as i64
                ),
                owner: session_id.to_string(),
                partition: crate::domain::local_event::PendingPartition::Owner,
                shutdown_plan: None,
            }),
            None,
        )?;
        let event = AgentSessionEvent::ObligationRecorded {
            obligation_id: obligation_id.clone(),
            kind: crate::domain::agent_session::events::ObligationKind::PermissionResponse,
            state: crate::domain::agent_session::events::ObligationState::Pending,
            at,
        };
        self.commit_agent_events_with_additional_mutations(
            session_id,
            std::slice::from_ref(&event),
            vec![obligation],
            None,
            None,
            None,
            crate::domain::local_event::CommitOperationKind::Recovery,
        )?;
        let fresh = self.canonical_obligation(&obligation_id)?.ok_or_else(|| {
            "permission response reservation was not readable after commit".to_string()
        })?;
        if fresh.record != record
            || fresh.pending.as_ref().map(|pending| pending.owner.as_str()) != Some(session_id)
        {
            return Err(
                "permission response reservation fresh-read did not match the accepted response"
                    .to_string(),
            );
        }
        Ok(obligation_id)
    }

    #[cfg(test)]
    pub(crate) fn claim_permission_response_effect(
        &self,
        session_id: &str,
        obligation_id: &str,
    ) -> Result<(), String> {
        if !self.canonical_authority_active() {
            #[cfg(test)]
            {
                let mut reservations = self.permission_response_reservations.write();
                let stored = reservations.get_mut(obligation_id).ok_or_else(|| {
                    "permission response claim has no durable reservation".to_string()
                })?;
                match stored {
                    crate::domain::local_event::ObligationRecord::PermissionResponse {
                        session_id: stored_session_id,
                        state,
                        ..
                    } if stored_session_id == session_id
                        && *state == crate::domain::local_event::ObligationStateRecord::Pending =>
                    {
                        *state = crate::domain::local_event::ObligationStateRecord::EffectReserved;
                    }
                    _ => {
                        return Err(
							"permission response effect was already claimed and requires reconciliation"
								.to_string(),
						);
                    }
                }
                return Ok(());
            }
            #[cfg(not(test))]
            return Err(
                "permission responses require the canonical local-event authority".to_string(),
            );
        }

        let current = self
            .canonical_obligation(obligation_id)?
            .ok_or_else(|| "permission response claim has no durable reservation".to_string())?;
        let mut record = current.record.clone();
        match &mut record {
            crate::domain::local_event::ObligationRecord::PermissionResponse {
                session_id: stored_session_id,
                state,
                ..
            } if stored_session_id == session_id
                && *state == crate::domain::local_event::ObligationStateRecord::Pending =>
            {
                *state = crate::domain::local_event::ObligationStateRecord::EffectReserved;
            }
            _ => {
                return Err(
                    "permission response effect was already claimed and requires reconciliation"
                        .to_string(),
                );
            }
        }
        let at = now_timestamp();
        let pending =
            current
                .pending
                .as_ref()
                .map(|pending| crate::domain::local_event::PendingIndexEntry {
                    ordered_key: pending.ordered_key.clone(),
                    owner: pending.owner.clone(),
                    partition: pending.partition,
                    shutdown_plan: pending.shutdown_plan.clone(),
                });
        let obligation = backend_recovery_obligation_mutation(
            obligation_id.to_string(),
            record,
            pending,
            Some(&current),
        )?;
        let event = AgentSessionEvent::ObligationRecorded {
            obligation_id: obligation_id.to_string(),
            kind: crate::domain::agent_session::events::ObligationKind::PermissionResponse,
            state: crate::domain::agent_session::events::ObligationState::EffectReserved,
            at,
        };
        self.commit_agent_events_with_additional_mutations(
            session_id,
            std::slice::from_ref(&event),
            vec![obligation],
            None,
            None,
            None,
            crate::domain::local_event::CommitOperationKind::Recovery,
        )?;
        let fresh = self
            .canonical_obligation(obligation_id)?
            .ok_or_else(|| "permission response claim was not readable after commit".to_string())?;
        if !matches!(
            fresh.record,
            crate::domain::local_event::ObligationRecord::PermissionResponse {
                state: crate::domain::local_event::ObligationStateRecord::EffectReserved,
                ..
            }
        ) {
            return Err(
                "permission response claim outcome could not be verified; provider effect was not started"
                    .to_string(),
            );
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn load_permission_response_obligation(
        &self,
        obligation_id: &str,
    ) -> Result<Option<crate::domain::local_event::ObligationStateRecord>, String> {
        if self.canonical_authority_active() {
            return self.canonical_obligation(obligation_id).map(|obligation| {
                obligation.and_then(|obligation| match obligation.record {
                    crate::domain::local_event::ObligationRecord::PermissionResponse {
                        state,
                        ..
                    } => Some(state),
                    _ => None,
                })
            });
        }
        Ok(self
            .permission_response_reservations
            .read()
            .get(obligation_id)
            .and_then(|record| match record {
                crate::domain::local_event::ObligationRecord::PermissionResponse {
                    state, ..
                } => Some(*state),
                _ => None,
            }))
    }

    #[cfg(test)]
    pub(crate) fn complete_permission_response(
        &self,
        _app_data_dir: &Path,
        session_id: &str,
        obligation_id: &str,
        resolved_event: AgentSessionEvent,
        message_id: Option<&str>,
        streaming_final_seq: Option<u64>,
    ) -> Result<(), String> {
        if !self.canonical_authority_active() {
            #[cfg(test)]
            {
                let state = self
                    .permission_response_reservations
                    .read()
                    .get(obligation_id)
                    .and_then(|record| match record {
                        crate::domain::local_event::ObligationRecord::PermissionResponse {
                            state,
                            ..
                        } => Some(*state),
                        _ => None,
                    });
                if state != Some(crate::domain::local_event::ObligationStateRecord::EffectReserved)
                {
                    return Err("permission response completion has no claimed effect".to_string());
                }
                if let (Some(message_id), Some(streaming_final_seq)) =
                    (message_id, streaming_final_seq)
                {
                    let mut events = self.load_session_events(_app_data_dir, session_id)?;
                    events.push(resolved_event.clone());
                    let projected = TurnEventLog::from_events(events).project();
                    self.persist_message_parts(
                        _app_data_dir,
                        session_id,
                        message_id,
                        &projected.agent_parts_for_message(message_id),
                        streaming_final_seq,
                        None,
                    )?;
                }
                self.append_session_event_and_project_state(
                    _app_data_dir,
                    session_id,
                    resolved_event,
                )?;
                self.permission_response_reservations
                    .write()
                    .remove(obligation_id);
                return Ok(());
            }
            #[cfg(not(test))]
            return Err(
                "permission responses require the canonical local-event authority".to_string(),
            );
        }
        let current = self.canonical_obligation(obligation_id)?.ok_or_else(|| {
            "permission response completion has no durable reservation".to_string()
        })?;
        let mut record = current.record.clone();
        match &mut record {
            crate::domain::local_event::ObligationRecord::PermissionResponse {
                state: crate::domain::local_event::ObligationStateRecord::Completed,
                ..
            } => return Ok(()),
            crate::domain::local_event::ObligationRecord::PermissionResponse { state, .. }
                if *state == crate::domain::local_event::ObligationStateRecord::EffectReserved =>
            {
                *state = crate::domain::local_event::ObligationStateRecord::Completed;
            }
            _ => {
                return Err("permission response reservation requires reconciliation".to_string());
            }
        }
        let at = now_timestamp();
        let obligation = backend_recovery_obligation_mutation(
            obligation_id.to_string(),
            record,
            None,
            Some(&current),
        )?;
        let events = vec![
            resolved_event,
            AgentSessionEvent::ObligationRecorded {
                obligation_id: obligation_id.to_string(),
                kind: crate::domain::agent_session::events::ObligationKind::PermissionResponse,
                state: crate::domain::agent_session::events::ObligationState::Completed,
                at,
            },
        ];
        let message_patch = match (message_id, streaming_final_seq) {
            (Some(message_id), Some(streaming_final_seq)) => Some(TerminalMessageProjectionPatch {
                message_id: message_id.to_string(),
                streaming_final_seq,
                timestamp: None,
                parts: None,
            }),
            (None, None) => None,
            _ => {
                return Err("permission response message projection is incomplete".to_string());
            }
        };
        self.commit_agent_events_with_additional_mutations(
            session_id,
            &events,
            vec![obligation],
            message_patch,
            None,
            None,
            crate::domain::local_event::CommitOperationKind::Recovery,
        )
    }

    pub fn begin_backend_session_recovery(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        recovery_id: &str,
        reason: BackendSessionRecoveryReason,
    ) -> Result<BackendSessionRecoveryStartOutcome, String> {
        self.ensure_canonical_mutation_admission()?;
        let current_meta = self.require_meta(app_data_dir, session_id)?;
        let old_provider_session_generation = current_meta.provider_session_generation;
        let publication_snapshot = recovery_publication_snapshot(recovery_id, &current_meta);
        let at = now_timestamp();
        let event = AgentSessionEvent::BackendSessionRecoveryStarted {
            recovery_id: recovery_id.to_string(),
            old_provider_session_generation,
            reason,
            at,
        };
        #[cfg(test)]
        if let Some(hook) = self.append_event_hook.read().clone() {
            hook(session_id, &event)?;
        }
        if !self.canonical_authority_active() {
            #[cfg(test)]
            {
                let mut updated = None;
                self.test_storage()
                    .update_session_meta_and_append_session_events(
                app_data_dir,
                session_id,
                &mut |meta| {
                    if meta.provider_session_generation != old_provider_session_generation {
                        return Err(format!(
                            "Backend session generation changed while starting recovery: expected {old_provider_session_generation}, actual {}",
                            meta.provider_session_generation
                        ));
                    }
                    meta.agent_session_id = None;
                    meta.provider_session_observation_id = None;
                    meta.context_reinjection_generation = None;
                    meta.context_carry = Some(ContextCarryState::Failed);
                    meta.recovery_publication_snapshot = Some(publication_snapshot.clone());
                    meta.updated_at = at;
                    updated = Some(meta.clone());
                    Ok(())
                },
                std::slice::from_ref(&event),
            )?;
                return updated
                    .map(Box::new)
                    .map(BackendSessionRecoveryStartOutcome::Started)
                    .ok_or_else(|| format!("Session not found: {session_id}"));
            }
            #[cfg(not(test))]
            unreachable!("production mutation admission rejects a missing SQLite authority");
        }

        let obligation_id = backend_recovery_obligation_id(session_id, recovery_id);
        if let Some(current) = self.canonical_obligation(&obligation_id)? {
            match &current.record {
                crate::domain::local_event::ObligationRecord::BackendSessionRecovery {
                    session_id: stored_session_id,
                    recovery_id: stored_recovery_id,
                    detail:
                        crate::domain::local_event::BackendSessionRecoveryObligationRecord::EffectReserved {
                            ..
                        },
                    state: crate::domain::local_event::ObligationStateRecord::EffectReserved,
                } if stored_session_id == session_id && stored_recovery_id == recovery_id => {
                    return self
                        .get_session_meta(app_data_dir, session_id)?
                        .map(Box::new)
                        .map(BackendSessionRecoveryStartOutcome::Started)
                        .ok_or_else(|| format!("Session not found: {session_id}"));
                }
                crate::domain::local_event::ObligationRecord::BackendSessionRecovery {
                    session_id: stored_session_id,
                    recovery_id: stored_recovery_id,
                    detail:
                        crate::domain::local_event::BackendSessionRecoveryObligationRecord::Completed {
                            ..
                        }
                        | crate::domain::local_event::BackendSessionRecoveryObligationRecord::Failed {
                            ..
                        },
                    ..
                } if stored_session_id == session_id && stored_recovery_id == recovery_id => {}
                _ => {
                    return Err(
                        "backend recovery obligation identity is inconsistent".to_string(),
                    );
                }
            }
            return Err("backend recovery identity was already resolved".to_string());
        }
        let obligation_record =
            crate::domain::local_event::ObligationRecord::BackendSessionRecovery {
            session_id: session_id.to_string(),
            recovery_id: recovery_id.to_string(),
                detail:
                    crate::domain::local_event::BackendSessionRecoveryObligationRecord::EffectReserved {
                        old_provider_session_generation,
                        reason,
                        reserved_at_bits: at.to_bits(),
                    },
                state: crate::domain::local_event::ObligationStateRecord::EffectReserved,
            };
        let obligation = backend_recovery_obligation_mutation(
            obligation_id.clone(),
            obligation_record,
            Some(crate::domain::local_event::PendingIndexEntry {
                ordered_key: format!("{:020}:{obligation_id}", (at * 1000.0).round() as i64),
                owner: session_id.to_string(),
                partition: crate::domain::local_event::PendingPartition::Owner,
                shutdown_plan: None,
            }),
            None,
        )?;
        match self.commit_agent_events_with_additional_mutations(
            session_id,
            std::slice::from_ref(&event),
            vec![obligation],
            None,
            Some(EventProjectionMetaPatch::Started {
                expected_generation: old_provider_session_generation,
                publication_snapshot: Box::new(publication_snapshot),
                at,
            }),
            None,
            crate::domain::local_event::CommitOperationKind::Recovery,
        ) {
            Ok(()) => {}
            Err(error) if error == BACKEND_RECOVERY_START_SUPPRESSED_BY_QUEUE_PAUSE => {
                return Ok(BackendSessionRecoveryStartOutcome::SuppressedByQueuePause);
            }
            Err(error) => return Err(error),
        }
        self.get_session_meta(app_data_dir, session_id)?
            .map(Box::new)
            .map(BackendSessionRecoveryStartOutcome::Started)
            .ok_or_else(|| format!("Session not found: {session_id}"))
    }

    pub fn complete_backend_session_recovery(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        recovery_id: &str,
        old_provider_session_generation: u64,
        backend_session_id: String,
    ) -> Result<SessionMeta, String> {
        self.ensure_canonical_mutation_admission()?;
        let provider_session_generation = next_sqlite_counter(
            old_provider_session_generation,
            "provider session generation",
        )?;
        let at = now_timestamp();
        let pending_recovery_message = PendingRecoveryMessage::Notice {
            recovery_id: recovery_id.to_string(),
            message_id: uuid::Uuid::new_v4().to_string(),
        };
        let events = vec![
            AgentSessionEvent::SessionConfigurationReactivated {
                recovery_id: recovery_id.to_string(),
                provider_session_generation,
                consumed_observation_id: None,
                at,
            },
            AgentSessionEvent::SessionGoalReactivated {
                recovery_id: recovery_id.to_string(),
                outcome: GoalReactivationOutcome::NoCurrentGoal,
                provider_session_generation,
                restoring_turn_id: None,
                consumed_observation_id: None,
                at,
            },
            AgentSessionEvent::BackendSessionRecoveryCompleted {
                recovery_id: recovery_id.to_string(),
                provider_session_generation,
                at,
            },
        ];
        #[cfg(test)]
        if let Some(hook) = self.append_event_hook.read().clone() {
            for event in &events {
                hook(session_id, event)?;
            }
        }
        if !self.canonical_authority_active() {
            #[cfg(test)]
            {
                let mut updated = None;
                self.test_storage()
                    .update_session_meta_and_append_session_events(
                app_data_dir,
                session_id,
                &mut |meta| {
                    if meta.provider_session_generation != old_provider_session_generation {
                        return Err(format!(
                            "Backend session generation changed while completing recovery: expected {old_provider_session_generation}, actual {}",
                            meta.provider_session_generation
                        ));
                    }
                    meta.agent_session_id = Some(backend_session_id.clone());
                    meta.provider_session_generation = provider_session_generation;
                    meta.provider_session_observation_id =
                        Some(backend_recovery_provider_observation_id(recovery_id));
                    meta.context_reinjection_generation = Some(provider_session_generation);
                    meta.pending_recovery_message = Some(pending_recovery_message.clone());
                    meta.recovery_publication_snapshot = None;
                    meta.updated_at = at;
                    updated = Some(meta.clone());
                    Ok(())
                },
                &events,
            )?;
                return updated.ok_or_else(|| format!("Session not found: {session_id}"));
            }
            #[cfg(not(test))]
            unreachable!("production mutation admission rejects a missing SQLite authority");
        }

        let obligation_id = backend_recovery_obligation_id(session_id, recovery_id);
        let current = self
            .canonical_obligation(&obligation_id)?
            .ok_or_else(|| "backend recovery completion has no durable reservation".to_string())?;
        match &current.record {
            crate::domain::local_event::ObligationRecord::BackendSessionRecovery {
                session_id: stored_session_id,
                recovery_id: stored_recovery_id,
                detail:
                    crate::domain::local_event::BackendSessionRecoveryObligationRecord::Completed {
                        ..
                    },
                state: crate::domain::local_event::ObligationStateRecord::Completed,
            } if stored_session_id == session_id && stored_recovery_id == recovery_id => {
                return self
                    .get_session_meta(app_data_dir, session_id)?
                    .ok_or_else(|| format!("Session not found: {session_id}"));
            }
            crate::domain::local_event::ObligationRecord::BackendSessionRecovery {
                session_id: stored_session_id,
                recovery_id: stored_recovery_id,
                detail:
                    crate::domain::local_event::BackendSessionRecoveryObligationRecord::EffectReserved {
                        old_provider_session_generation: stored_generation,
                        ..
                    },
                state: crate::domain::local_event::ObligationStateRecord::EffectReserved,
            } if stored_session_id == session_id
                && stored_recovery_id == recovery_id
                && *stored_generation == old_provider_session_generation => {}
            crate::domain::local_event::ObligationRecord::BackendSessionRecovery {
                session_id: stored_session_id,
                recovery_id: stored_recovery_id,
                detail:
                    crate::domain::local_event::BackendSessionRecoveryObligationRecord::Failed {
                        ..
                    },
                ..
            } if stored_session_id == session_id && stored_recovery_id == recovery_id => {
                return Err("backend recovery reservation is not pending".to_string());
            }
            _ => {
                return Err("backend recovery obligation identity is inconsistent".to_string());
            }
        }
        let obligation_record =
            crate::domain::local_event::ObligationRecord::BackendSessionRecovery {
                session_id: session_id.to_string(),
                recovery_id: recovery_id.to_string(),
                detail:
                    crate::domain::local_event::BackendSessionRecoveryObligationRecord::Completed {
                        old_provider_session_generation,
                        provider_session_generation,
                        backend_session_id: backend_session_id.clone(),
                        completed_at_bits: at.to_bits(),
                    },
                state: crate::domain::local_event::ObligationStateRecord::Completed,
            };
        let obligation = backend_recovery_obligation_mutation(
            obligation_id.clone(),
            obligation_record,
            None,
            Some(&current),
        )?;
        let (publication_recovery_id, publication_message_id) =
            pending_recovery_message_identity(&pending_recovery_message);
        let publication_obligation_id = recovery_publication_obligation_id(
            session_id,
            publication_recovery_id,
            publication_message_id,
        );
        let publication = recovery_publication_obligation_mutation(
            publication_obligation_id.clone(),
            crate::domain::local_event::ObligationRecord::RecoveryPublication {
                session_id: session_id.to_string(),
                recovery_id: publication_recovery_id.to_string(),
                message_id: publication_message_id.to_string(),
                source_obligation_id: obligation_id,
                detail: crate::domain::local_event::RecoveryPublicationObligationRecord::Pending {
                    pending_message: recovery_publication_message_record(&pending_recovery_message),
                },
                state: crate::domain::local_event::ObligationStateRecord::Pending,
            },
            Some(crate::domain::local_event::PendingIndexEntry {
                ordered_key: format!(
                    "{:020}:{publication_obligation_id}",
                    (at * 1000.0).round() as i64
                ),
                owner: session_id.to_string(),
                partition: crate::domain::local_event::PendingPartition::Owner,
                shutdown_plan: None,
            }),
            None,
        )?;
        self.commit_agent_events_with_additional_mutations(
            session_id,
            &events,
            vec![obligation, publication],
            None,
            Some(EventProjectionMetaPatch::Completed {
                expected_generation: old_provider_session_generation,
                provider_session_generation,
                backend_session_id,
                pending_recovery_message,
                at,
            }),
            None,
            crate::domain::local_event::CommitOperationKind::Recovery,
        )?;
        self.get_session_meta(app_data_dir, session_id)?
            .ok_or_else(|| format!("Session not found: {session_id}"))
    }

    pub fn fail_backend_session_recovery(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        recovery_id: &str,
        error: &str,
    ) -> Result<SessionMeta, String> {
        self.ensure_canonical_mutation_admission()?;
        let at = now_timestamp();
        let fallback_message_id = || {
            let digest = Sha256::digest(
                format!("backend-recovery-failure-v1\0{session_id}\0{recovery_id}").as_bytes(),
            );
            format!("backend-recovery-failure-{}", hex::encode(digest))
        };
        let message_id = if self.canonical_authority_active() {
            let current_projection = self
                .canonical_session_projection(session_id)?
                .ok_or_else(|| format!("Session projection not found: {session_id}"))?;
            TurnEventLog::from_events(current_projection.reducer_events)
                .project()
                .messages
                .into_iter()
                .rev()
                .find(|message| message.role == super::MessageRole::Agent)
                .map(|message| message.id)
                .unwrap_or_else(fallback_message_id)
        } else {
            #[cfg(test)]
            {
                self.load_full_session_for_restore(app_data_dir, session_id)?
                    .and_then(|session| {
                        session
                            .messages
                            .into_iter()
                            .rev()
                            .find(|message| message.role == super::MessageRole::Agent)
                            .map(|message| message.id)
                    })
                    .unwrap_or_else(fallback_message_id)
            }
            #[cfg(not(test))]
            unreachable!("production mutation admission rejects a missing SQLite authority");
        };
        let pending_recovery_message = PendingRecoveryMessage::Error {
            recovery_id: recovery_id.to_string(),
            message_id,
            error: error.to_string(),
        };
        let event = AgentSessionEvent::BackendSessionRecoveryFailed {
            recovery_id: recovery_id.to_string(),
            error: error.to_string(),
            at,
        };
        #[cfg(test)]
        if let Some(hook) = self.append_event_hook.read().clone() {
            hook(session_id, &event)?;
        }
        if !self.canonical_authority_active() {
            #[cfg(test)]
            {
                let mut updated = None;
                self.test_storage()
                    .update_session_meta_and_append_session_events(
                        app_data_dir,
                        session_id,
                        &mut |meta| {
                            meta.state = SessionState::Error;
                            meta.error_reason = Some(error.to_string());
                            meta.pending_recovery_message = Some(pending_recovery_message.clone());
                            meta.recovery_publication_snapshot = None;
                            meta.updated_at = at;
                            updated = Some(meta.clone());
                            Ok(())
                        },
                        std::slice::from_ref(&event),
                    )?;
                return updated.ok_or_else(|| format!("Session not found: {session_id}"));
            }
            #[cfg(not(test))]
            unreachable!("production mutation admission rejects a missing SQLite authority");
        }

        let obligation_id = backend_recovery_obligation_id(session_id, recovery_id);
        let current = self
            .canonical_obligation(&obligation_id)?
            .ok_or_else(|| "backend recovery failure has no durable reservation".to_string())?;
        match &current.record {
            crate::domain::local_event::ObligationRecord::BackendSessionRecovery {
                session_id: stored_session_id,
                recovery_id: stored_recovery_id,
                detail:
                    crate::domain::local_event::BackendSessionRecoveryObligationRecord::Failed {
                        ..
                    },
                state: crate::domain::local_event::ObligationStateRecord::Failed,
            } if stored_session_id == session_id && stored_recovery_id == recovery_id => {
                return self
                    .get_session_meta(app_data_dir, session_id)?
                    .ok_or_else(|| format!("Session not found: {session_id}"));
            }
            crate::domain::local_event::ObligationRecord::BackendSessionRecovery {
                session_id: stored_session_id,
                recovery_id: stored_recovery_id,
                detail:
                    crate::domain::local_event::BackendSessionRecoveryObligationRecord::EffectReserved {
                        ..
                    },
                state: crate::domain::local_event::ObligationStateRecord::EffectReserved,
            } if stored_session_id == session_id && stored_recovery_id == recovery_id => {}
            crate::domain::local_event::ObligationRecord::BackendSessionRecovery {
                session_id: stored_session_id,
                recovery_id: stored_recovery_id,
                detail:
                    crate::domain::local_event::BackendSessionRecoveryObligationRecord::Completed {
                        ..
                    },
                ..
            } if stored_session_id == session_id && stored_recovery_id == recovery_id => {
                return Err("backend recovery reservation is not pending".to_string());
            }
            _ => {
                return Err("backend recovery obligation identity is inconsistent".to_string());
            }
        }
        let error_digest: [u8; 32] = Sha256::digest(error.as_bytes()).into();
        let obligation_record =
            crate::domain::local_event::ObligationRecord::BackendSessionRecovery {
                session_id: session_id.to_string(),
                recovery_id: recovery_id.to_string(),
                detail:
                    crate::domain::local_event::BackendSessionRecoveryObligationRecord::Failed {
                        error_sha256: error_digest,
                        failed_at_bits: at.to_bits(),
                    },
                state: crate::domain::local_event::ObligationStateRecord::Failed,
            };
        let obligation = backend_recovery_obligation_mutation(
            obligation_id.clone(),
            obligation_record,
            None,
            Some(&current),
        )?;
        let (publication_recovery_id, publication_message_id) =
            pending_recovery_message_identity(&pending_recovery_message);
        let publication_obligation_id = recovery_publication_obligation_id(
            session_id,
            publication_recovery_id,
            publication_message_id,
        );
        let publication = recovery_publication_obligation_mutation(
            publication_obligation_id.clone(),
            crate::domain::local_event::ObligationRecord::RecoveryPublication {
                session_id: session_id.to_string(),
                recovery_id: publication_recovery_id.to_string(),
                message_id: publication_message_id.to_string(),
                source_obligation_id: obligation_id,
                detail: crate::domain::local_event::RecoveryPublicationObligationRecord::Pending {
                    pending_message: recovery_publication_message_record(&pending_recovery_message),
                },
                state: crate::domain::local_event::ObligationStateRecord::Pending,
            },
            Some(crate::domain::local_event::PendingIndexEntry {
                ordered_key: format!(
                    "{:020}:{publication_obligation_id}",
                    (at * 1000.0).round() as i64
                ),
                owner: session_id.to_string(),
                partition: crate::domain::local_event::PendingPartition::Owner,
                shutdown_plan: None,
            }),
            None,
        )?;
        self.commit_agent_events_with_additional_mutations(
            session_id,
            std::slice::from_ref(&event),
            vec![obligation, publication],
            None,
            Some(EventProjectionMetaPatch::Failed {
                pending_recovery_message,
                at,
            }),
            None,
            crate::domain::local_event::CommitOperationKind::Recovery,
        )?;
        self.get_session_meta(app_data_dir, session_id)?
            .ok_or_else(|| format!("Session not found: {session_id}"))
    }

    #[cfg(test)]
    pub(crate) fn clear_pending_recovery_message(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        delivered: &PendingRecoveryMessage,
    ) -> Result<(), String> {
        self.update_meta_only(app_data_dir, session_id, |meta| {
            if meta.pending_recovery_message.as_ref() == Some(delivered) {
                meta.pending_recovery_message = None;
            }
            Ok(())
        })?;
        Ok(())
    }

    pub(crate) fn publish_pending_recovery_message(
        &self,
        _app_data_dir: &Path,
        session_id: &str,
        pending: &PendingRecoveryMessage,
        message: ChatMessage,
    ) -> Result<bool, String> {
        self.ensure_canonical_mutation_admission()?;
        if !self.canonical_authority_active() {
            #[cfg(test)]
            {
                let exists = self
                    .load_full_session_for_restore(_app_data_dir, session_id)?
                    .is_some_and(|session| {
                        session
                            .messages
                            .iter()
                            .any(|stored| stored.id == message.id)
                    });
                if exists {
                    self.persist_message_parts(
                        _app_data_dir,
                        session_id,
                        &message.id,
                        message.parts.as_deref().unwrap_or_default(),
                        message.streaming_final_seq,
                        Some(message.timestamp),
                    )?;
                } else {
                    self.append_message(_app_data_dir, session_id, &message)?;
                }
                self.clear_pending_recovery_message(_app_data_dir, session_id, pending)?;
                return Ok(!exists);
            }
            #[cfg(not(test))]
            unreachable!("production mutation admission rejects a missing SQLite authority");
        }

        let mut projection = self
            .canonical_session_projection(session_id)?
            .ok_or_else(|| format!("Session projection not found: {session_id}"))?;
        let existing = self.canonical_message_projection(session_id, &message.id)?;
        let (recovery_id, message_id) = pending_recovery_message_identity(pending);
        if message.id != message_id {
            return Err("recovery publication message identity changed".to_string());
        }
        let source_obligation_id = backend_recovery_obligation_id(session_id, recovery_id);
        let publication_obligation_id =
            recovery_publication_obligation_id(session_id, recovery_id, message_id);
        let current_publication = self.canonical_obligation(&publication_obligation_id)?;
        let publication_completed = current_publication
            .as_ref()
            .map(|current| match &current.record {
                crate::domain::local_event::ObligationRecord::RecoveryPublication {
                    session_id: stored_session_id,
                    recovery_id: stored_recovery_id,
                    message_id: stored_message_id,
                    source_obligation_id: stored_source,
                    detail:
                        crate::domain::local_event::RecoveryPublicationObligationRecord::Pending {
                            pending_message,
                        },
                    state: crate::domain::local_event::ObligationStateRecord::Pending,
                } if stored_session_id == session_id
                    && stored_recovery_id == recovery_id
                    && stored_message_id == message_id
                    && stored_source == &source_obligation_id
                    && recovery_publication_message_matches(pending_message, pending) =>
                {
                    Ok(false)
                }
                crate::domain::local_event::ObligationRecord::RecoveryPublication {
                    session_id: stored_session_id,
                    recovery_id: stored_recovery_id,
                    message_id: stored_message_id,
                    source_obligation_id: stored_source,
                    detail:
                        crate::domain::local_event::RecoveryPublicationObligationRecord::Completed {
                            ..
                        },
                    state: crate::domain::local_event::ObligationStateRecord::Completed,
                } if stored_session_id == session_id
                    && stored_recovery_id == recovery_id
                    && stored_message_id == message_id
                    && stored_source == &source_obligation_id =>
                {
                    Ok(true)
                }
                _ => Err("recovery publication obligation identity is inconsistent".to_string()),
            })
            .transpose()?;
        match projection.meta.pending_recovery_message.as_ref() {
            Some(current) if current == pending && publication_completed != Some(true) => {}
            None if existing.is_some() && publication_completed != Some(false) => {
                return Ok(false);
            }
            Some(_) => {
                return Err("pending backend recovery publication identity changed".to_string());
            }
            None => return Err("backend recovery publication is no longer pending".to_string()),
        }
        let inserted = existing.is_none();
        if inserted {
            projection.meta.message_count =
                add_sqlite_count(projection.meta.message_count, 1, "session message count")?;
            if projection.meta.first_message_preview.is_empty() {
                projection.meta.first_message_preview =
                    super::first_message_preview(std::slice::from_ref(&message));
            }
        }
        projection.meta.pending_recovery_message = None;
        projection.meta.updated_at = message.timestamp;
        projection.messages = vec![message];
        let completed_publication = recovery_publication_obligation_mutation(
            publication_obligation_id,
            crate::domain::local_event::ObligationRecord::RecoveryPublication {
                session_id: session_id.to_string(),
                recovery_id: recovery_id.to_string(),
                message_id: message_id.to_string(),
                source_obligation_id,
                detail:
                    crate::domain::local_event::RecoveryPublicationObligationRecord::Completed {
                        published_at_bits: projection.meta.updated_at.to_bits(),
                    },
                state: crate::domain::local_event::ObligationStateRecord::Completed,
            },
            None,
            current_publication.as_ref(),
        )?;
        self.commit_session_projection_snapshot_with_kind_and_mutations(
            projection,
            crate::domain::local_event::CommitOperationKind::Projection,
            vec![completed_publication],
        )?;
        Ok(inserted)
    }

    pub fn record_backend_session_established(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        expected_provider_session_generation: u64,
        observation_id: &str,
        backend_session_id: String,
        context_carry: Option<ContextCarryState>,
    ) -> Result<ProviderSessionEstablishmentOutcome, String> {
        if observation_id.is_empty() {
            return Err("provider session observation identity is empty".to_string());
        }
        #[cfg(test)]
        if let Some(hook) = self.backend_established_hook.read().clone() {
            hook(session_id, &backend_session_id)?;
        }
        let mut fenced = false;
        let updated = self.update_meta_if_changed(app_data_dir, session_id, |meta| {
            if meta.provider_session_observation_id.as_deref() == Some(observation_id) {
                if meta.agent_session_id.as_deref() != Some(backend_session_id.as_str()) {
                    return Err(
                        "provider session observation identity has conflicting backend identity"
                            .to_string(),
                    );
                }
                return Ok(false);
            }
            if meta.recovery_publication_snapshot.is_some()
                || matches!(
                    meta.pending_recovery_message.as_ref(),
                    Some(PendingRecoveryMessage::Error { .. })
                )
                || matches!(
                    &meta.state,
                    SessionState::Error | SessionState::Closed | SessionState::Archived
                )
            {
                fenced = true;
                return Ok(false);
            }
            if meta.provider_session_generation != expected_provider_session_generation {
                return Err(format!(
                    "Provider session generation changed while recording establishment: expected {expected_provider_session_generation}, actual {}",
                    meta.provider_session_generation
                ));
            }
            let provider_session_generation = next_sqlite_counter(
                expected_provider_session_generation,
                "provider session generation",
            )?;
            let reinjection_pending =
                meta.context_reinjection_generation == Some(meta.provider_session_generation);
            meta.agent_session_id = Some(backend_session_id.clone());
            meta.provider_session_generation = provider_session_generation;
            meta.provider_session_observation_id = Some(observation_id.to_string());
            if let Some(context_carry) = context_carry.clone() {
                meta.context_carry = Some(context_carry);
                meta.context_reinjection_generation = None;
            } else if reinjection_pending {
                meta.context_reinjection_generation = Some(meta.provider_session_generation);
            }
            meta.updated_at = now_timestamp();
            Ok(true)
        })?;
        if let Some(meta) = updated {
            return Ok(ProviderSessionEstablishmentOutcome::Settled(Box::new(meta)));
        }
        if fenced {
            return Ok(ProviderSessionEstablishmentOutcome::Fenced);
        }
        let Some(meta) = self.get_session_meta(app_data_dir, session_id)? else {
            return Ok(ProviderSessionEstablishmentOutcome::Missing);
        };
        if meta.provider_session_observation_id.as_deref() != Some(observation_id)
            || meta.agent_session_id.as_deref() != Some(backend_session_id.as_str())
        {
            return Err(
                "provider session observation replay no longer owns the durable generation"
                    .to_string(),
            );
        }
        Ok(ProviderSessionEstablishmentOutcome::Settled(Box::new(meta)))
    }

    #[cfg(test)]
    pub fn append_session_events(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        events: &[AgentSessionEvent],
    ) -> Result<(), String> {
        #[cfg(test)]
        if let Some(hook) = self.append_event_hook.read().clone() {
            for event in events {
                hook(session_id, event)?;
            }
        }
        self.commit_agent_events(app_data_dir, session_id, events)?;
        #[cfg(test)]
        if !self.canonical_authority_active() {
            self.test_storage()
                .append_session_events(app_data_dir, session_id, events)?;
            if self.test_storage().take_event_log_recovered(session_id) {
                self.notify_event_log_recovered(session_id);
            }
        }
        Ok(())
    }

    pub(crate) fn append_session_events_from_user(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        events: &[AgentSessionEvent],
    ) -> Result<(), String> {
        #[cfg(test)]
        if let Some(hook) = self.append_event_hook.read().clone() {
            for event in events {
                hook(session_id, event)?;
            }
        }
        self.commit_agent_events_with_kind(
            app_data_dir,
            session_id,
            events,
            crate::domain::local_event::CommitOperationKind::UserMutation,
        )?;
        #[cfg(test)]
        if !self.canonical_authority_active() {
            self.test_storage()
                .append_session_events(app_data_dir, session_id, events)?;
            if self.test_storage().take_event_log_recovered(session_id) {
                self.notify_event_log_recovered(session_id);
            }
        }
        Ok(())
    }

    pub fn load_previous_human_message_before_agent(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        agent_message_id: &str,
    ) -> Result<Option<ChatMessage>, String> {
        #[cfg(test)]
        if !self.canonical_authority_active() {
            return self
                .test_storage()
                .load_previous_human_message_before_agent(
                    app_data_dir,
                    session_id,
                    agent_message_id,
                );
        }
        let Some(session) = self.load_full_session_for_restore(app_data_dir, session_id)? else {
            return Ok(None);
        };
        let Some(agent_index) = session
            .messages
            .iter()
            .position(|message| message.id == agent_message_id)
        else {
            return Ok(None);
        };
        Ok(session.messages[..agent_index]
            .iter()
            .rev()
            .find(|message| message.role == super::MessageRole::Human)
            .cloned())
    }

    /// workflow step session のセットアップ失敗時に、作成済みの子 session を
    /// 取り除くロールバック経路。storage 層へ削除を委譲する。
    pub(crate) fn remove_session_for_rollback(
        &self,
        _app_data_dir: &Path,
        session_id: &str,
    ) -> Result<(), String> {
        self.ensure_canonical_mutation_admission()?;
        self.remove_canonical_session_projection(session_id)?;
        #[cfg(test)]
        if !self.canonical_authority_active() {
            self.test_storage()
                .remove_session(_app_data_dir, session_id);
        }
        Ok(())
    }

    #[cfg(test)]
    pub fn list_worktree_sessions(
        &self,
        app_data_dir: &Path,
        worktree_path: &str,
    ) -> Result<Vec<ChatSession>, String> {
        Ok(self
            .list_metas_for_active_read_authority(app_data_dir)?
            .into_iter()
            .filter(|session| same_worktree_path(&session.worktree_path, worktree_path))
            .map(|meta| meta.to_session(Vec::new()))
            .collect())
    }

    pub fn list_worktree_sessions_full(
        &self,
        app_data_dir: &Path,
        worktree_path: &str,
    ) -> Result<Vec<ChatSession>, String> {
        let ids = self
            .list_metas_for_active_read_authority(app_data_dir)?
            .into_iter()
            .filter(|session| same_worktree_path(&session.worktree_path, worktree_path))
            .map(|session| session.id)
            .collect::<Vec<_>>();
        ids.into_iter()
            .filter_map(|id| {
                self.load_full_session_for_restore(app_data_dir, &id)
                    .transpose()
            })
            .collect()
    }

    /// Full-session replacement for cold paths that own a complete `ChatSession`.
    ///
    /// Do not pass shell/page sessions returned by `get_session_shell` or `get_session_page`.
    /// Normal runtime updates must use `append_message`, `persist_message_parts`, or meta-only
    /// update methods so page-external message chunks cannot be removed by partial input.
    pub fn save_full_session_for_restore(
        &self,
        app_data_dir: &Path,
        session: &ChatSession,
    ) -> Result<(), String> {
        self.save_full_session_with_kind(
            app_data_dir,
            session,
            crate::domain::local_event::CommitOperationKind::Projection,
        )
    }

    pub(crate) fn save_full_session_from_user(
        &self,
        app_data_dir: &Path,
        session: &ChatSession,
    ) -> Result<(), String> {
        self.save_full_session_with_kind(
            app_data_dir,
            session,
            crate::domain::local_event::CommitOperationKind::UserMutation,
        )
    }

    fn save_full_session_with_kind(
        &self,
        app_data_dir: &Path,
        session: &ChatSession,
        operation_kind: crate::domain::local_event::CommitOperationKind,
    ) -> Result<(), String> {
        self.ensure_canonical_mutation_admission()?;
        let permission_mode =
            crate::domain::agent_session::PermissionMode::parse(&session.permission_mode)
                .map_err(|e| e.to_string())?;
        let normalized_session;
        let session = if session.permission_mode == permission_mode.as_str() {
            session
        } else {
            normalized_session = {
                let mut session = session.clone();
                session.permission_mode = permission_mode.as_str().to_string();
                session
            };
            &normalized_session
        };

        #[cfg(test)]
        if let Some(hook) = self.save_hook.read().clone() {
            hook(session)?;
        }

        let previous_projection = self.canonical_session_projection(&session.id)?;
        let previous_state = previous_projection
            .as_ref()
            .map(|projection| projection.meta.state.clone());
        let previous_title = previous_projection
            .as_ref()
            .and_then(|projection| projection.title.clone());
        let previous_queue_paused_at = previous_projection
            .as_ref()
            .and_then(|projection| projection.queue_paused_at);
        let reducer_events = previous_projection
            .as_ref()
            .map(|projection| projection.reducer_events.clone())
            .unwrap_or_default();
        let pending_send_queue = previous_projection
            .map(|projection| projection.pending_send_queue)
            .unwrap_or_default();
        let saved_meta = SessionMeta::from_session(session);
        self.commit_session_projection_snapshot_with_kind(
            CanonicalAgentSessionProjection {
                meta: saved_meta,
                title: previous_title,
                messages: session.messages.clone(),
                reducer_events,
                queue_paused_at: previous_queue_paused_at,
                latest_token_usage: None,
                pending_send_queue,
            },
            operation_kind,
        )?;
        #[cfg(test)]
        if !self.canonical_authority_active() {
            self.test_storage()
                .save_full_session_for_restore(app_data_dir, session)?;
        }
        if previous_state.as_ref() != Some(&session.state) {
            let revision = self.require_meta(app_data_dir, &session.id)?.state_revision;
            self.notify_state_change(
                &session.id,
                &session.worktree_path,
                &session.state,
                revision,
            );
        }
        Ok(())
    }

    /// `SessionState` または Error 理由 projection の変更を購読するリスナーを登録する。
    /// Error 理由だけが変わる場合は同じ `SessionState` で再通知される。
    /// 登録順に保存後に発火される。AgentStatusCenter のような中央管理が
    /// SessionStore からの状態変更を一方向に受け取るための入口。
    pub fn register_state_change_listener(&self, listener: SessionStateChangeListener) {
        self.state_change_listeners.write().push(listener);
    }

    pub fn register_event_log_recovery_listener(&self, listener: SessionEventLogRecoveryListener) {
        self.event_log_recovery_listeners.write().push(listener);
    }

    fn notify_state_change(
        &self,
        session_id: &str,
        worktree_path: &str,
        new_state: &SessionState,
        state_revision: u64,
    ) {
        let listeners = self.state_change_listeners.read().clone();
        for listener in listeners {
            listener(session_id, worktree_path, new_state, state_revision);
        }
    }

    #[cfg(test)]
    fn notify_event_log_recovered(&self, session_id: &str) {
        let listeners = self.event_log_recovery_listeners.read().clone();
        for listener in listeners {
            listener(session_id);
        }
    }

    fn require_meta(&self, app_data_dir: &Path, session_id: &str) -> Result<SessionMeta, String> {
        self.get_session_meta(app_data_dir, session_id)?
            .ok_or_else(|| format!("Session not found: {session_id}"))
    }

    fn update_meta_only(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        update: impl FnOnce(&mut SessionMeta) -> Result<(), String>,
    ) -> Result<(SessionMeta, bool), String> {
        self.update_meta_only_with_kind(
            app_data_dir,
            session_id,
            crate::domain::local_event::CommitOperationKind::Projection,
            update,
        )
    }

    fn update_meta_only_with_kind(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        operation_kind: crate::domain::local_event::CommitOperationKind,
        update: impl FnOnce(&mut SessionMeta) -> Result<(), String>,
    ) -> Result<(SessionMeta, bool), String> {
        self.ensure_canonical_mutation_admission()?;
        if self.event_authority.read().is_none() {
            #[cfg(test)]
            {
                let mut update = Some(update);
                let mut state_changed = false;
                let meta = self.test_storage().update_session_meta(
                    app_data_dir,
                    session_id,
                    &mut |meta| {
                        let previous_state = meta.state.clone();
                        update.take().expect("legacy meta update runs once")(meta)?;
                        meta.state_revision =
                            next_sqlite_counter(meta.state_revision, "session state revision")?;
                        state_changed = previous_state != meta.state;
                        Ok(())
                    },
                )?;
                return Ok((meta, state_changed));
            }
            #[cfg(not(test))]
            unreachable!("production mutation admission rejects a missing SQLite authority");
        }
        let mut meta = self
            .get_session_meta(app_data_dir, session_id)?
            .ok_or_else(|| format!("Session not found: {session_id}"))?;
        let previous_state = meta.state.clone();
        update(&mut meta)?;
        meta.state_revision = next_sqlite_counter(meta.state_revision, "session state revision")?;
        let state_changed = previous_state != meta.state;
        if operation_kind == crate::domain::local_event::CommitOperationKind::UserMutation {
            self.commit_user_meta_projection_snapshot(meta.clone())?;
        } else {
            self.commit_meta_projection_snapshot_with_kind(meta.clone(), operation_kind)?;
        }
        Ok((meta, state_changed))
    }

    fn update_meta_if_changed(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        update: impl FnOnce(&mut SessionMeta) -> Result<bool, String>,
    ) -> Result<Option<SessionMeta>, String> {
        self.ensure_canonical_mutation_admission()?;
        if self.event_authority.read().is_none() {
            #[cfg(test)]
            {
                let mut update = Some(update);
                let mut changed = false;
                let meta = self.test_storage().update_session_meta(
                    app_data_dir,
                    session_id,
                    &mut |meta| {
                        changed = update.take().expect("legacy meta update runs once")(meta)?;
                        Ok(())
                    },
                )?;
                return Ok(changed.then_some(meta));
            }
            #[cfg(not(test))]
            unreachable!("production mutation admission rejects a missing SQLite authority");
        }
        let mut meta = self
            .get_session_meta(app_data_dir, session_id)?
            .ok_or_else(|| format!("Session not found: {session_id}"))?;
        if !update(&mut meta)? {
            return Ok(None);
        }
        self.commit_meta_projection_snapshot(meta.clone())?;
        Ok(Some(meta))
    }

    #[cfg(test)]
    pub fn set_session_state(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        state: SessionState,
    ) -> Result<(), String> {
        self.set_session_state_with_kind(
            app_data_dir,
            session_id,
            state,
            crate::domain::local_event::CommitOperationKind::Projection,
        )
    }

    pub(crate) fn set_session_state_from_user(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        state: SessionState,
    ) -> Result<(), String> {
        self.set_session_state_with_kind(
            app_data_dir,
            session_id,
            state,
            crate::domain::local_event::CommitOperationKind::UserMutation,
        )
    }

    fn set_session_state_with_kind(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        state: SessionState,
        operation_kind: crate::domain::local_event::CommitOperationKind,
    ) -> Result<(), String> {
        #[cfg(test)]
        if let Some(hook) = self.set_state_hook.read().clone() {
            hook(session_id, &state)?;
        }
        let state_for_notify = state.clone();
        let (meta, state_changed) =
            self.update_meta_only_with_kind(app_data_dir, session_id, operation_kind, |meta| {
                if state != SessionState::Error {
                    meta.error_reason = None;
                }
                meta.state = state;
                meta.updated_at = now_timestamp();
                Ok(())
            })?;
        if state_changed {
            self.notify_state_change(
                session_id,
                &meta.worktree_path,
                &state_for_notify,
                meta.state_revision,
            );
        }
        Ok(())
    }

    #[cfg(test)]
    fn set_event_projection(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        state: SessionState,
        error_reason: Option<String>,
        last_turn_interruption: Option<TurnInterruption>,
        last_turn_id: Option<u64>,
    ) -> Result<(), String> {
        #[cfg(test)]
        if let Some(hook) = self.projection_hook.read().clone() {
            hook(session_id, &state, error_reason.as_deref())?;
        }
        #[cfg(test)]
        if let Some(hook) = self.event_projection_hook.read().clone() {
            hook(session_id, last_turn_id)?;
        }
        let state_for_notify = state.clone();
        let projected_error_reason = error_reason_for_state(&state, &error_reason);
        let mut previous_error_reason = None;
        let (meta, state_changed) = self.update_meta_only(app_data_dir, session_id, |meta| {
            previous_error_reason = Some(meta.error_reason.clone());
            meta.state = state;
            meta.error_reason = projected_error_reason.clone();
            meta.last_turn_interruption = last_turn_interruption;
            meta.last_turn_id = last_turn_id;
            meta.updated_at = now_timestamp();
            Ok(())
        })?;
        if state_changed
            || previous_error_reason
                .expect("update_session_meta must invoke closure before returning Ok")
                != projected_error_reason
        {
            self.notify_state_change(
                session_id,
                &meta.worktree_path,
                &state_for_notify,
                meta.state_revision,
            );
        }
        Ok(())
    }

    #[cfg(test)]
    pub fn update_permission_mode(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        permission_mode: &str,
    ) -> Result<(), String> {
        self.update_permission_mode_with_kind(
            app_data_dir,
            session_id,
            permission_mode,
            crate::domain::local_event::CommitOperationKind::Projection,
        )
    }

    pub(crate) fn update_permission_mode_from_user(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        permission_mode: &str,
    ) -> Result<(), String> {
        self.update_permission_mode_with_kind(
            app_data_dir,
            session_id,
            permission_mode,
            crate::domain::local_event::CommitOperationKind::UserMutation,
        )
    }

    fn update_permission_mode_with_kind(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        permission_mode: &str,
        operation_kind: crate::domain::local_event::CommitOperationKind,
    ) -> Result<(), String> {
        let permission_mode = crate::domain::agent_session::PermissionMode::parse(permission_mode)
            .map_err(|e| e.to_string())?;
        self.update_meta_only_with_kind(app_data_dir, session_id, operation_kind, |meta| {
            meta.permission_mode = permission_mode.as_str().to_string();
            Ok(())
        })?;
        Ok(())
    }

    #[cfg(test)]
    pub fn update_plan_mode(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        plan_mode: bool,
    ) -> Result<(), String> {
        self.update_plan_mode_with_kind(
            app_data_dir,
            session_id,
            plan_mode,
            crate::domain::local_event::CommitOperationKind::Projection,
        )
    }

    pub(crate) fn update_plan_mode_from_user(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        plan_mode: bool,
    ) -> Result<(), String> {
        self.update_plan_mode_with_kind(
            app_data_dir,
            session_id,
            plan_mode,
            crate::domain::local_event::CommitOperationKind::UserMutation,
        )
    }

    fn update_plan_mode_with_kind(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        plan_mode: bool,
        operation_kind: crate::domain::local_event::CommitOperationKind,
    ) -> Result<(), String> {
        self.update_meta_only_with_kind(app_data_dir, session_id, operation_kind, |meta| {
            meta.plan_mode = plan_mode;
            Ok(())
        })?;
        Ok(())
    }

    #[cfg(test)]
    pub fn update_backend_selection(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        backend_id: String,
        selected_model: Option<String>,
    ) -> Result<(), String> {
        self.update_backend_selection_with_kind(
            app_data_dir,
            session_id,
            backend_id,
            selected_model,
            crate::domain::local_event::CommitOperationKind::Projection,
        )
    }

    pub(crate) fn update_backend_selection_from_user(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        backend_id: String,
        selected_model: Option<String>,
    ) -> Result<(), String> {
        self.update_backend_selection_with_kind(
            app_data_dir,
            session_id,
            backend_id,
            selected_model,
            crate::domain::local_event::CommitOperationKind::UserMutation,
        )
    }

    fn update_backend_selection_with_kind(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        backend_id: String,
        selected_model: Option<String>,
        operation_kind: crate::domain::local_event::CommitOperationKind,
    ) -> Result<(), String> {
        self.update_meta_only_with_kind(app_data_dir, session_id, operation_kind, |meta| {
            meta.backend_id = backend_id;
            meta.selected_model = selected_model;
            meta.updated_at = now_timestamp();
            Ok(())
        })?;
        Ok(())
    }

    #[allow(dead_code)] // issues-1301 G-1: retained for permission profile settings surface; current runtime only reads the stored profile id.
    pub fn update_permission_profile_id(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        permission_profile_id: Option<&str>,
    ) -> Result<(), String> {
        let profile_id = permission_profile_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| {
                if value.chars().any(char::is_control) {
                    Err("Permission profile id cannot contain control characters".to_string())
                } else {
                    Ok(value.to_string())
                }
            })
            .transpose()?;
        self.update_meta_only(app_data_dir, session_id, |meta| {
            meta.permission_profile_id = profile_id;
            Ok(())
        })?;
        Ok(())
    }

    #[cfg(test)]
    pub fn update_agent_session_id(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        agent_session_id: Option<String>,
    ) -> Result<(), String> {
        self.update_meta_only(app_data_dir, session_id, |meta| {
            meta.agent_session_id = agent_session_id;
            meta.updated_at = now_timestamp();
            Ok(())
        })?;
        Ok(())
    }

    #[cfg(test)]
    pub fn update_agent_session_id_if_changed(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        agent_session_id: Option<String>,
    ) -> Result<Option<SessionMeta>, String> {
        self.update_meta_if_changed(app_data_dir, session_id, |meta| {
            if meta.agent_session_id == agent_session_id {
                return Ok(false);
            }
            meta.agent_session_id = agent_session_id;
            meta.updated_at = now_timestamp();
            Ok(true)
        })
    }

    #[cfg(test)]
    pub fn update_context_carry_if_changed(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        context_carry: Option<ContextCarryState>,
    ) -> Result<Option<SessionMeta>, String> {
        self.update_meta_if_changed(app_data_dir, session_id, |meta| {
            if meta.context_carry == context_carry {
                return Ok(false);
            }
            meta.context_carry = context_carry;
            meta.updated_at = now_timestamp();
            Ok(true)
        })
    }

    pub fn complete_context_reinjection_if_required(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        expected_provider_session_generation: u64,
        reinjected: bool,
    ) -> Result<Option<SessionMeta>, String> {
        self.complete_context_restore_after_start_if_current(
            app_data_dir,
            session_id,
            ContextRestoreCompletionRequest {
                expected_provider_session_generation,
                expected_turn_id: None,
                reinjected,
                clear_context_carry: false,
                recovery_restore_required: true,
            },
        )
    }

    pub fn complete_context_restore_after_start_if_current(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        request: ContextRestoreCompletionRequest,
    ) -> Result<Option<SessionMeta>, String> {
        let ContextRestoreCompletionRequest {
            expected_provider_session_generation,
            expected_turn_id,
            reinjected,
            clear_context_carry,
            recovery_restore_required,
        } = request;
        if !reinjected && !clear_context_carry && !recovery_restore_required {
            return Ok(None);
        }
        let at = now_timestamp();
        if !self.canonical_authority_active() {
            #[cfg(test)]
            {
                return match self.update_meta_if_changed(app_data_dir, session_id, |meta| {
                    apply_context_restore_completion_to_meta(
                        meta,
                        expected_provider_session_generation,
                        expected_turn_id,
                        reinjected,
                        clear_context_carry,
                        recovery_restore_required,
                        at,
                    )
                }) {
                    Err(error)
                        if error == CONTEXT_RESTORE_COMPLETION_FENCED
                            || error == CONTEXT_RESTORE_COMPLETION_UNCHANGED =>
                    {
                        Ok(None)
                    }
                    result => result,
                };
            }
            #[cfg(not(test))]
            unreachable!("production mutation admission rejects a missing SQLite authority");
        }
        let patch = EventProjectionMetaPatch::ContextRestoreCompleted {
            expected_provider_session_generation,
            expected_turn_id,
            reinjected,
            clear_context_carry,
            recovery_restore_required,
            at,
        };
        match self.commit_agent_events_with_additional_mutations(
            session_id,
            &[],
            Vec::new(),
            None,
            Some(patch),
            None,
            crate::domain::local_event::CommitOperationKind::Projection,
        ) {
            Ok(()) => {}
            Err(error)
                if error == CONTEXT_RESTORE_COMPLETION_FENCED
                    || error == CONTEXT_RESTORE_COMPLETION_UNCHANGED =>
            {
                return Ok(None);
            }
            Err(error) => return Err(error),
        }
        let meta = self
            .get_session_meta(app_data_dir, session_id)?
            .ok_or_else(|| format!("Session not found: {session_id}"))?;
        let lifecycle_fenced = matches!(
            meta.state,
            SessionState::Error | SessionState::Closed | SessionState::Archived
        );
        let settled = if recovery_restore_required {
            meta.provider_session_generation == expected_provider_session_generation
                && meta.context_reinjection_generation.is_none()
                && meta.recovery_publication_snapshot.is_none()
                && !matches!(
                    meta.pending_recovery_message,
                    Some(PendingRecoveryMessage::Error { .. })
                )
                && !lifecycle_fenced
                && (!reinjected || meta.context_carry == Some(ContextCarryState::Reinjected))
        } else {
            let next_generation = expected_provider_session_generation.checked_add(1);
            let generation_matches = meta.provider_session_generation
                == expected_provider_session_generation
                || next_generation == Some(meta.provider_session_generation);
            let desired_carry = reinjected.then_some(ContextCarryState::Reinjected);
            let backend_recovery_observation = meta
                .provider_session_observation_id
                .as_deref()
                .is_some_and(|identity| identity.starts_with("backend-recovery/v1:"));
            generation_matches
                && expected_turn_id.is_some()
                && meta.last_turn_id == expected_turn_id
                && !backend_recovery_observation
                && meta.context_carry == desired_carry
                && meta.recovery_publication_snapshot.is_none()
                && meta.pending_recovery_message.is_none()
                && meta.context_reinjection_generation.is_none()
                && !lifecycle_fenced
        };
        Ok(settled.then_some(meta))
    }

    pub fn update_system_context_private_meta_if_changed(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        context_epoch: Option<ContextEpochMeta>,
        workflow_instructions: Vec<String>,
        agent_read_paths: Option<Vec<PathBuf>>,
    ) -> Result<Option<SessionMeta>, String> {
        self.update_meta_if_changed(app_data_dir, session_id, |meta| {
            if meta.context_epoch == context_epoch
                && meta.workflow_instructions == workflow_instructions
                && (agent_read_paths.is_none() || meta.agent_read_paths == agent_read_paths)
            {
                return Ok(false);
            }
            meta.context_epoch = context_epoch;
            meta.workflow_instructions = workflow_instructions;
            if agent_read_paths.is_some() {
                meta.agent_read_paths = agent_read_paths.clone();
            }
            meta.updated_at = now_timestamp();
            Ok(true)
        })
    }

    #[cfg(test)]
    pub fn update_resume_metadata_if_changed(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        agent_session_id: Option<String>,
        context_carry: Option<ContextCarryState>,
    ) -> Result<Option<SessionMeta>, String> {
        self.update_meta_if_changed(app_data_dir, session_id, |meta| {
            if meta.agent_session_id == agent_session_id && meta.context_carry == context_carry {
                return Ok(false);
            }
            meta.agent_session_id = agent_session_id;
            meta.context_carry = context_carry;
            meta.updated_at = now_timestamp();
            Ok(true)
        })
    }

    pub fn get_session_page(
        &self,
        _app_data_dir: &Path,
        session_id: &str,
        cursor: Option<PageCursor>,
        limit: usize,
    ) -> Result<Option<SessionPage>, String> {
        if self.canonical_authority_active() {
            return self
                .canonical_message_page(session_id, cursor, limit)
                .map(Some);
        }
        #[cfg(test)]
        return self
            .test_storage()
            .get_session_page(_app_data_dir, session_id, cursor, limit);
        #[cfg(not(test))]
        unreachable!("production always has a SQLite event authority")
    }

    #[cfg(test)]
    pub fn append_message(
        &self,
        _app_data_dir: &Path,
        session_id: &str,
        message: &ChatMessage,
    ) -> Result<SessionMeta, String> {
        self.ensure_canonical_mutation_admission()?;
        #[cfg(test)]
        if let Some(hook) = self.append_message_hook.read().clone() {
            hook(session_id, message)?;
        }
        if !self.canonical_authority_active() {
            #[cfg(test)]
            return self
                .test_storage()
                .append_message(_app_data_dir, session_id, message);
            #[cfg(not(test))]
            return Err("agent-session SQLite event authority is not configured".to_string());
        }
        let current = self
            .canonical_session_projection(session_id)?
            .ok_or_else(|| format!("Session projection not found: {session_id}"))?;
        let mut meta = current.meta;
        meta.message_count = add_sqlite_count(meta.message_count, 1, "session message count")?;
        if meta.first_message_preview.is_empty() {
            meta.first_message_preview =
                super::first_message_preview(std::slice::from_ref(message));
        }
        meta.updated_at = meta.updated_at.max(message.timestamp);
        meta.state_revision = next_sqlite_counter(meta.state_revision, "session state revision")?;
        self.commit_session_projection_snapshot(CanonicalAgentSessionProjection {
            meta: meta.clone(),
            title: current.title,
            messages: vec![message.clone()],
            reducer_events: current.reducer_events,
            queue_paused_at: current.queue_paused_at,
            latest_token_usage: None,
            pending_send_queue: current.pending_send_queue,
        })?;
        Ok(meta)
    }

    pub fn get_session_attachment(
        &self,
        _app_data_dir: &Path,
        session_id: &str,
        attachment_id: &str,
    ) -> Result<Option<SessionAttachment>, String> {
        if self.canonical_authority_active() {
            if attachment_id.len() != 64
                || !attachment_id.bytes().all(|byte| byte.is_ascii_hexdigit())
            {
                return Ok(None);
            }
            return self
                .canonical_content_blob(session_id, format!("attachment:{attachment_id}"))
                .and_then(|record| {
                    record
                        .map(|record| {
                            let crate::domain::local_event::AgentContentBlobRecord::Attachment {
                                id,
                                media_type,
                                bytes,
                            } = record
                            else {
                                return Err(
                                    "SQLite attachment identity is incompatible".to_string()
                                );
                            };
                            if id != attachment_id {
                                return Err(
                                    "SQLite attachment identity is incompatible".to_string()
                                );
                            }
                            Ok(SessionAttachment {
                                data: base64::engine::general_purpose::STANDARD.encode(bytes),
                                media_type,
                            })
                        })
                        .transpose()
                });
        }
        #[cfg(test)]
        return self.test_storage().get_session_attachment(
            _app_data_dir,
            session_id,
            attachment_id,
        );
        #[cfg(not(test))]
        unreachable!("production always has a SQLite event authority")
    }

    pub fn get_session_tool_output(
        &self,
        _app_data_dir: &Path,
        session_id: &str,
        tool_output_id: &str,
    ) -> Result<Option<SessionToolOutput>, String> {
        if self.canonical_authority_active() {
            return self
                .canonical_content_blob(session_id, format!("tool_output:{tool_output_id}"))
                .and_then(|record| {
                    record
                        .map(|record| {
                            let crate::domain::local_event::AgentContentBlobRecord::ToolOutput {
                                id,
                                content,
                            } = record
                            else {
                                return Err(
                                    "SQLite tool output identity is incompatible".to_string()
                                );
                            };
                            if id != tool_output_id {
                                return Err(
                                    "SQLite tool output identity is incompatible".to_string()
                                );
                            }
                            Ok(SessionToolOutput {
                                byte_size: content.len() as u64,
                                content,
                            })
                        })
                        .transpose()
                });
        }
        #[cfg(test)]
        return self.test_storage().get_session_tool_output(
            _app_data_dir,
            session_id,
            tool_output_id,
        );
        #[cfg(not(test))]
        unreachable!("production always has a SQLite event authority")
    }

    /// Atomically records streaming-domain events and the exact public message snapshot.
    ///
    /// Runtime streaming must use this boundary before publishing a delta. Otherwise an event
    /// can become durable while its message projection fails (or the inverse), leaving live and
    /// reload views with different prefixes.
    pub(crate) fn persist_streaming_parts_with_events(
        &self,
        _app_data_dir: &Path,
        session_id: &str,
        events: &[AgentSessionEvent],
        message_id: &str,
        parts: &[MessagePart],
        streaming_final_seq: u64,
    ) -> Result<Vec<MessagePart>, String> {
        #[cfg(test)]
        if let Some(hook) = self.append_event_hook.read().clone() {
            for event in events {
                hook(session_id, event)?;
            }
        }
        #[cfg(test)]
        if let Some(hook) = self.persist_parts_hook.read().clone() {
            hook(session_id, message_id, parts)?;
        }
        if !self.canonical_authority_active() {
            #[cfg(test)]
            {
                let supplied_parts = parts.to_vec();
                let (_, (), persisted_parts) = self.commit_projection_and_notify(
                    _app_data_dir,
                    session_id,
                    events,
                    |projected, projected_meta| {
                        let completed_at = projected
                            .message_for_id(message_id)
                            .filter(|message| message.role == MessageRole::Agent)
                            .ok_or_else(|| {
                                format!(
                                    "Streaming projection omitted message {message_id} for {session_id}"
                                )
                            })?
                            .timestamp;
                        Ok((
                            AgentSessionProjectionCommit {
                                meta: projected_meta,
                                message: AgentSessionProjectedMessage::PersistParts {
                                    message_id: message_id.to_string(),
                                    parts: supplied_parts.clone(),
                                    streaming_final_seq,
                                    completed_at,
                                },
                            },
                            (),
                        ))
                    },
                )?;
                return Ok(persisted_parts);
            }
            #[cfg(not(test))]
            return Err("agent-session SQLite event authority is not configured".to_string());
        }
        self.commit_agent_events_with_additional_mutations(
            session_id,
            events,
            Vec::new(),
            Some(TerminalMessageProjectionPatch {
                message_id: message_id.to_string(),
                streaming_final_seq,
                timestamp: None,
                parts: Some(parts.to_vec()),
            }),
            None,
            None,
            crate::domain::local_event::CommitOperationKind::Projection,
        )?;
        self.canonical_message_projection(session_id, message_id)?
            .and_then(|message| message.parts)
            .ok_or_else(|| {
                format!("Streaming message projection not found: {session_id}/{message_id}")
            })
    }

    pub fn persist_message_parts(
        &self,
        _app_data_dir: &Path,
        session_id: &str,
        message_id: &str,
        parts: &[MessagePart],
        streaming_final_seq: u64,
        completed_at: Option<f64>,
    ) -> Result<Vec<MessagePart>, String> {
        #[cfg(test)]
        if let Some(hook) = self.persist_parts_hook.read().clone() {
            hook(session_id, message_id, parts)?;
        }
        if !self.canonical_authority_active() {
            #[cfg(test)]
            return self.test_storage().persist_message_parts(
                _app_data_dir,
                session_id,
                message_id,
                parts,
                streaming_final_seq,
                completed_at,
            );
            #[cfg(not(test))]
            return Err("agent-session SQLite event authority is not configured".to_string());
        }
        let current = self
            .canonical_session_projection(session_id)?
            .ok_or_else(|| format!("Session projection not found: {session_id}"))?;
        let mut message = self
            .canonical_message_projection(session_id, message_id)?
            .ok_or_else(|| {
                format!("Message not found after projection: {session_id}/{message_id}")
            })?;
        message.parts = Some(parts.to_vec());
        message.streaming_final_seq = streaming_final_seq;
        if let Some(completed_at) = completed_at {
            message.timestamp = completed_at;
        }
        self.commit_session_projection_snapshot(CanonicalAgentSessionProjection {
            meta: current.meta,
            title: current.title,
            messages: vec![message],
            reducer_events: current.reducer_events,
            queue_paused_at: current.queue_paused_at,
            latest_token_usage: current.latest_token_usage,
            pending_send_queue: current.pending_send_queue,
        })?;
        Ok(parts.to_vec())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Barrier};

    use super::*;

    fn ids(values: impl IntoIterator<Item = String>) -> HashSet<String> {
        values.into_iter().collect()
    }

    fn turn_started(turn_id: u64) -> AgentSessionEvent {
        AgentSessionEvent::TurnStarted {
            turn_id,
            message_id: format!("human-{turn_id}"),
            assistant_message_id: Some(format!("agent-{turn_id}")),
            prompt: crate::domain::agent_session::events::PromptInput::default(),
            at: turn_id as f64,
        }
    }

    fn interrupted(turn_id: u64) -> AgentSessionEvent {
        AgentSessionEvent::TurnInterrupted {
            turn_id,
            reason: crate::domain::agent_session::events::InterruptReason::Abort,
            exit_code: 130,
            error: None,
        }
    }

    fn stop_resolution(turn_id: u64) -> AgentSessionEvent {
        AgentSessionEvent::StopResolutionRecorded {
            operation_id: format!("stop-{turn_id}"),
            turn_id,
            resolution: crate::domain::agent_session::events::StopResolution::Superseded,
            at: 9.0,
        }
    }

    #[test]
    fn backend_recovery_obligation_mutation_keeps_closed_identity_and_detail() {
        let record = crate::domain::local_event::ObligationRecord::BackendSessionRecovery {
            session_id: "session-1".to_string(),
            recovery_id: "recovery-1".to_string(),
            detail:
                crate::domain::local_event::BackendSessionRecoveryObligationRecord::EffectReserved {
                    old_provider_session_generation: 7,
                    reason: BackendSessionRecoveryReason::BackendSessionLost,
                    reserved_at_bits: 42,
                },
            state: crate::domain::local_event::ObligationStateRecord::EffectReserved,
        };
        let mutation = backend_recovery_obligation_mutation(
            "backend-recovery:session-1:recovery-1".to_string(),
            record.clone(),
            None,
            None,
        )
        .unwrap();

        assert!(matches!(
            mutation,
            crate::domain::local_event::LocalStateMutation::Obligation(
                crate::domain::local_event::ObligationMutation {
                    obligation_id,
                    record: stored,
                    ..
                }
            ) if obligation_id == "backend-recovery:session-1:recovery-1" && stored == record
        ));
    }

    #[test]
    fn terminal_projection_drops_same_turn_duplicate_but_keeps_resolution_fact() {
        let previous = vec![turn_started(4), interrupted(4)];
        let supplied = vec![
            AgentSessionEvent::FinalPartsRecorded {
                turn_id: 4,
                message_id: "agent-4".to_string(),
                parts: Vec::new(),
            },
            interrupted(4),
            stop_resolution(4),
        ];

        assert_eq!(
            complete_terminal_projection_events(&previous, &supplied),
            vec![stop_resolution(4)]
        );
    }

    #[test]
    fn terminal_projection_ignores_old_turn_candidate_after_newer_turn_started() {
        let previous = vec![turn_started(4), interrupted(4), turn_started(5)];
        let supplied = vec![
            AgentSessionEvent::QueuePaused { at: 8.0 },
            interrupted(4),
            stop_resolution(4),
        ];

        assert_eq!(
            complete_terminal_projection_events(&previous, &supplied),
            vec![stop_resolution(4)]
        );
    }

    #[test]
    fn failed_terminal_adds_pause_only_when_latest_projection_is_unpaused() {
        let terminal = AgentSessionEvent::TurnCompleted {
            turn_id: 4,
            exit_code: 1,
            stop_reason: None,
            token_usage: None,
        };
        let supplied = vec![terminal.clone(), AgentSessionEvent::QueuePaused { at: 9.0 }];

        assert_eq!(
            complete_terminal_projection_events(&[turn_started(4)], &supplied),
            supplied
        );
        assert_eq!(
            complete_terminal_projection_events(
                &[turn_started(4), AgentSessionEvent::QueuePaused { at: 8.0 },],
                &supplied,
            ),
            vec![terminal]
        );
    }

    #[test]
    fn permission_response_pending_replays_before_claim_but_effect_reserved_does_not() {
        let store = crate::test_support::build_session_store();
        let response = crate::domain::agent_session::entities::PermissionResponse {
            request_id: "permission-1".to_string(),
            decision: crate::domain::agent_session::entities::PermissionResponseDecision::Allow {
                updated_input: None,
                answers: None,
            },
        };
        let first = store
            .reserve_permission_response(
                Path::new("/unused"),
                "session-1",
                7,
                "permission-1",
                response.clone(),
            )
            .unwrap();
        let replay = store
            .reserve_permission_response(
                Path::new("/unused"),
                "session-1",
                7,
                "permission-1",
                response.clone(),
            )
            .unwrap();
        assert_eq!(replay, first);
        assert_eq!(
            store.load_permission_response_obligation(&first).unwrap(),
            Some(crate::domain::local_event::ObligationStateRecord::Pending)
        );

        store
            .claim_permission_response_effect("session-1", &first)
            .unwrap();
        let retry = store.reserve_permission_response(
            Path::new("/unused"),
            "session-1",
            7,
            "permission-1",
            response,
        );
        assert!(retry.unwrap_err().contains("requires reconciliation"));
    }

    fn rewrite_persisted_worktree_path(app_data_dir: &Path, session_id: &str, worktree_path: &str) {
        let meta_path = app_data_dir
            .join("sessions")
            .join(session_id)
            .join("meta.json");
        let mut meta: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&meta_path).unwrap()).unwrap();
        meta["worktreePath"] = serde_json::Value::String(worktree_path.to_string());
        std::fs::write(meta_path, serde_json::to_vec_pretty(&meta).unwrap()).unwrap();
    }

    #[test]
    fn worktree_session_queries_match_legacy_trailing_slash_without_prefix_collision() {
        let app_data_dir = tempfile::tempdir().unwrap();
        let writer = Arc::new(crate::test_support::build_session_store());
        let legacy = super::super::create_session_internal(
            &writer,
            app_data_dir.path(),
            "/repo/",
            Some("claude".to_string()),
        )
        .unwrap();
        let canonical = super::super::create_session_internal(
            &writer,
            app_data_dir.path(),
            "/repo",
            Some("claude".to_string()),
        )
        .unwrap();
        let other = super::super::create_session_internal(
            &writer,
            app_data_dir.path(),
            "/repository",
            Some("claude".to_string()),
        )
        .unwrap();

        // Simulate metadata written before worktree paths were normalized on save.
        rewrite_persisted_worktree_path(app_data_dir.path(), &legacy.id, "/repo/");
        drop(writer);

        let reader = crate::test_support::build_session_store();
        let expected = HashSet::from([legacy.id.clone(), canonical.id.clone()]);
        for query in ["/repo", "/repo/"] {
            let summaries = reader.list_sessions(app_data_dir.path(), query).unwrap();
            assert_eq!(
                ids(summaries.iter().map(|session| session.id.clone())),
                expected
            );
            assert!(
                summaries
                    .iter()
                    .all(|session| session.worktree_path == "/repo"),
                "read models must expose the normalized identity"
            );

            assert_eq!(
                ids(reader
                    .list_worktree_sessions(app_data_dir.path(), query)
                    .unwrap()
                    .into_iter()
                    .map(|session| session.id),),
                expected
            );
            assert_eq!(
                ids(reader
                    .list_worktree_sessions_full(app_data_dir.path(), query)
                    .unwrap()
                    .into_iter()
                    .map(|session| session.id),),
                expected
            );
        }

        assert_eq!(
            ids(reader
                .list_sessions(app_data_dir.path(), "/repository")
                .unwrap()
                .into_iter()
                .map(|session| session.id),),
            HashSet::from([other.id])
        );
    }

    #[test]
    fn published_lists_restore_recovery_snapshot_and_classification_after_restart() {
        let app_data_dir = tempfile::tempdir().unwrap();
        let writer = crate::test_support::build_session_store();
        let active = super::super::create_session_internal(
            &writer,
            app_data_dir.path(),
            "/repo",
            Some("codex".to_string()),
        )
        .unwrap();
        let closed = super::super::create_session_internal(
            &writer,
            app_data_dir.path(),
            "/repo",
            Some("codex".to_string()),
        )
        .unwrap();
        writer
            .set_session_state(app_data_dir.path(), &closed.id, SessionState::Closed)
            .unwrap();
        let workflow = super::super::build_new_session_with_id(
            "00000000-0000-4000-8000-000000000149".to_string(),
            "/repo",
            Some("codex".to_string()),
            crate::domain::agent_session::PermissionMode::Edit,
            None,
            false,
            true,
            Some(WorkflowNodeContextDto {
                execution_id: "recovery-workflow-execution".to_string(),
                node_execution_id: "recovery-workflow-node".to_string(),
                workflow_name: "Recovery workflow".to_string(),
                node_name: "Recover session".to_string(),
                attempt: 1,
                parent_node_name: None,
                parent_attempt: None,
                order: 0,
                startup_timeout_secs: None,
                startup_max_retries: None,
                stale_timeout_secs: None,
            }),
        );
        writer
            .save_full_session_for_restore(app_data_dir.path(), &workflow)
            .unwrap();

        for session_id in [&active.id, &closed.id, &workflow.id] {
            writer
                .begin_backend_session_recovery(
                    app_data_dir.path(),
                    session_id,
                    &format!("recovery-{session_id}"),
                    BackendSessionRecoveryReason::BackendSessionLost,
                )
                .unwrap();
        }

        assert_eq!(
            ids(writer
                .list_published_sessions(app_data_dir.path(), "/repo")
                .unwrap()
                .into_iter()
                .map(|session| session.id)),
            HashSet::from([active.id.clone(), workflow.id.clone()])
        );
        assert_eq!(
            ids(writer
                .list_published_closed_sessions(app_data_dir.path(), "/repo")
                .unwrap()
                .into_iter()
                .map(|session| session.id)),
            HashSet::from([closed.id.clone()])
        );
        drop(writer);

        let reopened = crate::test_support::build_session_store();
        assert_eq!(
            ids(reopened
                .list_sessions(app_data_dir.path(), "/repo")
                .unwrap()
                .into_iter()
                .map(|session| session.id)),
            HashSet::from([active.id.clone(), workflow.id.clone()])
        );
        assert_eq!(
            ids(reopened
                .list_closed_sessions(app_data_dir.path(), "/repo")
                .unwrap()
                .into_iter()
                .map(|session| session.id)),
            HashSet::from([closed.id.clone()])
        );
        for session_id in [&active.id, &closed.id, &workflow.id] {
            let recovery = TurnEventLog::from_events(
                reopened
                    .load_session_events(app_data_dir.path(), session_id)
                    .unwrap(),
            )
            .project()
            .backend_recovery;
            assert_eq!(
                recovery,
                Some(BackendSessionRecoveryProjection::Recovering {
                    recovery_id: format!("recovery-{session_id}"),
                    old_provider_session_generation: 0,
                    reason: BackendSessionRecoveryReason::BackendSessionLost,
                })
            );
        }
        assert_eq!(
            reopened
                .get_session_meta(app_data_dir.path(), &closed.id)
                .unwrap()
                .unwrap()
                .state,
            SessionState::Closed,
            "recovery publication must never reopen a closed session"
        );
        let published = reopened
            .list_published_sessions(app_data_dir.path(), "/repo")
            .unwrap();
        let workflow_after_restart = published
            .iter()
            .find(|session| session.id == workflow.id)
            .expect("workflow-owned recovery remains published under its owner");
        assert!(workflow_after_restart.workflow_node_session);
        let owner = workflow_after_restart
            .workflow_node_context
            .as_ref()
            .expect("workflow recovery owner context");
        assert_eq!(owner.execution_id, "recovery-workflow-execution");
        assert_eq!(owner.node_execution_id, "recovery-workflow-node");
        assert_eq!(
            ids(published.into_iter().map(|session| session.id)),
            HashSet::from([active.id, workflow.id])
        );
        assert_eq!(
            ids(reopened
                .list_published_closed_sessions(app_data_dir.path(), "/repo")
                .unwrap()
                .into_iter()
                .map(|session| session.id)),
            HashSet::from([closed.id])
        );
    }

    #[test]
    fn recovery_start_atomically_persists_publication_snapshot_with_recovering_projection() {
        let app_data_dir = tempfile::tempdir().unwrap();
        let local_store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
            crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
                app_data_dir.path().to_path_buf(),
            ),
        )
        .unwrap();
        let installation_id = local_store.installation_id().to_string();
        let writer = crate::test_support::build_session_store();
        let repository: Arc<dyn crate::domain::local_event::LocalEventTransactionRepository> =
            local_store.clone();
        writer.set_local_event_repository(
            repository,
            installation_id.clone(),
            Arc::new(
                crate::adaptor::gateway::agent_session::session_storage::AgentSessionProjectionCodecV1,
            ),
        );
        let session = super::super::create_session_internal(
            &writer,
            app_data_dir.path(),
            "/repo",
            Some("codex".to_string()),
        )
        .unwrap();

        local_store.fault_injector().arm_fail_before_begin();
        assert!(writer
            .begin_backend_session_recovery(
                app_data_dir.path(),
                &session.id,
                "atomic-recovery",
                BackendSessionRecoveryReason::BackendSessionLost,
            )
            .is_err());
        let unchanged = writer
            .canonical_session_projection(&session.id)
            .unwrap()
            .unwrap();
        assert!(unchanged.meta.recovery_publication_snapshot.is_none());
        assert!(unchanged.reducer_events.iter().all(|event| !matches!(
            event,
            AgentSessionEvent::BackendSessionRecoveryStarted { .. }
        )));

        writer
            .begin_backend_session_recovery(
                app_data_dir.path(),
                &session.id,
                "atomic-recovery",
                BackendSessionRecoveryReason::BackendSessionLost,
            )
            .unwrap();
        drop(writer);

        let reopened = crate::test_support::build_session_store();
        let repository: Arc<dyn crate::domain::local_event::LocalEventTransactionRepository> =
            local_store;
        reopened.set_local_event_repository(
            repository,
            installation_id,
            Arc::new(
                crate::adaptor::gateway::agent_session::session_storage::AgentSessionProjectionCodecV1,
            ),
        );
        let projection = reopened
            .canonical_session_projection(&session.id)
            .unwrap()
            .unwrap();
        let snapshot = projection
            .meta
            .recovery_publication_snapshot
            .expect("recovery publication snapshot persisted with the start event");
        assert_eq!(snapshot.recovery_id, "atomic-recovery");
        assert_eq!(
            snapshot.classification.list,
            RecoveryPublicationList::SessionList
        );
        assert_eq!(snapshot.summary.state, SessionState::Active);
        assert_eq!(
            TurnEventLog::from_events(projection.reducer_events)
                .project()
                .backend_recovery,
            Some(BackendSessionRecoveryProjection::Recovering {
                recovery_id: "atomic-recovery".to_string(),
                old_provider_session_generation: 0,
                reason: BackendSessionRecoveryReason::BackendSessionLost,
            })
        );
        assert_eq!(
            ids(reopened
                .list_published_sessions(app_data_dir.path(), "/repo")
                .unwrap()
                .into_iter()
                .map(|summary| summary.id)),
            HashSet::from([session.id])
        );
    }

    #[test]
    fn recovery_start_atomically_loses_to_queue_pause_after_its_projection_read() {
        let app_data_dir = tempfile::tempdir().unwrap();
        let local_store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
            crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
                app_data_dir.path().to_path_buf(),
            ),
        )
        .unwrap();
        let writer = Arc::new(crate::test_support::build_session_store());
        let repository: Arc<dyn crate::domain::local_event::LocalEventTransactionRepository> =
            local_store.clone();
        writer.set_local_event_repository(
            repository,
            local_store.installation_id().to_string(),
            Arc::new(
                crate::adaptor::gateway::agent_session::session_storage::AgentSessionProjectionCodecV1,
            ),
        );
        let session = super::super::create_session_internal(
            &writer,
            app_data_dir.path(),
            "/repo",
            Some("codex".to_string()),
        )
        .unwrap();

        let first_recovery_commit = Arc::new(AtomicBool::new(true));
        let projection_read = Arc::new(Barrier::new(2));
        let release_recovery = Arc::new(Barrier::new(2));
        writer.set_atomic_event_commit_hook_for_test(Arc::new({
            let first_recovery_commit = first_recovery_commit.clone();
            let projection_read = projection_read.clone();
            let release_recovery = release_recovery.clone();
            move |operation_kind| {
                if operation_kind == crate::domain::local_event::CommitOperationKind::Recovery
                    && first_recovery_commit.swap(false, Ordering::SeqCst)
                {
                    projection_read.wait();
                    release_recovery.wait();
                }
                Ok(())
            }
        }));

        let recovery_writer = writer.clone();
        let recovery_data_dir = app_data_dir.path().to_path_buf();
        let recovery_session_id = session.id.clone();
        let recovery = std::thread::spawn(move || {
            recovery_writer.begin_backend_session_recovery(
                &recovery_data_dir,
                &recovery_session_id,
                "stop-race-recovery",
                BackendSessionRecoveryReason::BackendSessionLost,
            )
        });
        projection_read.wait();
        writer
            .append_session_events(
                app_data_dir.path(),
                &session.id,
                &[AgentSessionEvent::QueuePaused { at: 8.0 }],
            )
            .unwrap();
        release_recovery.wait();

        let outcome = recovery.join().unwrap().unwrap();
        assert!(matches!(
            outcome,
            BackendSessionRecoveryStartOutcome::SuppressedByQueuePause
        ));
        let projection = writer
            .canonical_session_projection(&session.id)
            .unwrap()
            .unwrap();
        assert_eq!(projection.queue_paused_at, Some(8.0));
        assert!(projection.reducer_events.iter().all(|event| !matches!(
            event,
            AgentSessionEvent::BackendSessionRecoveryStarted { .. }
        )));
        assert!(writer
            .canonical_obligation(&backend_recovery_obligation_id(
                &session.id,
                "stop-race-recovery"
            ))
            .unwrap()
            .is_none());
    }

    #[test]
    fn stop_acceptance_prepared_from_stale_revision_loses_to_recovery_start() {
        let app_data_dir = tempfile::tempdir().unwrap();
        let local_store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
            crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
                app_data_dir.path().to_path_buf(),
            ),
        )
        .unwrap();
        let writer = crate::test_support::build_session_store();
        let repository: Arc<dyn crate::domain::local_event::LocalEventTransactionRepository> =
            local_store.clone();
        writer.set_local_event_repository(
            repository,
            local_store.installation_id().to_string(),
            Arc::new(
                crate::adaptor::gateway::agent_session::session_storage::AgentSessionProjectionCodecV1,
            ),
        );
        let session = super::super::create_session_internal(
            &writer,
            app_data_dir.path(),
            "/repo",
            Some("codex".to_string()),
        )
        .unwrap();
        writer
            .append_session_events(app_data_dir.path(), &session.id, &[turn_started(1)])
            .unwrap();
        let expected_stop_revision = writer
            .canonical_session_projection(&session.id)
            .unwrap()
            .unwrap()
            .meta
            .state_revision;

        let recovery = writer
            .begin_backend_session_recovery(
                app_data_dir.path(),
                &session.id,
                "recovery-wins-before-stop-acceptance",
                BackendSessionRecoveryReason::BackendSessionLost,
            )
            .unwrap();
        assert!(matches!(
            recovery,
            BackendSessionRecoveryStartOutcome::Started(_)
        ));

        let stop_events = [
            AgentSessionEvent::StopOperationAccepted {
                operation_id: "stop-after-stale-snapshot".to_string(),
                target_turn_id: 1,
                at: 9.0,
            },
            AgentSessionEvent::TurnInterruptRequested {
                turn_id: 1,
                at: 9.0,
            },
            AgentSessionEvent::ObligationRecorded {
                obligation_id: "stop-interrupt:session:1".to_string(),
                kind: crate::domain::agent_session::events::ObligationKind::ProviderInterrupt,
                state: crate::domain::agent_session::events::ObligationState::EffectReserved,
                at: 9.0,
            },
            AgentSessionEvent::QueuePaused { at: 9.0 },
        ];
        assert!(writer
            .prepare_event_projection_mutations_if_current_revision(
                &session.id,
                expected_stop_revision,
                &stop_events,
            )
            .unwrap()
            .is_none());

        let projection = writer
            .canonical_session_projection(&session.id)
            .unwrap()
            .unwrap();
        assert!(projection.reducer_events.iter().any(|event| matches!(
            event,
            AgentSessionEvent::BackendSessionRecoveryStarted { .. }
        )));
        assert!(projection.reducer_events.iter().all(|event| !matches!(
            event,
            AgentSessionEvent::StopOperationAccepted { .. }
                | AgentSessionEvent::TurnInterruptRequested { .. }
                | AgentSessionEvent::QueuePaused { .. }
        )));
    }

    #[test]
    fn next_turn_id_advances_past_every_durable_queue_reservation() {
        let app_data_dir = tempfile::tempdir().unwrap();
        let local_store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
            crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
                app_data_dir.path().to_path_buf(),
            ),
        )
        .unwrap();
        let writer = crate::test_support::build_session_store();
        let repository: Arc<dyn crate::domain::local_event::LocalEventTransactionRepository> =
            local_store.clone();
        writer.set_local_event_repository(
            repository,
            local_store.installation_id().to_string(),
            Arc::new(
                crate::adaptor::gateway::agent_session::session_storage::AgentSessionProjectionCodecV1,
            ),
        );
        let session = super::super::create_session_internal(
            &writer,
            app_data_dir.path(),
            "/repo",
            Some("codex".to_string()),
        )
        .unwrap();
        writer
            .append_session_events(app_data_dir.path(), &session.id, &[turn_started(1)])
            .unwrap();
        let mut projection = writer
            .canonical_session_projection(&session.id)
            .unwrap()
            .unwrap();
        projection.pending_send_queue.extend([
            CanonicalQueuedSend {
                queue_item_id: "queue-2".to_string(),
                human_message_id: "human-2".to_string(),
                reserved_turn_id: "2".to_string(),
                input_ref: "input-2".to_string(),
            },
            CanonicalQueuedSend {
                queue_item_id: "queue-4".to_string(),
                human_message_id: "human-4".to_string(),
                reserved_turn_id: "4".to_string(),
                input_ref: "input-4".to_string(),
            },
        ]);
        writer
            .commit_session_projection_snapshot(projection)
            .unwrap();

        assert_eq!(
            writer
                .next_turn_id(app_data_dir.path(), &session.id)
                .unwrap(),
            5
        );
    }

    #[test]
    fn send_acceptance_rejects_an_allocation_from_an_older_queue_projection() {
        let app_data_dir = tempfile::tempdir().unwrap();
        let local_store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
            crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
                app_data_dir.path().to_path_buf(),
            ),
        )
        .unwrap();
        let writer = crate::test_support::build_session_store();
        let repository: Arc<dyn crate::domain::local_event::LocalEventTransactionRepository> =
            local_store.clone();
        writer.set_local_event_repository(
            repository,
            local_store.installation_id().to_string(),
            Arc::new(
                crate::adaptor::gateway::agent_session::session_storage::AgentSessionProjectionCodecV1,
            ),
        );
        let session = super::super::create_session_internal(
            &writer,
            app_data_dir.path(),
            "/repo",
            Some("codex".to_string()),
        )
        .unwrap();
        writer
            .append_session_events(app_data_dir.path(), &session.id, &[turn_started(1)])
            .unwrap();

        let stale = writer.send_acceptance_allocation(&session.id).unwrap();
        assert_eq!(stale.next_turn_id, 2);
        assert!(stale.has_active_turn);
        let mut projection = writer
            .canonical_session_projection(&session.id)
            .unwrap()
            .unwrap();
        projection.pending_send_queue.push(CanonicalQueuedSend {
            queue_item_id: "queue-winner".to_string(),
            human_message_id: "human-winner".to_string(),
            reserved_turn_id: "2".to_string(),
            input_ref: "input-winner".to_string(),
        });
        writer
            .commit_session_projection_snapshot(projection)
            .unwrap();
        assert!(writer
            .canonical_queue_contains_exact(
                &session.id,
                "queue-winner",
                "human-winner",
                "2",
                Some("input-winner"),
            )
            .unwrap());
        assert!(!writer
            .canonical_queue_contains_exact(
                &session.id,
                "queue-winner",
                "human-winner",
                "2",
                Some("different-input"),
            )
            .unwrap());

        let prompt = crate::domain::agent_session::events::PromptInput {
            content: "stale queued input".to_string(),
            ..Default::default()
        };
        let disposition = crate::domain::agent_session::events::SendDisposition::Queued {
            queue_item_id: "queue-stale".to_string(),
        };
        let error = writer
            .prepare_send_acceptance_mutations(SendAcceptanceProjectionInput {
                session_id: &session.id,
                initial_session: None,
                session_projection_guard: stale.session_projection_guard,
                human_message_id: "human-stale",
                prompt: &prompt,
                disposition: &disposition,
                reserved_turn_id: Some("2"),
                input_ref: "input-stale",
                events: &[],
            })
            .unwrap_err();
        assert!(error.contains("allocation projection changed"));

        let fresh = writer.send_acceptance_allocation(&session.id).unwrap();
        assert_eq!(fresh.next_turn_id, 3);
        assert!(fresh.has_active_turn);
        assert!(fresh.has_pending_queue);
        assert_ne!(
            fresh.session_projection_guard,
            stale.session_projection_guard
        );
    }

    #[test]
    fn accepted_queued_turn_can_commit_only_the_canonical_front_without_regression() {
        let app_data_dir = tempfile::tempdir().unwrap();
        let local_store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
            crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
                app_data_dir.path().to_path_buf(),
            ),
        )
        .unwrap();
        let writer = crate::test_support::build_session_store();
        let repository: Arc<dyn crate::domain::local_event::LocalEventTransactionRepository> =
            local_store.clone();
        writer.set_local_event_repository(
            repository,
            local_store.installation_id().to_string(),
            Arc::new(
                crate::adaptor::gateway::agent_session::session_storage::AgentSessionProjectionCodecV1,
            ),
        );
        let session = super::super::create_session_internal(
            &writer,
            app_data_dir.path(),
            "/repo",
            Some("codex".to_string()),
        )
        .unwrap();
        writer
            .append_session_events(
                app_data_dir.path(),
                &session.id,
                &[turn_started(1), interrupted(1)],
            )
            .unwrap();
        let mut projection = writer
            .canonical_session_projection(&session.id)
            .unwrap()
            .unwrap();
        projection.pending_send_queue.extend([
            CanonicalQueuedSend {
                queue_item_id: "queue-2".to_string(),
                human_message_id: "human-2".to_string(),
                reserved_turn_id: "2".to_string(),
                input_ref: "input-2".to_string(),
            },
            CanonicalQueuedSend {
                queue_item_id: "queue-3".to_string(),
                human_message_id: "human-3".to_string(),
                reserved_turn_id: "3".to_string(),
                input_ref: "input-3".to_string(),
            },
        ]);
        writer
            .commit_session_projection_snapshot(projection)
            .unwrap();

        assert!(writer
            .append_accepted_queued_turn_started_and_project_state(
                app_data_dir.path(),
                &session.id,
                "queue-3",
                turn_started(3),
            )
            .is_err());
        let unchanged = writer
            .canonical_session_projection(&session.id)
            .unwrap()
            .unwrap();
        assert_eq!(unchanged.meta.last_turn_id, Some(1));
        assert_eq!(
            unchanged
                .pending_send_queue
                .iter()
                .map(|entry| entry.queue_item_id.as_str())
                .collect::<Vec<_>>(),
            vec!["queue-2", "queue-3"]
        );

        writer
            .append_session_events(
                app_data_dir.path(),
                &session.id,
                &[AgentSessionEvent::QueuePaused { at: 8.0 }],
            )
            .unwrap();
        let paused = writer
            .canonical_session_projection(&session.id)
            .unwrap()
            .unwrap();
        assert_eq!(paused.queue_paused_at, Some(8.0));
        assert!(writer
            .append_accepted_queued_turn_started_and_project_state(
                app_data_dir.path(),
                &session.id,
                "queue-2",
                turn_started(2),
            )
            .is_err());
        let still_paused = writer
            .canonical_session_projection(&session.id)
            .unwrap()
            .unwrap();
        assert_eq!(still_paused.meta.state, paused.meta.state);
        assert_eq!(still_paused.queue_paused_at, paused.queue_paused_at);
        assert_eq!(still_paused.reducer_events, paused.reducer_events);
        assert_eq!(still_paused.pending_send_queue, paused.pending_send_queue);

        writer
            .append_session_events(
                app_data_dir.path(),
                &session.id,
                &[AgentSessionEvent::QueueResumed {
                    expected_paused_at: 8.0,
                    at: 9.0,
                }],
            )
            .unwrap();
        writer
            .append_accepted_queued_turn_started_and_project_state(
                app_data_dir.path(),
                &session.id,
                "queue-2",
                turn_started(2),
            )
            .unwrap();
        let after_front = writer
            .canonical_session_projection(&session.id)
            .unwrap()
            .unwrap();
        assert_eq!(after_front.meta.last_turn_id, Some(2));
        assert_eq!(after_front.pending_send_queue.len(), 1);
        assert_eq!(after_front.pending_send_queue[0].queue_item_id, "queue-3");

        assert!(writer
            .append_accepted_queued_turn_started_and_project_state(
                app_data_dir.path(),
                &session.id,
                "queue-3",
                turn_started(2),
            )
            .is_err());
        assert_eq!(
            writer
                .canonical_session_projection(&session.id)
                .unwrap()
                .unwrap()
                .meta
                .last_turn_id,
            Some(2)
        );

        writer
            .append_session_events(
                app_data_dir.path(),
                &session.id,
                &[
                    interrupted(2),
                    AgentSessionEvent::SessionClosed { at: 12.0 },
                ],
            )
            .unwrap();
        let closed = writer
            .canonical_session_projection(&session.id)
            .unwrap()
            .unwrap();
        assert_eq!(closed.meta.state, SessionState::Closed);
        assert!(writer
            .append_accepted_queued_turn_started_and_project_state(
                app_data_dir.path(),
                &session.id,
                "queue-3",
                turn_started(3),
            )
            .is_err());
        let still_closed = writer
            .canonical_session_projection(&session.id)
            .unwrap()
            .unwrap();
        assert_eq!(still_closed.meta.state, SessionState::Closed);
        assert_eq!(still_closed.reducer_events, closed.reducer_events);
        assert_eq!(still_closed.pending_send_queue, closed.pending_send_queue);
        assert!(still_closed
            .reducer_events
            .iter()
            .all(|event| !matches!(event, AgentSessionEvent::TurnStarted { turn_id: 3, .. })));
    }

    #[test]
    fn accepted_queued_turn_cannot_cross_canonical_backend_recovery() {
        let app_data_dir = tempfile::tempdir().unwrap();
        let local_store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
            crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
                app_data_dir.path().to_path_buf(),
            ),
        )
        .unwrap();
        let writer = crate::test_support::build_session_store();
        let repository: Arc<dyn crate::domain::local_event::LocalEventTransactionRepository> =
            local_store.clone();
        writer.set_local_event_repository(
            repository,
            local_store.installation_id().to_string(),
            Arc::new(
                crate::adaptor::gateway::agent_session::session_storage::AgentSessionProjectionCodecV1,
            ),
        );
        let session = super::super::create_session_internal(
            &writer,
            app_data_dir.path(),
            "/repo",
            Some("codex".to_string()),
        )
        .unwrap();
        let mut projection = writer
            .canonical_session_projection(&session.id)
            .unwrap()
            .unwrap();
        projection.pending_send_queue.push(CanonicalQueuedSend {
            queue_item_id: "queue-1".to_string(),
            human_message_id: "human-1".to_string(),
            reserved_turn_id: "1".to_string(),
            input_ref: "input-1".to_string(),
        });
        writer
            .commit_session_projection_snapshot(projection)
            .unwrap();
        let recovery = writer
            .begin_backend_session_recovery(
                app_data_dir.path(),
                &session.id,
                "recovery-wins-before-queued-start",
                BackendSessionRecoveryReason::BackendSessionLost,
            )
            .unwrap();
        assert!(matches!(
            recovery,
            BackendSessionRecoveryStartOutcome::Started(_)
        ));
        let recovering = writer
            .canonical_session_projection(&session.id)
            .unwrap()
            .unwrap();
        assert_eq!(recovering.meta.state, SessionState::Idle);

        assert!(writer
            .append_accepted_queued_turn_started_and_project_state(
                app_data_dir.path(),
                &session.id,
                "queue-1",
                turn_started(1),
            )
            .is_err());
        let unchanged = writer
            .canonical_session_projection(&session.id)
            .unwrap()
            .unwrap();
        assert_eq!(unchanged.meta.state, recovering.meta.state);
        assert_eq!(unchanged.reducer_events, recovering.reducer_events);
        assert_eq!(unchanged.pending_send_queue, recovering.pending_send_queue);
        assert!(unchanged
            .reducer_events
            .iter()
            .all(|event| !matches!(event, AgentSessionEvent::TurnStarted { turn_id: 1, .. })));
    }

    #[test]
    fn stale_provider_establishment_cannot_cross_canonical_backend_recovery() {
        let app_data_dir = tempfile::tempdir().unwrap();
        let local_store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
            crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
                app_data_dir.path().to_path_buf(),
            ),
        )
        .unwrap();
        let writer = crate::test_support::build_session_store();
        let repository: Arc<dyn crate::domain::local_event::LocalEventTransactionRepository> =
            local_store.clone();
        writer.set_local_event_repository(
            repository,
            local_store.installation_id().to_string(),
            Arc::new(
                crate::adaptor::gateway::agent_session::session_storage::AgentSessionProjectionCodecV1,
            ),
        );
        let session = super::super::create_session_internal(
            &writer,
            app_data_dir.path(),
            "/repo",
            Some("codex".to_string()),
        )
        .unwrap();
        let recovery_id = "recovery-wins-before-stale-provider-observation";

        let recovery = writer
            .begin_backend_session_recovery(
                app_data_dir.path(),
                &session.id,
                recovery_id,
                BackendSessionRecoveryReason::BackendSessionLost,
            )
            .unwrap();
        assert!(matches!(
            recovery,
            BackendSessionRecoveryStartOutcome::Started(_)
        ));
        let recovering = writer
            .get_session_meta(app_data_dir.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(recovering.provider_session_generation, 0);
        assert!(recovering.agent_session_id.is_none());
        assert!(recovering.provider_session_observation_id.is_none());

        let outcome = writer
            .record_backend_session_established(
                app_data_dir.path(),
                &session.id,
                0,
                "stale-normal-provider-observation",
                "stale-provider".to_string(),
                None,
            )
            .unwrap();
        assert!(matches!(
            outcome,
            ProviderSessionEstablishmentOutcome::Fenced
        ));
        let unchanged = writer
            .get_session_meta(app_data_dir.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(unchanged.provider_session_generation, 0);
        assert!(unchanged.agent_session_id.is_none());
        assert!(unchanged.provider_session_observation_id.is_none());

        let completed = writer
            .complete_backend_session_recovery(
                app_data_dir.path(),
                &session.id,
                recovery_id,
                0,
                "replacement-provider".to_string(),
            )
            .unwrap();
        assert_eq!(completed.provider_session_generation, 1);
        assert_eq!(
            completed.agent_session_id.as_deref(),
            Some("replacement-provider")
        );
        assert_eq!(
            completed.provider_session_observation_id,
            Some(backend_recovery_provider_observation_id(recovery_id))
        );
    }

    #[test]
    fn ordinary_context_restore_completion_cannot_cross_active_backend_recovery() {
        let app_data_dir = tempfile::tempdir().unwrap();
        let local_store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
            crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
                app_data_dir.path().to_path_buf(),
            ),
        )
        .unwrap();
        let writer = Arc::new(crate::test_support::build_session_store());
        let repository: Arc<dyn crate::domain::local_event::LocalEventTransactionRepository> =
            local_store.clone();
        writer.set_local_event_repository(
            repository,
            local_store.installation_id().to_string(),
            Arc::new(
                crate::adaptor::gateway::agent_session::session_storage::AgentSessionProjectionCodecV1,
            ),
        );
        let session = super::super::create_session_internal(
            &writer,
            app_data_dir.path(),
            "/repo",
            Some("codex".to_string()),
        )
        .unwrap();
        writer
            .append_session_events(app_data_dir.path(), &session.id, &[turn_started(1)])
            .unwrap();
        let established = writer
            .record_backend_session_established(
                app_data_dir.path(),
                &session.id,
                0,
                "ordinary-context-restore-provider",
                "ordinary-provider".to_string(),
                None,
            )
            .unwrap();
        assert!(matches!(
            established,
            ProviderSessionEstablishmentOutcome::Settled(_)
        ));

        let first_context_completion_commit = Arc::new(AtomicBool::new(true));
        let projection_read = Arc::new(Barrier::new(2));
        let release_context_completion = Arc::new(Barrier::new(2));
        writer.set_atomic_event_commit_hook_for_test(Arc::new({
            let first_context_completion_commit = first_context_completion_commit.clone();
            let projection_read = projection_read.clone();
            let release_context_completion = release_context_completion.clone();
            move |operation_kind| {
                if operation_kind == crate::domain::local_event::CommitOperationKind::Projection
                    && first_context_completion_commit.swap(false, Ordering::SeqCst)
                {
                    projection_read.wait();
                    release_context_completion.wait();
                }
                Ok(())
            }
        }));

        let context_writer = writer.clone();
        let context_data_dir = app_data_dir.path().to_path_buf();
        let context_session_id = session.id.clone();
        let context_completion = std::thread::spawn(move || {
            context_writer.complete_context_restore_after_start_if_current(
                &context_data_dir,
                &context_session_id,
                ContextRestoreCompletionRequest::after_started_turn(0, 1, true, false, false),
            )
        });
        projection_read.wait();

        let recovery_id = "recovery-wins-before-ordinary-context-completion";
        let recovery = writer.begin_backend_session_recovery(
            app_data_dir.path(),
            &session.id,
            recovery_id,
            BackendSessionRecoveryReason::BackendSessionLost,
        );
        release_context_completion.wait();
        let recovery = recovery.unwrap();
        assert!(matches!(
            recovery,
            BackendSessionRecoveryStartOutcome::Started(_)
        ));

        let outcome = context_completion.join().unwrap().unwrap();
        assert!(outcome.is_none());

        let projection = writer
            .canonical_session_projection(&session.id)
            .unwrap()
            .unwrap();
        assert_eq!(projection.meta.provider_session_generation, 1);
        assert!(projection.meta.agent_session_id.is_none());
        assert!(projection.meta.provider_session_observation_id.is_none());
        assert_eq!(projection.meta.context_reinjection_generation, None);
        assert_eq!(
            projection.meta.context_carry,
            Some(ContextCarryState::Failed)
        );
        assert_eq!(
            projection
                .meta
                .recovery_publication_snapshot
                .as_ref()
                .map(|snapshot| snapshot.recovery_id.as_str()),
            Some(recovery_id)
        );
        assert_eq!(
            projection
                .reducer_events
                .iter()
                .filter(|event| matches!(
                    event,
                    AgentSessionEvent::BackendSessionRecoveryStarted {
                        recovery_id: stored_recovery_id,
                        ..
                    } if stored_recovery_id == recovery_id
                ))
                .count(),
            1
        );
    }

    #[test]
    fn ordinary_context_restore_completion_cannot_cross_a_newer_turn() {
        let app_data_dir = tempfile::tempdir().unwrap();
        let local_store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
            crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
                app_data_dir.path().to_path_buf(),
            ),
        )
        .unwrap();
        let writer = Arc::new(crate::test_support::build_session_store());
        let repository: Arc<dyn crate::domain::local_event::LocalEventTransactionRepository> =
            local_store.clone();
        writer.set_local_event_repository(
            repository,
            local_store.installation_id().to_string(),
            Arc::new(
                crate::adaptor::gateway::agent_session::session_storage::AgentSessionProjectionCodecV1,
            ),
        );
        let session = super::super::create_session_internal(
            &writer,
            app_data_dir.path(),
            "/repo",
            Some("codex".to_string()),
        )
        .unwrap();
        writer
            .append_session_events(app_data_dir.path(), &session.id, &[turn_started(1)])
            .unwrap();

        let first_context_completion_commit = Arc::new(AtomicBool::new(true));
        let projection_read = Arc::new(Barrier::new(2));
        let release_context_completion = Arc::new(Barrier::new(2));
        writer.set_atomic_event_commit_hook_for_test(Arc::new({
            let first_context_completion_commit = first_context_completion_commit.clone();
            let projection_read = projection_read.clone();
            let release_context_completion = release_context_completion.clone();
            move |operation_kind| {
                if operation_kind == crate::domain::local_event::CommitOperationKind::Projection
                    && first_context_completion_commit.swap(false, Ordering::SeqCst)
                {
                    projection_read.wait();
                    release_context_completion.wait();
                }
                Ok(())
            }
        }));

        let context_writer = writer.clone();
        let context_data_dir = app_data_dir.path().to_path_buf();
        let context_session_id = session.id.clone();
        let context_completion = std::thread::spawn(move || {
            context_writer.complete_context_restore_after_start_if_current(
                &context_data_dir,
                &context_session_id,
                ContextRestoreCompletionRequest::after_started_turn(0, 1, true, false, false),
            )
        });
        projection_read.wait();
        writer
            .append_session_events(app_data_dir.path(), &session.id, &[turn_started(2)])
            .unwrap();
        release_context_completion.wait();

        assert!(context_completion.join().unwrap().unwrap().is_none());
        let projection = writer
            .canonical_session_projection(&session.id)
            .unwrap()
            .unwrap();
        assert_eq!(projection.meta.last_turn_id, Some(2));
        assert_eq!(projection.meta.context_carry, None);
    }

    #[test]
    fn stale_recovery_context_completion_cannot_clear_newer_generation_marker() {
        let app_data_dir = tempfile::tempdir().unwrap();
        let local_store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
            crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
                app_data_dir.path().to_path_buf(),
            ),
        )
        .unwrap();
        let writer = crate::test_support::build_session_store();
        let repository: Arc<dyn crate::domain::local_event::LocalEventTransactionRepository> =
            local_store.clone();
        writer.set_local_event_repository(
            repository,
            local_store.installation_id().to_string(),
            Arc::new(
                crate::adaptor::gateway::agent_session::session_storage::AgentSessionProjectionCodecV1,
            ),
        );
        let session = super::super::create_session_internal(
            &writer,
            app_data_dir.path(),
            "/repo",
            Some("codex".to_string()),
        )
        .unwrap();

        let first_recovery_id = "first-context-recovery-generation";
        let first_recovery = writer
            .begin_backend_session_recovery(
                app_data_dir.path(),
                &session.id,
                first_recovery_id,
                BackendSessionRecoveryReason::BackendSessionLost,
            )
            .unwrap();
        assert!(matches!(
            first_recovery,
            BackendSessionRecoveryStartOutcome::Started(_)
        ));
        let first_completed = writer
            .complete_backend_session_recovery(
                app_data_dir.path(),
                &session.id,
                first_recovery_id,
                0,
                "first-recovery-provider".to_string(),
            )
            .unwrap();
        assert_eq!(first_completed.provider_session_generation, 1);
        assert_eq!(first_completed.context_reinjection_generation, Some(1));

        let second_recovery_id = "second-context-recovery-generation";
        let second_recovery = writer
            .begin_backend_session_recovery(
                app_data_dir.path(),
                &session.id,
                second_recovery_id,
                BackendSessionRecoveryReason::BackendSessionLost,
            )
            .unwrap();
        assert!(matches!(
            second_recovery,
            BackendSessionRecoveryStartOutcome::Started(_)
        ));
        let second_completed = writer
            .complete_backend_session_recovery(
                app_data_dir.path(),
                &session.id,
                second_recovery_id,
                1,
                "second-recovery-provider".to_string(),
            )
            .unwrap();
        assert_eq!(second_completed.provider_session_generation, 2);
        assert_eq!(second_completed.context_reinjection_generation, Some(2));

        let outcome = writer
            .complete_context_reinjection_if_required(app_data_dir.path(), &session.id, 1, true)
            .unwrap();
        assert!(outcome.is_none());

        let projection = writer
            .canonical_session_projection(&session.id)
            .unwrap()
            .unwrap();
        assert_eq!(projection.meta.provider_session_generation, 2);
        assert_eq!(
            projection.meta.agent_session_id.as_deref(),
            Some("second-recovery-provider")
        );
        assert_eq!(
            projection.meta.provider_session_observation_id,
            Some(backend_recovery_provider_observation_id(second_recovery_id))
        );
        assert_eq!(projection.meta.context_reinjection_generation, Some(2));
        assert_eq!(
            projection.meta.context_carry,
            Some(ContextCarryState::Failed)
        );
        assert!(projection.meta.recovery_publication_snapshot.is_none());
        assert!(matches!(
            projection.meta.pending_recovery_message,
            Some(PendingRecoveryMessage::Notice {
                ref recovery_id,
                ..
            }) if recovery_id == second_recovery_id
        ));
    }

    #[tokio::test]
    async fn prepared_stop_acceptance_loses_when_recovery_commits_before_stop() {
        let app_data_dir = tempfile::tempdir().unwrap();
        let local_store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
            crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
                app_data_dir.path().to_path_buf(),
            ),
        )
        .unwrap();
        let writer = crate::test_support::build_session_store();
        let repository: Arc<dyn crate::domain::local_event::LocalEventTransactionRepository> =
            local_store.clone();
        writer.set_local_event_repository(
            repository.clone(),
            local_store.installation_id().to_string(),
            Arc::new(
                crate::adaptor::gateway::agent_session::session_storage::AgentSessionProjectionCodecV1,
            ),
        );
        let session = super::super::create_session_internal(
            &writer,
            app_data_dir.path(),
            "/repo",
            Some("codex".to_string()),
        )
        .unwrap();
        writer
            .append_session_events(app_data_dir.path(), &session.id, &[turn_started(1)])
            .unwrap();
        let expected_stop_revision = writer
            .canonical_session_projection(&session.id)
            .unwrap()
            .unwrap()
            .meta
            .state_revision;
        let stop_events = [
            AgentSessionEvent::StopOperationAccepted {
                operation_id: "prepared-stop".to_string(),
                target_turn_id: 1,
                at: 9.0,
            },
            AgentSessionEvent::TurnInterruptRequested {
                turn_id: 1,
                at: 9.0,
            },
            AgentSessionEvent::QueuePaused { at: 9.0 },
        ];
        let stop_mutations = writer
            .prepare_event_projection_mutations_if_current_revision(
                &session.id,
                expected_stop_revision,
                &stop_events,
            )
            .unwrap()
            .expect("Stop preparation starts from the exact snapshot revision");

        let recovery = writer
            .begin_backend_session_recovery(
                app_data_dir.path(),
                &session.id,
                "recovery-commits-after-stop-preparation",
                BackendSessionRecoveryReason::BackendSessionLost,
            )
            .unwrap();
        assert!(matches!(
            recovery,
            BackendSessionRecoveryStartOutcome::Started(_)
        ));

        let error = repository
            .commit_batch(crate::domain::local_event::LocalAtomicBatch {
                commit_id: crate::domain::local_event::CommitIdentity::parse(
                    "prepared-stop-projection-cas",
                )
                .unwrap(),
                idempotency: crate::domain::local_event::IdempotencyBinding {
                    installation_id: local_store.installation_id().to_string(),
                    operation_kind: crate::domain::local_event::CommitOperationKind::Projection,
                    idempotency_key: "prepared-stop-projection-cas".to_string(),
                    payload_hash: [29; 32],
                },
                expected_heads: Vec::new(),
                events: Vec::new(),
                state_mutations: stop_mutations,
            })
            .await
            .expect_err("the Stop projection CAS must lose to the recovery commit");
        assert!(matches!(
            error,
            crate::domain::local_event::CommitBatchError::PayloadConflict
                | crate::domain::local_event::CommitBatchError::StreamHeadConflict { .. }
        ));
        let projection = writer
            .canonical_session_projection(&session.id)
            .unwrap()
            .unwrap();
        assert!(projection.reducer_events.iter().any(|event| matches!(
            event,
            AgentSessionEvent::BackendSessionRecoveryStarted { .. }
        )));
        assert!(projection.reducer_events.iter().all(|event| !matches!(
            event,
            AgentSessionEvent::StopOperationAccepted { .. }
                | AgentSessionEvent::TurnInterruptRequested { .. }
                | AgentSessionEvent::QueuePaused { .. }
        )));
    }

    #[test]
    fn recovering_publication_survives_sqlite_authority_restart_without_provider_resume() {
        let app_data_dir = tempfile::tempdir().unwrap();
        let local_store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
            crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
                app_data_dir.path().to_path_buf(),
            ),
        )
        .unwrap();
        let installation_id = local_store.installation_id().to_string();
        let writer = crate::test_support::build_session_store();
        let repository: Arc<dyn crate::domain::local_event::LocalEventTransactionRepository> =
            local_store.clone();
        writer.set_local_event_repository(
            repository,
            installation_id.clone(),
            Arc::new(
                crate::adaptor::gateway::agent_session::session_storage::AgentSessionProjectionCodecV1,
            ),
        );

        let active = super::super::create_session_internal(
            &writer,
            app_data_dir.path(),
            "/repo",
            Some("codex".to_string()),
        )
        .unwrap();
        let closed = super::super::create_session_internal(
            &writer,
            app_data_dir.path(),
            "/repo",
            Some("codex".to_string()),
        )
        .unwrap();
        let archived = super::super::create_session_internal(
            &writer,
            app_data_dir.path(),
            "/repo",
            Some("codex".to_string()),
        )
        .unwrap();
        let workflow = super::super::build_new_session_with_id(
            "00000000-0000-4000-8000-000000000150".to_string(),
            "/repo",
            Some("codex".to_string()),
            crate::domain::agent_session::PermissionMode::Edit,
            None,
            false,
            true,
            Some(WorkflowNodeContextDto {
                execution_id: "sqlite-restart-execution".to_string(),
                node_execution_id: "sqlite-restart-node".to_string(),
                workflow_name: "SQLite restart workflow".to_string(),
                node_name: "Recover session".to_string(),
                attempt: 1,
                parent_node_name: None,
                parent_attempt: None,
                order: 0,
                startup_timeout_secs: None,
                startup_max_retries: None,
                stale_timeout_secs: None,
            }),
        );
        writer
            .save_full_session_for_restore(app_data_dir.path(), &workflow)
            .unwrap();

        for session_id in [&active.id, &closed.id, &archived.id, &workflow.id] {
            writer
                .record_backend_session_established(
                    app_data_dir.path(),
                    session_id,
                    0,
                    &format!("provider-establishment-{session_id}"),
                    format!("provider-{session_id}"),
                    None,
                )
                .unwrap();
        }
        writer
            .set_session_state(app_data_dir.path(), &closed.id, SessionState::Closed)
            .unwrap();
        writer
            .set_session_state(app_data_dir.path(), &archived.id, SessionState::Archived)
            .unwrap();

        for session_id in [&active.id, &closed.id, &archived.id, &workflow.id] {
            writer
                .begin_backend_session_recovery(
                    app_data_dir.path(),
                    session_id,
                    &format!("restart-recovery-{session_id}"),
                    BackendSessionRecoveryReason::BackendSessionLost,
                )
                .unwrap();
        }

        let turn_lifecycle = super::super::create_session_internal(
            &writer,
            app_data_dir.path(),
            "/turn-lifecycle",
            Some("codex".to_string()),
        )
        .unwrap();
        writer
            .set_session_state(app_data_dir.path(), &turn_lifecycle.id, SessionState::Idle)
            .unwrap();
        writer
            .append_turn_started_and_project_state(
                app_data_dir.path(),
                &turn_lifecycle.id,
                turn_started(41),
            )
            .unwrap();
        assert_eq!(
            writer
                .get_session_meta(app_data_dir.path(), &turn_lifecycle.id)
                .unwrap()
                .unwrap()
                .state,
            SessionState::Active
        );
        writer
            .append_session_events(app_data_dir.path(), &turn_lifecycle.id, &[interrupted(41)])
            .unwrap();
        assert_eq!(
            writer
                .get_session_meta(app_data_dir.path(), &turn_lifecycle.id)
                .unwrap()
                .unwrap()
                .state,
            SessionState::Idle,
            "ordinary turn terminal projection remains reducer-owned"
        );

        // Release both the usecase-owned authority reference and the concrete
        // SQLite writer lock. The second open must rebuild every public read
        // from the permanent store rather than an in-memory projection.
        drop(writer);
        drop(local_store);

        let reopened_local_store =
            crate::adaptor::gateway::local_event_store::LocalEventStore::open(
                crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
                    app_data_dir.path().to_path_buf(),
                ),
            )
            .unwrap();
        assert_eq!(reopened_local_store.installation_id(), installation_id);
        let reopened = crate::test_support::build_session_store();
        let repository: Arc<dyn crate::domain::local_event::LocalEventTransactionRepository> =
            reopened_local_store;
        reopened.set_local_event_repository(
            repository,
            installation_id,
            Arc::new(
                crate::adaptor::gateway::agent_session::session_storage::AgentSessionProjectionCodecV1,
            ),
        );

        let published = reopened
            .list_published_sessions(app_data_dir.path(), "/repo")
            .unwrap();
        assert_eq!(
            ids(published
                .iter()
                .filter(|summary| !summary.workflow_node_session)
                .map(|summary| summary.id.clone())),
            HashSet::from([active.id.clone()])
        );
        assert_eq!(
            ids(published
                .iter()
                .filter(|summary| summary.workflow_node_session)
                .map(|summary| summary.id.clone())),
            HashSet::from([workflow.id.clone()])
        );
        let published_workflow = published
            .iter()
            .find(|summary| summary.id == workflow.id)
            .expect("workflow-owned session remains in the public workflow classification");
        let published_owner = published_workflow
            .workflow_node_context
            .as_ref()
            .expect("workflow owner survives SQLite reopen");
        assert_eq!(published_owner.execution_id, "sqlite-restart-execution");
        assert_eq!(published_owner.node_execution_id, "sqlite-restart-node");
        assert_eq!(
            ids(reopened
                .list_published_closed_sessions(app_data_dir.path(), "/repo")
                .unwrap()
                .into_iter()
                .map(|summary| summary.id)),
            HashSet::from([closed.id.clone()])
        );

        for (
            session_id,
            expected_current_state,
            expected_published_state,
            expected_list,
            expected_owner,
        ) in [
            (
                active.id.as_str(),
                SessionState::Idle,
                SessionState::Active,
                RecoveryPublicationList::SessionList,
                None,
            ),
            (
                closed.id.as_str(),
                SessionState::Closed,
                SessionState::Closed,
                RecoveryPublicationList::ClosedHistory,
                None,
            ),
            (
                archived.id.as_str(),
                SessionState::Archived,
                SessionState::Archived,
                RecoveryPublicationList::ArchivedHistory,
                None,
            ),
            (
                workflow.id.as_str(),
                SessionState::Idle,
                SessionState::Active,
                RecoveryPublicationList::SessionList,
                Some(("sqlite-restart-execution", "sqlite-restart-node")),
            ),
        ] {
            let projection = reopened
                .canonical_session_projection(session_id)
                .unwrap()
                .expect("session projection survives SQLite reopen");
            assert_eq!(projection.meta.state, expected_current_state);
            assert_eq!(projection.meta.agent_session_id, None);
            assert_eq!(projection.meta.provider_session_generation, 1);
            assert_eq!(projection.meta.context_reinjection_generation, None);
            assert_eq!(
                projection.meta.context_carry,
                Some(ContextCarryState::Failed)
            );

            let snapshot = projection
                .meta
                .recovery_publication_snapshot
                .as_ref()
                .expect("recovering projection retains its publication snapshot");
            let recovery_id = format!("restart-recovery-{session_id}");
            assert_eq!(snapshot.recovery_id, recovery_id);
            assert_eq!(snapshot.summary.id, session_id);
            assert_eq!(snapshot.summary.state, expected_published_state);
            let expected_provider_session_id = format!("provider-{session_id}");
            assert_eq!(
                snapshot.summary.agent_session_id.as_deref(),
                Some(expected_provider_session_id.as_str())
            );
            assert_eq!(snapshot.classification.list, expected_list);
            match (
                snapshot.classification.workflow_owner.as_ref(),
                expected_owner,
            ) {
                (None, None) => {}
                (Some(owner), Some((execution_id, node_execution_id))) => {
                    assert_eq!(owner.execution_id.as_deref(), Some(execution_id));
                    assert_eq!(owner.node_execution_id.as_deref(), Some(node_execution_id));
                }
                other => panic!("unexpected recovery publication owner: {other:?}"),
            }
            assert_eq!(
                TurnEventLog::from_events(projection.reducer_events.clone())
                    .project()
                    .backend_recovery,
                Some(BackendSessionRecoveryProjection::Recovering {
                    recovery_id: recovery_id.clone(),
                    old_provider_session_generation: 1,
                    reason: BackendSessionRecoveryReason::BackendSessionLost,
                })
            );

            let obligation = reopened
                .canonical_obligation(&backend_recovery_obligation_id(session_id, &recovery_id))
                .unwrap()
                .expect("recovery effect remains durably reserved");
            assert!(matches!(
                obligation.record,
                crate::domain::local_event::ObligationRecord::BackendSessionRecovery {
                    session_id: stored_session_id,
                    recovery_id: stored_recovery_id,
                    detail:
                        crate::domain::local_event::BackendSessionRecoveryObligationRecord::EffectReserved {
                            old_provider_session_generation: 1,
                            reason: BackendSessionRecoveryReason::BackendSessionLost,
                            ..
                        },
                    state: crate::domain::local_event::ObligationStateRecord::EffectReserved,
                } if stored_session_id == session_id && stored_recovery_id == recovery_id
            ));
        }

        let turn_projection = reopened
            .canonical_session_projection(&turn_lifecycle.id)
            .unwrap()
            .expect("ordinary turn projection survives SQLite reopen");
        assert_eq!(turn_projection.meta.state, SessionState::Idle);
        assert!(turn_projection.meta.recovery_publication_snapshot.is_none());
        assert_eq!(
            TurnEventLog::from_events(turn_projection.reducer_events)
                .project()
                .backend_recovery,
            None
        );
    }

    #[test]
    fn projection_reason_change_notifies_listener_when_state_stays_error() {
        let app_data_dir = tempfile::tempdir().unwrap();
        let store = crate::test_support::build_session_store();
        let session = super::super::create_session_internal(
            &store,
            app_data_dir.path(),
            "/repo",
            Some("codex".to_string()),
        )
        .unwrap();
        let notifications = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let notifications_for_listener = Arc::clone(&notifications);
        store.register_state_change_listener(Arc::new(move |_, _, state, _| {
            notifications_for_listener.lock().push(state.clone());
        }));

        store
            .append_error_episode_and_materialize(
                app_data_dir.path(),
                &session.id,
                ErrorEpisodeInput {
                    message_id: "fatal-1".to_string(),
                    reason: "first fatal".to_string(),
                    at: 1.0,
                },
            )
            .unwrap();
        notifications.lock().clear();
        store
            .append_error_episode_and_materialize(
                app_data_dir.path(),
                &session.id,
                ErrorEpisodeInput {
                    message_id: "fatal-2".to_string(),
                    reason: "latest fatal".to_string(),
                    at: 2.0,
                },
            )
            .unwrap();

        assert_eq!(*notifications.lock(), vec![SessionState::Error]);
        let meta = store
            .get_session_meta(app_data_dir.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(meta.error_reason.as_deref(), Some("latest fatal"));
    }

    #[test]
    fn fork_session_clears_parent_error_reason_from_disk_and_later_error_state() {
        let app_data_dir = tempfile::tempdir().unwrap();
        let store = crate::test_support::build_session_store();
        let parent = super::super::create_session_internal(
            &store,
            app_data_dir.path(),
            "/repo",
            Some("codex".to_string()),
        )
        .unwrap();
        store
            .append_error_episode_and_materialize(
                app_data_dir.path(),
                &parent.id,
                ErrorEpisodeInput {
                    message_id: "fatal-parent".to_string(),
                    reason: "parent fatal".to_string(),
                    at: 1.0,
                },
            )
            .unwrap();

        let fork = store.fork_session(app_data_dir.path(), &parent.id).unwrap();
        let cached_meta = store
            .get_session_meta(app_data_dir.path(), &fork.id)
            .unwrap()
            .unwrap();
        assert_eq!(cached_meta.state, SessionState::Idle);
        assert_eq!(cached_meta.error_reason, None);
        drop(store);

        let reloaded_store = crate::test_support::build_session_store();
        let disk_meta = reloaded_store
            .get_session_meta(app_data_dir.path(), &fork.id)
            .unwrap()
            .unwrap();
        assert_eq!(disk_meta.error_reason, None);

        reloaded_store
            .set_session_state(app_data_dir.path(), &fork.id, SessionState::Error)
            .unwrap();
        let errored = reloaded_store
            .get_session_shell(app_data_dir.path(), &fork.id)
            .unwrap()
            .unwrap();
        assert_eq!(errored.state, SessionState::Error);
        assert_eq!(errored.error_reason, None);
    }

    #[test]
    fn b060_application_shutdown_inventory_uses_canonical_session_ownership_and_state() {
        let app_data_dir = tempfile::tempdir().unwrap();
        let local_store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
            crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
                app_data_dir.path().to_path_buf(),
            ),
        )
        .unwrap();

        let store = crate::test_support::build_session_store();
        let repository: Arc<dyn crate::domain::local_event::LocalEventTransactionRepository> =
            local_store.clone();
        store.set_local_event_repository(
            repository,
            local_store.installation_id().to_string(),
            Arc::new(
                crate::adaptor::gateway::agent_session::session_storage::AgentSessionProjectionCodecV1,
            ),
        );

        let create = |identity: &str,
                      workflow_node_session: bool|
         -> crate::usecase::agent_session::session::ChatSession {
            super::super::create_session_internal_with_attributes(
                &store,
                app_data_dir.path(),
                &format!("/repo/{identity}"),
                Some("codex".to_string()),
                crate::domain::agent_session::PermissionMode::Edit,
                super::super::SessionCreationAttributes {
                    workflow_node_session,
                    ..Default::default()
                },
            )
            .unwrap()
        };

        let active = create("b060-active", false);
        let idle = create("b060-idle", false);
        let closed = create("b060-closed-recovery", false);
        let archived = create("b060-archived-recovery", false);
        let workflow_child = create("b060-workflow-child", true);
        store
            .set_session_state(app_data_dir.path(), &active.id, SessionState::Active)
            .unwrap();
        store
            .set_session_state(app_data_dir.path(), &closed.id, SessionState::Closed)
            .unwrap();
        store
            .set_session_state(app_data_dir.path(), &archived.id, SessionState::Archived)
            .unwrap();

        let mut inventory = store
            .application_shutdown_target_session_ids(app_data_dir.path())
            .unwrap();
        inventory.sort();
        let mut expected = vec![active.id.clone(), idle.id.clone()];
        expected.sort();
        assert_eq!(inventory, expected);
        assert!(!inventory.contains(&closed.id));
        assert!(!inventory.contains(&archived.id));
        assert!(!inventory.contains(&workflow_child.id));
    }
}
