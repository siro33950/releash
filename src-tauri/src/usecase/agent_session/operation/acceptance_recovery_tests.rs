use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use crate::adaptor::controller::agent_session_operation_wiring::{
    ActiveSendRecoveryContext, ConservativeRecoveryExecutor, RuntimeAgentSessionOperationGate,
    RuntimePermissionResponseOperationGate, RuntimeSendOperationGate,
};
use crate::adaptor::gateway::local_event_store::{LocalEventStore, LocalEventStoreConfig};
use crate::domain::agent_session::entities::{
    InterruptReason as TurnInterruptReason, MessagePart, PermissionResponse,
    PermissionResponseDecision, TurnResult,
};
use crate::domain::agent_session::events::{
    AgentSessionDomainEvent, BackendSessionRecoveryReason, RecoveryActionKind,
    RecoveryResultClassification, SendDisposition, StopResolution,
};
use crate::domain::local_event::{
    AgentMessageProjectionRecord, AgentMessageRoleRecord, AgentSessionStateRecord,
    AgentTerminalKind, AgentTurnTerminalResultRecord, AuthoritativeEffectObservationRecord,
    BackendSessionRecoveryObligationRecord, CommitBatchError, CommitBatchResult, CommitIdentity,
    CommitOperationKind, ExpectedStreamHead, IdempotencyBinding, LoadStreamRequest,
    LoadedDomainEvent, LocalAtomicBatch, LocalDomainEvent, LocalEventQuery, LocalEventQueryResult,
    LocalEventTransactionRepository, LocalStateMutation, MessageProjectionMutation,
    MessageProjectionRecord, ObligationMutation, ObligationRecord, ObligationStateRecord,
    OperationKind, OperationReceiptRecord, OperationRecordMutation, OperationStatusRecord,
    OperationStatusValue, PendingIndexEntry, PendingPartition, RecordAuthentication, Revision,
    RevisionGuard, SendObligationDispositionRecord, SendObligationKindRecord,
    SessionOperationFailureKind, SessionProjectionMutation, SessionProjectionRecord,
    ShutdownPlanKey, StopResolutionKind, StreamId, TerminalRecordMutation, TerminalResultRecord,
    UncommittedDomainEvent, WorkflowTurnCompletionObligationRecord,
};

use super::PermissionResponseOperationUsecase;
use super::{
    derive_recovery_action_id, AcceptedSendEffect, AcceptedStopEffect, AgentSendOperationUsecase,
    PendingRecoveryOwnerTarget, PendingRecoveryQuery, RecoveryActionError, RecoveryActionOutcome,
    RecoveryActionRejection, RecoveryActionRequest, RecoveryActionResultOutcome,
    RecoveryActionStatus, RecoveryActionUsecase, RecoveryEffectExecutor, RecoveryEffectRequest,
    RecoveryEffectResult, SendAdmissionGate, SendPlan, SessionLifecycleOperationUsecase,
    StopAdmissionGate, StopCommandOutcome, StopEffectObservation, StopOperationRequest,
    StopOperationState, StopOperationUsecase, StopTargetSnapshot,
};
use crate::domain::local_event::SafeOperationFailure;
use crate::usecase::agent_session::operation::OperationBindingAuthority;
use crate::usecase::agent_session::session::{
    AgentSessionProjectionCodec, CanonicalAgentSessionProjection, SessionMeta, SessionState,
};

fn open_store() -> (tempfile::TempDir, Arc<LocalEventStore>) {
    let directory = tempfile::tempdir().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
    ))
    .unwrap();
    (directory, store)
}

fn canonical_agent_session_projection(session_id: &str) -> SessionProjectionRecord {
    canonical_agent_session_projection_with_state(session_id, SessionState::Idle)
}

fn canonical_agent_session_projection_with_state(
    session_id: &str,
    state: SessionState,
) -> SessionProjectionRecord {
    let mut session = crate::usecase::agent_session::session::build_new_session_with_id(
        session_id.to_string(),
        "/tmp/issue-1499-recovery-owner",
        Some("codex".to_string()),
        crate::domain::agent_session::PermissionMode::Ask,
        Some("gpt-5.6-sol".to_string()),
        false,
        false,
        None,
    );
    session.state = state;
    crate::adaptor::gateway::agent_session::session_storage::AgentSessionProjectionCodecV1
        .encode(&CanonicalAgentSessionProjection {
            meta: SessionMeta::from_session(&session),
            title: None,
            messages: Vec::new(),
            reducer_events: Vec::new(),
            queue_paused_at: None,
            latest_token_usage: None,
            pending_send_queue: Vec::new(),
        })
        .unwrap()
}

fn marked_agent_session_projection(
    session_id: &str,
    state: SessionState,
    marker: Option<&str>,
    queue_paused: bool,
) -> SessionProjectionRecord {
    let mut projection = canonical_agent_session_projection_with_state(session_id, state);
    let SessionProjectionRecord::AgentSession(agent_projection) = &mut projection else {
        unreachable!("agent codec returned a non-agent projection");
    };
    agent_projection.title = marker.map(str::to_string);
    agent_projection.queue_paused_at_bits = queue_paused.then_some(0.0_f64.to_bits());
    projection
}

fn agent_message_projection(
    message_id: &str,
    marker: &str,
    part_content: &str,
    streaming_final_seq: u64,
) -> MessageProjectionRecord {
    MessageProjectionRecord::AgentMessage(AgentMessageProjectionRecord {
        id: message_id.to_string(),
        role: AgentMessageRoleRecord::Agent,
        content: marker.to_string(),
        thinking: None,
        activities: None,
        parts: Some(vec![MessagePart::Text {
            content: part_content.to_string(),
            parent_tool_use_id: None,
        }]),
        streaming_final_seq,
        timestamp_bits: 0.0_f64.to_bits(),
        mentions: None,
    })
}

fn pending_obligation(
    obligation_id: &str,
    owner: &str,
    partition: PendingPartition,
    ordinal: usize,
    record: ObligationRecord,
) -> LocalStateMutation {
    LocalStateMutation::Obligation(ObligationMutation {
        obligation_id: obligation_id.to_string(),
        record,
        pending: Some(PendingIndexEntry {
            ordered_key: format!("{ordinal:020}-{obligation_id}"),
            owner: owner.to_string(),
            partition,
            shutdown_plan: None,
        }),
        expected: RevisionGuard::Absent,
        revision: Revision::new(0).unwrap(),
    })
}

fn permission_obligation(state: ObligationStateRecord) -> ObligationRecord {
    ObligationRecord::PermissionResponse {
        operation_id: "b022-permission-operation".to_string(),
        effect_identity: "permission-response:b022-permission-operation".to_string(),
        session_id: "b022-session".to_string(),
        turn_id: "1".to_string(),
        response: PermissionResponse {
            request_id: "permission-1".to_string(),
            decision: PermissionResponseDecision::Deny { message: None },
        },
        owner_access: true,
        from_runtime_state: true,
        state,
    }
}

fn agent_terminal_result(
    session_id: &str,
    turn_id: &str,
    message_id: &str,
    result: TurnResult,
) -> TerminalResultRecord {
    TerminalResultRecord::AgentTurn {
        kind: match &result {
            TurnResult::Completed { .. } => AgentTerminalKind::Completed,
            TurnResult::Interrupted { reason, .. } => match reason {
                TurnInterruptReason::Abort => AgentTerminalKind::Abort,
                TurnInterruptReason::Timeout => AgentTerminalKind::Timeout,
                TurnInterruptReason::Crash => AgentTerminalKind::Crash,
                TurnInterruptReason::SessionClosed => AgentTerminalKind::SessionClosed,
            },
            TurnResult::Failed { .. } => AgentTerminalKind::Crash,
        },
        session_id: session_id.to_string(),
        turn_id: turn_id.to_string(),
        message_id: message_id.to_string(),
        streaming_final_sequence: 0,
        completed_at_bits: 0,
        result: AgentTurnTerminalResultRecord::Current(result),
    }
}

fn backend_reconciliation(session_id: &str, recovery_id: &str) -> ObligationRecord {
    ObligationRecord::BackendSessionRecovery {
        session_id: session_id.to_string(),
        recovery_id: recovery_id.to_string(),
        detail: BackendSessionRecoveryObligationRecord::EffectReserved {
            old_provider_session_generation: 0,
            reason: BackendSessionRecoveryReason::BackendSessionLost,
            reserved_at_bits: 0,
        },
        state: ObligationStateRecord::ReconciliationRequired,
    }
}

async fn commit_mutations(
    store: &Arc<LocalEventStore>,
    identity: &str,
    mutations: Vec<LocalStateMutation>,
) {
    store
        .commit_batch(LocalAtomicBatch {
            commit_id: CommitIdentity::parse(identity).unwrap(),
            idempotency: IdempotencyBinding {
                installation_id: store.installation_id().to_string(),
                operation_kind: CommitOperationKind::Recovery,
                idempotency_key: identity.to_string(),
                payload_hash: OperationBindingAuthority::digest(
                    store.as_ref(),
                    identity.as_bytes(),
                ),
            },
            expected_heads: Vec::new(),
            events: Vec::new(),
            state_mutations: mutations,
        })
        .await
        .unwrap_or_else(|error| panic!("commit {identity} failed: {error:?}"));
}

#[derive(Default)]
struct RecordingStartupSendGate {
    effects: Mutex<Vec<AcceptedSendEffect>>,
}

impl RecordingStartupSendGate {
    fn effect_count(&self) -> usize {
        self.effects.lock().unwrap().len()
    }
}

#[async_trait::async_trait]
impl SendAdmissionGate for RecordingStartupSendGate {
    async fn plan_send(
        &self,
        _principal: &str,
        _operation_id: &str,
        _canonical_payload: &str,
    ) -> Result<SendPlan, SafeOperationFailure> {
        panic!("startup recovery must not plan a new send")
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
        effect: &AcceptedSendEffect,
    ) -> Result<super::ports::SendEffectDispatch, SafeOperationFailure> {
        self.effects.lock().unwrap().push(effect.clone());
        Ok(super::ports::SendEffectDispatch::Scheduled)
    }
}

async fn assert_b022_terminal_projection(store: &Arc<LocalEventStore>, committed: bool) {
    let session_id = "b022-session";
    let terminal = store
        .query(LocalEventQuery::TerminalByTurn {
            session_id: session_id.to_string(),
            turn_id: "1".to_string(),
        })
        .await
        .unwrap();
    assert_eq!(
        matches!(terminal, LocalEventQueryResult::TerminalByTurn(Some(_))),
        committed
    );

    let message = store
        .query(LocalEventQuery::MessageProjectionByIdentity {
            session_id: session_id.to_string(),
            message_id: "agent-1".to_string(),
        })
        .await
        .unwrap();
    let LocalEventQueryResult::MessageProjectionByIdentity(Some(message)) = message else {
        panic!("B022 assistant projection missing");
    };
    let MessageProjectionRecord::AgentMessage(message) = message.projection else {
        panic!("B022 assistant projection has the wrong semantic family");
    };
    assert_eq!(message.streaming_final_seq > 0, committed);
    assert!(matches!(
        message.parts.as_deref(),
        Some([MessagePart::Text { content, .. }])
            if content == if committed { "complete" } else { "before" }
    ));

    let session = store
        .query(LocalEventQuery::SessionProjectionByIdentity {
            session_id: session_id.to_string(),
        })
        .await
        .unwrap();
    let LocalEventQueryResult::SessionProjectionByIdentity(Some(session)) = session else {
        panic!("B022 session projection missing");
    };
    let SessionProjectionRecord::AgentSession(session) = session.projection else {
        panic!("B022 session projection has the wrong semantic family");
    };
    assert_eq!(
        session.meta.state,
        if committed {
            AgentSessionStateRecord::Idle
        } else {
            AgentSessionStateRecord::Active
        }
    );
    assert_eq!(session.queue_paused_at_bits.is_some(), committed);

    let permission = store
        .query(LocalEventQuery::ObligationByIdentity {
            obligation_id: "b022-permission".to_string(),
        })
        .await
        .unwrap();
    let LocalEventQueryResult::ObligationByIdentity(Some(permission)) = permission else {
        panic!("B022 permission participant missing");
    };
    assert!(
        matches!(
            &permission.record,
            ObligationRecord::PermissionResponse {
                state: ObligationStateRecord::Completed,
                ..
            }
        ) == committed
    );
    assert_eq!(permission.pending.is_none(), committed);
}

#[tokio::test]
async fn b022_each_terminal_commit_boundary_is_all_old_or_all_new_after_reload() {
    for boundary in [
        "before_begin",
        "after_participant_write",
        "before_commit",
        "after_commit_before_readback",
        "committed",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let store = LocalEventStore::open(LocalEventStoreConfig::production(
            directory.path().to_path_buf(),
        ))
        .unwrap();
        commit_mutations(
            &store,
            &format!("b022-seed-{boundary}"),
            vec![
                LocalStateMutation::MessageProjection(MessageProjectionMutation {
                    session_id: "b022-session".to_string(),
                    message_id: "agent-1".to_string(),
                    projection: agent_message_projection("agent-1", "before", "before", 0),
                    expected: RevisionGuard::Absent,
                    revision: Revision::new(0).unwrap(),
                }),
                LocalStateMutation::SessionProjection(SessionProjectionMutation {
                    session_id: "b022-session".to_string(),
                    projection: marked_agent_session_projection(
                        "b022-session",
                        SessionState::Active,
                        None,
                        false,
                    ),
                    expected: RevisionGuard::Absent,
                    revision: Revision::new(0).unwrap(),
                }),
                pending_obligation(
                    "b022-permission",
                    "b022-session",
                    PendingPartition::Owner,
                    1,
                    permission_obligation(ObligationStateRecord::Pending),
                ),
            ],
        )
        .await;

        match boundary {
            "before_begin" => store.fault_injector().arm_fail_before_begin(),
            "after_participant_write" => store.fault_injector().arm_fail_after_participant_write(),
            "before_commit" => store.fault_injector().arm_fail_before_commit(),
            "after_commit_before_readback" => store
                .fault_injector()
                .arm_crash_after_commit_before_readback(),
            "committed" => {}
            _ => unreachable!(),
        }
        let result = store
            .commit_batch(LocalAtomicBatch {
                commit_id: CommitIdentity::parse(&format!("b022-terminal-{boundary}")).unwrap(),
                idempotency: IdempotencyBinding {
                    installation_id: store.installation_id().to_string(),
                    operation_kind: CommitOperationKind::OperationProgress,
                    idempotency_key: format!("b022-terminal-{boundary}"),
                    payload_hash: OperationBindingAuthority::digest(
                        store.as_ref(),
                        boundary.as_bytes(),
                    ),
                },
                expected_heads: Vec::new(),
                events: Vec::new(),
                state_mutations: vec![
                    LocalStateMutation::TerminalRecord(TerminalRecordMutation {
                        session_id: "b022-session".to_string(),
                        turn_id: "1".to_string(),
                        terminal_identity: "b022-completed".to_string(),
                        result: agent_terminal_result(
                            "b022-session",
                            "1",
                            "agent-1",
                            TurnResult::Completed {
                                stop_reason: None,
                                token_usage: None,
                            },
                        ),
                        participant_digest: OperationBindingAuthority::digest(
                            store.as_ref(),
                            b"b022-all-terminal-participants",
                        ),
                    }),
                    LocalStateMutation::MessageProjection(MessageProjectionMutation {
                        session_id: "b022-session".to_string(),
                        message_id: "agent-1".to_string(),
                        projection: agent_message_projection("agent-1", "complete", "complete", 1),
                        expected: RevisionGuard::Expected(Revision::new(0).unwrap()),
                        revision: Revision::new(1).unwrap(),
                    }),
                    LocalStateMutation::SessionProjection(SessionProjectionMutation {
                        session_id: "b022-session".to_string(),
                        projection: marked_agent_session_projection(
                            "b022-session",
                            SessionState::Idle,
                            None,
                            true,
                        ),
                        expected: RevisionGuard::Expected(Revision::new(0).unwrap()),
                        revision: Revision::new(1).unwrap(),
                    }),
                    LocalStateMutation::Obligation(ObligationMutation {
                        obligation_id: "b022-permission".to_string(),
                        record: permission_obligation(ObligationStateRecord::Completed),
                        pending: None,
                        expected: RevisionGuard::Expected(Revision::new(0).unwrap()),
                        revision: Revision::new(1).unwrap(),
                    }),
                ],
            })
            .await;
        let committed = matches!(boundary, "after_commit_before_readback" | "committed");
        match boundary {
            "after_commit_before_readback" => {
                assert!(matches!(
                    result,
                    Err(CommitBatchError::OutcomeUnknown { .. })
                ))
            }
            "committed" => assert!(matches!(result, Ok(CommitBatchResult::Committed(_)))),
            _ => assert!(matches!(
                result,
                Err(CommitBatchError::StorageUnavailable { .. })
            )),
        }
        assert_b022_terminal_projection(&store, committed).await;

        drop(store);
        let reopened = LocalEventStore::open(LocalEventStoreConfig::production(
            directory.path().to_path_buf(),
        ))
        .unwrap();
        assert_b022_terminal_projection(&reopened, committed).await;
    }
}

#[tokio::test]
async fn b026_all_terminal_contender_orders_keep_one_complete_winner_across_reload() {
    fn permutations(values: &mut [usize], from: usize, output: &mut Vec<Vec<usize>>) {
        if from == values.len() {
            output.push(values.to_vec());
            return;
        }
        for index in from..values.len() {
            values.swap(from, index);
            permutations(values, from + 1, output);
            values.swap(from, index);
        }
    }

    let directory = tempfile::tempdir().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
    ))
    .unwrap();
    let contenders = ["stop", "watchdog", "close", "fatal", "completion"];
    let mut order = [0, 1, 2, 3, 4];
    let mut orders = Vec::new();
    permutations(&mut order, 0, &mut orders);
    assert_eq!(orders.len(), 120);
    let mut winners = Vec::with_capacity(orders.len());

    for (case_index, permutation) in orders.iter().enumerate() {
        let session_id = format!("b026-session-{case_index}");
        let winner = contenders[permutation[0]];
        winners.push((session_id.clone(), winner));
        for (attempt_index, contender_index) in permutation.iter().copied().enumerate() {
            let contender = contenders[contender_index];
            let result = agent_terminal_result(
                &session_id,
                "1",
                "agent-1",
                TurnResult::Failed {
                    error: contender.to_string(),
                    token_usage: None,
                },
            );
            let batch_identity = format!("b026-{case_index}-{attempt_index}-{contender}");
            let committed = store
                .commit_batch(LocalAtomicBatch {
                    commit_id: CommitIdentity::parse(&batch_identity).unwrap(),
                    idempotency: IdempotencyBinding {
                        installation_id: store.installation_id().to_string(),
                        operation_kind: CommitOperationKind::Stop,
                        idempotency_key: batch_identity.clone(),
                        payload_hash: OperationBindingAuthority::digest(
                            store.as_ref(),
                            contender.as_bytes(),
                        ),
                    },
                    expected_heads: Vec::new(),
                    events: Vec::new(),
                    state_mutations: vec![
                        LocalStateMutation::TerminalRecord(TerminalRecordMutation {
                            session_id: session_id.clone(),
                            turn_id: "1".to_string(),
                            terminal_identity: format!("terminal-{contender}"),
                            result: result.clone(),
                            participant_digest: OperationBindingAuthority::digest(
                                store.as_ref(),
                                format!("{session_id}\0{contender}").as_bytes(),
                            ),
                        }),
                        LocalStateMutation::MessageProjection(MessageProjectionMutation {
                            session_id: session_id.clone(),
                            message_id: "agent-1".to_string(),
                            projection: agent_message_projection(
                                "agent-1",
                                contender,
                                &format!("{contender}-part"),
                                0,
                            ),
                            expected: RevisionGuard::Absent,
                            revision: Revision::new(0).unwrap(),
                        }),
                        LocalStateMutation::SessionProjection(SessionProjectionMutation {
                            session_id: session_id.clone(),
                            projection: marked_agent_session_projection(
                                &session_id,
                                if contender == "fatal" {
                                    SessionState::Error
                                } else {
                                    SessionState::Idle
                                },
                                Some(contender),
                                true,
                            ),
                            expected: RevisionGuard::Absent,
                            revision: Revision::new(0).unwrap(),
                        }),
                    ],
                })
                .await;
            if attempt_index == 0 {
                assert!(
                    matches!(committed, Ok(CommitBatchResult::Committed(_))),
                    "first contender must commit for permutation {case_index} ({contender}): {committed:?}"
                );
            } else {
                assert_eq!(committed, Err(CommitBatchError::PayloadConflict));
            }
        }

        let terminal = store
            .query(LocalEventQuery::TerminalByTurn {
                session_id: session_id.clone(),
                turn_id: "1".to_string(),
            })
            .await
            .unwrap();
        let LocalEventQueryResult::TerminalByTurn(Some(terminal)) = terminal else {
            panic!("terminal winner missing for permutation {case_index}");
        };
        assert_eq!(terminal.terminal_identity, format!("terminal-{winner}"));
        assert!(matches!(
            terminal.result,
            TerminalResultRecord::AgentTurn {
                result: AgentTurnTerminalResultRecord::Current(TurnResult::Failed {
                    ref error,
                    ..
                }),
                ..
            } if error == winner
        ));
    }

    drop(store);
    let reopened = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
    ))
    .unwrap();
    for (session_id, winner) in winners {
        let terminal = reopened
            .query(LocalEventQuery::TerminalByTurn {
                session_id: session_id.clone(),
                turn_id: "1".to_string(),
            })
            .await
            .unwrap();
        let LocalEventQueryResult::TerminalByTurn(Some(terminal)) = terminal else {
            panic!("terminal winner missing after reload for {session_id}");
        };
        assert_eq!(terminal.terminal_identity, format!("terminal-{winner}"));
        assert!(matches!(
            terminal.result,
            TerminalResultRecord::AgentTurn {
                result: AgentTurnTerminalResultRecord::Current(TurnResult::Failed {
                    ref error,
                    ..
                }),
                ..
            } if error == winner
        ));
        let message = reopened
            .query(LocalEventQuery::MessageProjectionByIdentity {
                session_id: session_id.clone(),
                message_id: "agent-1".to_string(),
            })
            .await
            .unwrap();
        assert!(matches!(
            message,
            LocalEventQueryResult::MessageProjectionByIdentity(Some(ref projection))
                if matches!(
                    &projection.projection,
                    MessageProjectionRecord::AgentMessage(message)
                        if message.content == winner
                )
        ));
        let session = reopened
            .query(LocalEventQuery::SessionProjectionByIdentity { session_id })
            .await
            .unwrap();
        assert!(matches!(
            session,
            LocalEventQueryResult::SessionProjectionByIdentity(Some(ref projection))
                if matches!(
                    &projection.projection,
                    SessionProjectionRecord::AgentSession(session)
                        if session.title.as_deref() == Some(winner)
                )
        ));
    }
}

#[derive(Default)]
struct ClosedActionExecutor {
    effects: Mutex<Vec<RecoveryEffectRequest>>,
}

impl ClosedActionExecutor {
    fn effect_count(&self) -> usize {
        self.effects.lock().unwrap().len()
    }
}

#[async_trait::async_trait]
impl RecoveryEffectExecutor for ClosedActionExecutor {
    fn supports_read_again(
        &self,
        _obligation_id: &str,
        _immutable_obligation: &ObligationRecord,
    ) -> bool {
        true
    }

    async fn execute(
        &self,
        request: &RecoveryEffectRequest,
    ) -> Result<RecoveryEffectResult, SafeOperationFailure> {
        assert!(matches!(
            &request.immutable_obligation,
            ObligationRecord::Observed {
                original,
                observation,
            } if matches!(
                original.as_ref(),
                ObligationRecord::PermissionResponse {
                    effect_identity,
                    ..
                } if observation.effect_identity == *effect_identity
            )
        ));
        if matches!(
            request.action,
            RecoveryActionKind::UseObservedResult | RecoveryActionKind::CancelIfSafe
        ) {
            assert!(request.authoritative_observation.is_some());
        }
        self.effects.lock().unwrap().push(request.clone());
        let (classification, safe_result) = match request.action {
            RecoveryActionKind::ReadAgain => (
                RecoveryResultClassification::Pending,
                "readback remains pending",
            ),
            RecoveryActionKind::RetrySameEffect => (
                RecoveryResultClassification::Succeeded,
                "same effect succeeded",
            ),
            RecoveryActionKind::UseObservedResult => (
                RecoveryResultClassification::ConfirmedNoEffect,
                "stored proof confirms no effect",
            ),
            RecoveryActionKind::CancelIfSafe => (
                RecoveryResultClassification::CancelledBeforeEffect,
                "cancelled from confirmed-no-effect proof",
            ),
            RecoveryActionKind::KeepForManualResolution => (
                RecoveryResultClassification::Unchanged,
                "retained for manual resolution",
            ),
        };
        Ok(RecoveryEffectResult {
            classification,
            safe_result: safe_result.to_string(),
            owner_mutations: Vec::new(),
            owner_batch: None,
        })
    }
}

fn recovery_usecase(
    store: &Arc<LocalEventStore>,
    executor: Arc<dyn RecoveryEffectExecutor>,
) -> RecoveryActionUsecase {
    RecoveryActionUsecase::new(
        store.clone(),
        store.clone(),
        executor,
        store.installation_id().to_string(),
    )
}

fn pending_query(owner: Option<&str>, partition: Option<PendingPartition>) -> PendingRecoveryQuery {
    PendingRecoveryQuery {
        limit: 200,
        partition,
        owner: owner.map(str::to_string),
        shutdown_plan: None,
        cursor: None,
    }
}

#[tokio::test]
async fn b037_real_store_routes_normal_workflow_closed_archived_and_unowned_owners() {
    let (_directory, store) = open_store();
    let normal_session = "normal-session";
    let workflow_session = "workflow-session";
    let workflow_record = ObligationRecord::WorkflowTurnCompletion {
        session_id: workflow_session.to_string(),
        turn_id: "7".to_string(),
        terminal_identity: "workflow-terminal-7".to_string(),
        notification_sha256: [7; 32],
        detail: WorkflowTurnCompletionObligationRecord::Pending {
            workflow_context: Box::new(crate::domain::workflow::WorkflowNodeContext {
                execution_id: "workflow-run-7".to_string(),
                node_execution_id: "node-execution-11".to_string(),
                workflow_name: "release-workflow".to_string(),
                node_name: "review".to_string(),
                attempt: 3,
                parent_node_name: None,
                parent_attempt: None,
                order: 4,
                startup_timeout_secs: None,
                startup_max_retries: None,
                stale_timeout_secs: None,
            }),
            message_id: "agent-7".to_string(),
            exit_code: 0,
            failure_signal: None,
            token_usage: None,
            interrupted: false,
        },
        state: ObligationStateRecord::ReconciliationRequired,
    };
    let record =
        |recovery_id: &str, session_id: &str| backend_reconciliation(session_id, recovery_id);
    commit_mutations(
        &store,
        "b037-owner-routing",
        vec![
            LocalStateMutation::SessionProjection(SessionProjectionMutation {
                session_id: normal_session.to_string(),
                projection: canonical_agent_session_projection(normal_session),
                expected: RevisionGuard::Absent,
                revision: Revision::new(0).unwrap(),
            }),
            LocalStateMutation::SessionProjection(SessionProjectionMutation {
                session_id: workflow_session.to_string(),
                projection: canonical_agent_session_projection(workflow_session),
                expected: RevisionGuard::Absent,
                revision: Revision::new(0).unwrap(),
            }),
            pending_obligation(
                "recovery-normal",
                normal_session,
                PendingPartition::Owner,
                1,
                record("normal-recovery", normal_session),
            ),
            pending_obligation(
                "recovery-workflow",
                workflow_session,
                PendingPartition::Owner,
                2,
                workflow_record,
            ),
            pending_obligation(
                "recovery-closed",
                "closed-session",
                PendingPartition::ClosedSession,
                3,
                record("closed-recovery", "closed-session"),
            ),
            pending_obligation(
                "recovery-archived",
                "archived-session",
                PendingPartition::ArchivedSession,
                4,
                record("archived-recovery", "archived-session"),
            ),
            pending_obligation(
                "recovery-unowned",
                "runtime-orphan-9",
                PendingPartition::UnownedRuntime,
                5,
                record("unowned-recovery", "runtime-orphan-9"),
            ),
        ],
    )
    .await;
    let recovery = recovery_usecase(&store, Arc::new(ClosedActionExecutor::default()));

    let normal = recovery
        .pending(pending_query(Some(normal_session), None))
        .await
        .unwrap();
    assert_eq!(normal.entries.len(), 1);
    assert_eq!(
        normal.entries[0].owner_target,
        PendingRecoveryOwnerTarget::Session {
            session_id: normal_session.to_string(),
        }
    );

    let workflow = recovery
        .pending(pending_query(Some(workflow_session), None))
        .await
        .unwrap();
    assert_eq!(workflow.entries.len(), 1);
    assert_eq!(
        workflow.entries[0].owner_target,
        PendingRecoveryOwnerTarget::WorkflowNode {
            execution_id: "workflow-run-7".to_string(),
            node_execution_id: "node-execution-11".to_string(),
            workflow_name: "release-workflow".to_string(),
            node_name: "review".to_string(),
            attempt: 3,
        }
    );
    let closed = recovery
        .pending(pending_query(None, Some(PendingPartition::ClosedSession)))
        .await
        .unwrap();
    assert_eq!(closed.entries.len(), 1);
    assert!(matches!(
        closed.entries[0].owner_target,
        PendingRecoveryOwnerTarget::ClosedSession { .. }
    ));
    let archived = recovery
        .pending(pending_query(None, Some(PendingPartition::ArchivedSession)))
        .await
        .unwrap();
    assert_eq!(archived.entries.len(), 1);
    assert!(matches!(
        archived.entries[0].owner_target,
        PendingRecoveryOwnerTarget::ArchivedSession { .. }
    ));
    let unowned = recovery
        .pending(pending_query(None, Some(PendingPartition::UnownedRuntime)))
        .await
        .unwrap();
    assert_eq!(unowned.entries.len(), 1);
    assert!(matches!(
        unowned.entries[0].owner_target,
        PendingRecoveryOwnerTarget::UnownedRuntime { .. }
    ));
}

#[tokio::test]
async fn b037_startup_send_recovery_skips_non_owner_partitions_after_restart() {
    async fn snapshot(
        store: &Arc<LocalEventStore>,
        cases: &[(&str, &str, PendingPartition, Option<SessionState>)],
    ) -> Vec<LocalEventQueryResult> {
        let mut results = Vec::with_capacity(cases.len() * 4);
        for (label, session_id, _, _) in cases {
            let operation_id = format!("b037-{label}-send");
            for query in [
                LocalEventQuery::OperationByIdentity {
                    kind: OperationKind::Send,
                    operation_id: operation_id.clone(),
                },
                LocalEventQuery::ObligationByIdentity {
                    obligation_id: format!("{operation_id}.establish"),
                },
                LocalEventQuery::ObligationByIdentity {
                    obligation_id: format!("{operation_id}.exec"),
                },
                LocalEventQuery::SessionProjectionByIdentity {
                    session_id: (*session_id).to_string(),
                },
            ] {
                results.push(store.query(query).await.unwrap());
            }
        }
        results
    }

    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().to_path_buf();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(root.clone())).unwrap();
    let cases = [
        (
            "closed",
            "b037-closed-session",
            PendingPartition::ClosedSession,
            Some(SessionState::Closed),
        ),
        (
            "archived",
            "b037-archived-session",
            PendingPartition::ArchivedSession,
            Some(SessionState::Archived),
        ),
        (
            "unowned",
            "b037-unowned-runtime",
            PendingPartition::UnownedRuntime,
            None,
        ),
    ];
    let mut mutations = Vec::new();
    for (ordinal, (label, session_id, partition, session_state)) in cases.iter().enumerate() {
        let operation_id = format!("b037-{label}-send");
        let establish_obligation_id = format!("{operation_id}.establish");
        let execution_obligation_id = format!("{operation_id}.exec");
        if let Some(session_state) = session_state {
            mutations.push(LocalStateMutation::SessionProjection(
                SessionProjectionMutation {
                    session_id: (*session_id).to_string(),
                    projection: canonical_agent_session_projection_with_state(
                        session_id,
                        session_state.clone(),
                    ),
                    expected: RevisionGuard::Absent,
                    revision: Revision::new(0).unwrap(),
                },
            ));
        }
        mutations.push(LocalStateMutation::OperationRecord(
            OperationRecordMutation {
                kind: OperationKind::Send,
                operation_id: operation_id.clone(),
                receipt: OperationReceiptRecord::Send {
                    operation_id: operation_id.clone(),
                    session_id: (*session_id).to_string(),
                    input_ref: execution_obligation_id.clone(),
                    disposition: SendDisposition::StartedTurn {
                        turn_id: "1".to_string(),
                    },
                    authentication: RecordAuthentication {
                        principal_mac: [1; 32],
                        binding_hmac: [2; 32],
                    },
                },
                latest_status: OperationStatusRecord {
                    kind: OperationKind::Send,
                    value: OperationStatusValue::AwaitingProviderStart {
                        dependency_obligation_ids: vec![establish_obligation_id.clone()],
                    },
                },
                expected: RevisionGuard::Absent,
                revision: Revision::new(0).unwrap(),
            },
        ));
        for (offset, (obligation_id, kind, dependencies)) in [
            (
                establish_obligation_id.clone(),
                SendObligationKindRecord::ProviderEstablish,
                Vec::new(),
            ),
            (
                execution_obligation_id,
                SendObligationKindRecord::TurnExecution,
                vec![establish_obligation_id],
            ),
        ]
        .into_iter()
        .enumerate()
        {
            mutations.push(pending_obligation(
                &obligation_id,
                session_id,
                *partition,
                ordinal * 2 + offset,
                ObligationRecord::Send {
                    obligation_id: obligation_id.clone(),
                    operation_id: operation_id.clone(),
                    session_id: (*session_id).to_string(),
                    kind,
                    disposition: SendObligationDispositionRecord::StartedTurn,
                    human_message_id: Some(format!("b037-{label}-human")),
                    assistant_message_id: Some(format!("b037-{label}-assistant")),
                    reserved_turn_id: None,
                    turn_id: Some("1".to_string()),
                    dependency_obligation_ids: dependencies,
                    canonical_payload: format!(r#"{{"case":"{label}"}}"#),
                    state: ObligationStateRecord::Pending,
                },
            ));
        }
    }
    commit_mutations(&store, "b037-non-owner-startup-send", mutations).await;
    let before_restart = snapshot(&store, &cases).await;
    drop(store);

    let reopened = LocalEventStore::open(LocalEventStoreConfig::production(root)).unwrap();
    assert_eq!(snapshot(&reopened, &cases).await, before_restart);
    let gate = Arc::new(RecordingStartupSendGate::default());
    let recovery = AgentSendOperationUsecase::new(
        reopened.clone(),
        reopened.clone(),
        gate.clone(),
        reopened.installation_id().to_string(),
    );

    assert_eq!(
        recovery
            .recover_pending_provider_effects_pass()
            .await
            .unwrap(),
        0
    );
    assert_eq!(gate.effect_count(), 0, "provider turn start must stay at 0");
    assert_eq!(snapshot(&reopened, &cases).await, before_restart);

    for (label, session_id, partition, expected_state) in cases {
        let operation_id = format!("b037-{label}-send");
        for obligation_id in [
            format!("{operation_id}.establish"),
            format!("{operation_id}.exec"),
        ] {
            let LocalEventQueryResult::ObligationByIdentity(Some(obligation)) = reopened
                .query(LocalEventQuery::ObligationByIdentity { obligation_id })
                .await
                .unwrap()
            else {
                panic!("{label}: pending obligation disappeared");
            };
            assert_eq!(
                obligation.pending.map(|pending| pending.partition),
                Some(partition),
                "{label}: recovery partition changed"
            );
        }
        let projection = reopened
            .query(LocalEventQuery::SessionProjectionByIdentity {
                session_id: session_id.to_string(),
            })
            .await
            .unwrap();
        match expected_state {
            Some(expected_state) => {
                let LocalEventQueryResult::SessionProjectionByIdentity(Some(projection)) =
                    projection
                else {
                    panic!("{label}: session projection disappeared");
                };
                let projection =
                    crate::adaptor::gateway::agent_session::session_storage::AgentSessionProjectionCodecV1
                        .decode(&projection.projection)
                        .unwrap();
                assert_eq!(projection.meta.state, expected_state);
            }
            None => assert!(matches!(
                projection,
                LocalEventQueryResult::SessionProjectionByIdentity(None)
            )),
        }
    }
}

#[tokio::test]
async fn b090_real_store_decoded_public_pages_keep_exact_shutdown_plan_association() {
    let (_directory, store) = open_store();
    let selected = ShutdownPlanKey {
        shutdown_id: "b090-plan-1".to_string(),
    };
    let mut mutations = Vec::with_capacity(204);
    let mut associated = |obligation_id: String, ordinal: usize, plan: ShutdownPlanKey| {
        let LocalStateMutation::Obligation(mut mutation) = pending_obligation(
            &obligation_id,
            "b090-owner",
            PendingPartition::Owner,
            ordinal,
            backend_reconciliation("b090-owner", &obligation_id),
        ) else {
            unreachable!("pending helper returns obligation mutation");
        };
        mutation.pending.as_mut().unwrap().shutdown_plan = Some(plan);
        mutations.push(LocalStateMutation::Obligation(mutation));
    };
    for ordinal in 0..201 {
        associated(
            format!("b090-selected-{ordinal:03}"),
            ordinal,
            selected.clone(),
        );
    }
    associated(
        "b090-other-shutdown".to_string(),
        201,
        ShutdownPlanKey {
            shutdown_id: "b090-plan-other".to_string(),
        },
    );
    associated(
        "b090-other-plan".to_string(),
        202,
        ShutdownPlanKey {
            shutdown_id: "b090-plan-2".to_string(),
        },
    );
    mutations.push(pending_obligation(
        "b090-unassociated",
        "b090-owner",
        PendingPartition::Owner,
        203,
        backend_reconciliation("b090-owner", "b090-unassociated"),
    ));
    commit_mutations(&store, "b090-seed", mutations).await;

    let recovery = recovery_usecase(&store, Arc::new(ClosedActionExecutor::default()));
    let first = recovery
        .pending(PendingRecoveryQuery {
            limit: 200,
            partition: None,
            owner: None,
            shutdown_plan: Some(selected.clone()),
            cursor: None,
        })
        .await
        .unwrap();
    assert_eq!(first.entries.len(), 200);
    assert!(first
        .entries
        .iter()
        .all(|entry| entry.shutdown_plan.as_ref() == Some(&selected)));
    let cursor = first
        .next_cursor
        .clone()
        .expect("selected entry 201 cursor");
    let second = recovery
        .pending(PendingRecoveryQuery {
            limit: 200,
            partition: None,
            owner: None,
            shutdown_plan: Some(selected.clone()),
            cursor: Some(cursor),
        })
        .await
        .unwrap();
    assert_eq!(second.entries.len(), 1);
    assert_eq!(second.entries[0].shutdown_plan, Some(selected));
    assert_eq!(second.entries[0].obligation_id, "b090-selected-200");
    assert!(second.next_cursor.is_none());
}

fn observed_recovery_record(
    store: &LocalEventStore,
    obligation_id: &str,
    include_observation: bool,
) -> ObligationRecord {
    let safe_view = "provider confirms the effect never started";
    let effect_identity = format!("permission-response:{obligation_id}");
    let canonical = serde_json::to_vec(&serde_json::json!({
        "schema": "authoritative_effect_observation_v1",
        "effect_identity": effect_identity,
        "origin_revision": 0,
        "classification": "confirmed_no_effect",
        "cancellable": true,
        "safe_view": safe_view,
    }))
    .unwrap();
    let original = ObligationRecord::PermissionResponse {
        operation_id: obligation_id.to_string(),
        effect_identity: effect_identity.clone(),
        session_id: "action-owner".to_string(),
        turn_id: "1".to_string(),
        response: PermissionResponse {
            request_id: format!("request-{obligation_id}"),
            decision: PermissionResponseDecision::Deny { message: None },
        },
        owner_access: true,
        from_runtime_state: true,
        state: ObligationStateRecord::Pending,
    };
    if !include_observation {
        return original;
    }
    ObligationRecord::Observed {
        original: Box::new(original),
        observation: AuthoritativeEffectObservationRecord {
            effect_identity,
            origin_revision: 0,
            classification: RecoveryResultClassification::ConfirmedNoEffect,
            cancellable: true,
            safe_view: safe_view.to_string(),
            result_sha256: OperationBindingAuthority::digest(store, &canonical),
            proof_mac: OperationBindingAuthority::mac(store, &canonical),
        },
    }
}

struct StableReadbackAfterExternalEffect {
    store: Arc<LocalEventStore>,
    fail_result_commit_once: std::sync::atomic::AtomicBool,
    provider_effects: Arc<AtomicUsize>,
    readbacks: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl RecoveryEffectExecutor for StableReadbackAfterExternalEffect {
    fn supports_read_again(
        &self,
        _obligation_id: &str,
        _immutable_obligation: &ObligationRecord,
    ) -> bool {
        true
    }

    async fn execute(
        &self,
        request: &RecoveryEffectRequest,
    ) -> Result<RecoveryEffectResult, SafeOperationFailure> {
        assert_eq!(request.action, RecoveryActionKind::ReadAgain);
        assert_eq!(request.obligation_id, "b018-stable-effect");
        assert_eq!(self.provider_effects.load(Ordering::SeqCst), 1);
        self.readbacks.fetch_add(1, Ordering::SeqCst);
        if self.fail_result_commit_once.swap(false, Ordering::SeqCst) {
            self.store.fault_injector().arm_fail_before_commit();
        }
        Ok(RecoveryEffectResult {
            classification: RecoveryResultClassification::Succeeded,
            safe_result: "stable provider effect completed".to_string(),
            owner_mutations: Vec::new(),
            owner_batch: None,
        })
    }
}

#[tokio::test]
async fn b018_stable_external_effect_readback_resumes_after_result_save_crash_without_replay() {
    let directory = tempfile::tempdir().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
    ))
    .unwrap();
    commit_mutations(
        &store,
        "b018-seed-completed-provider-effect",
        vec![pending_obligation(
            "b018-stable-effect",
            "b018-session",
            PendingPartition::Owner,
            1,
            ObligationRecord::RecoveryReserved {
                recovery_id: "b018-recovery".to_string(),
                effect_identity: "b018-stable-effect".to_string(),
                state: ObligationStateRecord::ReconciliationRequired,
            },
        )],
    )
    .await;
    // The provider mutation happened before this recovery process. ReadAgain
    // is an authoritative observation and must never increment this counter.
    let provider_effects = Arc::new(AtomicUsize::new(1));
    let readbacks = Arc::new(AtomicUsize::new(0));
    let first_executor = Arc::new(StableReadbackAfterExternalEffect {
        store: Arc::clone(&store),
        fail_result_commit_once: std::sync::atomic::AtomicBool::new(true),
        provider_effects: Arc::clone(&provider_effects),
        readbacks: Arc::clone(&readbacks),
    });
    let first_usecase = recovery_usecase(&store, first_executor.clone());
    let page = first_usecase
        .pending(pending_query(Some("b018-session"), None))
        .await
        .unwrap();
    let identity = page.entries[0]
        .action_identities
        .iter()
        .find(|identity| identity.action == RecoveryActionKind::ReadAgain)
        .unwrap();
    let request = RecoveryActionRequest {
        action_id: identity.action_id.clone(),
        obligation_id: "b018-stable-effect".to_string(),
        origin_revision: identity.origin_revision,
        action: RecoveryActionKind::ReadAgain,
    };

    assert!(matches!(
        first_usecase.request(request.clone()).await,
        Err(RecoveryActionError::StorageUnavailable { .. })
    ));
    assert_eq!(provider_effects.load(Ordering::SeqCst), 1);
    assert_eq!(readbacks.load(Ordering::SeqCst), 1);
    assert!(matches!(
        first_usecase
            .get_action_status(&request.action_id)
            .await
            .unwrap(),
        RecoveryActionStatus::InProgress { .. }
    ));

    drop(first_usecase);
    drop(first_executor);
    drop(store);
    let reopened = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
    ))
    .unwrap();
    let restarted_executor = Arc::new(StableReadbackAfterExternalEffect {
        store: Arc::clone(&reopened),
        fail_result_commit_once: std::sync::atomic::AtomicBool::new(false),
        provider_effects: Arc::clone(&provider_effects),
        readbacks: Arc::clone(&readbacks),
    });
    let restarted = recovery_usecase(&reopened, restarted_executor);
    let completed = restarted.request(request.clone()).await.unwrap();
    let RecoveryActionOutcome::Completed { action_id, result } = completed.clone() else {
        panic!("same readback action must complete after restart: {completed:?}");
    };
    assert_eq!(action_id, request.action_id);
    assert_eq!(result.outcome, RecoveryActionResultOutcome::Terminal);
    assert_eq!(
        result.classification,
        RecoveryResultClassification::Succeeded
    );
    assert_eq!(result.resource_view, "stable provider effect completed");
    assert_eq!(provider_effects.load(Ordering::SeqCst), 1);
    assert_eq!(readbacks.load(Ordering::SeqCst), 2);
    assert!(restarted
        .pending(pending_query(Some("b018-session"), None))
        .await
        .unwrap()
        .entries
        .is_empty());

    assert_eq!(restarted.request(request).await.unwrap(), completed);
    assert_eq!(provider_effects.load(Ordering::SeqCst), 1);
    assert_eq!(readbacks.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn b081_real_store_executes_all_five_closed_actions_and_rejects_unoffered_cancel() {
    let (_directory, store) = open_store();
    let action_kinds = [
        RecoveryActionKind::ReadAgain,
        RecoveryActionKind::RetrySameEffect,
        RecoveryActionKind::UseObservedResult,
        RecoveryActionKind::CancelIfSafe,
        RecoveryActionKind::KeepForManualResolution,
    ];
    let mut mutations = Vec::new();
    for (ordinal, action) in action_kinds.iter().enumerate() {
        let obligation_id = format!("closed-action-{ordinal}");
        mutations.push(pending_obligation(
            &obligation_id,
            "action-owner",
            PendingPartition::Owner,
            ordinal,
            observed_recovery_record(&store, &obligation_id, true),
        ));
        let _ = action;
    }
    let unavailable_id = "cancel-unavailable";
    mutations.push(pending_obligation(
        unavailable_id,
        "action-owner",
        PendingPartition::Owner,
        10,
        observed_recovery_record(&store, unavailable_id, false),
    ));
    commit_mutations(&store, "b081-five-actions", mutations).await;

    let executor = Arc::new(ClosedActionExecutor::default());
    let recovery = recovery_usecase(&store, executor.clone());
    let page = recovery
        .pending(pending_query(Some("action-owner"), None))
        .await
        .unwrap();
    for ordinal in 0..action_kinds.len() {
        let obligation_id = format!("closed-action-{ordinal}");
        let entry = page
            .entries
            .iter()
            .find(|entry| entry.obligation_id == obligation_id)
            .unwrap();
        assert_eq!(entry.actions, action_kinds);
        let action = action_kinds[ordinal];
        let identity = entry
            .action_identities
            .iter()
            .find(|identity| identity.action == action)
            .unwrap();
        let outcome = recovery
            .request(RecoveryActionRequest {
                action_id: identity.action_id.clone(),
                obligation_id: obligation_id.clone(),
                origin_revision: identity.origin_revision,
                action,
            })
            .await
            .unwrap();
        let RecoveryActionOutcome::Completed { result, .. } = outcome else {
            panic!("closed action did not complete: {outcome:?}");
        };
        let (expected_outcome, expected_classification) = match action {
            RecoveryActionKind::ReadAgain => (
                RecoveryActionResultOutcome::Pending,
                RecoveryResultClassification::Pending,
            ),
            RecoveryActionKind::RetrySameEffect => (
                RecoveryActionResultOutcome::Terminal,
                RecoveryResultClassification::Succeeded,
            ),
            RecoveryActionKind::UseObservedResult => (
                RecoveryActionResultOutcome::Pending,
                RecoveryResultClassification::ConfirmedNoEffect,
            ),
            RecoveryActionKind::CancelIfSafe => (
                RecoveryActionResultOutcome::Terminal,
                RecoveryResultClassification::CancelledBeforeEffect,
            ),
            RecoveryActionKind::KeepForManualResolution => (
                RecoveryActionResultOutcome::Unchanged,
                RecoveryResultClassification::Unchanged,
            ),
        };
        assert_eq!(result.outcome, expected_outcome);
        assert_eq!(result.classification, expected_classification);
    }
    assert_eq!(executor.effect_count(), 5);

    let direct_cancel_id = derive_recovery_action_id(
        store.as_ref(),
        store.installation_id(),
        unavailable_id,
        0,
        RecoveryActionKind::CancelIfSafe,
    );
    let rejected = recovery
        .request(RecoveryActionRequest {
            action_id: direct_cancel_id.clone(),
            obligation_id: unavailable_id.to_string(),
            origin_revision: 0,
            action: RecoveryActionKind::CancelIfSafe,
        })
        .await
        .unwrap();
    assert_eq!(
        rejected,
        RecoveryActionOutcome::Rejected {
            action_id: direct_cancel_id,
            rejection: RecoveryActionRejection::ActionUnavailable,
        }
    );
    assert_eq!(executor.effect_count(), 5);
}

struct ReconciliationStopGate {
    interrupts: AtomicUsize,
}

#[async_trait::async_trait]
impl StopAdmissionGate for ReconciliationStopGate {
    async fn target_snapshot(
        &self,
        _session_id: &str,
    ) -> Result<StopTargetSnapshot, SafeOperationFailure> {
        Ok(StopTargetSnapshot {
            session_revision: 0,
            active_turn_id: "1".to_string(),
            queue_paused: false,
        })
    }

    async fn interrupt(
        &self,
        _effect: &AcceptedStopEffect,
    ) -> Result<StopEffectObservation, SafeOperationFailure> {
        self.interrupts.fetch_add(1, Ordering::SeqCst);
        Err(SafeOperationFailure::new(
            SessionOperationFailureKind::OutcomeUnknown,
            true,
            "The interrupt outcome requires reconciliation.",
            "stop-interrupt-unknown",
        ))
    }
}

#[tokio::test]
async fn b079_real_store_keeps_normal_fatal_and_close_superseded_terminal_unique_after_restart() {
    let cases = [
        (
            "normal",
            TurnResult::Completed {
                stop_reason: None,
                token_usage: None,
            },
        ),
        (
            "fatal",
            TurnResult::Failed {
                error: "fatal provider failure".to_string(),
                token_usage: None,
            },
        ),
        (
            "close",
            TurnResult::Interrupted {
                reason: TurnInterruptReason::SessionClosed,
                error: None,
            },
        ),
    ];
    for (case, turn_result) in cases {
        let (_directory, store) = open_store();
        let gate = Arc::new(ReconciliationStopGate {
            interrupts: AtomicUsize::new(0),
        });
        let stop = StopOperationUsecase::new(
            store.clone(),
            store.clone(),
            gate.clone(),
            store.installation_id().to_string(),
        );
        let session_id = format!("superseded-{case}");
        let request_id = format!("stop-request-{case}");
        let request = StopOperationRequest {
            principal: "local-app".to_string(),
            request_id: request_id.clone(),
            session_id: session_id.clone(),
            turn_id: "1".to_string(),
            expected_session_revision: 0,
        };
        let accepted = stop.request(request.clone()).await.unwrap();
        let StopCommandOutcome::Accepted { receipt, state } = accepted else {
            panic!("Stop was not durably accepted: {accepted:?}");
        };
        assert!(matches!(
            state,
            StopOperationState::ReconciliationRequired { .. }
        ));
        assert_eq!(gate.interrupts.load(Ordering::SeqCst), 1);

        let terminal = TerminalRecordMutation {
            session_id: session_id.clone(),
            turn_id: "1".to_string(),
            terminal_identity: format!("{case}-terminal-winner"),
            result: agent_terminal_result(&session_id, "1", "agent-1", turn_result),
            participant_digest: OperationBindingAuthority::digest(
                store.as_ref(),
                format!("{case}-terminal").as_bytes(),
            ),
        };
        let participants = stop
            .prepare_runtime_terminal_participants(&terminal)
            .await
            .unwrap();
        assert_eq!(participants.mutations.len(), 3);
        assert_eq!(participants.events.len(), 1);
        let stream_id = StreamId::agent_session(&session_id).unwrap();
        let head = store
            .load_stream(LoadStreamRequest {
                stream_id: stream_id.clone(),
                after: None,
                limit: 1,
            })
            .await
            .unwrap()
            .head;
        let mut mutations = vec![LocalStateMutation::TerminalRecord(terminal.clone())];
        mutations.extend(participants.mutations);
        store
            .commit_batch(LocalAtomicBatch {
                commit_id: CommitIdentity::parse(&format!("b079-{case}-terminal")).unwrap(),
                idempotency: IdempotencyBinding {
                    installation_id: store.installation_id().to_string(),
                    operation_kind: CommitOperationKind::OperationProgress,
                    idempotency_key: format!("b079-{case}-terminal"),
                    payload_hash: OperationBindingAuthority::digest(
                        store.as_ref(),
                        terminal.terminal_identity.as_bytes(),
                    ),
                },
                expected_heads: vec![ExpectedStreamHead {
                    stream_id: stream_id.clone(),
                    expected: head,
                }],
                events: participants
                    .events
                    .into_iter()
                    .map(|event| UncommittedDomainEvent {
                        stream_id: stream_id.clone(),
                        event: LocalDomainEvent::AgentSession(event),
                        occurred_at_ms: 79,
                    })
                    .collect(),
                state_mutations: mutations,
            })
            .await
            .unwrap();

        let restarted = StopOperationUsecase::new(
            store.clone(),
            store.clone(),
            gate.clone(),
            store.installation_id().to_string(),
        );
        let (saved_receipt, saved_state) = restarted
            .get_operation("local-app", &request_id)
            .await
            .unwrap();
        assert_eq!(saved_receipt, receipt);
        assert_eq!(
            saved_state,
            StopOperationState::Completed {
                resolution: StopResolution::Superseded,
            }
        );
        let replay = restarted.request(request).await.unwrap();
        assert!(matches!(
            replay,
            StopCommandOutcome::Accepted {
                state: StopOperationState::Completed {
                    resolution: StopResolution::Superseded,
                },
                ..
            }
        ));
        restarted.recover_pending_stops().await.unwrap();
        assert_eq!(gate.interrupts.load(Ordering::SeqCst), 1);

        let terminal_read = store
            .query(LocalEventQuery::TerminalByTurn {
                session_id: session_id.clone(),
                turn_id: "1".to_string(),
            })
            .await
            .unwrap();
        assert!(matches!(
            terminal_read,
            LocalEventQueryResult::TerminalByTurn(Some(ref saved))
                if saved.terminal_identity == terminal.terminal_identity
        ));
        let resolution = store
            .query(LocalEventQuery::StopResolutionByOperation {
                stop_operation_id: receipt.operation_id.clone(),
            })
            .await
            .unwrap();
        assert!(matches!(
            resolution,
            LocalEventQueryResult::StopResolutionByOperation(Some(ref saved))
                if saved.resolution == StopResolutionKind::Superseded
        ));
        let events = store
            .load_stream(LoadStreamRequest {
                stream_id,
                after: None,
                limit: 200,
            })
            .await
            .unwrap();
        assert_eq!(
            events
                .events
                .iter()
                .filter(|event| matches!(
                    event.event,
                    LoadedDomainEvent::Known(ref event)
                        if matches!(
                            event.as_ref(),
                            LocalDomainEvent::AgentSession(
                                AgentSessionDomainEvent::StopResolutionRecorded {
                                    operation_id,
                                    resolution: StopResolution::Superseded,
                                    ..
                                }
                            ) if operation_id == &receipt.operation_id
                        )
                ))
                .count(),
            1
        );
        assert!(restarted
            .prepare_runtime_terminal_participants(&terminal)
            .await
            .unwrap()
            .mutations
            .is_empty());
    }
}

fn f05_production_session_close_recovery_graph(
    store: &Arc<LocalEventStore>,
    data_dir: &std::path::Path,
) -> (RecoveryActionUsecase, Arc<SessionLifecycleOperationUsecase>) {
    let session_store = Arc::new(crate::test_support::build_session_store());
    let repository: Arc<dyn LocalEventTransactionRepository> = store.clone();
    let authority: Arc<dyn OperationBindingAuthority> = store.clone();
    session_store.set_local_event_repository(
        repository.clone(),
        store.installation_id().to_string(),
        Arc::new(
            crate::adaptor::gateway::agent_session::session_storage::AgentSessionProjectionCodecV1,
        ),
    );
    let (runtime, _controller) = crate::test_support::build_agent_runtime_usecase_with_controller(
        session_store.clone(),
        data_dir,
    );
    let operation_gate = Arc::new(RuntimeAgentSessionOperationGate::new(
        runtime.clone(),
        session_store.clone(),
        data_dir.to_path_buf(),
    ));
    let lifecycle = Arc::new(SessionLifecycleOperationUsecase::new(
        repository.clone(),
        authority.clone(),
        operation_gate.clone(),
        store.installation_id().to_string(),
    ));
    let stop = Arc::new(StopOperationUsecase::new(
        repository.clone(),
        authority.clone(),
        operation_gate.clone(),
        store.installation_id().to_string(),
    ));
    operation_gate.bind_stop_operation(Arc::downgrade(&stop));
    let send_gate = Arc::new(RuntimeSendOperationGate::new(
        runtime.clone(),
        session_store.clone(),
        data_dir.to_path_buf(),
    ));
    let send = Arc::new(AgentSendOperationUsecase::new(
        repository.clone(),
        authority.clone(),
        send_gate.clone(),
        store.installation_id().to_string(),
    ));
    operation_gate.bind_send_operation(Arc::downgrade(&send));
    send_gate.bind_status_sink(Arc::downgrade(&send));
    let permission = Arc::new(PermissionResponseOperationUsecase::new(
        repository,
        authority,
        Arc::new(RuntimePermissionResponseOperationGate::new(
            runtime.clone(),
            session_store,
        )),
        store.installation_id().to_string(),
    ));
    let executor = Arc::new(ConservativeRecoveryExecutor::new(
        stop,
        lifecycle.clone(),
        operation_gate,
        ActiveSendRecoveryContext::new(send, runtime, send_gate.current_process_claims()),
        permission,
        store.clone(),
    ));
    (
        RecoveryActionUsecase::new(
            store.clone(),
            store.clone(),
            executor,
            store.installation_id().to_string(),
        ),
        lifecycle,
    )
}

#[tokio::test]
async fn f05_production_session_close_read_again_settles_owner_operation_and_obligation() {
    // Given: the close owner projection reached Closed, but the process died
    // before the accepted operation and its effect-reserved obligation could
    // be settled.
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().to_path_buf();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(root.clone())).unwrap();
    let session_id = "f05-production-session-close";
    let operation_id = "f05-production-session-close-operation";
    let obligation_id = format!(
        "session-lifecycle-target-{}",
        hex::encode(OperationBindingAuthority::digest(
            store.as_ref(),
            format!("session-lifecycle-target/v1\0{session_id}").as_bytes(),
        )),
    );
    commit_mutations(
        &store,
        "f05-production-session-close-crash-state",
        vec![
            LocalStateMutation::SessionProjection(SessionProjectionMutation {
                session_id: session_id.to_string(),
                projection: canonical_agent_session_projection_with_state(
                    session_id,
                    SessionState::Closed,
                ),
                expected: RevisionGuard::Absent,
                revision: Revision::new(0).unwrap(),
            }),
            LocalStateMutation::SessionLifecycleOperation(OperationRecordMutation {
                kind: OperationKind::SessionLifecycle,
                operation_id: operation_id.to_string(),
                receipt: OperationReceiptRecord::SessionLifecycle {
                    operation_id: operation_id.to_string(),
                    session_id: session_id.to_string(),
                    action: crate::domain::local_event::SessionLifecycleRecordAction::Close,
                    first_accepted_revision: 0,
                    commit_operation_kind: CommitOperationKind::SessionLifecycle,
                    authentication: RecordAuthentication {
                        principal_mac: [5; 32],
                        binding_hmac: [6; 32],
                    },
                },
                latest_status: OperationStatusRecord {
                    kind: OperationKind::SessionLifecycle,
                    value: OperationStatusValue::Accepted,
                },
                expected: RevisionGuard::Absent,
                revision: Revision::new(0).unwrap(),
            }),
            pending_obligation(
                &obligation_id,
                session_id,
                PendingPartition::Owner,
                0,
                ObligationRecord::SessionClose {
                    obligation_id: obligation_id.clone(),
                    operation_id: operation_id.to_string(),
                    session_id: session_id.to_string(),
                    action: crate::domain::local_event::SessionLifecycleRecordAction::Close,
                    state: ObligationStateRecord::EffectReserved,
                },
            ),
        ],
    )
    .await;
    drop(store);

    let reopened = LocalEventStore::open(LocalEventStoreConfig::production(root)).unwrap();
    let (recovery, _lifecycle) =
        f05_production_session_close_recovery_graph(&reopened, directory.path());
    let pending = recovery
        .pending(pending_query(Some(session_id), None))
        .await
        .unwrap();
    let read_again = pending
        .entries
        .iter()
        .flat_map(|entry| &entry.action_identities)
        .find(|identity| identity.action == RecoveryActionKind::ReadAgain)
        .expect("the production session-close obligation must expose ReadAgain");
    let request = RecoveryActionRequest {
        action_id: read_again.action_id.clone(),
        obligation_id: obligation_id.clone(),
        origin_revision: read_again.origin_revision,
        action: RecoveryActionKind::ReadAgain,
    };

    // When
    let completed = recovery.request(request.clone()).await.unwrap();

    // Then: the concrete production readback must use the durable owner fact
    // and publish one terminal result. The operation and obligation must be
    // settled by the same recovery finish transaction, without replaying the
    // runtime-close effect.
    let RecoveryActionOutcome::Completed { result, .. } = &completed else {
        panic!("session-close ReadAgain did not complete: {completed:?}");
    };
    assert_eq!(
        result.outcome,
        RecoveryActionResultOutcome::Terminal,
        "a Closed owner projection is authoritative completion evidence"
    );
    assert!(
        matches!(
            result.classification,
            RecoveryResultClassification::Succeeded
                | RecoveryResultClassification::ConfirmedNoEffect
        ),
        "the concrete readback must return a terminal close classification"
    );
    let terminal_classification = result.classification;

    let operation = reopened
        .query(LocalEventQuery::OperationByIdentity {
            kind: OperationKind::SessionLifecycle,
            operation_id: operation_id.to_string(),
        })
        .await
        .unwrap();
    assert!(matches!(
        operation,
        LocalEventQueryResult::OperationByIdentity(Some(ref operation))
            if operation.latest_status.value == OperationStatusValue::Completed
    ));
    let obligation = reopened
        .query(LocalEventQuery::ObligationByIdentity {
            obligation_id: obligation_id.clone(),
        })
        .await
        .unwrap();
    assert!(matches!(
        obligation,
        LocalEventQueryResult::ObligationByIdentity(Some(ref obligation))
            if obligation.pending.is_none()
                && matches!(
                    &obligation.record,
                    ObligationRecord::RecoveryTransition {
                        recovery_action,
                        ..
                    } if recovery_action.state == ObligationStateRecord::Completed
                        && recovery_action.classification
                            == Some(terminal_classification)
                )
    ));
    let projection = reopened
        .query(LocalEventQuery::SessionProjectionByIdentity {
            session_id: session_id.to_string(),
        })
        .await
        .unwrap();
    let LocalEventQueryResult::SessionProjectionByIdentity(Some(projection)) = projection else {
        panic!("session-close owner projection disappeared");
    };
    let projection =
        crate::adaptor::gateway::agent_session::session_storage::AgentSessionProjectionCodecV1
            .decode(&projection.projection)
            .unwrap();
    assert_eq!(projection.meta.state, SessionState::Closed);
    assert_eq!(recovery.request(request).await.unwrap(), completed);
}
