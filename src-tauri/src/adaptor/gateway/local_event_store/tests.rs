//! Store-level tests: B-050 atomicity and fault matrix, capacity and signed
//! 64-bit boundaries, unknown-event raw preservation, cursor integration,
//! startup validation, and query plan snapshots.

use std::sync::Arc;
use std::time::Instant;

use sha2::Digest;
use tempfile::TempDir;

use crate::adaptor::gateway::agent_session::session_storage::{
    AgentSessionProjectionCodecV1, FileSessionStorage,
};
use crate::adaptor::gateway::local_event_store::agent_session_codec::AgentSessionEventCodec;
use crate::adaptor::gateway::local_event_store::canonical_cbor::CborValue;
use crate::adaptor::gateway::local_event_store::clock::FakeStoreClock;
use crate::adaptor::gateway::local_event_store::commit::SQL_SEAL_EVENT_COUNT;
use crate::adaptor::gateway::local_event_store::connection::open_reader;
use crate::adaptor::gateway::local_event_store::envelope::{
    EventCodecError, EventCodecRegistry, LocalEventPayloadCodec, StoredUnknownEvent,
};
use crate::adaptor::gateway::local_event_store::fault::FaultInjector;
use crate::adaptor::gateway::local_event_store::layout::StoreLayout;
use crate::adaptor::gateway::local_event_store::reader::{
    read_envelope, READ_QUEUE_MAX_DEPTH, SQL_OPERATION_LOOKUP, SQL_PENDING_FIRST_PAGE,
    SQL_PENDING_FIRST_PAGE_PARTITION, SQL_PENDING_FIRST_PAGE_PREFIX, SQL_TERMINAL_LOOKUP,
};
use crate::adaptor::gateway::local_event_store::state_record_codec::{
    canonicalize_recovery_result_record, StoredObligationV1, StoredOperationReceiptV1,
    StoredOperationStatusV1, StoredShutdownPlanV1, StoredShutdownTargetV1, StoredTerminalV1,
};
use crate::adaptor::gateway::local_event_store::store::{LocalEventStore, LocalEventStoreConfig};
use crate::adaptor::gateway::local_event_store::writer::{
    MAX_BATCH_EVENTS, MAX_BATCH_STATE_MUTATIONS,
};
use crate::domain::agent_session::events::{
    AgentSessionDomainEvent, BackendSessionRecoveryReason, RecoveryActionKind,
    RecoveryResultClassification, SessionLifecycleKind,
};
use crate::domain::local_event::{
    AgentContentBlobRecord, AgentMessageProjectionRecord, AgentMessageRoleRecord,
    AgentSessionMetadataRecord, AgentSessionProjectionRecord, AgentSessionStateRecord,
    ApplicationDomainEvent, ApplicationShutdownPhase, CallerOperationKey, CommitBatchError,
    CommitBatchResult, CommitIdentity, CommitOperationKind, CommitResolution, ExpectedStreamHead,
    IdempotencyBinding, LoadStreamRequest, LoadedDomainEvent, LocalAtomicBatch, LocalDomainEvent,
    LocalEventQuery, LocalEventQueryError, LocalEventQueryResult, LocalEventTransactionRepository,
    LocalStateMutation, MessageProjectionMutation, MessageProjectionRecord, ObligationMutation,
    ObligationRecord, ObligationStateRecord, OperationBindingMutation, OperationKind,
    OperationReceiptRecord, OperationRecordMutation, OperationStatusRecord, OperationStatusValue,
    PendingIndexEntry, PendingPartition, QueryCursor, QuitIntent, RecordAuthentication,
    RecoveryActionMutation, RecoveryAttemptRecord, RecoveryResourceViewRecord,
    RecoveryResultOutcomeRecord, Revision, RevisionGuard, SafeEffectObservation,
    SafeOperationFailure, SessionLifecycleRecordAction, SessionOperationFailureKind,
    SessionProjectionMutation, SessionProjectionRecord, SessionProjectionRemovalMutation,
    ShutdownDetailsState, ShutdownLatestPointerMutation, ShutdownPlanKey, ShutdownPlanMutation,
    ShutdownPlanRecord, ShutdownRecoverySnapshotMutation, ShutdownTargetKindRecord,
    ShutdownTargetMutation, ShutdownTargetRecord, ShutdownTargetRecoveryRecord,
    ShutdownTargetStateRecord, StreamId, StreamVersion, TerminalRecordMutation,
    TerminalResultRecord, UncommittedDomainEvent, WorkflowExecutionMetadataRecord,
    WorkflowExecutionProjectionRecord,
};
use crate::domain::workflow::{
    ExecutionOrigin, ExecutionStatus, TokenUsage as WorkflowTokenUsage, WorkflowDomainEvent,
};
use crate::usecase::agent_session::feedback::{
    FeedbackAction, FeedbackError, FeedbackResolutionPort, FeedbackRetryOutcome,
    SessionFeedbackEntry, SessionFeedbackUsecase,
};
use crate::usecase::agent_session::notice::AgentSessionNoticeOperation;
use crate::usecase::agent_session::session::{
    build_new_session_with_id, ChatMessage, MessageRole, PendingRecoveryMessage, SessionStore,
};
use crate::usecase::agent_session::session_feedback_load::{
    SessionFeedbackLoadError, SessionFeedbackLoadUsecase, SessionLoadPort,
};

/// Test codec for one constructible agent-session event so multi-stream
/// batches can be exercised without the production agent-session codecs
/// (registered by the tasks that route those events through the store).
struct TestAgentSessionCodec;

impl LocalEventPayloadCodec for TestAgentSessionCodec {
    fn event_type(&self) -> &'static str {
        "test.agent_session.recovery_started"
    }

    fn payload_version(&self) -> i64 {
        1
    }

    fn handles(&self, event: &LocalDomainEvent) -> bool {
        matches!(
            event,
            LocalDomainEvent::AgentSession(
                AgentSessionDomainEvent::BackendSessionRecoveryStarted { .. }
            )
        )
    }

    fn encode(&self, event: &LocalDomainEvent) -> Result<CborValue, EventCodecError> {
        let LocalDomainEvent::AgentSession(
            AgentSessionDomainEvent::BackendSessionRecoveryStarted {
                recovery_id,
                old_provider_session_generation,
                reason,
                at,
            },
        ) = event
        else {
            return Err(EventCodecError::UnregisteredEvent {
                description: "unsupported test event".to_string(),
            });
        };
        Ok(CborValue::Map(vec![
            (
                CborValue::Text("recovery_id".to_string()),
                CborValue::Text(recovery_id.clone()),
            ),
            (
                CborValue::Text("generation".to_string()),
                CborValue::Unsigned(*old_provider_session_generation),
            ),
            (
                CborValue::Text("reason".to_string()),
                CborValue::Text(
                    match reason {
                        BackendSessionRecoveryReason::ResumeMismatch => "resume_mismatch",
                        BackendSessionRecoveryReason::BackendSessionLost => "backend_session_lost",
                    }
                    .to_string(),
                ),
            ),
            (
                CborValue::Text("at_ms".to_string()),
                CborValue::int(*at as i64),
            ),
        ]))
    }

    fn decode(
        &self,
        payload_version: i64,
        value: &CborValue,
    ) -> Result<Option<LocalDomainEvent>, EventCodecError> {
        if payload_version != 1 {
            return Ok(None);
        }
        let malformed = || EventCodecError::MalformedPayload {
            event_type: self.event_type().to_string(),
        };
        let CborValue::Map(entries) = value else {
            return Err(malformed());
        };
        let get_text = |key: &str| {
            entries
                .iter()
                .find_map(|(entry_key, entry_value)| match (entry_key, entry_value) {
                    (CborValue::Text(text), CborValue::Text(inner)) if text == key => {
                        Some(inner.clone())
                    }
                    _ => None,
                })
        };
        let generation = entries
            .iter()
            .find_map(|(key, value)| match (key, value) {
                (CborValue::Text(text), CborValue::Unsigned(n)) if text == "generation" => Some(*n),
                _ => None,
            })
            .ok_or_else(malformed)?;
        let at_ms = entries
            .iter()
            .find_map(|(key, value)| match key {
                CborValue::Text(text) if text == "at_ms" => value.as_i64(),
                _ => None,
            })
            .ok_or_else(malformed)?;
        let reason = match get_text("reason").ok_or_else(malformed)?.as_str() {
            "resume_mismatch" => BackendSessionRecoveryReason::ResumeMismatch,
            "backend_session_lost" => BackendSessionRecoveryReason::BackendSessionLost,
            _ => return Err(malformed()),
        };
        Ok(Some(LocalDomainEvent::AgentSession(
            AgentSessionDomainEvent::BackendSessionRecoveryStarted {
                recovery_id: get_text("recovery_id").ok_or_else(malformed)?,
                old_provider_session_generation: generation,
                reason,
                at: at_ms as f64,
            },
        )))
    }
}

fn test_registry() -> Arc<EventCodecRegistry> {
    let mut registry = EventCodecRegistry::new();
    registry.register(Arc::new(TestAgentSessionCodec));
    Arc::new(registry)
}

const TEST_INSTALLATION_ID: &str = "11111111-1111-4111-8111-111111111111";

struct Harness {
    _dir: TempDir,
    root: std::path::PathBuf,
    store: Arc<LocalEventStore>,
    clock: FakeStoreClock,
    fault: Arc<FaultInjector>,
}

impl Harness {
    fn open() -> Self {
        Self::open_with_registry(test_registry())
    }

    fn open_with_registry(registry: Arc<EventCodecRegistry>) -> Self {
        let dir = TempDir::new().expect("temp app data");
        let root = dir.path().to_path_buf();
        let clock = FakeStoreClock::at(1_000);
        let fault = Arc::new(FaultInjector::new());
        fault.set_initial_installation_id(TEST_INSTALLATION_ID);
        let store = LocalEventStore::open(LocalEventStoreConfig {
            app_data_root: root.clone(),
            clock: Arc::new(clock.clone()),
            registry,
            fault: Arc::clone(&fault),
            path_observer: Arc::new(
                crate::adaptor::gateway::local_event_store::layout::NoopStorePathObserver,
            ),
        })
        .expect("open store");
        assert_eq!(store.installation_id(), TEST_INSTALLATION_ID);
        Self {
            _dir: dir,
            root,
            store,
            clock,
            fault,
        }
    }

    fn database_path(&self) -> std::path::PathBuf {
        StoreLayout::new(&self.root).database_path()
    }

    /// Direct maintenance connection for boundary setup and plan snapshots.
    fn raw_connection(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(self.database_path()).expect("raw connection")
    }
}

fn application_event(at_ms: i64) -> UncommittedDomainEvent {
    UncommittedDomainEvent {
        stream_id: StreamId::application(),
        event: LocalDomainEvent::Application(ApplicationDomainEvent::ApplicationQuitAccepted {
            quit_operation_id: "quit-1".to_string(),
            intent: QuitIntent::Exit { code: 0 },
            at_ms,
        }),
        occurred_at_ms: at_ms,
    }
}

fn session_event(session: &str, at_ms: i64) -> UncommittedDomainEvent {
    UncommittedDomainEvent {
        stream_id: StreamId::agent_session(session).unwrap(),
        event: LocalDomainEvent::AgentSession(
            AgentSessionDomainEvent::BackendSessionRecoveryStarted {
                recovery_id: format!("rec-{session}"),
                old_provider_session_generation: 1,
                reason: BackendSessionRecoveryReason::ResumeMismatch,
                at: at_ms as f64,
            },
        ),
        occurred_at_ms: at_ms,
    }
}

fn queue_pause_event(session: &str, at_ms: i64) -> UncommittedDomainEvent {
    UncommittedDomainEvent {
        stream_id: StreamId::agent_session(session).unwrap(),
        event: LocalDomainEvent::AgentSession(AgentSessionDomainEvent::QueuePaused {
            at: at_ms as f64,
        }),
        occurred_at_ms: at_ms,
    }
}

fn workflow_event(execution_id: &str, at_ms: i64) -> UncommittedDomainEvent {
    UncommittedDomainEvent {
        stream_id: StreamId::workflow(execution_id).unwrap(),
        event: LocalDomainEvent::Workflow(WorkflowDomainEvent::WorkflowExecutionAborted {
            execution_id: execution_id.to_string(),
            aborted_node: Some("review".to_string()),
            timestamp: at_ms as f64,
        }),
        occurred_at_ms: at_ms,
    }
}

trait FixturePayload: Sized {
    fn decode_fixture(text: &str) -> Self;
}

macro_rules! closed_fixture_payload {
    ($value:ty, $stored:ty) => {
        impl FixturePayload for $value {
            fn decode_fixture(text: &str) -> Self {
                <$stored>::decode(text)
                    .unwrap_or_else(|error| {
                        panic!("invalid {:?} fixture: {error:?}", stringify!($value))
                    })
                    .into_value()
            }
        }
    };
}

closed_fixture_payload!(TerminalResultRecord, StoredTerminalV1);
closed_fixture_payload!(ObligationRecord, StoredObligationV1);
closed_fixture_payload!(OperationReceiptRecord, StoredOperationReceiptV1);
closed_fixture_payload!(OperationStatusRecord, StoredOperationStatusV1);
closed_fixture_payload!(ShutdownPlanRecord, StoredShutdownPlanV1);
closed_fixture_payload!(ShutdownTargetRecord, StoredShutdownTargetV1);

fn payload<T: FixturePayload>(text: &str) -> T {
    T::decode_fixture(text)
}

fn agent_session_projection(session_id: &str) -> SessionProjectionRecord {
    SessionProjectionRecord::AgentSession(Box::new(AgentSessionProjectionRecord {
        meta: AgentSessionMetadataRecord {
            id: session_id.to_string(),
            worktree_path: "/tmp/releash-test".to_string(),
            state: AgentSessionStateRecord::Active,
            error_reason: None,
            state_revision: 0,
            created_at_bits: 1.0_f64.to_bits(),
            updated_at_bits: 1.0_f64.to_bits(),
            agent_session_id: None,
            provider_session_generation: 0,
            provider_session_observation_id: None,
            context_reinjection_generation: None,
            context_carry: None,
            pending_recovery_message: None,
            recovery_publication_snapshot: None,
            permission_mode: "ask".to_string(),
            plan_mode: false,
            selected_model: None,
            permission_profile_id: None,
            backend_id: "claude".to_string(),
            workflow_node_session: false,
            workflow_node_context: None,
            workflow_instructions: Vec::new(),
            agent_read_paths: None,
            context_epoch: None,
            last_turn_interruption: None,
            last_turn_id: None,
            first_message_preview: String::new(),
            message_count: 0,
            body_format_version: 1,
        },
        title: None,
        reducer_events: Vec::new(),
        queue_paused_at_bits: None,
        latest_token_usage: None,
        pending_send_queue: Vec::new(),
    }))
}

fn agent_message_projection(message_id: &str, content: &str) -> MessageProjectionRecord {
    MessageProjectionRecord::AgentMessage(AgentMessageProjectionRecord {
        id: message_id.to_string(),
        role: AgentMessageRoleRecord::Agent,
        content: content.to_string(),
        thinking: None,
        activities: None,
        parts: None,
        streaming_final_seq: 0,
        timestamp_bits: 1.0_f64.to_bits(),
        mentions: None,
    })
}

fn workflow_execution_projection(
    execution_id: &str,
    status: ExecutionStatus,
) -> SessionProjectionRecord {
    SessionProjectionRecord::WorkflowExecution(WorkflowExecutionProjectionRecord::Present(
        WorkflowExecutionMetadataRecord {
            execution_id: execution_id.to_string(),
            workflow_name: "test-workflow".to_string(),
            status,
            worktree_path: "/tmp/releash-workflow".to_string(),
            current_node: None,
            created_from: ExecutionOrigin::DesktopUi,
            started_at_bits: 1.0_f64.to_bits(),
            updated_at_bits: 1.0_f64.to_bits(),
            completed_at_bits: None,
            error_reason: None,
            interruption_reason: None,
            resume_from_node: None,
            total_token_usage: WorkflowTokenUsage::default(),
        },
    ))
}

fn workflow_execution_obligation(execution_id: &str) -> ObligationRecord {
    ObligationRecord::WorkflowExecution {
        execution: WorkflowExecutionMetadataRecord {
            execution_id: execution_id.to_string(),
            workflow_name: "fixture-workflow".to_string(),
            status: ExecutionStatus::Running,
            worktree_path: "/fixture".to_string(),
            current_node: None,
            created_from: ExecutionOrigin::DesktopUi,
            started_at_bits: 0.0_f64.to_bits(),
            updated_at_bits: 0.0_f64.to_bits(),
            completed_at_bits: None,
            error_reason: None,
            interruption_reason: None,
            resume_from_node: None,
            total_token_usage: WorkflowTokenUsage::default(),
        },
    }
}

fn shutdown_plan_fixture(operation_id: &str) -> ShutdownPlanRecord {
    ShutdownPlanRecord {
        operation_id: operation_id.to_string(),
        intent: QuitIntent::Exit { code: 0 },
        t0_ms: 0,
        preparation_cutoff_ms: None,
        deadline_ms: 15_000,
        target_count: None,
        prepared_count: None,
        effect_reserved_count: None,
        terminal_count: None,
        completed_count: None,
        unresolved_count: None,
        recovery_snapshot_count: None,
        recovery_snapshot_id: None,
        process_instance_id: "fixture-boot".to_string(),
        outcome: None,
        failure: None,
        shutdown_effect_count: None,
        admission_open: None,
        retry_quit_same_boot: None,
    }
}

fn send_obligation_fixture(obligation_id: &str, state: ObligationStateRecord) -> ObligationRecord {
    ObligationRecord::Send {
        obligation_id: obligation_id.to_string(),
        operation_id: format!("operation-{obligation_id}"),
        session_id: "fixture-session".to_string(),
        kind: crate::domain::local_event::SendObligationKindRecord::TurnExecution,
        disposition: crate::domain::local_event::SendObligationDispositionRecord::StartedTurn,
        human_message_id: None,
        assistant_message_id: None,
        reserved_turn_id: None,
        turn_id: None,
        dependency_obligation_ids: Vec::new(),
        canonical_payload: String::new(),
        state,
    }
}

fn terminal_mutation(session: &str, turn: &str) -> LocalStateMutation {
    LocalStateMutation::TerminalRecord(TerminalRecordMutation {
        session_id: session.to_string(),
        turn_id: turn.to_string(),
        terminal_identity: format!("terminal-{session}-{turn}"),
        result: payload(
            &serde_json::json!({
                "schema": "agent_turn_terminal_v1",
                "terminal_kind": "completed",
                "session_id": session,
                "turn_id": turn,
                "message_id": format!("message-{session}-{turn}"),
                "streaming_final_seq": "0",
                "completed_at_bits": "0",
                "turn_result": { "type": "completed" },
            })
            .to_string(),
        ),
        participant_digest: [7; 32],
    })
}

fn obligation_mutation(obligation_id: &str, pending: bool) -> LocalStateMutation {
    LocalStateMutation::Obligation(ObligationMutation {
        obligation_id: obligation_id.to_string(),
        record: payload(
            &serde_json::json!({
                "schema": "send_obligation_v1",
                "obligation_id": obligation_id,
                "operation_id": format!("operation-{obligation_id}"),
                "session_id": "session-1",
                "kind": "turn_execution",
                "state": "prepared",
            })
            .to_string(),
        ),
        pending: pending.then(|| PendingIndexEntry {
            ordered_key: format!("0000-{obligation_id}"),
            owner: "session-1".to_string(),
            partition: PendingPartition::Owner,
            shutdown_plan: None,
        }),
        expected: RevisionGuard::Absent,
        revision: Revision::new(0).unwrap(),
    })
}

fn obligation_progress_mutation(obligation_id: &str, pending: bool) -> LocalStateMutation {
    let LocalStateMutation::Obligation(mut mutation) = obligation_mutation(obligation_id, pending)
    else {
        unreachable!("obligation helper always returns an obligation mutation");
    };
    mutation.record = payload(
        &serde_json::json!({
            "schema": "send_obligation_v1",
            "obligation_id": obligation_id,
            "operation_id": format!("operation-{obligation_id}"),
            "session_id": "session-1",
            "kind": "turn_execution",
            "state": "completed",
        })
        .to_string(),
    );
    mutation.expected = RevisionGuard::Expected(Revision::new(0).unwrap());
    mutation.revision = Revision::new(1).unwrap();
    LocalStateMutation::Obligation(mutation)
}

fn obligation_mutation_for_plan(obligation_id: &str, shutdown_id: &str) -> LocalStateMutation {
    let LocalStateMutation::Obligation(mut mutation) = obligation_mutation(obligation_id, true)
    else {
        unreachable!("obligation helper always returns an obligation mutation");
    };
    mutation
        .pending
        .as_mut()
        .expect("pending entry")
        .shutdown_plan = Some(ShutdownPlanKey {
        shutdown_id: shutdown_id.to_string(),
    });
    LocalStateMutation::Obligation(mutation)
}

fn batch(
    commit_id: &str,
    idempotency_key: &str,
    payload_hash: [u8; 32],
    expected_heads: Vec<ExpectedStreamHead>,
    events: Vec<UncommittedDomainEvent>,
    state_mutations: Vec<LocalStateMutation>,
) -> LocalAtomicBatch {
    LocalAtomicBatch {
        commit_id: CommitIdentity::parse(commit_id).unwrap(),
        idempotency: IdempotencyBinding {
            installation_id: TEST_INSTALLATION_ID.to_string(),
            operation_kind: OperationKind::Send.into(),
            idempotency_key: idempotency_key.to_string(),
            payload_hash,
        },
        expected_heads,
        events,
        state_mutations,
    }
}

fn head(stream_id: StreamId, expected: i64) -> ExpectedStreamHead {
    ExpectedStreamHead {
        stream_id,
        expected: StreamVersion::new(expected).unwrap(),
    }
}

fn b050_binding_key() -> CallerOperationKey {
    CallerOperationKey {
        principal: "desktop".to_string(),
        installation_id: TEST_INSTALLATION_ID.to_string(),
        kind: OperationKind::Send,
        caller_request_id: "request-b050".to_string(),
    }
}

fn b050_shutdown_plan_key() -> ShutdownPlanKey {
    ShutdownPlanKey {
        shutdown_id: "plan-b050".to_string(),
    }
}

const B050_PARTICIPANT_WRITES: usize = 8;

fn multi_stream_batch(commit_id: &str, key: &str) -> LocalAtomicBatch {
    batch(
        commit_id,
        key,
        [1; 32],
        vec![
            head(StreamId::application(), 0),
            head(StreamId::agent_session("s-1").unwrap(), 0),
            head(StreamId::workflow("wf-1").unwrap(), 0),
        ],
        vec![
            application_event(1_000),
            session_event("s-1", 1_001),
            queue_pause_event("s-1", 1_002),
            workflow_event("wf-1", 1_003),
        ],
        vec![
            LocalStateMutation::OperationBinding(OperationBindingMutation {
                key: b050_binding_key(),
                operation_id: "operation-b050".to_string(),
                binding_hmac: [50; 32],
            }),
            terminal_mutation("s-1", "turn-1"),
            obligation_mutation("ob-1", true),
            LocalStateMutation::ShutdownPlan(ShutdownPlanMutation {
                key: b050_shutdown_plan_key(),
                phase: ApplicationShutdownPhase::Prepared,
                summary: shutdown_plan_fixture("quit-b050"),
                details_state: ShutdownDetailsState::Available,
                expected: RevisionGuard::Absent,
                revision: Revision::new(0).unwrap(),
            }),
        ],
    )
}

fn b059_shutdown_target_detail(
    target_id: &str,
    state: ShutdownTargetStateRecord,
    recovery_action: Option<ShutdownTargetRecoveryRecord>,
) -> ShutdownTargetRecord {
    ShutdownTargetRecord::Target {
        target_id: target_id.to_string(),
        kind: ShutdownTargetKindRecord::AgentSession,
        state,
        effect_identity: format!("effect-{target_id}"),
        owner_operation_id: Some("quit-b059-fixed-target".to_string()),
        failure: None,
        recovery_action,
    }
}

fn b059_shutdown_target_key(target_id: &str) -> String {
    use base64::Engine as _;

    fn push_lp(material: &mut Vec<u8>, value: &str) {
        material.extend_from_slice(&(value.len() as u32).to_be_bytes());
        material.extend_from_slice(value.as_bytes());
    }
    let mut material = Vec::new();
    push_lp(&mut material, "application-shutdown-target/v1");
    push_lp(&mut material, "agent_session");
    push_lp(&mut material, target_id);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(sha2::Sha256::digest(material))
}

#[allow(clippy::too_many_arguments)] // Closed recovery fixture carries every bound identity explicitly.
fn b059_shutdown_recovery_reserve_mutations(
    plan: ShutdownPlanKey,
    ordinal: i64,
    target_id: &str,
    target_key: &str,
    action_id: &str,
    target_action_id: &str,
    target_guard: RevisionGuard,
    target_revision: Revision,
) -> Vec<LocalStateMutation> {
    let effect_identity = format!("effect-{target_id}");
    let origin_revision = match target_guard {
        RevisionGuard::Absent => 0,
        RevisionGuard::Expected(revision) => revision.value() as u64,
    };
    let recovery_action = ShutdownTargetRecoveryRecord {
        action_id: target_action_id.to_string(),
        origin_revision,
        action: RecoveryActionKind::RetrySameEffect,
        state: ObligationStateRecord::EffectReserved,
    };
    vec![
        LocalStateMutation::RecoveryAction(RecoveryActionMutation {
            action_id: action_id.to_string(),
            binding_hash: [59; 32],
            attempt: RecoveryAttemptRecord::ShutdownTarget {
                resource_ref: format!(
                    "shutdown-target:{}:{ordinal}:{target_key}",
                    plan.shutdown_id
                ),
                plan: plan.clone(),
                ordinal,
                target_key: target_key.to_string(),
                origin_revision: recovery_action.origin_revision,
                action: RecoveryActionKind::RetrySameEffect,
                effect_identity_sha256: sha2::Sha256::digest(effect_identity.as_bytes()).into(),
                intent: QuitIntent::Exit { code: 0 },
                state: ObligationStateRecord::EffectReserved,
                failure: None,
            },
            completed: None,
            expected: RevisionGuard::Absent,
            revision: Revision::new(0).unwrap(),
        }),
        LocalStateMutation::ShutdownTarget(ShutdownTargetMutation {
            key: plan,
            ordinal,
            detail: b059_shutdown_target_detail(
                target_id,
                ShutdownTargetStateRecord::ReconciliationRequired,
                Some(recovery_action),
            ),
            expected: target_guard,
            revision: target_revision,
        }),
    ]
}

fn b059_shutdown_recovery_finish_mutations(
    plan: ShutdownPlanKey,
    ordinal: i64,
    target_id: &str,
    target_key: &str,
    action_id: &str,
) -> Vec<LocalStateMutation> {
    let effect_identity = format!("effect-{target_id}");
    let recovery_action = ShutdownTargetRecoveryRecord {
        action_id: action_id.to_string(),
        origin_revision: 0,
        action: RecoveryActionKind::RetrySameEffect,
        state: ObligationStateRecord::Completed,
    };
    vec![
        LocalStateMutation::RecoveryAction(RecoveryActionMutation {
            action_id: action_id.to_string(),
            binding_hash: [59; 32],
            attempt: RecoveryAttemptRecord::ShutdownTarget {
                resource_ref: format!(
                    "shutdown-target:{}:{ordinal}:{target_key}",
                    plan.shutdown_id
                ),
                plan: plan.clone(),
                ordinal,
                target_key: target_key.to_string(),
                origin_revision: 0,
                action: RecoveryActionKind::RetrySameEffect,
                effect_identity_sha256: sha2::Sha256::digest(effect_identity.as_bytes()).into(),
                intent: QuitIntent::Exit { code: 0 },
                state: ObligationStateRecord::Completed,
                failure: None,
            },
            completed: Some(
                canonicalize_recovery_result_record(
                    RecoveryResultOutcomeRecord::Terminal,
                    RecoveryResultClassification::Succeeded,
                    2,
                    RecoveryResourceViewRecord::ShutdownTarget {
                        plan: plan.clone(),
                        ordinal,
                        target_id: target_key.to_string(),
                        state: ShutdownTargetStateRecord::Completed,
                    },
                )
                .unwrap(),
            ),
            expected: RevisionGuard::Expected(Revision::new(0).unwrap()),
            revision: Revision::new(1).unwrap(),
        }),
        LocalStateMutation::ShutdownTarget(ShutdownTargetMutation {
            key: plan,
            ordinal,
            detail: b059_shutdown_target_detail(
                target_id,
                ShutdownTargetStateRecord::Completed,
                Some(recovery_action),
            ),
            expected: RevisionGuard::Expected(Revision::new(1).unwrap()),
            revision: Revision::new(2).unwrap(),
        }),
    ]
}

#[tokio::test]
async fn b059_shutdown_admission_blocks_new_user_mutations_but_preserves_replay_and_drain() {
    let harness = Harness::open();

    let mut invalid_progress_open = batch(
        "commit-invalid-progress-open",
        "key-invalid-progress-open",
        [30; 32],
        vec![],
        vec![],
        vec![obligation_mutation("invalid-progress-open", true)],
    );
    invalid_progress_open.idempotency.operation_kind = CommitOperationKind::OperationProgress;
    assert!(matches!(
        harness.store.commit_batch(invalid_progress_open).await,
        Err(CommitBatchError::Corrupt { .. })
    ));

    let mut accepted_before_shutdown = batch(
        "commit-user-before-shutdown",
        "key-user-before-shutdown",
        [31; 32],
        vec![],
        vec![],
        vec![
            obligation_mutation("accepted-before-shutdown", true),
            LocalStateMutation::Obligation(ObligationMutation {
                obligation_id: "workflow-execution-b059-drain".to_string(),
                record: workflow_execution_obligation("b059-drain"),
                pending: Some(PendingIndexEntry {
                    ordered_key: "workflow_execution:b059-drain".to_string(),
                    owner: "workflow-runtime".to_string(),
                    partition: PendingPartition::UnownedRuntime,
                    shutdown_plan: None,
                }),
                expected: RevisionGuard::Absent,
                revision: Revision::new(0).unwrap(),
            }),
            LocalStateMutation::SessionProjection(SessionProjectionMutation {
                session_id: "b059-projection-drain".to_string(),
                projection: agent_session_projection("b059-projection-drain"),
                expected: RevisionGuard::Absent,
                revision: Revision::new(0).unwrap(),
            }),
        ],
    );
    accepted_before_shutdown.idempotency.operation_kind = CommitOperationKind::UserMutation;
    let acceptance = harness
        .store
        .commit_batch(accepted_before_shutdown.clone())
        .await;
    assert!(
        matches!(acceptance, Ok(CommitBatchResult::Committed(_))),
        "unexpected pre-shutdown acceptance result: {acceptance:?}"
    );

    let plan = ShutdownPlanKey {
        shutdown_id: "plan-b059-admission".to_string(),
    };
    let mut install_shutdown = batch(
        "commit-install-b059-shutdown",
        "key-install-b059-shutdown",
        [32; 32],
        vec![],
        vec![],
        vec![
            LocalStateMutation::ShutdownPlan(ShutdownPlanMutation {
                key: plan.clone(),
                phase: ApplicationShutdownPhase::Prepared,
                summary: shutdown_plan_fixture("quit-b059-admission"),
                details_state: ShutdownDetailsState::Available,
                expected: RevisionGuard::Absent,
                revision: Revision::new(0).unwrap(),
            }),
            LocalStateMutation::ShutdownLatestPointer(ShutdownLatestPointerMutation {
                expected: None,
                new: Some(plan),
            }),
        ],
    );
    install_shutdown.idempotency.operation_kind = CommitOperationKind::ApplicationQuit;
    harness
        .store
        .commit_batch(install_shutdown)
        .await
        .expect("install current shutdown");

    // The idempotency lookup is deliberately ahead of shutdown admission.
    assert!(matches!(
        harness.store.commit_batch(accepted_before_shutdown).await,
        Ok(CommitBatchResult::Replayed(_))
    ));

    let mut new_user_mutation = batch(
        "commit-user-during-shutdown",
        "key-user-during-shutdown",
        [33; 32],
        vec![],
        vec![],
        vec![obligation_mutation("must-not-be-admitted", true)],
    );
    new_user_mutation.idempotency.operation_kind = CommitOperationKind::UserMutation;
    assert!(matches!(
        harness.store.commit_batch(new_user_mutation).await,
        Err(CommitBatchError::StorageUnavailable { failure })
            if failure.kind
                == SessionOperationFailureKind::PreviousShutdownReconciliationRequired
    ));

    let mut accepted_progress = batch(
        "commit-accepted-progress-during-shutdown",
        "key-accepted-progress-during-shutdown",
        [40; 32],
        vec![],
        vec![],
        vec![obligation_progress_mutation(
            "accepted-before-shutdown",
            false,
        )],
    );
    accepted_progress.idempotency.operation_kind = CommitOperationKind::OperationProgress;
    assert!(matches!(
        harness.store.commit_batch(accepted_progress).await,
        Ok(CommitBatchResult::Committed(_))
    ));

    let mut invalid_progress_closed = batch(
        "commit-invalid-progress-closed",
        "key-invalid-progress-closed",
        [41; 32],
        vec![],
        vec![],
        vec![obligation_mutation("invalid-progress-closed", true)],
    );
    invalid_progress_closed.idempotency.operation_kind = CommitOperationKind::OperationProgress;
    assert!(matches!(
        harness.store.commit_batch(invalid_progress_closed).await,
        Err(CommitBatchError::StorageUnavailable { failure })
            if failure.kind
                == SessionOperationFailureKind::PreviousShutdownReconciliationRequired
    ));

    // Internal lanes may advance a real pre-shutdown owner, but the lane
    // label alone cannot create a new owner after the shutdown gate closes.
    let mut workflow_drain = batch(
        "commit-b059-workflow-progress",
        "key-b059-workflow-progress",
        [34; 32],
        vec![],
        vec![],
        vec![LocalStateMutation::Obligation(ObligationMutation {
            obligation_id: "workflow-execution-b059-drain".to_string(),
            record: workflow_execution_obligation("b059-drain"),
            pending: Some(PendingIndexEntry {
                ordered_key: "workflow_execution:b059-drain".to_string(),
                owner: "workflow-runtime".to_string(),
                partition: PendingPartition::UnownedRuntime,
                shutdown_plan: None,
            }),
            expected: RevisionGuard::Expected(Revision::new(0).unwrap()),
            revision: Revision::new(1).unwrap(),
        })],
    );
    workflow_drain.idempotency.operation_kind = CommitOperationKind::Workflow;
    assert!(matches!(
        harness.store.commit_batch(workflow_drain).await,
        Ok(CommitBatchResult::Committed(_))
    ));

    let mut projection_drain = batch(
        "commit-b059-projection-progress",
        "key-b059-projection-progress",
        [35; 32],
        vec![],
        vec![],
        vec![LocalStateMutation::SessionProjection(
            SessionProjectionMutation {
                session_id: "b059-projection-drain".to_string(),
                projection: agent_session_projection("b059-projection-drain"),
                expected: RevisionGuard::Expected(Revision::new(0).unwrap()),
                revision: Revision::new(1).unwrap(),
            },
        )],
    );
    projection_drain.idempotency.operation_kind = CommitOperationKind::Projection;
    assert!(matches!(
        harness.store.commit_batch(projection_drain).await,
        Ok(CommitBatchResult::Committed(_))
    ));

    let mut foreign_projection = batch(
        "commit-b059-foreign-projection",
        "key-b059-foreign-projection",
        [43; 32],
        vec![],
        vec![],
        vec![
            LocalStateMutation::SessionProjection(SessionProjectionMutation {
                session_id: "b059-projection-drain".to_string(),
                projection: agent_session_projection("b059-projection-drain"),
                expected: RevisionGuard::Expected(Revision::new(1).unwrap()),
                revision: Revision::new(2).unwrap(),
            }),
            LocalStateMutation::MessageProjection(MessageProjectionMutation {
                session_id: "foreign-session".to_string(),
                message_id: "foreign-message".to_string(),
                projection: agent_message_projection("foreign-message", "must not commit"),
                expected: RevisionGuard::Absent,
                revision: Revision::new(0).unwrap(),
            }),
        ],
    );
    foreign_projection.idempotency.operation_kind = CommitOperationKind::Projection;
    assert!(matches!(
        harness.store.commit_batch(foreign_projection).await,
        Err(CommitBatchError::StorageUnavailable { failure })
            if failure.kind
                == SessionOperationFailureKind::PreviousShutdownReconciliationRequired
    ));

    let mut foreign_workflow = batch(
        "commit-b059-foreign-workflow",
        "key-b059-foreign-workflow",
        [44; 32],
        vec![],
        vec![],
        vec![
            LocalStateMutation::Obligation(ObligationMutation {
                obligation_id: "workflow-execution-b059-drain".to_string(),
                record: workflow_execution_obligation("b059-drain"),
                pending: Some(PendingIndexEntry {
                    ordered_key: "workflow_execution:b059-drain".to_string(),
                    owner: "workflow-runtime".to_string(),
                    partition: PendingPartition::UnownedRuntime,
                    shutdown_plan: None,
                }),
                expected: RevisionGuard::Expected(Revision::new(1).unwrap()),
                revision: Revision::new(2).unwrap(),
            }),
            LocalStateMutation::SessionProjection(SessionProjectionMutation {
                session_id: "foreign-session".to_string(),
                projection: agent_session_projection("foreign-session"),
                expected: RevisionGuard::Absent,
                revision: Revision::new(0).unwrap(),
            }),
        ],
    );
    foreign_workflow.idempotency.operation_kind = CommitOperationKind::Workflow;
    assert!(matches!(
        harness.store.commit_batch(foreign_workflow).await,
        Err(CommitBatchError::StorageUnavailable { failure })
            if failure.kind
                == SessionOperationFailureKind::PreviousShutdownReconciliationRequired
    ));

    for (kind, suffix, hash) in [
        (CommitOperationKind::Workflow, "workflow", [36; 32]),
        (CommitOperationKind::Projection, "projection", [37; 32]),
        (
            CommitOperationKind::ShutdownTarget,
            "shutdown-target",
            [38; 32],
        ),
        (
            CommitOperationKind::ApplicationQuit,
            "application-quit",
            [39; 32],
        ),
    ] {
        let mut relabeled = batch(
            &format!("commit-b059-relabeled-{suffix}"),
            &format!("key-b059-relabeled-{suffix}"),
            hash,
            vec![],
            vec![],
            vec![obligation_mutation(&format!("drain-{suffix}"), true)],
        );
        relabeled.idempotency.operation_kind = kind;
        assert!(matches!(
            harness.store.commit_batch(relabeled).await,
            Err(CommitBatchError::StorageUnavailable { failure })
                if failure.kind
                    == SessionOperationFailureKind::PreviousShutdownReconciliationRequired
        ));
    }

    let connection = harness.raw_connection();
    let blocked_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM obligations WHERE obligation_id = 'must-not-be-admitted'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(blocked_count, 0);
    let drain_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM obligations WHERE obligation_id LIKE 'drain-%'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(drain_count, 0);
}

#[tokio::test]
async fn b059_shutdown_target_lane_accepts_only_the_effect_bound_agent_session_close() {
    let harness = Harness::open();
    let session_id = "b059-shutdown-session";
    let quit_operation_id = "quit-b059-shutdown-lifecycle";
    let lifecycle_operation_id = "lifecycle-b059-shutdown-close";
    let effect_identity = format!("shutdown-target/{quit_operation_id}/0");
    let caller_request_id = format!(
        "shutdown-{}",
        hex::encode(sha2::Sha256::digest(effect_identity.as_bytes()))
    );
    let binding_hmac = [59; 32];

    let mut seed_session = batch(
        "commit-b059-seed-shutdown-session",
        "key-b059-seed-shutdown-session",
        [45; 32],
        vec![],
        vec![],
        vec![LocalStateMutation::SessionProjection(
            SessionProjectionMutation {
                session_id: session_id.to_string(),
                projection: agent_session_projection(session_id),
                expected: RevisionGuard::Absent,
                revision: Revision::new(0).unwrap(),
            },
        )],
    );
    seed_session.idempotency.operation_kind = CommitOperationKind::UserMutation;
    harness
        .store
        .commit_batch(seed_session)
        .await
        .expect("seed target session");

    let plan = ShutdownPlanKey {
        shutdown_id: "plan-b059-shutdown-lifecycle".to_string(),
    };
    let mut install_shutdown = batch(
        "commit-b059-install-shutdown-lifecycle",
        "key-b059-install-shutdown-lifecycle",
        [46; 32],
        vec![],
        vec![],
        vec![
            LocalStateMutation::ShutdownPlan(ShutdownPlanMutation {
                key: plan.clone(),
                phase: ApplicationShutdownPhase::Activated,
                summary: shutdown_plan_fixture(quit_operation_id),
                details_state: ShutdownDetailsState::Available,
                expected: RevisionGuard::Absent,
                revision: Revision::new(0).unwrap(),
            }),
            LocalStateMutation::ShutdownTarget(ShutdownTargetMutation {
                key: plan.clone(),
                ordinal: 0,
                detail: ShutdownTargetRecord::Target {
                    target_id: session_id.to_string(),
                    kind: ShutdownTargetKindRecord::AgentSession,
                    state: ShutdownTargetStateRecord::EffectReserved,
                    effect_identity: effect_identity.clone(),
                    owner_operation_id: Some(quit_operation_id.to_string()),
                    failure: None,
                    recovery_action: None,
                },
                expected: RevisionGuard::Absent,
                revision: Revision::new(0).unwrap(),
            }),
            LocalStateMutation::ShutdownLatestPointer(ShutdownLatestPointerMutation {
                expected: None,
                new: Some(plan),
            }),
        ],
    );
    install_shutdown.idempotency.operation_kind = CommitOperationKind::ApplicationQuit;
    harness
        .store
        .commit_batch(install_shutdown)
        .await
        .expect("install active target");

    let stream_id = StreamId::agent_session(session_id).unwrap();
    let mut close = batch(
        "commit-b059-effect-bound-close",
        lifecycle_operation_id,
        binding_hmac,
        vec![head(stream_id.clone(), 0)],
        vec![
            UncommittedDomainEvent {
                stream_id: stream_id.clone(),
                event: LocalDomainEvent::AgentSession(
                    AgentSessionDomainEvent::SessionLifecycleOperationAccepted {
                        operation_id: lifecycle_operation_id.to_string(),
                        kind: SessionLifecycleKind::Close,
                        at: 1.0,
                    },
                ),
                occurred_at_ms: 1_000,
            },
            UncommittedDomainEvent {
                stream_id,
                event: LocalDomainEvent::AgentSession(AgentSessionDomainEvent::SessionClosed {
                    at: 1.0,
                }),
                occurred_at_ms: 1_000,
            },
        ],
        vec![
            LocalStateMutation::OperationBinding(OperationBindingMutation {
                key: CallerOperationKey {
                    principal: format!("shutdown:{quit_operation_id}"),
                    installation_id: TEST_INSTALLATION_ID.to_string(),
                    kind: OperationKind::SessionLifecycle,
                    caller_request_id,
                },
                operation_id: lifecycle_operation_id.to_string(),
                binding_hmac,
            }),
            LocalStateMutation::SessionLifecycleOperation(OperationRecordMutation {
                kind: OperationKind::SessionLifecycle,
                operation_id: lifecycle_operation_id.to_string(),
                receipt: OperationReceiptRecord::SessionLifecycle {
                    operation_id: lifecycle_operation_id.to_string(),
                    session_id: session_id.to_string(),
                    action: SessionLifecycleRecordAction::Close,
                    first_accepted_revision: 0,
                    commit_operation_kind: CommitOperationKind::ShutdownTarget,
                    authentication: RecordAuthentication {
                        principal_mac: [58; 32],
                        binding_hmac,
                    },
                },
                latest_status: OperationStatusRecord {
                    kind: OperationKind::SessionLifecycle,
                    value: OperationStatusValue::Completed,
                },
                expected: RevisionGuard::Absent,
                revision: Revision::new(0).unwrap(),
            }),
            LocalStateMutation::SessionProjection(SessionProjectionMutation {
                session_id: session_id.to_string(),
                projection: agent_session_projection(session_id),
                expected: RevisionGuard::Expected(Revision::new(0).unwrap()),
                revision: Revision::new(1).unwrap(),
            }),
        ],
    );
    close.idempotency.operation_kind = CommitOperationKind::ShutdownTarget;
    assert!(matches!(
        harness.store.commit_batch(close).await,
        Ok(CommitBatchResult::Committed(_))
    ));
}

#[tokio::test]
async fn b059_operation_progress_cannot_insert_recovery_publication_during_shutdown() {
    let harness = Harness::open();
    let session_id = "session-b059-publication";
    let recovery_id = "recovery-b059-publication";
    let source_obligation_id = format!("backend-recovery:{session_id}:{recovery_id}");
    let initial_source = ObligationRecord::BackendSessionRecovery {
        session_id: session_id.to_string(),
        recovery_id: recovery_id.to_string(),
        detail:
            crate::domain::local_event::BackendSessionRecoveryObligationRecord::EffectReserved {
                old_provider_session_generation: 0,
                reason: BackendSessionRecoveryReason::BackendSessionLost,
                reserved_at_bits: 0,
            },
        state: ObligationStateRecord::EffectReserved,
    };
    let mut seed_source = batch(
        "commit-b059-publication-source",
        "key-b059-publication-source",
        [68; 32],
        vec![],
        vec![],
        vec![LocalStateMutation::Obligation(ObligationMutation {
            obligation_id: source_obligation_id.clone(),
            record: initial_source,
            pending: Some(PendingIndexEntry {
                ordered_key: format!("0000-{source_obligation_id}"),
                owner: session_id.to_string(),
                partition: PendingPartition::Owner,
                shutdown_plan: None,
            }),
            expected: RevisionGuard::Absent,
            revision: Revision::new(0).unwrap(),
        })],
    );
    seed_source.idempotency.operation_kind = CommitOperationKind::Recovery;
    harness
        .store
        .commit_batch(seed_source)
        .await
        .expect("seed accepted backend recovery");

    let plan = ShutdownPlanKey {
        shutdown_id: "plan-b059-publication".to_string(),
    };
    let mut install = batch(
        "commit-install-b059-publication-shutdown",
        "key-install-b059-publication-shutdown",
        [69; 32],
        vec![],
        vec![],
        vec![
            LocalStateMutation::ShutdownPlan(ShutdownPlanMutation {
                key: plan.clone(),
                phase: ApplicationShutdownPhase::Prepared,
                summary: shutdown_plan_fixture("quit-b059-publication"),
                details_state: ShutdownDetailsState::Available,
                expected: RevisionGuard::Absent,
                revision: Revision::new(0).unwrap(),
            }),
            LocalStateMutation::ShutdownLatestPointer(ShutdownLatestPointerMutation {
                expected: None,
                new: Some(plan),
            }),
        ],
    );
    install.idempotency.operation_kind = CommitOperationKind::ApplicationQuit;
    harness
        .store
        .commit_batch(install)
        .await
        .expect("close admission around accepted recovery");

    let message_id = "message-b059-publication";
    let publication_digest = sha2::Sha256::digest(
        format!("recovery-publication/v1\0{session_id}\0{recovery_id}\0{message_id}").as_bytes(),
    );
    let publication_id = format!("recovery-publication-{}", hex::encode(publication_digest));
    let source_completion = ObligationMutation {
        obligation_id: source_obligation_id.clone(),
        record: ObligationRecord::RecoveryTransition {
            original: Box::new(ObligationRecord::BackendSessionRecovery {
                session_id: session_id.to_string(),
                recovery_id: recovery_id.to_string(),
                detail:
                    crate::domain::local_event::BackendSessionRecoveryObligationRecord::Completed {
                        old_provider_session_generation: 0,
                        provider_session_generation: 1,
                        backend_session_id: "provider-session-b059".to_string(),
                        completed_at_bits: 1.0_f64.to_bits(),
                    },
                state: ObligationStateRecord::Completed,
            }),
            recovery_action: crate::domain::local_event::ObligationRecoveryActionRecord {
                action_id: "action-b059-publication".to_string(),
                origin_revision: 0,
                action: RecoveryActionKind::RetrySameEffect,
                effect_identity: source_obligation_id.clone(),
                state: ObligationStateRecord::Completed,
                classification: Some(RecoveryResultClassification::Succeeded),
            },
        },
        pending: None,
        expected: RevisionGuard::Expected(Revision::new(0).unwrap()),
        revision: Revision::new(1).unwrap(),
    };
    let publication = ObligationMutation {
        obligation_id: publication_id.clone(),
        record: ObligationRecord::RecoveryPublication {
            session_id: session_id.to_string(),
            recovery_id: recovery_id.to_string(),
            message_id: message_id.to_string(),
            source_obligation_id: source_obligation_id.clone(),
            detail: crate::domain::local_event::RecoveryPublicationObligationRecord::Pending {
                pending_message: crate::domain::local_event::RecoveryPublicationMessageRecord {
                    kind: crate::domain::local_event::RecoveryPublicationMessageKindRecord::Notice,
                    recovery_id: recovery_id.to_string(),
                    message_id: message_id.to_string(),
                    error: None,
                },
            },
            state: ObligationStateRecord::Pending,
        },
        pending: Some(PendingIndexEntry {
            ordered_key: format!("0001-{publication_id}"),
            owner: session_id.to_string(),
            partition: PendingPartition::Owner,
            shutdown_plan: None,
        }),
        expected: RevisionGuard::Absent,
        revision: Revision::new(0).unwrap(),
    };
    let mut progress = batch(
        "commit-b059-publication-progress",
        "key-b059-publication-progress",
        [70; 32],
        vec![],
        vec![],
        vec![
            LocalStateMutation::Obligation(source_completion),
            LocalStateMutation::Obligation(publication),
        ],
    );
    progress.idempotency.operation_kind = CommitOperationKind::OperationProgress;
    assert!(matches!(
        harness.store.commit_batch(progress).await,
        Err(CommitBatchError::StorageUnavailable { failure })
            if failure.kind
                == SessionOperationFailureKind::PreviousShutdownReconciliationRequired
    ));

    let connection = harness.raw_connection();
    let source_revision: i64 = connection
        .query_row(
            "SELECT revision FROM obligations WHERE obligation_id = ?1",
            [&source_obligation_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        source_revision, 0,
        "blocked progress must roll back its owner"
    );
    let publication_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM obligations WHERE obligation_id = ?1",
            [&publication_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(publication_count, 0);
}

#[tokio::test]
async fn b059_shutdown_recovery_is_bound_to_the_existing_current_target() {
    let harness = Harness::open();
    let plan = ShutdownPlanKey {
        shutdown_id: "plan-b059-fixed-target".to_string(),
    };
    let mut summary = shutdown_plan_fixture("quit-b059-fixed-target");
    summary.target_count = Some(2);
    summary.unresolved_count = Some(2);
    let mut install = batch(
        "commit-install-b059-fixed-target",
        "key-install-b059-fixed-target",
        [61; 32],
        vec![],
        vec![],
        vec![
            LocalStateMutation::ShutdownPlan(ShutdownPlanMutation {
                key: plan.clone(),
                phase: ApplicationShutdownPhase::ReconciliationRequired,
                summary,
                details_state: ShutdownDetailsState::Available,
                expected: RevisionGuard::Absent,
                revision: Revision::new(0).unwrap(),
            }),
            LocalStateMutation::ShutdownTarget(ShutdownTargetMutation {
                key: plan.clone(),
                ordinal: 0,
                detail: b059_shutdown_target_detail(
                    "fixed-target-0",
                    ShutdownTargetStateRecord::ReconciliationRequired,
                    None,
                ),
                expected: RevisionGuard::Absent,
                revision: Revision::new(0).unwrap(),
            }),
            LocalStateMutation::ShutdownTarget(ShutdownTargetMutation {
                key: plan.clone(),
                ordinal: 1,
                detail: b059_shutdown_target_detail(
                    "fixed-target-1",
                    ShutdownTargetStateRecord::ReconciliationRequired,
                    None,
                ),
                expected: RevisionGuard::Absent,
                revision: Revision::new(0).unwrap(),
            }),
            LocalStateMutation::ShutdownLatestPointer(ShutdownLatestPointerMutation {
                expected: None,
                new: Some(plan.clone()),
            }),
        ],
    );
    install.idempotency.operation_kind = CommitOperationKind::ApplicationQuit;
    harness
        .store
        .commit_batch(install)
        .await
        .expect("install active fixed target set");

    let foreign_plan = ShutdownPlanKey {
        shutdown_id: "plan-b059-foreign".to_string(),
    };
    let blocked = [
        (
            "absent-ordinal",
            [62; 32],
            b059_shutdown_recovery_reserve_mutations(
                plan.clone(),
                2,
                "new-target",
                "new-target-key",
                "action-absent-ordinal",
                "action-absent-ordinal",
                RevisionGuard::Absent,
                Revision::new(0).unwrap(),
            ),
        ),
        (
            "foreign-plan",
            [63; 32],
            b059_shutdown_recovery_reserve_mutations(
                foreign_plan,
                0,
                "fixed-target-0",
                "fixed-target-key-0",
                "action-foreign-plan",
                "action-foreign-plan",
                RevisionGuard::Expected(Revision::new(0).unwrap()),
                Revision::new(1).unwrap(),
            ),
        ),
        (
            "foreign-target",
            [64; 32],
            b059_shutdown_recovery_reserve_mutations(
                plan.clone(),
                0,
                "foreign-target",
                "foreign-target-key",
                "action-foreign-target",
                "action-foreign-target",
                RevisionGuard::Expected(Revision::new(0).unwrap()),
                Revision::new(1).unwrap(),
            ),
        ),
        (
            "foreign-action",
            [65; 32],
            b059_shutdown_recovery_reserve_mutations(
                plan.clone(),
                0,
                "fixed-target-0",
                &b059_shutdown_target_key("fixed-target-0"),
                "action-foreign-action",
                "different-target-action",
                RevisionGuard::Expected(Revision::new(0).unwrap()),
                Revision::new(1).unwrap(),
            ),
        ),
    ];
    for (suffix, hash, mutations) in blocked {
        let mut attempt = batch(
            &format!("commit-b059-{suffix}"),
            &format!("key-b059-{suffix}"),
            hash,
            vec![],
            vec![],
            mutations,
        );
        attempt.idempotency.operation_kind = CommitOperationKind::Recovery;
        assert!(matches!(
            harness.store.commit_batch(attempt).await,
            Err(CommitBatchError::StorageUnavailable { failure })
                if failure.kind
                    == SessionOperationFailureKind::PreviousShutdownReconciliationRequired
        ));
    }

    let mut reserve = batch(
        "commit-b059-valid-target-reserve",
        "key-b059-valid-target-reserve",
        [66; 32],
        vec![],
        vec![],
        b059_shutdown_recovery_reserve_mutations(
            plan.clone(),
            0,
            "fixed-target-0",
            &b059_shutdown_target_key("fixed-target-0"),
            "action-valid-target",
            "action-valid-target",
            RevisionGuard::Expected(Revision::new(0).unwrap()),
            Revision::new(1).unwrap(),
        ),
    );
    reserve.idempotency.operation_kind = CommitOperationKind::Recovery;
    assert!(matches!(
        harness.store.commit_batch(reserve).await,
        Ok(CommitBatchResult::Committed(_))
    ));

    let mut finish = batch(
        "commit-b059-valid-target-finish",
        "key-b059-valid-target-finish",
        [67; 32],
        vec![],
        vec![],
        b059_shutdown_recovery_finish_mutations(
            plan,
            0,
            "fixed-target-0",
            &b059_shutdown_target_key("fixed-target-0"),
            "action-valid-target",
        ),
    );
    finish.idempotency.operation_kind = CommitOperationKind::Recovery;
    let finish_result = harness.store.commit_batch(finish).await;
    assert!(
        matches!(finish_result, Ok(CommitBatchResult::Committed(_))),
        "unexpected exact-target finish result: {finish_result:?}"
    );

    let connection = harness.raw_connection();
    let target_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM shutdown_targets
             WHERE shutdown_id = 'plan-b059-fixed-target'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(target_count, 2, "recovery must not broaden target ordinals");
    let action_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM recovery_action_attempts", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(action_count, 1, "only the exactly-bound action is durable");
}

#[tokio::test]
async fn b059_failed_and_cancelled_shutdown_pointers_reopen_user_admission() {
    for (phase, suffix, hash) in [
        (ApplicationShutdownPhase::Failed, "failed", [37; 32]),
        (ApplicationShutdownPhase::Cancelled, "cancelled", [38; 32]),
    ] {
        let harness = Harness::open();
        let plan = ShutdownPlanKey {
            shutdown_id: format!("plan-b059-{suffix}"),
        };
        let mut install_terminal_pointer = batch(
            &format!("commit-install-b059-{suffix}"),
            &format!("key-install-b059-{suffix}"),
            hash,
            vec![],
            vec![],
            vec![
                LocalStateMutation::ShutdownPlan(ShutdownPlanMutation {
                    key: plan.clone(),
                    phase,
                    summary: shutdown_plan_fixture(&format!("quit-b059-{suffix}")),
                    details_state: ShutdownDetailsState::Available,
                    expected: RevisionGuard::Absent,
                    revision: Revision::new(0).unwrap(),
                }),
                LocalStateMutation::ShutdownLatestPointer(ShutdownLatestPointerMutation {
                    expected: None,
                    new: Some(plan),
                }),
            ],
        );
        install_terminal_pointer.idempotency.operation_kind = CommitOperationKind::ApplicationQuit;
        harness
            .store
            .commit_batch(install_terminal_pointer)
            .await
            .expect("install terminal shutdown pointer");

        let mut user_mutation = batch(
            &format!("commit-user-after-b059-{suffix}"),
            &format!("key-user-after-b059-{suffix}"),
            [39; 32],
            vec![],
            vec![],
            vec![obligation_mutation(
                &format!("admitted-after-{suffix}"),
                true,
            )],
        );
        user_mutation.idempotency.operation_kind = CommitOperationKind::UserMutation;
        assert!(matches!(
            harness.store.commit_batch(user_mutation).await,
            Ok(CommitBatchResult::Committed(_))
        ));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b059_writer_transaction_race_never_commits_a_user_mutation_after_shutdown_wins() {
    let harness = Harness::open();
    let plan = ShutdownPlanKey {
        shutdown_id: "plan-b059-writer-race".to_string(),
    };
    let mut install_shutdown = batch(
        "commit-install-b059-writer-race",
        "key-install-b059-writer-race",
        [42; 32],
        vec![],
        vec![],
        vec![
            LocalStateMutation::ShutdownPlan(ShutdownPlanMutation {
                key: plan.clone(),
                phase: ApplicationShutdownPhase::Prepared,
                summary: shutdown_plan_fixture("quit-b059-writer-race"),
                details_state: ShutdownDetailsState::Available,
                expected: RevisionGuard::Absent,
                revision: Revision::new(0).unwrap(),
            }),
            LocalStateMutation::ShutdownLatestPointer(ShutdownLatestPointerMutation {
                expected: None,
                new: Some(plan),
            }),
        ],
    );
    install_shutdown.idempotency.operation_kind = CommitOperationKind::ApplicationQuit;
    let mut racing_user = batch(
        "commit-user-b059-writer-race",
        "key-user-b059-writer-race",
        [43; 32],
        vec![],
        vec![],
        vec![obligation_mutation("user-b059-writer-race", true)],
    );
    racing_user.idempotency.operation_kind = CommitOperationKind::UserMutation;

    let start = Arc::new(tokio::sync::Barrier::new(3));
    let shutdown_store = Arc::clone(&harness.store);
    let shutdown_start = Arc::clone(&start);
    let shutdown = tokio::spawn(async move {
        shutdown_start.wait().await;
        shutdown_store.commit_batch(install_shutdown).await
    });
    let user_store = Arc::clone(&harness.store);
    let user_start = Arc::clone(&start);
    let user = tokio::spawn(async move {
        user_start.wait().await;
        user_store.commit_batch(racing_user).await
    });
    start.wait().await;

    let shutdown_result = shutdown.await.unwrap();
    let user_result = user.await.unwrap();
    assert!(matches!(
        shutdown_result,
        Ok(CommitBatchResult::Committed(_))
    ));
    match user_result {
        Ok(CommitBatchResult::Committed(_)) => {}
        Err(CommitBatchError::StorageUnavailable { failure })
            if failure.kind
                == SessionOperationFailureKind::PreviousShutdownReconciliationRequired => {}
        other => panic!("unexpected raced user-mutation result: {other:?}"),
    }

    // Whichever request entered the serialized writer first determines the
    // race. Once the shutdown transaction wins, a later fresh mutation must
    // be rejected by that same transaction-owned authority.
    let mut after_shutdown = batch(
        "commit-user-after-b059-writer-race",
        "key-user-after-b059-writer-race",
        [44; 32],
        vec![],
        vec![],
        vec![obligation_mutation("user-after-b059-writer-race", true)],
    );
    after_shutdown.idempotency.operation_kind = CommitOperationKind::UserMutation;
    assert!(matches!(
        harness.store.commit_batch(after_shutdown).await,
        Err(CommitBatchError::StorageUnavailable { failure })
            if failure.kind
                == SessionOperationFailureKind::PreviousShutdownReconciliationRequired
    ));
    let post_shutdown_count: i64 = harness
        .raw_connection()
        .query_row(
            "SELECT COUNT(*) FROM obligations WHERE obligation_id = 'user-after-b059-writer-race'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(post_shutdown_count, 0);
}

async fn pending_first_page(store: &LocalEventStore) -> Vec<String> {
    match store
        .query(LocalEventQuery::PendingRecoveryPage {
            limit: 200,
            partition: None,
            owner: None,
            ordered_key_prefix: None,
            shutdown_plan: None,
            cursor: None,
        })
        .await
        .expect("pending page")
    {
        LocalEventQueryResult::PendingRecoveryPage(page) => page
            .entries
            .into_iter()
            .map(|entry| entry.obligation_id)
            .collect(),
        other => panic!("unexpected result {other:?}"),
    }
}

fn feedback_action_id(entry: &SessionFeedbackEntry, action: FeedbackAction) -> String {
    entry
        .action_identity(action)
        .expect("projected feedback action identity")
        .to_string()
}

#[tokio::test]
async fn durable_feedback_is_session_scoped_paged_and_revision_fenced() {
    let harness = Harness::open();
    let feedback = SessionFeedbackUsecase::new(
        harness.store.clone(),
        harness.store.installation_id().to_string(),
    );
    for ordinal in 0..33 {
        feedback
            .record_failure(
                "session-a",
                AgentSessionNoticeOperation::Send,
                SafeOperationFailure::new(
                    SessionOperationFailureKind::PersistFailure,
                    true,
                    "send failed",
                    format!("correlation-{ordinal}"),
                )
                .with_detail("safe detail"),
                ordinal == 0,
            )
            .await
            .expect("record feedback");
    }
    feedback
        .record_failure(
            "session-b",
            AgentSessionNoticeOperation::LoadSession,
            SafeOperationFailure::new(
                SessionOperationFailureKind::StorageUnavailable,
                true,
                "load failed",
                "other-session-correlation",
            ),
            false,
        )
        .await
        .expect("record other session feedback");

    let first = feedback
        .list("session-a", 32, None)
        .await
        .expect("first feedback page");
    assert_eq!(first.entries.len(), 32);
    assert!(first
        .entries
        .iter()
        .all(|entry| entry.session_id == "session-a"));
    let second = feedback
        .list("session-a", 32, first.next_cursor)
        .await
        .expect("second feedback page");
    assert_eq!(second.entries.len(), 1);
    assert!(second.next_cursor.is_none());

    let selected = first.entries[0].clone();
    let selected_dismiss = feedback_action_id(&selected, FeedbackAction::Dismiss);
    feedback
        .dismiss(
            "session-a",
            &selected.feedback_id,
            selected.revision,
            &selected_dismiss,
        )
        .await
        .expect("dismiss exact feedback");
    assert_eq!(
        feedback
            .dismiss(
                "session-a",
                &selected.feedback_id,
                selected.revision,
                &selected_dismiss,
            )
            .await,
        Err(FeedbackError::RevisionConflict {
            current_revision: 1
        })
    );
    let other = &first.entries[1];
    assert!(matches!(
        feedback
            .dismiss(
                "session-b",
                &other.feedback_id,
                other.revision,
                &feedback_action_id(other, FeedbackAction::Dismiss),
            )
            .await,
        Err(FeedbackError::NotFound)
    ));
}

#[tokio::test]
async fn feedback_public_round_trip_bounds_utf8_text_and_keeps_failure_fields_nested() {
    let harness = Harness::open();
    let feedback = SessionFeedbackUsecase::new(
        harness.store.clone(),
        harness.store.installation_id().to_string(),
    );
    feedback
        .record_failure(
            "session-bounded-feedback",
            AgentSessionNoticeOperation::LoadSession,
            SafeOperationFailure::new(
                SessionOperationFailureKind::PersistFailure,
                true,
                &"表示".repeat(100),
                "bounded-feedback-correlation",
            )
            .with_detail(&"詳細".repeat(400)),
            false,
        )
        .await
        .expect("persist bounded feedback");

    let page = feedback
        .list("session-bounded-feedback", 32, None)
        .await
        .expect("round-trip bounded feedback");
    let entry = page.entries.into_iter().next().expect("feedback entry");
    assert!(entry.failure.label.value().len() <= 160);
    assert!(entry.failure.label.value().ends_with('…'));
    let detail = entry.failure.detail.as_ref().expect("bounded detail");
    assert!(detail.value().len() <= 2_048);
    assert!(detail.value().ends_with('…'));

    let public = serde_json::to_value(
        crate::adaptor::protocol::agent_session_notice::SessionFeedbackEntryMessage::from(entry),
    )
    .expect("public feedback DTO");
    assert!(public.get("kind").is_none());
    assert!(public.get("retryable").is_none());
    assert!(public.get("label").is_none());
    assert!(public.get("detail").is_none());
    assert_eq!(public["failure"]["kind"], "persist_failure");
    assert!(public["failure"]["label"]
        .as_str()
        .expect("public label")
        .ends_with('…'));
    assert!(public["failure"]["detail"]
        .as_str()
        .expect("public detail")
        .ends_with('…'));
}

#[tokio::test]
async fn feedback_attempt_reservation_is_hidden_until_failure_and_success_is_identity_local() {
    let harness = Harness::open();
    let feedback = SessionFeedbackUsecase::new(
        harness.store.clone(),
        harness.store.installation_id().to_string(),
    );
    let failed = feedback
        .reserve_attempt(
            "session-load",
            AgentSessionNoticeOperation::LoadSession,
            "load-attempt-1",
        )
        .await
        .expect("reserve failed load capacity");
    assert!(feedback
        .list("session-load", 32, None)
        .await
        .expect("reserved slot query")
        .entries
        .is_empty());

    let entry = feedback
        .materialize_failure(
            &failed,
            SafeOperationFailure::new(
                SessionOperationFailureKind::StorageUnavailable,
                true,
                "The session could not be loaded.",
                "load-failure-1",
            ),
            None,
        )
        .await
        .expect("materialize the exact failed attempt");
    assert_eq!(entry.feedback_id, failed.feedback_id);
    assert_eq!(entry.attempt_id, "load-attempt-1");
    assert_eq!(entry.revision, 1);

    let succeeded = feedback
        .reserve_attempt(
            "session-load",
            AgentSessionNoticeOperation::LoadSession,
            "load-attempt-2",
        )
        .await
        .expect("reserve successful load capacity");
    feedback
        .complete_success(&succeeded)
        .await
        .expect("settle only the successful attempt");
    let page = feedback
        .list("session-load", 32, None)
        .await
        .expect("failed load remains visible");
    assert_eq!(page.entries.len(), 1);
    assert_eq!(page.entries[0].feedback_id, failed.feedback_id);
}

#[tokio::test]
async fn abandoned_feedback_reservation_is_invisible_then_materialized_on_restart() {
    let harness = Harness::open();
    let first_process = SessionFeedbackUsecase::new(
        harness.store.clone(),
        harness.store.installation_id().to_string(),
    );
    let reservation = first_process
        .reserve_attempt(
            "session-abandoned-load",
            AgentSessionNoticeOperation::LoadSession,
            "abandoned-load-attempt",
        )
        .await
        .expect("reserve before the simulated crash");
    assert_eq!(
        first_process
            .recover_abandoned_reservations()
            .await
            .expect("current-process scan"),
        0
    );
    assert!(first_process
        .list("session-abandoned-load", 32, None)
        .await
        .expect("reservation stays hidden")
        .entries
        .is_empty());
    let general_recovery = crate::usecase::agent_session::operation::RecoveryActionUsecase::new(
        harness.store.clone(),
        harness.store.clone(),
        Arc::new(PerformanceRecoveryExecutor),
        harness.store.installation_id().to_string(),
    );
    assert!(general_recovery
        .pending(
            crate::usecase::agent_session::operation::PendingRecoveryQuery {
                limit: 32,
                partition: None,
                owner: None,
                shutdown_plan: None,
                cursor: None,
            },
        )
        .await
        .expect("internal reservation is hidden from general recovery")
        .entries
        .is_empty());

    let restarted = SessionFeedbackUsecase::new(
        harness.store.clone(),
        harness.store.installation_id().to_string(),
    );
    assert_eq!(
        restarted
            .recover_abandoned_reservations()
            .await
            .expect("recover the earlier process reservation"),
        1
    );
    let page = restarted
        .list("session-abandoned-load", 32, None)
        .await
        .expect("recovered feedback page");
    assert_eq!(page.entries.len(), 1);
    assert_eq!(page.entries[0].feedback_id, reservation.feedback_id);
    assert_eq!(page.entries[0].attempt_id, "abandoned-load-attempt");
    assert_eq!(page.entries[0].revision, 1);
    assert_eq!(
        page.entries[0].failure.kind,
        SessionOperationFailureKind::OutcomeUnknown
    );
    assert_eq!(
        restarted
            .recover_abandoned_reservations()
            .await
            .expect("recovery is idempotent"),
        0
    );
}

#[tokio::test]
async fn failed_success_settlement_keeps_the_slot_recoverable_and_reply_loss_cleans_it() {
    let harness = Harness::open();
    let first_process = SessionFeedbackUsecase::new(
        harness.store.clone(),
        harness.store.installation_id().to_string(),
    );
    let unsettled = first_process
        .reserve_attempt(
            "session-settlement",
            AgentSessionNoticeOperation::LoadSession,
            "settlement-failure",
        )
        .await
        .expect("reserve settlement slot");
    harness.fault.arm_fail_before_begin();
    assert!(matches!(
        first_process.complete_success(&unsettled).await,
        Err(FeedbackError::StorageUnavailable { .. })
    ));
    assert!(first_process
        .list("session-settlement", 32, None)
        .await
        .expect("failed settlement reservation is hidden")
        .entries
        .is_empty());

    let restarted = SessionFeedbackUsecase::new(
        harness.store.clone(),
        harness.store.installation_id().to_string(),
    );
    assert_eq!(
        restarted
            .recover_abandoned_reservations()
            .await
            .expect("failed settlement is recoverable"),
        1
    );
    let recovered = restarted
        .list("session-settlement", 32, None)
        .await
        .expect("recovered settlement feedback");
    assert_eq!(recovered.entries.len(), 1);
    assert_eq!(recovered.entries[0].feedback_id, unsettled.feedback_id);

    let reply_loss = restarted
        .reserve_attempt(
            "session-settlement",
            AgentSessionNoticeOperation::LoadSession,
            "settlement-reply-loss",
        )
        .await
        .expect("reserve reply-loss slot");
    harness.fault.arm_drop_reply();
    restarted
        .complete_success(&reply_loss)
        .await
        .expect("commit resolution proves the success settlement");
    let next_process = SessionFeedbackUsecase::new(
        harness.store.clone(),
        harness.store.installation_id().to_string(),
    );
    assert_eq!(
        next_process
            .recover_abandoned_reservations()
            .await
            .expect("settled reply-loss slot is no longer pending"),
        0
    );
    let page = next_process
        .list("session-settlement", 32, None)
        .await
        .expect("only the prior genuine failure remains");
    assert_eq!(page.entries.len(), 1);
    assert_eq!(page.entries[0].feedback_id, unsettled.feedback_id);
}

struct FailingFeedbackResolution;

#[async_trait::async_trait]
impl FeedbackResolutionPort for FailingFeedbackResolution {
    async fn retry_exact_resolution(
        &self,
        resolution_identity: &str,
    ) -> Result<(), SafeOperationFailure> {
        assert!(resolution_identity.starts_with("feedback-"));
        Err(SafeOperationFailure::new(
            SessionOperationFailureKind::StorageUnavailable,
            true,
            "retry still unavailable",
            "retry-correlation",
        ))
    }
}

struct CountingFeedbackResolution {
    calls: std::sync::atomic::AtomicUsize,
}

struct CountingFailingFeedbackResolution {
    calls: std::sync::atomic::AtomicUsize,
}

struct CountingSessionLoader {
    calls: std::sync::atomic::AtomicUsize,
}

#[async_trait::async_trait]
impl SessionLoadPort for CountingSessionLoader {
    async fn load_session(
        &self,
        _session_id: &str,
    ) -> Result<Option<crate::usecase::agent_session::session::GetSessionResponse>, String> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(None)
    }
}

#[async_trait::async_trait]
impl FeedbackResolutionPort for CountingFeedbackResolution {
    async fn retry_exact_resolution(
        &self,
        _resolution_identity: &str,
    ) -> Result<(), SafeOperationFailure> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }
}

#[tokio::test]
async fn feedback_resolution_success_clears_only_the_exact_identity() {
    let harness = Harness::open();
    let resolution = Arc::new(CountingFeedbackResolution {
        calls: std::sync::atomic::AtomicUsize::new(0),
    });
    let feedback = SessionFeedbackUsecase::new(
        harness.store.clone(),
        harness.store.installation_id().to_string(),
    )
    .with_resolution_port(resolution.clone());
    let resolved = feedback
        .record_failure_with_resolution(
            "session-exact-resolution",
            AgentSessionNoticeOperation::Send,
            SafeOperationFailure::new(
                SessionOperationFailureKind::PersistFailure,
                true,
                "first failure",
                "first-resolution-correlation",
            ),
            Some("feedback-resolution-success".to_string()),
        )
        .await
        .expect("record exact retry feedback");
    let untouched = feedback
        .record_failure(
            "session-exact-resolution",
            AgentSessionNoticeOperation::LoadOlder,
            SafeOperationFailure::new(
                SessionOperationFailureKind::PersistFailure,
                true,
                "second failure",
                "untouched-correlation",
            ),
            false,
        )
        .await
        .expect("record independent feedback");

    assert_eq!(
        feedback
            .retry_resolution(
                "session-exact-resolution",
                &resolved.feedback_id,
                resolved.revision,
                &feedback_action_id(&resolved, FeedbackAction::RetryResolution),
            )
            .await,
        Ok(FeedbackRetryOutcome::Resolved)
    );
    assert_eq!(
        resolution.calls.load(std::sync::atomic::Ordering::SeqCst),
        1
    );
    assert_eq!(
        feedback
            .retry_resolution(
                "session-exact-resolution",
                &resolved.feedback_id,
                resolved.revision,
                &feedback_action_id(&resolved, FeedbackAction::RetryResolution),
            )
            .await,
        Err(FeedbackError::RevisionConflict {
            current_revision: 1
        })
    );
    assert_eq!(
        resolution.calls.load(std::sync::atomic::Ordering::SeqCst),
        1
    );
    let page = feedback
        .list("session-exact-resolution", 32, None)
        .await
        .expect("exact resolved identity disappears");
    assert_eq!(page.entries.len(), 1);
    assert_eq!(page.entries[0].feedback_id, untouched.feedback_id);
}

#[async_trait::async_trait]
impl FeedbackResolutionPort for CountingFailingFeedbackResolution {
    async fn retry_exact_resolution(
        &self,
        _resolution_identity: &str,
    ) -> Result<(), SafeOperationFailure> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Err(SafeOperationFailure::new(
            SessionOperationFailureKind::StorageUnavailable,
            true,
            "retry still unavailable",
            "capacity-retry-correlation",
        ))
    }
}

#[tokio::test]
async fn feedback_retry_updates_the_same_identity_and_replays_the_saved_result() {
    let harness = Harness::open();
    let feedback = SessionFeedbackUsecase::new(
        harness.store.clone(),
        harness.store.installation_id().to_string(),
    )
    .with_resolution_port(Arc::new(FailingFeedbackResolution));
    let original = feedback
        .record_failure_with_resolution(
            "session-retry",
            AgentSessionNoticeOperation::Send,
            SafeOperationFailure::new(
                SessionOperationFailureKind::PersistFailure,
                true,
                "send failed",
                "original-correlation",
            ),
            Some("feedback-resolution-1".to_string()),
        )
        .await
        .expect("record retryable feedback");

    let first = feedback
        .retry_resolution(
            "session-retry",
            &original.feedback_id,
            original.revision,
            &feedback_action_id(&original, FeedbackAction::RetryResolution),
        )
        .await
        .expect("retry result");
    let FeedbackRetryOutcome::Failed(first) = first else {
        panic!("retry must preserve the failed feedback");
    };
    assert_eq!(first.feedback_id, original.feedback_id);
    assert_eq!(first.revision, 1);
    assert_eq!(first.failure.label.value(), "retry still unavailable");

    assert_eq!(
        feedback
            .retry_resolution(
                "session-retry",
                &original.feedback_id,
                original.revision,
                &feedback_action_id(&original, FeedbackAction::RetryResolution),
            )
            .await,
        Err(FeedbackError::RevisionConflict {
            current_revision: 1
        })
    );
    let second = feedback
        .retry_resolution(
            "session-retry",
            &original.feedback_id,
            1,
            &feedback_action_id(&first, FeedbackAction::RetryResolution),
        )
        .await
        .expect("second exact retry result");
    let FeedbackRetryOutcome::Failed(second) = second else {
        panic!("second retry must preserve the same failed feedback");
    };
    assert_eq!(second.feedback_id, original.feedback_id);
    assert_eq!(second.revision, 2);
    let third = feedback
        .retry_resolution(
            "session-retry",
            &original.feedback_id,
            2,
            &feedback_action_id(&second, FeedbackAction::RetryResolution),
        )
        .await
        .expect("revision two to three retry result");
    let FeedbackRetryOutcome::Failed(third) = third else {
        panic!("third retry must preserve the same failed feedback");
    };
    assert_eq!(third.feedback_id, original.feedback_id);
    assert_eq!(third.revision, 3);
    assert_eq!(
        feedback
            .dismiss(
                "session-retry",
                &original.feedback_id,
                1,
                &feedback_action_id(&third, FeedbackAction::Dismiss),
            )
            .await,
        Err(FeedbackError::RevisionConflict {
            current_revision: 3
        })
    );
    let page = feedback
        .list("session-retry", 32, None)
        .await
        .expect("feedback page");
    assert_eq!(page.entries.len(), 1);
    assert_eq!(page.entries[0].revision, 3);
}

#[tokio::test]
async fn feedback_mutations_racing_with_shutdown_are_typed_and_retry_starts_no_effect() {
    let harness = Harness::open();
    let resolution = Arc::new(CountingFeedbackResolution {
        calls: std::sync::atomic::AtomicUsize::new(0),
    });
    let feedback = SessionFeedbackUsecase::new(
        harness.store.clone(),
        harness.store.installation_id().to_string(),
    )
    .with_resolution_port(resolution.clone());
    let original = feedback
        .record_failure_with_resolution(
            "session-feedback-shutdown",
            AgentSessionNoticeOperation::Send,
            SafeOperationFailure::new(
                SessionOperationFailureKind::PersistFailure,
                true,
                "send failed",
                "feedback-shutdown-correlation",
            ),
            Some("feedback-shutdown-resolution".to_string()),
        )
        .await
        .expect("record retryable feedback before shutdown");

    let plan = ShutdownPlanKey {
        shutdown_id: "plan-feedback-shutdown".to_string(),
    };
    let mut install_shutdown = batch(
        "commit-install-feedback-shutdown",
        "key-install-feedback-shutdown",
        [42; 32],
        vec![],
        vec![],
        vec![
            LocalStateMutation::ShutdownPlan(ShutdownPlanMutation {
                key: plan.clone(),
                phase: ApplicationShutdownPhase::Prepared,
                summary: shutdown_plan_fixture("quit-feedback-shutdown"),
                details_state: ShutdownDetailsState::Available,
                expected: RevisionGuard::Absent,
                revision: Revision::new(0).unwrap(),
            }),
            LocalStateMutation::ShutdownLatestPointer(ShutdownLatestPointerMutation {
                expected: None,
                new: Some(plan),
            }),
        ],
    );
    install_shutdown.idempotency.operation_kind = CommitOperationKind::ApplicationQuit;
    harness
        .store
        .commit_batch(install_shutdown)
        .await
        .expect("install current shutdown");

    assert_eq!(
        feedback
            .retry_resolution(
                "session-feedback-shutdown",
                &original.feedback_id,
                original.revision,
                &feedback_action_id(&original, FeedbackAction::RetryResolution),
            )
            .await,
        Err(FeedbackError::ShutdownInProgress)
    );
    assert_eq!(
        resolution.calls.load(std::sync::atomic::Ordering::SeqCst),
        0
    );
    assert_eq!(
        feedback
            .dismiss(
                "session-feedback-shutdown",
                &original.feedback_id,
                original.revision,
                &feedback_action_id(&original, FeedbackAction::Dismiss),
            )
            .await,
        Err(FeedbackError::ShutdownInProgress)
    );
    let page = feedback
        .list("session-feedback-shutdown", 32, None)
        .await
        .expect("shutdown leaves feedback pending");
    assert_eq!(page.entries.len(), 1);
    assert_eq!(page.entries[0].revision, original.revision);
}

#[tokio::test]
async fn feedback_capacity_is_enforced_for_the_whole_atomic_batch() {
    let harness = Harness::open();
    let resolution = Arc::new(CountingFailingFeedbackResolution {
        calls: std::sync::atomic::AtomicUsize::new(0),
    });
    let feedback = Arc::new(
        SessionFeedbackUsecase::new(
            harness.store.clone(),
            harness.store.installation_id().to_string(),
        )
        .with_resolution_port(resolution.clone()),
    );
    let dismissible = feedback
        .record_failure(
            "capacity-control-session",
            AgentSessionNoticeOperation::LoadSession,
            SafeOperationFailure::new(
                SessionOperationFailureKind::PersistFailure,
                true,
                "dismissible failure",
                "capacity-dismiss-correlation",
            ),
            false,
        )
        .await
        .expect("seed dismissible feedback");
    let retryable = feedback
        .record_failure_with_resolution(
            "capacity-control-session",
            AgentSessionNoticeOperation::Send,
            SafeOperationFailure::new(
                SessionOperationFailureKind::PersistFailure,
                true,
                "retryable failure",
                "capacity-retry-origin",
            ),
            Some("capacity-retry-resolution".to_string()),
        )
        .await
        .expect("seed retryable feedback");
    let mut retry_projection = retryable.clone();
    for expected_revision in 0..2 {
        let outcome = feedback
            .retry_resolution(
                "capacity-control-session",
                &retryable.feedback_id,
                expected_revision,
                &feedback_action_id(&retry_projection, FeedbackAction::RetryResolution),
            )
            .await
            .expect("advance feedback before filling capacity");
        let FeedbackRetryOutcome::Failed(next_projection) = outcome else {
            panic!("failed retry remains pending");
        };
        retry_projection = *next_projection;
    }
    assert_eq!(retry_projection.revision, 2);
    assert_eq!(
        resolution.calls.load(std::sync::atomic::Ordering::SeqCst),
        2
    );

    let mutations = (0..510)
        .map(|ordinal| {
            let feedback_id = format!("feedback-capacity-fixture-{ordinal:04}");
            LocalStateMutation::Obligation(ObligationMutation {
                obligation_id: feedback_id.clone(),
                record: send_obligation_fixture(&feedback_id, ObligationStateRecord::Pending),
                pending: Some(PendingIndexEntry {
                    ordered_key: format!("feedback:capacity-session:{feedback_id}"),
                    owner: "capacity-session".to_string(),
                    partition: PendingPartition::Owner,
                    shutdown_plan: None,
                }),
                expected: RevisionGuard::Absent,
                revision: Revision::new(0).unwrap(),
            })
        })
        .collect();
    harness
        .store
        .commit_batch(batch(
            "feedback-capacity-seed",
            "feedback-capacity-seed",
            [91; 32],
            Vec::new(),
            Vec::new(),
            mutations,
        ))
        .await
        .expect("seed exactly the feedback capacity");

    let loader = Arc::new(CountingSessionLoader {
        calls: std::sync::atomic::AtomicUsize::new(0),
    });
    let supervised = SessionFeedbackLoadUsecase::new(loader.clone(), feedback.clone());
    assert!(matches!(
        supervised
            .get_session("new-session", "capacity-attempt")
            .await,
        Err(SessionFeedbackLoadError::Feedback(
            FeedbackError::CapacityExceeded
        ))
    ));
    assert_eq!(loader.calls.load(std::sync::atomic::Ordering::SeqCst), 0);

    let at_capacity = feedback
        .list("capacity-control-session", 32, None)
        .await
        .expect("feedback page remains available at capacity");
    assert_eq!(at_capacity.entries.len(), 2);
    let current_retry = at_capacity
        .entries
        .iter()
        .find(|entry| entry.feedback_id == retryable.feedback_id)
        .expect("retryable feedback at capacity")
        .clone();
    assert_eq!(current_retry.revision, 2);
    assert_eq!(
        feedback
            .retry_resolution(
                "capacity-control-session",
                &retryable.feedback_id,
                1,
                &feedback_action_id(&current_retry, FeedbackAction::RetryResolution),
            )
            .await,
        Err(FeedbackError::RevisionConflict {
            current_revision: 2
        })
    );
    assert_eq!(
        feedback
            .dismiss(
                "capacity-control-session",
                &retryable.feedback_id,
                1,
                &feedback_action_id(&current_retry, FeedbackAction::Dismiss),
            )
            .await,
        Err(FeedbackError::RevisionConflict {
            current_revision: 2
        })
    );
    assert_eq!(
        resolution.calls.load(std::sync::atomic::Ordering::SeqCst),
        2
    );
    let retried = feedback
        .retry_resolution(
            "capacity-control-session",
            &retryable.feedback_id,
            2,
            &feedback_action_id(&current_retry, FeedbackAction::RetryResolution),
        )
        .await
        .expect("retry control remains available at capacity");
    let FeedbackRetryOutcome::Failed(retried) = retried else {
        panic!("capacity retry remains the same unresolved identity");
    };
    assert_eq!(retried.feedback_id, retryable.feedback_id);
    assert_eq!(retried.revision, 3);
    assert_eq!(
        resolution.calls.load(std::sync::atomic::Ordering::SeqCst),
        3
    );
    assert_eq!(
        feedback
            .retry_resolution(
                "capacity-control-session",
                &retryable.feedback_id,
                1,
                &feedback_action_id(&retried, FeedbackAction::RetryResolution),
            )
            .await,
        Err(FeedbackError::RevisionConflict {
            current_revision: 3
        })
    );
    assert_eq!(
        resolution.calls.load(std::sync::atomic::Ordering::SeqCst),
        3
    );
    assert_eq!(
        feedback
            .dismiss(
                "capacity-control-session",
                &dismissible.feedback_id,
                7,
                &feedback_action_id(&dismissible, FeedbackAction::Dismiss),
            )
            .await,
        Err(FeedbackError::RevisionConflict {
            current_revision: 0
        })
    );
    let pending_at_capacity: i64 = harness
        .raw_connection()
        .query_row(
            "SELECT COUNT(*) FROM pending_obligations WHERE obligation_id LIKE 'feedback-%'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(pending_at_capacity, 512);
    let unchanged = feedback
        .list("capacity-control-session", 32, None)
        .await
        .expect("stale controls preserve the page");
    assert_eq!(unchanged.entries.len(), 2);
    assert_eq!(
        unchanged
            .entries
            .iter()
            .find(|entry| entry.feedback_id == retryable.feedback_id)
            .expect("retryable entry")
            .revision,
        3
    );

    feedback
        .dismiss(
            "capacity-control-session",
            &dismissible.feedback_id,
            dismissible.revision,
            &feedback_action_id(&dismissible, FeedbackAction::Dismiss),
        )
        .await
        .expect("dismiss control remains available at capacity");
    let pending_after_dismiss: i64 = harness
        .raw_connection()
        .query_row(
            "SELECT COUNT(*) FROM pending_obligations WHERE obligation_id LIKE 'feedback-%'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(pending_after_dismiss, 511);
}

// --- B-050: multi-stream atomicity ---

#[tokio::test]
async fn b050_complete_multi_stream_batch_commits_atomically_with_contiguous_sequences() {
    let harness = Harness::open();
    let legacy_titles = harness.root.join("session_titles.json");
    let legacy_sessions = harness.root.join("sessions");
    std::fs::create_dir_all(&legacy_sessions).expect("legacy sentinel directory");
    std::fs::write(&legacy_titles, b"{\"legacy\":true}\n").expect("legacy sentinel file");
    let legacy_before = std::fs::read(&legacy_titles).expect("legacy sentinel before commit");

    let result = harness
        .store
        .commit_batch(multi_stream_batch("commit-1", "key-1"))
        .await
        .expect("commit");
    let CommitBatchResult::Committed(committed) = result else {
        panic!("expected Committed");
    };
    let (first, last) = committed.sequence_range.expect("range");
    assert_eq!(first.value(), 1);
    assert_eq!(last.value(), 4);
    assert_eq!(committed.event_count, 4);
    assert_eq!(committed.mutation_count, 4);

    // Public queries see all participants.
    let binding = harness
        .store
        .query(LocalEventQuery::OperationBindingByIdentity {
            key: b050_binding_key(),
        })
        .await
        .expect("operation binding query");
    let LocalEventQueryResult::OperationBindingByIdentity(Some(binding)) = binding else {
        panic!("operation binding missing");
    };
    assert_eq!(binding.operation_id, "operation-b050");
    assert_eq!(binding.binding_hmac, [50; 32]);

    let terminal = harness
        .store
        .query(LocalEventQuery::TerminalByTurn {
            session_id: "s-1".to_string(),
            turn_id: "turn-1".to_string(),
        })
        .await
        .expect("terminal query");
    let LocalEventQueryResult::TerminalByTurn(Some(view)) = terminal else {
        panic!("terminal record missing");
    };
    assert_eq!(view.terminal_identity, "terminal-s-1-turn-1");
    assert_eq!(pending_first_page(&harness.store).await, vec!["ob-1"]);

    let application_page = harness
        .store
        .load_stream(LoadStreamRequest {
            stream_id: StreamId::application(),
            after: None,
            limit: 10,
        })
        .await
        .expect("application stream page");
    assert_eq!(application_page.head.value(), 1);
    assert_eq!(application_page.events.len(), 1);

    let agent_page = harness
        .store
        .load_stream(LoadStreamRequest {
            stream_id: StreamId::agent_session("s-1").unwrap(),
            after: None,
            limit: 10,
        })
        .await
        .expect("agent stream page");
    assert_eq!(agent_page.head.value(), 2);
    assert_eq!(agent_page.events.len(), 2);
    assert!(matches!(
        &agent_page.events[0].event,
        LoadedDomainEvent::Known(event)
            if matches!(
                event.as_ref(),
                LocalDomainEvent::AgentSession(
                    AgentSessionDomainEvent::BackendSessionRecoveryStarted { .. }
                )
            )
    ));
    assert!(matches!(
        &agent_page.events[1].event,
        LoadedDomainEvent::Known(event)
            if matches!(
                event.as_ref(),
                LocalDomainEvent::AgentSession(AgentSessionDomainEvent::QueuePaused { .. })
            )
    ));
    assert!(agent_page.next_after.is_none());

    let workflow_page = harness
        .store
        .load_stream(LoadStreamRequest {
            stream_id: StreamId::workflow("wf-1").unwrap(),
            after: None,
            limit: 10,
        })
        .await
        .expect("workflow stream page");
    assert_eq!(workflow_page.head.value(), 1);
    assert_eq!(workflow_page.events.len(), 1);
    assert!(matches!(
        &workflow_page.events[0].event,
        LoadedDomainEvent::Known(event)
            if matches!(
                event.as_ref(),
                LocalDomainEvent::Workflow(
                    WorkflowDomainEvent::WorkflowExecutionAborted { .. }
                )
            )
    ));

    let global_sequences = application_page
        .events
        .iter()
        .chain(agent_page.events.iter())
        .chain(workflow_page.events.iter())
        .map(|event| event.global_sequence.value())
        .collect::<Vec<_>>();
    assert_eq!(global_sequences, vec![1, 2, 3, 4]);

    let plan = harness
        .store
        .query(LocalEventQuery::ShutdownPlanPage {
            plan: b050_shutdown_plan_key(),
            limit: 1,
            cursor: None,
        })
        .await
        .expect("shutdown plan query");
    let LocalEventQueryResult::ShutdownPlanPage(plan) = plan else {
        panic!("shutdown plan missing");
    };
    assert_eq!(plan.plan.plan, b050_shutdown_plan_key());
    assert_eq!(plan.plan.phase, ApplicationShutdownPhase::Prepared);
    assert!(plan.targets.is_empty());

    assert_eq!(
        std::fs::read(&legacy_titles).expect("legacy sentinel after commit"),
        legacy_before,
        "SQLite batch must not dual-write a legacy authority",
    );
    assert!(
        !legacy_sessions.join("s-1").exists(),
        "SQLite batch must not create a legacy session projection",
    );
}

#[tokio::test]
async fn same_key_replay_converges_and_different_payload_conflicts() {
    let harness = Harness::open();
    let first = harness
        .store
        .commit_batch(multi_stream_batch("commit-1", "key-1"))
        .await
        .expect("first commit");
    let replay = harness
        .store
        .commit_batch(multi_stream_batch("commit-1", "key-1"))
        .await
        .expect("replay commit");
    let CommitBatchResult::Replayed(replayed) = replay else {
        panic!("expected Replayed");
    };
    assert_eq!(first.batch(), &replayed);

    // Same key, different canonical payload.
    let mut different = multi_stream_batch("commit-1", "key-1");
    different.idempotency.payload_hash = [9; 32];
    assert!(matches!(
        harness.store.commit_batch(different).await,
        Err(CommitBatchError::PayloadConflict)
    ));
    // Same key + payload but a different commit identity is also a conflict.
    let different_commit = batch(
        "commit-other",
        "key-1",
        [1; 32],
        vec![head(StreamId::application(), 1)],
        vec![],
        vec![],
    );
    assert!(matches!(
        harness.store.commit_batch(different_commit).await,
        Err(CommitBatchError::PayloadConflict)
    ));
}

#[tokio::test]
async fn expected_head_mismatch_is_a_typed_conflict() {
    let harness = Harness::open();
    harness
        .store
        .commit_batch(multi_stream_batch("commit-1", "key-1"))
        .await
        .expect("commit");
    let stale = batch(
        "commit-2",
        "key-2",
        [2; 32],
        vec![head(StreamId::application(), 0)],
        vec![application_event(2_000)],
        vec![],
    );
    match harness.store.commit_batch(stale).await {
        Err(CommitBatchError::StreamHeadConflict { current }) => {
            assert_eq!(current.value(), 1);
        }
        other => panic!("expected StreamHeadConflict, got {other:?}"),
    }
}

// --- B-050: fault matrix boundaries ---

async fn assert_store_unchanged(harness: &Harness, commit_id: &str) {
    let resolution = harness
        .store
        .resolve_commit(CommitIdentity::parse(commit_id).unwrap())
        .await
        .expect("resolve");
    assert_eq!(resolution, CommitResolution::NotCommitted);
    assert!(pending_first_page(&harness.store).await.is_empty());
    for stream_id in [
        StreamId::application(),
        StreamId::agent_session("s-1").unwrap(),
        StreamId::workflow("wf-1").unwrap(),
    ] {
        let page = harness
            .store
            .load_stream(LoadStreamRequest {
                stream_id,
                after: None,
                limit: 10,
            })
            .await
            .expect("stream");
        assert_eq!(page.head.value(), 0);
        assert!(page.events.is_empty());
    }
    assert!(matches!(
        harness
            .store
            .query(LocalEventQuery::OperationBindingByIdentity {
                key: b050_binding_key(),
            })
            .await
            .expect("operation binding query"),
        LocalEventQueryResult::OperationBindingByIdentity(None)
    ));
    assert!(matches!(
        harness
            .store
            .query(LocalEventQuery::TerminalByTurn {
                session_id: "s-1".to_string(),
                turn_id: "turn-1".to_string(),
            })
            .await
            .expect("terminal query"),
        LocalEventQueryResult::TerminalByTurn(None)
    ));
    assert!(matches!(
        harness
            .store
            .query(LocalEventQuery::ShutdownPlanPage {
                plan: b050_shutdown_plan_key(),
                limit: 1,
                cursor: None,
            })
            .await,
        Err(LocalEventQueryError::NotFound)
    ));
}

#[tokio::test]
async fn failure_before_begin_leaves_store_unchanged() {
    let harness = Harness::open();
    harness.fault.arm_fail_before_begin();
    assert!(matches!(
        harness
            .store
            .commit_batch(multi_stream_batch("commit-1", "key-1"))
            .await,
        Err(CommitBatchError::StorageUnavailable { .. })
    ));
    assert_store_unchanged(&harness, "commit-1").await;
    // The same identity retries cleanly afterwards.
    assert!(harness
        .store
        .commit_batch(multi_stream_batch("commit-1", "key-1"))
        .await
        .is_ok());
}

#[tokio::test]
async fn b050_failure_after_each_participant_write_rolls_back_the_complete_batch() {
    for write_number in 1..=B050_PARTICIPANT_WRITES {
        let harness = Harness::open();
        harness
            .fault
            .arm_fail_after_participant_write_number(write_number);
        assert!(matches!(
            harness
                .store
                .commit_batch(multi_stream_batch("commit-1", "key-1"))
                .await,
            Err(CommitBatchError::StorageUnavailable { .. })
        ));
        assert_store_unchanged(&harness, "commit-1").await;
        let retry = harness
            .store
            .commit_batch(multi_stream_batch("commit-1", "key-1"))
            .await
            .expect("retry");
        let CommitBatchResult::Committed(committed) = retry else {
            panic!("retry after participant {write_number} did not commit");
        };
        assert_eq!(committed.event_count, 4);
        assert_eq!(committed.mutation_count, 4);
    }
}

#[tokio::test]
async fn failure_before_commit_rolls_back_all_participants() {
    let harness = Harness::open();
    harness.fault.arm_fail_before_commit();
    assert!(matches!(
        harness
            .store
            .commit_batch(multi_stream_batch("commit-1", "key-1"))
            .await,
        Err(CommitBatchError::StorageUnavailable { .. })
    ));
    assert_store_unchanged(&harness, "commit-1").await;
}

#[tokio::test]
async fn crash_between_commit_and_readback_is_outcome_unknown_then_resolves() {
    let harness = Harness::open();
    harness.fault.arm_crash_after_commit_before_readback();
    match harness
        .store
        .commit_batch(multi_stream_batch("commit-1", "key-1"))
        .await
    {
        Err(CommitBatchError::OutcomeUnknown { identity }) => {
            assert_eq!(identity.as_str(), "commit-1");
        }
        other => panic!("expected OutcomeUnknown, got {other:?}"),
    }
    // Same-identity resolution converges on the committed result.
    let resolution = harness
        .store
        .resolve_commit(CommitIdentity::parse("commit-1").unwrap())
        .await
        .expect("resolve");
    let CommitResolution::Committed(committed) = resolution else {
        panic!("expected Committed");
    };
    assert_eq!(committed.event_count, 4);
    // Retrying the same batch replays the same result.
    let retry = harness
        .store
        .commit_batch(multi_stream_batch("commit-1", "key-1"))
        .await
        .expect("retry");
    let CommitBatchResult::Replayed(replayed) = retry else {
        panic!("expected Replayed");
    };
    assert_eq!(replayed, committed);
}

#[tokio::test]
async fn reply_loss_is_outcome_unknown_and_resolves_to_committed() {
    let harness = Harness::open();
    harness.fault.arm_drop_reply();
    match harness
        .store
        .commit_batch(multi_stream_batch("commit-1", "key-1"))
        .await
    {
        Err(CommitBatchError::OutcomeUnknown { identity }) => {
            assert_eq!(identity.as_str(), "commit-1");
        }
        other => panic!("expected OutcomeUnknown, got {other:?}"),
    }
    let resolution = harness
        .store
        .resolve_commit(CommitIdentity::parse("commit-1").unwrap())
        .await
        .expect("resolve");
    assert!(matches!(resolution, CommitResolution::Committed(_)));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn five_hundred_concurrent_operation_acceptances_are_sealed_once_and_point_queryable() {
    let harness = Harness::open();
    let mut tasks = Vec::with_capacity(500);
    for ordinal in 0..500 {
        let store = harness.store.clone();
        tasks.push(tokio::spawn(async move {
            let session_id = format!("concurrent-session-{ordinal}");
            let operation_id = format!("concurrent-operation-{ordinal}");
            store
                .commit_batch(batch(
                    &format!("concurrent-commit-{ordinal}"),
                    &format!("concurrent-key-{ordinal}"),
                    [ordinal as u8; 32],
                    vec![head(StreamId::agent_session(&session_id).unwrap(), 0)],
                    vec![session_event(&session_id, ordinal as i64 + 1)],
                    vec![LocalStateMutation::OperationRecord(
                        OperationRecordMutation {
                            kind: OperationKind::Send,
                            operation_id: operation_id.clone(),
                            receipt: payload(
                                &serde_json::json!({
                                    "schema": "send_receipt_v1",
                                    "operation_id": operation_id,
                                    "session_id": session_id,
                                    "input_ref": "input-1",
                                    "disposition": { "type": "started_turn", "turn_id": "1" },
                                    "principal_mac": "00".repeat(32),
                                    "binding_hmac": "00".repeat(32),
                                })
                                .to_string(),
                            ),
                            latest_status: payload(
                                &serde_json::json!({
                                    "schema": "send_status_v1",
                                    "status": { "type": "awaiting_provider_start", "dependency_obligation_ids": [] },
                                })
                                .to_string(),
                            ),
                            expected: RevisionGuard::Absent,
                            revision: Revision::new(0).unwrap(),
                        },
                    )],
                ))
                .await
        }));
    }

    for task in tasks {
        assert!(matches!(
            task.await.expect("acceptance task"),
            Ok(CommitBatchResult::Committed(_))
        ));
    }

    let mut query_samples = Vec::with_capacity(500);
    for ordinal in 0..500 {
        let started = Instant::now();
        let result = harness
            .store
            .query(LocalEventQuery::OperationByIdentity {
                kind: OperationKind::Send,
                operation_id: format!("concurrent-operation-{ordinal}"),
            })
            .await
            .expect("accepted operation point query");
        assert!(matches!(
            result,
            LocalEventQueryResult::OperationByIdentity(Some(_))
        ));
        query_samples.push(started.elapsed().as_micros());
    }
    let mut p95 = query_samples.clone();
    assert!(percentile_micros(&mut p95, 95) <= 150_000);
    assert!(percentile_micros(&mut query_samples, 99) <= 300_000);

    let connection = harness.raw_connection();
    let (operations, events): (i64, i64) = connection
        .query_row(
            "SELECT
                (SELECT COUNT(*) FROM operation_records
                 WHERE kind = 'send' AND operation_id LIKE 'concurrent-operation-%'),
                (SELECT COUNT(*) FROM events)",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("concurrent acceptance counts");
    assert_eq!(operations, 500);
    assert_eq!(events, 500);
}

#[tokio::test]
async fn batch_capacity_limits_reject_before_admission() {
    let harness = Harness::open();
    let events: Vec<UncommittedDomainEvent> = (0..MAX_BATCH_EVENTS + 1)
        .map(|index| application_event(index as i64))
        .collect();
    let over_events = batch(
        "commit-1",
        "key-1",
        [1; 32],
        vec![head(StreamId::application(), 0)],
        events,
        vec![],
    );
    assert!(matches!(
        harness.store.commit_batch(over_events).await,
        Err(CommitBatchError::CapacityExceeded)
    ));

    let mutations: Vec<LocalStateMutation> = (0..MAX_BATCH_STATE_MUTATIONS + 1)
        .map(|index| obligation_mutation(&format!("ob-{index}"), false))
        .collect();
    let over_mutations = batch("commit-1", "key-1", [1; 32], vec![], vec![], mutations);
    assert!(matches!(
        harness.store.commit_batch(over_mutations).await,
        Err(CommitBatchError::CapacityExceeded)
    ));
    // Nothing was admitted or committed.
    assert_store_unchanged(&harness, "commit-1").await;
}

#[tokio::test]
async fn two_thousand_five_hundred_deltas_and_a_64_kib_final_projection_commit_atomically() {
    let mut registry = EventCodecRegistry::new();
    registry.register(Arc::new(AgentSessionEventCodec));
    let harness = Harness::open_with_registry(Arc::new(registry));
    let session_id = "delta-finalization-session";
    let stream_id = StreamId::agent_session(session_id).unwrap();
    let events = (0..2_500)
        .map(|ordinal| UncommittedDomainEvent {
            stream_id: stream_id.clone(),
            event: LocalDomainEvent::AgentSession(AgentSessionDomainEvent::TextRecorded {
                turn_id: 1,
                message_id: "delta-finalization-message".to_string(),
                content: format!("delta-{ordinal}"),
                parent_tool_use_id: None,
            }),
            occurred_at_ms: ordinal,
        })
        .collect();
    let final_content = "x".repeat(64 * 1024);
    let committed = harness
        .store
        .commit_batch(batch(
            "delta-finalization-commit",
            "delta-finalization-key",
            [42; 32],
            vec![head(stream_id, 0)],
            events,
            vec![LocalStateMutation::MessageProjection(
                MessageProjectionMutation {
                    session_id: session_id.to_string(),
                    message_id: "delta-finalization-message".to_string(),
                    projection: agent_message_projection(
                        "delta-finalization-message",
                        &final_content,
                    ),
                    expected: RevisionGuard::Absent,
                    revision: Revision::new(0).unwrap(),
                },
            )],
        ))
        .await
        .expect("atomic delta finalization");
    let CommitBatchResult::Committed(committed) = committed else {
        panic!("first delta finalization must commit");
    };
    assert_eq!(committed.event_count, 2_500);

    let result = harness
        .store
        .query(LocalEventQuery::MessageProjectionByIdentity {
            session_id: session_id.to_string(),
            message_id: "delta-finalization-message".to_string(),
        })
        .await
        .expect("final projection query");
    let LocalEventQueryResult::MessageProjectionByIdentity(Some(projection)) = result else {
        panic!("final projection is absent");
    };
    let MessageProjectionRecord::AgentMessage(projection) = projection.projection else {
        panic!("wrong final projection family");
    };
    assert_eq!(projection.content, final_content);

    let page = harness
        .store
        .load_stream(LoadStreamRequest {
            stream_id: StreamId::agent_session(session_id).unwrap(),
            after: Some(crate::domain::local_event::StreamSequence::new(2_499).unwrap()),
            limit: 1,
        })
        .await
        .expect("last delta query");
    assert_eq!(page.head.value(), 2_500);
    assert_eq!(page.events.len(), 1);
    assert_eq!(page.events[0].stream_sequence.value(), 2_500);
}

#[test]
fn commit_seal_event_count_is_bounded_by_the_global_sequence_primary_key() {
    let harness = Harness::open();
    let connection = harness.raw_connection();
    let mut statement = connection
        .prepare(&format!("EXPLAIN QUERY PLAN {SQL_SEAL_EVENT_COUNT}"))
        .expect("seal event count query plan");
    let steps = statement
        .query_map(rusqlite::params![1_i64, 2_i64, "commit-1"], |row| {
            row.get::<_, String>(3)
        })
        .expect("seal plan rows")
        .collect::<Result<Vec<_>, _>>()
        .expect("seal plan decode");

    assert!(
        steps
            .iter()
            .any(|step| step.contains("SEARCH events USING INTEGER PRIMARY KEY")),
        "unexpected seal plan: {steps:?}"
    );
    assert!(!steps.iter().any(|step| step.contains("SCAN events")));
}

#[tokio::test]
async fn global_sequence_exhaustion_is_typed_before_overflow() {
    let harness = Harness::open();
    harness
        .raw_connection()
        .execute(
            "UPDATE store_metadata SET next_global_sequence = ?1 WHERE id = 1",
            rusqlite::params![i64::MAX],
        )
        .expect("set boundary");
    let two_events = batch(
        "commit-1",
        "key-1",
        [1; 32],
        vec![head(StreamId::application(), 0)],
        vec![application_event(1), application_event(2)],
        vec![],
    );
    assert!(matches!(
        harness.store.commit_batch(two_events).await,
        Err(CommitBatchError::SequenceExhausted)
    ));
}

#[tokio::test]
async fn stream_head_exhaustion_is_typed_before_overflow() {
    let harness = Harness::open();
    // Seed one commit so the referenced commit row exists, then push the
    // stream head to the boundary directly.
    harness
        .store
        .commit_batch(batch(
            "commit-seed",
            "key-seed",
            [1; 32],
            vec![head(StreamId::application(), 0)],
            vec![application_event(1)],
            vec![],
        ))
        .await
        .expect("seed");
    harness
        .raw_connection()
        .execute(
            "UPDATE stream_heads SET head = ?1 WHERE stream_id = 'application'",
            rusqlite::params![i64::MAX],
        )
        .expect("set boundary");
    let overflowing = batch(
        "commit-1",
        "key-1",
        [1; 32],
        vec![head(StreamId::application(), i64::MAX)],
        vec![application_event(2)],
        vec![],
    );
    assert!(matches!(
        harness.store.commit_batch(overflowing).await,
        Err(CommitBatchError::SequenceExhausted)
    ));
}

#[tokio::test]
async fn sequences_are_allocated_exactly_once_across_commits() {
    let harness = Harness::open();
    let first = harness
        .store
        .commit_batch(batch(
            "commit-1",
            "key-1",
            [1; 32],
            vec![head(StreamId::application(), 0)],
            vec![application_event(1)],
            vec![],
        ))
        .await
        .expect("first");
    let second = harness
        .store
        .commit_batch(batch(
            "commit-2",
            "key-2",
            [2; 32],
            vec![head(StreamId::application(), 1)],
            vec![application_event(2), application_event(3)],
            vec![],
        ))
        .await
        .expect("second");
    assert_eq!(first.batch().sequence_range.unwrap().0.value(), 1);
    assert_eq!(first.batch().sequence_range.unwrap().1.value(), 1);
    assert_eq!(second.batch().sequence_range.unwrap().0.value(), 2);
    assert_eq!(second.batch().sequence_range.unwrap().1.value(), 3);
}

// --- Unknown events stay raw ---

#[tokio::test]
async fn unknown_event_type_is_preserved_raw_and_surfaced_as_unknown() {
    let harness = Harness::open();
    harness
        .store
        .commit_batch(batch(
            "commit-1",
            "key-1",
            [1; 32],
            vec![head(StreamId::agent_session("s-1").unwrap(), 0)],
            vec![session_event("s-1", 5)],
            vec![],
        ))
        .await
        .expect("commit");
    let database_path = harness.database_path();
    // Keep the temp directory alive while releasing the store (and its
    // exclusive writer lock) so the same app-data can be reopened.
    let Harness {
        _dir: keepalive_dir,
        root,
        store,
        ..
    } = harness;
    drop(store);
    let _keepalive = keepalive_dir;

    // Reopen with a registry that does not know the test event type.
    let clock = FakeStoreClock::at(2_000);
    let store = LocalEventStore::open(LocalEventStoreConfig {
        app_data_root: root,
        clock: Arc::new(clock),
        registry: Arc::new(EventCodecRegistry::new()),
        fault: Arc::new(FaultInjector::new()),
        path_observer: Arc::new(
            crate::adaptor::gateway::local_event_store::layout::NoopStorePathObserver,
        ),
    })
    .expect("reopen");
    let page = store
        .load_stream(LoadStreamRequest {
            stream_id: StreamId::agent_session("s-1").unwrap(),
            after: None,
            limit: 10,
        })
        .await
        .expect("stream");
    let LoadedDomainEvent::Unknown {
        event_type,
        payload_version,
    } = &page.events[0].event
    else {
        panic!("expected Unknown loaded event");
    };
    assert_eq!(event_type, "test.agent_session.recovery_started");
    assert_eq!(*payload_version, 1);

    // The raw envelope bytes are untouched.
    let connection = open_reader(&database_path).expect("reader");
    let envelope = read_envelope(&connection, page.events[0].event_id.as_str())
        .expect("read envelope")
        .expect("envelope present");
    let unknown = StoredUnknownEvent { envelope };
    assert_eq!(
        unknown.envelope.event_type,
        "test.agent_session.recovery_started"
    );
    assert_eq!(
        unknown.envelope.payload_sha256,
        <[u8; 32]>::from(sha2::Sha256::digest(&unknown.envelope.payload))
    );
}

// --- Cursor integration ---

#[tokio::test]
async fn pending_recovery_cursor_pages_and_rejects_tampering() {
    let harness = Harness::open();
    let mutations: Vec<LocalStateMutation> = (0..5)
        .map(|index| obligation_mutation(&format!("ob-{index}"), true))
        .collect();
    harness
        .store
        .commit_batch(batch(
            "commit-1",
            "key-1",
            [1; 32],
            vec![],
            vec![],
            mutations,
        ))
        .await
        .expect("commit");

    let LocalEventQueryResult::PendingRecoveryPage(first_page) = harness
        .store
        .query(LocalEventQuery::PendingRecoveryPage {
            limit: 2,
            partition: None,
            owner: None,
            ordered_key_prefix: None,
            shutdown_plan: None,
            cursor: None,
        })
        .await
        .expect("first page")
    else {
        panic!("unexpected result");
    };
    assert_eq!(first_page.entries.len(), 2);
    let cursor = first_page.next_cursor.expect("next cursor");

    let LocalEventQueryResult::PendingRecoveryPage(second_page) = harness
        .store
        .query(LocalEventQuery::PendingRecoveryPage {
            limit: 2,
            partition: None,
            owner: None,
            ordered_key_prefix: None,
            shutdown_plan: None,
            cursor: Some(cursor.clone()),
        })
        .await
        .expect("second page")
    else {
        panic!("unexpected result");
    };
    assert_eq!(
        second_page
            .entries
            .iter()
            .map(|entry| entry.obligation_id.as_str())
            .collect::<Vec<_>>(),
        vec!["ob-2", "ob-3"]
    );

    // A different filter with the same cursor is CursorMismatch.
    assert!(matches!(
        harness
            .store
            .query(LocalEventQuery::PendingRecoveryPage {
                limit: 2,
                partition: Some(PendingPartition::Owner),
                owner: None,
                ordered_key_prefix: None,
                shutdown_plan: None,
                cursor: Some(cursor.clone()),
            })
            .await,
        Err(LocalEventQueryError::CursorMismatch)
    ));

    // Tampered token is CursorMismatch.
    let mut tampered = cursor.as_str().to_string();
    let head_char = tampered.remove(0);
    tampered.insert(0, if head_char == 'A' { 'B' } else { 'A' });
    assert!(matches!(
        harness
            .store
            .query(LocalEventQuery::PendingRecoveryPage {
                limit: 2,
                partition: None,
                owner: None,
                ordered_key_prefix: None,
                shutdown_plan: None,
                cursor: Some(QueryCursor::from_opaque(tampered)),
            })
            .await,
        Err(LocalEventQueryError::CursorExpired) | Err(LocalEventQueryError::CursorMismatch)
    ));

    // Expiry through the fake clock is CursorExpired.
    harness.clock.advance_ms(10 * 60 * 1_000);
    assert!(matches!(
        harness
            .store
            .query(LocalEventQuery::PendingRecoveryPage {
                limit: 2,
                partition: None,
                owner: None,
                ordered_key_prefix: None,
                shutdown_plan: None,
                cursor: Some(cursor),
            })
            .await,
        Err(LocalEventQueryError::CursorExpired)
    ));

    // Over-limit page requests are invalid.
    assert!(matches!(
        harness
            .store
            .query(LocalEventQuery::PendingRecoveryPage {
                limit: 201,
                partition: None,
                owner: None,
                ordered_key_prefix: None,
                shutdown_plan: None,
                cursor: None,
            })
            .await,
        Err(LocalEventQueryError::InvalidRequest)
    ));
}

#[tokio::test]
async fn b038_current_recovery_cursor_holds_first_page_snapshot_across_update_and_restart() {
    let harness = Harness::open();
    let mutations = (0..201)
        .map(|ordinal| obligation_mutation(&format!("b038-{ordinal:03}"), true))
        .collect();
    harness
        .store
        .commit_batch(batch(
            "commit-b038-seed",
            "key-b038-seed",
            [38; 32],
            vec![],
            vec![],
            mutations,
        ))
        .await
        .expect("seed 201 current recovery entries");

    let LocalEventQueryResult::PendingRecoveryPage(first) = harness
        .store
        .query(LocalEventQuery::PendingRecoveryPage {
            limit: 200,
            partition: None,
            owner: None,
            ordered_key_prefix: None,
            shutdown_plan: None,
            cursor: None,
        })
        .await
        .expect("first fixed page")
    else {
        panic!("unexpected first page result");
    };
    assert_eq!(first.entries.len(), 200);
    let cursor = first.next_cursor.expect("entry 201 cursor");

    harness
        .store
        .commit_batch(batch(
            "commit-b038-update",
            "key-b038-update",
            [39; 32],
            vec![],
            vec![],
            vec![
                obligation_progress_mutation("b038-200", false),
                obligation_mutation("b038-new", true),
            ],
        ))
        .await
        .expect("update current inventory between pages");

    let LocalEventQueryResult::PendingRecoveryPage(second) = harness
        .store
        .query(LocalEventQuery::PendingRecoveryPage {
            limit: 200,
            partition: None,
            owner: None,
            ordered_key_prefix: None,
            shutdown_plan: None,
            cursor: Some(cursor.clone()),
        })
        .await
        .expect("second fixed page")
    else {
        panic!("unexpected second page result");
    };
    assert_eq!(second.entries.len(), 1);
    assert_eq!(second.entries[0].obligation_id, "b038-200");
    assert!(matches!(
        &second.entries[0].record,
        ObligationRecord::Send {
            state: crate::domain::local_event::ObligationStateRecord::Prepared,
            ..
        }
    ));
    assert!(second.next_cursor.is_none());

    assert!(matches!(
        harness
            .store
            .query(LocalEventQuery::PendingRecoveryPage {
                limit: 200,
                partition: Some(PendingPartition::Owner),
                owner: None,
                ordered_key_prefix: None,
                shutdown_plan: None,
                cursor: Some(cursor.clone()),
            })
            .await,
        Err(LocalEventQueryError::CursorMismatch)
    ));
    let mut tampered = cursor.as_str().to_string();
    let first_byte = tampered.remove(0);
    tampered.insert(0, if first_byte == 'A' { 'B' } else { 'A' });
    assert!(matches!(
        harness
            .store
            .query(LocalEventQuery::PendingRecoveryPage {
                limit: 200,
                partition: None,
                owner: None,
                ordered_key_prefix: None,
                shutdown_plan: None,
                cursor: Some(QueryCursor::from_opaque(tampered)),
            })
            .await,
        Err(LocalEventQueryError::CursorMismatch)
    ));

    let Harness {
        _dir: keepalive_dir,
        root,
        store,
        ..
    } = harness;
    drop(store);
    let _keepalive = keepalive_dir;
    let reopened = LocalEventStore::open(LocalEventStoreConfig {
        app_data_root: root,
        clock: Arc::new(FakeStoreClock::at(1_000)),
        registry: test_registry(),
        fault: Arc::new(FaultInjector::new()),
        path_observer: Arc::new(
            crate::adaptor::gateway::local_event_store::layout::NoopStorePathObserver,
        ),
    })
    .expect("reopen with a new boot identity");
    assert!(matches!(
        reopened
            .query(LocalEventQuery::PendingRecoveryPage {
                limit: 200,
                partition: None,
                owner: None,
                ordered_key_prefix: None,
                shutdown_plan: None,
                cursor: Some(cursor),
            })
            .await,
        Err(LocalEventQueryError::CursorExpired)
    ));
}

#[tokio::test]
async fn pending_recovery_global_prefix_pages_past_200_without_mixed_categories() {
    let harness = Harness::open();
    let mut mutations = Vec::new();
    for ordinal in 0..205 {
        mutations.push(LocalStateMutation::Obligation(ObligationMutation {
            obligation_id: format!("permission-bulk-{ordinal:03}"),
            record: send_obligation_fixture(
                &format!("permission-bulk-{ordinal:03}"),
                ObligationStateRecord::Pending,
            ),
            pending: Some(PendingIndexEntry {
                ordered_key: format!("permission-response:{ordinal:03}"),
                owner: format!("session-{}", ordinal % 7),
                partition: PendingPartition::Owner,
                shutdown_plan: None,
            }),
            expected: RevisionGuard::Absent,
            revision: Revision::new(0).unwrap(),
        }));
    }
    for ordinal in 0..75 {
        mutations.push(LocalStateMutation::Obligation(ObligationMutation {
            obligation_id: format!("stop-bulk-{ordinal:03}"),
            record: send_obligation_fixture(
                &format!("stop-bulk-{ordinal:03}"),
                ObligationStateRecord::Pending,
            ),
            pending: Some(PendingIndexEntry {
                ordered_key: format!("stop-target:{ordinal:03}"),
                owner: format!("session-{}", ordinal % 5),
                partition: PendingPartition::Owner,
                shutdown_plan: None,
            }),
            expected: RevisionGuard::Absent,
            revision: Revision::new(0).unwrap(),
        }));
    }
    harness
        .store
        .commit_batch(batch(
            "commit-permission-prefix-pages",
            "key-permission-prefix-pages",
            [91; 32],
            vec![],
            vec![],
            mutations,
        ))
        .await
        .expect("mixed pending obligations");

    let LocalEventQueryResult::PendingRecoveryPage(first) = harness
        .store
        .query(LocalEventQuery::PendingRecoveryPage {
            limit: 200,
            partition: None,
            owner: None,
            ordered_key_prefix: Some("permission-response:".to_string()),
            shutdown_plan: None,
            cursor: None,
        })
        .await
        .expect("first permission namespace page")
    else {
        panic!("unexpected first permission page result");
    };
    assert_eq!(first.entries.len(), 200);
    assert!(first
        .entries
        .iter()
        .all(|entry| entry.ordered_key.starts_with("permission-response:")));
    let cursor = first.next_cursor.expect("permission page 201 cursor");

    let LocalEventQueryResult::PendingRecoveryPage(second) = harness
        .store
        .query(LocalEventQuery::PendingRecoveryPage {
            limit: 200,
            partition: None,
            owner: None,
            ordered_key_prefix: Some("permission-response:".to_string()),
            shutdown_plan: None,
            cursor: Some(cursor),
        })
        .await
        .expect("second permission namespace page")
    else {
        panic!("unexpected second permission page result");
    };
    assert_eq!(second.entries.len(), 5);
    assert!(second.next_cursor.is_none());
    assert!(second
        .entries
        .iter()
        .all(|entry| entry.ordered_key.starts_with("permission-response:")));

    let connection = harness.raw_connection();
    let mut statement = connection
        .prepare(&format!(
            "EXPLAIN QUERY PLAN {SQL_PENDING_FIRST_PAGE_PREFIX}"
        ))
        .expect("permission prefix query plan");
    let steps = statement
        .query_map(
            rusqlite::params!["", "permission-response:", "permission-response;", 201_i64],
            |row| row.get::<_, String>(3),
        )
        .expect("permission prefix plan rows")
        .collect::<Result<Vec<_>, _>>()
        .expect("permission prefix plan decode");
    assert!(
        steps
            .iter()
            .any(|step| step.contains("SEARCH po USING INDEX")),
        "unexpected permission prefix plan: {steps:?}"
    );
    assert!(!steps.iter().any(|step| step.contains("SCAN po")));
}

#[tokio::test]
async fn pending_recovery_shutdown_plan_filter_is_exact_and_cursor_bound() {
    let harness = Harness::open();
    harness
        .store
        .commit_batch(batch(
            "commit-plan-filter",
            "key-plan-filter",
            [17; 32],
            vec![],
            vec![],
            vec![
                obligation_mutation_for_plan("ob-plan-a", "plan-1"),
                obligation_mutation_for_plan("ob-plan-b", "plan-1"),
                obligation_mutation_for_plan("ob-plan-other", "plan-2"),
                obligation_mutation("ob-unassociated", true),
            ],
        ))
        .await
        .expect("commit plan-filter fixtures");

    let plan = ShutdownPlanKey {
        shutdown_id: "plan-1".to_string(),
    };
    let LocalEventQueryResult::PendingRecoveryPage(first) = harness
        .store
        .query(LocalEventQuery::PendingRecoveryPage {
            limit: 1,
            partition: None,
            owner: None,
            ordered_key_prefix: None,
            shutdown_plan: Some(plan.clone()),
            cursor: None,
        })
        .await
        .expect("first associated page")
    else {
        panic!("unexpected result");
    };
    assert_eq!(first.entries.len(), 1);
    assert_eq!(first.entries[0].shutdown_plan, Some(plan));
    let cursor = first.next_cursor.expect("second associated page");

    let LocalEventQueryResult::PendingRecoveryPage(second) = harness
        .store
        .query(LocalEventQuery::PendingRecoveryPage {
            limit: 1,
            partition: None,
            owner: None,
            ordered_key_prefix: None,
            shutdown_plan: Some(ShutdownPlanKey {
                shutdown_id: "plan-1".to_string(),
            }),
            cursor: Some(cursor.clone()),
        })
        .await
        .expect("second associated page")
    else {
        panic!("unexpected result");
    };
    assert_eq!(second.entries.len(), 1);
    assert!(second.next_cursor.is_none());

    assert!(matches!(
        harness
            .store
            .query(LocalEventQuery::PendingRecoveryPage {
                limit: 1,
                partition: None,
                owner: None,
                ordered_key_prefix: None,
                shutdown_plan: Some(ShutdownPlanKey {
                    shutdown_id: "plan-2".to_string(),
                }),
                cursor: Some(cursor),
            })
            .await,
        Err(LocalEventQueryError::CursorMismatch)
    ));
}

#[tokio::test]
async fn b089_shutdown_recovery_snapshot_is_identity_partition_and_cursor_bound() {
    let harness = Harness::open();
    let plan = ShutdownPlanKey {
        shutdown_id: "plan-snapshot".to_string(),
    };
    let other_plan = ShutdownPlanKey {
        shutdown_id: "plan-other-snapshot".to_string(),
    };
    let snapshot_id = "snapshot-1";
    let summary: ShutdownPlanRecord = payload(
        &serde_json::json!({
            "schema": "shutdown_plan_summary_v1",
            "operation_id": "quit-snapshot",
            "intent": "exit",
            "exit_code": 0,
            "t0_ms": 1,
            "deadline_ms": 15_001,
            "target_count": 0,
            "recovery_snapshot_count": 3,
            "recovery_snapshot_id": snapshot_id,
        })
        .to_string(),
    );
    let snapshot = |partition, ordinal, obligation_id: &str| {
        LocalStateMutation::ShutdownRecoverySnapshot(ShutdownRecoverySnapshotMutation {
            key: plan.clone(),
            partition,
            ordinal,
            detail: payload(
                &serde_json::json!({
                    "schema": "shutdown_recovery_snapshot_v1",
                    "obligation_id": obligation_id,
                    "ordered_key": format!("0000-{obligation_id}"),
                    "owner": obligation_id,
                    "revision": 0,
                    "record": serde_json::json!({
                        "schema": "turn_execution_obligation_v1",
                        "operation_id": format!("operation-{obligation_id}"),
                        "session_id": "snapshot-session",
                        "turn_id": obligation_id,
                        "state": "pending",
                    }).to_string(),
                })
                .to_string(),
            ),
        })
    };
    let mut current_archived = obligation_mutation("current-archived-only", true);
    let LocalStateMutation::Obligation(current_archived_mutation) = &mut current_archived else {
        unreachable!("obligation helper always returns an obligation mutation");
    };
    let current_archived_pending = current_archived_mutation
        .pending
        .as_mut()
        .expect("current archived pending fixture");
    current_archived_pending.owner = "current-archived-session".to_string();
    current_archived_pending.partition = PendingPartition::ArchivedSession;
    let other_summary = payload(
        &serde_json::json!({
            "schema": "shutdown_plan_summary_v1",
            "operation_id": "quit-other-snapshot",
            "intent": "exit",
            "exit_code": 0,
            "t0_ms": 1,
            "deadline_ms": 15_001,
            "target_count": 0,
            "recovery_snapshot_count": 0,
            "recovery_snapshot_id": "snapshot-other",
        })
        .to_string(),
    );
    harness
        .store
        .commit_batch(batch(
            "commit-snapshot",
            "key-snapshot",
            [18; 32],
            vec![],
            vec![],
            vec![
                LocalStateMutation::ShutdownPlan(ShutdownPlanMutation {
                    key: plan.clone(),
                    phase: ApplicationShutdownPhase::Completed,
                    summary: summary.clone(),
                    details_state: ShutdownDetailsState::Available,
                    expected: RevisionGuard::Absent,
                    revision: Revision::new(0).unwrap(),
                }),
                LocalStateMutation::ShutdownPlan(ShutdownPlanMutation {
                    key: other_plan.clone(),
                    phase: ApplicationShutdownPhase::Completed,
                    summary: other_summary,
                    details_state: ShutdownDetailsState::Available,
                    expected: RevisionGuard::Absent,
                    revision: Revision::new(0).unwrap(),
                }),
                snapshot(PendingPartition::ClosedSession, 0, "closed-1"),
                snapshot(PendingPartition::ClosedSession, 1, "closed-2"),
                snapshot(PendingPartition::UnownedRuntime, 2, "orphan-1"),
                current_archived,
            ],
        ))
        .await
        .expect("commit snapshot fixtures");

    let LocalEventQueryResult::PendingRecoverySnapshotPage(closed) = harness
        .store
        .query(LocalEventQuery::PendingRecoverySnapshotPage {
            plan: plan.clone(),
            snapshot_id: snapshot_id.to_string(),
            partition: PendingPartition::ClosedSession,
            limit: 1,
            cursor: None,
        })
        .await
        .expect("closed snapshot")
    else {
        panic!("unexpected result");
    };
    assert_eq!(closed.entries.len(), 1);
    let cursor = closed.next_cursor.expect("second closed page");

    let LocalEventQueryResult::PendingRecoverySnapshotPage(archived) = harness
        .store
        .query(LocalEventQuery::PendingRecoverySnapshotPage {
            plan: plan.clone(),
            snapshot_id: snapshot_id.to_string(),
            partition: PendingPartition::ArchivedSession,
            limit: 200,
            cursor: None,
        })
        .await
        .expect("empty archived snapshot")
    else {
        panic!("unexpected result");
    };
    assert!(archived.entries.is_empty());
    assert!(archived.next_cursor.is_none());

    let LocalEventQueryResult::PendingRecoverySnapshotPage(unowned) = harness
        .store
        .query(LocalEventQuery::PendingRecoverySnapshotPage {
            plan: plan.clone(),
            snapshot_id: snapshot_id.to_string(),
            partition: PendingPartition::UnownedRuntime,
            limit: 200,
            cursor: None,
        })
        .await
        .expect("unowned snapshot")
    else {
        panic!("unexpected result");
    };
    assert_eq!(unowned.entries.len(), 1);
    assert!(matches!(
        &unowned.entries[0].detail,
        ShutdownTargetRecord::RecoverySnapshot { obligation_id, .. }
            if obligation_id == "orphan-1"
    ));
    assert!(unowned.next_cursor.is_none());

    assert!(matches!(
        harness
            .store
            .query(LocalEventQuery::PendingRecoverySnapshotPage {
                plan: other_plan,
                snapshot_id: snapshot_id.to_string(),
                partition: PendingPartition::ClosedSession,
                limit: 1,
                cursor: None,
            })
            .await,
        Err(LocalEventQueryError::SnapshotMismatch)
    ));

    assert!(matches!(
        harness
            .store
            .query(LocalEventQuery::PendingRecoverySnapshotPage {
                plan: plan.clone(),
                snapshot_id: "another-snapshot".to_string(),
                partition: PendingPartition::ClosedSession,
                limit: 1,
                cursor: None,
            })
            .await,
        Err(LocalEventQueryError::SnapshotMismatch)
    ));
    assert!(matches!(
        harness
            .store
            .query(LocalEventQuery::PendingRecoverySnapshotPage {
                plan: plan.clone(),
                snapshot_id: snapshot_id.to_string(),
                partition: PendingPartition::UnownedRuntime,
                limit: 1,
                cursor: Some(cursor.clone()),
            })
            .await,
        Err(LocalEventQueryError::CursorMismatch)
    ));

    let tampered = QueryCursor::from_opaque(format!("{}x", cursor.as_str()));
    assert!(matches!(
        harness
            .store
            .query(LocalEventQuery::PendingRecoverySnapshotPage {
                plan: plan.clone(),
                snapshot_id: snapshot_id.to_string(),
                partition: PendingPartition::ClosedSession,
                limit: 1,
                cursor: Some(tampered),
            })
            .await,
        Err(LocalEventQueryError::CursorMismatch)
    ));

    harness.clock.advance_ms(10 * 60 * 1_000);
    assert!(matches!(
        harness
            .store
            .query(LocalEventQuery::PendingRecoverySnapshotPage {
                plan: plan.clone(),
                snapshot_id: snapshot_id.to_string(),
                partition: PendingPartition::ClosedSession,
                limit: 1,
                cursor: Some(cursor),
            })
            .await,
        Err(LocalEventQueryError::CursorExpired)
    ));
    assert!(matches!(
        harness
            .store
            .query(LocalEventQuery::PendingRecoverySnapshotPage {
                plan: plan.clone(),
                snapshot_id: snapshot_id.to_string(),
                partition: PendingPartition::Owner,
                limit: 1,
                cursor: None,
            })
            .await,
        Err(LocalEventQueryError::InvalidRequest)
    ));
}

// --- Query plan snapshots ---

#[tokio::test]
async fn public_query_plans_never_scan_the_events_table() {
    let harness = Harness::open();
    harness
        .store
        .commit_batch(multi_stream_batch("commit-1", "key-1"))
        .await
        .expect("commit");
    let connection = harness.raw_connection();
    let plans = [
        ("pending_first_page", SQL_PENDING_FIRST_PAGE),
        (
            "pending_first_page_partition",
            SQL_PENDING_FIRST_PAGE_PARTITION,
        ),
        ("terminal_lookup", SQL_TERMINAL_LOOKUP),
        ("operation_lookup", SQL_OPERATION_LOOKUP),
    ];
    for (name, sql) in plans {
        let mut statement = connection
            .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
            .expect("plan prepare");
        let column_count = statement.column_count();
        let parameter_count = statement.parameter_count();
        let parameters = std::iter::repeat_n(rusqlite::types::Value::Null, parameter_count);
        let steps: Vec<String> = statement
            .query_map(rusqlite::params_from_iter(parameters), |row| {
                row.get::<_, String>(column_count - 1)
            })
            .expect("plan query")
            .collect::<Result<_, _>>()
            .expect("plan rows");
        assert!(!steps.is_empty(), "plan for {name} is empty");
        for step in &steps {
            assert!(
                !step.contains("SCAN events"),
                "plan for {name} scans events: {steps:?}"
            );
        }
        // Direct lookups must use an index or primary key search.
        assert!(
            steps.iter().any(|step| step.contains("USING")),
            "plan for {name} does not use an index: {steps:?}"
        );
    }
}

fn seed_performance_fixture(harness: &Harness, row_count: usize, identity: &str) {
    let mut connection = harness.raw_connection();
    let transaction = connection
        .transaction()
        .expect("performance seed transaction");
    let commit_id = format!("perf-seed-{identity}");
    transaction
        .execute(
            "INSERT INTO logical_commits
                (commit_id, installation_id, operation_kind, idempotency_key,
                 payload_hash, state, first_global_sequence, last_global_sequence,
                 event_count, mutation_count, stream_heads_json, result_hash,
                 committed_at_ms)
             VALUES (?1, ?2, 'send', ?3, zeroblob(32), 'sealed',
                     1, ?4, ?4, 12, '{}', zeroblob(32), 1)",
            rusqlite::params![
                commit_id,
                harness.store.installation_id(),
                format!("perf-{identity}"),
                row_count as i64
            ],
        )
        .expect("performance logical commit");
    transaction
        .execute(
            "INSERT INTO stream_heads (stream_id, head, updated_commit_id)
             VALUES (?1, ?2, ?3)",
            rusqlite::params![
                format!("agent_session:history-{identity}"),
                row_count as i64,
                commit_id
            ],
        )
        .expect("performance stream head");
    {
        let mut events = transaction
            .prepare(
                "INSERT INTO events
                    (global_sequence, event_id, commit_id, stream_id, stream_sequence,
                     event_type, payload_version, occurred_at, payload, payload_sha256)
                 VALUES (?1, ?2, ?3, ?4, ?1, 'fixture.unrelated_history', 1,
                         '1970-01-01T00:00:00Z', x'80', zeroblob(32))",
            )
            .expect("prepare history events");
        let stream_id = format!("agent_session:history-{identity}");
        for sequence in 1..=row_count {
            events
                .execute(rusqlite::params![
                    sequence as i64,
                    format!("perf-event-{identity}-{sequence:07}"),
                    commit_id,
                    stream_id,
                ])
                .expect("insert history event");
        }
    }
    transaction
        .execute(
            "UPDATE store_metadata SET next_global_sequence = ?1 WHERE id = 1",
            [row_count as i64 + 1],
        )
        .expect("advance performance global sequence");
    transaction
        .execute(
            "INSERT INTO operation_records
                (kind, operation_id, receipt, latest_status, revision, commit_id)
             VALUES ('send', ?1, '{\"receipt\":\"ok\"}',
                     '{\"status\":\"accepted\"}', 0, ?2)",
            rusqlite::params![format!("perf-operation-{identity}"), commit_id],
        )
        .expect("performance operation");
    transaction
        .execute(
            "INSERT INTO terminal_records
                (session_id, turn_id, terminal_identity, result,
                 participant_digest, commit_id)
             VALUES (?1, '1', ?2, '{\"result\":\"completed\"}',
                     zeroblob(32), ?3)",
            rusqlite::params![
                format!("perf-session-{identity}"),
                format!("perf-terminal-{identity}"),
                commit_id,
            ],
        )
        .expect("performance terminal");
    {
        let mut obligations = transaction
            .prepare(
                "INSERT INTO obligations
                    (obligation_id, record, pending, revision, commit_id)
                 VALUES (?1, ?2, ?3, 0, ?4)",
            )
            .expect("prepare obligations");
        let mut pending = transaction
            .prepare(
                "INSERT INTO pending_obligations
                    (ordered_key, obligation_id, owner, partition, shutdown_id, commit_id)
                 VALUES (?1, ?2, ?3, 'owner', NULL, ?4)",
            )
            .expect("prepare pending");
        for index in 0..200 {
            let obligation_id = format!("perf-{identity}-{index:07}");
            let record =
                StoredObligationV1::encode_new(&ObligationRecord::BackendSessionRecovery {
                    session_id: format!("perf-session-{identity}"),
                    recovery_id: obligation_id.clone(),
                    detail:
                        crate::domain::local_event::BackendSessionRecoveryObligationRecord::EffectReserved {
                            old_provider_session_generation: 0,
                            reason: BackendSessionRecoveryReason::BackendSessionLost,
                            reserved_at_bits: 0,
                        },
                    state: ObligationStateRecord::ReconciliationRequired,
                })
                .expect("encode closed performance obligation");
            obligations
                .execute(rusqlite::params![&obligation_id, record, 1_i64, commit_id,])
                .expect("insert obligation");
            pending
                .execute(rusqlite::params![
                    format!("{index:020}"),
                    obligation_id,
                    format!("perf-session-{identity}"),
                    commit_id,
                ])
                .expect("insert pending");
        }
    }
    transaction.commit().expect("commit performance fixture");
    connection
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
        .expect("checkpoint performance fixture before measurement");
}

fn percentile_micros(samples: &mut [u128], percentile: usize) -> u128 {
    samples.sort_unstable();
    let index = (samples.len() * percentile).div_ceil(100).saturating_sub(1);
    samples[index]
}

struct PerformanceRecoveryExecutor;

#[async_trait::async_trait]
impl crate::usecase::agent_session::operation::RecoveryEffectExecutor
    for PerformanceRecoveryExecutor
{
    fn supports_read_again(
        &self,
        _obligation_id: &str,
        _immutable_obligation: &ObligationRecord,
    ) -> bool {
        true
    }

    async fn execute(
        &self,
        _request: &crate::usecase::agent_session::operation::RecoveryEffectRequest,
    ) -> Result<crate::usecase::agent_session::operation::RecoveryEffectResult, SafeOperationFailure>
    {
        unreachable!("the startup pending-page benchmark never starts an effect")
    }
}

async fn sample_pending_usecase(harness: &Harness) -> (u128, u128, usize, usize) {
    let repository: Arc<dyn LocalEventTransactionRepository> = harness.store.clone();
    let authority: Arc<dyn crate::usecase::agent_session::operation::OperationBindingAuthority> =
        harness.store.clone();
    let usecase = crate::usecase::agent_session::operation::RecoveryActionUsecase::new(
        repository,
        authority,
        Arc::new(PerformanceRecoveryExecutor),
        harness.store.installation_id().to_string(),
    );
    let mut samples = Vec::with_capacity(1_000);
    let mut count = 0;
    let mut response_bytes = 0;
    for _ in 0..1_000 {
        let started = Instant::now();
        let page = usecase
            .pending(
                crate::usecase::agent_session::operation::PendingRecoveryQuery {
                    limit: 200,
                    partition: None,
                    owner: None,
                    shutdown_plan: None,
                    cursor: None,
                },
            )
            .await
            .expect("public pending recovery query");
        let dto = crate::adaptor::protocol::agent_session_v1::checked_pending_recovery_page(page)
            .expect("bounded pending page response");
        response_bytes = serde_json::to_vec(&dto)
            .expect("pending page response")
            .len();
        count = dto.entries.len();
        samples.push(started.elapsed().as_micros());
    }
    let mut p95 = samples.clone();
    (
        percentile_micros(&mut p95, 95),
        percentile_micros(&mut samples, 99),
        count,
        response_bytes,
    )
}

struct PerformanceSendGate {
    session_store: Arc<SessionStore>,
    session_id: String,
    effects: std::sync::atomic::AtomicUsize,
    planned: std::sync::atomic::AtomicUsize,
}

struct PerformanceStopGate {
    effects: std::sync::atomic::AtomicUsize,
}

struct OutcomeUnknownStopGate {
    fault: Arc<FaultInjector>,
    effects: std::sync::atomic::AtomicUsize,
}

#[async_trait::async_trait]
impl crate::usecase::agent_session::operation::StopAdmissionGate for PerformanceStopGate {
    async fn target_snapshot(
        &self,
        _session_id: &str,
    ) -> Result<crate::usecase::agent_session::operation::StopTargetSnapshot, SafeOperationFailure>
    {
        Ok(
            crate::usecase::agent_session::operation::StopTargetSnapshot {
                session_revision: 0,
                active_turn_id: "1".to_string(),
                queue_paused: false,
            },
        )
    }

    async fn interrupt(
        &self,
        _effect: &crate::usecase::agent_session::operation::AcceptedStopEffect,
    ) -> Result<crate::usecase::agent_session::operation::StopEffectObservation, SafeOperationFailure>
    {
        self.effects
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(
            crate::usecase::agent_session::operation::StopEffectObservation {
                terminal_reason: Some(crate::domain::agent_session::events::InterruptReason::Abort),
            },
        )
    }
}

#[async_trait::async_trait]
impl crate::usecase::agent_session::operation::StopAdmissionGate for OutcomeUnknownStopGate {
    async fn target_snapshot(
        &self,
        _session_id: &str,
    ) -> Result<crate::usecase::agent_session::operation::StopTargetSnapshot, SafeOperationFailure>
    {
        Ok(
            crate::usecase::agent_session::operation::StopTargetSnapshot {
                session_revision: 0,
                active_turn_id: "1".to_string(),
                queue_paused: false,
            },
        )
    }

    async fn interrupt(
        &self,
        _effect: &crate::usecase::agent_session::operation::AcceptedStopEffect,
    ) -> Result<crate::usecase::agent_session::operation::StopEffectObservation, SafeOperationFailure>
    {
        self.effects
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.fault.arm_crash_after_commit_before_readback();
        Ok(
            crate::usecase::agent_session::operation::StopEffectObservation {
                terminal_reason: Some(crate::domain::agent_session::events::InterruptReason::Abort),
            },
        )
    }
}

#[tokio::test]
async fn stop_terminal_outcome_unknown_resolves_committed_without_reconciliation_overwrite() {
    let harness = Harness::open();
    let gate = Arc::new(OutcomeUnknownStopGate {
        fault: harness.fault.clone(),
        effects: std::sync::atomic::AtomicUsize::new(0),
    });
    let usecase = crate::usecase::agent_session::operation::StopOperationUsecase::new(
        harness.store.clone(),
        harness.store.clone(),
        gate.clone(),
        harness.store.installation_id().to_string(),
    );
    let outcome = usecase
        .request(
            crate::usecase::agent_session::operation::StopOperationRequest {
                principal: "desktop".to_string(),
                request_id: "stop-outcome-unknown".to_string(),
                session_id: "stop-outcome-session".to_string(),
                turn_id: "1".to_string(),
                expected_session_revision: 0,
            },
        )
        .await
        .unwrap();
    let crate::usecase::agent_session::operation::StopCommandOutcome::Accepted { receipt, state } =
        outcome
    else {
        panic!("durably committed Stop terminal must remain Accepted");
    };
    assert_eq!(
        state,
        crate::usecase::agent_session::operation::StopOperationState::Completed {
            resolution: crate::domain::agent_session::events::StopResolution::Succeeded,
        }
    );
    assert_eq!(
        usecase
            .get_operation("desktop", &receipt.operation_id)
            .await
            .unwrap(),
        (
            receipt,
            crate::usecase::agent_session::operation::StopOperationState::Completed {
                resolution: crate::domain::agent_session::events::StopResolution::Succeeded,
            },
        )
    );
    assert_eq!(gate.effects.load(std::sync::atomic::Ordering::SeqCst), 1);
    let connection = harness.raw_connection();
    let terminal_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM terminal_records WHERE session_id = 'stop-outcome-session' AND turn_id = '1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let resolution_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM stop_resolutions", [], |row| {
            row.get(0)
        })
        .unwrap();
    let pending_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM pending_obligations", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(terminal_count, 1);
    assert_eq!(resolution_count, 1);
    assert_eq!(pending_count, 0);
}

struct PerformancePublicMetrics {
    p95: u128,
    p99: u128,
    partial_count: usize,
    effect_count: usize,
    response_bytes: usize,
}

struct PerformanceSendMetrics {
    mutation: PerformancePublicMetrics,
    identity_p95: u128,
    identity_p99: u128,
    identity_count: usize,
    identity_response_bytes: usize,
}

#[async_trait::async_trait]
impl crate::usecase::agent_session::operation::SendAdmissionGate for PerformanceSendGate {
    async fn plan_send(
        &self,
        _principal: &str,
        _operation_id: &str,
        _canonical_payload: &str,
    ) -> Result<crate::usecase::agent_session::operation::SendPlan, SafeOperationFailure> {
        let ordinal = self
            .planned
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            + 1;
        let allocation = self
            .session_store
            .send_acceptance_allocation(&self.session_id)
            .expect("performance send allocation must be readable");
        Ok(crate::usecase::agent_session::operation::SendPlan {
            session_id: self.session_id.clone(),
            initial_session: None,
            session_projection_guard: allocation.session_projection_guard,
            disposition: crate::domain::agent_session::events::SendDisposition::StartedTurn {
                turn_id: allocation.next_turn_id.to_string(),
            },
            input_ref: format!("performance-input-{ordinal}"),
            human_message_id: format!("performance-human-{ordinal}"),
            prompt: crate::domain::agent_session::events::PromptInput {
                content: "performance payload".to_string(),
                ..Default::default()
            },
            reserved_turn_id: None,
        })
    }

    async fn acceptance_state_mutations(
        &self,
        plan: &crate::usecase::agent_session::operation::SendPlan,
        events: &[AgentSessionDomainEvent],
    ) -> Result<Vec<LocalStateMutation>, SafeOperationFailure> {
        self.session_store
            .prepare_send_acceptance_mutations(
                crate::usecase::agent_session::session::SendAcceptanceProjectionInput {
                    session_id: &plan.session_id,
                    initial_session: None,
                    session_projection_guard: plan.session_projection_guard,
                    human_message_id: &plan.human_message_id,
                    prompt: &plan.prompt,
                    disposition: &plan.disposition,
                    reserved_turn_id: plan.reserved_turn_id.as_deref(),
                    input_ref: &plan.input_ref,
                    events,
                },
            )
            .map_err(|error| {
                SafeOperationFailure::new(
                    SessionOperationFailureKind::PersistFailure,
                    false,
                    &error,
                    "performance-send-projection",
                )
            })
    }

    async fn canonical_immediate_turn_is_current(
        &self,
        _session_id: &str,
        _turn_id: u64,
    ) -> Result<bool, SafeOperationFailure> {
        Ok(true)
    }

    async fn start_provider_effect(
        &self,
        _effect: &crate::usecase::agent_session::operation::AcceptedSendEffect,
    ) -> Result<crate::usecase::agent_session::operation::SendEffectDispatch, SafeOperationFailure>
    {
        self.effects
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(crate::usecase::agent_session::operation::SendEffectDispatch::Scheduled)
    }
}

async fn sample_public_mutation(harness: &Harness, identity: &str) -> PerformanceSendMetrics {
    let session_id = uuid::Uuid::new_v4().to_string();
    let session_store = SessionStore::new(Arc::new(FileSessionStorage::default()));
    let repository: Arc<dyn LocalEventTransactionRepository> = harness.store.clone();
    session_store.set_local_event_repository(
        repository,
        harness.store.installation_id().to_string(),
        Arc::new(AgentSessionProjectionCodecV1),
    );
    let session = build_new_session_with_id(
        session_id.clone(),
        "/performance-fixture",
        Some("codex".to_string()),
        crate::domain::agent_session::PermissionMode::Ask,
        None,
        false,
        false,
        None,
    );
    session_store
        .save_full_session_for_restore(&harness.root, &session)
        .expect("seed production session projection");
    let gate = Arc::new(PerformanceSendGate {
        session_store: Arc::new(session_store),
        session_id: session_id.clone(),
        effects: std::sync::atomic::AtomicUsize::new(0),
        planned: std::sync::atomic::AtomicUsize::new(0),
    });
    let authority: Arc<dyn crate::usecase::agent_session::operation::OperationBindingAuthority> =
        harness.store.clone();
    let usecase = crate::usecase::agent_session::operation::AgentSendOperationUsecase::new(
        harness.store.clone(),
        authority,
        gate.clone(),
        harness.store.installation_id().to_string(),
    );
    let mut samples = Vec::with_capacity(1_000);
    let mut response_bytes = 0;
    let mut last_operation_id = String::new();
    for ordinal in 0..1_000 {
        let request = crate::usecase::agent_session::operation::SendOperationRequest {
            principal: "performance-client".to_string(),
            operation_id: format!("performance-{identity}-{ordinal}"),
            // Every sample has the same exact payload but a distinct valid
            // caller identity, so all 1,000 timings include a real commit
            // rather than 999 idempotent point-read replays.
            canonical_payload: "{\"content\":\"performance payload\"}".to_string(),
        };
        let started = Instant::now();
        let outcome = usecase
            .send(request.clone())
            .await
            .expect("same-payload production send");
        let dto: crate::adaptor::protocol::agent_session_v1::SendCommandOutcomeDtoV1 =
            outcome.into();
        let encoded = serde_json::to_vec(&dto).expect("public send result");
        response_bytes = encoded.len();
        samples.push(started.elapsed().as_micros());
        last_operation_id = request.operation_id;
    }
    let mut identity_samples = Vec::with_capacity(1_000);
    let mut identity_response_bytes = 0;
    let mut identity_count = 0;
    for _ in 0..1_000 {
        let started = Instant::now();
        let operation = usecase
            .get_operation("performance-client", &last_operation_id)
            .await
            .expect("public send identity query");
        let dto: crate::adaptor::protocol::agent_session_v1::SendOperationViewDtoV1 =
            operation.into();
        identity_response_bytes = serde_json::to_vec(&dto)
            .expect("public identity response")
            .len();
        identity_count = 1;
        identity_samples.push(started.elapsed().as_micros());
    }
    let projected_message_count: i64 = harness
        .raw_connection()
        .query_row(
            "SELECT COUNT(*) FROM message_projection WHERE session_id = ?1",
            [&session_id],
            |row| row.get(0),
        )
        .expect("public send projection count");
    let committed_operation_count: i64 = harness
        .raw_connection()
        .query_row(
            "SELECT COUNT(*) FROM operation_records
             WHERE kind = 'send' AND operation_id LIKE ?1 ESCAPE '\\'",
            [format!("performance-{identity}-%")],
            |row| row.get(0),
        )
        .expect("public send operation count");
    let partial_count =
        usize::from(committed_operation_count != 1_000 || projected_message_count != 2_000);
    let mut p95 = samples.clone();
    let mut identity_p95 = identity_samples.clone();
    PerformanceSendMetrics {
        mutation: PerformancePublicMetrics {
            p95: percentile_micros(&mut p95, 95),
            p99: percentile_micros(&mut samples, 99),
            partial_count,
            effect_count: gate.effects.load(std::sync::atomic::Ordering::SeqCst),
            response_bytes,
        },
        identity_p95: percentile_micros(&mut identity_p95, 95),
        identity_p99: percentile_micros(&mut identity_samples, 99),
        identity_count,
        identity_response_bytes,
    }
}

async fn sample_terminal_usecase(harness: &Harness, identity: &str) -> PerformancePublicMetrics {
    let gate = Arc::new(PerformanceStopGate {
        effects: std::sync::atomic::AtomicUsize::new(0),
    });
    let authority: Arc<dyn crate::usecase::agent_session::operation::OperationBindingAuthority> =
        harness.store.clone();
    let usecase = crate::usecase::agent_session::operation::StopOperationUsecase::new(
        harness.store.clone(),
        authority,
        gate.clone(),
        harness.store.installation_id().to_string(),
    );
    let request = crate::usecase::agent_session::operation::StopOperationRequest {
        principal: "performance-client".to_string(),
        request_id: format!("terminal-{identity}"),
        session_id: format!("terminal-session-{identity}"),
        turn_id: "1".to_string(),
        expected_session_revision: 0,
    };
    let mut samples = Vec::with_capacity(1_000);
    let mut response_bytes = 0;
    let mut partial_count = 0;
    for _ in 0..1_000 {
        let started = Instant::now();
        let outcome = usecase
            .request(request.clone())
            .await
            .expect("same-turn public Stop request");
        let dto: crate::adaptor::protocol::agent_session_v1::StopCommandOutcomeDtoV1 =
            outcome.into();
        let encoded = serde_json::to_vec(&dto).expect("public Stop result");
        response_bytes = encoded.len();
        partial_count += usize::from(!matches!(
            dto,
            crate::adaptor::protocol::agent_session_v1::StopCommandOutcomeDtoV1::Accepted {
                state:
                    crate::adaptor::protocol::agent_session_v1::StopOperationStateDtoV1::Completed { .. },
                ..
            }
        ));
        samples.push(started.elapsed().as_micros());
    }
    let mut p95 = samples.clone();
    PerformancePublicMetrics {
        p95: percentile_micros(&mut p95, 95),
        p99: percentile_micros(&mut samples, 99),
        partial_count,
        effect_count: gate.effects.load(std::sync::atomic::Ordering::SeqCst),
        response_bytes,
    }
}

fn performance_query_plan_steps(harness: &Harness) -> Vec<String> {
    let connection = harness.raw_connection();
    [
        SQL_PENDING_FIRST_PAGE,
        SQL_PENDING_FIRST_PAGE_PARTITION,
        SQL_TERMINAL_LOOKUP,
        SQL_OPERATION_LOOKUP,
        "SELECT projection, revision FROM session_projection WHERE session_id = ?1",
        "SELECT head FROM stream_heads WHERE stream_id = ?1",
        SQL_SEAL_EVENT_COUNT,
    ]
    .into_iter()
    .flat_map(|sql| {
        let mut statement = connection
            .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
            .expect("performance query plan");
        let column_count = statement.column_count();
        let parameter_count = statement.parameter_count();
        let parameters = std::iter::repeat_n(rusqlite::types::Value::Null, parameter_count);
        statement
            .query_map(rusqlite::params_from_iter(parameters), |row| {
                row.get::<_, String>(column_count - 1)
            })
            .expect("performance plan rows")
            .collect::<Result<Vec<_>, _>>()
            .expect("performance plan decode")
    })
    .collect()
}

/// A production performance sample must not consult the retained legacy
/// directory authority. On Unix, unreadable sentinel directories turn any
/// accidental session/workflow directory scan into an immediate test failure;
/// the SQLite-only call path remains unaffected.
struct LegacyDirectoryScanTripwire {
    #[cfg(unix)]
    paths: Vec<std::path::PathBuf>,
}

impl LegacyDirectoryScanTripwire {
    fn install(root: &std::path::Path) -> Self {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mut paths = Vec::new();
            for name in ["sessions", "workflow_executions"] {
                let path = root.join(name);
                std::fs::create_dir_all(&path).expect("legacy scan tripwire directory");
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000))
                    .expect("arm legacy scan tripwire");
                paths.push(path);
            }
            Self { paths }
        }
        #[cfg(not(unix))]
        {
            let _ = root;
            Self {}
        }
    }
}

impl Drop for LegacyDirectoryScanTripwire {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            for path in &self.paths {
                let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700));
            }
        }
    }
}

async fn assert_reader_failure_contract(harness: &Harness) -> (usize, usize) {
    let pool = harness.store.reader_pool_for_test();
    let executor = TestShutdownExecutor::with_targets(0, ShutdownExecutorMode::Complete);
    let coordinator = shutdown_coordinator(harness, &executor);
    let accepted = coordinator
        .request(
            crate::usecase::shutdown_coordinator::ApplicationQuitRequest {
                principal: "performance-client".to_string(),
                request_id: "shutdown-reader-failure-plan".to_string(),
                intent: crate::usecase::shutdown_coordinator::ApplicationQuitIntent::Exit {
                    code: 0,
                },
            },
        )
        .await
        .expect("seed bounded shutdown snapshot");
    let crate::usecase::shutdown_coordinator::ApplicationQuitOutcome::Accepted { receipt, .. } =
        accepted
    else {
        panic!("shutdown snapshot fixture must be accepted");
    };
    let plan = ShutdownPlanKey {
        shutdown_id: receipt.shutdown_id,
    };
    let effects_before = executor.effects.load(std::sync::atomic::Ordering::SeqCst);
    let subordinate_shutdowns_before = executor
        .subordinate_shutdowns
        .load(std::sync::atomic::Ordering::SeqCst);

    let release = Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
    let started = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut blockers = Vec::new();
    for _ in 0..super::reader::READER_POOL_SIZE {
        let release = release.clone();
        let started = started.clone();
        blockers.push(
            pool.submit(move |_| {
                started.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let (lock, ready) = &*release;
                let released = lock.lock().expect("reader blocker poisoned");
                drop(
                    ready
                        .wait_while(released, |released| !*released)
                        .expect("reader blocker wait poisoned"),
                );
                Ok::<_, LocalEventQueryError>(())
            })
            .expect("install reader blocker"),
        );
    }
    while started.load(std::sync::atomic::Ordering::SeqCst) < super::reader::READER_POOL_SIZE {
        tokio::task::yield_now().await;
    }
    let mut queued = Vec::new();
    for _ in 0..READ_QUEUE_MAX_DEPTH {
        queued.push(
            pool.submit(move |_| Ok::<_, LocalEventQueryError>(()))
                .unwrap(),
        );
    }
    let busy = coordinator
        .shutdown_plan_page_read_model(plan.clone(), 128, None)
        .await;
    assert!(matches!(busy, Err(LocalEventQueryError::QueryBusy)));
    assert_eq!(
        executor.effects.load(std::sync::atomic::Ordering::SeqCst),
        effects_before
    );
    assert_eq!(
        executor
            .subordinate_shutdowns
            .load(std::sync::atomic::Ordering::SeqCst),
        subordinate_shutdowns_before
    );
    {
        let (lock, ready) = &*release;
        *lock.lock().expect("reader release poisoned") = true;
        ready.notify_all();
    }
    drop(queued);
    drop(blockers);
    while pool.queued_len_for_test() != 0 {
        tokio::task::yield_now().await;
    }

    let release = Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
    let started = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut deadline_blockers = Vec::new();
    for _ in 0..super::reader::READER_POOL_SIZE {
        let release = release.clone();
        let started = started.clone();
        deadline_blockers.push(
            pool.submit(move |_| {
                started.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let (lock, ready) = &*release;
                let released = lock.lock().expect("reader blocker poisoned");
                drop(
                    ready
                        .wait_while(released, |released| !*released)
                        .expect("reader blocker wait poisoned"),
                );
                Ok::<_, LocalEventQueryError>(())
            })
            .expect("install deadline reader blocker"),
        );
    }
    while started.load(std::sync::atomic::Ordering::SeqCst) < super::reader::READER_POOL_SIZE {
        tokio::task::yield_now().await;
    }
    let deadline_coordinator = coordinator.clone();
    let deadline_plan = plan.clone();
    let deadline = tokio::spawn(async move {
        deadline_coordinator
            .shutdown_plan_page_read_model(deadline_plan, 128, None)
            .await
    });
    while pool.queued_len_for_test() == 0 {
        tokio::task::yield_now().await;
    }
    harness.clock.advance_ms(2_001);
    {
        let (lock, ready) = &*release;
        *lock.lock().expect("reader release poisoned") = true;
        ready.notify_all();
    }
    let deadline = deadline.await.expect("deadline public query task");
    assert!(matches!(
        deadline,
        Err(LocalEventQueryError::DeadlineExceeded)
    ));
    assert_eq!(
        executor.effects.load(std::sync::atomic::Ordering::SeqCst),
        effects_before
    );
    assert_eq!(
        executor
            .subordinate_shutdowns
            .load(std::sync::atomic::Ordering::SeqCst),
        subordinate_shutdowns_before
    );
    drop(deadline_blockers);
    (0, 0)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn b069_shutdown_snapshot_query_returns_bounded_failures_without_mutation() {
    let harness = Harness::open();
    assert_eq!(assert_reader_failure_contract(&harness).await, (0, 0));
}

#[tokio::test]
async fn session_projection_removal_deletes_message_rows_atomically_and_replays() {
    let harness = Harness::open();
    let create = batch(
        "projection-create",
        "projection-create",
        [41; 32],
        Vec::new(),
        Vec::new(),
        vec![
            LocalStateMutation::SessionProjection(SessionProjectionMutation {
                session_id: "rollback-session".to_string(),
                projection: agent_session_projection("rollback-session"),
                expected: RevisionGuard::Absent,
                revision: Revision::new(0).unwrap(),
            }),
            LocalStateMutation::MessageProjection(MessageProjectionMutation {
                session_id: "rollback-session".to_string(),
                message_id: "message-1".to_string(),
                projection: agent_message_projection("message-1", "one"),
                expected: RevisionGuard::Absent,
                revision: Revision::new(0).unwrap(),
            }),
        ],
    );
    harness.store.commit_batch(create).await.unwrap();

    let remove = batch(
        "projection-remove",
        "projection-remove",
        [42; 32],
        Vec::new(),
        Vec::new(),
        vec![LocalStateMutation::SessionProjectionRemoval(
            SessionProjectionRemovalMutation {
                session_id: "rollback-session".to_string(),
                expected: RevisionGuard::Expected(Revision::new(0).unwrap()),
            },
        )],
    );
    harness.store.commit_batch(remove.clone()).await.unwrap();
    assert!(matches!(
        harness.store.commit_batch(remove).await.unwrap(),
        CommitBatchResult::Replayed(_)
    ));

    let session = harness
        .store
        .query(LocalEventQuery::SessionProjectionByIdentity {
            session_id: "rollback-session".to_string(),
        })
        .await
        .unwrap();
    assert!(matches!(
        session,
        LocalEventQueryResult::SessionProjectionByIdentity(None)
    ));
    let message = harness
        .store
        .query(LocalEventQuery::MessageProjectionByIdentity {
            session_id: "rollback-session".to_string(),
            message_id: "message-1".to_string(),
        })
        .await
        .unwrap();
    assert!(matches!(
        message,
        LocalEventQueryResult::MessageProjectionByIdentity(None)
    ));
}

#[tokio::test]
async fn projection_families_reject_cross_family_and_malformed_mutation_or_query() {
    let harness = Harness::open();
    let invalid_mutations = [
        LocalStateMutation::SessionProjection(SessionProjectionMutation {
            session_id: "workflow:cross-family-agent-session".to_string(),
            projection: agent_session_projection("workflow:cross-family-agent-session"),
            expected: RevisionGuard::Absent,
            revision: Revision::new(0).unwrap(),
        }),
        LocalStateMutation::SessionProjection(SessionProjectionMutation {
            session_id: "cross-family-workflow".to_string(),
            projection: workflow_execution_projection(
                "cross-family-workflow",
                ExecutionStatus::Running,
            ),
            expected: RevisionGuard::Absent,
            revision: Revision::new(0).unwrap(),
        }),
        LocalStateMutation::MessageProjection(MessageProjectionMutation {
            session_id: "cross-family-message".to_string(),
            message_id: "message-1".to_string(),
            projection: MessageProjectionRecord::AgentContentBlob(
                AgentContentBlobRecord::ToolOutput {
                    id: "message-1".to_string(),
                    content: "cross-family".to_string(),
                },
            ),
            expected: RevisionGuard::Absent,
            revision: Revision::new(0).unwrap(),
        }),
    ];
    for (index, mutation) in invalid_mutations.into_iter().enumerate() {
        let error = harness
            .store
            .commit_batch(batch(
                &format!("projection-family-conflict-{index}"),
                &format!("projection-family-conflict-{index}"),
                [index as u8; 32],
                Vec::new(),
                Vec::new(),
                vec![mutation],
            ))
            .await
            .expect_err("cross-family or malformed projection must be rejected");
        assert_eq!(error, CommitBatchError::PayloadConflict);
    }

    harness
        .store
        .commit_batch(batch(
            "projection-family-query-seed",
            "projection-family-query-seed",
            [87; 32],
            Vec::new(),
            Vec::new(),
            vec![
                LocalStateMutation::SessionProjection(SessionProjectionMutation {
                    session_id: "query-family-session".to_string(),
                    projection: agent_session_projection("query-family-session"),
                    expected: RevisionGuard::Absent,
                    revision: Revision::new(0).unwrap(),
                }),
                LocalStateMutation::MessageProjection(MessageProjectionMutation {
                    session_id: "query-family-session".to_string(),
                    message_id: "message-1".to_string(),
                    projection: agent_message_projection("message-1", "seed"),
                    expected: RevisionGuard::Absent,
                    revision: Revision::new(0).unwrap(),
                }),
            ],
        ))
        .await
        .expect("seed valid projection families");
    let connection = harness.raw_connection();
    connection
        .execute(
            "UPDATE session_projection SET projection = ?1 WHERE session_id = ?2",
            [
                r#"{"schema":"workflow_execution_projection_v1"}"#,
                "query-family-session",
            ],
        )
        .unwrap();
    connection
        .execute(
            "UPDATE message_projection SET projection = ?1 WHERE session_id = ?2 AND message_id = ?3",
            [
                r#"{"schema":"agent_content_blob_v1"}"#,
                "query-family-session",
                "message-1",
            ],
        )
        .unwrap();
    drop(connection);

    let session_error = harness
        .store
        .query(LocalEventQuery::SessionProjectionByIdentity {
            session_id: "query-family-session".to_string(),
        })
        .await
        .expect_err("cross-family session projection must fail closed on read");
    assert!(matches!(
        session_error,
        LocalEventQueryError::Corrupt { .. }
    ));
    let message_error = harness
        .store
        .query(LocalEventQuery::MessageProjectionByIdentity {
            session_id: "query-family-session".to_string(),
            message_id: "message-1".to_string(),
        })
        .await
        .expect_err("cross-family message projection must fail closed on read");
    assert!(matches!(
        message_error,
        LocalEventQueryError::Corrupt { .. }
    ));
}

#[tokio::test]
async fn projection_range_queries_page_without_legacy_directory_or_message_index() {
    let harness = Harness::open();
    let create = batch(
        "projection-page-create",
        "projection-page-create",
        [43; 32],
        Vec::new(),
        Vec::new(),
        vec![
            LocalStateMutation::SessionProjection(SessionProjectionMutation {
                session_id: "session-a".to_string(),
                projection: agent_session_projection("session-a"),
                expected: RevisionGuard::Absent,
                revision: Revision::new(0).unwrap(),
            }),
            LocalStateMutation::SessionProjection(SessionProjectionMutation {
                session_id: "session-b".to_string(),
                projection: agent_session_projection("session-b"),
                expected: RevisionGuard::Absent,
                revision: Revision::new(0).unwrap(),
            }),
            LocalStateMutation::MessageProjection(MessageProjectionMutation {
                session_id: "session-a".to_string(),
                message_id: "message-1".to_string(),
                projection: agent_message_projection("message-1", "one"),
                expected: RevisionGuard::Absent,
                revision: Revision::new(0).unwrap(),
            }),
            LocalStateMutation::MessageProjection(MessageProjectionMutation {
                session_id: "session-a".to_string(),
                message_id: "message-2".to_string(),
                projection: agent_message_projection("message-2", "two"),
                expected: RevisionGuard::Absent,
                revision: Revision::new(0).unwrap(),
            }),
            LocalStateMutation::MessageProjection(MessageProjectionMutation {
                session_id: "session-a".to_string(),
                message_id: "message-3".to_string(),
                projection: agent_message_projection("message-3", "three"),
                expected: RevisionGuard::Absent,
                revision: Revision::new(0).unwrap(),
            }),
        ],
    );
    harness.store.commit_batch(create).await.unwrap();

    let sessions = harness
        .store
        .query(LocalEventQuery::SessionProjectionPage {
            limit: 1,
            after_session_id: None,
        })
        .await
        .unwrap();
    let LocalEventQueryResult::SessionProjectionPage(sessions) = sessions else {
        panic!("wrong session projection page shape");
    };
    assert_eq!(sessions[0].session_id, "session-a");

    let latest = harness
        .store
        .query(LocalEventQuery::MessageProjectionPage {
            session_id: "session-a".to_string(),
            before_position: None,
            limit: 2,
        })
        .await
        .unwrap();
    let LocalEventQueryResult::MessageProjectionPage(latest) = latest else {
        panic!("wrong message projection page shape");
    };
    assert_eq!(latest.total_count, 3);
    assert_eq!(latest.entries.len(), 2);
    assert_eq!(latest.entries[0].message.message_id, "message-2");
    assert_eq!(latest.entries[1].message.message_id, "message-3");
    let older = harness
        .store
        .query(LocalEventQuery::MessageProjectionPage {
            session_id: "session-a".to_string(),
            before_position: latest.next_before_position,
            limit: 2,
        })
        .await
        .unwrap();
    let LocalEventQueryResult::MessageProjectionPage(older) = older else {
        panic!("wrong message projection page shape");
    };
    assert_eq!(older.entries.len(), 1);
    assert_eq!(older.entries[0].message.message_id, "message-1");
    assert_eq!(older.next_before_position, None);
}

#[tokio::test]
async fn caller_outbox_encrypts_exact_command_and_ack_removes_retry_material() {
    use crate::usecase::agent_session::operation::CallerAttemptJournal;

    let harness = Harness::open();
    let repository: Arc<dyn LocalEventTransactionRepository> = harness.store.clone();
    let authority: Arc<dyn crate::usecase::agent_session::operation::OperationBindingAuthority> =
        harness.store.clone();
    let journal = CallerAttemptJournal::new(
        repository,
        authority,
        harness.store.installation_id().to_string(),
    );
    let exact = br#"{"content":"private-marker","worktree_path":"/private/path"}"#;
    journal
        .record_attempt_scoped(
            "local-app",
            OperationKind::Send,
            "send-encrypted-1",
            exact,
            Some("session-encrypted"),
        )
        .await
        .unwrap();
    let connection = harness.raw_connection();
    let sealed: Vec<u8> = connection
        .query_row(
            "SELECT sealed_command FROM caller_attempts WHERE caller_request_id = ?1",
            ["send-encrypted-1"],
            |row| row.get(0),
        )
        .unwrap();
    assert!(sealed.starts_with(b"RLSA1"));
    assert!(!sealed
        .windows(b"private-marker".len())
        .any(|value| value == b"private-marker"));
    assert!(!sealed
        .windows(b"/private/path".len())
        .any(|value| value == b"/private/path"));
    drop(connection);

    journal
        .clear_attempt(
            "local-app",
            OperationKind::Send,
            "send-encrypted-1",
            exact,
            true,
        )
        .await
        .unwrap();
    journal
        .acknowledge_attempt("local-app", OperationKind::Send, "send-encrypted-1")
        .await
        .unwrap();
    let connection = harness.raw_connection();
    let (resolution, length): (String, i64) = connection
        .query_row(
            "SELECT resolution, length(sealed_command) FROM caller_attempts WHERE caller_request_id = ?1",
            ["send-encrypted-1"],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(resolution, "cleared");
    assert_eq!(length, 0);
}

#[tokio::test]
async fn canonical_full_session_save_creates_an_absent_sqlite_projection() {
    let harness = Harness::open();
    let repository: Arc<dyn LocalEventTransactionRepository> = harness.store.clone();
    let session_store = SessionStore::new(Arc::new(FileSessionStorage::default()));
    session_store.set_local_event_repository(
        repository,
        harness.store.installation_id().to_string(),
        Arc::new(AgentSessionProjectionCodecV1),
    );
    let session = build_new_session_with_id(
        uuid::Uuid::new_v4().to_string(),
        "/canonical-create-fixture",
        Some("codex".to_string()),
        crate::domain::agent_session::PermissionMode::Ask,
        None,
        false,
        false,
        None,
    );

    session_store
        .save_full_session_for_restore(&harness.root, &session)
        .expect("create missing canonical projection");

    let saved = session_store
        .load_full_session_for_restore(&harness.root, &session.id)
        .expect("read canonical projection")
        .expect("created session");
    assert_eq!(saved.id, session.id);
    assert_eq!(saved.worktree_path, "/canonical-create-fixture");
}

fn workflow_node_context_fixture() -> crate::usecase::agent_session::session::WorkflowNodeContextDto
{
    crate::usecase::agent_session::session::WorkflowNodeContextDto {
        execution_id: "execution-outbox-1".to_string(),
        node_execution_id: "node-execution-outbox-1".to_string(),
        workflow_name: "outbox-workflow".to_string(),
        node_name: "agent-step".to_string(),
        attempt: 1,
        parent_node_name: None,
        parent_attempt: None,
        order: 0,
        startup_timeout_secs: Some(30),
        startup_max_retries: Some(2),
        stale_timeout_secs: Some(300),
    }
}

fn persist_terminal_fixture(
    harness: &Harness,
    session_store: &SessionStore,
    session_id: &str,
    turn_id: u64,
    text_parts: &[&str],
) {
    let assistant_message_id = format!("agent-{turn_id}");
    session_store
        .append_session_events(
            &harness.root,
            session_id,
            &[
                crate::usecase::agent_session::event_log::AgentSessionEvent::TurnStarted {
                    turn_id,
                    message_id: format!("human-{turn_id}"),
                    assistant_message_id: Some(assistant_message_id.clone()),
                    prompt: crate::domain::agent_session::events::PromptInput {
                        content: "run the workflow step".to_string(),
                        ..Default::default()
                    },
                    at: 1.0,
                },
            ],
        )
        .expect("persist turn start");
    let parts = text_parts
        .iter()
        .map(
            |content| crate::usecase::agent_session::session::MessagePart::Text {
                content: (*content).to_string(),
                parent_tool_use_id: None,
            },
        )
        .collect::<Vec<_>>();
    session_store
        .append_terminal_events_and_materialize(
            &harness.root,
            session_id,
            &[
                crate::usecase::agent_session::event_log::AgentSessionEvent::FinalPartsRecorded {
                    turn_id,
                    message_id: assistant_message_id.clone(),
                    parts,
                },
                crate::usecase::agent_session::event_log::AgentSessionEvent::TurnCompleted {
                    turn_id,
                    exit_code: 0,
                    stop_reason: None,
                    token_usage: Some(crate::domain::agent_session::events::TurnTokenUsage {
                        input_tokens: 17,
                        output_tokens: 9,
                    }),
                },
            ],
            &assistant_message_id,
            4,
            2.0,
            &crate::domain::agent_session::entities::TurnResult::Completed {
                stop_reason: None,
                token_usage: Some(crate::domain::agent_session::entities::TokenUsage {
                    input_tokens: 17,
                    output_tokens: 9,
                    total_tokens: Some(26),
                    context_window_tokens: None,
                }),
            },
        )
        .expect("persist terminal batch");
}

#[tokio::test]
async fn workflow_terminal_atomically_creates_exact_bounded_pending_handoff() {
    let mut registry = EventCodecRegistry::new();
    registry.register(Arc::new(AgentSessionEventCodec));
    let harness = Harness::open_with_registry(Arc::new(registry));
    let repository: Arc<dyn LocalEventTransactionRepository> = harness.store.clone();
    let session_store = SessionStore::new(Arc::new(FileSessionStorage::default()));
    session_store.set_local_event_repository(
        repository,
        harness.store.installation_id().to_string(),
        Arc::new(AgentSessionProjectionCodecV1),
    );
    let context = workflow_node_context_fixture();
    let session = build_new_session_with_id(
        "workflow-outbox-session".to_string(),
        "/workflow-outbox-fixture",
        Some("codex".to_string()),
        crate::domain::agent_session::PermissionMode::Ask,
        None,
        false,
        true,
        Some(context.clone()),
    );
    session_store
        .save_full_session_for_restore(&harness.root, &session)
        .expect("seed workflow session projection");

    persist_terminal_fixture(
        &harness,
        &session_store,
        &session.id,
        7,
        &["first exact part", "second exact part"],
    );

    let pending = session_store
        .pending_workflow_turn_completion(&session.id, 7)
        .expect("read exact handoff")
        .expect("pending handoff");
    assert_eq!(pending.session_id, session.id);
    assert_eq!(pending.workflow_context, context);
    assert_eq!(pending.input.turn_id, 7);
    assert_eq!(pending.input.exit_code, 0);
    assert_eq!(
        pending.input.final_text_parts,
        vec!["first exact part", "second exact part"]
    );
    assert_eq!(
        pending.input.token_usage,
        Some(crate::domain::agent_session::events::TurnTokenUsage {
            input_tokens: 17,
            output_tokens: 9,
        })
    );

    let page = session_store
        .pending_workflow_turn_completion_page(None, None, 1, None)
        .expect("bounded prefix page");
    assert_eq!(page.entries.len(), 1);
    assert!(page.next_cursor.is_none());

    session_store
        .complete_workflow_turn_completion(&pending)
        .expect("consume after workflow commit");
    session_store
        .complete_workflow_turn_completion(&pending)
        .expect("idempotent consume replay");
    assert!(session_store
        .pending_workflow_turn_completion(&session.id, 7)
        .expect("read consumed handoff")
        .is_none());
}

#[tokio::test]
async fn ordinary_chat_terminal_never_enters_workflow_handoff_inventory() {
    let mut registry = EventCodecRegistry::new();
    registry.register(Arc::new(AgentSessionEventCodec));
    let harness = Harness::open_with_registry(Arc::new(registry));
    let repository: Arc<dyn LocalEventTransactionRepository> = harness.store.clone();
    let session_store = SessionStore::new(Arc::new(FileSessionStorage::default()));
    session_store.set_local_event_repository(
        repository,
        harness.store.installation_id().to_string(),
        Arc::new(AgentSessionProjectionCodecV1),
    );
    let session = build_new_session_with_id(
        "ordinary-outbox-session".to_string(),
        "/ordinary-outbox-fixture",
        Some("codex".to_string()),
        crate::domain::agent_session::PermissionMode::Ask,
        None,
        false,
        false,
        None,
    );
    session_store
        .save_full_session_for_restore(&harness.root, &session)
        .expect("seed ordinary session projection");

    persist_terminal_fixture(
        &harness,
        &session_store,
        &session.id,
        3,
        &["ordinary response"],
    );

    assert!(session_store
        .pending_workflow_turn_completion_page(None, None, 8, None)
        .expect("bounded workflow-only page")
        .entries
        .is_empty());
}

#[tokio::test]
async fn clean_workflow_interruption_does_not_create_an_impossible_completion_handoff() {
    let mut registry = EventCodecRegistry::new();
    registry.register(Arc::new(AgentSessionEventCodec));
    let harness = Harness::open_with_registry(Arc::new(registry));
    let repository: Arc<dyn LocalEventTransactionRepository> = harness.store.clone();
    let session_store = SessionStore::new(Arc::new(FileSessionStorage::default()));
    session_store.set_local_event_repository(
        repository,
        harness.store.installation_id().to_string(),
        Arc::new(AgentSessionProjectionCodecV1),
    );
    let session = build_new_session_with_id(
        "workflow-clean-interruption-session".to_string(),
        "/workflow-clean-interruption-fixture",
        Some("codex".to_string()),
        crate::domain::agent_session::PermissionMode::Ask,
        None,
        false,
        true,
        Some(workflow_node_context_fixture()),
    );
    session_store
        .save_full_session_for_restore(&harness.root, &session)
        .expect("seed workflow session projection");
    let message_id = "agent-interrupted";
    session_store
        .append_session_events(
            &harness.root,
            &session.id,
            &[
                crate::usecase::agent_session::event_log::AgentSessionEvent::TurnStarted {
                    turn_id: 11,
                    message_id: "human-interrupted".to_string(),
                    assistant_message_id: Some(message_id.to_string()),
                    prompt: crate::domain::agent_session::events::PromptInput {
                        content: "stop safely".to_string(),
                        ..Default::default()
                    },
                    at: 1.0,
                },
            ],
        )
        .expect("persist turn start");
    session_store
        .append_terminal_events_and_materialize(
            &harness.root,
            &session.id,
            &[
                crate::usecase::agent_session::event_log::AgentSessionEvent::FinalPartsRecorded {
                    turn_id: 11,
                    message_id: message_id.to_string(),
                    parts: Vec::new(),
                },
                crate::usecase::agent_session::event_log::AgentSessionEvent::TurnInterrupted {
                    turn_id: 11,
                    reason: crate::domain::agent_session::events::InterruptReason::Abort,
                    exit_code: 0,
                    error: None,
                },
                crate::usecase::agent_session::event_log::AgentSessionEvent::QueuePaused {
                    at: 2.0,
                },
            ],
            message_id,
            1,
            2.0,
            &crate::domain::agent_session::entities::TurnResult::Interrupted {
                reason: crate::domain::agent_session::entities::InterruptReason::Abort,
                error: None,
            },
        )
        .expect("persist clean interruption terminal batch");

    assert!(session_store
        .pending_workflow_turn_completion_page(None, None, 8, None)
        .expect("bounded workflow-only page")
        .entries
        .is_empty());
}

#[tokio::test]
async fn b035_backend_recovery_hands_off_and_settles_publication_in_atomic_commits() {
    let mut registry = EventCodecRegistry::new();
    registry.register(Arc::new(AgentSessionEventCodec));
    let harness = Harness::open_with_registry(Arc::new(registry));
    let repository: Arc<dyn LocalEventTransactionRepository> = harness.store.clone();
    let session_store = SessionStore::new(Arc::new(FileSessionStorage::default()));
    session_store.set_local_event_repository(
        repository,
        harness.store.installation_id().to_string(),
        Arc::new(AgentSessionProjectionCodecV1),
    );
    let session = build_new_session_with_id(
        uuid::Uuid::new_v4().to_string(),
        "/recovery-publication-fixture",
        Some("codex".to_string()),
        crate::domain::agent_session::PermissionMode::Ask,
        None,
        false,
        false,
        None,
    );
    session_store
        .save_full_session_for_restore(&harness.root, &session)
        .expect("seed canonical session projection");
    session_store
        .begin_backend_session_recovery(
            &harness.root,
            &session.id,
            "recovery-publication-1",
            BackendSessionRecoveryReason::BackendSessionLost,
        )
        .expect("reserve backend recovery");
    let completed = session_store
        .complete_backend_session_recovery(
            &harness.root,
            &session.id,
            "recovery-publication-1",
            0,
            "provider-session-2".to_string(),
        )
        .expect("atomically complete recovery and reserve publication");
    let pending = completed
        .pending_recovery_message
        .expect("publication payload must remain pending");
    let message_id = match &pending {
        PendingRecoveryMessage::Notice { message_id, .. } => message_id.clone(),
        PendingRecoveryMessage::Error { .. } => panic!("completion publishes a notice"),
    };

    let recovery_usecase = crate::usecase::agent_session::operation::RecoveryActionUsecase::new(
        harness.store.clone(),
        harness.store.clone(),
        Arc::new(PendingRecoveryActionExecutor),
        harness.store.installation_id().to_string(),
    );
    let page = recovery_usecase
        .pending(
            crate::usecase::agent_session::operation::PendingRecoveryQuery {
                limit: 32,
                partition: None,
                owner: Some(session.id.clone()),
                shutdown_plan: None,
                cursor: None,
            },
        )
        .await
        .expect("query pending publication directly from the index");
    assert_eq!(page.entries.len(), 1);
    assert_eq!(
        page.entries[0].category,
        crate::usecase::agent_session::operation::PendingRecoveryCategory::RecoveryPublication
    );
    assert_eq!(page.entries[0].original_identity, message_id);
    assert_eq!(
        page.entries[0].known_status,
        crate::usecase::agent_session::operation::PendingRecoveryKnownStatus::Pending
    );

    let message = ChatMessage {
        id: message_id.clone(),
        role: MessageRole::Agent,
        content: String::new(),
        thinking: None,
        activities: None,
        parts: Some(Vec::new()),
        streaming_final_seq: 0,
        timestamp: 2.0,
        mentions: None,
    };
    assert!(session_store
        .publish_pending_recovery_message(&harness.root, &session.id, &pending, message.clone())
        .expect("publish message and settle obligation"));
    assert!(!session_store
        .publish_pending_recovery_message(&harness.root, &session.id, &pending, message)
        .expect("exact publication retry converges"));
    let after = recovery_usecase
        .pending(
            crate::usecase::agent_session::operation::PendingRecoveryQuery {
                limit: 32,
                partition: None,
                owner: Some(session.id),
                shutdown_plan: None,
                cursor: None,
            },
        )
        .await
        .expect("query settled pending index");
    assert!(after.entries.is_empty());
}

#[derive(Clone, Copy)]
enum ShutdownExecutorMode {
    Complete,
    Hang,
    HangNamedTarget,
    HangTargets,
    FailThenReadCompleted,
}

struct TestShutdownExecutor {
    targets: Vec<crate::usecase::shutdown_coordinator::ShutdownTarget>,
    mode: ShutdownExecutorMode,
    target_queries: std::sync::atomic::AtomicUsize,
    effects: std::sync::atomic::AtomicUsize,
    readbacks: std::sync::atomic::AtomicUsize,
    subordinate_shutdowns: std::sync::atomic::AtomicUsize,
    drop_reply_on_readback: Option<Arc<FaultInjector>>,
}

struct InventoryRaceShutdownExecutor {
    store: Arc<LocalEventStore>,
    current_targets: std::sync::Mutex<Vec<crate::usecase::shutdown_coordinator::ShutdownTarget>>,
    target_queries: std::sync::atomic::AtomicUsize,
    effects: std::sync::atomic::AtomicUsize,
}

struct PendingRecoveryActionExecutor;

struct HangingShutdownQueryRepository {
    inner: Arc<LocalEventStore>,
    queries: std::sync::atomic::AtomicUsize,
}

struct RecoveryReplayOnlyRepository {
    inner: Arc<LocalEventStore>,
    unavailable_resource_queries: std::sync::atomic::AtomicUsize,
}

struct CurrentShutdownUnavailableRepository {
    inner: Arc<LocalEventStore>,
}

#[async_trait::async_trait]
impl LocalEventTransactionRepository for HangingShutdownQueryRepository {
    async fn commit_batch(
        &self,
        batch: LocalAtomicBatch,
    ) -> Result<CommitBatchResult, CommitBatchError> {
        self.inner.commit_batch(batch).await
    }

    async fn resolve_commit(
        &self,
        identity: CommitIdentity,
    ) -> Result<CommitResolution, LocalEventQueryError> {
        self.inner.resolve_commit(identity).await
    }

    async fn load_stream(
        &self,
        request: LoadStreamRequest,
    ) -> Result<crate::domain::local_event::DomainEventPage, LocalEventQueryError> {
        self.inner.load_stream(request).await
    }

    async fn query(
        &self,
        _request: LocalEventQuery,
    ) -> Result<LocalEventQueryResult, LocalEventQueryError> {
        self.queries
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        futures_util::future::pending().await
    }

    fn subscribe(
        &self,
        after: crate::domain::local_event::GlobalSequence,
    ) -> crate::domain::local_event::LocalEventSubscription {
        self.inner.subscribe(after)
    }
}

#[async_trait::async_trait]
impl LocalEventTransactionRepository for RecoveryReplayOnlyRepository {
    async fn commit_batch(
        &self,
        batch: LocalAtomicBatch,
    ) -> Result<CommitBatchResult, CommitBatchError> {
        self.inner.commit_batch(batch).await
    }

    async fn resolve_commit(
        &self,
        identity: CommitIdentity,
    ) -> Result<CommitResolution, LocalEventQueryError> {
        self.inner.resolve_commit(identity).await
    }

    async fn load_stream(
        &self,
        request: LoadStreamRequest,
    ) -> Result<crate::domain::local_event::DomainEventPage, LocalEventQueryError> {
        self.inner.load_stream(request).await
    }

    async fn query(
        &self,
        request: LocalEventQuery,
    ) -> Result<LocalEventQueryResult, LocalEventQueryError> {
        if matches!(&request, LocalEventQuery::RecoveryActionByIdentity { .. }) {
            return self.inner.query(request).await;
        }
        self.unavailable_resource_queries
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Err(LocalEventQueryError::StorageUnavailable {
            failure: SafeOperationFailure::new(
                SessionOperationFailureKind::StorageUnavailable,
                true,
                "The current recovery resource is unavailable.",
                "b093-current-resource-unavailable",
            ),
        })
    }

    fn subscribe(
        &self,
        after: crate::domain::local_event::GlobalSequence,
    ) -> crate::domain::local_event::LocalEventSubscription {
        self.inner.subscribe(after)
    }
}

#[async_trait::async_trait]
impl LocalEventTransactionRepository for CurrentShutdownUnavailableRepository {
    async fn commit_batch(
        &self,
        batch: LocalAtomicBatch,
    ) -> Result<CommitBatchResult, CommitBatchError> {
        self.inner.commit_batch(batch).await
    }

    async fn resolve_commit(
        &self,
        identity: CommitIdentity,
    ) -> Result<CommitResolution, LocalEventQueryError> {
        self.inner.resolve_commit(identity).await
    }

    async fn load_stream(
        &self,
        request: LoadStreamRequest,
    ) -> Result<crate::domain::local_event::DomainEventPage, LocalEventQueryError> {
        self.inner.load_stream(request).await
    }

    async fn query(
        &self,
        _request: LocalEventQuery,
    ) -> Result<LocalEventQueryResult, LocalEventQueryError> {
        Err(LocalEventQueryError::StorageUnavailable {
            failure: SafeOperationFailure::new(
                SessionOperationFailureKind::StorageUnavailable,
                true,
                "The current shutdown authority is unavailable.",
                "b076-current-storage-unavailable",
            ),
        })
    }

    fn subscribe(
        &self,
        after: crate::domain::local_event::GlobalSequence,
    ) -> crate::domain::local_event::LocalEventSubscription {
        self.inner.subscribe(after)
    }
}

#[async_trait::async_trait]
impl crate::usecase::agent_session::operation::RecoveryEffectExecutor
    for PendingRecoveryActionExecutor
{
    fn supports_read_again(
        &self,
        _obligation_id: &str,
        _immutable_obligation: &ObligationRecord,
    ) -> bool {
        true
    }

    async fn execute(
        &self,
        _request: &crate::usecase::agent_session::operation::RecoveryEffectRequest,
    ) -> Result<crate::usecase::agent_session::operation::RecoveryEffectResult, SafeOperationFailure>
    {
        Ok(
            crate::usecase::agent_session::operation::RecoveryEffectResult {
                classification:
                    crate::domain::agent_session::events::RecoveryResultClassification::Pending,
                safe_result: "The Stop terminal is still pending.".to_string(),
                owner_mutations: Vec::new(),
                owner_batch: None,
            },
        )
    }
}

#[async_trait::async_trait]
impl crate::usecase::shutdown_coordinator::ShutdownTargetExecutor
    for InventoryRaceShutdownExecutor
{
    async fn targets(
        &self,
    ) -> Result<Vec<crate::usecase::shutdown_coordinator::ShutdownTarget>, SafeOperationFailure>
    {
        let query = self
            .target_queries
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let snapshot = self
            .current_targets
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        if query == 0 {
            // Compute the first inventory, then accept a workflow before the
            // quit acceptance transaction can install its current pointer.
            // The second inventory must observe this durable owner.
            let workflow_id = "workflow-raced-before-quit";
            let projection = workflow_execution_projection(workflow_id, ExecutionStatus::Running);
            let mut accepted_workflow = batch(
                "commit-workflow-raced-before-quit",
                "key-workflow-raced-before-quit",
                [45; 32],
                vec![],
                vec![],
                vec![
                    LocalStateMutation::SessionProjection(SessionProjectionMutation {
                        session_id: format!("workflow:{workflow_id}"),
                        projection: projection.clone(),
                        expected: RevisionGuard::Absent,
                        revision: Revision::new(0).unwrap(),
                    }),
                    LocalStateMutation::Obligation(ObligationMutation {
                        obligation_id: format!("workflow-execution-{workflow_id}"),
                        record: workflow_execution_obligation(workflow_id),
                        pending: Some(PendingIndexEntry {
                            ordered_key: format!("workflow_execution:{workflow_id}"),
                            owner: "workflow-runtime".to_string(),
                            partition: PendingPartition::UnownedRuntime,
                            shutdown_plan: None,
                        }),
                        expected: RevisionGuard::Absent,
                        revision: Revision::new(0).unwrap(),
                    }),
                ],
            );
            accepted_workflow.idempotency.operation_kind = CommitOperationKind::UserMutation;
            assert!(matches!(
                self.store.commit_batch(accepted_workflow).await,
                Ok(CommitBatchResult::Committed(_))
            ));
            self.current_targets
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(crate::usecase::shutdown_coordinator::ShutdownTarget {
                    target_id: workflow_id.to_string(),
                    kind: "workflow_execution".to_string(),
                });
        }
        Ok(snapshot)
    }

    async fn execute_target(
        &self,
        _operation_id: &str,
        _effect_identity: &str,
        _owner_revision: Revision,
        _target: &crate::usecase::shutdown_coordinator::ShutdownTarget,
    ) -> Result<(), SafeOperationFailure> {
        self.effects
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }

    async fn read_target_effect(
        &self,
        _operation_id: &str,
        _effect_identity: &str,
        _owner_revision: Revision,
        _target: &crate::usecase::shutdown_coordinator::ShutdownTarget,
    ) -> Result<crate::usecase::shutdown_coordinator::ShutdownEffectReadback, SafeOperationFailure>
    {
        Ok(crate::usecase::shutdown_coordinator::ShutdownEffectReadback::Ambiguous)
    }
}

impl TestShutdownExecutor {
    fn with_targets(count: usize, mode: ShutdownExecutorMode) -> Arc<Self> {
        Arc::new(Self {
            targets: (0..count)
                .map(
                    |index| crate::usecase::shutdown_coordinator::ShutdownTarget {
                        target_id: format!("target-{index}"),
                        kind: "agent_session".to_string(),
                    },
                )
                .collect(),
            mode,
            target_queries: std::sync::atomic::AtomicUsize::new(0),
            effects: std::sync::atomic::AtomicUsize::new(0),
            readbacks: std::sync::atomic::AtomicUsize::new(0),
            subordinate_shutdowns: std::sync::atomic::AtomicUsize::new(0),
            drop_reply_on_readback: None,
        })
    }

    fn fail_then_read_completed(
        count: usize,
        drop_reply_on_readback: Option<Arc<FaultInjector>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            targets: (0..count)
                .map(
                    |index| crate::usecase::shutdown_coordinator::ShutdownTarget {
                        target_id: format!("target-{index}"),
                        kind: "agent_session".to_string(),
                    },
                )
                .collect(),
            mode: ShutdownExecutorMode::FailThenReadCompleted,
            target_queries: std::sync::atomic::AtomicUsize::new(0),
            effects: std::sync::atomic::AtomicUsize::new(0),
            readbacks: std::sync::atomic::AtomicUsize::new(0),
            subordinate_shutdowns: std::sync::atomic::AtomicUsize::new(0),
            drop_reply_on_readback,
        })
    }
}

#[async_trait::async_trait]
impl crate::usecase::shutdown_coordinator::ShutdownTargetExecutor for TestShutdownExecutor {
    async fn targets(
        &self,
    ) -> Result<Vec<crate::usecase::shutdown_coordinator::ShutdownTarget>, SafeOperationFailure>
    {
        self.target_queries
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if matches!(self.mode, ShutdownExecutorMode::HangTargets) {
            futures_util::future::pending::<()>().await;
            unreachable!()
        }
        Ok(self.targets.clone())
    }

    async fn execute_target(
        &self,
        _operation_id: &str,
        _effect_identity: &str,
        _owner_revision: Revision,
        target: &crate::usecase::shutdown_coordinator::ShutdownTarget,
    ) -> Result<(), SafeOperationFailure> {
        self.effects
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        match self.mode {
            ShutdownExecutorMode::Complete => Ok(()),
            ShutdownExecutorMode::Hang => {
                futures_util::future::pending::<()>().await;
                unreachable!()
            }
            ShutdownExecutorMode::HangNamedTarget if target.target_id == "b064-hanging-target" => {
                futures_util::future::pending::<()>().await;
                unreachable!()
            }
            ShutdownExecutorMode::HangNamedTarget => Ok(()),
            ShutdownExecutorMode::HangTargets => unreachable!("target enumeration never returns"),
            ShutdownExecutorMode::FailThenReadCompleted => Err(SafeOperationFailure::new(
                SessionOperationFailureKind::ExternalEffectFailed,
                true,
                "The test shutdown target requires readback.",
                "test-shutdown-readback",
            )),
        }
    }

    async fn read_target_effect(
        &self,
        _operation_id: &str,
        _effect_identity: &str,
        _owner_revision: Revision,
        _target: &crate::usecase::shutdown_coordinator::ShutdownTarget,
    ) -> Result<crate::usecase::shutdown_coordinator::ShutdownEffectReadback, SafeOperationFailure>
    {
        self.readbacks
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        match self.mode {
            ShutdownExecutorMode::FailThenReadCompleted => {
                if let Some(fault) = self.drop_reply_on_readback.as_ref() {
                    fault.arm_drop_reply();
                }
                Ok(crate::usecase::shutdown_coordinator::ShutdownEffectReadback::Completed)
            }
            ShutdownExecutorMode::Complete
            | ShutdownExecutorMode::Hang
            | ShutdownExecutorMode::HangNamedTarget
            | ShutdownExecutorMode::HangTargets => {
                Ok(crate::usecase::shutdown_coordinator::ShutdownEffectReadback::Ambiguous)
            }
        }
    }

    async fn shutdown_subordinates(&self) -> Result<(), SafeOperationFailure> {
        self.subordinate_shutdowns
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }
}

struct LateInventoryShutdownExecutor {
    target_queries: std::sync::atomic::AtomicUsize,
    effects: std::sync::atomic::AtomicUsize,
    subordinate_shutdowns: std::sync::atomic::AtomicUsize,
    late_results: Arc<std::sync::atomic::AtomicUsize>,
    release_late_result: Arc<tokio::sync::Notify>,
}

impl LateInventoryShutdownExecutor {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            target_queries: std::sync::atomic::AtomicUsize::new(0),
            effects: std::sync::atomic::AtomicUsize::new(0),
            subordinate_shutdowns: std::sync::atomic::AtomicUsize::new(0),
            late_results: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            release_late_result: Arc::new(tokio::sync::Notify::new()),
        })
    }
}

#[async_trait::async_trait]
impl crate::usecase::shutdown_coordinator::ShutdownTargetExecutor
    for LateInventoryShutdownExecutor
{
    async fn targets(
        &self,
    ) -> Result<Vec<crate::usecase::shutdown_coordinator::ShutdownTarget>, SafeOperationFailure>
    {
        let call = self
            .target_queries
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if call == 0 {
            let (reply, result) = tokio::sync::oneshot::channel();
            let release = Arc::clone(&self.release_late_result);
            let late_results = Arc::clone(&self.late_results);
            tokio::spawn(async move {
                release.notified().await;
                late_results.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let _ = reply.send(Vec::new());
            });
            return result.await.map_err(|_| {
                SafeOperationFailure::new(
                    SessionOperationFailureKind::OutcomeUnknown,
                    true,
                    "The stale inventory result was detached.",
                    "b067-detached-inventory",
                )
            });
        }
        Ok(Vec::new())
    }

    async fn execute_target(
        &self,
        _operation_id: &str,
        _effect_identity: &str,
        _owner_revision: Revision,
        _target: &crate::usecase::shutdown_coordinator::ShutdownTarget,
    ) -> Result<(), SafeOperationFailure> {
        self.effects
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }

    async fn read_target_effect(
        &self,
        _operation_id: &str,
        _effect_identity: &str,
        _owner_revision: Revision,
        _target: &crate::usecase::shutdown_coordinator::ShutdownTarget,
    ) -> Result<crate::usecase::shutdown_coordinator::ShutdownEffectReadback, SafeOperationFailure>
    {
        Ok(crate::usecase::shutdown_coordinator::ShutdownEffectReadback::Ambiguous)
    }

    async fn shutdown_subordinates(&self) -> Result<(), SafeOperationFailure> {
        self.subordinate_shutdowns
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }
}

fn shutdown_coordinator(
    harness: &Harness,
    executor: &Arc<TestShutdownExecutor>,
) -> Arc<crate::usecase::shutdown_coordinator::ShutdownCoordinator> {
    Arc::new(
        crate::usecase::shutdown_coordinator::ShutdownCoordinator::new(
            harness.store.clone(),
            harness.store.clone(),
            executor.clone(),
            harness.store.installation_id().to_string(),
            "test-boot".to_string(),
        ),
    )
}

#[tokio::test]
async fn b076_b100_first_quit_writer_unknown_keeps_durable_operation_and_intent() {
    let harness = Harness::open();
    let executor = TestShutdownExecutor::with_targets(1, ShutdownExecutorMode::Complete);
    let coordinator = Arc::new(
        crate::usecase::shutdown_coordinator::ShutdownCoordinator::new(
            harness.store.clone(),
            harness.store.clone(),
            executor.clone(),
            harness.store.installation_id().to_string(),
            harness.store.process_instance_id().to_string(),
        ),
    );
    coordinator.set_pre_acceptance_hook(Arc::new({
        let fault = Arc::clone(&harness.fault);
        move || {
            let fault = Arc::clone(&fault);
            Box::pin(async move {
                fault.arm_crash_after_commit_before_readback();
            }) as std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>
        }
    }));
    let request = crate::usecase::shutdown_coordinator::ApplicationQuitRequest {
        principal: "desktop".to_string(),
        request_id: "quit-first-writer-unknown".to_string(),
        intent: crate::usecase::shutdown_coordinator::ApplicationQuitIntent::Exit { code: 7 },
    };

    let outcome = coordinator
        .request(request.clone())
        .await
        .expect("first quit request");
    let crate::usecase::shutdown_coordinator::ApplicationQuitOutcome::OutcomeUnknown {
        request_id,
        operation_id,
        intent,
    } = &outcome
    else {
        panic!("acceptance readback loss must stay top-level unknown: {outcome:?}");
    };
    assert_eq!(request_id, &request.request_id);
    assert_eq!(intent, &request.intent);
    assert_eq!(operation_id.len(), 64);
    assert_eq!(
        executor.effects.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "top-level ambiguity must not execute a target"
    );

    let lookup = coordinator
        .get_application_quit_projection(operation_id)
        .await
        .expect("known operation lookup")
        .expect("durable ambiguity anchor must not become NotFound");
    assert_eq!(
        lookup,
        crate::usecase::shutdown_coordinator::ApplicationQuitProjection::OutcomeUnknown {
            operation_id: operation_id.clone(),
            intent: request.intent,
        }
    );
    let current = coordinator
        .current_application_shutdown_projection()
        .await
        .expect("current shutdown lookup");
    assert_eq!(
        current,
        crate::usecase::shutdown_coordinator::CurrentApplicationShutdownProjection::OutcomeUnknown {
            operation_id: operation_id.clone(),
            intent: request.intent,
        }
    );

    let outcome_wire = serde_json::to_value(
        crate::adaptor::presenter::application_lifecycle::application_quit_outcome(outcome.clone())
            .0,
    )
    .expect("shared outcome presenter");
    let lookup_wire = serde_json::to_value(
        crate::adaptor::presenter::application_lifecycle::application_quit_lookup(lookup),
    )
    .expect("shared lookup presenter");
    let current_wire = serde_json::to_value(
        crate::adaptor::presenter::application_lifecycle::current_shutdown(current),
    )
    .expect("shared current presenter");
    for wire in [&outcome_wire, &lookup_wire, &current_wire] {
        assert_eq!(wire["type"], "outcome_unknown");
        assert_eq!(wire["operation_id"], operation_id.as_str());
        assert_eq!(
            wire["intent"],
            serde_json::json!({ "type": "exit", "code": 7 })
        );
    }

    let replay = coordinator
        .request(request)
        .await
        .expect("same-identity reconciliation");
    assert!(matches!(
        replay,
        crate::usecase::shutdown_coordinator::ApplicationQuitOutcome::Accepted { .. }
    ));
    assert!(matches!(
        coordinator
            .get_application_quit_projection(operation_id)
            .await
            .expect("reconciled operation lookup"),
        Some(crate::usecase::shutdown_coordinator::ApplicationQuitProjection::Shutdown { .. })
    ));
}

#[tokio::test]
async fn b065_b088_activation_writer_unknown_is_accepted_inner_and_exit_permitted() {
    let harness = Harness::open();
    let executor = TestShutdownExecutor::with_targets(1, ShutdownExecutorMode::Complete);
    let coordinator = Arc::new(
        crate::usecase::shutdown_coordinator::ShutdownCoordinator::new(
            harness.store.clone(),
            harness.store.clone(),
            executor.clone(),
            harness.store.installation_id().to_string(),
            harness.store.process_instance_id().to_string(),
        ),
    );
    coordinator.set_pre_activation_hook(Arc::new({
        let fault = Arc::clone(&harness.fault);
        move || {
            let fault = Arc::clone(&fault);
            Box::pin(async move {
                fault.arm_crash_after_commit_before_readback();
            }) as std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>
        }
    }));

    let outcome = coordinator
        .request(
            crate::usecase::shutdown_coordinator::ApplicationQuitRequest {
                principal: "desktop".to_string(),
                request_id: "quit-activation-writer-unknown".to_string(),
                intent: crate::usecase::shutdown_coordinator::ApplicationQuitIntent::Exit {
                    code: 23,
                },
            },
        )
        .await
        .expect("quit request");
    let crate::usecase::shutdown_coordinator::ApplicationQuitOutcome::Accepted { receipt, state } =
        &outcome
    else {
        panic!("post-acceptance ambiguity must remain Accepted: {outcome:?}");
    };
    let crate::usecase::shutdown_coordinator::ApplicationQuitState::OutcomeUnknown {
        operation_id,
        shutdown_id,
        activation_commit_id,
    } = state
    else {
        panic!("activation ambiguity must use the closed inner state: {state:?}");
    };
    assert_eq!(operation_id, &receipt.operation_id);
    assert_eq!(shutdown_id, &receipt.shutdown_id);
    assert_eq!(activation_commit_id.len(), 64);
    assert!(state.grants_exit_permit());
    assert_eq!(
        executor.effects.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "the ambiguous activation boundary exits for recovery without retrying a target"
    );

    let (outcome_dto, process_action) =
        crate::adaptor::presenter::application_lifecycle::application_quit_outcome(outcome.clone());
    assert_eq!(
        process_action,
        Some(crate::usecase::shutdown_coordinator::ApplicationProcessAction::Exit { code: 23 }),
        "both transports must take the exit path"
    );
    let outcome_wire = serde_json::to_value(outcome_dto).expect("shared outcome presenter");
    let lookup_wire = serde_json::to_value(
        crate::adaptor::presenter::application_lifecycle::application_quit_lookup(
            crate::usecase::shutdown_coordinator::ApplicationQuitProjection::Shutdown {
                receipt: receipt.clone(),
                state: state.clone(),
            },
        ),
    )
    .expect("shared known-operation presenter");
    assert_eq!(outcome_wire["type"], "accepted");
    assert_eq!(lookup_wire["type"], "found");
    assert_eq!(outcome_wire["state"], lookup_wire["state"]);
    assert_eq!(outcome_wire["state"]["type"], "outcome_unknown");
    assert_eq!(outcome_wire["state"]["operation_id"], operation_id.as_str());
    assert_eq!(outcome_wire["state"]["shutdown_id"], shutdown_id.as_str());
    assert!(outcome_wire["state"].get("epoch").is_none());
    assert_eq!(
        outcome_wire["state"]["activation_commit_id"],
        activation_commit_id.as_str()
    );

    let durable = coordinator
        .get_application_quit_projection(operation_id)
        .await
        .expect("durable activation readback")
        .expect("accepted operation");
    if !matches!(
        &durable,
        crate::usecase::shutdown_coordinator::ApplicationQuitProjection::Shutdown {
            state: crate::usecase::shutdown_coordinator::ApplicationQuitState::Activated,
            ..
        }
    ) {
        panic!("activation commit must resolve durably after reply loss: {durable:?}");
    }

    let restart = crate::usecase::shutdown_coordinator::ShutdownCoordinator::new(
        harness.store.clone(),
        harness.store.clone(),
        executor.clone(),
        harness.store.installation_id().to_string(),
        "restart-boot".to_string(),
    );
    let restarted_current = restart
        .current_shutdown_read_model()
        .await
        .expect("restart current query")
        .expect("activation-possible plan remains anchored");
    assert_eq!(restarted_current.plan.shutdown_id, receipt.shutdown_id);
    assert_eq!(
        restarted_current.phase,
        ApplicationShutdownPhase::ReconciliationRequired
    );
    let restarted_known = restart
        .get_application_quit_projection(operation_id)
        .await
        .expect("restart known operation query")
        .expect("accepted operation remains known");
    let crate::usecase::shutdown_coordinator::ApplicationQuitProjection::Shutdown {
        receipt: restarted_receipt,
        ..
    } = restarted_known
    else {
        panic!("activation ambiguity must retain the normal shutdown projection");
    };
    assert_eq!(restarted_receipt.shutdown_id, receipt.shutdown_id);
    assert_eq!(
        executor.effects.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "restart readback must not start a second shutdown command"
    );
}

#[tokio::test]
async fn b059_b063_b092_pre_activation_abort_preserves_available_details_and_retry_matrix() {
    let harness = Harness::open();
    let LocalStateMutation::Obligation(mut recovery) =
        obligation_mutation("b063-fixed-closed-recovery", true)
    else {
        unreachable!("obligation helper always returns an obligation mutation");
    };
    let pending = recovery.pending.as_mut().expect("pending recovery entry");
    pending.owner = "b063-closed-session".to_string();
    pending.partition = PendingPartition::ClosedSession;
    let mut seed = batch(
        "b063-fixed-recovery-seed",
        "b063-fixed-recovery-seed",
        [63; 32],
        vec![],
        vec![],
        vec![LocalStateMutation::Obligation(recovery)],
    );
    seed.idempotency.installation_id = harness.store.installation_id().to_string();
    harness
        .store
        .commit_batch(seed)
        .await
        .expect("seed recovery present before shutdown snapshot");
    let executor = Arc::new(InventoryRaceShutdownExecutor {
        store: Arc::clone(&harness.store),
        current_targets: std::sync::Mutex::new(vec![
            crate::usecase::shutdown_coordinator::ShutdownTarget {
                target_id: "b063-successfully-prepared-session".to_string(),
                kind: "agent_session".to_string(),
            },
        ]),
        target_queries: std::sync::atomic::AtomicUsize::new(0),
        effects: std::sync::atomic::AtomicUsize::new(0),
    });
    let repository: Arc<dyn LocalEventTransactionRepository> = harness.store.clone();
    let authority: Arc<dyn crate::usecase::agent_session::operation::OperationBindingAuthority> =
        harness.store.clone();
    let coordinator = crate::usecase::shutdown_coordinator::ShutdownCoordinator::new(
        repository,
        authority,
        executor.clone(),
        harness.store.installation_id().to_string(),
        harness.store.process_instance_id().to_string(),
    );

    let outcome = coordinator
        .request(
            crate::usecase::shutdown_coordinator::ApplicationQuitRequest {
                principal: "desktop".to_string(),
                request_id: "quit-inventory-race".to_string(),
                intent: crate::usecase::shutdown_coordinator::ApplicationQuitIntent::Exit {
                    code: 0,
                },
            },
        )
        .await
        .expect("quit request");
    let crate::usecase::shutdown_coordinator::ApplicationQuitOutcome::Accepted {
        receipt,
        state:
            crate::usecase::shutdown_coordinator::ApplicationQuitState::FailedBeforeActivation {
                failure,
            },
        ..
    } = outcome
    else {
        panic!("inventory mismatch must abort before activation: {outcome:?}");
    };
    assert_eq!(
        failure.kind,
        SessionOperationFailureKind::TargetRevisionChanged
    );
    assert_eq!(
        executor
            .target_queries
            .load(std::sync::atomic::Ordering::SeqCst),
        2
    );
    assert_eq!(
        executor.effects.load(std::sync::atomic::Ordering::SeqCst),
        0
    );
    let current = coordinator
        .current_shutdown()
        .await
        .expect("current shutdown query")
        .expect("failed plan remains readable");
    assert_eq!(current.phase, ApplicationShutdownPhase::Failed);
    assert_eq!(current.details_state, ShutdownDetailsState::Available);
    assert_eq!(
        current.summary.outcome,
        Some(crate::domain::local_event::ShutdownOutcomeRecord::AbortedBeforeActivation)
    );
    assert_eq!(current.summary.target_count, Some(1));
    assert_eq!(current.summary.prepared_count, Some(1));
    assert_eq!(current.summary.completed_count, Some(0));
    assert_eq!(current.summary.unresolved_count, Some(1));
    assert_eq!(current.summary.recovery_snapshot_count, Some(1));
    assert_eq!(current.summary.shutdown_effect_count, Some(0));
    let plan_page = coordinator
        .shutdown_plan_page_read_model(current.plan.clone(), 128, None)
        .await
        .expect("failed-before-activation target page");
    assert_eq!(
        plan_page.plan.details_state,
        ShutdownDetailsState::Available
    );
    assert_eq!(plan_page.targets.len(), 1);
    assert_eq!(
        plan_page.targets[0].target_id,
        "b063-successfully-prepared-session"
    );
    assert_eq!(plan_page.targets[0].state, "cancelled_before_activation");
    let snapshot_id = current
        .summary
        .recovery_snapshot_id
        .clone()
        .expect("fixed recovery snapshot identity");
    let LocalEventQueryResult::PendingRecoverySnapshotPage(recovery_page) = harness
        .store
        .query(LocalEventQuery::PendingRecoverySnapshotPage {
            plan: current.plan.clone(),
            snapshot_id,
            partition: PendingPartition::ClosedSession,
            limit: 200,
            cursor: None,
        })
        .await
        .expect("failed-before-activation recovery detail")
    else {
        panic!("unexpected recovery snapshot query result");
    };
    assert_eq!(recovery_page.entries.len(), 1);
    assert!(matches!(
        &recovery_page.entries[0].detail,
        ShutdownTargetRecord::RecoverySnapshot { obligation_id, .. }
            if obligation_id == "b063-fixed-closed-recovery"
    ));

    let retry_projection = coordinator
        .current_shutdown_read_model()
        .await
        .expect("healthy failed projection")
        .expect("failed plan remains current");
    assert_eq!(retry_projection.actions, vec!["retry_quit"]);

    let connection = harness.raw_connection();
    let original_summary: String = connection
        .query_row(
            "SELECT summary FROM shutdown_plans WHERE shutdown_id = ?1",
            rusqlite::params![receipt.shutdown_id],
            |row| row.get(0),
        )
        .unwrap();
    let original_operation_status: String = connection
        .query_row(
            "SELECT latest_status FROM operation_records WHERE kind = 'application_quit' AND operation_id = ?1",
            rusqlite::params![receipt.operation_id],
            |row| row.get(0),
        )
        .unwrap();
    let terminal_commit_id: String = connection
        .query_row(
            "SELECT commit_id FROM shutdown_plans WHERE shutdown_id = ?1",
            rusqlite::params![receipt.shutdown_id],
            |row| row.get(0),
        )
        .unwrap();
    let nonterminal_commit_id: String = connection
        .query_row(
            "SELECT commit_id FROM logical_commits WHERE commit_id <> ?1 ORDER BY first_global_sequence LIMIT 1",
            rusqlite::params![terminal_commit_id],
            |row| row.get(0),
        )
        .expect("an earlier acceptance commit exists");

    for missing_condition in [
        "pre_activation_failed",
        "effect_zero",
        "terminal_fence",
        "admission_open",
        "same_boot",
        "known_terminal_state",
    ] {
        match missing_condition {
            "pre_activation_failed" => {
                connection
                    .execute(
                        "UPDATE shutdown_plans SET phase = 'cancelled' WHERE shutdown_id = ?1",
                        rusqlite::params![receipt.shutdown_id],
                    )
                    .unwrap();
            }
            "terminal_fence" => {
                connection
                    .execute(
                        "UPDATE operation_records SET commit_id = ?1 WHERE kind = 'application_quit' AND operation_id = ?2",
                        rusqlite::params![nonterminal_commit_id, receipt.operation_id],
                    )
                    .unwrap();
            }
            "known_terminal_state" => {
                connection
                    .execute(
                        "UPDATE operation_records SET latest_status = ?1 WHERE kind = 'application_quit' AND operation_id = ?2",
                        rusqlite::params![
                            serde_json::json!({
                                "schema": "application_quit_status_v1",
                                "state": {
                                    "type": "outcome_unknown",
                                    "operation_id": receipt.operation_id.clone(),
                                    "shutdown_id": receipt.shutdown_id.clone(),
                                    "activation_commit_id": "activation-unknown",
                                }
                            })
                            .to_string(),
                            receipt.operation_id,
                        ],
                    )
                    .unwrap();
            }
            key => {
                let mut summary: serde_json::Value =
                    serde_json::from_str(&original_summary).unwrap();
                match key {
                    "effect_zero" => summary["shutdown_effect_count"] = serde_json::json!(1),
                    "admission_open" => summary["admission_open"] = serde_json::json!(false),
                    "same_boot" => summary["process_instance_id"] = serde_json::json!("fresh-boot"),
                    _ => unreachable!(),
                }
                connection
                    .execute(
                        "UPDATE shutdown_plans SET summary = ?1 WHERE shutdown_id = ?2",
                        rusqlite::params![summary.to_string(), receipt.shutdown_id],
                    )
                    .unwrap();
            }
        }

        let durable_before: (String, String, i64, String, String) = connection
            .query_row(
                "SELECT p.phase, p.summary, p.revision, p.commit_id, o.latest_status
                 FROM shutdown_plans p JOIN operation_records o
                   ON o.kind = 'application_quit' AND o.operation_id = ?1
                 WHERE p.shutdown_id = ?2",
                rusqlite::params![receipt.operation_id, receipt.shutdown_id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .unwrap();
        let projection = coordinator
            .current_shutdown_read_model()
            .await
            .expect("matrix projection")
            .expect("matrix plan remains current");
        assert!(
            projection.actions.is_empty(),
            "RetryQuit leaked when {missing_condition} was false"
        );
        let durable_after: (String, String, i64, String, String) = connection
            .query_row(
                "SELECT p.phase, p.summary, p.revision, p.commit_id, o.latest_status
                 FROM shutdown_plans p JOIN operation_records o
                   ON o.kind = 'application_quit' AND o.operation_id = ?1
                 WHERE p.shutdown_id = ?2",
                rusqlite::params![receipt.operation_id, receipt.shutdown_id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(durable_after, durable_before, "query mutated durable state");
        assert_eq!(
            executor.effects.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "RetryQuit query performed an external effect"
        );

        connection
            .execute(
                "UPDATE shutdown_plans SET phase = 'failed', summary = ?1 WHERE shutdown_id = ?2",
                rusqlite::params![original_summary, receipt.shutdown_id],
            )
            .unwrap();
        connection
            .execute(
                "UPDATE operation_records SET latest_status = ?1, commit_id = ?2 WHERE kind = 'application_quit' AND operation_id = ?3",
                rusqlite::params![original_operation_status, terminal_commit_id, receipt.operation_id],
            )
            .unwrap();
    }

    // Failed-before-activation is terminal and explicitly reopens writer
    // admission so the raced workflow can continue and a later quit can take
    // a fresh complete inventory.
    let mut after_abort = batch(
        "commit-user-after-inventory-abort",
        "key-user-after-inventory-abort",
        [46; 32],
        vec![],
        vec![],
        vec![obligation_mutation("user-after-inventory-abort", true)],
    );
    after_abort.idempotency.operation_kind = CommitOperationKind::UserMutation;
    assert!(matches!(
        harness.store.commit_batch(after_abort).await,
        Ok(CommitBatchResult::Committed(_))
    ));
}

#[tokio::test]
async fn b059_pre_activation_revalidation_aborts_when_recovery_snapshot_missed_an_orphan() {
    let harness = Harness::open();
    let executor = TestShutdownExecutor::with_targets(0, ShutdownExecutorMode::Complete);
    let coordinator = shutdown_coordinator(&harness, &executor);
    coordinator.set_pre_acceptance_hook(Arc::new({
        let store = Arc::clone(&harness.store);
        move || {
            let store = Arc::clone(&store);
            Box::pin(async move {
                let mut late_orphan = batch(
                    "commit-late-orphan-before-quit",
                    "key-late-orphan-before-quit",
                    [47; 32],
                    vec![],
                    vec![],
                    vec![LocalStateMutation::Obligation(ObligationMutation {
                        obligation_id: "late-orphan-before-quit".to_string(),
                        record: send_obligation_fixture(
                            "late-orphan-before-quit",
                            ObligationStateRecord::Pending,
                        ),
                        pending: Some(PendingIndexEntry {
                            ordered_key: "orphan:late-orphan-before-quit".to_string(),
                            owner: "orphan-runtime".to_string(),
                            partition: PendingPartition::UnownedRuntime,
                            shutdown_plan: None,
                        }),
                        expected: RevisionGuard::Absent,
                        revision: Revision::new(0).unwrap(),
                    })],
                );
                late_orphan.idempotency.operation_kind = CommitOperationKind::UserMutation;
                assert!(matches!(
                    store.commit_batch(late_orphan).await,
                    Ok(CommitBatchResult::Committed(_))
                ));
            }) as std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>
        }
    }));

    let outcome = coordinator
        .request(
            crate::usecase::shutdown_coordinator::ApplicationQuitRequest {
                principal: "desktop".to_string(),
                request_id: "quit-recovery-inventory-race".to_string(),
                intent: crate::usecase::shutdown_coordinator::ApplicationQuitIntent::Exit {
                    code: 0,
                },
            },
        )
        .await
        .expect("quit request");
    let crate::usecase::shutdown_coordinator::ApplicationQuitOutcome::Accepted {
        state:
            crate::usecase::shutdown_coordinator::ApplicationQuitState::FailedBeforeActivation {
                failure,
            },
        ..
    } = outcome
    else {
        panic!("recovery mismatch must abort before activation: {outcome:?}");
    };
    assert_eq!(
        failure.kind,
        SessionOperationFailureKind::OwnerRevisionChanged
    );
    assert_eq!(
        executor.effects.load(std::sync::atomic::Ordering::SeqCst),
        0
    );
    assert_eq!(
        coordinator
            .current_shutdown()
            .await
            .unwrap()
            .expect("failed plan")
            .phase,
        ApplicationShutdownPhase::Failed
    );
}

#[tokio::test]
async fn completed_recovery_action_replays_exactly_and_preserves_pending_stop_identity() {
    let harness = Harness::open();
    let repository: Arc<dyn LocalEventTransactionRepository> = harness.store.clone();
    repository
        .commit_batch(batch(
            "seed-stop-recovery",
            "seed-stop-recovery",
            [81; 32],
            Vec::new(),
            Vec::new(),
            vec![LocalStateMutation::Obligation(ObligationMutation {
                obligation_id: "stop-obligation-1".to_string(),
                record: payload(
                    &serde_json::json!({
                        "schema": "stop_interrupt_obligation_v1",
                        "operation_id": "stop-operation-1",
                        "session_id": "session-1",
                        "turn_id": "1",
                        "expected_revision": 0,
                        "deadline_ms": i64::MAX,
                        "state": "reconciliation_required",
                    })
                    .to_string(),
                ),
                pending: Some(PendingIndexEntry {
                    ordered_key: "00000000000000000001-stop-obligation-1".to_string(),
                    owner: "session-1".to_string(),
                    partition: PendingPartition::Owner,
                    shutdown_plan: None,
                }),
                expected: RevisionGuard::Absent,
                revision: Revision::new(0).unwrap(),
            })],
        ))
        .await
        .unwrap();
    let usecase = crate::usecase::agent_session::operation::RecoveryActionUsecase::new(
        harness.store.clone(),
        harness.store.clone(),
        Arc::new(PendingRecoveryActionExecutor),
        harness.store.installation_id().to_string(),
    );
    let pending = usecase
        .pending(
            crate::usecase::agent_session::operation::PendingRecoveryQuery {
                limit: 32,
                partition: None,
                owner: None,
                shutdown_plan: None,
                cursor: None,
            },
        )
        .await
        .unwrap();
    let action_id = pending.entries[0]
        .action_identities
        .iter()
        .find(|identity| {
            identity.action == crate::domain::agent_session::events::RecoveryActionKind::ReadAgain
        })
        .unwrap()
        .action_id
        .clone();
    let request = crate::usecase::agent_session::operation::RecoveryActionRequest {
        action_id,
        obligation_id: "stop-obligation-1".to_string(),
        origin_revision: 0,
        action: crate::domain::agent_session::events::RecoveryActionKind::ReadAgain,
    };
    let first = usecase.request(request.clone()).await.unwrap();
    let crate::usecase::agent_session::operation::RecoveryActionOutcome::Completed {
        action_id,
        result,
    } = first
    else {
        panic!("recovery action must have a durable result");
    };
    assert_eq!(action_id, request.action_id);
    assert_eq!(
        result.classification,
        crate::domain::agent_session::events::RecoveryResultClassification::Pending
    );
    assert_eq!(result.resource_view, "The Stop terminal is still pending.");

    let replay = usecase.request(request.clone()).await.unwrap();
    let crate::usecase::agent_session::operation::RecoveryActionOutcome::Completed {
        action_id: replay_id,
        result: replay_result,
    } = replay
    else {
        panic!("completed action must replay");
    };
    assert_eq!(replay_id, action_id);
    assert_eq!(replay_result, result);

    for conflict in [
        crate::usecase::agent_session::operation::RecoveryActionRequest {
            action:
                crate::domain::agent_session::events::RecoveryActionKind::KeepForManualResolution,
            ..request.clone()
        },
        crate::usecase::agent_session::operation::RecoveryActionRequest {
            obligation_id: "other-obligation".to_string(),
            ..request.clone()
        },
        crate::usecase::agent_session::operation::RecoveryActionRequest {
            origin_revision: 1,
            ..request.clone()
        },
    ] {
        assert!(matches!(
            usecase.request(conflict).await,
            Err(crate::usecase::agent_session::operation::RecoveryActionError::NotFound)
        ));
    }

    let saved = repository
        .query(LocalEventQuery::ObligationByIdentity {
            obligation_id: request.obligation_id,
        })
        .await
        .unwrap();
    let LocalEventQueryResult::ObligationByIdentity(Some(saved)) = saved else {
        panic!("pending Stop obligation missing");
    };
    let ObligationRecord::RecoveryTransition {
        original,
        recovery_action,
    } = &saved.record
    else {
        panic!("saved Stop obligation must retain its recovery transition");
    };
    assert!(matches!(
        original.as_ref(),
        ObligationRecord::StopInterrupt {
            operation_id,
            deadline_ms: i64::MAX,
            ..
        } if operation_id == "stop-operation-1"
    ));
    assert_eq!(
        recovery_action.classification,
        Some(crate::domain::agent_session::events::RecoveryResultClassification::Pending)
    );
    assert!(saved.pending.is_some());
    assert_eq!(saved.revision, Revision::new(2).unwrap());
}

async fn assert_shared_shutdown_ingress(
    request_id: &str,
    intent: crate::usecase::shutdown_coordinator::ApplicationQuitIntent,
) {
    let harness = Harness::open();
    let executor = TestShutdownExecutor::with_targets(0, ShutdownExecutorMode::Complete);
    let coordinator = shutdown_coordinator(&harness, &executor);
    let (completed_tx, mut completed_rx) = tokio::sync::mpsc::unbounded_channel();
    let ingress = crate::adaptor::controller::application_lifecycle::ApplicationQuitIngress::new({
        let coordinator = Arc::clone(&coordinator);
        let request_id = request_id.to_string();
        move |intent| {
            let coordinator = Arc::clone(&coordinator);
            let request_id = request_id.clone();
            let completed_tx = completed_tx.clone();
            tokio::spawn(async move {
                completed_tx
                    .send(
                        coordinator
                            .request(
                                crate::usecase::shutdown_coordinator::ApplicationQuitRequest {
                                    principal: crate::adaptor::controller::agent_session_operation_wiring::LOCAL_INSTALLATION_OPERATION_PRINCIPAL.to_string(),
                                    request_id,
                                    intent,
                                },
                            )
                            .await,
                    )
                    .unwrap();
            });
        }
    });
    ingress.request(intent);
    let outcome = completed_rx.recv().await.unwrap().unwrap();
    let crate::usecase::shutdown_coordinator::ApplicationQuitOutcome::Accepted { receipt, state } =
        outcome
    else {
        panic!("graceful surface must reach the shared Accepted result");
    };
    assert_eq!(receipt.intent, intent);
    assert_eq!(
        state,
        crate::usecase::shutdown_coordinator::ApplicationQuitState::Completed
    );
    assert_eq!(
        executor
            .subordinate_shutdowns
            .load(std::sync::atomic::Ordering::SeqCst),
        1
    );
    assert_eq!(
        executor.effects.load(std::sync::atomic::Ordering::SeqCst),
        0
    );
}

async fn assert_programmatic_shutdown_permit(
    request_id: &str,
    intent: crate::adaptor::protocol::application_lifecycle_v1::ApplicationQuitIntentDtoV1,
    expected_intent: &str,
    expected_code: i32,
) {
    let harness = Harness::open();
    let executor = TestShutdownExecutor::with_targets(0, ShutdownExecutorMode::Complete);
    let coordinator = shutdown_coordinator(&harness, &executor);
    let (outcome, process_action) =
        crate::adaptor::controller::command::application_lifecycle::request_application_quit_result(
            coordinator.as_ref(),
            crate::adaptor::protocol::application_lifecycle_v1::ApplicationQuitRequestDtoV1 {
                request_id: request_id.to_string(),
                intent,
            },
        )
        .await
        .expect("programmatic quit reaches the shared coordinator");
    let crate::adaptor::protocol::application_lifecycle_v1::ApplicationQuitOutcomeDtoV1::Accepted {
        receipt,
        state,
    } = outcome
    else {
        panic!("programmatic quit must expose Accepted");
    };
    assert!(matches!(
        state,
        crate::adaptor::protocol::application_lifecycle_v1::ApplicationQuitStateDtoV1::Completed
    ));
    assert_eq!(receipt.intent, expected_intent);
    assert_eq!(receipt.exit_code, expected_code);
    let expected_action = match expected_intent {
        "exit" => crate::usecase::shutdown_coordinator::ApplicationProcessAction::Exit {
            code: expected_code,
        },
        "restart" => crate::usecase::shutdown_coordinator::ApplicationProcessAction::Restart {
            code: expected_code,
        },
        other => panic!("unexpected process destination {other}"),
    };
    assert_eq!(process_action, Some(expected_action));
    assert_eq!(
        executor
            .subordinate_shutdowns
            .load(std::sync::atomic::Ordering::SeqCst),
        1
    );
}

#[tokio::test]
async fn close_quit_cmd_q_routes_to_shared_shutdown() {
    assert_shared_shutdown_ingress(
        "decision-cmd-q",
        crate::infrastructure::platform::window_lifecycle::native_exit_intent(None),
    )
    .await;
}

#[tokio::test]
async fn close_quit_application_menu_routes_to_shared_shutdown() {
    assert_shared_shutdown_ingress(
        "decision-application-menu",
        crate::infrastructure::platform::window_lifecycle::native_exit_intent(None),
    )
    .await;
}

#[tokio::test]
async fn close_quit_dock_native_exit_uses_shared_shutdown_contract() {
    assert_shared_shutdown_ingress(
        "decision-dock",
        crate::infrastructure::platform::window_lifecycle::native_exit_intent(Some(23)),
    )
    .await;
}

#[tokio::test]
async fn close_quit_tray_routes_to_shared_shutdown() {
    assert_shared_shutdown_ingress(
        "decision-tray",
        crate::usecase::shutdown_coordinator::ApplicationQuitIntent::Exit { code: 0 },
    )
    .await;
}

#[tokio::test]
async fn close_quit_cooperative_os_exit_uses_shared_shutdown_contract() {
    assert_shared_shutdown_ingress(
        "decision-cooperative-os",
        crate::infrastructure::platform::window_lifecycle::native_exit_intent(Some(-15)),
    )
    .await;
}

#[tokio::test]
async fn close_quit_programmatic_exit_requires_coordinator_permit() {
    assert_programmatic_shutdown_permit(
        "decision-programmatic-exit",
        crate::adaptor::protocol::application_lifecycle_v1::ApplicationQuitIntentDtoV1::Exit {
            code: -7,
        },
        "exit",
        -7,
    )
    .await;
}

#[tokio::test]
async fn close_quit_programmatic_restart_requires_coordinator_permit() {
    assert_programmatic_shutdown_permit(
        "decision-programmatic-restart",
        crate::adaptor::protocol::application_lifecycle_v1::ApplicationQuitIntentDtoV1::Restart {
            code: 31,
        },
        "restart",
        31,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn close_quit_first_ingress_owns_exit_intent() {
    let harness = Harness::open();
    let executor = TestShutdownExecutor::with_targets(0, ShutdownExecutorMode::Complete);
    let coordinator = shutdown_coordinator(&harness, &executor);
    let first_entered = Arc::new(tokio::sync::Notify::new());
    let release_first = Arc::new(tokio::sync::Notify::new());
    let first_hook = Arc::new(std::sync::atomic::AtomicBool::new(true));
    coordinator.set_pre_acceptance_hook(Arc::new({
        let first_entered = Arc::clone(&first_entered);
        let release_first = Arc::clone(&release_first);
        let first_hook = Arc::clone(&first_hook);
        move || {
            let first_entered = Arc::clone(&first_entered);
            let release_first = Arc::clone(&release_first);
            let first_hook = Arc::clone(&first_hook);
            Box::pin(async move {
                if first_hook.swap(false, std::sync::atomic::Ordering::SeqCst) {
                    first_entered.notify_one();
                    release_first.notified().await;
                }
            }) as std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>
        }
    }));

    let surface_names = Arc::new([
        "cmd-q",
        "application-menu",
        "dock",
        "tray",
        "native-exit",
        "os-logout",
    ]);
    let next_surface = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let (completed_tx, mut completed_rx) = tokio::sync::mpsc::unbounded_channel();
    let ingress = crate::adaptor::controller::application_lifecycle::ApplicationQuitIngress::new({
        let coordinator = Arc::clone(&coordinator);
        let surface_names = Arc::clone(&surface_names);
        let next_surface = Arc::clone(&next_surface);
        move |intent| {
            let ordinal = next_surface.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let request_id = surface_names[ordinal].to_string();
            let coordinator = Arc::clone(&coordinator);
            let completed_tx = completed_tx.clone();
            tokio::spawn(async move {
                let outcome = coordinator
                    .request(
                        crate::usecase::shutdown_coordinator::ApplicationQuitRequest {
                            principal: crate::adaptor::controller::agent_session_operation_wiring::LOCAL_INSTALLATION_OPERATION_PRINCIPAL.to_string(),
                            request_id: request_id.clone(),
                            intent,
                        },
                    )
                    .await;
                completed_tx.send((request_id, outcome)).unwrap();
            });
        }
    });

    // Tauri exposes the five native graceful sources as the same
    // ExitRequested event (their origin must not be guessed); tray has its
    // explicit callback. Both production paths call this ingress object.
    ingress
        .request(crate::infrastructure::platform::window_lifecycle::native_exit_intent(Some(17)));
    first_entered.notified().await;
    for intent in [
        crate::usecase::shutdown_coordinator::ApplicationQuitIntent::Restart { code: 99 },
        crate::usecase::shutdown_coordinator::ApplicationQuitIntent::Exit { code: -1 },
        crate::usecase::shutdown_coordinator::ApplicationQuitIntent::Restart { code: 0 },
        crate::usecase::shutdown_coordinator::ApplicationQuitIntent::Exit { code: 42 },
        crate::usecase::shutdown_coordinator::ApplicationQuitIntent::Restart { code: -9 },
    ] {
        ingress.request(intent);
    }
    while coordinator.registered_ingress_count() != 6 {
        tokio::task::yield_now().await;
    }
    release_first.notify_one();

    let mut accepted = Vec::new();
    for _ in 0..6 {
        let (_surface, outcome) = completed_rx.recv().await.unwrap();
        let outcome = outcome.unwrap();
        let crate::usecase::shutdown_coordinator::ApplicationQuitOutcome::Accepted {
            receipt,
            state,
        } = outcome
        else {
            panic!("every graceful ingress must join Accepted");
        };
        assert_eq!(
            state,
            crate::usecase::shutdown_coordinator::ApplicationQuitState::Completed
        );
        accepted.push(receipt);
    }
    assert!(accepted.iter().all(|receipt| receipt == &accepted[0]));
    assert_eq!(
        accepted[0].intent,
        crate::usecase::shutdown_coordinator::ApplicationQuitIntent::Exit { code: 17 },
        "B-091 fixes the first surface's intent even when later IDs request a different mode/code"
    );
    assert_eq!(
        executor.effects.load(std::sync::atomic::Ordering::SeqCst),
        0
    );
    assert_eq!(
        executor
            .subordinate_shutdowns
            .load(std::sync::atomic::Ordering::SeqCst),
        1
    );
    assert!(coordinator.current_shutdown().await.unwrap().is_none());
    assert_eq!(
        coordinator
            .request(
                crate::usecase::shutdown_coordinator::ApplicationQuitRequest {
                    principal: crate::adaptor::controller::agent_session_operation_wiring::LOCAL_INSTALLATION_OPERATION_PRINCIPAL.to_string(),
                    request_id: "cmd-q".to_string(),
                    intent: crate::usecase::shutdown_coordinator::ApplicationQuitIntent::Restart {
                        code: 99,
                    },
                },
            )
            .await,
        Err(crate::usecase::shutdown_coordinator::ApplicationQuitError::PayloadConflict)
    );

    let connection = harness.raw_connection();
    let operation_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM operation_records WHERE kind = 'application_quit'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let plan_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM shutdown_plans", [], |row| row.get(0))
        .unwrap();
    let binding_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM operation_bindings WHERE kind = 'application_quit'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(operation_count, 1);
    assert_eq!(plan_count, 1);
    assert_eq!(binding_count, 6);

    let plan = ShutdownPlanKey {
        shutdown_id: accepted[0].shutdown_id.clone(),
    };
    let before = coordinator
        .shutdown_plan_page_read_model(plan.clone(), 128, None)
        .await
        .unwrap();
    assert_eq!(before.plan.details_state, ShutdownDetailsState::Available);
    assert_eq!(before.plan.outcome.as_deref(), Some("completed"));
    coordinator
        .compact_shutdown_details_read_model(plan.clone())
        .await
        .unwrap();
    let after = coordinator
        .shutdown_plan_page_read_model(plan, 128, None)
        .await
        .unwrap();
    assert_eq!(after.plan.details_state, ShutdownDetailsState::Compacted);
    assert_eq!(after.plan.operation_id, before.plan.operation_id);
    assert_eq!(after.plan.intent, before.plan.intent);
    assert_eq!(after.plan.deadline_ms, before.plan.deadline_ms);
    assert_eq!(after.plan.outcome, before.plan.outcome);
    assert!(after.targets.is_empty());
    assert!(after.next_cursor.is_none());
}

#[tokio::test]
async fn b087_quit_request_identity_accepts_exact_bounds_and_rejects_without_side_effects() {
    use crate::usecase::shutdown_coordinator::{
        ApplicationQuitError, ApplicationQuitIntent, ApplicationQuitOutcome, ApplicationQuitRequest,
    };

    for request_id in ["q".to_string(), "a".repeat(128)] {
        let harness = Harness::open();
        let executor = TestShutdownExecutor::with_targets(0, ShutdownExecutorMode::Complete);
        let outcome = shutdown_coordinator(&harness, &executor)
            .request(ApplicationQuitRequest {
                principal: "desktop".to_string(),
                request_id,
                intent: ApplicationQuitIntent::Exit { code: 0 },
            })
            .await
            .expect("valid boundary identity");
        assert!(matches!(outcome, ApplicationQuitOutcome::Accepted { .. }));
    }

    for request_id in [
        String::new(),
        "a".repeat(129),
        "quit-é".to_string(),
        "quit request".to_string(),
        "quit/request".to_string(),
    ] {
        let harness = Harness::open();
        let executor = TestShutdownExecutor::with_targets(0, ShutdownExecutorMode::Complete);
        let coordinator = shutdown_coordinator(&harness, &executor);
        assert_eq!(
            coordinator
                .request(ApplicationQuitRequest {
                    principal: "desktop".to_string(),
                    request_id,
                    intent: ApplicationQuitIntent::Exit { code: 0 },
                })
                .await,
            Err(ApplicationQuitError::InvalidRequest)
        );
        assert!(matches!(
            crate::adaptor::presenter::application_lifecycle::application_quit_error(
                ApplicationQuitError::InvalidRequest,
            ),
            Some(
                crate::adaptor::protocol::agent_session_v1::ApplicationQuitErrorDtoV1::InvalidRequest
            )
        ));
        assert_eq!(
            executor
                .target_queries
                .load(std::sync::atomic::Ordering::SeqCst),
            0
        );
        assert_eq!(
            executor.effects.load(std::sync::atomic::Ordering::SeqCst),
            0
        );
        let connection = harness.raw_connection();
        for table in [
            "operation_bindings",
            "operation_records",
            "caller_attempts",
            "shutdown_plans",
            "shutdown_targets",
        ] {
            let count: i64 = connection
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .expect("side-effect count");
            assert_eq!(count, 0, "invalid identity mutated {table}");
        }
        assert!(coordinator.current_shutdown().await.unwrap().is_none());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn b061_terminal_plan_allows_one_new_flight_same_process_and_after_restart_concurrency() {
    use crate::usecase::shutdown_coordinator::{
        ApplicationQuitIntent, ApplicationQuitOutcome, ApplicationQuitRequest,
        ApplicationQuitState, CurrentApplicationShutdownProjection, ShutdownCoordinator,
    };

    let harness = Harness::open();
    let executor = TestShutdownExecutor::with_targets(0, ShutdownExecutorMode::Complete);
    let coordinator = shutdown_coordinator(&harness, &executor);
    let first = coordinator
        .request(ApplicationQuitRequest {
            principal: "desktop".to_string(),
            request_id: "b061-first-terminal".to_string(),
            intent: ApplicationQuitIntent::Exit { code: 0 },
        })
        .await
        .expect("first flight");
    let ApplicationQuitOutcome::Accepted {
        receipt: first_receipt,
        state: ApplicationQuitState::Completed,
    } = first
    else {
        panic!("first flight must complete");
    };
    let second = coordinator
        .request(ApplicationQuitRequest {
            principal: "desktop".to_string(),
            request_id: "b061-second-terminal".to_string(),
            intent: ApplicationQuitIntent::Restart { code: 42 },
        })
        .await
        .expect("one Available terminal plan does not block a new flight");
    let ApplicationQuitOutcome::Accepted {
        receipt: second_receipt,
        state: ApplicationQuitState::Completed,
    } = second
    else {
        panic!("second flight must complete");
    };
    assert_ne!(second_receipt.operation_id, first_receipt.operation_id);
    assert_eq!(
        second_receipt.intent,
        ApplicationQuitIntent::Restart { code: 42 }
    );
    let connection = harness.raw_connection();
    let operation_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM operation_records WHERE kind = 'application_quit'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let plan_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM shutdown_plans", [], |row| row.get(0))
        .unwrap();
    assert_eq!(operation_count, 2);
    assert_eq!(plan_count, 2);
    assert_eq!(
        executor
            .subordinate_shutdowns
            .load(std::sync::atomic::Ordering::SeqCst),
        2
    );
    assert_eq!(
        executor.effects.load(std::sync::atomic::Ordering::SeqCst),
        0
    );
    let target_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM shutdown_targets", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(target_count, 0);
    drop(connection);

    // Make the oldest terminal detail explicitly eligible for a future
    // flight, then cross a physical boot. Merely reading the previous-boot
    // terminal plan must not enumerate targets or start any shutdown effect.
    coordinator
        .compact_shutdown_details(ShutdownPlanKey {
            shutdown_id: first_receipt.shutdown_id,
        })
        .await
        .expect("terminal detail compaction");
    let target_queries_before_restart = executor
        .target_queries
        .load(std::sync::atomic::Ordering::SeqCst);
    let subordinate_before_restart = executor
        .subordinate_shutdowns
        .load(std::sync::atomic::Ordering::SeqCst);
    let restarted = Arc::new(ShutdownCoordinator::new(
        harness.store.clone(),
        harness.store.clone(),
        executor.clone(),
        harness.store.installation_id().to_string(),
        "b061-restarted-boot".to_string(),
    ));
    assert_eq!(
        restarted
            .current_application_shutdown_projection()
            .await
            .expect("previous-boot terminal query"),
        CurrentApplicationShutdownProjection::Current(None),
    );
    assert_eq!(
        executor
            .target_queries
            .load(std::sync::atomic::Ordering::SeqCst),
        target_queries_before_restart,
    );
    assert_eq!(
        executor
            .subordinate_shutdowns
            .load(std::sync::atomic::Ordering::SeqCst),
        subordinate_before_restart,
    );
    assert_eq!(
        executor.effects.load(std::sync::atomic::Ordering::SeqCst),
        0
    );

    // Four callers that enter before the new flight completes all join one
    // canonical plan. Restart cannot turn them into four new plans.
    let first_entered = Arc::new(tokio::sync::Notify::new());
    let release_first = Arc::new(tokio::sync::Notify::new());
    let first_hook = Arc::new(std::sync::atomic::AtomicBool::new(true));
    restarted.set_pre_acceptance_hook(Arc::new({
        let first_entered = Arc::clone(&first_entered);
        let release_first = Arc::clone(&release_first);
        let first_hook = Arc::clone(&first_hook);
        move || {
            let first_entered = Arc::clone(&first_entered);
            let release_first = Arc::clone(&release_first);
            let first_hook = Arc::clone(&first_hook);
            Box::pin(async move {
                if first_hook.swap(false, std::sync::atomic::Ordering::SeqCst) {
                    first_entered.notify_one();
                    release_first.notified().await;
                }
            }) as std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>
        }
    }));
    let mut requests = Vec::new();
    for ordinal in 0..4 {
        let restarted = Arc::clone(&restarted);
        requests.push(tokio::spawn(async move {
            restarted
                .request(ApplicationQuitRequest {
                    principal: "desktop".to_string(),
                    request_id: format!("b061-restarted-concurrent-{ordinal}"),
                    intent: ApplicationQuitIntent::Restart { code: 61 },
                })
                .await
        }));
    }
    first_entered.notified().await;
    while restarted.registered_ingress_count() != 4 {
        tokio::task::yield_now().await;
    }
    release_first.notify_one();

    let mut concurrent_receipts = Vec::new();
    for request in requests {
        let ApplicationQuitOutcome::Accepted {
            receipt,
            state: ApplicationQuitState::Completed,
        } = request.await.unwrap().unwrap()
        else {
            panic!("every pre-completion caller must join one restarted flight");
        };
        concurrent_receipts.push(receipt);
    }
    assert!(concurrent_receipts
        .iter()
        .all(|receipt| receipt == &concurrent_receipts[0]));
    assert_ne!(
        concurrent_receipts[0].operation_id,
        second_receipt.operation_id
    );
    assert_eq!(
        concurrent_receipts[0].intent,
        ApplicationQuitIntent::Restart { code: 61 }
    );

    let connection = harness.raw_connection();
    let operation_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM operation_records WHERE kind = 'application_quit'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let plan_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM shutdown_plans", [], |row| row.get(0))
        .unwrap();
    let target_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM shutdown_targets", [], |row| {
            row.get(0)
        })
        .unwrap();
    let binding_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM operation_bindings WHERE kind = 'application_quit'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(operation_count, 3);
    assert_eq!(plan_count, 3);
    assert_eq!(target_count, 0);
    assert_eq!(binding_count, 6);
    assert_eq!(
        executor
            .subordinate_shutdowns
            .load(std::sync::atomic::Ordering::SeqCst),
        3,
    );
    assert_eq!(
        executor.effects.load(std::sync::atomic::Ordering::SeqCst),
        0,
    );
    assert_eq!(
        executor.readbacks.load(std::sync::atomic::Ordering::SeqCst),
        0,
    );
}

#[tokio::test]
async fn b064_activated_hanging_quit_decides_reconciliation_within_the_fixed_deadline() {
    let harness = Harness::open();
    let executor = TestShutdownExecutor::with_targets(1, ShutdownExecutorMode::Hang);
    let coordinator = shutdown_coordinator(&harness, &executor);
    let started = Instant::now();
    let outcome = coordinator
        .request(
            crate::usecase::shutdown_coordinator::ApplicationQuitRequest {
                principal: "desktop".to_string(),
                request_id: "quit-hang".to_string(),
                intent: crate::usecase::shutdown_coordinator::ApplicationQuitIntent::Exit {
                    code: 0,
                },
            },
        )
        .await
        .unwrap();
    assert!(
        started.elapsed() <= std::time::Duration::from_millis(15_500),
        "shutdown exceeded its fixed decision deadline: {:?}",
        started.elapsed()
    );
    let crate::usecase::shutdown_coordinator::ApplicationQuitOutcome::Accepted { receipt, state } =
        outcome
    else {
        panic!("activated shutdown remains Accepted");
    };
    assert!(matches!(
        state,
        crate::usecase::shutdown_coordinator::ApplicationQuitState::ReconciliationRequired { .. }
    ));
    assert!(state.grants_exit_permit());
    assert_eq!(
        executor.effects.load(std::sync::atomic::Ordering::SeqCst),
        1
    );
    assert_eq!(
        executor
            .subordinate_shutdowns
            .load(std::sync::atomic::Ordering::SeqCst),
        0
    );
    let current = coordinator
        .current_shutdown_read_model()
        .await
        .unwrap()
        .expect("activated unresolved plan");
    assert_eq!(
        current.phase,
        ApplicationShutdownPhase::ReconciliationRequired
    );
    assert_eq!(current.operation_id, receipt.operation_id);
    assert_eq!(current.unresolved_count, Some(1));
    let page = coordinator
        .shutdown_plan_page_read_model(
            ShutdownPlanKey {
                shutdown_id: receipt.shutdown_id,
            },
            128,
            None,
        )
        .await
        .unwrap();
    assert_eq!(page.targets.len(), 1);
    assert_eq!(page.targets[0].state, "reconciliation_required");
    assert_eq!(page.targets[0].actions, vec!["retry_same_effect"]);

    let connection = harness.raw_connection();
    let operation_count_before: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM operation_records WHERE kind = 'application_quit'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let plan_count_before: i64 = connection
        .query_row("SELECT COUNT(*) FROM shutdown_plans", [], |row| row.get(0))
        .unwrap();
    let effects_before = executor.effects.load(std::sync::atomic::Ordering::SeqCst);
    let target_queries_before = executor
        .target_queries
        .load(std::sync::atomic::Ordering::SeqCst);
    let blocked = coordinator
        .request(
            crate::usecase::shutdown_coordinator::ApplicationQuitRequest {
                principal: "desktop".to_string(),
                request_id: "quit-blocked-by-unresolved".to_string(),
                intent: crate::usecase::shutdown_coordinator::ApplicationQuitIntent::Restart {
                    code: 42,
                },
            },
        )
        .await;
    let Err(
        crate::usecase::shutdown_coordinator::ApplicationQuitError::PreviousShutdownReconciliationRequired {
            blocking,
        },
    ) = blocked
    else {
        panic!("same-boot unresolved shutdown must return its authoritative blocker");
    };
    assert_eq!(blocking.plan, current.plan);
    assert_eq!(blocking.phase, current.phase);
    assert_eq!(blocking.operation_id, current.operation_id);
    assert_eq!(blocking.actions, current.actions);
    let public = crate::adaptor::presenter::application_lifecycle::application_quit_failure(
        crate::usecase::shutdown_coordinator::ApplicationQuitError::PreviousShutdownReconciliationRequired {
            blocking: blocking.clone(),
        },
    )
    .expect("reconciliation blocker is a public outcome");
    let crate::adaptor::protocol::application_lifecycle_v1::ApplicationQuitOutcomeDtoV1::PreviousShutdownReconciliationRequired {
        blocking: public_blocker,
    } = public else {
        panic!("wrong public reconciliation blocker outcome");
    };
    assert_eq!(public_blocker.shutdown_id, current.plan.shutdown_id);
    assert_eq!(public_blocker.actions, current.actions);
    let operation_count_after: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM operation_records WHERE kind = 'application_quit'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let plan_count_after: i64 = connection
        .query_row("SELECT COUNT(*) FROM shutdown_plans", [], |row| row.get(0))
        .unwrap();
    assert_eq!(operation_count_after, operation_count_before);
    assert_eq!(plan_count_after, plan_count_before);
    assert_eq!(
        executor.effects.load(std::sync::atomic::Ordering::SeqCst),
        effects_before
    );
    assert_eq!(
        executor
            .target_queries
            .load(std::sync::atomic::Ordering::SeqCst),
        target_queries_before
    );
}

#[tokio::test]
async fn b064_restart_keeps_completed_targets_and_does_not_replay_shutdown_effects() {
    let harness = Harness::open();
    let executor = Arc::new(TestShutdownExecutor {
        targets: vec![
            crate::usecase::shutdown_coordinator::ShutdownTarget {
                target_id: "b064-completed-session-with-child".to_string(),
                kind: "agent_session".to_string(),
            },
            crate::usecase::shutdown_coordinator::ShutdownTarget {
                target_id: "b064-completed-workflow".to_string(),
                kind: "workflow_execution".to_string(),
            },
            crate::usecase::shutdown_coordinator::ShutdownTarget {
                target_id: "b064-hanging-target".to_string(),
                kind: "agent_session".to_string(),
            },
        ],
        mode: ShutdownExecutorMode::HangNamedTarget,
        target_queries: std::sync::atomic::AtomicUsize::new(0),
        effects: std::sync::atomic::AtomicUsize::new(0),
        readbacks: std::sync::atomic::AtomicUsize::new(0),
        subordinate_shutdowns: std::sync::atomic::AtomicUsize::new(0),
        drop_reply_on_readback: None,
    });
    let coordinator = shutdown_coordinator(&harness, &executor);
    let outcome = coordinator
        .request(
            crate::usecase::shutdown_coordinator::ApplicationQuitRequest {
                principal: "desktop".to_string(),
                request_id: "b064-partial-before-restart".to_string(),
                intent: crate::usecase::shutdown_coordinator::ApplicationQuitIntent::Exit {
                    code: 0,
                },
            },
        )
        .await
        .unwrap();
    let crate::usecase::shutdown_coordinator::ApplicationQuitOutcome::Accepted {
        receipt,
        state:
            crate::usecase::shutdown_coordinator::ApplicationQuitState::ReconciliationRequired {
                ..
            },
    } = outcome
    else {
        panic!("activated partial shutdown must exit with recovery: {outcome:?}");
    };
    assert_eq!(
        executor.effects.load(std::sync::atomic::Ordering::SeqCst),
        3
    );
    assert_eq!(
        executor
            .subordinate_shutdowns
            .load(std::sync::atomic::Ordering::SeqCst),
        0
    );
    let before_restart = coordinator
        .shutdown_plan_page_read_model(
            ShutdownPlanKey {
                shutdown_id: receipt.shutdown_id.clone(),
            },
            128,
            None,
        )
        .await
        .unwrap();
    assert_eq!(before_restart.plan.completed_count, Some(2));
    assert_eq!(before_restart.plan.unresolved_count, Some(1));
    assert_eq!(
        before_restart
            .targets
            .iter()
            .filter(|target| target.state == "completed")
            .count(),
        2
    );

    let restart_executor = TestShutdownExecutor::with_targets(3, ShutdownExecutorMode::Complete);
    let repository: Arc<dyn LocalEventTransactionRepository> = harness.store.clone();
    let authority: Arc<dyn crate::usecase::agent_session::operation::OperationBindingAuthority> =
        harness.store.clone();
    let restarted = crate::usecase::shutdown_coordinator::ShutdownCoordinator::new(
        repository,
        authority,
        restart_executor.clone(),
        harness.store.installation_id().to_string(),
        "b064-restarted-boot".to_string(),
    );
    let recovered = restarted
        .current_shutdown_read_model()
        .await
        .unwrap()
        .expect("previous-boot unresolved shutdown");
    assert_eq!(recovered.operation_id, receipt.operation_id);
    assert_eq!(
        recovered.phase,
        ApplicationShutdownPhase::ReconciliationRequired
    );
    assert_eq!(recovered.completed_count, Some(2));
    assert_eq!(recovered.unresolved_count, Some(1));
    let after_restart = restarted
        .shutdown_plan_page_read_model(recovered.plan.clone(), 128, None)
        .await
        .unwrap();
    assert_eq!(
        after_restart
            .targets
            .iter()
            .filter(|target| target.state == "completed")
            .map(|target| target.target_id.as_str())
            .collect::<Vec<_>>(),
        vec![
            "b064-completed-session-with-child",
            "b064-completed-workflow"
        ]
    );
    assert_eq!(
        restart_executor
            .target_queries
            .load(std::sync::atomic::Ordering::SeqCst),
        0
    );
    assert_eq!(
        restart_executor
            .effects
            .load(std::sync::atomic::Ordering::SeqCst),
        0
    );
    assert_eq!(
        restart_executor
            .readbacks
            .load(std::sync::atomic::Ordering::SeqCst),
        0
    );
    assert_eq!(
        restart_executor
            .subordinate_shutdowns
            .load(std::sync::atomic::Ordering::SeqCst),
        0
    );
}

#[tokio::test]
async fn close_quit_hard_kill_recovers_as_crash() {
    let harness = Harness::open();
    let plan = ShutdownPlanKey {
        shutdown_id: "quit-exit-coupled".to_string(),
    };
    let summary = serde_json::json!({
        "schema": "shutdown_plan_summary_v1",
        "operation_id": "quit-exit-coupled",
        "intent": "exit",
        "exit_code": 0,
        "t0_ms": 1_000,
        "preparation_cutoff_ms": 14_000,
        "deadline_ms": 16_000,
        "target_count": 1,
        "prepared_count": 1,
        "effect_reserved_count": 0,
        "terminal_count": 0,
        "completed_count": 0,
        "unresolved_count": 1,
        "recovery_snapshot_count": 0,
        "recovery_snapshot_id": null,
        "process_instance_id": "previous-boot",
    });
    let mut seed = batch(
        "b066-exit-coupled-seed",
        "b066-exit-coupled-seed",
        [66; 32],
        Vec::new(),
        Vec::new(),
        vec![
            LocalStateMutation::ShutdownPlan(ShutdownPlanMutation {
                key: plan.clone(),
                phase: ApplicationShutdownPhase::Activated,
                summary: payload(&summary.to_string()),
                details_state: ShutdownDetailsState::Available,
                expected: RevisionGuard::Absent,
                revision: Revision::new(1).unwrap(),
            }),
            LocalStateMutation::ShutdownTarget(ShutdownTargetMutation {
                key: plan.clone(),
                ordinal: 0,
                detail: payload(
                    &serde_json::json!({
                        "schema": "shutdown_target_v1",
                        "target_id": "session-with-child",
                        "kind": "agent_session",
                        "state": "prepared",
                        "effect_identity": "shutdown-target/quit-exit-coupled/0",
                    })
                    .to_string(),
                ),
                expected: RevisionGuard::Absent,
                revision: Revision::new(0).unwrap(),
            }),
            LocalStateMutation::OperationRecord(OperationRecordMutation {
                kind: OperationKind::ApplicationQuit,
                operation_id: "quit-exit-coupled".to_string(),
                receipt: payload(
                    &serde_json::json!({
                        "schema": "application_quit_receipt_v1",
                        "operation_id": "quit-exit-coupled",
                        "shutdown_id": "quit-exit-coupled",
                        "intent": "exit",
                        "exit_code": 0,
                        "t0_ms": 1_000,
                        "deadline_ms": 16_000,
                        "binding_hmac": "00".repeat(32),
                    })
                    .to_string(),
                ),
                latest_status: payload(
                    &serde_json::json!({
                        "schema": "application_quit_status_v1",
                        "state": { "type": "activated" },
                    })
                    .to_string(),
                ),
                expected: RevisionGuard::Absent,
                revision: Revision::new(1).unwrap(),
            }),
            LocalStateMutation::ShutdownLatestPointer(ShutdownLatestPointerMutation {
                expected: None,
                new: Some(plan.clone()),
            }),
        ],
    );
    seed.idempotency.installation_id = harness.store.installation_id().to_string();
    seed.idempotency.operation_kind = CommitOperationKind::ApplicationQuit;
    harness
        .store
        .commit_batch(seed)
        .await
        .expect("seed previous-boot activated plan");

    let executor = TestShutdownExecutor::with_targets(1, ShutdownExecutorMode::Complete);
    let repository: Arc<dyn LocalEventTransactionRepository> = harness.store.clone();
    let authority: Arc<dyn crate::usecase::agent_session::operation::OperationBindingAuthority> =
        harness.store.clone();
    let coordinator = crate::usecase::shutdown_coordinator::ShutdownCoordinator::new(
        repository,
        authority,
        executor.clone(),
        harness.store.installation_id().to_string(),
        "current-boot".to_string(),
    );

    let current = coordinator
        .current_shutdown_read_model()
        .await
        .expect("current shutdown query")
        .expect("previous nonterminal plan remains current");
    assert_eq!(
        current.phase,
        ApplicationShutdownPhase::ReconciliationRequired
    );
    assert_eq!(current.plan, plan);

    let page = coordinator
        .shutdown_plan_page_read_model(plan.clone(), 128, None)
        .await
        .expect("previous-boot plan page");
    assert_eq!(
        page.plan.phase,
        ApplicationShutdownPhase::ReconciliationRequired
    );
    assert_eq!(page.targets.len(), 1);
    let target = &page.targets[0];
    assert_eq!(target.state, "reconciliation_required");
    assert_eq!(
        target.effect_identity,
        "shutdown-target/quit-exit-coupled/0"
    );
    assert_eq!(
        target.observation,
        Some(SafeEffectObservation::ExitCoupledOutcomeUnknown {
            shutdown_id: plan.shutdown_id.clone(),
        })
    );
    assert_eq!(target.actions, vec!["retry_same_effect"]);

    let connection = harness.raw_connection();
    let operation_count_before: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM operation_records WHERE kind = 'application_quit'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let plan_count_before: i64 = connection
        .query_row("SELECT COUNT(*) FROM shutdown_plans", [], |row| row.get(0))
        .unwrap();
    let blocked = coordinator
        .request(
            crate::usecase::shutdown_coordinator::ApplicationQuitRequest {
                principal: "desktop".to_string(),
                request_id: "b039-new-quit-after-previous-boot-plan".to_string(),
                intent: crate::usecase::shutdown_coordinator::ApplicationQuitIntent::Restart {
                    code: 42,
                },
            },
        )
        .await;
    let Err(
        crate::usecase::shutdown_coordinator::ApplicationQuitError::PreviousShutdownReconciliationRequired {
            blocking,
        },
    ) = blocked
    else {
        panic!("a previous-boot nonterminal plan must block a new quit");
    };
    assert_eq!(blocking.plan, plan);
    assert_eq!(
        blocking.phase,
        ApplicationShutdownPhase::ReconciliationRequired
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM operation_records WHERE kind = 'application_quit'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        operation_count_before
    );
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM shutdown_plans", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        plan_count_before
    );

    let wire = serde_json::to_value(crate::adaptor::presenter::application_lifecycle::plan_page(
        page,
    ))
    .expect("shared Tauri/WebSocket shutdown presenter");
    assert_eq!(wire["targets"][0]["state"], "reconciliation_required");
    assert_eq!(
        wire["targets"][0]["effect_identity"],
        "shutdown-target/quit-exit-coupled/0"
    );
    assert_eq!(
        wire["targets"][0]["observation"],
        serde_json::json!({
            "type": "exit_coupled_outcome_unknown",
            "shutdown_id": "quit-exit-coupled",
        })
    );
    assert_eq!(
        executor.effects.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "restart readback must not replay the shutdown effect"
    );
    assert_eq!(
        executor.readbacks.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "restart readback must not call the provider readback port"
    );
    assert_eq!(
        executor
            .subordinate_shutdowns
            .load(std::sync::atomic::Ordering::SeqCst),
        0,
        "restart readback must not infer or perform child cleanup"
    );
}

#[tokio::test(start_paused = true)]
async fn hanging_target_inventory_aborts_before_activation_at_the_absolute_cutoff() {
    let harness = Harness::open();
    let executor = TestShutdownExecutor::with_targets(1, ShutdownExecutorMode::HangTargets);
    let coordinator = shutdown_coordinator(&harness, &executor);
    let task_coordinator = Arc::clone(&coordinator);
    let request = tokio::spawn(async move {
        task_coordinator
            .request(
                crate::usecase::shutdown_coordinator::ApplicationQuitRequest {
                    principal: "desktop".to_string(),
                    request_id: "quit-hanging-inventory".to_string(),
                    intent: crate::usecase::shutdown_coordinator::ApplicationQuitIntent::Exit {
                        code: 0,
                    },
                },
            )
            .await
    });
    for _ in 0..1_000 {
        if executor
            .target_queries
            .load(std::sync::atomic::Ordering::SeqCst)
            == 1
        {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert_eq!(
        executor
            .target_queries
            .load(std::sync::atomic::Ordering::SeqCst),
        1,
        "request did not reach the target inventory"
    );

    tokio::time::advance(std::time::Duration::from_secs(13)).await;
    let outcome = request.await.unwrap().unwrap();
    let crate::usecase::shutdown_coordinator::ApplicationQuitOutcome::RejectedBeforeCommit {
        failure,
    } = outcome
    else {
        panic!("a known pre-acceptance timeout must abort before commit");
    };
    assert_eq!(failure.kind, SessionOperationFailureKind::DeadlineExceeded);
    assert_eq!(
        executor.effects.load(std::sync::atomic::Ordering::SeqCst),
        0
    );
    assert_eq!(
        executor
            .subordinate_shutdowns
            .load(std::sync::atomic::Ordering::SeqCst),
        0
    );
    assert!(coordinator.current_shutdown().await.unwrap().is_none());
}

#[tokio::test(start_paused = true)]
async fn b067_late_aborted_inventory_result_cannot_change_a_new_shutdown_flight() {
    use crate::usecase::shutdown_coordinator::{
        ApplicationQuitIntent, ApplicationQuitOutcome, ApplicationQuitRequest, ApplicationQuitState,
    };

    let harness = Harness::open();
    let executor = LateInventoryShutdownExecutor::new();
    let repository: Arc<dyn LocalEventTransactionRepository> = harness.store.clone();
    let authority: Arc<dyn crate::usecase::agent_session::operation::OperationBindingAuthority> =
        harness.store.clone();
    let coordinator = Arc::new(
        crate::usecase::shutdown_coordinator::ShutdownCoordinator::new(
            repository,
            authority,
            executor.clone(),
            harness.store.installation_id().to_string(),
            "test-boot".to_string(),
        ),
    );
    let first_coordinator = Arc::clone(&coordinator);
    let first = tokio::spawn(async move {
        first_coordinator
            .request(ApplicationQuitRequest {
                principal: "desktop".to_string(),
                request_id: "b067-aborted-flight".to_string(),
                intent: ApplicationQuitIntent::Exit { code: 0 },
            })
            .await
    });
    for _ in 0..1_000 {
        if executor
            .target_queries
            .load(std::sync::atomic::Ordering::SeqCst)
            == 1
        {
            break;
        }
        tokio::task::yield_now().await;
    }
    tokio::time::advance(std::time::Duration::from_secs(13)).await;
    assert!(matches!(
        first.await.unwrap().unwrap(),
        ApplicationQuitOutcome::RejectedBeforeCommit { .. }
    ));
    // Resume wall-clock scheduling before starting the independent flight;
    // paused Tokio time would otherwise auto-advance while the SQLite writer
    // thread is preparing its reply and manufacture an acceptance timeout.
    tokio::time::resume();

    let second = coordinator
        .request(ApplicationQuitRequest {
            principal: "desktop".to_string(),
            request_id: "b067-new-flight".to_string(),
            intent: ApplicationQuitIntent::Restart { code: 7 },
        })
        .await
        .expect("new flight after the known pre-acceptance abort");
    let ApplicationQuitOutcome::Accepted {
        receipt,
        state: ApplicationQuitState::Completed,
    } = second
    else {
        panic!("new flight must complete: {second:?}");
    };
    let connection = harness.raw_connection();
    let durable_before: (i64, i64, i64, String) = connection
        .query_row(
            "SELECT
                (SELECT COUNT(*) FROM operation_records WHERE kind = 'application_quit'),
                (SELECT COUNT(*) FROM shutdown_plans),
                (SELECT COUNT(*) FROM shutdown_targets),
                (SELECT latest_status FROM operation_records
                  WHERE kind = 'application_quit' AND operation_id = ?1)",
            rusqlite::params![receipt.operation_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    let effects_before = executor.effects.load(std::sync::atomic::Ordering::SeqCst);
    let subordinate_before = executor
        .subordinate_shutdowns
        .load(std::sync::atomic::Ordering::SeqCst);

    executor.release_late_result.notify_waiters();
    for _ in 0..1_000 {
        if executor
            .late_results
            .load(std::sync::atomic::Ordering::SeqCst)
            == 1
        {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert_eq!(
        executor
            .late_results
            .load(std::sync::atomic::Ordering::SeqCst),
        1,
        "late fixture result did not arrive"
    );
    let durable_after: (i64, i64, i64, String) = connection
        .query_row(
            "SELECT
                (SELECT COUNT(*) FROM operation_records WHERE kind = 'application_quit'),
                (SELECT COUNT(*) FROM shutdown_plans),
                (SELECT COUNT(*) FROM shutdown_targets),
                (SELECT latest_status FROM operation_records
                  WHERE kind = 'application_quit' AND operation_id = ?1)",
            rusqlite::params![receipt.operation_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(durable_after, durable_before);
    assert_eq!(
        executor.effects.load(std::sync::atomic::Ordering::SeqCst),
        effects_before
    );
    assert_eq!(
        executor
            .subordinate_shutdowns
            .load(std::sync::atomic::Ordering::SeqCst),
        subordinate_before
    );
    assert!(coordinator.current_shutdown().await.unwrap().is_none());
}

#[tokio::test(start_paused = true)]
async fn hanging_authority_query_is_bounded_by_the_absolute_decision_deadline() {
    let harness = Harness::open();
    let executor = TestShutdownExecutor::with_targets(1, ShutdownExecutorMode::Complete);
    let hanging_repository = Arc::new(HangingShutdownQueryRepository {
        inner: Arc::clone(&harness.store),
        queries: std::sync::atomic::AtomicUsize::new(0),
    });
    let repository: Arc<dyn LocalEventTransactionRepository> = hanging_repository.clone();
    let authority: Arc<dyn crate::usecase::agent_session::operation::OperationBindingAuthority> =
        harness.store.clone();
    let coordinator = Arc::new(
        crate::usecase::shutdown_coordinator::ShutdownCoordinator::new(
            repository,
            authority,
            executor.clone(),
            harness.store.installation_id().to_string(),
            "test-boot".to_string(),
        ),
    );
    let task = tokio::spawn(async move {
        coordinator
            .request(
                crate::usecase::shutdown_coordinator::ApplicationQuitRequest {
                    principal: "desktop".to_string(),
                    request_id: "quit-hanging-authority-query".to_string(),
                    intent: crate::usecase::shutdown_coordinator::ApplicationQuitIntent::Exit {
                        code: 0,
                    },
                },
            )
            .await
    });
    for _ in 0..1_000 {
        if hanging_repository
            .queries
            .load(std::sync::atomic::Ordering::SeqCst)
            == 1
        {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert_eq!(
        hanging_repository
            .queries
            .load(std::sync::atomic::Ordering::SeqCst),
        1,
        "request did not reach the authority query"
    );

    tokio::time::advance(std::time::Duration::from_secs(15)).await;
    assert!(matches!(
        task.await.unwrap().unwrap(),
        crate::usecase::shutdown_coordinator::ApplicationQuitOutcome::OutcomeUnknown { .. }
    ));
    assert_eq!(
        executor
            .target_queries
            .load(std::sync::atomic::Ordering::SeqCst),
        0
    );
    assert_eq!(
        executor.effects.load(std::sync::atomic::Ordering::SeqCst),
        0
    );
    assert!(matches!(
        harness
            .store
            .query(LocalEventQuery::CurrentShutdown)
            .await
            .unwrap(),
        LocalEventQueryResult::CurrentShutdown(None)
    ));
}

#[tokio::test]
// The production cutoff is part of this capacity oracle, so isolate it from
// the >16 MiB migration fixtures that the Rust test harness runs in parallel.
#[allow(clippy::await_holding_lock)]
async fn b060_exactly_4096_shutdown_targets_are_durably_accepted_as_one_plan() {
    let _heavy_test_lock = crate::test_support::LOCAL_EVENT_STORE_HEAVY_TEST_LOCK.lock();
    let harness = Harness::open();
    let executor = TestShutdownExecutor::with_targets(4096, ShutdownExecutorMode::Complete);
    let coordinator = shutdown_coordinator(&harness, &executor);
    let outcome = coordinator
        .request(
            crate::usecase::shutdown_coordinator::ApplicationQuitRequest {
                principal: "desktop".to_string(),
                request_id: "quit-capacity-exact".to_string(),
                intent: crate::usecase::shutdown_coordinator::ApplicationQuitIntent::Exit {
                    code: 0,
                },
            },
        )
        .await
        .unwrap();
    let crate::usecase::shutdown_coordinator::ApplicationQuitOutcome::Accepted {
        receipt,
        state: crate::usecase::shutdown_coordinator::ApplicationQuitState::Completed,
    } = outcome
    else {
        panic!("the exact 4,096-target boundary must complete: {outcome:?}");
    };
    assert_eq!(
        executor.effects.load(std::sync::atomic::Ordering::SeqCst),
        4096
    );
    assert_eq!(
        executor
            .subordinate_shutdowns
            .load(std::sync::atomic::Ordering::SeqCst),
        1
    );
    let connection = harness.raw_connection();
    let summary: String = connection
        .query_row(
            "SELECT summary FROM shutdown_plans WHERE shutdown_id = ?1",
            rusqlite::params![receipt.shutdown_id],
            |row| row.get(0),
        )
        .unwrap();
    let summary: serde_json::Value = serde_json::from_str(&summary).unwrap();
    assert_eq!(
        summary
            .get("target_count")
            .and_then(serde_json::Value::as_u64),
        Some(4096)
    );
    let stored_target_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM shutdown_targets", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(stored_target_count, 4096);
    let stored_phase: String = connection
        .query_row("SELECT phase FROM shutdown_plans", [], |row| row.get(0))
        .unwrap();
    assert_eq!(stored_phase, "completed");
}

#[tokio::test]
async fn b060_non_target_recovery_is_excluded_from_targets_and_retained_in_exit_summary() {
    let harness = Harness::open();
    let pending = |obligation_id: &str, partition: PendingPartition| {
        let LocalStateMutation::Obligation(mut mutation) = obligation_mutation(obligation_id, true)
        else {
            unreachable!("obligation helper always returns an obligation mutation");
        };
        let entry = mutation.pending.as_mut().expect("pending entry");
        entry.owner = obligation_id.to_string();
        entry.partition = partition;
        LocalStateMutation::Obligation(mutation)
    };
    let mut recovery_seed = batch(
        "b060-recovery-summary-seed",
        "b060-recovery-summary-seed",
        [60; 32],
        vec![],
        vec![],
        vec![
            pending("b060-closed-recovery", PendingPartition::ClosedSession),
            pending("b060-archived-recovery", PendingPartition::ArchivedSession),
            pending("b060-unowned-runtime", PendingPartition::UnownedRuntime),
        ],
    );
    recovery_seed.idempotency.installation_id = harness.store.installation_id().to_string();
    harness
        .store
        .commit_batch(recovery_seed)
        .await
        .expect("seed non-target recovery inventory");

    let executor = TestShutdownExecutor::with_targets(2, ShutdownExecutorMode::Complete);
    let coordinator = shutdown_coordinator(&harness, &executor);
    let outcome = coordinator
        .request(
            crate::usecase::shutdown_coordinator::ApplicationQuitRequest {
                principal: "desktop".to_string(),
                request_id: "b060-recovery-summary".to_string(),
                intent: crate::usecase::shutdown_coordinator::ApplicationQuitIntent::Exit {
                    code: 0,
                },
            },
        )
        .await
        .unwrap();
    let crate::usecase::shutdown_coordinator::ApplicationQuitOutcome::Accepted {
        receipt,
        state: crate::usecase::shutdown_coordinator::ApplicationQuitState::Completed,
    } = outcome
    else {
        panic!("bounded shutdown must complete: {outcome:?}");
    };

    let connection = harness.raw_connection();
    let summary: String = connection
        .query_row(
            "SELECT summary FROM shutdown_plans WHERE shutdown_id = ?1",
            rusqlite::params![receipt.shutdown_id],
            |row| row.get(0),
        )
        .unwrap();
    let summary: serde_json::Value = serde_json::from_str(&summary).unwrap();
    assert_eq!(summary["target_count"], 2);
    assert_eq!(summary["recovery_snapshot_count"], 3);
    assert_eq!(summary["completed_count"], 2);
    let mut statement = connection
        .prepare(
            "SELECT partition FROM shutdown_recovery_snapshots
             WHERE shutdown_id = ?1 ORDER BY partition",
        )
        .unwrap();
    let partitions = statement
        .query_map(rusqlite::params![receipt.shutdown_id], |row| {
            row.get::<_, String>(0)
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        partitions,
        vec!["archived_session", "closed_session", "unowned_runtime"]
    );
    assert_eq!(
        executor.effects.load(std::sync::atomic::Ordering::SeqCst),
        2
    );
    assert_eq!(
        executor
            .subordinate_shutdowns
            .load(std::sync::atomic::Ordering::SeqCst),
        1
    );
}

#[tokio::test]
async fn b060_4097_shutdown_targets_and_acceptance_failure_start_zero_effects() {
    let oversized = Harness::open();
    let oversized_executor =
        TestShutdownExecutor::with_targets(4097, ShutdownExecutorMode::Complete);
    let oversized_coordinator = shutdown_coordinator(&oversized, &oversized_executor);
    assert_eq!(
        oversized_coordinator
            .request(
                crate::usecase::shutdown_coordinator::ApplicationQuitRequest {
                    principal: "desktop".to_string(),
                    request_id: "quit-capacity".to_string(),
                    intent: crate::usecase::shutdown_coordinator::ApplicationQuitIntent::Exit {
                        code: 0,
                    },
                },
            )
            .await,
        Err(crate::usecase::shutdown_coordinator::ApplicationQuitError::CapacityExceeded)
    );
    assert_eq!(
        oversized_executor
            .effects
            .load(std::sync::atomic::Ordering::SeqCst),
        0
    );
    assert!(oversized_coordinator
        .current_shutdown()
        .await
        .unwrap()
        .is_none());

    let failed = Harness::open();
    let failed_executor = TestShutdownExecutor::with_targets(1, ShutdownExecutorMode::Complete);
    let failed_coordinator = shutdown_coordinator(&failed, &failed_executor);
    failed.fault.arm_fail_before_begin();
    let outcome = failed_coordinator
        .request(
            crate::usecase::shutdown_coordinator::ApplicationQuitRequest {
                principal: "desktop".to_string(),
                request_id: "quit-storage".to_string(),
                intent: crate::usecase::shutdown_coordinator::ApplicationQuitIntent::Exit {
                    code: 0,
                },
            },
        )
        .await
        .unwrap();
    assert!(matches!(
        outcome,
        crate::usecase::shutdown_coordinator::ApplicationQuitOutcome::RejectedBeforeCommit { .. }
    ));
    assert_eq!(
        failed_executor
            .effects
            .load(std::sync::atomic::Ordering::SeqCst),
        0
    );
    assert!(failed_coordinator
        .current_shutdown()
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn b062_public_shutdown_page_enforces_encoded_one_mib_without_partial_result() {
    let harness = Harness::open();
    let executor = Arc::new(TestShutdownExecutor {
        targets: vec![crate::usecase::shutdown_coordinator::ShutdownTarget {
            target_id: "x".repeat(1_048_000),
            kind: "agent_session".to_string(),
        }],
        mode: ShutdownExecutorMode::Complete,
        target_queries: std::sync::atomic::AtomicUsize::new(0),
        effects: std::sync::atomic::AtomicUsize::new(0),
        readbacks: std::sync::atomic::AtomicUsize::new(0),
        subordinate_shutdowns: std::sync::atomic::AtomicUsize::new(0),
        drop_reply_on_readback: None,
    });
    let coordinator = shutdown_coordinator(&harness, &executor);
    let outcome = coordinator
        .request(
            crate::usecase::shutdown_coordinator::ApplicationQuitRequest {
                principal: "desktop".to_string(),
                request_id: "quit-encoded-page-boundary".to_string(),
                intent: crate::usecase::shutdown_coordinator::ApplicationQuitIntent::Exit {
                    code: 0,
                },
            },
        )
        .await
        .expect("quit request");
    let crate::usecase::shutdown_coordinator::ApplicationQuitOutcome::Accepted { receipt, .. } =
        outcome
    else {
        panic!("quit must be accepted");
    };
    let page = coordinator
        .shutdown_plan_page_read_model(
            ShutdownPlanKey {
                shutdown_id: receipt.shutdown_id,
            },
            1,
            None,
        )
        .await
        .expect("raw indexed page remains within its storage bound");
    let encoded = serde_json::to_vec(
        &crate::adaptor::presenter::application_lifecycle::plan_page(page.clone()),
    )
    .expect("encode public page");
    assert!(
        encoded.len() > 1024 * 1024,
        "fixture must cross the public encoded-byte boundary: {}",
        encoded.len()
    );
    assert!(matches!(
        crate::adaptor::presenter::application_lifecycle::checked_plan_page(page),
        Err(LocalEventQueryError::ResponseTooLarge)
    ));
    assert_eq!(
        executor.effects.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "read-side rejection must not repeat the already completed effect"
    );
}

#[tokio::test]
// Paging is exercised after a real 4,096-target flight; unrelated migration
// I/O must not consume that flight's fixed production deadline.
#[allow(clippy::await_holding_lock)]
async fn b062_shutdown_plan_page_closes_count_limit_unknown_and_cursor_boundaries() {
    let _heavy_test_lock = crate::test_support::LOCAL_EVENT_STORE_HEAVY_TEST_LOCK.lock();
    let harness = Harness::open();
    let executor = TestShutdownExecutor::with_targets(4096, ShutdownExecutorMode::Complete);
    // The subject here is paging, not the deadline: b060 owns the capacity
    // oracle at the production cutoff. Reserving 4,096 targets takes seconds on
    // a slow runner, so keep this flight's budget clear of that boundary
    // instead of racing the machine.
    let coordinator = Arc::new(
        crate::usecase::shutdown_coordinator::ShutdownCoordinator::new(
            harness.store.clone(),
            harness.store.clone(),
            executor.clone(),
            harness.store.installation_id().to_string(),
            "test-boot".to_string(),
        )
        .with_flight_budget_for_test(
            std::time::Duration::from_secs(600),
            std::time::Duration::from_secs(660),
        ),
    );
    let outcome = coordinator
        .request(
            crate::usecase::shutdown_coordinator::ApplicationQuitRequest {
                principal: "desktop".to_string(),
                request_id: "b062-page-cursor-boundaries".to_string(),
                intent: crate::usecase::shutdown_coordinator::ApplicationQuitIntent::Exit {
                    code: 0,
                },
            },
        )
        .await
        .expect("4,096-target shutdown fixture");
    let crate::usecase::shutdown_coordinator::ApplicationQuitOutcome::Accepted {
        receipt,
        state: crate::usecase::shutdown_coordinator::ApplicationQuitState::Completed,
    } = outcome
    else {
        panic!("4,096-target shutdown fixture must complete: {outcome:?}");
    };
    let plan = ShutdownPlanKey {
        shutdown_id: receipt.shutdown_id,
    };

    let first = coordinator
        .shutdown_plan_page(plan.clone(), 1, None)
        .await
        .expect("limit 1 page");
    assert_eq!(first.targets.len(), 1);
    assert_eq!(first.targets[0].ordinal, 0);
    let first_cursor = first.next_cursor.expect("limit 1 next cursor");

    let second = coordinator
        .shutdown_plan_page(plan.clone(), 1, Some(first_cursor.as_str().to_string()))
        .await
        .expect("valid cursor page");
    assert_eq!(second.targets.len(), 1);
    assert_eq!(second.targets[0].ordinal, 1);
    assert!(second.next_cursor.is_some());

    let maximum = coordinator
        .shutdown_plan_page(plan.clone(), 128, None)
        .await
        .expect("limit 128 page");
    assert_eq!(maximum.targets.len(), 128);
    assert_eq!(maximum.targets[0].ordinal, 0);
    assert_eq!(maximum.targets[127].ordinal, 127);
    assert!(maximum.next_cursor.is_some());

    assert!(matches!(
        coordinator
            .shutdown_plan_page(plan.clone(), 129, None)
            .await,
        Err(LocalEventQueryError::InvalidRequest)
    ));
    assert!(matches!(
        coordinator
            .shutdown_plan_page(
                ShutdownPlanKey {
                    shutdown_id: "b062-unknown-plan".to_string(),
                },
                128,
                None,
            )
            .await,
        Err(LocalEventQueryError::NotFound)
    ));

    let mut tampered = first_cursor.as_str().to_string();
    let first_byte = tampered.remove(0);
    tampered.insert(0, if first_byte == 'A' { 'B' } else { 'A' });
    assert!(matches!(
        coordinator
            .shutdown_plan_page(plan.clone(), 1, Some(tampered))
            .await,
        Err(LocalEventQueryError::CursorMismatch)
    ));

    harness.clock.advance_ms(10 * 60 * 1_000);
    assert!(matches!(
        coordinator
            .shutdown_plan_page(plan, 1, Some(first_cursor.as_str().to_string()))
            .await,
        Err(LocalEventQueryError::CursorExpired)
    ));
}

#[tokio::test]
async fn b062_terminal_counts_fail_closed_while_unavailable_preparing_counts_remain_none() {
    use crate::usecase::shutdown_coordinator::{
        ApplicationQuitIntent, ApplicationQuitOutcome, ApplicationQuitRequest,
    };

    let harness = Harness::open();
    let executor = TestShutdownExecutor::with_targets(0, ShutdownExecutorMode::Complete);
    let coordinator = shutdown_coordinator(&harness, &executor);
    let outcome = coordinator
        .request(ApplicationQuitRequest {
            principal: "desktop".to_string(),
            request_id: "b062-terminal-counts".to_string(),
            intent: ApplicationQuitIntent::Exit { code: 0 },
        })
        .await
        .expect("completed terminal fixture");
    let ApplicationQuitOutcome::Accepted { receipt, .. } = outcome else {
        panic!("terminal count fixture must be accepted");
    };
    let terminal_plan = ShutdownPlanKey {
        shutdown_id: receipt.shutdown_id,
    };
    let connection = harness.raw_connection();
    let original_summary: String = connection
        .query_row(
            "SELECT summary FROM shutdown_plans WHERE shutdown_id = ?1",
            rusqlite::params![terminal_plan.shutdown_id],
            |row| row.get(0),
        )
        .unwrap();
    let original: serde_json::Value = serde_json::from_str(&original_summary).unwrap();
    for (case, replacement) in [
        ("missing", None),
        ("string", Some(serde_json::json!("0"))),
        ("negative", Some(serde_json::json!(-1))),
        (
            "overflow",
            Some(serde_json::json!(9_223_372_036_854_775_808u64)),
        ),
    ] {
        let mut malformed = original.clone();
        match replacement {
            Some(value) => malformed["target_count"] = value,
            None => {
                malformed
                    .as_object_mut()
                    .expect("summary object")
                    .remove("target_count");
            }
        }
        connection
            .execute(
                "UPDATE shutdown_plans SET summary = ?1 WHERE shutdown_id = ?2",
                rusqlite::params![malformed.to_string(), terminal_plan.shutdown_id],
            )
            .unwrap();
        let error = coordinator
            .shutdown_plan_page_read_model(terminal_plan.clone(), 128, None)
            .await
            .expect_err("terminal count must fail closed");
        if case == "missing" {
            assert!(matches!(error, LocalEventQueryError::Internal { .. }));
        } else {
            assert!(matches!(
                error,
                LocalEventQueryError::IncompatibleStoredEvent { .. }
            ));
        }
        connection
            .execute(
                "UPDATE shutdown_plans SET summary = ?1 WHERE shutdown_id = ?2",
                rusqlite::params![original_summary, terminal_plan.shutdown_id],
            )
            .unwrap();
    }

    let preparing_plan = ShutdownPlanKey {
        shutdown_id: "b062-preparing-counts-unavailable".to_string(),
    };
    let mut seed = batch(
        "b062-preparing-counts-unavailable",
        "b062-preparing-counts-unavailable",
        [62; 32],
        Vec::new(),
        Vec::new(),
        vec![LocalStateMutation::ShutdownPlan(ShutdownPlanMutation {
            key: preparing_plan.clone(),
            phase: ApplicationShutdownPhase::Prepared,
            summary: payload(
                &serde_json::json!({
                    "schema": "shutdown_plan_summary_v1",
                    "operation_id": "b062-preparing-counts-unavailable",
                    "intent": "exit",
                    "exit_code": 0,
                    "t0_ms": 1_000,
                    "preparation_cutoff_ms": 14_000,
                    "deadline_ms": 16_000,
                    "process_instance_id": "test-boot",
                })
                .to_string(),
            ),
            details_state: ShutdownDetailsState::Available,
            expected: RevisionGuard::Absent,
            revision: Revision::new(0).unwrap(),
        })],
    );
    seed.idempotency.operation_kind = CommitOperationKind::ApplicationQuit;
    harness.store.commit_batch(seed).await.unwrap();
    let preparing = coordinator
        .shutdown_plan_page_read_model(preparing_plan, 128, None)
        .await
        .expect("preparing counts may be unavailable");
    assert_eq!(preparing.plan.target_count, None);
    assert_eq!(preparing.plan.prepared_count, None);
    assert_eq!(preparing.plan.unresolved_count, None);
}

async fn b076_seed_current_shutdown(
    harness: &Harness,
    suffix: &str,
    phase: ApplicationShutdownPhase,
    operation_state: &str,
    process_instance_id: &str,
) -> (ShutdownPlanKey, String) {
    let operation_id = format!("b076-operation-{suffix}");
    let plan = ShutdownPlanKey {
        shutdown_id: format!("b076-plan-{suffix}"),
    };
    let binding = [76; 32];
    let failure = SafeOperationFailure::new(
        SessionOperationFailureKind::DeadlineExceeded,
        true,
        "B076 shutdown reconciliation is required.",
        format!("b076-{suffix}-failure"),
    );
    let status = match operation_state {
        "failed_before_activation" | "reconciliation_required" => serde_json::json!({
            "schema": "application_quit_status_v1",
            "state": {
                "type": operation_state,
                "failure": {
                    "kind": "DeadlineExceeded",
                    "retryable": true,
                    "message": "B076 shutdown reconciliation is required.",
                    "correlation_id": format!("b076-{suffix}-failure"),
                }
            }
        }),
        _ => serde_json::json!({
            "schema": "application_quit_status_v1",
            "state": { "type": operation_state }
        }),
    };
    let mut summary = serde_json::json!({
        "schema": "shutdown_plan_summary_v1",
        "operation_id": operation_id,
        "intent": "exit",
        "exit_code": 76,
        "t0_ms": 1_000,
        "preparation_cutoff_ms": 14_000,
        "deadline_ms": 16_000,
        "target_count": 0,
        "prepared_count": 0,
        "effect_reserved_count": 0,
        "terminal_count": 0,
        "completed_count": 0,
        "unresolved_count": 0,
        "recovery_snapshot_count": 0,
        "recovery_snapshot_id": null,
        "outcome": match phase {
            ApplicationShutdownPhase::Completed => "completed",
            ApplicationShutdownPhase::Failed | ApplicationShutdownPhase::Cancelled => {
                "aborted_before_activation"
            }
            ApplicationShutdownPhase::ReconciliationRequired => "exited_with_recovery",
            _ => "in_progress",
        },
        "process_instance_id": process_instance_id,
        "shutdown_effect_count": 0,
        "admission_open": false,
        "retry_quit_same_boot": false,
    });
    if matches!(
        phase,
        ApplicationShutdownPhase::Failed
            | ApplicationShutdownPhase::Cancelled
            | ApplicationShutdownPhase::ReconciliationRequired
    ) {
        summary["failure"] = serde_json::json!({
            "kind": "deadline_exceeded",
            "retryable": failure.retryable,
            "label": failure.label.value(),
            "detail": failure.detail.as_ref().map(|detail| detail.value()),
            "correlation_id": failure.correlation_id,
        });
    }
    let receipt = serde_json::json!({
        "schema": "application_quit_receipt_v1",
        "operation_id": operation_id,
        "shutdown_id": plan.shutdown_id,
        "intent": "exit",
        "exit_code": 76,
        "t0_ms": 1_000,
        "deadline_ms": 16_000,
        "binding_hmac": hex::encode(binding),
    });
    let mut seed = batch(
        &format!("b076-seed-{suffix}"),
        &format!("b076-seed-{suffix}"),
        [76; 32],
        Vec::new(),
        Vec::new(),
        vec![
            LocalStateMutation::OperationBinding(OperationBindingMutation {
                key: CallerOperationKey {
                    principal: "desktop".to_string(),
                    installation_id: harness.store.installation_id().to_string(),
                    kind: OperationKind::ApplicationQuit,
                    caller_request_id: format!("b076-request-{suffix}"),
                },
                operation_id: operation_id.clone(),
                binding_hmac: binding,
            }),
            LocalStateMutation::OperationRecord(OperationRecordMutation {
                kind: OperationKind::ApplicationQuit,
                operation_id: operation_id.clone(),
                receipt: payload(&receipt.to_string()),
                latest_status: payload(&status.to_string()),
                expected: RevisionGuard::Absent,
                revision: Revision::new(0).unwrap(),
            }),
            LocalStateMutation::ShutdownPlan(ShutdownPlanMutation {
                key: plan.clone(),
                phase,
                summary: payload(&summary.to_string()),
                details_state: ShutdownDetailsState::Available,
                expected: RevisionGuard::Absent,
                revision: Revision::new(0).unwrap(),
            }),
            LocalStateMutation::ShutdownLatestPointer(ShutdownLatestPointerMutation {
                expected: None,
                new: Some(plan.clone()),
            }),
        ],
    );
    seed.idempotency.installation_id = harness.store.installation_id().to_string();
    seed.idempotency.operation_kind = CommitOperationKind::ApplicationQuit;
    harness
        .store
        .commit_batch(seed)
        .await
        .expect("seed B076 current shutdown authority");
    (plan, operation_id)
}

fn b076_coordinator(
    harness: &Harness,
    executor: &Arc<TestShutdownExecutor>,
    process_instance_id: &str,
) -> Arc<crate::usecase::shutdown_coordinator::ShutdownCoordinator> {
    Arc::new(
        crate::usecase::shutdown_coordinator::ShutdownCoordinator::new(
            harness.store.clone(),
            harness.store.clone(),
            executor.clone(),
            harness.store.installation_id().to_string(),
            process_instance_id.to_string(),
        ),
    )
}

#[tokio::test]
async fn b076_same_boot_all_phases_and_previous_boot_nonterminal_keep_the_exact_plan_identity() {
    use crate::usecase::shutdown_coordinator::CurrentApplicationShutdownProjection;

    let cases = [
        (ApplicationShutdownPhase::Prepared, "preparing"),
        (ApplicationShutdownPhase::Activated, "activated"),
        (ApplicationShutdownPhase::Quiescing, "activated"),
        (ApplicationShutdownPhase::Completed, "completed"),
        (ApplicationShutdownPhase::Failed, "failed_before_activation"),
        (
            ApplicationShutdownPhase::Cancelled,
            "failed_before_activation",
        ),
        (
            ApplicationShutdownPhase::ReconciliationRequired,
            "reconciliation_required",
        ),
    ];
    for (index, (phase, operation_state)) in cases.into_iter().enumerate() {
        let harness = Harness::open();
        let process_instance_id = harness.store.process_instance_id().to_string();
        let (plan, operation_id) = b076_seed_current_shutdown(
            &harness,
            &format!("phase-{index}"),
            phase,
            operation_state,
            &process_instance_id,
        )
        .await;
        let executor = TestShutdownExecutor::with_targets(0, ShutdownExecutorMode::Complete);
        let coordinator = b076_coordinator(&harness, &executor, &process_instance_id);
        let CurrentApplicationShutdownProjection::Current(Some(current)) = coordinator
            .current_application_shutdown_projection()
            .await
            .expect("same-boot current shutdown")
        else {
            panic!("same-boot phase must remain current: {phase:?}");
        };
        assert_eq!(current.plan, plan);
        assert_eq!(current.operation_id, operation_id);
        assert_eq!(current.phase, phase);
        assert_eq!(
            executor.effects.load(std::sync::atomic::Ordering::SeqCst),
            0
        );
        assert_eq!(
            executor
                .subordinate_shutdowns
                .load(std::sync::atomic::Ordering::SeqCst),
            0
        );
    }

    let harness = Harness::open();
    let current_boot = harness.store.process_instance_id().to_string();
    let (plan, operation_id) = b076_seed_current_shutdown(
        &harness,
        "previous-nonterminal",
        ApplicationShutdownPhase::Quiescing,
        "activated",
        "b076-previous-boot",
    )
    .await;
    let executor = TestShutdownExecutor::with_targets(0, ShutdownExecutorMode::Complete);
    let coordinator = b076_coordinator(&harness, &executor, &current_boot);
    let CurrentApplicationShutdownProjection::Current(Some(current)) = coordinator
        .current_application_shutdown_projection()
        .await
        .expect("previous-boot nonterminal current shutdown")
    else {
        panic!("previous-boot nonterminal plan must reconcile");
    };
    assert_eq!(current.plan, plan);
    assert_eq!(current.operation_id, operation_id);
    assert_eq!(
        current.phase,
        ApplicationShutdownPhase::ReconciliationRequired
    );
    assert!(current.safe_failure.is_some());
    assert_eq!(
        executor.effects.load(std::sync::atomic::Ordering::SeqCst),
        0
    );
}

#[tokio::test]
async fn b076_same_boot_completed_is_current_but_previous_boot_terminal_is_history_only() {
    use crate::usecase::shutdown_coordinator::{
        ApplicationQuitIntent, ApplicationQuitOutcome, ApplicationQuitRequest,
        CurrentApplicationShutdownProjection,
    };

    let harness = Harness::open();
    let process_instance_id = harness.store.process_instance_id().to_string();
    let executor = TestShutdownExecutor::with_targets(0, ShutdownExecutorMode::Complete);
    let coordinator = b076_coordinator(&harness, &executor, &process_instance_id);
    let ApplicationQuitOutcome::Accepted { receipt, .. } = coordinator
        .request(ApplicationQuitRequest {
            principal: "desktop".to_string(),
            request_id: "b076-same-boot-completed".to_string(),
            intent: ApplicationQuitIntent::Exit { code: 76 },
        })
        .await
        .expect("same-boot completed quit")
    else {
        panic!("quit must complete");
    };
    let CurrentApplicationShutdownProjection::Current(Some(current)) = coordinator
        .current_application_shutdown_projection()
        .await
        .expect("same-boot terminal current query")
    else {
        panic!("same-boot completed flight must be current");
    };
    assert_eq!(current.plan.shutdown_id, receipt.shutdown_id);
    assert_eq!(current.phase, ApplicationShutdownPhase::Completed);

    let restarted = b076_coordinator(&harness, &executor, "b076-next-boot");
    assert_eq!(
        restarted
            .current_application_shutdown_projection()
            .await
            .expect("previous-boot terminal current query"),
        CurrentApplicationShutdownProjection::Current(None)
    );
    let history = restarted
        .shutdown_plan_page_read_model(
            ShutdownPlanKey {
                shutdown_id: receipt.shutdown_id.clone(),
            },
            128,
            None,
        )
        .await
        .expect("exact previous-boot terminal history");
    assert_eq!(history.plan.plan.shutdown_id, receipt.shutdown_id);
    assert_eq!(history.plan.phase, ApplicationShutdownPhase::Completed);
    assert_eq!(
        executor.effects.load(std::sync::atomic::Ordering::SeqCst),
        0
    );
    assert_eq!(
        executor
            .subordinate_shutdowns
            .load(std::sync::atomic::Ordering::SeqCst),
        1
    );
}

#[tokio::test]
async fn b076_redundant_authority_mismatch_reconciles_the_same_plan_without_effects() {
    use crate::usecase::shutdown_coordinator::CurrentApplicationShutdownProjection;

    let harness = Harness::open();
    let process_instance_id = harness.store.process_instance_id().to_string();
    let (plan, operation_id) = b076_seed_current_shutdown(
        &harness,
        "authority-mismatch",
        ApplicationShutdownPhase::Activated,
        "preparing",
        &process_instance_id,
    )
    .await;
    let executor = TestShutdownExecutor::with_targets(0, ShutdownExecutorMode::Complete);
    let coordinator = b076_coordinator(&harness, &executor, &process_instance_id);
    let CurrentApplicationShutdownProjection::Current(Some(current)) = coordinator
        .current_application_shutdown_projection()
        .await
        .expect("redundant authority mismatch is a safe projection")
    else {
        panic!("authority mismatch must retain the same plan");
    };
    assert_eq!(current.plan, plan);
    assert_eq!(current.operation_id, operation_id);
    assert_eq!(
        current.phase,
        ApplicationShutdownPhase::ReconciliationRequired
    );
    assert_eq!(
        current.safe_failure.as_ref().map(|failure| failure.kind),
        Some(SessionOperationFailureKind::ShutdownAuthorityMismatch)
    );
    assert!(current.actions.is_empty());
    assert_eq!(
        executor.effects.load(std::sync::atomic::Ordering::SeqCst),
        0
    );
    assert_eq!(
        executor
            .subordinate_shutdowns
            .load(std::sync::atomic::Ordering::SeqCst),
        0
    );
}

#[tokio::test]
async fn b076_storage_decode_integrity_reference_and_identity_failures_are_internal() {
    for case in [
        "summary-decode",
        "status-decode",
        "binding-integrity",
        "required-operation",
        "receipt-reference",
        "identity-uniqueness",
    ] {
        let harness = Harness::open();
        let process_instance_id = harness.store.process_instance_id().to_string();
        let (plan, operation_id) = b076_seed_current_shutdown(
            &harness,
            case,
            ApplicationShutdownPhase::Prepared,
            "preparing",
            &process_instance_id,
        )
        .await;
        let connection = harness.raw_connection();
        match case {
            "summary-decode" => {
                connection
                    .execute(
                        "UPDATE shutdown_plans SET summary = 'not-json'
                         WHERE shutdown_id = ?1",
                        rusqlite::params![plan.shutdown_id],
                    )
                    .unwrap();
            }
            "status-decode" => {
                connection
                    .execute(
                        "UPDATE operation_records SET latest_status = 'not-json'
                         WHERE kind = 'application_quit' AND operation_id = ?1",
                        rusqlite::params![operation_id],
                    )
                    .unwrap();
            }
            "binding-integrity" => {
                connection
                    .execute(
                        "UPDATE operation_bindings SET binding_hmac = zeroblob(32)
                         WHERE kind = 'application_quit' AND operation_id = ?1",
                        rusqlite::params![operation_id],
                    )
                    .unwrap();
            }
            "required-operation" => {
                connection
                    .execute(
                        "DELETE FROM operation_records
                         WHERE kind = 'application_quit' AND operation_id = ?1",
                        rusqlite::params![operation_id],
                    )
                    .unwrap();
            }
            "receipt-reference" => {
                let receipt: String = connection
                    .query_row(
                        "SELECT receipt FROM operation_records
                         WHERE kind = 'application_quit' AND operation_id = ?1",
                        rusqlite::params![operation_id],
                        |row| row.get(0),
                    )
                    .unwrap();
                let mut receipt: serde_json::Value = serde_json::from_str(&receipt).unwrap();
                receipt["shutdown_id"] = serde_json::Value::String("different-plan".to_string());
                connection
                    .execute(
                        "UPDATE operation_records SET receipt = ?1
                         WHERE kind = 'application_quit' AND operation_id = ?2",
                        rusqlite::params![receipt.to_string(), operation_id],
                    )
                    .unwrap();
            }
            "identity-uniqueness" => {
                let binding: Vec<u8> = connection
                    .query_row(
                        "SELECT binding_hmac FROM operation_bindings
                         WHERE kind = 'application_quit' AND operation_id = ?1",
                        rusqlite::params![operation_id],
                        |row| row.get(0),
                    )
                    .unwrap();
                let commit_id: String = connection
                    .query_row(
                        "SELECT commit_id FROM operation_bindings
                         WHERE kind = 'application_quit' AND operation_id = ?1",
                        rusqlite::params![operation_id],
                        |row| row.get(0),
                    )
                    .unwrap();
                connection
                    .execute(
                        "INSERT INTO operation_bindings
                         (principal, installation_id, kind, caller_request_id,
                          operation_id, binding_hmac, commit_id)
                         VALUES ('desktop-duplicate', ?1, 'application_quit',
                                 'b076-duplicate-request', ?2, ?3, ?4)",
                        rusqlite::params![
                            harness.store.installation_id(),
                            operation_id,
                            binding,
                            commit_id
                        ],
                    )
                    .unwrap();
            }
            _ => unreachable!(),
        }
        let executor = TestShutdownExecutor::with_targets(0, ShutdownExecutorMode::Complete);
        let coordinator = b076_coordinator(&harness, &executor, &process_instance_id);
        let error = coordinator
            .current_application_shutdown_projection()
            .await
            .expect_err("current authority failure must fail closed");
        assert!(matches!(
            crate::adaptor::presenter::application_lifecycle::current_shutdown_error(error),
            crate::adaptor::protocol::agent_session_v1::CurrentShutdownErrorDtoV1::Internal { .. }
        ));
        assert_eq!(
            executor.effects.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "{case}"
        );
        assert_eq!(
            executor
                .subordinate_shutdowns
                .load(std::sync::atomic::Ordering::SeqCst),
            0,
            "{case}"
        );
    }

    let harness = Harness::open();
    let executor = TestShutdownExecutor::with_targets(0, ShutdownExecutorMode::Complete);
    let repository: Arc<dyn LocalEventTransactionRepository> =
        Arc::new(CurrentShutdownUnavailableRepository {
            inner: harness.store.clone(),
        });
    let authority: Arc<dyn crate::usecase::agent_session::operation::OperationBindingAuthority> =
        harness.store.clone();
    let coordinator = crate::usecase::shutdown_coordinator::ShutdownCoordinator::new(
        repository,
        authority,
        executor.clone(),
        harness.store.installation_id().to_string(),
        harness.store.process_instance_id().to_string(),
    );
    let storage = coordinator
        .current_application_shutdown_projection()
        .await
        .expect_err("storage failure must not become Current(None)");
    assert!(matches!(
        crate::adaptor::presenter::application_lifecycle::current_shutdown_error(storage),
        crate::adaptor::protocol::agent_session_v1::CurrentShutdownErrorDtoV1::Internal { .. }
    ));
    assert_eq!(
        executor.effects.load(std::sync::atomic::Ordering::SeqCst),
        0
    );
}

#[tokio::test]
async fn b076_current_shutdown_missing_required_operation_reference_is_internal() {
    let harness = Harness::open();
    let plan = ShutdownPlanKey {
        shutdown_id: "b076-missing-operation".to_string(),
    };
    let mut seed = batch(
        "b076-missing-operation",
        "b076-missing-operation",
        [76; 32],
        Vec::new(),
        Vec::new(),
        vec![
            LocalStateMutation::ShutdownPlan(ShutdownPlanMutation {
                key: plan.clone(),
                phase: ApplicationShutdownPhase::Prepared,
                summary: payload(
                    &serde_json::json!({
                        "schema": "shutdown_plan_summary_v1",
                        "operation_id": "b076-missing-operation",
                        "intent": "exit",
                        "exit_code": 0,
                        "t0_ms": 1_000,
                        "preparation_cutoff_ms": 14_000,
                        "deadline_ms": 16_000,
                        "process_instance_id": "test-boot",
                    })
                    .to_string(),
                ),
                details_state: ShutdownDetailsState::Available,
                expected: RevisionGuard::Absent,
                revision: Revision::new(0).unwrap(),
            }),
            LocalStateMutation::ShutdownLatestPointer(ShutdownLatestPointerMutation {
                expected: None,
                new: Some(plan),
            }),
        ],
    );
    seed.idempotency.operation_kind = CommitOperationKind::ApplicationQuit;
    harness.store.commit_batch(seed).await.unwrap();
    let executor = TestShutdownExecutor::with_targets(0, ShutdownExecutorMode::Complete);
    let coordinator = shutdown_coordinator(&harness, &executor);
    let error = coordinator
        .current_application_shutdown_projection()
        .await
        .expect_err("required operation reference must fail closed");
    assert!(matches!(error, LocalEventQueryError::Corrupt { .. }));
    assert!(matches!(
        crate::adaptor::presenter::application_lifecycle::current_shutdown_error(error),
        crate::adaptor::protocol::agent_session_v1::CurrentShutdownErrorDtoV1::Internal { .. }
    ));
    assert_eq!(
        executor.effects.load(std::sync::atomic::Ordering::SeqCst),
        0
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn b085_b093_shutdown_action_replays_saved_result_after_compaction_restart_and_resolver_failure(
) {
    use crate::domain::agent_session::events::{RecoveryActionKind, RecoveryResultClassification};
    use crate::usecase::agent_session::operation::{RecoveryActionOutcome, RecoveryActionStatus};
    use crate::usecase::shutdown_coordinator::{
        ApplicationQuitIntent, ApplicationQuitOutcome, ApplicationQuitRequest,
        ApplicationQuitState, ShutdownTargetActionRequest,
    };

    let harness = Harness::open();
    let executor =
        TestShutdownExecutor::fail_then_read_completed(1, Some(Arc::clone(&harness.fault)));
    let coordinator = shutdown_coordinator(&harness, &executor);
    let outcome = coordinator
        .request(ApplicationQuitRequest {
            principal: "desktop".to_string(),
            request_id: "quit-for-action-reply-loss".to_string(),
            intent: ApplicationQuitIntent::Exit { code: 7 },
        })
        .await
        .expect("activated quit");
    let ApplicationQuitOutcome::Accepted { receipt, state } = outcome else {
        panic!("quit must be accepted before target reconciliation");
    };
    assert!(matches!(
        state,
        ApplicationQuitState::ReconciliationRequired { .. }
    ));
    let plan = ShutdownPlanKey {
        shutdown_id: receipt.shutdown_id.clone(),
    };
    let page = coordinator
        .shutdown_plan_page_read_model(plan.clone(), 128, None)
        .await
        .expect("shutdown target capability");
    let target = &page.targets[0];
    let identity = target.action_identities[0].clone();
    let request = ShutdownTargetActionRequest {
        action_id: identity.action_id.clone(),
        plan: plan.clone(),
        ordinal: target.ordinal,
        target_key: target.target_key.clone(),
        origin_revision: identity.origin_revision,
        action: RecoveryActionKind::RetrySameEffect,
    };

    let first = coordinator
        .resolve_shutdown_target_action(request.clone())
        .await
        .expect("reply-loss action result");
    assert_eq!(
        first.outcome,
        RecoveryActionOutcome::ActionOutcomeUnknown {
            action_id: request.action_id.clone(),
        }
    );
    assert_eq!(first.process_action, None);
    let saved = coordinator
        .get_shutdown_target_action_status(&request.action_id)
        .await
        .expect("identity-only completed action lookup");
    let RecoveryActionStatus::Completed {
        action_id,
        result: saved_result,
    } = saved
    else {
        panic!("the dropped reply follows a durable completed result");
    };
    assert_eq!(action_id, request.action_id);
    assert_eq!(
        saved_result.classification,
        RecoveryResultClassification::Succeeded
    );
    assert_eq!(
        executor.readbacks.load(std::sync::atomic::Ordering::SeqCst),
        1
    );
    assert_eq!(
        executor.effects.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "the action readback proves the original effect; it does not start another"
    );
    assert_eq!(
        executor
            .subordinate_shutdowns
            .load(std::sync::atomic::Ordering::SeqCst),
        1
    );
    assert!(
        coordinator.current_shutdown().await.unwrap().is_none(),
        "the action, target, operation, plan and latest pointer close in one batch"
    );

    let replay = coordinator
        .resolve_shutdown_target_action(request.clone())
        .await
        .expect("same action replay");
    assert_eq!(
        replay.outcome,
        RecoveryActionOutcome::Completed {
            action_id: request.action_id.clone(),
            result: saved_result.clone(),
        }
    );
    assert_eq!(
        replay.process_action,
        Some(crate::usecase::shutdown_coordinator::ApplicationProcessAction::Exit { code: 7 })
    );
    assert_eq!(
        executor.readbacks.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "completed replay never consults the target executor"
    );

    let advanced_resource_revision = {
        let connection = harness.raw_connection();
        connection
            .execute(
                "UPDATE shutdown_targets SET revision = revision + 1
                 WHERE shutdown_id = ?1 AND ordinal = ?2",
                rusqlite::params![plan.shutdown_id, target.ordinal],
            )
            .expect("advance the current resource after the completed action");
        connection
            .query_row(
                "SELECT revision FROM shutdown_targets
                 WHERE shutdown_id = ?1 AND ordinal = ?2",
                rusqlite::params![plan.shutdown_id, target.ordinal],
                |row| row.get::<_, i64>(0),
            )
            .unwrap()
    };
    assert!(advanced_resource_revision as u64 > saved_result.resource_revision);

    coordinator
        .compact_shutdown_details(plan.clone())
        .await
        .expect("compact completed shutdown details");
    assert!(matches!(
        coordinator
            .shutdown_plan_page(plan.clone(), 1, None)
            .await
            .expect("archive-owned plan")
            .plan
            .details_state,
        ShutdownDetailsState::Compacted
    ));
    let after_compaction = coordinator
        .get_shutdown_target_action_status(&request.action_id)
        .await
        .expect("completed action does not depend on compacted target details");
    assert_eq!(
        after_compaction,
        RecoveryActionStatus::Completed {
            action_id: request.action_id.clone(),
            result: saved_result.clone(),
        }
    );
    let replay_after_compaction = coordinator
        .resolve_shutdown_target_action(request.clone())
        .await
        .expect("same action replay after compaction");
    assert!(matches!(
        replay_after_compaction.outcome,
        RecoveryActionOutcome::Completed { .. }
    ));
    assert_eq!(
        executor.readbacks.load(std::sync::atomic::Ordering::SeqCst),
        1
    );

    harness.clock.advance_ms(30 * 24 * 60 * 60 * 1_000);
    let replay_only = Arc::new(RecoveryReplayOnlyRepository {
        inner: Arc::clone(&harness.store),
        unavailable_resource_queries: std::sync::atomic::AtomicUsize::new(0),
    });
    let repository: Arc<dyn LocalEventTransactionRepository> = replay_only.clone();
    let authority: Arc<dyn crate::usecase::agent_session::operation::OperationBindingAuthority> =
        harness.store.clone();
    let restart_executor = TestShutdownExecutor::with_targets(1, ShutdownExecutorMode::Complete);
    let restarted = crate::usecase::shutdown_coordinator::ShutdownCoordinator::new(
        repository,
        authority,
        restart_executor.clone(),
        harness.store.installation_id().to_string(),
        "b093-restarted-boot".to_string(),
    );
    assert!(matches!(
        restarted.shutdown_plan_page(plan.clone(), 1, None).await,
        Err(LocalEventQueryError::StorageUnavailable { .. })
    ));
    let restarted_status = restarted
        .get_shutdown_target_action_status(&request.action_id)
        .await
        .expect("identity-only replay survives current-resource failure");
    assert_eq!(
        restarted_status,
        RecoveryActionStatus::Completed {
            action_id: request.action_id.clone(),
            result: saved_result.clone(),
        }
    );
    let restarted_replay = restarted
        .resolve_shutdown_target_action(request.clone())
        .await
        .expect("same action replay after restart and resolver failure");
    assert_eq!(
        restarted_replay.outcome,
        RecoveryActionOutcome::Completed {
            action_id: request.action_id.clone(),
            result: saved_result,
        }
    );
    assert_eq!(
        replay_only
            .unavailable_resource_queries
            .load(std::sync::atomic::Ordering::SeqCst),
        1,
        "completed action replay must not reconstruct its result from the unavailable resource"
    );
    assert_eq!(
        restart_executor
            .effects
            .load(std::sync::atomic::Ordering::SeqCst),
        0
    );
    assert_eq!(
        restart_executor
            .readbacks
            .load(std::sync::atomic::Ordering::SeqCst),
        0
    );

    let next_executor = TestShutdownExecutor::with_targets(0, ShutdownExecutorMode::Complete);
    let next = shutdown_coordinator(&harness, &next_executor)
        .request(ApplicationQuitRequest {
            principal: "desktop".to_string(),
            request_id: "quit-after-recovered-plan".to_string(),
            intent: ApplicationQuitIntent::Exit { code: 0 },
        })
        .await
        .expect("new quit is no longer blocked by the resolved plan");
    assert!(matches!(
        next,
        ApplicationQuitOutcome::Accepted {
            state: ApplicationQuitState::Completed,
            ..
        }
    ));
}

#[tokio::test]
async fn b085_each_target_action_commits_one_result_and_the_last_action_terminates_the_plan() {
    use crate::domain::agent_session::events::{RecoveryActionKind, RecoveryResultClassification};
    use crate::usecase::agent_session::operation::{RecoveryActionOutcome, RecoveryActionStatus};
    use crate::usecase::shutdown_coordinator::{
        ApplicationQuitIntent, ApplicationQuitOutcome, ApplicationQuitRequest,
        ApplicationQuitState, ShutdownTargetActionRequest,
    };

    let harness = Harness::open();
    let executor = TestShutdownExecutor::fail_then_read_completed(3, None);
    let coordinator = shutdown_coordinator(&harness, &executor);
    let outcome = coordinator
        .request(ApplicationQuitRequest {
            principal: "desktop".to_string(),
            request_id: "b085-three-target-plan".to_string(),
            intent: ApplicationQuitIntent::Exit { code: 9 },
        })
        .await
        .expect("accepted shutdown with recoverable targets");
    let ApplicationQuitOutcome::Accepted { receipt, state } = outcome else {
        panic!("shutdown must be accepted");
    };
    assert!(matches!(
        state,
        ApplicationQuitState::ReconciliationRequired { .. }
    ));
    let plan = ShutdownPlanKey {
        shutdown_id: receipt.shutdown_id.clone(),
    };
    let issued = coordinator
        .shutdown_plan_page_read_model(plan.clone(), 128, None)
        .await
        .expect("three issued target capabilities");
    assert_eq!(issued.targets.len(), 3);

    for (index, target) in issued.targets.iter().enumerate() {
        let identity = target
            .action_identities
            .iter()
            .find(|identity| identity.action == RecoveryActionKind::RetrySameEffect)
            .expect("retry capability");
        let request = ShutdownTargetActionRequest {
            action_id: identity.action_id.clone(),
            plan: plan.clone(),
            ordinal: target.ordinal,
            target_key: target.target_key.clone(),
            origin_revision: identity.origin_revision,
            action: RecoveryActionKind::RetrySameEffect,
        };
        let execution = coordinator
            .resolve_shutdown_target_action(request.clone())
            .await
            .expect("target action");
        let (action_id, result) = match execution.outcome {
            RecoveryActionOutcome::Completed { action_id, result } => (action_id, result),
            other => panic!("target action must complete: {other:?}"),
        };
        assert_eq!(action_id, request.action_id);
        assert_eq!(
            result.classification,
            RecoveryResultClassification::Succeeded
        );
        assert_eq!(
            coordinator
                .get_shutdown_target_action_status(&request.action_id)
                .await
                .expect("same identity status"),
            RecoveryActionStatus::Completed {
                action_id: request.action_id.clone(),
                result: result.clone(),
            },
            "the owner lookup and target mutation expose one saved completed result"
        );

        let durable = coordinator
            .shutdown_plan_page_read_model(plan.clone(), 128, None)
            .await
            .expect("progressive plan page");
        assert_eq!(durable.plan.completed_count, Some((index + 1) as i64));
        assert_eq!(durable.plan.unresolved_count, Some((2 - index) as i64));
        assert_eq!(durable.targets[index].state, "completed");
        assert_eq!(
            execution.process_action,
            Some(crate::usecase::shutdown_coordinator::ApplicationProcessAction::Exit { code: 9 }),
            "an already activated plan keeps its bounded exit permit while targets are resolved"
        );
        if index < 2 {
            assert!(coordinator.current_shutdown().await.unwrap().is_some());
        } else {
            assert_eq!(durable.plan.phase, ApplicationShutdownPhase::Completed);
            assert!(coordinator.current_shutdown().await.unwrap().is_none());
        }
    }
    assert_eq!(
        executor.effects.load(std::sync::atomic::Ordering::SeqCst),
        3,
        "recovery reads the three original effect identities without starting a new effect"
    );
    assert_eq!(
        executor.readbacks.load(std::sync::atomic::Ordering::SeqCst),
        3
    );
    assert_eq!(
        executor
            .subordinate_shutdowns
            .load(std::sync::atomic::Ordering::SeqCst),
        1
    );

    let next_executor = TestShutdownExecutor::with_targets(0, ShutdownExecutorMode::Complete);
    let next = shutdown_coordinator(&harness, &next_executor)
        .request(ApplicationQuitRequest {
            principal: "desktop".to_string(),
            request_id: "b085-next-quit".to_string(),
            intent: ApplicationQuitIntent::Exit { code: 0 },
        })
        .await
        .expect("resolved plan does not block the next quit");
    assert!(matches!(
        next,
        ApplicationQuitOutcome::Accepted {
            state: ApplicationQuitState::Completed,
            ..
        }
    ));
}

#[tokio::test]
async fn shutdown_action_rejects_unavailable_tampered_and_stale_capabilities_without_effects() {
    use crate::domain::agent_session::events::RecoveryActionKind;
    use crate::usecase::agent_session::operation::{
        derive_recovery_action_id, RecoveryActionOutcome, RecoveryActionRejection,
    };
    use crate::usecase::shutdown_coordinator::{
        ApplicationQuitIntent, ApplicationQuitOutcome, ApplicationQuitRequest,
        ShutdownTargetActionRequest,
    };

    let harness = Harness::open();
    let executor = TestShutdownExecutor::fail_then_read_completed(1, None);
    let coordinator = shutdown_coordinator(&harness, &executor);
    let outcome = coordinator
        .request(ApplicationQuitRequest {
            principal: "desktop".to_string(),
            request_id: "quit-for-invalid-actions".to_string(),
            intent: ApplicationQuitIntent::Exit { code: 0 },
        })
        .await
        .expect("activated quit");
    let ApplicationQuitOutcome::Accepted { receipt, .. } = outcome else {
        panic!("quit must be accepted");
    };
    let plan = ShutdownPlanKey {
        shutdown_id: receipt.shutdown_id,
    };
    let page = coordinator
        .shutdown_plan_page_read_model(plan.clone(), 128, None)
        .await
        .expect("issued retry capability");
    let target = page.targets[0].clone();
    let retry = target.action_identities[0].clone();
    let retry_request = ShutdownTargetActionRequest {
        action_id: retry.action_id.clone(),
        plan: plan.clone(),
        ordinal: target.ordinal,
        target_key: target.target_key.clone(),
        origin_revision: retry.origin_revision,
        action: RecoveryActionKind::RetrySameEffect,
    };
    let resource_ref = format!(
        "shutdown-target:{}:{}:{}",
        plan.shutdown_id, target.ordinal, target.target_key
    );
    let unavailable_id = derive_recovery_action_id(
        harness.store.as_ref(),
        harness.store.installation_id(),
        &resource_ref,
        retry.origin_revision,
        RecoveryActionKind::ReadAgain,
    );
    let unavailable = coordinator
        .resolve_shutdown_target_action(ShutdownTargetActionRequest {
            action_id: unavailable_id.clone(),
            action: RecoveryActionKind::ReadAgain,
            ..retry_request.clone()
        })
        .await
        .expect("closed unavailable result");
    assert_eq!(
        unavailable.outcome,
        RecoveryActionOutcome::Rejected {
            action_id: unavailable_id,
            rejection: RecoveryActionRejection::ActionUnavailable,
        }
    );
    let mut tampered = retry_request.clone();
    tampered.target_key.push('x');
    assert!(matches!(
        coordinator.resolve_shutdown_target_action(tampered).await,
        Err(crate::usecase::agent_session::operation::RecoveryActionError::NotFound)
    ));
    assert_eq!(
        executor.readbacks.load(std::sync::atomic::Ordering::SeqCst),
        0
    );

    let point = harness
        .store
        .query(LocalEventQuery::ShutdownTargetByIdentity {
            plan: plan.clone(),
            ordinal: target.ordinal,
        })
        .await
        .expect("point target query");
    let LocalEventQueryResult::ShutdownTargetByIdentity(Some(point)) = point else {
        panic!("shutdown target is available");
    };
    let stale_revision = point.revision.next().unwrap();
    let changed = harness
        .raw_connection()
        .execute(
            "UPDATE shutdown_targets SET revision = ?1
             WHERE shutdown_id = ?2 AND ordinal = ?3 AND revision = ?4",
            rusqlite::params![
                stale_revision.value(),
                plan.shutdown_id,
                target.ordinal,
                point.revision.value()
            ],
        )
        .expect("advance target revision as an external race fixture");
    assert_eq!(changed, 1);
    let stale = coordinator
        .resolve_shutdown_target_action(retry_request)
        .await
        .expect("closed stale result");
    assert_eq!(
        stale.outcome,
        RecoveryActionOutcome::Rejected {
            action_id: retry.action_id,
            rejection: RecoveryActionRejection::RevisionConflict {
                current_revision: stale_revision.value() as u64,
            },
        }
    );
    assert_eq!(
        executor.effects.load(std::sync::atomic::Ordering::SeqCst),
        1
    );
    assert_eq!(
        executor.readbacks.load(std::sync::atomic::Ordering::SeqCst),
        0
    );
}

#[tokio::test]
async fn shutdown_action_revision_change_after_reservation_rejects_before_effect_handoff() {
    use crate::domain::agent_session::events::RecoveryActionKind;
    use crate::usecase::agent_session::operation::{
        RecoveryActionOutcome, RecoveryActionRejection,
    };
    use crate::usecase::shutdown_coordinator::{
        ApplicationQuitIntent, ApplicationQuitOutcome, ApplicationQuitRequest,
        ShutdownTargetActionRequest,
    };

    let harness = Harness::open();
    let executor = TestShutdownExecutor::fail_then_read_completed(1, None);
    let coordinator = shutdown_coordinator(&harness, &executor);
    let outcome = coordinator
        .request(ApplicationQuitRequest {
            principal: "desktop".to_string(),
            request_id: "quit-for-post-reservation-race".to_string(),
            intent: ApplicationQuitIntent::Exit { code: 0 },
        })
        .await
        .expect("activated quit");
    let ApplicationQuitOutcome::Accepted { receipt, .. } = outcome else {
        panic!("quit must be accepted");
    };
    let plan = ShutdownPlanKey {
        shutdown_id: receipt.shutdown_id,
    };
    let page = coordinator
        .shutdown_plan_page_read_model(plan.clone(), 128, None)
        .await
        .expect("issued retry capability");
    let target = page.targets[0].clone();
    let retry = target.action_identities[0].clone();
    let request = ShutdownTargetActionRequest {
        action_id: retry.action_id.clone(),
        plan: plan.clone(),
        ordinal: target.ordinal,
        target_key: target.target_key,
        origin_revision: retry.origin_revision,
        action: RecoveryActionKind::RetrySameEffect,
    };
    let effects_before = executor.effects.load(std::sync::atomic::Ordering::SeqCst);
    let readbacks_before = executor.readbacks.load(std::sync::atomic::Ordering::SeqCst);
    let hook_fired = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let database_path = harness.database_path();
    coordinator.set_recovery_pre_handoff_hook(Arc::new({
        let store = Arc::clone(&harness.store);
        let plan = plan.clone();
        let hook_fired = Arc::clone(&hook_fired);
        let database_path = database_path.clone();
        move || {
            let store = Arc::clone(&store);
            let plan = plan.clone();
            let hook_fired = Arc::clone(&hook_fired);
            let database_path = database_path.clone();
            Box::pin(async move {
                if hook_fired.swap(true, std::sync::atomic::Ordering::SeqCst) {
                    return;
                }
                let point = store
                    .query(LocalEventQuery::ShutdownTargetByIdentity {
                        plan: plan.clone(),
                        ordinal: target.ordinal,
                    })
                    .await
                    .expect("reserved target query");
                let LocalEventQueryResult::ShutdownTargetByIdentity(Some(point)) = point else {
                    panic!("reserved shutdown target is available");
                };
                let next_revision = point.revision.next().expect("bounded target revision");
                let changed = rusqlite::Connection::open(database_path)
                    .expect("open external race fixture connection")
                    .execute(
                        "UPDATE shutdown_targets SET revision = ?1
                         WHERE shutdown_id = ?2 AND ordinal = ?3 AND revision = ?4",
                        rusqlite::params![
                            next_revision.value(),
                            plan.shutdown_id,
                            target.ordinal,
                            point.revision.value()
                        ],
                    )
                    .expect("advance the claimed target before effect handoff");
                assert_eq!(changed, 1);
            })
        }
    }));

    let result = coordinator
        .resolve_shutdown_target_action(request)
        .await
        .expect("closed post-reservation race result");

    assert_eq!(
        result.outcome,
        RecoveryActionOutcome::Rejected {
            action_id: retry.action_id,
            rejection: RecoveryActionRejection::TargetRevisionChanged,
        }
    );
    assert_eq!(result.process_action, None);
    assert!(hook_fired.load(std::sync::atomic::Ordering::SeqCst));
    assert_eq!(
        executor.effects.load(std::sync::atomic::Ordering::SeqCst),
        effects_before
    );
    assert_eq!(
        executor.readbacks.load(std::sync::atomic::Ordering::SeqCst),
        readbacks_before
    );
}

/// Release-only R-017/B-068/B-069 acceptance owner. This is intentionally
/// ignored in ordinary coverage/debug runs; the Quality Gate invokes this
/// exact test once under `--release`.
#[tokio::test]
#[ignore = "release performance acceptance gate"]
async fn release_performance_acceptance_gate() {
    let small = Harness::open();
    let large = Harness::open();
    seed_performance_fixture(&small, 10, "small-10");
    seed_performance_fixture(&large, 1_000_000, "large-1000000");
    let _small_directory_tripwire = LegacyDirectoryScanTripwire::install(&small.root);
    let _large_directory_tripwire = LegacyDirectoryScanTripwire::install(&large.root);

    let small_pending = sample_pending_usecase(&small).await;
    let large_pending = sample_pending_usecase(&large).await;
    let small_mutation = sample_public_mutation(&small, "perf-commit-small").await;
    let large_mutation = sample_public_mutation(&large, "perf-commit-large").await;
    let small_terminal = sample_terminal_usecase(&small, "small").await;
    let large_terminal = sample_terminal_usecase(&large, "large").await;
    let query_plan = performance_query_plan_steps(&large);
    let (query_busy_partial_count, deadline_partial_count) =
        assert_reader_failure_contract(&large).await;

    let ratio = |large: u128, small: u128| large as f64 / small.max(1) as f64;
    let pending_ratio = ratio(large_pending.0, small_pending.0);
    let identity_ratio = ratio(large_mutation.identity_p95, small_mutation.identity_p95);
    let terminal_ratio = ratio(large_terminal.p95, small_terminal.p95);
    let mutation_ratio = ratio(large_mutation.mutation.p95, small_mutation.mutation.p95);
    let report = serde_json::json!({
        "fixture": { "small_rows": 10, "large_rows": 1_000_000, "pending_entries": 200, "samples": 1_000 },
        "pending_first_page": { "p95_us": large_pending.0, "p99_us": large_pending.1, "large_small_p95_ratio": pending_ratio, "count": large_pending.2, "response_bytes": large_pending.3 },
        "identity": { "p95_us": large_mutation.identity_p95, "p99_us": large_mutation.identity_p99, "large_small_p95_ratio": identity_ratio, "count": large_mutation.identity_count, "response_bytes": large_mutation.identity_response_bytes },
        "terminal": { "p95_us": large_terminal.p95, "p99_us": large_terminal.p99, "large_small_p95_ratio": terminal_ratio, "partial_count": large_terminal.partial_count, "effect_count": large_terminal.effect_count, "response_bytes": large_terminal.response_bytes },
        "same_payload_mutation": { "p95_us": large_mutation.mutation.p95, "p99_us": large_mutation.mutation.p99, "large_small_p95_ratio": mutation_ratio, "partial_count": large_mutation.mutation.partial_count, "effect_count": large_mutation.mutation.effect_count, "response_bytes": large_mutation.mutation.response_bytes },
        "query_plan": { "steps": query_plan.clone(), "scan_events": query_plan.iter().any(|step| step.contains("SCAN events")), "session_directory_scan": query_plan.iter().any(|step| step.contains("workflow_executions") || step.contains("sessions/")) },
        "deadline_failure": { "deadline_ms": 2_000, "partial_count": deadline_partial_count },
        "query_busy": { "partial_count": query_busy_partial_count },
    });
    println!(
        "{}",
        serde_json::to_string(&report).expect("performance report")
    );

    assert_eq!(small_pending.2, 200);
    assert_eq!(large_pending.2, 200);
    assert!(large_pending.3 > 0);
    assert_eq!(small_mutation.identity_count, 1);
    assert_eq!(large_mutation.identity_count, 1);
    assert!(large_mutation.identity_response_bytes > 0);
    assert_eq!(small_terminal.partial_count, 0);
    assert_eq!(large_terminal.partial_count, 0);
    assert_eq!(small_terminal.effect_count, 1);
    assert_eq!(large_terminal.effect_count, 1);
    assert!(large_terminal.response_bytes > 0);
    assert_eq!(small_mutation.mutation.partial_count, 0);
    assert_eq!(large_mutation.mutation.partial_count, 0);
    assert_eq!(small_mutation.mutation.effect_count, 1_000);
    assert_eq!(large_mutation.mutation.effect_count, 1_000);
    assert!(large_mutation.mutation.response_bytes > 0);
    assert_eq!(query_busy_partial_count, 0);
    assert_eq!(deadline_partial_count, 0);
    assert!(!query_plan.iter().any(|step| step.contains("SCAN events")));
    assert!(
        pending_ratio <= 1.25,
        "pending p95 history ratio exceeded 1.25"
    );
    assert!(
        identity_ratio <= 1.25,
        "identity p95 history ratio exceeded 1.25"
    );
    assert!(
        terminal_ratio <= 1.25,
        "terminal p95 history ratio exceeded 1.25"
    );
    assert!(
        mutation_ratio <= 1.25,
        "mutation p95 history ratio exceeded 1.25"
    );
    assert!(large_pending.0 <= 50_000, "pending p95 exceeded 50ms");
    assert!(large_pending.1 <= 300_000, "pending p99 exceeded 300ms");
    assert!(
        large_mutation.identity_p95 <= 20_000,
        "identity p95 exceeded 20ms"
    );
    assert!(
        large_mutation.identity_p99 <= 50_000,
        "identity p99 exceeded 50ms"
    );
    assert!(large_terminal.p95 <= 150_000, "terminal p95 exceeded 150ms");
    assert!(large_terminal.p99 <= 300_000, "terminal p99 exceeded 300ms");
    assert!(
        large_mutation.mutation.p95 <= 150_000,
        "mutation p95 exceeded 150ms"
    );
    assert!(
        large_mutation.mutation.p99 <= 300_000,
        "mutation p99 exceeded 300ms"
    );
}

// --- Permission-response acceptance / completion crash matrix -------------

use crate::domain::agent_session::entities::{
    PermissionResponse as SqlitePermissionResponse,
    PermissionResponseDecision as SqlitePermissionResponseDecision,
};
use crate::usecase::agent_session::operation::{
    AcceptedPermissionResponseEffect as SqliteAcceptedPermissionResponseEffect,
    PermissionResponseCommandOutcome as SqlitePermissionResponseCommandOutcome,
    PermissionResponseExecutionStatus as SqlitePermissionResponseExecutionStatus,
    PermissionResponseGate as SqlitePermissionResponseGate,
    PermissionResponseOperationRequest as SqlitePermissionResponseOperationRequest,
    PermissionResponseOperationUsecase as SqlitePermissionResponseOperationUsecase,
    PermissionResponsePlan as SqlitePermissionResponsePlan,
};

#[derive(Debug, Clone, Copy)]
enum PermissionAcceptanceFault {
    BeforeBegin,
    AfterParticipantWrite(usize),
    BeforeCommit,
    AfterCommitBeforeReadback,
    DroppedReply,
}

impl PermissionAcceptanceFault {
    fn label(self) -> String {
        match self {
            Self::BeforeBegin => "before-begin".to_string(),
            Self::AfterParticipantWrite(write) => {
                format!("after-participant-write-{write}")
            }
            Self::BeforeCommit => "before-commit".to_string(),
            Self::AfterCommitBeforeReadback => "after-commit-before-readback".to_string(),
            Self::DroppedReply => "dropped-reply".to_string(),
        }
    }

    fn committed(self) -> bool {
        matches!(self, Self::AfterCommitBeforeReadback | Self::DroppedReply)
    }

    fn arm(self, fault: &FaultInjector) {
        match self {
            Self::BeforeBegin => fault.arm_fail_before_begin(),
            Self::AfterParticipantWrite(write) => {
                fault.arm_fail_after_participant_write_number(write)
            }
            Self::BeforeCommit => fault.arm_fail_before_commit(),
            Self::AfterCommitBeforeReadback => fault.arm_crash_after_commit_before_readback(),
            Self::DroppedReply => fault.arm_drop_reply(),
        }
    }
}

struct SqlitePermissionGate {
    effects: std::sync::Mutex<Vec<SqliteAcceptedPermissionResponseEffect>>,
    after_completion: std::sync::Mutex<Vec<SqliteAcceptedPermissionResponseEffect>>,
    completion_fault: Option<Arc<FaultInjector>>,
    arm_completion_fault: std::sync::atomic::AtomicBool,
}

impl SqlitePermissionGate {
    fn normal() -> Arc<Self> {
        Arc::new(Self {
            effects: std::sync::Mutex::new(Vec::new()),
            after_completion: std::sync::Mutex::new(Vec::new()),
            completion_fault: None,
            arm_completion_fault: std::sync::atomic::AtomicBool::new(false),
        })
    }

    fn crash_after_completion_commit(fault: Arc<FaultInjector>) -> Arc<Self> {
        Arc::new(Self {
            effects: std::sync::Mutex::new(Vec::new()),
            after_completion: std::sync::Mutex::new(Vec::new()),
            completion_fault: Some(fault),
            arm_completion_fault: std::sync::atomic::AtomicBool::new(true),
        })
    }

    fn effect_count(&self) -> usize {
        self.effects.lock().expect("permission effects").len()
    }

    fn after_completion_count(&self) -> usize {
        self.after_completion
            .lock()
            .expect("permission completion callbacks")
            .len()
    }
}

#[async_trait::async_trait]
impl SqlitePermissionResponseGate for SqlitePermissionGate {
    async fn plan_response(
        &self,
        session_id: &str,
        response: &SqlitePermissionResponse,
    ) -> Result<SqlitePermissionResponsePlan, SafeOperationFailure> {
        Ok(SqlitePermissionResponsePlan {
            session_id: session_id.to_string(),
            request_id: response.request_id.clone(),
            turn_id: 17,
            response: response.clone(),
            from_runtime_state: true,
        })
    }

    async fn execute(
        &self,
        effect: &SqliteAcceptedPermissionResponseEffect,
    ) -> Result<(), SafeOperationFailure> {
        self.effects
            .lock()
            .expect("permission effects")
            .push(effect.clone());
        if self
            .arm_completion_fault
            .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            self.completion_fault
                .as_ref()
                .expect("configured completion fault")
                .arm_crash_after_commit_before_readback();
        }
        Ok(())
    }

    async fn after_completion(&self, effect: &SqliteAcceptedPermissionResponseEffect) {
        self.after_completion
            .lock()
            .expect("permission completion callbacks")
            .push(effect.clone());
    }
}

fn sqlite_permission_response(request_id: &str) -> SqlitePermissionResponse {
    SqlitePermissionResponse {
        request_id: request_id.to_string(),
        decision: SqlitePermissionResponseDecision::Allow {
            updated_input: Some(
                crate::domain::agent_session::value_objects::JsonPayload::new_unchecked(
                    r#"{"nested":[1,true,{"k":"v"}],"path":"/owner/private"}"#.to_string(),
                ),
            ),
            answers: Some(
                crate::domain::agent_session::value_objects::JsonPayload::new_unchecked(
                    r#"{"approval":"yes","scope":["once","exact"]}"#.to_string(),
                ),
            ),
        },
    }
}

fn sqlite_permission_request(
    operation_id: &str,
    response: SqlitePermissionResponse,
) -> SqlitePermissionResponseOperationRequest {
    SqlitePermissionResponseOperationRequest {
        principal: "permission-owner".to_string(),
        operation_id: operation_id.to_string(),
        session_id: "permission-sqlite-session".to_string(),
        response,
    }
}

fn sqlite_permission_obligation_id(response: &SqlitePermissionResponse) -> String {
    let digest = sha2::Sha256::digest(
        format!(
            "permission-response-target\0{}\0{}\0{}",
            "permission-sqlite-session", 17, response.request_id
        )
        .as_bytes(),
    );
    let hex = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("permission-response-{hex}")
}

fn sqlite_permission_usecase(
    store: &Arc<LocalEventStore>,
    gate: Arc<SqlitePermissionGate>,
) -> SqlitePermissionResponseOperationUsecase {
    let repository: Arc<dyn LocalEventTransactionRepository> = store.clone();
    let authority: Arc<dyn crate::usecase::agent_session::operation::OperationBindingAuthority> =
        store.clone();
    SqlitePermissionResponseOperationUsecase::new(
        repository,
        authority,
        gate,
        store.installation_id().to_string(),
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExpectedSqlitePermissionState {
    Absent,
    AcceptedPending,
    Completed,
}

async fn assert_sqlite_permission_state(
    store: &Arc<LocalEventStore>,
    installation_id: &str,
    request: &SqlitePermissionResponseOperationRequest,
    expected: ExpectedSqlitePermissionState,
) {
    let obligation_id = sqlite_permission_obligation_id(&request.response);
    let operation = store
        .query(LocalEventQuery::OperationByIdentity {
            kind: OperationKind::PermissionResponse,
            operation_id: request.operation_id.clone(),
        })
        .await
        .expect("permission operation query");
    let binding = store
        .query(LocalEventQuery::OperationBindingByIdentity {
            key: CallerOperationKey {
                principal: request.principal.clone(),
                installation_id: installation_id.to_string(),
                kind: OperationKind::PermissionResponse,
                caller_request_id: request.operation_id.clone(),
            },
        })
        .await
        .expect("permission binding query");
    let obligation = store
        .query(LocalEventQuery::ObligationByIdentity {
            obligation_id: obligation_id.clone(),
        })
        .await
        .expect("permission obligation query");
    let page = store
        .load_stream(LoadStreamRequest {
            stream_id: StreamId::agent_session(&request.session_id).unwrap(),
            after: None,
            limit: 10,
        })
        .await
        .expect("permission stream query");

    if expected == ExpectedSqlitePermissionState::Absent {
        assert!(matches!(
            operation,
            LocalEventQueryResult::OperationByIdentity(None)
        ));
        assert!(matches!(
            binding,
            LocalEventQueryResult::OperationBindingByIdentity(None)
        ));
        assert!(matches!(
            obligation,
            LocalEventQueryResult::ObligationByIdentity(None)
        ));
        assert_eq!(page.head.value(), 0);
        assert!(page.events.is_empty());
        return;
    }

    let LocalEventQueryResult::OperationByIdentity(Some(operation)) = operation else {
        panic!("permission operation must be present: {expected:?}");
    };
    let OperationReceiptRecord::PermissionResponse {
        operation_id,
        session_id,
        request_id,
        input_ref,
        ..
    } = &operation.receipt
    else {
        panic!("permission operation receipt has the wrong closed variant");
    };
    assert_eq!(operation_id, &request.operation_id);
    assert_eq!(session_id, &request.session_id);
    assert_eq!(request_id, &request.response.request_id);
    assert_eq!(input_ref, &obligation_id);
    match expected {
        ExpectedSqlitePermissionState::AcceptedPending => assert!(matches!(
            &operation.latest_status.value,
            crate::domain::local_event::OperationStatusValue::AwaitingProviderResponse {
                obligation_id: saved
            } if saved == &obligation_id
        )),
        ExpectedSqlitePermissionState::Completed => assert!(matches!(
            &operation.latest_status.value,
            crate::domain::local_event::OperationStatusValue::PermissionCompleted {
                decision: crate::domain::local_event::PermissionDecisionRecord::Allowed
            }
        )),
        ExpectedSqlitePermissionState::Absent => unreachable!(),
    }

    let LocalEventQueryResult::OperationBindingByIdentity(Some(binding)) = binding else {
        panic!("permission binding must be present: {expected:?}");
    };
    assert_eq!(binding.operation_id, request.operation_id);
    let LocalEventQueryResult::ObligationByIdentity(Some(obligation)) = obligation else {
        panic!("permission obligation must be present: {expected:?}");
    };
    let ObligationRecord::PermissionResponse {
        operation_id,
        effect_identity,
        session_id,
        turn_id,
        response,
        owner_access,
        from_runtime_state,
        state,
    } = &obligation.record
    else {
        panic!("permission obligation has the wrong closed variant");
    };
    assert_eq!(operation_id, &request.operation_id);
    assert_eq!(
        effect_identity,
        &format!("permission-response:{}", request.operation_id)
    );
    assert_eq!(session_id, &request.session_id);
    assert_eq!(turn_id, "17");
    assert_eq!(response, &request.response, "exact response must survive");
    assert!(*owner_access);
    assert!(*from_runtime_state);
    let expected_obligation_state = match expected {
        ExpectedSqlitePermissionState::AcceptedPending => ObligationStateRecord::Pending,
        ExpectedSqlitePermissionState::Completed => ObligationStateRecord::Completed,
        ExpectedSqlitePermissionState::Absent => unreachable!(),
    };
    assert_eq!(*state, expected_obligation_state);
    assert_eq!(
        obligation.pending.is_some(),
        expected == ExpectedSqlitePermissionState::AcceptedPending
    );

    let expected_events = match expected {
        ExpectedSqlitePermissionState::AcceptedPending => 1,
        ExpectedSqlitePermissionState::Completed => 3,
        ExpectedSqlitePermissionState::Absent => unreachable!(),
    };
    assert_eq!(page.head.value(), expected_events);
    assert_eq!(page.events.len(), expected_events as usize);
    assert!(matches!(
        &page.events[0].event,
        LoadedDomainEvent::Known(event)
            if matches!(
                event.as_ref(),
                LocalDomainEvent::AgentSession(
                    AgentSessionDomainEvent::ObligationRecorded {
                        obligation_id: saved,
                        kind: crate::domain::agent_session::events::ObligationKind::PermissionResponse,
                        state: crate::domain::agent_session::events::ObligationState::Pending,
                        ..
                    }
                ) if saved == &obligation_id
            )
    ));
    if expected == ExpectedSqlitePermissionState::Completed {
        assert!(matches!(
            &page.events[1].event,
            LoadedDomainEvent::Known(event)
                if matches!(
                    event.as_ref(),
                    LocalDomainEvent::AgentSession(
                        AgentSessionDomainEvent::PermissionResolved {
                            turn_id: 17,
                            request_id: Some(saved),
                            decision: crate::domain::agent_session::events::PermissionDecision::Allowed,
                            ..
                        }
                    ) if saved == &request.response.request_id
                )
        ));
        assert!(matches!(
            &page.events[2].event,
            LoadedDomainEvent::Known(event)
                if matches!(
                    event.as_ref(),
                    LocalDomainEvent::AgentSession(
                        AgentSessionDomainEvent::ObligationRecorded {
                            obligation_id: saved,
                            kind: crate::domain::agent_session::events::ObligationKind::PermissionResponse,
                            state: crate::domain::agent_session::events::ObligationState::Completed,
                            ..
                        }
                    ) if saved == &obligation_id
                )
        ));
    }
}

fn reopen_permission_store(
    harness: Harness,
) -> (TempDir, Arc<LocalEventStore>, Arc<FaultInjector>) {
    let Harness {
        _dir,
        root,
        store,
        clock,
        fault: _,
    } = harness;
    drop(store);
    let fault = Arc::new(FaultInjector::new());
    let reopened = LocalEventStore::open(LocalEventStoreConfig {
        app_data_root: root,
        clock: Arc::new(clock),
        registry: Arc::new(EventCodecRegistry::new()),
        fault: Arc::clone(&fault),
        path_observer: Arc::new(
            crate::adaptor::gateway::local_event_store::layout::NoopStorePathObserver,
        ),
    })
    .expect("physically reopen permission store");
    (_dir, reopened, fault)
}

#[tokio::test]
async fn b015_permission_response_real_sqlite_acceptance_faults_are_atomic_and_same_identity_converges(
) {
    let mut faults = vec![PermissionAcceptanceFault::BeforeBegin];
    faults.extend((1..=4).map(PermissionAcceptanceFault::AfterParticipantWrite));
    faults.extend([
        PermissionAcceptanceFault::BeforeCommit,
        PermissionAcceptanceFault::AfterCommitBeforeReadback,
        PermissionAcceptanceFault::DroppedReply,
    ]);

    for acceptance_fault in faults {
        let label = acceptance_fault.label();
        let harness = Harness::open();
        let installation_id = harness.store.installation_id().to_string();
        let response = sqlite_permission_response(&format!("permission-request-{label}"));
        let request = sqlite_permission_request(&format!("permission-operation-{label}"), response);
        let gate = SqlitePermissionGate::normal();
        let usecase = sqlite_permission_usecase(&harness.store, Arc::clone(&gate));
        acceptance_fault.arm(&harness.fault);

        let first = usecase
            .request(request.clone())
            .await
            .expect("faulted permission acceptance has a safe outcome");
        assert_eq!(
            gate.effect_count(),
            0,
            "{label}: provider effect must stay behind durable acceptance"
        );
        if acceptance_fault.committed() {
            assert_eq!(
                first,
                SqlitePermissionResponseCommandOutcome::OutcomeUnknown {
                    operation_id: request.operation_id.clone(),
                },
                "{label}"
            );
            assert_sqlite_permission_state(
                &harness.store,
                &installation_id,
                &request,
                ExpectedSqlitePermissionState::AcceptedPending,
            )
            .await;
        } else {
            assert!(
                matches!(
                    first,
                    SqlitePermissionResponseCommandOutcome::RejectedBeforeCommit { .. }
                ),
                "{label}: pre-COMMIT fault must be a deterministic rejection"
            );
            assert_sqlite_permission_state(
                &harness.store,
                &installation_id,
                &request,
                ExpectedSqlitePermissionState::Absent,
            )
            .await;
        }

        drop(usecase);
        let (_dir, reopened, _restart_fault) = reopen_permission_store(harness);
        assert_sqlite_permission_state(
            &reopened,
            &installation_id,
            &request,
            if acceptance_fault.committed() {
                ExpectedSqlitePermissionState::AcceptedPending
            } else {
                ExpectedSqlitePermissionState::Absent
            },
        )
        .await;

        let restarted = sqlite_permission_usecase(&reopened, Arc::clone(&gate));
        let converged = restarted
            .request(request.clone())
            .await
            .expect("same identity converges after physical restart");
        let SqlitePermissionResponseCommandOutcome::Accepted(converged) = converged else {
            panic!("{label}: same identity must converge to one accepted operation");
        };
        assert_eq!(converged.receipt.operation_id, request.operation_id);
        assert_eq!(converged.receipt.session_id, request.session_id);
        assert_eq!(converged.receipt.request_id, request.response.request_id);
        if !matches!(
            &converged.latest_status,
            SqlitePermissionResponseExecutionStatus::Completed { .. }
        ) {
            let obligation = reopened
                .query(LocalEventQuery::ObligationByIdentity {
                    obligation_id: sqlite_permission_obligation_id(&request.response),
                })
                .await;
            panic!(
                "{label}: unexpected converged status {:?}; obligation {obligation:?}",
                converged.latest_status
            );
        }
        assert_eq!(
            gate.effect_count(),
            1,
            "{label}: exactly one provider effect"
        );
        assert_sqlite_permission_state(
            &reopened,
            &installation_id,
            &request,
            ExpectedSqlitePermissionState::Completed,
        )
        .await;

        let replay = restarted
            .request(request.clone())
            .await
            .expect("completed identity replays");
        assert_eq!(
            replay,
            SqlitePermissionResponseCommandOutcome::Accepted(converged)
        );
        assert_eq!(
            gate.effect_count(),
            1,
            "{label}: completed replay must not duplicate the provider effect"
        );
        assert_eq!(gate.after_completion_count(), 1);
    }
}

#[tokio::test]
async fn b015_permission_response_completion_post_commit_crash_reopens_exact_result_without_blind_replay(
) {
    let harness = Harness::open();
    let installation_id = harness.store.installation_id().to_string();
    let response = sqlite_permission_response("permission-request-completion-crash");
    let request =
        sqlite_permission_request("permission-operation-completion-crash", response.clone());
    let gate = SqlitePermissionGate::crash_after_completion_commit(Arc::clone(&harness.fault));
    let usecase = sqlite_permission_usecase(&harness.store, Arc::clone(&gate));

    let first = usecase
        .request(request.clone())
        .await
        .expect("completion reply-loss has a safe accepted result");
    let SqlitePermissionResponseCommandOutcome::Accepted(first) = first else {
        panic!("acceptance and the effect claim committed before completion reply loss");
    };
    assert!(matches!(
        first.latest_status,
        SqlitePermissionResponseExecutionStatus::ReconciliationRequired { .. }
    ));
    assert_eq!(gate.effect_count(), 1);
    assert_eq!(gate.after_completion_count(), 0);
    let effect = gate
        .effects
        .lock()
        .expect("permission effects")
        .first()
        .cloned()
        .expect("one exact provider handoff");
    assert_eq!(effect.operation_id, request.operation_id);
    assert_eq!(effect.plan.session_id, request.session_id);
    assert_eq!(effect.plan.response, response);

    assert_sqlite_permission_state(
        &harness.store,
        &installation_id,
        &request,
        ExpectedSqlitePermissionState::Completed,
    )
    .await;
    drop(usecase);
    let (_dir, reopened, _restart_fault) = reopen_permission_store(harness);
    assert_sqlite_permission_state(
        &reopened,
        &installation_id,
        &request,
        ExpectedSqlitePermissionState::Completed,
    )
    .await;

    let restarted = sqlite_permission_usecase(&reopened, Arc::clone(&gate));
    assert_eq!(
        restarted
            .recover_pending_permission_responses_pass()
            .await
            .expect("restart recovery scan"),
        0,
        "completed exact response is not pending recovery work"
    );
    let saved = restarted
        .get_operation(&request.principal, &request.operation_id)
        .await
        .expect("exact completed result after restart");
    assert_eq!(saved.receipt.operation_id, request.operation_id);
    assert_eq!(saved.receipt.session_id, request.session_id);
    assert_eq!(saved.receipt.request_id, request.response.request_id);
    assert!(matches!(
        saved.latest_status,
        SqlitePermissionResponseExecutionStatus::Completed { .. }
    ));
    assert_eq!(gate.effect_count(), 1);

    assert_eq!(
        restarted
            .request(request)
            .await
            .expect("same completed identity replays"),
        SqlitePermissionResponseCommandOutcome::Accepted(saved)
    );
    assert_eq!(
        gate.effect_count(),
        1,
        "restart and exact replay must never blindly resend an effect-reserved response"
    );
    assert_eq!(gate.after_completion_count(), 0);
}

fn f06_send_receipt(operation_id: &str) -> serde_json::Value {
    serde_json::json!({
        "schema": "send_receipt_v1",
        "operation_id": operation_id,
        "session_id": "f06-session",
        "input_ref": "f06-input",
        "disposition": { "type": "started_turn", "turn_id": "f06-turn" },
        "principal_mac": "00".repeat(32),
        "binding_hmac": "00".repeat(32),
    })
}

fn f06_stop_receipt(operation_id: &str) -> serde_json::Value {
    serde_json::json!({
        "schema": "stop_receipt_v1",
        "operation_id": operation_id,
        "session_id": "f06-session",
        "turn_id": "f06-turn",
        "accepted_revision": 0,
        "principal_mac": "00".repeat(32),
        "binding_hmac": "00".repeat(32),
    })
}

fn f06_send_status(value: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "schema": "send_status_v1",
        "status": value,
    })
}

fn f06_stop_status(value: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "schema": "stop_status_v1",
        "state": value,
    })
}

fn f06_operation_mutation(
    operation_id: &str,
    receipt: serde_json::Value,
    latest_status: serde_json::Value,
) -> LocalStateMutation {
    LocalStateMutation::OperationRecord(OperationRecordMutation {
        kind: OperationKind::Send,
        operation_id: operation_id.to_string(),
        receipt: payload(&receipt.to_string()),
        latest_status: payload(&latest_status.to_string()),
        expected: RevisionGuard::Absent,
        revision: Revision::new(0).unwrap(),
    })
}

#[tokio::test]
async fn f06_operation_record_write_rejects_cross_field_inconsistency() {
    let harness = Harness::open();
    let cases = vec![
        (
            "row-receipt-operation-id",
            f06_operation_mutation(
                "f06-row-id",
                f06_send_receipt("f06-different-receipt-id"),
                f06_send_status(serde_json::json!({
                    "type": "awaiting_provider_start",
                    "dependency_obligation_ids": [],
                })),
            ),
        ),
        (
            "row-receipt-family",
            f06_operation_mutation(
                "f06-receipt-family",
                f06_stop_receipt("f06-receipt-family"),
                f06_send_status(serde_json::json!({
                    "type": "awaiting_provider_start",
                    "dependency_obligation_ids": [],
                })),
            ),
        ),
        (
            "row-status-family",
            f06_operation_mutation(
                "f06-status-family",
                f06_send_receipt("f06-status-family"),
                f06_stop_status(serde_json::json!({ "type": "accepted" })),
            ),
        ),
        (
            "receipt-status-combination",
            f06_operation_mutation(
                "f06-invalid-combination",
                f06_send_receipt("f06-invalid-combination"),
                f06_send_status(serde_json::json!({ "type": "preparing" })),
            ),
        ),
    ];

    let mut accepted = Vec::new();
    for (ordinal, (label, mutation)) in cases.into_iter().enumerate() {
        let outcome = harness
            .store
            .commit_batch(batch(
                &format!("f06-write-{ordinal}"),
                &format!("f06-write-{ordinal}"),
                [ordinal as u8; 32],
                Vec::new(),
                Vec::new(),
                vec![mutation],
            ))
            .await;

        if !matches!(
            outcome,
            Err(CommitBatchError::PayloadConflict | CommitBatchError::Corrupt { .. })
        ) {
            accepted.push(format!("{label}: {outcome:?}"));
        }
    }

    assert!(
        accepted.is_empty(),
        "inconsistent operation records reached durable storage:\n{}",
        accepted.join("\n")
    );
}

fn f06_reopen_after_database_update(
    harness: Harness,
    update: impl FnOnce(&rusqlite::Connection),
) -> (TempDir, Arc<LocalEventStore>) {
    let database_path = harness.database_path();
    let Harness {
        _dir,
        root,
        store,
        clock,
        ..
    } = harness;
    drop(store);

    let connection = rusqlite::Connection::open(database_path).expect("open durable database");
    update(&connection);
    drop(connection);

    let reopened = LocalEventStore::open(LocalEventStoreConfig {
        app_data_root: root,
        clock: Arc::new(clock),
        registry: test_registry(),
        fault: Arc::new(FaultInjector::new()),
        path_observer: Arc::new(
            crate::adaptor::gateway::local_event_store::layout::NoopStorePathObserver,
        ),
    })
    .expect("physically reopen store");

    (_dir, reopened)
}

#[tokio::test]
async fn f06_operation_record_reopen_query_fails_closed_on_cross_field_corruption() {
    let harness = Harness::open();
    let ids = [
        "f06-read-operation-id",
        "f06-read-receipt-family",
        "f06-read-status-family",
        "f06-read-invalid-combination",
        "f06-read-unknown-failure-kind",
    ];
    let mutations = ids
        .iter()
        .map(|operation_id| {
            f06_operation_mutation(
                operation_id,
                f06_send_receipt(operation_id),
                f06_send_status(serde_json::json!({
                    "type": "awaiting_provider_start",
                    "dependency_obligation_ids": [],
                })),
            )
        })
        .collect();

    harness
        .store
        .commit_batch(batch(
            "f06-read-seed",
            "f06-read-seed",
            [60; 32],
            Vec::new(),
            Vec::new(),
            mutations,
        ))
        .await
        .expect("commit valid operation records before corruption");

    let (_keepalive, reopened) = f06_reopen_after_database_update(harness, |connection| {
        connection
            .execute(
                "UPDATE operation_records SET receipt = ?1
                 WHERE kind = 'send' AND operation_id = ?2",
                rusqlite::params![
                    f06_send_receipt("f06-other-operation-id").to_string(),
                    ids[0],
                ],
            )
            .unwrap();
        connection
            .execute(
                "UPDATE operation_records SET receipt = ?1
                 WHERE kind = 'send' AND operation_id = ?2",
                rusqlite::params![f06_stop_receipt(ids[1]).to_string(), ids[1]],
            )
            .unwrap();
        connection
            .execute(
                "UPDATE operation_records SET latest_status = ?1
                 WHERE kind = 'send' AND operation_id = ?2",
                rusqlite::params![
                    f06_stop_status(serde_json::json!({ "type": "accepted" })).to_string(),
                    ids[2],
                ],
            )
            .unwrap();
        connection
            .execute(
                "UPDATE operation_records SET latest_status = ?1
                 WHERE kind = 'send' AND operation_id = ?2",
                rusqlite::params![
                    f06_send_status(serde_json::json!({ "type": "preparing" })).to_string(),
                    ids[3],
                ],
            )
            .unwrap();
        connection
            .execute(
                "UPDATE operation_records SET latest_status = ?1
                 WHERE kind = 'send' AND operation_id = ?2",
                rusqlite::params![
                    f06_send_status(serde_json::json!({
                        "type": "failed",
                        "failure": {
                            "kind": "future_required_failure",
                            "retryable": false,
                            "label": "future failure",
                            "correlation_id": "f06-future-failure",
                        },
                    }))
                    .to_string(),
                    ids[4],
                ],
            )
            .unwrap();
    });

    let mut exposed = Vec::new();
    for operation_id in ids {
        let outcome = reopened
            .query(LocalEventQuery::OperationByIdentity {
                kind: OperationKind::Send,
                operation_id: operation_id.to_string(),
            })
            .await;

        if !matches!(
            outcome,
            Err(LocalEventQueryError::IncompatibleStoredEvent { .. }
                | LocalEventQueryError::Corrupt { .. })
        ) {
            exposed.push(format!("{operation_id}: {outcome:?}"));
        }
    }

    assert!(
        exposed.is_empty(),
        "incompatible durable records were exposed as normal operation views:\n{}",
        exposed.join("\n")
    );
}

#[tokio::test]
async fn f05_send_read_again_uses_durable_terminal_winner_after_restart() {
    use crate::domain::agent_session::events::RecoveryResultClassification;
    use crate::domain::local_event::{AgentTerminalKind, AgentTurnTerminalResultRecord};
    use crate::usecase::agent_session::operation::{
        AgentSendOperationUsecase, SendCommandOutcome, SendExecutionStatus, SendOperationRequest,
        SendRecoveryReadbackKind, SendRecoveryReadbackPort, SendRecoveryReadbackRequest,
        StableRecoveryEffectIdentity,
    };

    let harness = Harness::open();
    let installation_id = harness.store.installation_id().to_string();
    let session_id = "f05-production-send-session";
    let operation_id = "f05-production-send-operation";
    let execution_obligation_id = format!("{operation_id}.exec");
    let session_store = Arc::new(SessionStore::new(Arc::new(FileSessionStorage::default())));
    let repository: Arc<dyn LocalEventTransactionRepository> = harness.store.clone();
    session_store.set_local_event_repository(
        repository,
        harness.store.installation_id().to_string(),
        Arc::new(AgentSessionProjectionCodecV1),
    );
    let session = build_new_session_with_id(
        session_id.to_string(),
        "/f05-send-readback",
        Some("codex".to_string()),
        crate::domain::agent_session::PermissionMode::Ask,
        None,
        false,
        false,
        None,
    );
    session_store
        .save_full_session_for_restore(&harness.root, &session)
        .expect("seed production session projection");
    let gate = Arc::new(PerformanceSendGate {
        session_store: Arc::clone(&session_store),
        session_id: session_id.to_string(),
        effects: std::sync::atomic::AtomicUsize::new(0),
        planned: std::sync::atomic::AtomicUsize::new(0),
    });
    let usecase = AgentSendOperationUsecase::new(
        harness.store.clone(),
        harness.store.clone(),
        gate.clone(),
        harness.store.installation_id().to_string(),
    );

    let accepted = usecase
        .send(SendOperationRequest {
            principal: "local-app".to_string(),
            operation_id: operation_id.to_string(),
            canonical_payload: "{\"content\":\"f05 send\"}".to_string(),
        })
        .await
        .expect("durable send acceptance");
    assert!(matches!(accepted, SendCommandOutcome::Accepted(_)));
    assert_eq!(gate.effects.load(std::sync::atomic::Ordering::SeqCst), 1);
    usecase
        .record_execution_status(
            operation_id,
            SendExecutionStatus::ProviderStartReserved {
                obligation_id: execution_obligation_id.clone(),
            },
        )
        .await
        .expect("reserve the exact provider effect");

    let mut terminal_batch = batch(
        "f05-production-send-terminal-evidence",
        "f05-production-send-terminal-evidence",
        [5; 32],
        Vec::new(),
        Vec::new(),
        vec![LocalStateMutation::TerminalRecord(TerminalRecordMutation {
            session_id: session_id.to_string(),
            turn_id: "1".to_string(),
            terminal_identity: "f05-production-send-terminal".to_string(),
            result: TerminalResultRecord::AgentTurn {
                kind: AgentTerminalKind::Completed,
                session_id: session_id.to_string(),
                turn_id: "1".to_string(),
                message_id: "f05-production-send-assistant".to_string(),
                streaming_final_sequence: 0,
                completed_at_bits: 1.0_f64.to_bits(),
                result: AgentTurnTerminalResultRecord::Current(
                    crate::domain::agent_session::entities::TurnResult::Completed {
                        stop_reason: None,
                        token_usage: None,
                    },
                ),
            },
            participant_digest: [5; 32],
        })],
    );
    terminal_batch.idempotency.installation_id = harness.store.installation_id().to_string();
    terminal_batch.idempotency.operation_kind = CommitOperationKind::Recovery;
    harness
        .store
        .commit_batch(terminal_batch)
        .await
        .expect("commit provider terminal evidence without send result participants");

    drop(usecase);
    drop(gate);
    drop(session_store);
    let (_keepalive, reopened, _fault) = reopen_permission_store(harness);
    let readback_session_store =
        Arc::new(SessionStore::new(Arc::new(FileSessionStorage::default())));
    let readback_gate = Arc::new(PerformanceSendGate {
        session_store: readback_session_store,
        session_id: session_id.to_string(),
        effects: std::sync::atomic::AtomicUsize::new(0),
        planned: std::sync::atomic::AtomicUsize::new(0),
    });
    let readback =
        AgentSendOperationUsecase::new(reopened.clone(), reopened, readback_gate, installation_id);

    let result = SendRecoveryReadbackPort::read_send(
        &readback,
        &SendRecoveryReadbackRequest {
            effect_identity: StableRecoveryEffectIdentity::parse(execution_obligation_id)
                .expect("stable send effect identity"),
            operation_id: operation_id.to_string(),
            session_id: session_id.to_string(),
            kind: SendRecoveryReadbackKind::TurnExecution,
        },
    )
    .await
    .expect("production send readback");

    assert_eq!(
        result.classification,
        RecoveryResultClassification::Succeeded,
        "the durable terminal winner is authoritative evidence that the send effect completed"
    );
    assert!(
        result
            .owner_mutations
            .iter()
            .any(|mutation| matches!(mutation, LocalStateMutation::OperationRecord(_))),
        "readback must atomically advance the send operation"
    );
    assert!(
        result
            .owner_mutations
            .iter()
            .any(|mutation| matches!(mutation, LocalStateMutation::Obligation(_))),
        "readback must atomically close the send obligation"
    );
}

#[derive(Default)]
struct RecordingStorePathObserver {
    operations: std::sync::Mutex<
        Vec<(
            crate::adaptor::gateway::local_event_store::layout::StorePathOperation,
            std::path::PathBuf,
        )>,
    >,
}

impl crate::adaptor::gateway::local_event_store::layout::StorePathObserver
    for RecordingStorePathObserver
{
    fn observe(
        &self,
        operation: crate::adaptor::gateway::local_event_store::layout::StorePathOperation,
        path: &std::path::Path,
    ) {
        self.operations
            .lock()
            .expect("path observation lock")
            .push((operation, path.to_path_buf()));
    }
}

struct FixedLayoutStartupObserver {
    source_root: std::path::PathBuf,
    source_store_bytes: Vec<Vec<u8>>,
    operations: std::sync::Mutex<
        Vec<(
            crate::adaptor::gateway::local_event_store::layout::StorePathOperation,
            std::path::PathBuf,
        )>,
    >,
    preexisting_temp_entries: std::collections::HashSet<std::path::PathBuf>,
    discovered_external_store_copies: std::sync::Mutex<Vec<std::path::PathBuf>>,
}

impl FixedLayoutStartupObserver {
    fn new(source_root: &std::path::Path) -> Self {
        let source_layout = StoreLayout::new(source_root);
        let source_database = source_layout.database_path();
        let source_wal = std::path::PathBuf::from(format!("{}-wal", source_database.display()));
        let source_store_bytes = [source_database, source_wal]
            .into_iter()
            .filter_map(|path| std::fs::read(path).ok())
            .filter(|bytes| !bytes.is_empty())
            .collect::<Vec<_>>();
        assert_eq!(
            source_store_bytes.len(),
            2,
            "the fixed-layout oracle requires non-empty DB and WAL source snapshots"
        );
        Self {
            source_root: source_root.to_path_buf(),
            source_store_bytes,
            operations: std::sync::Mutex::new(Vec::new()),
            preexisting_temp_entries: Self::temp_entries().into_iter().collect(),
            discovered_external_store_copies: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn temp_entries() -> Vec<std::path::PathBuf> {
        let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) else {
            return Vec::new();
        };
        entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .collect()
    }

    fn capture_copy_destinations(&self) {
        let mut discovered = self
            .discovered_external_store_copies
            .lock()
            .expect("external-store-copy observation lock");
        for entry in Self::temp_entries() {
            if self.preexisting_temp_entries.contains(&entry)
                || entry.starts_with(&self.source_root)
            {
                continue;
            }
            let candidates = if entry.is_dir() {
                std::fs::read_dir(&entry)
                    .into_iter()
                    .flatten()
                    .filter_map(Result::ok)
                    .map(|child| child.path())
                    .collect::<Vec<_>>()
            } else {
                vec![entry]
            };
            for candidate in candidates {
                let Ok(bytes) = std::fs::read(&candidate) else {
                    continue;
                };
                if self
                    .source_store_bytes
                    .iter()
                    .any(|source| source == &bytes)
                    && !discovered.contains(&candidate)
                {
                    discovered.push(candidate);
                }
            }
        }
    }

    fn assert_fixed_layout_only(&self, root: &std::path::Path) {
        self.capture_copy_destinations();
        let layout = StoreLayout::new(root);
        let database = layout.database_path();
        let allowed = [
            root.to_path_buf(),
            database.clone(),
            std::path::PathBuf::from(format!("{}-wal", database.display())),
            std::path::PathBuf::from(format!("{}-shm", database.display())),
            layout.writer_lock_path(),
            layout.initial_create_evidence_path(),
        ];
        let operations = self
            .operations
            .lock()
            .expect("startup path observation lock");
        assert!(
            operations.iter().all(|(_, path)| allowed.contains(path)),
            "startup observed a path outside the fixed store layout: {operations:?}"
        );
        assert!(
            operations.iter().any(|(_, path)| path == &database),
            "the acceptance oracle did not observe the fixed database source"
        );
        let wal = std::path::PathBuf::from(format!("{}-wal", database.display()));
        for expected_operation in [
            crate::adaptor::gateway::local_event_store::layout::StorePathOperation::Metadata,
            crate::adaptor::gateway::local_event_store::layout::StorePathOperation::Open,
            crate::adaptor::gateway::local_event_store::layout::StorePathOperation::Read,
        ] {
            assert!(
                operations
                    .iter()
                    .any(|(operation, path)| *operation == expected_operation && path == &wal),
                "the acceptance oracle did not observe {expected_operation:?} on the fixed WAL source"
            );
        }
        let discovered = self
            .discovered_external_store_copies
            .lock()
            .expect("external-store-copy observation lock");
        assert!(
            discovered.is_empty(),
            "startup created or referenced a database copy outside the fixed layout: {discovered:?}"
        );
    }
}

impl crate::adaptor::gateway::local_event_store::layout::StorePathObserver
    for FixedLayoutStartupObserver
{
    fn observe(
        &self,
        operation: crate::adaptor::gateway::local_event_store::layout::StorePathOperation,
        path: &std::path::Path,
    ) {
        self.operations
            .lock()
            .expect("startup path observation lock")
            .push((operation, path.to_path_buf()));
        // The former inspection implementation did not report its copy
        // destination through the path port. Scan at every observed source
        // operation so both sides of a transient DB/WAL copy are in the
        // acceptance oracle even when the destination is removed before
        // startup returns.
        self.capture_copy_destinations();
    }
}

fn acceptance_store_config(
    root: &std::path::Path,
    fault: Arc<FaultInjector>,
    observer: Arc<dyn crate::adaptor::gateway::local_event_store::layout::StorePathObserver>,
) -> LocalEventStoreConfig {
    LocalEventStoreConfig {
        app_data_root: root.to_path_buf(),
        clock: Arc::new(FakeStoreClock::at(1_000)),
        registry: test_registry(),
        fault,
        path_observer: observer,
    }
}

fn noop_path_observer(
) -> Arc<dyn crate::adaptor::gateway::local_event_store::layout::StorePathObserver> {
    Arc::new(crate::adaptor::gateway::local_event_store::layout::NoopStorePathObserver)
}

fn leave_nonempty_wal(root: &std::path::Path, update: &str) -> std::path::PathBuf {
    let database = StoreLayout::new(root).database_path();
    let connection = rusqlite::Connection::open(&database).expect("open non-empty WAL fixture");
    connection
        .set_db_config(
            rusqlite::config::DbConfig::SQLITE_DBCONFIG_NO_CKPT_ON_CLOSE,
            true,
        )
        .expect("preserve committed WAL after fixture connection close");
    connection
        .execute_batch("PRAGMA journal_mode = WAL; PRAGMA wal_autocheckpoint = 0;")
        .expect("configure non-empty WAL fixture");
    connection
        .execute_batch(update)
        .expect("append a committed frame to the fixture WAL");
    let wal = std::path::PathBuf::from(format!("{}-wal", database.display()));
    drop(connection);
    assert!(
        std::fs::metadata(&wal)
            .expect("non-empty fixture WAL")
            .len()
            > 0,
        "the startup fixture must exercise a non-empty WAL"
    );
    assert!(
        std::path::PathBuf::from(format!("{}-shm", database.display())).is_file(),
        "the startup fixture must contain SQLite's fixed SHM sidecar"
    );
    wal
}

fn fixed_store_files(root: &std::path::Path) -> Vec<(String, Option<Vec<u8>>)> {
    let database = StoreLayout::new(root).database_path();
    [
        database.clone(),
        std::path::PathBuf::from(format!("{}-wal", database.display())),
        std::path::PathBuf::from(format!("{}-shm", database.display())),
    ]
    .into_iter()
    .map(|path| {
        (
            path.file_name()
                .expect("fixed store file name")
                .to_string_lossy()
                .into_owned(),
            std::fs::read(path).ok(),
        )
    })
    .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InitialCreateArtifactSnapshot {
    database: Option<Vec<u8>>,
    wal: Option<Vec<u8>>,
    shm: Option<Vec<u8>>,
    evidence: Option<Vec<u8>>,
}

fn read_optional_artifact(path: &std::path::Path) -> Option<Vec<u8>> {
    match std::fs::read(path) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => panic!("read initial-create artifact {}: {error}", path.display()),
    }
}

fn initial_create_artifacts(root: &std::path::Path) -> InitialCreateArtifactSnapshot {
    let layout = StoreLayout::new(root);
    let database = layout.database_path();
    InitialCreateArtifactSnapshot {
        database: read_optional_artifact(&database),
        wal: read_optional_artifact(&std::path::PathBuf::from(format!(
            "{}-wal",
            database.display()
        ))),
        shm: read_optional_artifact(&std::path::PathBuf::from(format!(
            "{}-shm",
            database.display()
        ))),
        evidence: read_optional_artifact(&layout.initial_create_evidence_path()),
    }
}

fn metadata_identity_and_keys_from_artifacts(
    artifacts: &InitialCreateArtifactSnapshot,
) -> Option<(String, Vec<u8>, Vec<u8>)> {
    let database_bytes = artifacts.database.as_ref()?;
    let snapshot_root = tempfile::tempdir().expect("create crash-artifact inspection root");
    let layout = StoreLayout::new(snapshot_root.path());
    let database = layout.database_path();
    std::fs::write(&database, database_bytes).expect("copy crash-artifact database");
    if let Some(wal) = &artifacts.wal {
        std::fs::write(
            std::path::PathBuf::from(format!("{}-wal", database.display())),
            wal,
        )
        .expect("copy crash-artifact WAL");
    }
    // Do not copy SHM. SQLite rebuilds it against the private DB+WAL snapshot,
    // so inspecting the crash oracle cannot mutate the original sidecars.
    let connection =
        rusqlite::Connection::open(&database).expect("open private crash-artifact snapshot");
    let metadata_exists: bool = connection
        .query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM sqlite_schema
                 WHERE type = 'table' AND name = 'store_metadata'
             )",
            [],
            |row| row.get(0),
        )
        .expect("inspect private crash-artifact metadata table");
    metadata_exists.then(|| {
        connection
            .query_row(
                "SELECT installation_id, cursor_hmac_key, operation_binding_hmac_key
                 FROM store_metadata WHERE id = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("read private crash-artifact metadata")
    })
}

fn metadata_identity_and_keys(root: &std::path::Path) -> (String, Vec<u8>, Vec<u8>) {
    rusqlite::Connection::open(StoreLayout::new(root).database_path())
        .expect("open fixed store metadata")
        .query_row(
            "SELECT installation_id, cursor_hmac_key, operation_binding_hmac_key
             FROM store_metadata WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("read fixed store metadata")
}

const B071_CRASH_CHILD_TEST: &str =
    "adaptor::gateway::local_event_store::tests::b071_initial_create_abrupt_crash_child";
const B071_CRASH_ROOT_ENV: &str = "RELEASH_TEST_B071_INITIAL_CREATE_CRASH_ROOT";
const B071_CRASH_BOUNDARY_ENV: &str = "RELEASH_TEST_B071_INITIAL_CREATE_CRASH_BOUNDARY";
const B071_CHILD_INSTALLATION_ID: &str = "71717171-7171-4171-8171-717171717171";
const B071_RESTART_INSTALLATION_ID: &str = "72727272-7272-4272-8272-727272727272";

fn initial_create_fault_boundaries(
) -> [crate::adaptor::gateway::local_event_store::fault::InitialCreateFaultPoint; 10] {
    use crate::adaptor::gateway::local_event_store::fault::InitialCreateFaultPoint as P;
    [
        P::BeforeEvidenceCreate,
        P::AfterPartialEvidenceWrite,
        P::AfterEvidenceFileSync,
        P::AfterEvidenceDirectorySync,
        P::AfterSqliteFileCreate,
        P::BeforeInitializationCommit,
        P::AfterInitializationCommitReplyLoss,
        P::AfterDatabaseSync,
        P::BeforeEvidenceUnlink,
        P::AfterEvidenceUnlink,
    ]
}

fn initial_create_fault_boundary_name(
    point: crate::adaptor::gateway::local_event_store::fault::InitialCreateFaultPoint,
) -> &'static str {
    use crate::adaptor::gateway::local_event_store::fault::InitialCreateFaultPoint as P;
    match point {
        P::BeforeEvidenceCreate => "before-evidence-create",
        P::AfterPartialEvidenceWrite => "after-partial-evidence-write",
        P::AfterEvidenceFileSync => "after-evidence-file-sync",
        P::AfterEvidenceDirectorySync => "after-evidence-directory-sync",
        P::AfterSqliteFileCreate => "after-sqlite-file-create",
        P::BeforeInitializationCommit => "before-initialization-commit",
        P::AfterInitializationCommitReplyLoss => "after-initialization-commit",
        P::AfterDatabaseSync => "after-database-sync",
        P::BeforeEvidenceUnlink => "before-evidence-unlink",
        P::AfterEvidenceUnlink => "after-evidence-unlink",
    }
}

fn parse_initial_create_fault_boundary(
    name: &str,
) -> crate::adaptor::gateway::local_event_store::fault::InitialCreateFaultPoint {
    initial_create_fault_boundaries()
        .into_iter()
        .find(|point| initial_create_fault_boundary_name(*point) == name)
        .unwrap_or_else(|| panic!("unknown B-071 initial-create crash boundary {name:?}"))
}

fn initial_create_boundary_has_committed_metadata(
    point: crate::adaptor::gateway::local_event_store::fault::InitialCreateFaultPoint,
) -> bool {
    use crate::adaptor::gateway::local_event_store::fault::InitialCreateFaultPoint as P;
    matches!(
        point,
        P::AfterInitializationCommitReplyLoss
            | P::AfterDatabaseSync
            | P::BeforeEvidenceUnlink
            | P::AfterEvidenceUnlink
    )
}

fn initial_create_boundary_evidence_state(
    point: crate::adaptor::gateway::local_event_store::fault::InitialCreateFaultPoint,
) -> crate::adaptor::gateway::local_event_store::layout::InitialCreateEvidenceState {
    use crate::adaptor::gateway::local_event_store::fault::InitialCreateFaultPoint as P;
    use crate::adaptor::gateway::local_event_store::layout::InitialCreateEvidenceState as E;
    match point {
        P::BeforeEvidenceCreate | P::AfterEvidenceUnlink => E::Absent,
        P::AfterPartialEvidenceWrite => E::Invalid,
        P::AfterEvidenceFileSync
        | P::AfterEvidenceDirectorySync
        | P::AfterSqliteFileCreate
        | P::BeforeInitializationCommit
        | P::AfterInitializationCommitReplyLoss
        | P::AfterDatabaseSync
        | P::BeforeEvidenceUnlink => E::Valid,
    }
}

fn initial_create_boundary_has_database(
    point: crate::adaptor::gateway::local_event_store::fault::InitialCreateFaultPoint,
) -> bool {
    use crate::adaptor::gateway::local_event_store::fault::InitialCreateFaultPoint as P;
    !matches!(
        point,
        P::BeforeEvidenceCreate
            | P::AfterPartialEvidenceWrite
            | P::AfterEvidenceFileSync
            | P::AfterEvidenceDirectorySync
    )
}

fn fault_with_initial_installation_id(installation_id: &str) -> Arc<FaultInjector> {
    let fault = Arc::new(FaultInjector::new());
    fault.set_initial_installation_id(installation_id);
    fault
}

#[tokio::test]
async fn b070_production_sqlite_lifecycle_never_references_legacy_file_store_paths() {
    let directory = tempfile::tempdir().expect("B-070 app data");
    let root = directory.path();
    let directory_roots = [
        "sessions",
        "workflow_runs",
        "workflow_logs",
        "workflow_execution_logs",
        "workflow_executions",
        "workflow_event_logs",
    ];
    let sentinel = b"\0invalid legacy bytes\nB-070 sentinel";
    for legacy in directory_roots {
        let path = root.join(legacy);
        std::fs::create_dir(&path).expect("create legacy sentinel directory");
        std::fs::write(path.join("sentinel.invalid"), sentinel).expect("write legacy sentinel");
    }
    std::fs::write(root.join("session_titles.json"), sentinel)
        .expect("write legacy title sentinel");
    let before = [
        "sessions/sentinel.invalid",
        "session_titles.json",
        "workflow_runs/sentinel.invalid",
        "workflow_logs/sentinel.invalid",
        "workflow_execution_logs/sentinel.invalid",
        "workflow_executions/sentinel.invalid",
        "workflow_event_logs/sentinel.invalid",
    ]
    .into_iter()
    .map(|relative| {
        let path = root.join(relative);
        (
            relative,
            std::fs::read(&path).expect("read initial sentinel"),
            std::fs::metadata(&path)
                .expect("read initial sentinel metadata")
                .len(),
        )
    })
    .collect::<Vec<_>>();

    let observer = Arc::new(RecordingStorePathObserver::default());
    let store = LocalEventStore::open(acceptance_store_config(
        root,
        Arc::new(FaultInjector::new()),
        observer.clone(),
    ))
    .expect("B-070 cold startup");
    let repository: Arc<dyn LocalEventTransactionRepository> = store.clone();
    let session_store = Arc::new(SessionStore::new_canonical(
        repository,
        store.installation_id().to_string(),
        Arc::new(AgentSessionProjectionCodecV1),
    ));
    let session = build_new_session_with_id(
        "b070-session".to_string(),
        "/b070-worktree",
        Some("codex".to_string()),
        crate::domain::agent_session::PermissionMode::Ask,
        None,
        false,
        false,
        None,
    );
    session_store
        .save_full_session_from_user(root, &session)
        .expect("B-070 normal production session mutation");
    assert_eq!(
        session_store
            .get_session_meta(root, "b070-session")
            .expect("B-070 canonical query")
            .expect("B-070 stored session")
            .id,
        "b070-session"
    );
    let send_gate = Arc::new(PerformanceSendGate {
        session_store: session_store.clone(),
        session_id: "b070-session".to_string(),
        effects: std::sync::atomic::AtomicUsize::new(0),
        planned: std::sync::atomic::AtomicUsize::new(0),
    });
    let send_usecase = crate::usecase::agent_session::operation::AgentSendOperationUsecase::new(
        store.clone(),
        store.clone(),
        send_gate.clone(),
        store.installation_id().to_string(),
    );
    let send = send_usecase
        .send(
            crate::usecase::agent_session::operation::SendOperationRequest {
                principal: "desktop".to_string(),
                operation_id: "b070-send".to_string(),
                canonical_payload: "{\"content\":\"B-070 production send\"}".to_string(),
            },
        )
        .await
        .expect("B-070 normal production send");
    assert!(matches!(
        send,
        crate::usecase::agent_session::operation::SendCommandOutcome::Accepted(_)
    ));
    send_usecase
        .get_operation("desktop", "b070-send")
        .await
        .expect("B-070 production send query");
    assert_eq!(
        send_gate.effects.load(std::sync::atomic::Ordering::SeqCst),
        1
    );
    let feedback_maintenance = crate::usecase::agent_session::feedback::SessionFeedbackUsecase::new(
        store.clone(),
        store.installation_id().to_string(),
    );
    assert_eq!(
        feedback_maintenance
            .recover_abandoned_reservations()
            .await
            .expect("B-070 idle feedback maintenance"),
        0
    );
    let cleanup = crate::infrastructure::process::pid_registry::cleanup_orphan_processes(root);
    assert_eq!(cleanup.failures, 0);
    session_store
        .remove_session_for_rollback(root, "b070-session")
        .expect("B-070 canonical session cleanup");

    let shutdown = ShutdownPlanKey {
        shutdown_id: "b070-shutdown".to_string(),
    };
    let mut install_shutdown = batch(
        "b070-shutdown-commit",
        "b070-shutdown-key",
        [71; 32],
        Vec::new(),
        Vec::new(),
        vec![
            LocalStateMutation::ShutdownPlan(ShutdownPlanMutation {
                key: shutdown.clone(),
                phase: ApplicationShutdownPhase::Prepared,
                summary: shutdown_plan_fixture("b070-quit"),
                details_state: ShutdownDetailsState::Available,
                expected: RevisionGuard::Absent,
                revision: Revision::new(0).unwrap(),
            }),
            LocalStateMutation::ShutdownLatestPointer(ShutdownLatestPointerMutation {
                expected: None,
                new: Some(shutdown),
            }),
        ],
    );
    install_shutdown.idempotency.installation_id = store.installation_id().to_string();
    install_shutdown.idempotency.operation_kind = CommitOperationKind::ApplicationQuit;
    store
        .commit_batch(install_shutdown)
        .await
        .expect("B-070 graceful shutdown persistence");
    drop(feedback_maintenance);
    drop(send_usecase);
    drop(send_gate);
    drop(session_store);
    drop(store);
    let reopened = LocalEventStore::open(acceptance_store_config(
        root,
        Arc::new(FaultInjector::new()),
        observer.clone(),
    ))
    .expect("B-070 restart");
    assert!(matches!(
        reopened.query(LocalEventQuery::CurrentShutdown).await,
        Ok(LocalEventQueryResult::CurrentShutdown(Some(_)))
    ));
    drop(reopened);

    let legacy_paths = [
        root.join("sessions"),
        root.join("session_titles.json"),
        root.join("workflow_runs"),
        root.join("workflow_logs"),
        root.join("workflow_execution_logs"),
        root.join("workflow_executions"),
        root.join("workflow_event_logs"),
    ];
    let observed = observer.operations.lock().expect("path observations");
    assert!(
        observed.iter().all(|(_, path)| {
            legacy_paths
                .iter()
                .all(|legacy| path != legacy && !path.starts_with(legacy))
        }),
        "production fixed-store lifecycle accessed a legacy path: {observed:?}"
    );
    for (relative, bytes, length) in before {
        let path = root.join(relative);
        assert_eq!(std::fs::read(&path).expect("read final sentinel"), bytes);
        assert_eq!(
            std::fs::metadata(path)
                .expect("read final sentinel metadata")
                .len(),
            length
        );
    }
}

#[test]
fn b071_initial_create_cases_a_b_and_c_converge_without_replacing_ready_identity() {
    let case_a = tempfile::tempdir().expect("B-071 Case A app data");
    let root = case_a.path();
    let layout = StoreLayout::new(root);

    std::fs::write(layout.initial_create_evidence_path(), b"partial")
        .expect("write partial evidence");
    let store = LocalEventStore::open(acceptance_store_config(
        root,
        Arc::new(FaultInjector::new()),
        noop_path_observer(),
    ))
    .expect("Case A repairs evidence only while database is absent");
    drop(store);
    assert!(!layout.initial_create_evidence_path().exists());

    let case_b = tempfile::tempdir().expect("B-071 Case B app data");
    let case_b_layout = StoreLayout::new(case_b.path());
    crate::adaptor::gateway::local_event_store::layout::create_initial_create_evidence(
        &case_b_layout,
    )
    .expect("create valid evidence");
    std::fs::write(case_b_layout.database_path(), []).expect("create zero-byte residue");
    let store = LocalEventStore::open(acceptance_store_config(
        case_b.path(),
        Arc::new(FaultInjector::new()),
        noop_path_observer(),
    ))
    .expect("Case B retries a proven zero-byte first-create residue");
    let after_residue = metadata_identity_and_keys(case_b.path());
    drop(store);

    crate::adaptor::gateway::local_event_store::layout::create_initial_create_evidence(
        &case_b_layout,
    )
    .expect("create stale valid evidence");
    let reopened = LocalEventStore::open(acceptance_store_config(
        case_b.path(),
        Arc::new(FaultInjector::new()),
        noop_path_observer(),
    ))
    .expect("Case C opens the ready store");
    assert_eq!(metadata_identity_and_keys(case_b.path()), after_residue);
    assert_eq!(reopened.installation_id(), after_residue.0);
    assert!(!case_b_layout.initial_create_evidence_path().exists());
}

#[test]
fn b071_valid_evidence_does_not_authorize_deleting_an_unrelated_user_table() {
    let directory = tempfile::tempdir().expect("B-071 unrelated-table app data");
    let root = directory.path();
    let layout = StoreLayout::new(root);
    crate::adaptor::gateway::local_event_store::layout::create_initial_create_evidence(&layout)
        .expect("create valid evidence");
    rusqlite::Connection::open(layout.database_path())
        .expect("create tableless SQLite")
        .execute_batch("CREATE TABLE unrelated(value TEXT);")
        .expect("create unrelated user table");
    let before = fixed_store_files(root);
    let evidence_before =
        std::fs::read(layout.initial_create_evidence_path()).expect("read valid evidence");

    assert!(matches!(
        LocalEventStore::open(acceptance_store_config(
            root,
            Arc::new(FaultInjector::new()),
            noop_path_observer(),
        )),
        Err(super::store::LocalEventStoreOpenError::InitializationStateInvalid)
    ));
    assert_eq!(fixed_store_files(root), before);
    assert_eq!(
        std::fs::read(layout.initial_create_evidence_path()).expect("evidence must remain"),
        evidence_before
    );
}

#[test]
fn b071_evidenced_database_with_an_application_table_is_not_initial_create_residue() {
    let directory = tempfile::tempdir().expect("B-071 application-table app data");
    let root = directory.path();
    let layout = StoreLayout::new(root);
    crate::adaptor::gateway::local_event_store::layout::create_initial_create_evidence(&layout)
        .expect("create valid initial-create evidence");
    let fixture_connection = rusqlite::Connection::open(layout.database_path())
        .expect("create incomplete application SQLite");
    fixture_connection
        .set_db_config(
            rusqlite::config::DbConfig::SQLITE_DBCONFIG_NO_CKPT_ON_CLOSE,
            true,
        )
        .expect("preserve application-table WAL after fixture close");
    fixture_connection
        .execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA wal_autocheckpoint = 0;
             CREATE TABLE events (
                 global_sequence INTEGER PRIMARY KEY,
                 payload BLOB NOT NULL
             );
             INSERT INTO events(global_sequence, payload) VALUES (1, X'01020304');",
        )
        .expect("leave an application table in an uncheckpointed WAL");
    drop(fixture_connection);
    let before = fixed_store_files(root);
    assert!(
        before.iter().all(|(_, bytes)| bytes.is_some()),
        "the strong fixture must contain the database, WAL, and SHM"
    );
    let evidence_before =
        std::fs::read(layout.initial_create_evidence_path()).expect("read valid evidence");

    assert!(matches!(
        LocalEventStore::open(acceptance_store_config(
            root,
            Arc::new(FaultInjector::new()),
            noop_path_observer(),
        )),
        Err(super::store::LocalEventStoreOpenError::InitializationStateInvalid)
    ));
    assert_eq!(
        fixed_store_files(root),
        before,
        "the database, WAL, or SHM changed while failing closed"
    );
    assert_eq!(
        std::fs::read(layout.initial_create_evidence_path()).expect("evidence must remain"),
        evidence_before,
        "valid evidence is not authority to delete an application table"
    );
}

#[test]
fn b071_ready_store_with_nonempty_wal_never_uses_a_database_outside_fixed_layout() {
    let directory = tempfile::tempdir().expect("B-071 fixed-layout ready-store app data");
    let root = directory.path();
    let store = LocalEventStore::open(acceptance_store_config(
        root,
        Arc::new(FaultInjector::new()),
        noop_path_observer(),
    ))
    .expect("initialize B-071 fixed-layout ready store");
    let installation_id = store.installation_id().to_string();
    drop(store);

    let _wal_path = leave_nonempty_wal(
        root,
        "UPDATE store_metadata
            SET process_instance_id = '71717171-7171-4171-8171-717171717171'
          WHERE id = 1;",
    );
    let observer = Arc::new(FixedLayoutStartupObserver::new(root));
    let reopened = LocalEventStore::open(acceptance_store_config(
        root,
        Arc::new(FaultInjector::new()),
        observer.clone(),
    ))
    .expect("open a ready fixed store whose committed state includes a non-empty WAL");

    assert_eq!(reopened.installation_id(), installation_id);
    observer.assert_fixed_layout_only(root);
}

#[test]
fn b071_every_initial_create_crash_boundary_converges_to_one_ready_identity_and_key_set() {
    use crate::adaptor::gateway::local_event_store::fault::InitialCreateFaultPoint as P;

    for boundary in initial_create_fault_boundaries() {
        let directory = tempfile::tempdir().expect("B-071 initial-create boundary fixture");
        let root = directory.path();
        let layout = StoreLayout::new(root);
        let fault = Arc::new(FaultInjector::new());
        fault.arm_initial_create_fault(boundary);
        assert!(
            LocalEventStore::open(acceptance_store_config(root, fault, noop_path_observer(),))
                .is_err(),
            "B-071 {boundary:?} did not stop the real startup path"
        );
        let committed_before_restart = matches!(
            boundary,
            P::AfterInitializationCommitReplyLoss
                | P::AfterDatabaseSync
                | P::BeforeEvidenceUnlink
                | P::AfterEvidenceUnlink
        )
        .then(|| metadata_identity_and_keys(root));

        let store = LocalEventStore::open(acceptance_store_config(
            root,
            Arc::new(FaultInjector::new()),
            noop_path_observer(),
        ))
        .unwrap_or_else(|error| panic!("B-071 {boundary:?} did not converge: {error}"));
        let identity_and_keys = metadata_identity_and_keys(root);
        assert_eq!(store.installation_id(), identity_and_keys.0);
        if let Some(expected) = committed_before_restart {
            assert_eq!(
                identity_and_keys, expected,
                "B-071 {boundary:?} replaced a ready identity or key"
            );
        }
        assert!(
            !layout.initial_create_evidence_path().exists(),
            "B-071 {boundary:?} left initial-create evidence after Ready"
        );
        drop(store);

        let reopened = LocalEventStore::open(acceptance_store_config(
            root,
            Arc::new(FaultInjector::new()),
            noop_path_observer(),
        ))
        .expect("B-071 converged database must reopen");
        assert_eq!(metadata_identity_and_keys(root), identity_and_keys);
        assert_eq!(reopened.installation_id(), identity_and_keys.0);
    }
}

#[test]
fn b071_initial_create_abrupt_crash_child() {
    let Some(root) = std::env::var_os(B071_CRASH_ROOT_ENV) else {
        // Ordinary test-suite execution runs the fixture as a harmless no-op.
        // The parent acceptance test invokes this exact test with both vars.
        return;
    };
    let boundary_name =
        std::env::var(B071_CRASH_BOUNDARY_ENV).expect("B-071 child boundary environment");
    let boundary = parse_initial_create_fault_boundary(&boundary_name);
    let fault = fault_with_initial_installation_id(B071_CHILD_INSTALLATION_ID);
    fault.arm_initial_create_process_crash(boundary);

    match LocalEventStore::open(acceptance_store_config(
        std::path::Path::new(&root),
        fault,
        noop_path_observer(),
    )) {
        Ok(_) => panic!("B-071 {boundary:?} returned Ready instead of aborting"),
        Err(error) => {
            panic!("B-071 {boundary:?} returned {error} instead of abruptly terminating")
        }
    }
}

#[test]
fn b071_every_initial_create_boundary_recovers_after_abrupt_process_loss() {
    use crate::adaptor::gateway::local_event_store::layout::{
        inspect_initial_create_evidence, InitialCreateEvidenceState,
    };

    for boundary in initial_create_fault_boundaries() {
        let directory = tempfile::tempdir().expect("B-071 abrupt crash fixture");
        let root = directory.path();
        let output = std::process::Command::new(
            std::env::current_exe().expect("resolve current unit-test executable"),
        )
        .arg("--exact")
        .arg(B071_CRASH_CHILD_TEST)
        .arg("--nocapture")
        .env(B071_CRASH_ROOT_ENV, root)
        .env(
            B071_CRASH_BOUNDARY_ENV,
            initial_create_fault_boundary_name(boundary),
        )
        .output()
        .unwrap_or_else(|error| panic!("spawn B-071 {boundary:?} crash child: {error}"));

        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            assert!(
                output.status.signal().is_some(),
                "B-071 {boundary:?} child was not killed by an abrupt signal: status={:?}, \
                 stdout={}, stderr={}",
                output.status,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        #[cfg(not(unix))]
        assert!(
            !output.status.success() && output.status.code() != Some(101),
            "B-071 {boundary:?} child did not terminate abruptly: status={:?}, stdout={}, stderr={}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        // Capture every fixed SQLite artifact and the evidence file before any
        // parent SQLite connection is opened.
        let crashed = initial_create_artifacts(root);
        assert_eq!(
            crashed.database.is_some(),
            initial_create_boundary_has_database(boundary),
            "B-071 {boundary:?} database-presence crash oracle"
        );
        assert!(
            crashed.database.is_some() || (crashed.wal.is_none() && crashed.shm.is_none()),
            "B-071 {boundary:?} left a sidecar without its fixed database: {crashed:?}"
        );
        assert!(
            crashed.wal.is_some() || crashed.shm.is_none(),
            "B-071 {boundary:?} left SHM without WAL: {crashed:?}"
        );
        assert_eq!(
            inspect_initial_create_evidence(&StoreLayout::new(root))
                .expect("inspect abrupt-crash evidence"),
            initial_create_boundary_evidence_state(boundary),
            "B-071 {boundary:?} evidence crash oracle"
        );
        if initial_create_boundary_evidence_state(boundary) == InitialCreateEvidenceState::Invalid {
            assert!(
                crashed
                    .evidence
                    .as_ref()
                    .is_some_and(|bytes| !bytes.is_empty()),
                "B-071 {boundary:?} must leave the written partial evidence"
            );
        }

        let committed_before_restart = metadata_identity_and_keys_from_artifacts(&crashed);
        assert_eq!(
            committed_before_restart.is_some(),
            initial_create_boundary_has_committed_metadata(boundary),
            "B-071 {boundary:?} commit-state crash oracle"
        );
        if let Some((installation_id, cursor_key, binding_key)) = &committed_before_restart {
            assert_eq!(installation_id, B071_CHILD_INSTALLATION_ID);
            assert_eq!(cursor_key.len(), 32);
            assert_eq!(binding_key.len(), 32);
        }
        assert_eq!(
            initial_create_artifacts(root),
            crashed,
            "B-071 {boundary:?} parent crash-oracle inspection mutated DB/WAL/SHM/evidence"
        );

        // Offer a different identity on restart. Pre-COMMIT artifacts must be
        // discarded and use it; post-COMMIT artifacts must preserve the exact
        // child identity and HMAC keys.
        let store = LocalEventStore::open(acceptance_store_config(
            root,
            fault_with_initial_installation_id(B071_RESTART_INSTALLATION_ID),
            noop_path_observer(),
        ))
        .unwrap_or_else(|error| {
            panic!("B-071 {boundary:?} did not converge after abrupt crash: {error}")
        });
        let ready_metadata = metadata_identity_and_keys(root);
        let expected_installation_id = if committed_before_restart.is_some() {
            B071_CHILD_INSTALLATION_ID
        } else {
            B071_RESTART_INSTALLATION_ID
        };
        assert_eq!(store.installation_id(), expected_installation_id);
        assert_eq!(ready_metadata.0, expected_installation_id);
        if let Some(committed) = committed_before_restart {
            assert_eq!(
                ready_metadata, committed,
                "B-071 {boundary:?} replaced committed identity or HMAC keys"
            );
        }
        let ready_artifacts = initial_create_artifacts(root);
        assert!(ready_artifacts.database.is_some());
        assert!(
            ready_artifacts.wal.is_some(),
            "B-071 {boundary:?} Ready writer must own the WAL"
        );
        assert!(
            ready_artifacts.shm.is_some(),
            "B-071 {boundary:?} Ready writer must own the SHM"
        );
        assert!(
            ready_artifacts.evidence.is_none(),
            "B-071 {boundary:?} Ready must remove initial-create evidence"
        );
        drop(store);

        let reopened = LocalEventStore::open(acceptance_store_config(
            root,
            fault_with_initial_installation_id("73737373-7373-4373-8373-737373737373"),
            noop_path_observer(),
        ))
        .unwrap_or_else(|error| panic!("B-071 {boundary:?} second restart failed: {error}"));
        assert_eq!(metadata_identity_and_keys(root), ready_metadata);
        assert_eq!(reopened.installation_id(), expected_installation_id);
    }
}

#[tokio::test]
async fn b071_drop_joins_every_sqlite_worker_before_immediate_reopen() {
    let directory = tempfile::tempdir().expect("B-071 worker lifetime app data");
    let root = directory.path();
    let mut installation_id = None;

    for ordinal in 0..16 {
        let store = LocalEventStore::open(acceptance_store_config(
            root,
            Arc::new(FaultInjector::new()),
            noop_path_observer(),
        ))
        .unwrap_or_else(|error| panic!("immediate reopen {ordinal} failed: {error}"));
        if let Some(expected) = &installation_id {
            assert_eq!(store.installation_id(), expected);
        } else {
            installation_id = Some(store.installation_id().to_string());
        }

        let obligation_id = format!("b071-worker-lifetime-{ordinal:02}");
        let mut seed = batch(
            &format!("commit-{obligation_id}"),
            &format!("key-{obligation_id}"),
            [ordinal as u8; 32],
            vec![],
            vec![],
            vec![obligation_mutation(&obligation_id, true)],
        );
        seed.idempotency.installation_id = store.installation_id().to_string();
        store.commit_batch(seed).await.expect("seed worker fixture");

        // A non-empty first page keeps a recovery-snapshot SQLite worker
        // alive. Drop must join it together with the writer and all readers
        // before releasing the process writer lock.
        assert!(matches!(
            store
                .query(LocalEventQuery::PendingRecoveryPage {
                    limit: 1,
                    partition: None,
                    owner: None,
                    ordered_key_prefix: None,
                    shutdown_plan: None,
                    cursor: None,
                })
                .await,
            Ok(LocalEventQueryResult::PendingRecoveryPage(_))
        ));
        drop(store);
    }
}

#[test]
fn b071_cases_d_and_e_fail_closed_without_mutating_fixed_database_or_sidecars() {
    let zero_directory = tempfile::tempdir().expect("B-071 zero file");
    let zero_layout = StoreLayout::new(zero_directory.path());
    std::fs::write(zero_layout.database_path(), []).expect("create ambiguous zero file");
    let zero_before = fixed_store_files(zero_directory.path());
    assert!(matches!(
        LocalEventStore::open(acceptance_store_config(
            zero_directory.path(),
            Arc::new(FaultInjector::new()),
            noop_path_observer(),
        )),
        Err(super::store::LocalEventStoreOpenError::InitializationStateInvalid)
    ));
    assert_eq!(fixed_store_files(zero_directory.path()), zero_before);

    let unrelated_directory = tempfile::tempdir().expect("B-071 unrelated SQLite");
    let unrelated_layout = StoreLayout::new(unrelated_directory.path());
    rusqlite::Connection::open(unrelated_layout.database_path())
        .expect("create unrelated SQLite")
        .execute_batch("CREATE TABLE unrelated(value TEXT);")
        .expect("initialize unrelated SQLite");
    let unrelated_before = fixed_store_files(unrelated_directory.path());
    assert!(matches!(
        LocalEventStore::open(acceptance_store_config(
            unrelated_directory.path(),
            Arc::new(FaultInjector::new()),
            noop_path_observer(),
        )),
        Err(super::store::LocalEventStoreOpenError::InitializationStateInvalid)
    ));
    assert_eq!(
        fixed_store_files(unrelated_directory.path()),
        unrelated_before
    );

    let unsupported_directory = tempfile::tempdir().expect("B-071 unsupported schema");
    let unsupported_root = unsupported_directory.path();
    let store = LocalEventStore::open(acceptance_store_config(
        unsupported_root,
        Arc::new(FaultInjector::new()),
        noop_path_observer(),
    ))
    .expect("seed recognized store");
    drop(store);
    let unsupported_connection =
        rusqlite::Connection::open(StoreLayout::new(unsupported_root).database_path())
            .expect("open recognized store fixture");
    unsupported_connection
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE); PRAGMA journal_mode=DELETE;")
        .expect("stabilize unsupported-version fixture");
    unsupported_connection
        .pragma_update(None, "user_version", 99_i64)
        .expect("set unsupported version");
    drop(unsupported_connection);
    let unsupported_before = fixed_store_files(unsupported_root);
    assert!(matches!(
        LocalEventStore::open(acceptance_store_config(
            unsupported_root,
            Arc::new(FaultInjector::new()),
            noop_path_observer(),
        )),
        Err(super::store::LocalEventStoreOpenError::UnsupportedStoreVersion)
    ));
    assert_eq!(fixed_store_files(unsupported_root), unsupported_before);

    let invalid_directory = tempfile::tempdir().expect("B-071 invalid current store");
    let invalid_root = invalid_directory.path();
    let store = LocalEventStore::open(acceptance_store_config(
        invalid_root,
        Arc::new(FaultInjector::new()),
        noop_path_observer(),
    ))
    .expect("seed current store");
    drop(store);
    let invalid_connection =
        rusqlite::Connection::open(StoreLayout::new(invalid_root).database_path())
            .expect("open current fixture");
    invalid_connection
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE); PRAGMA journal_mode=DELETE;")
        .expect("stabilize invalid-schema fixture");
    invalid_connection
        .execute("DROP INDEX idx_pending_obligations_owner", [])
        .expect("remove required index");
    drop(invalid_connection);
    let invalid_before = fixed_store_files(invalid_root);
    assert!(matches!(
        LocalEventStore::open(acceptance_store_config(
            invalid_root,
            Arc::new(FaultInjector::new()),
            noop_path_observer(),
        )),
        Err(super::store::LocalEventStoreOpenError::StoreValidationFailed)
    ));
    assert_eq!(fixed_store_files(invalid_root), invalid_before);
}

#[tokio::test]
async fn b098_distinct_store_authorities_reject_cross_installation_batches() {
    let first_directory = tempfile::tempdir().expect("B-098 first installation");
    let second_directory = tempfile::tempdir().expect("B-098 second installation");
    let first = LocalEventStore::open(acceptance_store_config(
        first_directory.path(),
        Arc::new(FaultInjector::new()),
        noop_path_observer(),
    ))
    .expect("first installation");
    let second = LocalEventStore::open(acceptance_store_config(
        second_directory.path(),
        Arc::new(FaultInjector::new()),
        noop_path_observer(),
    ))
    .expect("second installation");
    assert_ne!(first.installation_id(), second.installation_id());

    let mut foreign = batch(
        "commit-b098-foreign-installation",
        "key-b098-foreign-installation",
        [98; 32],
        vec![],
        vec![],
        vec![obligation_mutation("b098-foreign-installation", true)],
    );
    foreign.idempotency.installation_id = first.installation_id().to_string();
    assert!(matches!(
        second.commit_batch(foreign).await,
        Err(CommitBatchError::Corrupt { .. })
    ));
    let second_connection =
        rusqlite::Connection::open(StoreLayout::new(second_directory.path()).database_path())
            .expect("inspect second installation");
    let commit_count: i64 = second_connection
        .query_row("SELECT COUNT(*) FROM logical_commits", [], |row| row.get(0))
        .expect("count second installation commits");
    assert_eq!(commit_count, 0);
}

#[derive(Debug, Clone, Copy)]
enum B071ReadyStoreCorruption {
    Header,
    ApplicationId,
    SchemaVersion,
    MetadataRow,
    InstallationIdentity,
    CursorHmacKey,
    OperationBindingHmacKey,
    RequiredIndex,
}

fn corrupt_b071_ready_store(root: &std::path::Path, corruption: B071ReadyStoreCorruption) {
    let database = StoreLayout::new(root).database_path();
    if matches!(corruption, B071ReadyStoreCorruption::Header) {
        use std::io::{Seek, Write};

        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .open(database)
            .expect("open B-071 header fixture");
        file.seek(std::io::SeekFrom::Start(0))
            .expect("seek B-071 header fixture");
        file.write_all(b"X").expect("corrupt B-071 SQLite header");
        file.sync_all().expect("sync B-071 header corruption");
        return;
    }

    let connection = rusqlite::Connection::open(database).expect("open B-071 ready fixture");
    match corruption {
        B071ReadyStoreCorruption::Header => unreachable!(),
        B071ReadyStoreCorruption::ApplicationId => connection
            .pragma_update(None, "application_id", 0_i64)
            .expect("corrupt B-071 application id"),
        B071ReadyStoreCorruption::SchemaVersion => connection
            .pragma_update(None, "user_version", 99_i64)
            .expect("corrupt B-071 schema version"),
        B071ReadyStoreCorruption::MetadataRow => {
            connection
                .execute("DELETE FROM store_metadata", [])
                .expect("remove B-071 metadata row");
        }
        B071ReadyStoreCorruption::InstallationIdentity => {
            connection
                .execute(
                    "UPDATE store_metadata SET installation_id = 'not-a-uuid' WHERE id = 1",
                    [],
                )
                .expect("corrupt B-071 installation identity");
        }
        B071ReadyStoreCorruption::CursorHmacKey => connection
            .execute_batch(
                "PRAGMA ignore_check_constraints = ON;
                 UPDATE store_metadata SET cursor_hmac_key = X'00' WHERE id = 1;",
            )
            .expect("corrupt B-071 cursor HMAC key"),
        B071ReadyStoreCorruption::OperationBindingHmacKey => connection
            .execute_batch(
                "PRAGMA ignore_check_constraints = ON;
                 UPDATE store_metadata
                    SET operation_binding_hmac_key = X'00' WHERE id = 1;",
            )
            .expect("corrupt B-071 operation-binding HMAC key"),
        B071ReadyStoreCorruption::RequiredIndex => {
            connection
                .execute("DROP INDEX idx_pending_obligations_owner", [])
                .expect("remove B-071 required index");
        }
    }
}

#[test]
fn b071_every_initialized_store_corruption_fails_closed_without_mutation() {
    use super::store::LocalEventStoreOpenError as E;

    for (corruption, expected) in [
        (
            B071ReadyStoreCorruption::Header,
            E::InitializationStateInvalid,
        ),
        (
            B071ReadyStoreCorruption::ApplicationId,
            E::StoreValidationFailed,
        ),
        (
            B071ReadyStoreCorruption::SchemaVersion,
            E::UnsupportedStoreVersion,
        ),
        (
            B071ReadyStoreCorruption::MetadataRow,
            E::StoreValidationFailed,
        ),
        (
            B071ReadyStoreCorruption::InstallationIdentity,
            E::StoreValidationFailed,
        ),
        (
            B071ReadyStoreCorruption::CursorHmacKey,
            E::StoreValidationFailed,
        ),
        (
            B071ReadyStoreCorruption::OperationBindingHmacKey,
            E::StoreValidationFailed,
        ),
        (
            B071ReadyStoreCorruption::RequiredIndex,
            E::StoreValidationFailed,
        ),
    ] {
        let directory = tempfile::tempdir().expect("B-071 initialized fixture");
        let root = directory.path();
        let store = LocalEventStore::open(acceptance_store_config(
            root,
            Arc::new(FaultInjector::new()),
            noop_path_observer(),
        ))
        .expect("seed B-071 initialized fixture");
        drop(store);
        rusqlite::Connection::open(StoreLayout::new(root).database_path())
            .expect("open B-071 fixture for checkpoint")
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE); PRAGMA journal_mode=DELETE;")
            .expect("stabilize B-071 initialized fixture");
        corrupt_b071_ready_store(root, corruption);
        let before = fixed_store_files(root);
        let error = match LocalEventStore::open(acceptance_store_config(
            root,
            Arc::new(FaultInjector::new()),
            noop_path_observer(),
        )) {
            Ok(_) => panic!("B-071 {corruption:?} unexpectedly opened"),
            Err(error) => error,
        };
        assert_eq!(error, expected, "B-071 {corruption:?} classification");
        assert_eq!(
            fixed_store_files(root),
            before,
            "B-071 {corruption:?} changed the fixed database or a sidecar"
        );
    }
}

#[test]
fn b071_writer_lock_is_nonblocking_and_maps_to_store_in_use() {
    let directory = tempfile::tempdir().expect("B-071 writer lock");
    let first = LocalEventStore::open(acceptance_store_config(
        directory.path(),
        Arc::new(FaultInjector::new()),
        noop_path_observer(),
    ))
    .expect("first writer");
    let started = std::time::Instant::now();
    assert!(matches!(
        LocalEventStore::open(acceptance_store_config(
            directory.path(),
            Arc::new(FaultInjector::new()),
            noop_path_observer(),
        )),
        Err(super::store::LocalEventStoreOpenError::WriterLockHeld)
    ));
    assert!(
        started.elapsed() < std::time::Duration::from_secs(1),
        "writer lock acquisition must not wait"
    );
    drop(first);
}

#[test]
fn b071_lock_path_filesystem_failure_is_storage_unavailable_not_store_in_use() {
    let directory = tempfile::tempdir().expect("B-071 lock filesystem failure");
    let layout = StoreLayout::new(directory.path());
    std::fs::create_dir(layout.writer_lock_path()).expect("occupy lock path with a directory");
    assert!(matches!(
        LocalEventStore::open(acceptance_store_config(
            directory.path(),
            Arc::new(FaultInjector::new()),
            noop_path_observer(),
        )),
        Err(super::store::LocalEventStoreOpenError::StorageUnavailable)
    ));
}

#[test]
fn b071_recognized_v1_version_gap_is_unsupported_without_mutation() {
    let directory = tempfile::tempdir().expect("B-071 v1 version gap");
    let root = directory.path();
    let store = LocalEventStore::open(acceptance_store_config(
        root,
        Arc::new(FaultInjector::new()),
        noop_path_observer(),
    ))
    .expect("seed current store before v1 downgrade");
    drop(store);
    downgrade_current_store_to_supported_v1(root);
    let connection =
        rusqlite::Connection::open(StoreLayout::new(root).database_path()).expect("open v1 store");
    connection
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE); PRAGMA journal_mode=DELETE;")
        .expect("stabilize unsupported v1 fixture");
    connection
        .pragma_update(None, "user_version", 99_i64)
        .expect("set unsupported v1 version");
    drop(connection);
    let before = fixed_store_files(root);

    assert!(matches!(
        LocalEventStore::open(acceptance_store_config(
            root,
            Arc::new(FaultInjector::new()),
            noop_path_observer(),
        )),
        Err(super::store::LocalEventStoreOpenError::UnsupportedStoreVersion)
    ));
    assert_eq!(fixed_store_files(root), before);
}

fn downgrade_current_store_to_supported_v1(root: &std::path::Path) -> (String, String) {
    let connection =
        rusqlite::Connection::open(StoreLayout::new(root).database_path()).expect("open B-098 DB");
    connection
        .execute_batch(
            "PRAGMA foreign_keys = OFF;
             BEGIN IMMEDIATE;
             ALTER TABLE logical_commits RENAME COLUMN installation_id TO generation_id;
             ALTER TABLE operation_bindings RENAME COLUMN installation_id TO generation_id;
             ALTER TABLE caller_attempts RENAME COLUMN installation_id TO generation_id;
             DROP INDEX idx_caller_attempts_scope;
             DROP INDEX idx_caller_attempts_pending_kind;
             DROP INDEX idx_operation_bindings_operation;
             CREATE TABLE shutdown_plans_v1 (
                 plan_id TEXT NOT NULL,
                 epoch INTEGER NOT NULL CHECK (epoch >= 0),
                 phase TEXT NOT NULL,
                 summary TEXT NOT NULL,
                 details_state TEXT NOT NULL,
                 revision INTEGER NOT NULL CHECK (revision >= 0),
                 commit_id TEXT NOT NULL REFERENCES logical_commits (commit_id),
                 PRIMARY KEY (plan_id, epoch)
             );
             INSERT INTO shutdown_plans_v1
             SELECT shutdown_id, 0, phase, summary, details_state, revision, commit_id
             FROM shutdown_plans;
             CREATE TABLE shutdown_targets_v1 (
                 plan_id TEXT NOT NULL,
                 epoch INTEGER NOT NULL,
                 ordinal INTEGER NOT NULL,
                 detail TEXT NOT NULL,
                 revision INTEGER NOT NULL,
                 commit_id TEXT NOT NULL REFERENCES logical_commits (commit_id),
                 PRIMARY KEY (plan_id, epoch, ordinal)
             );
             INSERT INTO shutdown_targets_v1
             SELECT shutdown_id, 0, ordinal, detail, revision, commit_id
             FROM shutdown_targets;
             CREATE TABLE shutdown_recovery_snapshots_v1 (
                 plan_id TEXT NOT NULL,
                 epoch INTEGER NOT NULL,
                 partition TEXT NOT NULL,
                 ordinal INTEGER NOT NULL,
                 detail TEXT NOT NULL,
                 commit_id TEXT NOT NULL REFERENCES logical_commits (commit_id),
                 PRIMARY KEY (plan_id, epoch, ordinal)
             );
             INSERT INTO shutdown_recovery_snapshots_v1
             SELECT shutdown_id, 0, partition, ordinal, detail, commit_id
             FROM shutdown_recovery_snapshots;
             CREATE TABLE pending_obligations_v1 (
                 ordered_key TEXT PRIMARY KEY,
                 obligation_id TEXT NOT NULL UNIQUE REFERENCES obligations (obligation_id),
                 owner TEXT NOT NULL,
                 partition TEXT NOT NULL,
                 shutdown_plan_id TEXT,
                 shutdown_epoch INTEGER,
                 commit_id TEXT NOT NULL REFERENCES logical_commits (commit_id)
             );
             INSERT INTO pending_obligations_v1
             SELECT ordered_key, obligation_id, owner, partition, shutdown_id,
                    CASE WHEN shutdown_id IS NULL THEN NULL ELSE 0 END, commit_id
             FROM pending_obligations;
             CREATE TABLE store_metadata_v1 (
                 id INTEGER PRIMARY KEY CHECK (id = 1),
                 schema_version INTEGER NOT NULL CHECK (schema_version = 1),
                 store_id TEXT NOT NULL,
                 generation_id TEXT NOT NULL,
                 created_at_ms INTEGER NOT NULL,
                 cursor_hmac_key BLOB NOT NULL,
                 operation_binding_hmac_key BLOB NOT NULL,
                 boot_id TEXT NOT NULL,
                 next_global_sequence INTEGER NOT NULL,
                 health TEXT NOT NULL,
                 current_shutdown_plan_id TEXT,
                 current_shutdown_epoch INTEGER,
                 shutdown_pointer_revision INTEGER NOT NULL,
                 retiring_shutdown_plan_id TEXT,
                 retiring_shutdown_epoch INTEGER,
                 shutdown_retiring_revision INTEGER NOT NULL DEFAULT 0
             );
             INSERT INTO store_metadata_v1 (
                 id, schema_version, store_id, generation_id, created_at_ms,
                 cursor_hmac_key, operation_binding_hmac_key, boot_id,
                 next_global_sequence, health, current_shutdown_plan_id,
                 current_shutdown_epoch, shutdown_pointer_revision,
                 retiring_shutdown_plan_id, retiring_shutdown_epoch,
                 shutdown_retiring_revision
             )
             SELECT id, 1, installation_id, installation_id, created_at_ms,
                    cursor_hmac_key, operation_binding_hmac_key, process_instance_id,
                    next_global_sequence, health, current_shutdown_id,
                    CASE WHEN current_shutdown_id IS NULL THEN NULL ELSE 0 END,
                    shutdown_pointer_revision, NULL, NULL, 0
             FROM store_metadata;
             DROP TABLE store_metadata;
             DROP TABLE pending_obligations;
             DROP TABLE shutdown_recovery_snapshots;
             DROP TABLE shutdown_targets;
             DROP TABLE shutdown_plans;
             ALTER TABLE shutdown_plans_v1 RENAME TO shutdown_plans;
             ALTER TABLE shutdown_targets_v1 RENAME TO shutdown_targets;
             ALTER TABLE shutdown_recovery_snapshots_v1
                 RENAME TO shutdown_recovery_snapshots;
             ALTER TABLE pending_obligations_v1 RENAME TO pending_obligations;
             ALTER TABLE store_metadata_v1 RENAME TO store_metadata;
             CREATE INDEX idx_caller_attempts_scope
                 ON caller_attempts (principal, generation_id, scope_id, kind, caller_request_id);
             CREATE INDEX idx_caller_attempts_pending_kind
                 ON caller_attempts (generation_id, kind, resolution, principal, caller_request_id);
             CREATE INDEX idx_operation_bindings_operation
                 ON operation_bindings (generation_id, kind, operation_id, principal, caller_request_id);
             CREATE TABLE local_store_migrations (migration_id TEXT PRIMARY KEY);
             CREATE TABLE legacy_source_inventory (source_path TEXT PRIMARY KEY);
             CREATE TABLE legacy_raw_records (record_id TEXT PRIMARY KEY);
             CREATE TABLE legacy_raw_record_chunks (chunk_id TEXT PRIMARY KEY);
             CREATE TABLE migration_quit_flights (flight_id TEXT PRIMARY KEY);
             CREATE TABLE shutdown_compact_archives (archive_id TEXT PRIMARY KEY);
             PRAGMA application_id = 0;
             PRAGMA user_version = 0;
             COMMIT;
             PRAGMA foreign_keys = ON;",
        )
        .expect("downgrade fixture to the supported v1 schema");
    let legacy_store_id = "11111111-1111-4111-8111-111111111111";
    connection
        .execute(
            "UPDATE store_metadata SET store_id = ?1 WHERE id = 1",
            [legacy_store_id],
        )
        .expect("make the old physical store identity differ from its operation generation");
    let identities = connection
        .query_row(
            "SELECT store_id, generation_id FROM store_metadata WHERE id = 1",
            [],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .expect("read distinct supported-v1 identities");
    assert_ne!(
        identities.0, identities.1,
        "the B-098 fixture must exercise a real split identity"
    );
    identities
}

#[test]
fn b098_supported_v1_evolution_with_nonempty_wal_never_uses_a_database_outside_fixed_layout() {
    let directory = tempfile::tempdir().expect("B-098 fixed-layout evolution app data");
    let root = directory.path();
    let store = LocalEventStore::open(acceptance_store_config(
        root,
        Arc::new(FaultInjector::new()),
        noop_path_observer(),
    ))
    .expect("initialize B-098 fixed-layout evolution store");
    let expected_installation_id = store.installation_id().to_string();
    drop(store);
    let (_legacy_store_id, legacy_generation_id) = downgrade_current_store_to_supported_v1(root);
    assert_eq!(legacy_generation_id, expected_installation_id);

    // The committed boot-id frame lives only in the WAL at classification
    // time. This proves startup reads and evolves the fixed authority itself,
    // rather than classifying a detached main-database image.
    let _wal_path = leave_nonempty_wal(
        root,
        "UPDATE store_metadata
            SET boot_id = '98989898-9898-4989-8989-989898989898'
          WHERE id = 1;",
    );
    let observer = Arc::new(FixedLayoutStartupObserver::new(root));
    let evolved = LocalEventStore::open(acceptance_store_config(
        root,
        Arc::new(FaultInjector::new()),
        observer.clone(),
    ))
    .expect("evolve the supported v1 fixed store with a non-empty WAL");

    assert_eq!(evolved.installation_id(), expected_installation_id);
    observer.assert_fixed_layout_only(root);
}

struct B098Fixture {
    _directory: tempfile::TempDir,
    root: std::path::PathBuf,
    identity_and_keys: (String, Vec<u8>, Vec<u8>),
    legacy_store_id: String,
    replay_batch: LocalAtomicBatch,
    plan: ShutdownPlanKey,
    signed_cursor: QueryCursor,
    terminal_before_evolution: LocalEventQueryResult,
    obligation_before_evolution: LocalEventQueryResult,
    current_shutdown_before_evolution: LocalEventQueryResult,
    shutdown_page_before_evolution: LocalEventQueryResult,
}

async fn b098_supported_v1_fixture() -> B098Fixture {
    use crate::usecase::agent_session::operation::{
        AgentSendOperationUsecase, CallerAttemptJournal, SendCommandOutcome, SendOperationRequest,
    };

    let directory = tempfile::tempdir().expect("B-098 app data");
    let root = directory.path().to_path_buf();
    let store = LocalEventStore::open(acceptance_store_config(
        &root,
        Arc::new(FaultInjector::new()),
        noop_path_observer(),
    ))
    .expect("initialize B-098 current store");
    let identity_and_keys = metadata_identity_and_keys(&root);
    let repository: Arc<dyn LocalEventTransactionRepository> = store.clone();
    let session_store = Arc::new(SessionStore::new_canonical(
        repository,
        store.installation_id().to_string(),
        Arc::new(AgentSessionProjectionCodecV1),
    ));
    let session = build_new_session_with_id(
        "b098-replay-session".to_string(),
        "/b098-replay-worktree",
        Some("codex".to_string()),
        crate::domain::agent_session::PermissionMode::Ask,
        None,
        false,
        false,
        None,
    );
    session_store
        .save_full_session_from_user(&root, &session)
        .expect("seed B-098 replay session");
    let replay_gate = Arc::new(PerformanceSendGate {
        session_store: session_store.clone(),
        session_id: session.id.clone(),
        effects: std::sync::atomic::AtomicUsize::new(0),
        planned: std::sync::atomic::AtomicUsize::new(0),
    });
    let send_usecase = AgentSendOperationUsecase::new(
        store.clone(),
        store.clone(),
        replay_gate.clone(),
        store.installation_id().to_string(),
    );
    let replay_request = SendOperationRequest {
        principal: "b098-principal".to_string(),
        operation_id: "b098-replay-send".to_string(),
        canonical_payload: "{\"content\":\"B-098 replay payload\"}".to_string(),
    };
    assert!(matches!(
        send_usecase
            .send(replay_request)
            .await
            .expect("accept B-098 operation before evolution"),
        SendCommandOutcome::Accepted(_)
    ));
    assert_eq!(
        replay_gate
            .effects
            .load(std::sync::atomic::Ordering::SeqCst),
        1
    );
    let caller_journal = CallerAttemptJournal::new(
        store.clone(),
        store.clone(),
        store.installation_id().to_string(),
    );
    caller_journal
        .record_attempt_scoped(
            "b098-principal",
            OperationKind::Send,
            "b098-caller-attempt",
            b"{\"content\":\"B-098 journal payload\"}",
            Some("b098-replay-session"),
        )
        .await
        .expect("seed a caller attempt under the old generation authority");
    let plan = ShutdownPlanKey {
        shutdown_id: "b098-quit".to_string(),
    };
    let mut summary = shutdown_plan_fixture(&plan.shutdown_id);
    summary.target_count = Some(2);
    summary.prepared_count = Some(2);
    summary.unresolved_count = Some(2);
    let mut seed = batch(
        "b098-seed",
        "b098-seed",
        [98; 32],
        Vec::new(),
        Vec::new(),
        vec![
            f06_operation_mutation(
                "b098-send-operation",
                f06_send_receipt("b098-send-operation"),
                f06_send_status(serde_json::json!({
                    "type": "awaiting_provider_start",
                    "dependency_obligation_ids": [],
                })),
            ),
            terminal_mutation("b098-session", "1"),
            obligation_mutation("b098-obligation", true),
            LocalStateMutation::ShutdownPlan(ShutdownPlanMutation {
                key: plan.clone(),
                phase: ApplicationShutdownPhase::Activated,
                summary,
                details_state: ShutdownDetailsState::Available,
                expected: RevisionGuard::Absent,
                revision: Revision::new(0).unwrap(),
            }),
            LocalStateMutation::ShutdownTarget(ShutdownTargetMutation {
                key: plan.clone(),
                ordinal: 0,
                detail: b059_shutdown_target_detail(
                    "b098-target",
                    ShutdownTargetStateRecord::Prepared,
                    None,
                ),
                expected: RevisionGuard::Absent,
                revision: Revision::new(0).unwrap(),
            }),
            LocalStateMutation::ShutdownTarget(ShutdownTargetMutation {
                key: plan.clone(),
                ordinal: 1,
                detail: b059_shutdown_target_detail(
                    "b098-target-2",
                    ShutdownTargetStateRecord::Prepared,
                    None,
                ),
                expected: RevisionGuard::Absent,
                revision: Revision::new(0).unwrap(),
            }),
            LocalStateMutation::ShutdownLatestPointer(ShutdownLatestPointerMutation {
                expected: None,
                new: Some(plan.clone()),
            }),
        ],
    );
    seed.idempotency.installation_id = store.installation_id().to_string();
    seed.idempotency.operation_kind = CommitOperationKind::Projection;
    let replay_batch = seed.clone();
    store
        .commit_batch(seed)
        .await
        .expect("seed B-098 semantic records");
    let first_page = store
        .query(LocalEventQuery::ShutdownPlanPage {
            plan: plan.clone(),
            limit: 1,
            cursor: None,
        })
        .await
        .expect("issue B-098 signed cursor before evolution");
    let LocalEventQueryResult::ShutdownPlanPage(first_page) = first_page else {
        panic!("B-098 first shutdown page query returned wrong shape");
    };
    assert_eq!(first_page.targets.len(), 1);
    let signed_cursor = first_page
        .next_cursor
        .expect("B-098 fixture must issue a signed continuation cursor");
    let terminal_before_evolution = store
        .query(LocalEventQuery::TerminalByTurn {
            session_id: "b098-session".to_string(),
            turn_id: "1".to_string(),
        })
        .await
        .expect("capture B-098 terminal read model before evolution");
    assert!(matches!(
        terminal_before_evolution,
        LocalEventQueryResult::TerminalByTurn(Some(_))
    ));
    let obligation_before_evolution = store
        .query(LocalEventQuery::ObligationByIdentity {
            obligation_id: "b098-obligation".to_string(),
        })
        .await
        .expect("capture B-098 obligation read model before evolution");
    assert!(matches!(
        obligation_before_evolution,
        LocalEventQueryResult::ObligationByIdentity(Some(_))
    ));
    let current_shutdown_before_evolution = store
        .query(LocalEventQuery::CurrentShutdown)
        .await
        .expect("capture B-098 current-shutdown read model before evolution");
    assert!(matches!(
        current_shutdown_before_evolution,
        LocalEventQueryResult::CurrentShutdown(Some(_))
    ));
    let shutdown_page_before_evolution = store
        .query(LocalEventQuery::ShutdownPlanPage {
            plan: plan.clone(),
            limit: 16,
            cursor: None,
        })
        .await
        .expect("capture complete B-098 shutdown read model before evolution");
    let LocalEventQueryResult::ShutdownPlanPage(shutdown_page) = &shutdown_page_before_evolution
    else {
        panic!("B-098 complete pre-evolution shutdown page returned wrong shape");
    };
    assert_eq!(
        shutdown_page.targets.len(),
        2,
        "B-098 exact shutdown oracle must include every target"
    );
    assert_eq!(
        shutdown_page
            .targets
            .iter()
            .map(|target| target.ordinal)
            .collect::<Vec<_>>(),
        vec![0, 1]
    );
    assert!(shutdown_page.next_cursor.is_none());
    drop(caller_journal);
    drop(send_usecase);
    drop(replay_gate);
    drop(session_store);
    drop(store);
    let (legacy_store_id, legacy_generation_id) = downgrade_current_store_to_supported_v1(&root);
    assert_eq!(
        legacy_generation_id, identity_and_keys.0,
        "the operation generation is the pre-evolution idempotency/HMAC authority"
    );
    B098Fixture {
        _directory: directory,
        root,
        identity_and_keys,
        legacy_store_id,
        replay_batch,
        plan,
        signed_cursor,
        terminal_before_evolution,
        obligation_before_evolution,
        current_shutdown_before_evolution,
        shutdown_page_before_evolution,
    }
}

async fn assert_b098_semantics(store: &Arc<LocalEventStore>, fixture: &B098Fixture) {
    use crate::usecase::agent_session::operation::{
        AgentSendOperationUsecase, CallerAttemptJournal, SendCommandOutcome, SendOperationRequest,
    };

    assert_eq!(
        metadata_identity_and_keys(&fixture.root),
        fixture.identity_and_keys
    );
    assert_eq!(store.installation_id(), fixture.identity_and_keys.0);
    assert_ne!(
        store.installation_id(),
        fixture.legacy_store_id,
        "the old physical store id cannot replace the operation/HMAC authority"
    );
    let connection = rusqlite::Connection::open(StoreLayout::new(&fixture.root).database_path())
        .expect("inspect converged B-098 identities");
    for table in ["logical_commits", "operation_bindings", "caller_attempts"] {
        let divergent: i64 = connection
            .query_row(
                &format!("SELECT COUNT(*) FROM {table} WHERE installation_id <> ?1"),
                [store.installation_id()],
                |row| row.get(0),
            )
            .expect("count divergent installation identities");
        assert_eq!(
            divergent, 0,
            "schema evolution left {table} under a split authority"
        );
    }
    drop(connection);

    let replay_session_store = Arc::new(SessionStore::new_canonical(
        store.clone(),
        store.installation_id().to_string(),
        Arc::new(AgentSessionProjectionCodecV1),
    ));
    let replay_gate = Arc::new(PerformanceSendGate {
        session_store: replay_session_store,
        session_id: "b098-replay-session".to_string(),
        effects: std::sync::atomic::AtomicUsize::new(0),
        planned: std::sync::atomic::AtomicUsize::new(0),
    });
    let send_usecase = AgentSendOperationUsecase::new(
        store.clone(),
        store.clone(),
        replay_gate.clone(),
        store.installation_id().to_string(),
    );
    assert!(matches!(
        send_usecase
            .send(SendOperationRequest {
                principal: "b098-principal".to_string(),
                operation_id: "b098-replay-send".to_string(),
                canonical_payload: "{\"content\":\"B-098 replay payload\"}".to_string(),
            })
            .await
            .expect("replay the pre-evolution send"),
        SendCommandOutcome::Accepted(_)
    ));
    assert_eq!(
        replay_gate
            .planned
            .load(std::sync::atomic::Ordering::SeqCst),
        0,
        "operation replay must not plan a second provider effect"
    );
    assert_eq!(
        replay_gate
            .effects
            .load(std::sync::atomic::Ordering::SeqCst),
        0,
        "operation replay must not dispatch a second provider effect"
    );
    let caller_journal = CallerAttemptJournal::new(
        store.clone(),
        store.clone(),
        store.installation_id().to_string(),
    );
    caller_journal
        .record_attempt_scoped(
            "b098-principal",
            OperationKind::Send,
            "b098-caller-attempt",
            b"{\"content\":\"B-098 journal payload\"}",
            Some("b098-replay-session"),
        )
        .await
        .expect("same caller attempt must replay after evolution");
    let caller_attempt = store
        .query(LocalEventQuery::CallerAttemptByIdentity {
            key: CallerOperationKey {
                principal: "b098-principal".to_string(),
                installation_id: store.installation_id().to_string(),
                kind: OperationKind::Send,
                caller_request_id: "b098-caller-attempt".to_string(),
            },
        })
        .await
        .expect("query evolved caller attempt");
    let LocalEventQueryResult::CallerAttemptByIdentity(Some(caller_attempt)) = caller_attempt
    else {
        panic!("the pre-evolution caller attempt was not found");
    };
    assert_eq!(
        caller_journal
            .open_attempt_command(&caller_attempt)
            .expect("open caller attempt under the preserved HMAC context"),
        b"{\"content\":\"B-098 journal payload\"}"
    );
    assert!(matches!(
        store.commit_batch(fixture.replay_batch.clone()).await,
        Ok(CommitBatchResult::Replayed(_))
    ));
    assert!(matches!(
        store
            .query(LocalEventQuery::OperationByIdentity {
                kind: OperationKind::Send,
                operation_id: "b098-send-operation".to_string(),
            })
            .await,
        Ok(LocalEventQueryResult::OperationByIdentity(Some(_)))
    ));
    assert_eq!(
        store
            .query(LocalEventQuery::TerminalByTurn {
                session_id: "b098-session".to_string(),
                turn_id: "1".to_string(),
            })
            .await
            .expect("read exact evolved terminal"),
        fixture.terminal_before_evolution,
        "B-098 evolution changed terminal identity, result, or participant digest"
    );
    assert_eq!(
        store
            .query(LocalEventQuery::ObligationByIdentity {
                obligation_id: "b098-obligation".to_string(),
            })
            .await
            .expect("read exact evolved obligation"),
        fixture.obligation_before_evolution,
        "B-098 evolution changed obligation state, pending detail, hash, or revision"
    );
    assert_eq!(
        store
            .query(LocalEventQuery::CurrentShutdown)
            .await
            .expect("read exact evolved current shutdown"),
        fixture.current_shutdown_before_evolution,
        "B-098 evolution changed current shutdown phase, summary, detail state, hash, or revision"
    );
    assert_eq!(
        store
            .query(LocalEventQuery::ShutdownPlanPage {
                plan: fixture.plan.clone(),
                limit: 16,
                cursor: None,
            })
            .await
            .expect("read complete evolved shutdown page"),
        fixture.shutdown_page_before_evolution,
        "B-098 evolution changed shutdown summary, target details/ordinals, hashes, or revisions"
    );
    let first_page = store
        .query(LocalEventQuery::ShutdownPlanPage {
            plan: fixture.plan.clone(),
            limit: 1,
            cursor: None,
        })
        .await
        .expect("B-098 shutdown page");
    let LocalEventQueryResult::ShutdownPlanPage(first_page) = first_page else {
        panic!("B-098 shutdown page query returned wrong shape");
    };
    assert_eq!(first_page.plan.plan, fixture.plan);
    assert_eq!(first_page.targets.len(), 1);
    let current_cursor = first_page
        .next_cursor
        .expect("B-098 evolved store must issue a signed continuation cursor");
    assert!(matches!(
        store
            .query(LocalEventQuery::ShutdownPlanPage {
                plan: fixture.plan.clone(),
                limit: 1,
                cursor: Some(fixture.signed_cursor.clone()),
            })
            .await,
        Err(LocalEventQueryError::CursorExpired)
    ));
    let continued_page = store
        .query(LocalEventQuery::ShutdownPlanPage {
            plan: fixture.plan.clone(),
            limit: 1,
            cursor: Some(current_cursor),
        })
        .await
        .expect("B-098 post-evolution signed cursor preserves pagination semantics");
    let LocalEventQueryResult::ShutdownPlanPage(continued_page) = continued_page else {
        panic!("B-098 continued shutdown page query returned wrong shape");
    };
    assert_eq!(continued_page.plan.plan, fixture.plan);
    assert_eq!(continued_page.targets.len(), 1);
    assert_eq!(continued_page.targets[0].ordinal, 1);
    assert!(continued_page.next_cursor.is_none());
}

#[tokio::test]
async fn b098_supported_schema_evolution_is_atomic_and_preserves_identity_keys_and_semantics() {
    enum Boundary {
        BeforeBegin,
        BeforeCommit,
        CommitReplyLoss,
        BeforeReadback,
    }
    for boundary in [
        Boundary::BeforeBegin,
        Boundary::BeforeCommit,
        Boundary::CommitReplyLoss,
        Boundary::BeforeReadback,
    ] {
        let fixture = b098_supported_v1_fixture().await;
        let fault = Arc::new(FaultInjector::new());
        match boundary {
            Boundary::BeforeBegin => fault.arm_schema_fail_before_begin(),
            Boundary::BeforeCommit => fault.arm_schema_fail_before_commit(),
            Boundary::CommitReplyLoss => fault.arm_schema_commit_reply_loss(),
            Boundary::BeforeReadback => fault.arm_schema_fail_before_readback(),
        }
        assert!(matches!(
            LocalEventStore::open(acceptance_store_config(
                &fixture.root,
                fault,
                noop_path_observer(),
            )),
            Err(super::store::LocalEventStoreOpenError::SchemaEvolutionFailed)
        ));

        let connection =
            rusqlite::Connection::open(StoreLayout::new(&fixture.root).database_path())
                .expect("inspect B-098 boundary");
        let user_version: i64 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .expect("read B-098 schema version");
        match boundary {
            Boundary::BeforeBegin | Boundary::BeforeCommit => assert_eq!(user_version, 0),
            Boundary::CommitReplyLoss | Boundary::BeforeReadback => {
                assert_eq!(user_version, 2)
            }
        }
        drop(connection);

        let reopened = LocalEventStore::open(acceptance_store_config(
            &fixture.root,
            Arc::new(FaultInjector::new()),
            noop_path_observer(),
        ))
        .expect("B-098 restart converges to a validated schema");
        assert_b098_semantics(&reopened, &fixture).await;
        let connection =
            rusqlite::Connection::open(StoreLayout::new(&fixture.root).database_path())
                .expect("inspect evolved schema");
        for removed in [
            "local_store_migrations",
            "legacy_source_inventory",
            "legacy_raw_records",
            "legacy_raw_record_chunks",
            "migration_quit_flights",
            "shutdown_compact_archives",
        ] {
            let count: i64 = connection
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_schema WHERE name = ?1",
                    [removed],
                    |row| row.get(0),
                )
                .expect("inspect removed obsolete schema object");
            assert_eq!(count, 0, "obsolete schema object remained: {removed}");
        }
    }
}

fn send_effect_admission_projection(
    session_id: &str,
    state: AgentSessionStateRecord,
    queue_paused: bool,
) -> SessionProjectionRecord {
    let SessionProjectionRecord::AgentSession(mut projection) =
        agent_session_projection(session_id)
    else {
        unreachable!("agent-session fixture has the closed agent-session variant");
    };
    projection.meta.state = state;
    projection.meta.last_turn_id = Some(1);
    projection.queue_paused_at_bits = queue_paused.then_some(1.0_f64.to_bits());
    SessionProjectionRecord::AgentSession(projection)
}

fn send_effect_admission_obligation(
    session_id: &str,
    obligation_id: &str,
    state: ObligationStateRecord,
    expected: RevisionGuard,
    revision: Revision,
) -> LocalStateMutation {
    LocalStateMutation::Obligation(ObligationMutation {
        obligation_id: obligation_id.to_string(),
        record: ObligationRecord::Send {
            obligation_id: obligation_id.to_string(),
            operation_id: format!("operation-{obligation_id}"),
            session_id: session_id.to_string(),
            kind: crate::domain::local_event::SendObligationKindRecord::TurnExecution,
            disposition: crate::domain::local_event::SendObligationDispositionRecord::StartedTurn,
            human_message_id: Some(format!("human-{obligation_id}")),
            assistant_message_id: Some(format!("assistant-{obligation_id}")),
            reserved_turn_id: None,
            turn_id: Some("1".to_string()),
            dependency_obligation_ids: Vec::new(),
            canonical_payload: format!("payload-{obligation_id}"),
            state,
        },
        pending: Some(PendingIndexEntry {
            ordered_key: format!("send-effect-admission:{obligation_id}"),
            owner: session_id.to_string(),
            partition: PendingPartition::Owner,
            shutdown_plan: None,
        }),
        expected,
        revision,
    })
}

async fn seed_send_effect_admission(
    harness: &Harness,
    fixture_id: &str,
    session_id: &str,
    obligation_id: &str,
    state: AgentSessionStateRecord,
    queue_paused: bool,
) {
    harness
        .store
        .commit_batch(batch(
            &format!("{fixture_id}-seed"),
            &format!("{fixture_id}-seed"),
            [101; 32],
            Vec::new(),
            Vec::new(),
            vec![
                LocalStateMutation::SessionProjection(SessionProjectionMutation {
                    session_id: session_id.to_string(),
                    projection: send_effect_admission_projection(session_id, state, queue_paused),
                    expected: RevisionGuard::Absent,
                    revision: Revision::new(0).unwrap(),
                }),
                send_effect_admission_obligation(
                    session_id,
                    obligation_id,
                    ObligationStateRecord::Pending,
                    RevisionGuard::Absent,
                    Revision::new(0).unwrap(),
                ),
            ],
        ))
        .await
        .expect("seed send effect admission fixture");
}

fn send_effect_admission_claim(
    fixture_id: &str,
    session_id: &str,
    obligation_id: &str,
) -> LocalAtomicBatch {
    batch(
        &format!("{fixture_id}-claim"),
        &format!("{fixture_id}-claim"),
        [102; 32],
        Vec::new(),
        Vec::new(),
        vec![send_effect_admission_obligation(
            session_id,
            obligation_id,
            ObligationStateRecord::EffectReserved,
            RevisionGuard::Expected(Revision::new(0).unwrap()),
            Revision::new(1).unwrap(),
        )],
    )
}

async fn send_effect_admission_obligation_snapshot(
    store: &Arc<LocalEventStore>,
    obligation_id: &str,
) -> (ObligationRecord, Revision, bool) {
    let result = store
        .query(LocalEventQuery::ObligationByIdentity {
            obligation_id: obligation_id.to_string(),
        })
        .await
        .expect("read send effect admission obligation");
    let LocalEventQueryResult::ObligationByIdentity(Some(obligation)) = result else {
        panic!("send effect admission obligation is missing");
    };
    (
        obligation.record,
        obligation.revision,
        obligation.pending.is_some(),
    )
}

#[tokio::test]
async fn send_effect_admission_unpaused_pending_claim_commits() {
    let harness = Harness::open();
    let session_id = "send-effect-admission-open-session";
    let obligation_id = "send-effect-admission-open.exec";
    seed_send_effect_admission(
        &harness,
        "send-effect-admission-open",
        session_id,
        obligation_id,
        AgentSessionStateRecord::Active,
        false,
    )
    .await;

    let result = harness
        .store
        .commit_batch(send_effect_admission_claim(
            "send-effect-admission-open",
            session_id,
            obligation_id,
        ))
        .await
        .expect("unpaused send effect claim must commit");
    assert!(matches!(result, CommitBatchResult::Committed(_)));

    let (record, revision, pending) =
        send_effect_admission_obligation_snapshot(&harness.store, obligation_id).await;
    assert!(matches!(
        record,
        ObligationRecord::Send {
            state: ObligationStateRecord::EffectReserved,
            ..
        }
    ));
    assert_eq!(revision, Revision::new(1).unwrap());
    assert!(pending);
}

#[tokio::test]
async fn send_effect_admission_paused_closed_or_archived_rejects_without_mutation() {
    for (fixture_id, state, queue_paused) in [
        (
            "send-effect-admission-paused",
            AgentSessionStateRecord::Active,
            true,
        ),
        (
            "send-effect-admission-closed",
            AgentSessionStateRecord::Closed,
            false,
        ),
        (
            "send-effect-admission-archived",
            AgentSessionStateRecord::Archived,
            false,
        ),
    ] {
        let harness = Harness::open();
        let session_id = format!("{fixture_id}-session");
        let obligation_id = format!("{fixture_id}.exec");
        seed_send_effect_admission(
            &harness,
            fixture_id,
            &session_id,
            &obligation_id,
            state,
            queue_paused,
        )
        .await;
        let before =
            send_effect_admission_obligation_snapshot(&harness.store, &obligation_id).await;
        let claim = send_effect_admission_claim(fixture_id, &session_id, &obligation_id);
        let claim_identity = claim.commit_id.clone();

        assert!(matches!(
            harness.store.commit_batch(claim).await,
            Err(CommitBatchError::EffectAdmissionBlocked)
        ));
        assert_eq!(
            send_effect_admission_obligation_snapshot(&harness.store, &obligation_id).await,
            before,
            "{fixture_id} changed the guarded obligation"
        );
        assert!(matches!(
            harness.store.resolve_commit(claim_identity).await.unwrap(),
            CommitResolution::NotCommitted
        ));
    }
}

#[tokio::test]
async fn send_effect_admission_rejects_a_superseded_immediate_turn_without_mutation() {
    let harness = Harness::open();
    let fixture_id = "send-effect-admission-stale-turn";
    let session_id = "send-effect-admission-stale-turn-session";
    let obligation_id = "send-effect-admission-stale-turn.exec";
    seed_send_effect_admission(
        &harness,
        fixture_id,
        session_id,
        obligation_id,
        AgentSessionStateRecord::Active,
        false,
    )
    .await;

    let SessionProjectionRecord::AgentSession(mut newer_projection) =
        send_effect_admission_projection(session_id, AgentSessionStateRecord::Active, false)
    else {
        unreachable!("agent-session fixture has the closed agent-session variant");
    };
    newer_projection.meta.last_turn_id = Some(2);
    harness
        .store
        .commit_batch(batch(
            "send-effect-admission-stale-turn-projection",
            "send-effect-admission-stale-turn-projection",
            [105; 32],
            Vec::new(),
            Vec::new(),
            vec![LocalStateMutation::SessionProjection(
                SessionProjectionMutation {
                    session_id: session_id.to_string(),
                    projection: SessionProjectionRecord::AgentSession(newer_projection),
                    expected: RevisionGuard::Expected(Revision::new(0).unwrap()),
                    revision: Revision::new(1).unwrap(),
                },
            )],
        ))
        .await
        .expect("advance canonical active turn");

    let before = send_effect_admission_obligation_snapshot(&harness.store, obligation_id).await;
    assert!(matches!(
        harness
            .store
            .commit_batch(send_effect_admission_claim(
                fixture_id,
                session_id,
                obligation_id,
            ))
            .await,
        Err(CommitBatchError::EffectAdmissionBlocked)
    ));
    assert_eq!(
        send_effect_admission_obligation_snapshot(&harness.store, obligation_id).await,
        before
    );
}

#[tokio::test]
async fn send_effect_admission_is_blocked_by_reserved_legacy_provider_establish() {
    let harness = Harness::open();
    let fixture_id = "send-effect-admission-legacy-establish";
    let session_id = "send-effect-admission-legacy-establish-session";
    let obligation_id = "send-effect-admission-legacy-establish-current.exec";
    let legacy_id = "send-effect-admission-legacy-establish.establish";
    seed_send_effect_admission(
        &harness,
        fixture_id,
        session_id,
        obligation_id,
        AgentSessionStateRecord::Active,
        false,
    )
    .await;

    harness
        .store
        .commit_batch(batch(
            "send-effect-admission-legacy-provider-seed",
            "send-effect-admission-legacy-provider-seed",
            [106; 32],
            Vec::new(),
            Vec::new(),
            vec![LocalStateMutation::Obligation(ObligationMutation {
                obligation_id: legacy_id.to_string(),
                record: ObligationRecord::Send {
                    obligation_id: legacy_id.to_string(),
                    operation_id: "send-effect-admission-legacy-operation".to_string(),
                    session_id: session_id.to_string(),
                    kind: crate::domain::local_event::SendObligationKindRecord::ProviderEstablish,
                    disposition:
                        crate::domain::local_event::SendObligationDispositionRecord::StartedTurn,
                    human_message_id: Some("legacy-human".to_string()),
                    assistant_message_id: Some("legacy-assistant".to_string()),
                    reserved_turn_id: None,
                    turn_id: Some("0".to_string()),
                    dependency_obligation_ids: Vec::new(),
                    canonical_payload: "legacy-payload".to_string(),
                    state: ObligationStateRecord::EffectReserved,
                },
                pending: Some(PendingIndexEntry {
                    ordered_key: format!("send-effect-admission:{legacy_id}"),
                    owner: session_id.to_string(),
                    partition: PendingPartition::Owner,
                    shutdown_plan: None,
                }),
                expected: RevisionGuard::Absent,
                revision: Revision::new(0).unwrap(),
            })],
        ))
        .await
        .expect("seed reserved legacy ProviderEstablish");

    let before = send_effect_admission_obligation_snapshot(&harness.store, obligation_id).await;
    assert!(matches!(
        harness
            .store
            .commit_batch(send_effect_admission_claim(
                fixture_id,
                session_id,
                obligation_id,
            ))
            .await,
        Err(CommitBatchError::EffectAdmissionBlocked)
    ));
    assert_eq!(
        send_effect_admission_obligation_snapshot(&harness.store, obligation_id).await,
        before
    );
}

#[tokio::test]
async fn send_effect_admission_owner_recovery_blocks_then_same_claim_identity_commits() {
    let harness = Harness::open();
    let fixture_id = "send-effect-admission-owner-recovery";
    let session_id = "send-effect-admission-owner-recovery-session";
    let obligation_id = "send-effect-admission-owner-recovery.exec";
    let blocker_id = "backend-recovery:send-effect-admission-owner-recovery";
    seed_send_effect_admission(
        &harness,
        fixture_id,
        session_id,
        obligation_id,
        AgentSessionStateRecord::Active,
        false,
    )
    .await;

    let blocker = LocalStateMutation::Obligation(ObligationMutation {
        obligation_id: blocker_id.to_string(),
        record: ObligationRecord::BackendSessionRecovery {
            session_id: session_id.to_string(),
            recovery_id: "send-effect-admission-recovery".to_string(),
            detail:
                crate::domain::local_event::BackendSessionRecoveryObligationRecord::EffectReserved {
                    old_provider_session_generation: 0,
                    reason: BackendSessionRecoveryReason::BackendSessionLost,
                    reserved_at_bits: 1.0_f64.to_bits(),
                },
            state: ObligationStateRecord::EffectReserved,
        },
        pending: Some(PendingIndexEntry {
            ordered_key: format!("send-effect-admission:{blocker_id}"),
            owner: session_id.to_string(),
            partition: PendingPartition::Owner,
            shutdown_plan: None,
        }),
        expected: RevisionGuard::Absent,
        revision: Revision::new(0).unwrap(),
    });
    let mut blocker_seed = batch(
        "send-effect-admission-owner-recovery-blocker",
        "send-effect-admission-owner-recovery-blocker",
        [103; 32],
        Vec::new(),
        Vec::new(),
        vec![blocker],
    );
    blocker_seed.idempotency.operation_kind = CommitOperationKind::Recovery;
    harness
        .store
        .commit_batch(blocker_seed)
        .await
        .expect("seed owner recovery blocker");

    let claim = send_effect_admission_claim(fixture_id, session_id, obligation_id);
    let claim_identity = claim.commit_id.clone();
    assert!(matches!(
        harness.store.commit_batch(claim.clone()).await,
        Err(CommitBatchError::EffectAdmissionBlocked)
    ));
    assert_eq!(
        send_effect_admission_obligation_snapshot(&harness.store, obligation_id).await,
        (
            match send_effect_admission_obligation(
                session_id,
                obligation_id,
                ObligationStateRecord::Pending,
                RevisionGuard::Absent,
                Revision::new(0).unwrap(),
            ) {
                LocalStateMutation::Obligation(obligation) => obligation.record,
                _ => unreachable!(),
            },
            Revision::new(0).unwrap(),
            true,
        )
    );
    assert!(matches!(
        harness
            .store
            .resolve_commit(claim_identity.clone())
            .await
            .unwrap(),
        CommitResolution::NotCommitted
    ));

    let resolved_blocker = LocalStateMutation::Obligation(ObligationMutation {
        obligation_id: blocker_id.to_string(),
        record: ObligationRecord::BackendSessionRecovery {
            session_id: session_id.to_string(),
            recovery_id: "send-effect-admission-recovery".to_string(),
            detail: crate::domain::local_event::BackendSessionRecoveryObligationRecord::Completed {
                old_provider_session_generation: 0,
                provider_session_generation: 1,
                backend_session_id: "backend-session-1".to_string(),
                completed_at_bits: 2.0_f64.to_bits(),
            },
            state: ObligationStateRecord::Completed,
        },
        pending: None,
        expected: RevisionGuard::Expected(Revision::new(0).unwrap()),
        revision: Revision::new(1).unwrap(),
    });
    let mut blocker_resolution = batch(
        "send-effect-admission-owner-recovery-resolved",
        "send-effect-admission-owner-recovery-resolved",
        [104; 32],
        Vec::new(),
        Vec::new(),
        vec![resolved_blocker],
    );
    blocker_resolution.idempotency.operation_kind = CommitOperationKind::Recovery;
    harness
        .store
        .commit_batch(blocker_resolution)
        .await
        .expect("resolve owner recovery blocker");

    let result = harness
        .store
        .commit_batch(claim)
        .await
        .expect("the exact rejected claim identity must commit after recovery resolution");
    assert!(matches!(result, CommitBatchResult::Committed(_)));
    assert!(matches!(
        harness.store.resolve_commit(claim_identity).await.unwrap(),
        CommitResolution::Committed(_)
    ));
    let (record, revision, pending) =
        send_effect_admission_obligation_snapshot(&harness.store, obligation_id).await;
    assert!(matches!(
        record,
        ObligationRecord::Send {
            state: ObligationStateRecord::EffectReserved,
            ..
        }
    ));
    assert_eq!(revision, Revision::new(1).unwrap());
    assert!(pending);
}
