use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::domain::agent_session::aggregates::backend_recovery_projection::{
    BackendRecoveryProjection as DomainBackendRecoveryProjection,
    BackendRecoveryProjectionRejection,
    ProviderSessionEstablishment as DomainProviderSessionEstablishment,
};
use crate::domain::agent_session::aggregates::session::{
    QueueStartRejection, Session as SessionAggregate, TransitionOutcome, TransitionRejection,
};
use crate::domain::agent_session::entities::Turn;
use crate::domain::agent_session::services::{
    add_durable_count, admit_backend_recovery_completion, admit_backend_recovery_failure,
    admit_backend_recovery_start, advance_durable_counter, allocate_next_turn_identity,
    backend_recovery_error_digest, backend_recovery_failure_message_id,
    backend_recovery_obligation_id, backend_recovery_provider_observation_id,
    context_restore_completion_is_settled, decide_backend_recovery_durable_completion,
    decide_context_restore_completion, decide_recovery_publication,
    decide_recovery_publication_commit, decide_session_fork, recovery_publication_obligation_id,
    turn_identity_advances, BackendRecoveryDurableCompletionDecision,
    BackendRecoveryDurableCompletionFacts, BackendRecoveryDurableCompletionRejection,
    BackendRecoveryReservationDecision, BackendRecoveryReservationRejection,
    ContextRestoreCompletionCommand, ContextRestoreCompletionFacts,
    ContextRestoreCompletionRejection, RecoveryPublicationCommitDecision,
    RecoveryPublicationCommitFacts, RecoveryPublicationCommitRejection,
    RecoveryPublicationDecision, ReservedTurnIdentity, TurnIdentityAllocationError,
    UserSessionMetadataAction,
};
#[cfg(test)]
use crate::domain::agent_session::AgentSessionStorage;
use crate::domain::agent_session::{
    AgentSessionProjectedMessage, AgentSessionProjectionCommit, AgentSessionReader,
    AgentSessionStorageTypes,
};
use crate::domain::local_event::{
    hex_lower, runtime_terminal_identity, workflow_turn_completion_ordered_key_prefix,
    DurableIdentityBuilder, WorkflowTurnCompletionIdentityFacts,
};
use crate::domain::path::same_worktree_path;
use crate::usecase::agent_session::context_meta::ContextEpochMeta;
#[cfg(test)]
use crate::usecase::agent_session::event_log::BackendSessionRecoveryProjection;
use crate::usecase::agent_session::event_log::{
    latest_turn_interruption, AgentSessionEvent, AgentTurnFailureSignal,
    BackendSessionRecoveryReason, GoalReactivationOutcome, SessionReadModel, TurnEventLog,
    WorkflowTurnCompleteInput,
};
use parking_lot::RwLock;

use super::{
    error_reason_for_state, now_timestamp, ChatMessage, ChatSession, ContextCarryState,
    MessagePageMetadata, MessagePart, MessageRole, PageCursor, PendingRecoveryMessage,
    RecoveryPublicationSnapshot, SessionAttachment, SessionMeta, SessionPage, SessionReviewContext,
    SessionState, SessionSummary, SessionToolOutput, TokenUsage, TurnInterruption,
    WorkflowNodeContextDto,
};
#[cfg(test)]
use super::{
    RecoveryPublicationClassification, RecoveryPublicationList, RecoveryPublicationWorkflowOwner,
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

impl From<TurnIdentityAllocationError> for NextTurnIdError {
    fn from(value: TurnIdentityAllocationError) -> Self {
        match value {
            TurnIdentityAllocationError::InvalidReservedIdentity { queue_item_id } => {
                Self::Unavailable(format!(
                    "queued send {queue_item_id} has an invalid reserved turn identity"
                ))
            }
            TurnIdentityAllocationError::NonAdvancingReservedIdentity { queue_item_id } => {
                Self::Unavailable(format!(
                    "queued send {queue_item_id} does not advance the canonical turn identity"
                ))
            }
            TurnIdentityAllocationError::CapacityExceeded => Self::CapacityExceeded,
        }
    }
}

fn next_sqlite_counter(value: u64, label: &str) -> Result<u64, String> {
    advance_durable_counter(value).map_err(|_| format!("{label} capacity is exhausted"))
}

fn add_sqlite_count(value: usize, delta: usize, label: &str) -> Result<usize, String> {
    add_durable_count(value, delta).map_err(|_| format!("{label} capacity is exhausted"))
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
    pub workflow_context: crate::domain::workflow::WorkflowNodeContext,
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
pub(crate) struct CanonicalContentBlob {
    pub identity: String,
    pub projection: crate::domain::local_event::AgentContentBlobRecord,
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

fn reducer_has_active_turn(events: &[AgentSessionEvent]) -> bool {
    SessionAggregate::current_turn_from_events(events, false).is_some()
}

fn reducer_allows_queue_start(state: &SessionState, events: &[AgentSessionEvent]) -> bool {
    SessionAggregate::projection_allows_queue_start(*state, events)
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
    previous: Vec<AgentSessionEvent>,
    appended: &[AgentSessionEvent],
) -> Vec<AgentSessionEvent> {
    SessionAggregate::bounded_reducer_events(previous, appended)
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

    fn restore_session_aggregate(
        &self,
        projection: &CanonicalAgentSessionProjection,
        pending_obligations: &[(String, crate::domain::local_event::ObligationRecord)],
    ) -> Result<SessionAggregate, String>;

    fn encode_message(
        &self,
        message: &ChatMessage,
    ) -> Result<crate::domain::local_event::MessageProjectionRecord, String>;

    fn decode_message(
        &self,
        payload: &crate::domain::local_event::MessageProjectionRecord,
    ) -> Result<ChatMessage, String>;

    fn externalize_message_content(
        &self,
        messages: &mut [ChatMessage],
    ) -> Result<Vec<CanonicalContentBlob>, String>;

    fn backend_recovery_from_projection(
        &self,
        projection: &CanonicalAgentSessionProjection,
    ) -> DomainBackendRecoveryProjection;

    fn backend_recovery_from_meta(
        &self,
        meta: &SessionMeta,
        queue_paused: bool,
    ) -> DomainBackendRecoveryProjection;

    fn apply_backend_recovery_to_projection(
        &self,
        projection: &mut CanonicalAgentSessionProjection,
        state: DomainBackendRecoveryProjection,
    );

    fn apply_backend_recovery_to_meta(
        &self,
        meta: &mut SessionMeta,
        state: DomainBackendRecoveryProjection,
    );

    fn recovery_publication_snapshot(
        &self,
        recovery_id: &str,
        meta: &SessionMeta,
        decision: RecoveryPublicationDecision,
    ) -> RecoveryPublicationSnapshot;

    fn recovery_publication_message_record(
        &self,
        message: &PendingRecoveryMessage,
    ) -> crate::domain::local_event::RecoveryPublicationMessageRecord;

    fn workflow_context(
        &self,
        context: &WorkflowNodeContextDto,
    ) -> crate::domain::workflow::WorkflowNodeContext;

    fn workflow_failure_signal(
        &self,
        signal: Option<AgentTurnFailureSignal>,
    ) -> Option<crate::domain::local_event::WorkflowTurnFailureSignalRecord>;

    fn workflow_turn_complete_input(
        &self,
        pending: &crate::domain::local_event::ValidatedPendingWorkflowTurnCompletion,
        final_text_parts: Vec<String>,
    ) -> WorkflowTurnCompleteInput;

    fn workflow_final_text_parts(
        &self,
        message: &ChatMessage,
        expected_message_id: &str,
    ) -> Result<Vec<String>, String>;

    fn context_restore_completion_facts(&self, meta: &SessionMeta)
        -> ContextRestoreCompletionFacts;

    fn apply_context_restore_completion_decision(
        &self,
        meta: &mut SessionMeta,
        decision: crate::domain::agent_session::services::ContextRestoreCompletionDecision,
        at: f64,
    );

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

    fn hash_terminal_message_projection_patch(
        &self,
        identity: &mut DurableIdentityBuilder,
        patch: &TerminalMessageProjectionPatch,
    ) -> Result<(), String>;

    fn hash_event_projection_meta_patch(
        &self,
        identity: &mut DurableIdentityBuilder,
        patch: &EventProjectionMetaPatch,
    ) -> Result<(), String>;
}

#[derive(Debug, Clone)]
pub(crate) struct TerminalMessageProjectionPatch {
    pub(crate) message_id: String,
    pub(crate) streaming_final_seq: u64,
    pub(crate) timestamp: Option<f64>,
    pub(crate) parts: Option<Vec<MessagePart>>,
}

fn terminal_requires_queue_pause(events: &[AgentSessionEvent]) -> bool {
    SessionAggregate::terminal_requires_queue_pause(events)
}

fn complete_terminal_projection_events(
    previous: &[AgentSessionEvent],
    supplied: &[AgentSessionEvent],
) -> Vec<AgentSessionEvent> {
    SessionAggregate::converge_terminal_events(previous, supplied, |events, message_id| {
        TurnEventLog::from_events(events.to_vec())
            .project()
            .agent_parts_for_message(message_id)
    })
}

#[cfg(test)]
fn test_recovery_publication_snapshot(
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

pub(crate) enum BackendSessionRecoveryStartOutcome {
    Started(Box<SessionMeta>),
    SuppressedByQueuePause,
}

pub(crate) enum ProviderSessionEstablishmentOutcome {
    Settled(Box<SessionMeta>),
    Missing,
    Fenced,
}

pub(crate) type ContextRestoreCompletionRequest = ContextRestoreCompletionCommand;

const BACKEND_RECOVERY_START_SUPPRESSED_BY_QUEUE_PAUSE: &str =
    "backend recovery start was suppressed by a durable queue pause";
const CONTEXT_RESTORE_COMPLETION_FENCED: &str =
    "context restore completion was fenced by newer durable session state";
const CONTEXT_RESTORE_COMPLETION_UNCHANGED: &str =
    "context restore completion is already reflected in durable session state";

#[cfg(test)]
fn test_context_restore_completion_facts(meta: &SessionMeta) -> ContextRestoreCompletionFacts {
    ContextRestoreCompletionFacts {
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

#[derive(Debug, Clone)]
pub(crate) enum EventProjectionMetaPatch {
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

fn apply_context_restore_completion_to_meta(
    codec: Option<&dyn AgentSessionProjectionCodec>,
    meta: &mut SessionMeta,
    command: ContextRestoreCompletionCommand,
    at: f64,
) -> Result<bool, String> {
    let facts = match codec {
        Some(codec) => codec.context_restore_completion_facts(meta),
        #[cfg(test)]
        None => test_context_restore_completion_facts(meta),
        #[cfg(not(test))]
        None => unreachable!("production mutation admission requires a projection codec"),
    };
    let decision =
        decide_context_restore_completion(facts, command).map_err(|rejection| match rejection {
            ContextRestoreCompletionRejection::Fenced => {
                CONTEXT_RESTORE_COMPLETION_FENCED.to_string()
            }
            ContextRestoreCompletionRejection::Unchanged => {
                CONTEXT_RESTORE_COMPLETION_UNCHANGED.to_string()
            }
        })?;
    match codec {
        Some(codec) => codec.apply_context_restore_completion_decision(meta, decision, at),
        #[cfg(test)]
        None => {
            if decision.clear_context_reinjection_generation {
                meta.context_reinjection_generation = None;
            }
            if let crate::domain::agent_session::services::ContextCarryChange::Replace(
                context_carry,
            ) = decision.context_carry
            {
                meta.context_carry = context_carry;
            }
            meta.updated_at = at;
        }
        #[cfg(not(test))]
        None => unreachable!("production mutation admission requires a projection codec"),
    }
    Ok(true)
}

#[cfg(test)]
fn test_domain_backend_recovery_meta(
    meta: &SessionMeta,
    queue_paused: bool,
) -> DomainBackendRecoveryProjection {
    DomainBackendRecoveryProjection {
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

#[cfg(test)]
fn test_apply_domain_backend_recovery_meta(
    meta: &mut SessionMeta,
    state: DomainBackendRecoveryProjection,
) {
    meta.state = state.session_state;
    meta.error_reason = state.error_reason;
    meta.agent_session_id = state.provider_session_id;
    meta.provider_session_generation = state.provider_session_generation;
    meta.provider_session_observation_id = state.provider_session_observation_id;
    meta.context_reinjection_generation = state.context_reinjection_generation;
    meta.context_carry = state.context_carry;
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
            let actual_generation = decoded.meta.provider_session_generation;
            let mut state = codec.backend_recovery_from_projection(&decoded);
            state
                .start(
                    *expected_generation,
                    publication_snapshot.summary.state,
                    publication_snapshot.summary.error_reason.clone(),
                )
                .map_err(|rejection| match rejection {
                    BackendRecoveryProjectionRejection::QueuePaused => {
                        BACKEND_RECOVERY_START_SUPPRESSED_BY_QUEUE_PAUSE.to_string()
                    }
                    BackendRecoveryProjectionRejection::StaleProviderGeneration => format!(
                        "Backend session generation changed while starting recovery: expected {expected_generation}, actual {actual_generation}"
                    ),
                    BackendRecoveryProjectionRejection::ProviderGenerationExhausted
                    | BackendRecoveryProjectionRejection::DurableEvidenceMismatch
                    | BackendRecoveryProjectionRejection::InvalidObservationIdentity
                    | BackendRecoveryProjectionRejection::ConflictingProviderIdentity => {
                        "backend recovery start decision is inconsistent".to_string()
                    }
                })?;
            // The domain projection preserves closed/archived lifecycle from
            // the publication snapshot instead of letting a recovery-only
            // event infer an open Idle state.
            codec.apply_backend_recovery_to_projection(&mut decoded, state);
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
            let actual_generation = decoded.meta.provider_session_generation;
            let publication_message =
                codec.recovery_publication_message_record(pending_recovery_message);
            let mut state = codec.backend_recovery_from_projection(&decoded);
            state
                .complete(
                    *expected_generation,
                    *provider_session_generation,
                    backend_session_id.clone(),
                    backend_recovery_provider_observation_id(
                        &publication_message.recovery_id,
                    ),
                )
                .map_err(|rejection| match rejection {
                    BackendRecoveryProjectionRejection::StaleProviderGeneration => format!(
                        "Backend session generation changed while completing recovery: expected {expected_generation}, actual {actual_generation}"
                    ),
                    _ => "backend recovery completion decision is inconsistent".to_string(),
                })?;
            codec.apply_backend_recovery_to_projection(&mut decoded, state);
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
            let publication_message =
                codec.recovery_publication_message_record(pending_recovery_message);
            let mut state = codec.backend_recovery_from_projection(&decoded);
            state
                .complete_from_readback(
                    *old_provider_session_generation,
                    *provider_session_generation,
                    backend_session_id,
                    backend_recovery_provider_observation_id(&publication_message.recovery_id),
                )
                .map_err(|rejection| match rejection {
                    BackendRecoveryProjectionRejection::ProviderGenerationExhausted => {
                        "provider session generation is exhausted".to_string()
                    }
                    _ => {
                        "durable backend recovery owner evidence no longer matches its reservation"
                            .to_string()
                    }
                })?;
            codec.apply_backend_recovery_to_projection(&mut decoded, state);
            decoded.meta.pending_recovery_message = Some(pending_recovery_message.clone());
            decoded.meta.recovery_publication_snapshot = None;
            decoded.meta.updated_at = *at;
        }
        EventProjectionMetaPatch::Failed {
            pending_recovery_message,
            at,
        } => {
            let mut state = codec.backend_recovery_from_projection(&decoded);
            state.fail(match pending_recovery_message {
                PendingRecoveryMessage::Error { error, .. } => Some(error.clone()),
                PendingRecoveryMessage::Notice { .. } => None,
            });
            codec.apply_backend_recovery_to_projection(&mut decoded, state);
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
            Some(codec),
            &mut decoded.meta,
            ContextRestoreCompletionCommand {
                expected_provider_session_generation: *expected_provider_session_generation,
                expected_turn_id: *expected_turn_id,
                reinjected: *reinjected,
                clear_context_carry: *clear_context_carry,
                recovery_restore_required: *recovery_restore_required,
            },
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
    meta.state = projected.status.session_state;
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
            total_tokens: usage.total_tokens(),
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
    let content_blobs = authority
        .projection_codec
        .externalize_message_content(&mut projected_messages)?;
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
    let identity = runtime_terminal_identity(
        session_id,
        turn_id,
        message_id,
        streaming_final_seq,
        completed_at.to_bits(),
        encoded_events,
        turn_result,
    );
    Ok(
        crate::domain::local_event::LocalStateMutation::TerminalRecord(
            crate::domain::local_event::TerminalRecordMutation {
                session_id: session_id.to_string(),
                turn_id: turn_id.to_string(),
                terminal_identity: identity.terminal_identity,
                result: crate::domain::local_event::TerminalResultRecord::AgentTurn {
                    kind: identity.terminal_kind,
                    session_id: session_id.to_string(),
                    turn_id: turn_id.to_string(),
                    message_id: message_id.to_string(),
                    streaming_final_sequence: streaming_final_seq,
                    completed_at_bits: completed_at.to_bits(),
                    result: crate::domain::local_event::AgentTurnTerminalResultRecord::Current(
                        turn_result.clone(),
                    ),
                },
                participant_digest: identity.participant_digest,
            },
        ),
    )
}

fn workflow_turn_completion_pending_mutation(
    codec: &dyn AgentSessionProjectionCodec,
    session_id: &str,
    workflow_context: &WorkflowNodeContextDto,
    terminal: &crate::domain::local_event::TerminalRecordMutation,
    message_id: &str,
    input: &WorkflowTurnCompleteInput,
) -> Result<crate::domain::local_event::LocalStateMutation, String> {
    let workflow_context = codec.workflow_context(workflow_context);
    let failure_signal = codec.workflow_failure_signal(input.failure_signal);
    let identity = crate::domain::local_event::decide_workflow_turn_completion_identity(
        terminal,
        WorkflowTurnCompletionIdentityFacts {
            session_id,
            workflow_context: &workflow_context,
            terminal_identity: &terminal.terminal_identity,
            message_id,
            turn_id: input.turn_id,
            exit_code: input.exit_code,
            final_text_parts: &input.final_text_parts,
            failure_signal,
            token_usage: input.token_usage,
            interrupted: input.interrupted,
        },
    )
    .map_err(|_| "workflow turn-completion terminal identity is inconsistent".to_string())?;
    let record = crate::domain::local_event::ObligationRecord::WorkflowTurnCompletion {
        session_id: session_id.to_string(),
        turn_id: input.turn_id.to_string(),
        terminal_identity: terminal.terminal_identity.clone(),
        notification_sha256: identity.notification_digest,
        detail: crate::domain::local_event::WorkflowTurnCompletionObligationRecord::Pending {
            workflow_context: Box::new(workflow_context),
            message_id: message_id.to_string(),
            exit_code: input.exit_code,
            failure_signal,
            token_usage: input.token_usage,
            interrupted: input.interrupted,
        },
        state: crate::domain::local_event::ObligationStateRecord::Pending,
    };
    Ok(crate::domain::local_event::LocalStateMutation::Obligation(
        crate::domain::local_event::ObligationMutation {
            obligation_id: identity.obligation_id,
            record,
            pending: Some(crate::domain::local_event::PendingIndexEntry {
                ordered_key: identity.ordered_key,
                owner: session_id.to_string(),
                partition: crate::domain::local_event::PendingPartition::Owner,
                shutdown_plan: None,
            }),
            expected: crate::domain::local_event::RevisionGuard::Absent,
            revision: crate::domain::local_event::Revision::new(0).expect("zero revision"),
        },
    ))
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
        self.read_session_metadata_inventory(app_data_dir)
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

include!("repository_core.rs");
include!("mutation_preparation.rs");
include!("queries.rs");
include!("event_projection.rs");
include!("operation_state.rs");
include!("persistence.rs");
#[cfg(test)]
include!("tests.rs");
