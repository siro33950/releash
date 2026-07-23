//! Durable pending-obligation discovery and explicitly classified recovery.

use std::sync::Arc;

use base64::Engine;
use sha2::Digest;

use crate::domain::agent_session::events::{
    RecoveryActionKind, RecoveryResultClassification, SendDisposition,
};
use crate::domain::local_event::{
    AuthoritativeEffectObservationRecord, CommitBatchError, CommitBatchResult, CommitIdentity,
    CommitOperationKind, IdempotencyBinding, LegacyReconciliationRecord, LocalAtomicBatch,
    LocalEventQuery, LocalEventQueryError, LocalEventQueryResult, LocalEventTransactionRepository,
    LocalStateMutation, ObligationMutation, ObligationRecord, ObligationRecoveryActionRecord,
    ObligationStateRecord, ObligationView, OperationReceiptRecord, OperationStatusValue,
    PendingIndexEntry, PendingPartition, QueryCursor, RecoveryActionMutation,
    RecoveryActionResultRecord, RecoveryActionView, RecoveryAttemptRecord,
    RecoveryResourceViewRecord, RecoveryResultOutcomeRecord, RecoveryResultRecord, Revision,
    RevisionGuard, SafeOperationFailure, SendObligationDispositionRecord, SendObligationKindRecord,
    SessionLifecycleRecordAction, SessionProjectionOwnerState, ShutdownTargetRecord,
    StopResolutionKind, TerminalRecordMutation, TerminalResultRecord,
    WorkflowTurnCompletionObligationRecord,
};

use super::identity::{constant_time_eq_32, validate_operation_identity};
use super::ports::{
    AuthoritativeEffectObservation, OperationBindingAuthority, RecoveryEffectExecutor,
    RecoveryEffectHandoff, RecoveryEffectRequest, RecoveryOwnerBatch,
};

#[derive(Debug, Clone)]
pub struct PendingRecoveryQuery {
    pub limit: usize,
    pub partition: Option<PendingPartition>,
    /// Exact backend owner filter.  Session supervision uses this instead of
    /// taking a global first page and filtering it in the client.
    pub owner: Option<String>,
    /// Exact current-inventory shutdown association. It is mutually
    /// exclusive with owner and partition filters.
    pub shutdown_plan: Option<crate::domain::local_event::ShutdownPlanKey>,
    pub cursor: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PendingRecoveryEntry {
    pub obligation_id: String,
    /// Closed semantic category decoded from the immutable obligation
    /// record. Unknown or incompatible records remain visible without being
    /// guessed into one of the executable categories.
    pub category: PendingRecoveryCategory,
    /// The operation/effect identity stored by the originating command. For
    /// incompatible records this falls back to the durable obligation ID;
    /// it is never reconstructed from current session state.
    pub original_identity: String,
    pub owner: String,
    /// Backend-owned display/routing target. Clients render this closed
    /// target and never infer workflow ownership from an opaque owner ID.
    pub owner_target: PendingRecoveryOwnerTarget,
    pub partition: PendingPartition,
    pub shutdown_plan: Option<crate::domain::local_event::ShutdownPlanKey>,
    pub revision: u64,
    /// Closed backend resource state.  In particular, an incompatible
    /// permission response is exposed as `Failed` rather than a retryable
    /// pending effect.
    pub state: RecoveryResourceState,
    /// Exact known lifecycle from the stored record. Missing or unsupported
    /// lifecycle values stay `Unknown`; discovery never promotes them to an
    /// inferred terminal or idle state.
    pub known_status: PendingRecoveryKnownStatus,
    pub safe_label: String,
    pub actions: Vec<RecoveryActionKind>,
    /// Backend-issued identities for the actions above.  Clients must echo
    /// one of these identities; they never mint a recovery action ID.
    pub action_identities: Vec<RecoveryActionIdentity>,
    /// Internal source cursor immediately after this entry. Public
    /// presentation uses it only when the exact encoded byte bound truncates
    /// a multi-entry page; it is never serialized as an entry field.
    pub(crate) continuation_cursor: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PendingRecoveryOwnerTarget {
    Session {
        session_id: String,
    },
    WorkflowExecution {
        execution_id: String,
    },
    WorkflowNode {
        execution_id: String,
        node_execution_id: String,
        workflow_name: String,
        node_name: String,
        attempt: u32,
    },
    ClosedSession {
        session_id: String,
    },
    ArchivedSession {
        session_id: String,
    },
    UnownedRuntime {
        runtime_id: String,
    },
    UnknownOwner {
        owner: String,
    },
}

fn bounded_owner_component(value: &str) -> Option<String> {
    (!value.is_empty() && value.len() <= 512).then(|| value.to_string())
}

fn original_obligation(record: &ObligationRecord) -> &ObligationRecord {
    match record {
        ObligationRecord::RecoveryTransition { original, .. }
        | ObligationRecord::Observed { original, .. } => original_obligation(original),
        ObligationRecord::Send { .. }
        | ObligationRecord::PermissionResponse { .. }
        | ObligationRecord::StopInterrupt { .. }
        | ObligationRecord::SessionClose { .. }
        | ObligationRecord::BackendSessionRecovery { .. }
        | ObligationRecord::WorkflowShutdown { .. }
        | ObligationRecord::WorkflowTurnCompletion { .. }
        | ObligationRecord::RecoveryPublication { .. }
        | ObligationRecord::LegacyReconciliation { .. }
        | ObligationRecord::ProviderEstablish { .. }
        | ObligationRecord::TurnExecution { .. }
        | ObligationRecord::TerminalCommit { .. }
        | ObligationRecord::RecoveryReserved { .. }
        | ObligationRecord::RecoveryCompleted { .. }
        | ObligationRecord::FeedbackReservation { .. }
        | ObligationRecord::Feedback { .. }
        | ObligationRecord::WorkflowExecution { .. } => record,
    }
}

fn workflow_node_owner_target(record: &ObligationRecord) -> Option<PendingRecoveryOwnerTarget> {
    let ObligationRecord::WorkflowTurnCompletion {
        detail:
            WorkflowTurnCompletionObligationRecord::Pending {
                workflow_context, ..
            },
        ..
    } = original_obligation(record)
    else {
        return None;
    };
    let context = workflow_context.as_ref();
    let execution_id = bounded_owner_component(&context.execution_id)?;
    let node_execution_id = bounded_owner_component(&context.node_execution_id)?;
    let workflow_name = bounded_owner_component(&context.workflow_name)?;
    let node_name = bounded_owner_component(&context.node_name)?;
    Some(PendingRecoveryOwnerTarget::WorkflowNode {
        execution_id,
        node_execution_id,
        workflow_name,
        node_name,
        attempt: context.attempt,
    })
}

/// Public pending-recovery categories. These mirror the closed obligation
/// family while retaining `Unknown` for incompatible/older local records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PendingRecoveryCategory {
    TurnExecution,
    QueueExecution,
    PermissionDelivery,
    ProviderEstablish,
    TerminalCommit,
    BackendRecovery,
    SessionClose,
    WorkflowShutdown,
    RecoveryPublication,
    Unknown,
}

/// Bounded, non-inferential lifecycle exposed by startup discovery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PendingRecoveryKnownStatus {
    Prepared,
    Pending,
    EffectReserved,
    Running,
    WaitingApproval,
    ReconciliationRequired,
    Failed,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryResourceState {
    Pending,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryActionIdentity {
    pub action_id: String,
    pub action: RecoveryActionKind,
    pub origin_revision: u64,
}

#[derive(Debug, Clone)]
pub struct PendingRecoveryPage {
    pub entries: Vec<PendingRecoveryEntry>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PendingRecoverySnapshotQuery {
    pub plan: crate::domain::local_event::ShutdownPlanKey,
    pub snapshot_id: String,
    pub partition: PendingPartition,
    pub limit: usize,
    pub cursor: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RecoveryActionRequest {
    pub action_id: String,
    pub obligation_id: String,
    pub origin_revision: u64,
    pub action: RecoveryActionKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryActionResultOutcome {
    Pending,
    Terminal,
    Unchanged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryActionCompletedResult {
    pub outcome: RecoveryActionResultOutcome,
    pub classification: RecoveryResultClassification,
    pub resource_revision: u64,
    pub canonical_result_sha256: String,
    pub resource_view: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryActionRejection {
    RevisionConflict { current_revision: u64 },
    ActionUnavailable,
    TargetRevisionChanged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryActionOutcome {
    Completed {
        action_id: String,
        result: RecoveryActionCompletedResult,
    },
    InProgress {
        action_id: String,
    },
    Rejected {
        action_id: String,
        rejection: RecoveryActionRejection,
    },
    ActionOutcomeUnknown {
        action_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryActionStatus {
    InProgress {
        action_id: String,
    },
    OutcomeUnknown {
        action_id: String,
    },
    ReconciliationRequired {
        action_id: String,
        failure: SafeOperationFailure,
    },
    Completed {
        action_id: String,
        result: RecoveryActionCompletedResult,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryActionError {
    InvalidRequest,
    ShutdownInProgress,
    NotFound,
    QueryBusy,
    DeadlineExceeded,
    CursorMismatch,
    CursorExpired,
    SnapshotMismatch,
    DetailsCompacted,
    ResponseTooLarge,
    StorageUnavailable { failure: SafeOperationFailure },
    Internal { correlation_id: String },
}

pub struct RecoveryActionUsecase {
    repository: Arc<dyn LocalEventTransactionRepository>,
    authority: Arc<dyn OperationBindingAuthority>,
    executor: Arc<dyn RecoveryEffectExecutor>,
    generation_id: String,
}

fn stop_completion_operation_is_bound(
    receipt: &OperationReceiptRecord,
    status: &OperationStatusValue,
    operation_id: &str,
    session_id: &str,
    turn_id: &str,
    expected_resolution: crate::domain::agent_session::events::StopResolution,
) -> bool {
    matches!(
        (receipt, status),
        (
            OperationReceiptRecord::Stop {
                operation_id: receipt_operation_id,
                session_id: receipt_session_id,
                turn_id: receipt_turn_id,
                ..
            },
            OperationStatusValue::StopCompleted { resolution },
        ) if receipt_operation_id == operation_id
            && receipt_session_id == session_id
            && receipt_turn_id == turn_id
            && *resolution == expected_resolution
    )
}

fn send_terminal_operation_is_bound(
    receipt: &OperationReceiptRecord,
    status: &OperationStatusValue,
    operation_id: &str,
    session_id: &str,
    turn_id: &str,
    terminal_result: &crate::domain::agent_session::entities::TurnResult,
) -> bool {
    matches!(
        (receipt, status),
        (
            OperationReceiptRecord::Send {
                operation_id: receipt_operation_id,
                session_id: receipt_session_id,
                disposition:
                    crate::domain::agent_session::events::SendDisposition::StartedTurn {
                        turn_id: receipt_turn_id,
                    },
                ..
            },
            OperationStatusValue::Terminal { result },
        ) if receipt_operation_id == operation_id
            && receipt_session_id == session_id
            && receipt_turn_id == turn_id
            && result == terminal_result
    )
}

impl RecoveryActionUsecase {
    pub fn new(
        repository: Arc<dyn LocalEventTransactionRepository>,
        authority: Arc<dyn OperationBindingAuthority>,
        executor: Arc<dyn RecoveryEffectExecutor>,
        generation_id: String,
    ) -> Self {
        Self {
            repository,
            authority,
            executor,
            generation_id,
        }
    }

    async fn owner_target(
        &self,
        owner: &str,
        partition: PendingPartition,
        record: &ObligationRecord,
        owner_projection: Option<SessionProjectionOwnerState>,
    ) -> Result<PendingRecoveryOwnerTarget, RecoveryActionError> {
        match partition {
            PendingPartition::ClosedSession => {
                return Ok(PendingRecoveryOwnerTarget::ClosedSession {
                    session_id: owner.to_string(),
                })
            }
            PendingPartition::ArchivedSession => {
                return Ok(PendingRecoveryOwnerTarget::ArchivedSession {
                    session_id: owner.to_string(),
                })
            }
            PendingPartition::UnownedRuntime => {
                return Ok(PendingRecoveryOwnerTarget::UnownedRuntime {
                    runtime_id: owner.to_string(),
                })
            }
            PendingPartition::Owner => {}
        }

        if let Some(target) = workflow_node_owner_target(record) {
            return Ok(target);
        }
        let workflow_execution_id = match original_obligation(record) {
            ObligationRecord::WorkflowShutdown { execution_id, .. } => Some(execution_id.as_str()),
            ObligationRecord::WorkflowExecution { execution } => {
                Some(execution.execution_id.as_str())
            }
            ObligationRecord::Send { .. }
            | ObligationRecord::PermissionResponse { .. }
            | ObligationRecord::StopInterrupt { .. }
            | ObligationRecord::SessionClose { .. }
            | ObligationRecord::BackendSessionRecovery { .. }
            | ObligationRecord::WorkflowTurnCompletion { .. }
            | ObligationRecord::RecoveryPublication { .. }
            | ObligationRecord::LegacyReconciliation { .. }
            | ObligationRecord::ProviderEstablish { .. }
            | ObligationRecord::TurnExecution { .. }
            | ObligationRecord::TerminalCommit { .. }
            | ObligationRecord::RecoveryReserved { .. }
            | ObligationRecord::RecoveryCompleted { .. }
            | ObligationRecord::FeedbackReservation { .. }
            | ObligationRecord::Feedback { .. }
            | ObligationRecord::RecoveryTransition { .. }
            | ObligationRecord::Observed { .. } => None,
        };
        if let Some(execution_id) = workflow_execution_id.and_then(bounded_owner_component) {
            return Ok(PendingRecoveryOwnerTarget::WorkflowExecution { execution_id });
        }

        let Some(state) = owner_projection else {
            return Ok(PendingRecoveryOwnerTarget::UnknownOwner {
                owner: owner.to_string(),
            });
        };
        Ok(match state {
            SessionProjectionOwnerState::Closed => PendingRecoveryOwnerTarget::ClosedSession {
                session_id: owner.to_string(),
            },
            SessionProjectionOwnerState::Archived => PendingRecoveryOwnerTarget::ArchivedSession {
                session_id: owner.to_string(),
            },
            SessionProjectionOwnerState::Normal => PendingRecoveryOwnerTarget::Session {
                session_id: owner.to_string(),
            },
        })
    }

    pub async fn pending(
        &self,
        query: PendingRecoveryQuery,
    ) -> Result<PendingRecoveryPage, RecoveryActionError> {
        let result = self
            .repository
            .query(LocalEventQuery::PendingRecoveryPage {
                limit: query.limit,
                partition: query.partition,
                owner: query.owner,
                ordered_key_prefix: None,
                shutdown_plan: query.shutdown_plan,
                cursor: query.cursor.map(QueryCursor::from_opaque),
            })
            .await
            .map_err(map_query_error)?;
        let LocalEventQueryResult::PendingRecoveryPage(page) = result else {
            return Err(internal("pending-shape"));
        };
        if page.entries.len() != page.continuation_cursors.len() {
            return Err(internal("pending-cursor-shape"));
        }
        let next_cursor = page.next_cursor.map(|cursor| cursor.as_str().to_string());
        let mut entries = Vec::with_capacity(page.entries.len());
        for (entry, continuation_cursor) in page.entries.into_iter().zip(page.continuation_cursors)
        {
            if is_internal_feedback_reservation(&entry.record) {
                continue;
            }
            let descriptor = pending_recovery_descriptor(&entry.obligation_id, &entry.record);
            let observation = authoritative_observation(
                &entry.record,
                entry.revision.value() as u64,
                &*self.authority,
            );
            let capabilities = recovery_capabilities(
                &entry.obligation_id,
                entry.revision.value() as u64,
                &entry.record,
                observation.as_ref(),
                &*self.authority,
                &self.generation_id,
                self.executor
                    .supports_read_again(&entry.obligation_id, &entry.record),
            );
            let action_identities = match capabilities.active_action.clone() {
                Some(active) => vec![active],
                None => capabilities
                    .actions
                    .iter()
                    .copied()
                    .map(|action| RecoveryActionIdentity {
                        action_id: self.issued_action_id(
                            &entry.obligation_id,
                            entry.revision.value() as u64,
                            action,
                        ),
                        action,
                        origin_revision: entry.revision.value() as u64,
                    })
                    .collect(),
            };
            let owner_target = self
                .owner_target(
                    &entry.owner,
                    entry.partition,
                    &entry.record,
                    entry.owner_projection,
                )
                .await?;
            entries.push(PendingRecoveryEntry {
                obligation_id: entry.obligation_id,
                category: descriptor.category,
                original_identity: descriptor.original_identity,
                owner: entry.owner,
                owner_target,
                partition: entry.partition,
                shutdown_plan: entry.shutdown_plan,
                revision: entry.revision.value() as u64,
                state: capabilities.state,
                known_status: descriptor.known_status,
                safe_label: capabilities.safe_label,
                actions: capabilities.actions,
                action_identities,
                continuation_cursor: continuation_cursor.as_str().to_string(),
            });
        }
        Ok(PendingRecoveryPage {
            entries,
            next_cursor,
        })
    }

    pub async fn pending_snapshot(
        &self,
        query: PendingRecoverySnapshotQuery,
    ) -> Result<PendingRecoveryPage, RecoveryActionError> {
        let result = self
            .repository
            .query(LocalEventQuery::PendingRecoverySnapshotPage {
                plan: query.plan.clone(),
                snapshot_id: query.snapshot_id,
                partition: query.partition,
                limit: query.limit,
                cursor: query.cursor.map(QueryCursor::from_opaque),
            })
            .await
            .map_err(map_query_error)?;
        let LocalEventQueryResult::PendingRecoverySnapshotPage(page) = result else {
            return Err(internal("pending-snapshot-shape"));
        };
        if page.entries.len() != page.continuation_cursors.len() {
            return Err(internal("pending-snapshot-cursor-shape"));
        }
        let mut entries = Vec::with_capacity(page.entries.len());
        for (entry, continuation_cursor) in page.entries.into_iter().zip(page.continuation_cursors)
        {
            let ShutdownTargetRecord::RecoverySnapshot {
                obligation_id,
                owner,
                revision,
                record,
                ..
            } = entry.detail
            else {
                return Err(internal("pending-snapshot-detail"));
            };
            if is_internal_feedback_reservation(&record) {
                continue;
            }
            let descriptor = pending_recovery_descriptor(&obligation_id, &record);
            let observation = authoritative_observation(&record, revision, &*self.authority);
            let capabilities = recovery_capabilities(
                &obligation_id,
                revision,
                &record,
                observation.as_ref(),
                &*self.authority,
                &self.generation_id,
                self.executor.supports_read_again(&obligation_id, &record),
            );
            let action_identities = match capabilities.active_action.clone() {
                Some(active) => vec![active],
                None => capabilities
                    .actions
                    .iter()
                    .copied()
                    .map(|action| RecoveryActionIdentity {
                        action_id: self.issued_action_id(&obligation_id, revision, action),
                        action,
                        origin_revision: revision,
                    })
                    .collect(),
            };
            let owner_target = self
                .owner_target(&owner, entry.partition, &record, None)
                .await?;
            entries.push(PendingRecoveryEntry {
                obligation_id,
                category: descriptor.category,
                original_identity: descriptor.original_identity,
                owner,
                owner_target,
                partition: entry.partition,
                shutdown_plan: Some(query.plan.clone()),
                revision,
                state: capabilities.state,
                known_status: descriptor.known_status,
                safe_label: capabilities.safe_label,
                actions: capabilities.actions,
                action_identities,
                continuation_cursor: continuation_cursor.as_str().to_string(),
            });
        }
        Ok(PendingRecoveryPage {
            entries,
            next_cursor: page.next_cursor.map(|cursor| cursor.as_str().to_string()),
        })
    }

    pub async fn get_action(
        &self,
        action_id: &str,
    ) -> Result<Option<RecoveryActionView>, RecoveryActionError> {
        validate_operation_identity(action_id).map_err(|_| RecoveryActionError::InvalidRequest)?;
        let result = self
            .repository
            .query(LocalEventQuery::RecoveryActionByIdentity {
                action_id: action_id.to_string(),
            })
            .await
            .map_err(map_query_error)?;
        let LocalEventQueryResult::RecoveryActionByIdentity(action) = result else {
            return Err(internal("action-shape"));
        };
        Ok(action)
    }

    pub async fn get_action_status(
        &self,
        action_id: &str,
    ) -> Result<RecoveryActionStatus, RecoveryActionError> {
        let Some(saved) = self.get_action(action_id).await? else {
            return if verify_recovery_action_id(&*self.authority, &self.generation_id, action_id) {
                Ok(RecoveryActionStatus::OutcomeUnknown {
                    action_id: action_id.to_string(),
                })
            } else {
                Err(RecoveryActionError::NotFound)
            };
        };
        decode_saved_status(&saved).ok_or_else(|| internal("saved-action-status"))
    }

    fn issued_action_id(
        &self,
        obligation_id: &str,
        origin_revision: u64,
        action: RecoveryActionKind,
    ) -> String {
        derive_recovery_action_id(
            &*self.authority,
            &self.generation_id,
            obligation_id,
            origin_revision,
            action,
        )
    }

    async fn obligation(&self, obligation_id: &str) -> Result<ObligationView, RecoveryActionError> {
        let result = self
            .repository
            .query(LocalEventQuery::ObligationByIdentity {
                obligation_id: obligation_id.to_string(),
            })
            .await
            .map_err(map_query_error)?;
        let LocalEventQueryResult::ObligationByIdentity(obligation) = result else {
            return Err(internal("obligation-shape"));
        };
        obligation.ok_or(RecoveryActionError::NotFound)
    }

    async fn has_durable_read_again_owner_evidence(
        &self,
        obligation: &ObligationView,
        owner_mutations: &[LocalStateMutation],
    ) -> Result<bool, RecoveryActionError> {
        fn terminal_turn_result(
            terminal: &TerminalResultRecord,
        ) -> Option<&crate::domain::agent_session::entities::TurnResult> {
            match terminal {
                TerminalResultRecord::AgentTurn {
                    result:
                        crate::domain::local_event::AgentTurnTerminalResultRecord::Current(result),
                    ..
                }
                | TerminalResultRecord::SessionClosed { result, .. }
                | TerminalResultRecord::Stop { result, .. } => Some(result),
                TerminalResultRecord::AgentTurn {
                    result: crate::domain::local_event::AgentTurnTerminalResultRecord::Legacy { .. },
                    ..
                }
                | TerminalResultRecord::StopSuperseded { .. }
                | TerminalResultRecord::LegacyStopResolution { .. } => None,
            }
        }

        match original_obligation(&obligation.record) {
            ObligationRecord::StopInterrupt {
                operation_id,
                session_id,
                turn_id,
                ..
            } => {
                let result = self
                    .repository
                    .query(LocalEventQuery::TerminalByTurn {
                        session_id: session_id.clone(),
                        turn_id: turn_id.clone(),
                    })
                    .await
                    .map_err(map_query_error)?;
                let LocalEventQueryResult::TerminalByTurn(Some(terminal)) = result else {
                    return Ok(false);
                };
                if terminal.session_id != *session_id || terminal.turn_id != *turn_id {
                    return Ok(false);
                }
                let expected_public_resolution = match &terminal.result {
                    TerminalResultRecord::Stop {
                        operation_id: result_operation_id,
                        ..
                    } if result_operation_id == operation_id => {
                        crate::domain::agent_session::events::StopResolution::Succeeded
                    }
                    // Any already-committed winner owned by another terminal
                    // path supersedes this Stop. The StopResolution
                    // participant binds that conclusion to the winner's
                    // identity and digest below.
                    _ => crate::domain::agent_session::events::StopResolution::Superseded,
                };
                let resolution_matches = |resolution: StopResolutionKind,
                                          detail: &TerminalResultRecord| {
                    match (resolution, detail) {
                        (
                            StopResolutionKind::Succeeded,
                            TerminalResultRecord::Stop {
                                operation_id: result_operation_id,
                                ..
                            },
                        ) => {
                            result_operation_id == operation_id && detail == &terminal.result
                        }
                        (
                            StopResolutionKind::Superseded,
                            TerminalResultRecord::StopSuperseded {
                                terminal_identity,
                                terminal_result_sha256,
                            },
                        ) => {
                            terminal_identity == &terminal.terminal_identity
                                && terminal_result_sha256 == &terminal.participant_digest
                        }
                        _ => false,
                    }
                };
                let proposed_resolution = owner_mutations.iter().find_map(|mutation| {
                    let LocalStateMutation::StopResolution(resolution) = mutation else {
                        return None;
                    };
                    (resolution.stop_operation_id == *operation_id).then_some(resolution)
                });
                let resolution_is_bound = if let Some(resolution) = proposed_resolution {
                    resolution_matches(resolution.resolution, &resolution.detail)
                } else {
                    let result = self
                        .repository
                        .query(LocalEventQuery::StopResolutionByOperation {
                            stop_operation_id: operation_id.clone(),
                        })
                        .await
                        .map_err(map_query_error)?;
                    matches!(
                        result,
                        LocalEventQueryResult::StopResolutionByOperation(Some(ref resolution))
                            if resolution.stop_operation_id == *operation_id
                                && resolution_matches(
                                    resolution.resolution,
                                    &resolution.detail,
                                )
                    )
                };
                if !resolution_is_bound {
                    return Ok(false);
                }
                let proposes_operation = owner_mutations.iter().any(|mutation| {
                    matches!(
                        mutation,
                        LocalStateMutation::OperationRecord(operation)
                            if operation.kind
                                == crate::domain::local_event::OperationKind::Stop
                                && operation.operation_id == *operation_id
                                && stop_completion_operation_is_bound(
                                    &operation.receipt,
                                    &operation.latest_status.value,
                                    operation_id,
                                    session_id,
                                    turn_id,
                                    expected_public_resolution,
                                )
                    )
                });
                if proposes_operation {
                    return Ok(true);
                }
                let operation = self
                    .repository
                    .query(LocalEventQuery::OperationByIdentity {
                        kind: crate::domain::local_event::OperationKind::Stop,
                        operation_id: operation_id.clone(),
                    })
                    .await
                    .map_err(map_query_error)?;
                Ok(matches!(
                    operation,
                    LocalEventQueryResult::OperationByIdentity(Some(ref operation))
                        if stop_completion_operation_is_bound(
                            &operation.receipt,
                            &operation.latest_status.value,
                            operation_id,
                            session_id,
                            turn_id,
                            expected_public_resolution,
                        )
                ))
            }
            ObligationRecord::Send {
                operation_id,
                session_id,
                kind: SendObligationKindRecord::TurnExecution,
                turn_id,
                reserved_turn_id,
                ..
            } => {
                let Some(turn_id) = turn_id.as_ref().or(reserved_turn_id.as_ref()) else {
                    return Ok(false);
                };
                let result = self
                    .repository
                    .query(LocalEventQuery::TerminalByTurn {
                        session_id: session_id.clone(),
                        turn_id: turn_id.clone(),
                    })
                    .await
                    .map_err(map_query_error)?;
                let LocalEventQueryResult::TerminalByTurn(Some(terminal)) = result else {
                    return Ok(false);
                };
                if terminal.session_id != *session_id || terminal.turn_id != *turn_id {
                    return Ok(false);
                }
                let Some(terminal_result) = terminal_turn_result(&terminal.result) else {
                    return Ok(false);
                };
                if let Some(proposed) = owner_mutations.iter().find_map(|mutation| {
                    let LocalStateMutation::OperationRecord(operation) = mutation else {
                        return None;
                    };
                    (operation.kind == crate::domain::local_event::OperationKind::Send
                        && operation.operation_id == *operation_id)
                        .then_some(operation)
                }) {
                    return Ok(send_terminal_operation_is_bound(
                        &proposed.receipt,
                        &proposed.latest_status.value,
                        operation_id,
                        session_id,
                        turn_id,
                        terminal_result,
                    ));
                }
                let operation = self
                    .repository
                    .query(LocalEventQuery::OperationByIdentity {
                        kind: crate::domain::local_event::OperationKind::Send,
                        operation_id: operation_id.clone(),
                    })
                    .await
                    .map_err(map_query_error)?;
                Ok(matches!(
                    operation,
                    LocalEventQueryResult::OperationByIdentity(Some(ref operation))
                        if send_terminal_operation_is_bound(
                            &operation.receipt,
                            &operation.latest_status.value,
                            operation_id,
                            session_id,
                            turn_id,
                            terminal_result,
                        )
                ))
            }
            ObligationRecord::SessionClose {
                operation_id,
                session_id,
                action: SessionLifecycleRecordAction::Close,
                ..
            } => {
                let result = self
                    .repository
                    .query(LocalEventQuery::SessionProjectionByIdentity {
                        session_id: session_id.clone(),
                    })
                    .await
                    .map_err(map_query_error)?;
                let closed = matches!(
                    result,
                    LocalEventQueryResult::SessionProjectionByIdentity(Some(ref projection))
                        if projection.session_id == *session_id
                            && matches!(
                                &projection.projection,
                                crate::domain::local_event::SessionProjectionRecord::AgentSession(
                                    owner,
                                ) if owner.meta.id == *session_id
                                    && owner.meta.state
                                        == crate::domain::local_event::AgentSessionStateRecord::Closed
                            )
                );
                if !closed {
                    return Ok(false);
                }
                let proposes_operation = owner_mutations.iter().any(|mutation| {
                    matches!(
                        mutation,
                        LocalStateMutation::SessionLifecycleOperation(operation)
                            if operation.kind
                                == crate::domain::local_event::OperationKind::SessionLifecycle
                                && operation.operation_id == *operation_id
                                && matches!(
                                    operation.latest_status.value,
                                    OperationStatusValue::Completed
                                )
                    )
                });
                if proposes_operation {
                    return Ok(true);
                }
                let operation = self
                    .repository
                    .query(LocalEventQuery::OperationByIdentity {
                        kind: crate::domain::local_event::OperationKind::SessionLifecycle,
                        operation_id: operation_id.clone(),
                    })
                    .await
                    .map_err(map_query_error)?;
                Ok(matches!(
                    operation,
                    LocalEventQueryResult::OperationByIdentity(Some(ref operation))
                        if matches!(
                            &operation.receipt,
                            OperationReceiptRecord::SessionLifecycle {
                                operation_id: receipt_operation_id,
                                session_id: receipt_session_id,
                                action: SessionLifecycleRecordAction::Close,
                                ..
                            } if receipt_operation_id == operation_id
                                && receipt_session_id == session_id
                        )
                            && matches!(
                                operation.latest_status.value,
                                OperationStatusValue::Completed
                            )
                ))
            }
            ObligationRecord::BackendSessionRecovery {
                session_id,
                recovery_id,
                detail:
                    Some(
                        crate::domain::local_event::BackendSessionRecoveryObligationRecord::EffectReserved {
                            old_provider_session_generation,
                            reserved_at_bits,
                            ..
                        },
                    ),
                state: ObligationStateRecord::EffectReserved,
            } => {
                let Some(expected_generation) = old_provider_session_generation.checked_add(1)
                else {
                    return Ok(false);
                };
                let reserved_at = f64::from_bits(*reserved_at_bits);
                if !reserved_at.is_finite() {
                    return Ok(false);
                }
                let result = self
                    .repository
                    .query(LocalEventQuery::SessionProjectionByIdentity {
                        session_id: session_id.clone(),
                    })
                    .await
                    .map_err(map_query_error)?;
                Ok(matches!(
                    result,
                    LocalEventQueryResult::SessionProjectionByIdentity(Some(ref projection))
                        if projection.session_id == *session_id
                            && matches!(
                                &projection.projection,
                                crate::domain::local_event::SessionProjectionRecord::AgentSession(
                                    owner,
                                ) if owner.meta.id == *session_id
                                    && owner
                                        .meta
                                        .agent_session_id
                                        .as_ref()
                                        .is_some_and(|provider_id| !provider_id.is_empty())
                                    && owner.meta.provider_session_generation
                                        == expected_generation
                                    && owner.meta.context_reinjection_generation
                                        == Some(expected_generation)
                                    && owner
                                        .meta
                                        .recovery_publication_snapshot
                                        .as_ref()
                                        .is_some_and(|snapshot| {
                                            snapshot.recovery_id == *recovery_id
                                                && snapshot.summary.id == *session_id
                                        })
                                    && f64::from_bits(owner.meta.updated_at_bits).is_finite()
                                    && f64::from_bits(owner.meta.updated_at_bits) >= reserved_at
                            )
                ))
            }
            ObligationRecord::Send {
                kind: SendObligationKindRecord::ProviderEstablish,
                ..
            }
            | ObligationRecord::SessionClose { .. }
            | ObligationRecord::BackendSessionRecovery { .. }
            | ObligationRecord::PermissionResponse { .. }
            | ObligationRecord::WorkflowShutdown { .. }
            | ObligationRecord::WorkflowTurnCompletion { .. }
            | ObligationRecord::RecoveryPublication { .. }
            | ObligationRecord::LegacyReconciliation { .. }
            | ObligationRecord::ProviderEstablish { .. }
            | ObligationRecord::TurnExecution { .. }
            | ObligationRecord::TerminalCommit { .. }
            | ObligationRecord::RecoveryReserved { .. }
            | ObligationRecord::RecoveryCompleted { .. }
            | ObligationRecord::FeedbackReservation { .. }
            | ObligationRecord::Feedback { .. }
            | ObligationRecord::WorkflowExecution { .. }
            | ObligationRecord::RecoveryTransition { .. }
            | ObligationRecord::Observed { .. } => Ok(false),
        }
    }

    /// Resume the one effect whose durable owner has its own compare-and-set
    /// fence. A permission response may be retried while its owner state is
    /// still `pending`: `respond_permission` first advances that state to
    /// `effect_reserved`, so concurrent/restarted recovery cannot dispatch a
    /// second provider response. Once reserved, recovery only observes.
    async fn resume_pending_permission_action(
        &self,
        saved: &RecoveryActionView,
        request: &RecoveryActionRequest,
    ) -> Result<Option<RecoveryActionOutcome>, RecoveryActionError> {
        let obligation = match self.obligation(&request.obligation_id).await {
            Ok(obligation) => obligation,
            Err(RecoveryActionError::NotFound) => return Ok(None),
            Err(error) => return Err(error),
        };
        if !matches!(
            original_obligation(&obligation.record),
            ObligationRecord::PermissionResponse { .. }
        ) || !action_claim_matches_request(&obligation.record, request)
        {
            return Ok(None);
        }

        let effect = RecoveryEffectRequest {
            action_id: request.action_id.clone(),
            obligation_id: request.obligation_id.clone(),
            origin_revision: request.origin_revision,
            expected_owner: obligation
                .pending
                .as_ref()
                .map(|pending| pending.owner.clone()),
            action: request.action,
            immutable_obligation: obligation.record.clone(),
            authoritative_observation: None,
        };
        let (mut classification, mut resource_view) = match obligation_state(&obligation.record) {
            Some(ObligationStateRecord::Pending)
                if obligation.pending.is_some()
                    && matches!(
                        original_obligation(&obligation.record),
                        ObligationRecord::PermissionResponse {
                            owner_access: true,
                            ..
                        }
                    ) =>
            {
                match self.executor.execute(&effect).await {
                    Ok(result) => (result.classification, result.safe_result),
                    Err(failure) => (
                        RecoveryResultClassification::Ambiguous,
                        failure.label.value().to_string(),
                    ),
                }
            }
            Some(ObligationStateRecord::Completed) => (
                RecoveryResultClassification::Succeeded,
                "The exact permission response was delivered.".to_string(),
            ),
            // An owner-side claim is the ambiguity fence. Never replay the
            // provider effect from this state; a later exact replay can
            // finalize after owner completion becomes visible.
            Some(ObligationStateRecord::EffectReserved) => return Ok(None),
            Some(ObligationStateRecord::Pending) => return Ok(None),
            Some(ObligationStateRecord::Prepared)
            | Some(ObligationStateRecord::Running)
            | Some(ObligationStateRecord::WaitingApproval)
            | Some(ObligationStateRecord::OutcomeUnknown)
            | Some(ObligationStateRecord::ReconciliationRequired)
            | Some(ObligationStateRecord::Failed)
            | Some(ObligationStateRecord::Cancelled)
            | None => return Ok(None),
        };

        let current = self.obligation(&request.obligation_id).await?;
        if obligation_state(&current.record) == Some(ObligationStateRecord::Completed) {
            classification = RecoveryResultClassification::Succeeded;
            resource_view = "The exact permission response was delivered.".to_string();
        }
        if !classification_allowed_for_action(request.action, None, classification) {
            classification = RecoveryResultClassification::Ambiguous;
            resource_view =
                "The recovery adapter returned an incompatible result classification.".to_string();
        }
        if resource_view.len() > 64 * 1024 {
            resource_view = "The recovery result exceeded the safe public view bound.".to_string();
        }

        let outcome = result_outcome(classification);
        let keep_pending = outcome != RecoveryActionResultOutcome::Terminal;
        let mut resource_revision = current.revision.value() as u64;
        let mut mutations = Vec::with_capacity(2);
        if current.revision == obligation.revision
            && action_claim_matches(&current.record, &request.action_id)
        {
            let next_revision = current
                .revision
                .next()
                .ok_or(RecoveryActionError::InvalidRequest)?;
            resource_revision = next_revision.value() as u64;
            mutations.push(LocalStateMutation::Obligation(ObligationMutation {
                obligation_id: request.obligation_id.clone(),
                record: obligation_with_action_result(
                    &current.record,
                    &request.action_id,
                    classification,
                    None,
                )?,
                pending: if keep_pending {
                    current.pending.as_ref().map(pending_entry)
                } else {
                    None
                },
                expected: RevisionGuard::Expected(current.revision),
                revision: next_revision,
            }));
        }
        let (_, completed) = encode_recovery_completed_result(
            outcome,
            classification,
            resource_revision,
            resource_view,
        )?;
        let finish_payload_hash = finish_payload_hash(
            self.repository.as_ref(),
            &*self.authority,
            &completed,
            &[],
            None,
            None,
        )?;
        let action_revision = saved
            .revision
            .next()
            .ok_or(RecoveryActionError::InvalidRequest)?;
        mutations.insert(
            0,
            LocalStateMutation::RecoveryAction(RecoveryActionMutation {
                action_id: request.action_id.clone(),
                binding_hash: saved.binding_hash,
                attempt: completed_attempt(request),
                completed: Some(completed),
                expected: RevisionGuard::Expected(saved.revision),
                revision: action_revision,
            }),
        );
        let finish = LocalAtomicBatch {
            commit_id: commit_identity(&self.authority, "finish", &request.action_id)?,
            idempotency: IdempotencyBinding {
                generation_id: self.generation_id.clone(),
                operation_kind: CommitOperationKind::OperationProgress,
                idempotency_key: format!("{}.finish", request.action_id),
                payload_hash: finish_payload_hash,
            },
            expected_heads: Vec::new(),
            events: Vec::new(),
            state_mutations: mutations,
        };
        match self.repository.commit_batch(finish).await {
            Ok(CommitBatchResult::Committed(_) | CommitBatchResult::Replayed(_)) => {
                self.executor.after_commit(&effect, classification).await;
                let status = self.get_action_status(&request.action_id).await?;
                let RecoveryActionStatus::Completed { result, .. } = status else {
                    return Err(internal("completed-action-readback"));
                };
                Ok(Some(RecoveryActionOutcome::Completed {
                    action_id: request.action_id.clone(),
                    result,
                }))
            }
            Err(CommitBatchError::OutcomeUnknown { .. }) => {
                Ok(Some(RecoveryActionOutcome::ActionOutcomeUnknown {
                    action_id: request.action_id.clone(),
                }))
            }
            Err(CommitBatchError::StreamHeadConflict { .. }) => {
                let outcome = match self.get_action(&request.action_id).await? {
                    Some(saved) => saved_status_outcome(&saved)?,
                    None => RecoveryActionOutcome::InProgress {
                        action_id: request.action_id.clone(),
                    },
                };
                Ok(Some(outcome))
            }
            Err(error) => map_commit_error(error, &request.action_id).map(Some),
        }
    }

    /// Resume a previously reserved authoritative readback after a process
    /// crash or a failed result commit. `ReadAgain` is the only action whose
    /// effect is itself a read-only observation of an already-started stable
    /// provider identity, so repeating this call cannot repeat the provider
    /// mutation. The durable action and obligation claim remain the CAS fence
    /// for publishing the observed result exactly once.
    async fn resume_pending_read_again_action(
        &self,
        saved: &RecoveryActionView,
        request: &RecoveryActionRequest,
    ) -> Result<Option<RecoveryActionOutcome>, RecoveryActionError> {
        if request.action != RecoveryActionKind::ReadAgain || saved.completed.is_some() {
            return Ok(None);
        }
        let obligation = match self.obligation(&request.obligation_id).await {
            Ok(obligation) => obligation,
            Err(RecoveryActionError::NotFound) => return Ok(None),
            Err(error) => return Err(error),
        };
        if obligation.pending.is_none()
            || !action_claim_matches_request(&obligation.record, request)
        {
            return Ok(None);
        }
        let observation = authoritative_observation(
            &obligation.record,
            request.origin_revision,
            &*self.authority,
        );
        let effect = RecoveryEffectRequest {
            action_id: request.action_id.clone(),
            obligation_id: request.obligation_id.clone(),
            origin_revision: request.origin_revision,
            expected_owner: obligation
                .pending
                .as_ref()
                .map(|pending| pending.owner.clone()),
            action: request.action,
            immutable_obligation: obligation.record.clone(),
            authoritative_observation: observation.clone(),
        };
        // The initial handoff guard already committed this exact action and
        // target claim. On restart, the stable effect identity is the
        // authority: re-running a mutable target guard could strand a
        // completed provider effect after unrelated projection movement.
        let (mut classification, mut resource_view, mut owner_mutations, mut owner_batch) =
            match self.executor.execute(&effect).await {
                Ok(result) => (
                    result.classification,
                    result.safe_result,
                    result.owner_mutations,
                    result.owner_batch,
                ),
                Err(failure) => (
                    RecoveryResultClassification::Ambiguous,
                    failure.label.value().to_string(),
                    Vec::new(),
                    None,
                ),
            };
        if !classification_allowed_for_action(request.action, observation.as_ref(), classification)
        {
            classification = RecoveryResultClassification::Ambiguous;
            resource_view =
                "The recovery adapter returned an incompatible result classification.".to_string();
            owner_mutations.clear();
            owner_batch = None;
        }
        let current = self.obligation(&request.obligation_id).await?;
        if request.action == RecoveryActionKind::ReadAgain
            && classification == RecoveryResultClassification::Succeeded
            && requires_durable_read_again_owner_evidence(&current.record)
            && !self
                .has_durable_read_again_owner_evidence(&current, &owner_mutations)
                .await?
        {
            classification = RecoveryResultClassification::Ambiguous;
            resource_view =
                "The durable owner does not contain sufficient completion evidence.".to_string();
            owner_mutations.clear();
            owner_batch = None;
        }
        let (normalized, source_completion) =
            normalize_owner_mutations(request, &current, owner_mutations)?;
        owner_mutations = normalized;
        let source_completion =
            ensure_succeeded_source_completion(&current, classification, source_completion)?;
        sort_owner_mutations(self.repository.as_ref(), &mut owner_mutations)?;
        validate_owner_mutations(
            self.repository.as_ref(),
            request,
            &current,
            &owner_mutations,
            source_completion.as_ref(),
            owner_batch.as_ref(),
            classification,
        )?;
        validate_owner_batch(
            self.repository.as_ref(),
            request,
            &current,
            &owner_mutations,
            source_completion.as_ref(),
            owner_batch.as_ref(),
        )?;
        if resource_view.len() > 64 * 1024 {
            resource_view = "The recovery result exceeded the safe public view bound.".to_string();
        }

        if !action_claim_matches_request(&current.record, request) {
            return Ok(Some(match self.get_action(&request.action_id).await? {
                Some(current_action) => saved_status_outcome(&current_action)?,
                None => RecoveryActionOutcome::InProgress {
                    action_id: request.action_id.clone(),
                },
            }));
        }
        let outcome = result_outcome(classification);
        let keep_pending = outcome != RecoveryActionResultOutcome::Terminal;
        let next_resource_revision = current
            .revision
            .next()
            .ok_or(RecoveryActionError::InvalidRequest)?;
        let (_, completed) = encode_recovery_completed_result(
            outcome,
            classification,
            next_resource_revision.value() as u64,
            resource_view,
        )?;
        let source_result = ObligationMutation {
            obligation_id: request.obligation_id.clone(),
            record: obligation_with_action_result(
                &current.record,
                &request.action_id,
                classification,
                source_completion.as_ref().map(|source| &source.record),
            )?,
            pending: if keep_pending {
                current.pending.as_ref().map(pending_entry)
            } else {
                None
            },
            expected: RevisionGuard::Expected(current.revision),
            revision: next_resource_revision,
        };
        let finish_payload_hash = finish_payload_hash(
            self.repository.as_ref(),
            &*self.authority,
            &completed,
            &owner_mutations,
            Some(&source_result),
            owner_batch.as_ref(),
        )?;
        let action_revision = saved
            .revision
            .next()
            .ok_or(RecoveryActionError::InvalidRequest)?;
        let mut state_mutations = vec![
            LocalStateMutation::RecoveryAction(RecoveryActionMutation {
                action_id: request.action_id.clone(),
                binding_hash: saved.binding_hash,
                attempt: completed_attempt(request),
                completed: Some(completed),
                expected: RevisionGuard::Expected(saved.revision),
                revision: action_revision,
            }),
            LocalStateMutation::Obligation(source_result),
        ];
        state_mutations.append(&mut owner_mutations);
        let finish_operation_kind = recovery_finish_operation_kind(&state_mutations);
        let (expected_heads, events) = owner_batch
            .take()
            .map(|batch| (batch.expected_heads, batch.events))
            .unwrap_or_default();
        let finish = LocalAtomicBatch {
            commit_id: commit_identity(&self.authority, "finish", &request.action_id)?,
            idempotency: IdempotencyBinding {
                generation_id: self.generation_id.clone(),
                operation_kind: finish_operation_kind,
                idempotency_key: format!("{}.finish", request.action_id),
                payload_hash: finish_payload_hash,
            },
            expected_heads,
            events,
            state_mutations,
        };
        match self.repository.commit_batch(finish).await {
            Ok(CommitBatchResult::Committed(_) | CommitBatchResult::Replayed(_)) => {
                self.executor.after_commit(&effect, classification).await;
                let status = self.get_action_status(&request.action_id).await?;
                let RecoveryActionStatus::Completed { result, .. } = status else {
                    return Err(internal("completed-read-again-readback"));
                };
                Ok(Some(RecoveryActionOutcome::Completed {
                    action_id: request.action_id.clone(),
                    result,
                }))
            }
            Err(CommitBatchError::OutcomeUnknown { .. }) => {
                Ok(Some(RecoveryActionOutcome::ActionOutcomeUnknown {
                    action_id: request.action_id.clone(),
                }))
            }
            Err(CommitBatchError::StreamHeadConflict { .. }) => {
                let outcome = match self.get_action(&request.action_id).await? {
                    Some(current_action) => saved_status_outcome(&current_action)?,
                    None => RecoveryActionOutcome::InProgress {
                        action_id: request.action_id.clone(),
                    },
                };
                Ok(Some(outcome))
            }
            Err(error) => map_commit_error(error, &request.action_id).map(Some),
        }
    }

    pub async fn request(
        &self,
        request: RecoveryActionRequest,
    ) -> Result<RecoveryActionOutcome, RecoveryActionError> {
        validate_operation_identity(&request.action_id)
            .map_err(|_| RecoveryActionError::InvalidRequest)?;
        if request.obligation_id.is_empty() || request.origin_revision > i64::MAX as u64 {
            return Err(RecoveryActionError::InvalidRequest);
        }

        // Durable decisions always win over the mutable resource.  This is
        // what makes response-loss and restart replay independent from a
        // later obligation revision or compaction.
        if let Some(saved) = self.get_action(&request.action_id).await? {
            if !saved_request_matches(&saved, &request)? {
                return Err(RecoveryActionError::NotFound);
            }
            if saved.completed.is_none() && request.action == RecoveryActionKind::RetrySameEffect {
                if let Some(outcome) = self
                    .resume_pending_permission_action(&saved, &request)
                    .await?
                {
                    return Ok(outcome);
                }
            }
            if saved.completed.is_none() && request.action == RecoveryActionKind::ReadAgain {
                if let Some(outcome) = self
                    .resume_pending_read_again_action(&saved, &request)
                    .await?
                {
                    return Ok(outcome);
                }
            }
            return saved_status_outcome(&saved);
        }

        // Verify that the caller echoed the backend-issued identity before
        // disclosing whether the referenced resource exists.
        if request.action_id
            != self.issued_action_id(
                &request.obligation_id,
                request.origin_revision,
                request.action,
            )
        {
            return Err(RecoveryActionError::NotFound);
        }
        let obligation = self.obligation(&request.obligation_id).await?;
        if obligation.revision.value() != request.origin_revision as i64 {
            return Ok(RecoveryActionOutcome::Rejected {
                action_id: request.action_id,
                rejection: RecoveryActionRejection::RevisionConflict {
                    current_revision: obligation.revision.value() as u64,
                },
            });
        }
        if obligation.pending.is_none() {
            return Ok(RecoveryActionOutcome::Rejected {
                action_id: request.action_id,
                rejection: RecoveryActionRejection::ActionUnavailable,
            });
        }
        let observation = authoritative_observation(
            &obligation.record,
            request.origin_revision,
            &*self.authority,
        );
        let capabilities = recovery_capabilities(
            &request.obligation_id,
            request.origin_revision,
            &obligation.record,
            observation.as_ref(),
            &*self.authority,
            &self.generation_id,
            self.executor
                .supports_read_again(&request.obligation_id, &obligation.record),
        );
        if !capabilities.actions.contains(&request.action) {
            return Ok(RecoveryActionOutcome::Rejected {
                action_id: request.action_id,
                rejection: RecoveryActionRejection::ActionUnavailable,
            });
        }

        let effect = RecoveryEffectRequest {
            action_id: request.action_id.clone(),
            obligation_id: request.obligation_id.clone(),
            origin_revision: request.origin_revision,
            expected_owner: obligation
                .pending
                .as_ref()
                .map(|pending| pending.owner.clone()),
            action: request.action,
            immutable_obligation: obligation.record.clone(),
            authoritative_observation: observation.clone(),
        };
        match self.executor.validate_handoff(&effect).await {
            Ok(RecoveryEffectHandoff::Ready) => {}
            Ok(RecoveryEffectHandoff::TargetRevisionChanged) => {
                return Ok(RecoveryActionOutcome::Rejected {
                    action_id: request.action_id,
                    rejection: RecoveryActionRejection::TargetRevisionChanged,
                });
            }
            Err(failure) => return Err(RecoveryActionError::StorageUnavailable { failure }),
        }

        let binding_material = format!(
            "recovery-action-binding/v1\0{}\0{}\0{}\0{}\0{}",
            self.generation_id,
            request.action_id,
            request.obligation_id,
            request.origin_revision,
            action_label(request.action),
        )
        .into_bytes();
        let binding_hash = self.authority.digest(&binding_material);
        if let Some(saved) = self.get_action(&request.action_id).await? {
            if !saved_request_matches(&saved, &request)? {
                return Err(RecoveryActionError::NotFound);
            }
            return saved_status_outcome(&saved);
        }

        let pending = obligation
            .pending
            .clone()
            .expect("checked pending obligation");
        let reserved_record = obligation_with_action_claim(&obligation.record, &request)?;
        let attempt = RecoveryAttemptRecord::Obligation {
            obligation_id: request.obligation_id.clone(),
            origin_revision: request.origin_revision,
            action: request.action,
            effect_identity: request.obligation_id.clone(),
            state: ObligationStateRecord::EffectReserved,
            failure: None,
        };
        let reserved_revision = obligation
            .revision
            .next()
            .ok_or(RecoveryActionError::InvalidRequest)?;
        let reserve = LocalAtomicBatch {
            commit_id: commit_identity(&self.authority, "reserve", &request.action_id)?,
            idempotency: IdempotencyBinding {
                generation_id: self.generation_id.clone(),
                operation_kind: CommitOperationKind::Recovery,
                idempotency_key: format!("{}.reserve", request.action_id),
                payload_hash: binding_hash,
            },
            expected_heads: Vec::new(),
            events: Vec::new(),
            state_mutations: vec![
                LocalStateMutation::RecoveryAction(RecoveryActionMutation {
                    action_id: request.action_id.clone(),
                    binding_hash,
                    attempt,
                    completed: None,
                    expected: RevisionGuard::Absent,
                    revision: Revision::new(0).expect("zero revision"),
                }),
                LocalStateMutation::Obligation(ObligationMutation {
                    obligation_id: request.obligation_id.clone(),
                    record: reserved_record,
                    pending: Some(pending_entry(&pending)),
                    expected: RevisionGuard::Expected(obligation.revision),
                    revision: reserved_revision,
                }),
            ],
        };
        match self.repository.commit_batch(reserve).await {
            Ok(CommitBatchResult::Committed(_) | CommitBatchResult::Replayed(_)) => {}
            Err(CommitBatchError::OutcomeUnknown { .. }) => {
                return Ok(RecoveryActionOutcome::ActionOutcomeUnknown {
                    action_id: request.action_id,
                });
            }
            Err(CommitBatchError::StreamHeadConflict { .. }) => {
                if let Some(saved) = self.get_action(&request.action_id).await? {
                    return saved_status_outcome(&saved);
                }
                return Ok(RecoveryActionOutcome::Rejected {
                    action_id: request.action_id,
                    rejection: RecoveryActionRejection::TargetRevisionChanged,
                });
            }
            Err(CommitBatchError::PayloadConflict) => return Err(RecoveryActionError::NotFound),
            Err(error) => return map_commit_error(error, &request.action_id),
        }

        let effect_result = self.executor.execute(&effect).await;
        let (mut classification, mut resource_view, mut owner_mutations, mut owner_batch) =
            match effect_result {
                Ok(result) => (
                    result.classification,
                    result.safe_result,
                    result.owner_mutations,
                    result.owner_batch,
                ),
                Err(failure) => (
                    RecoveryResultClassification::Ambiguous,
                    failure.label.value().to_string(),
                    Vec::new(),
                    None,
                ),
            };
        if !classification_allowed_for_action(request.action, observation.as_ref(), classification)
        {
            classification = RecoveryResultClassification::Ambiguous;
            resource_view =
                "The recovery adapter returned an incompatible result classification.".to_string();
            owner_mutations.clear();
            owner_batch = None;
        }
        let current = self.obligation(&request.obligation_id).await?;
        if request.action == RecoveryActionKind::ReadAgain
            && classification == RecoveryResultClassification::Succeeded
            && requires_durable_read_again_owner_evidence(&current.record)
            && !self
                .has_durable_read_again_owner_evidence(&current, &owner_mutations)
                .await?
        {
            classification = RecoveryResultClassification::Ambiguous;
            resource_view =
                "The durable owner does not contain sufficient completion evidence.".to_string();
            owner_mutations.clear();
            owner_batch = None;
        }
        let (normalized, source_completion) =
            normalize_owner_mutations(&request, &current, owner_mutations)?;
        owner_mutations = normalized;
        let source_completion =
            ensure_succeeded_source_completion(&current, classification, source_completion)?;
        sort_owner_mutations(self.repository.as_ref(), &mut owner_mutations)?;
        validate_owner_mutations(
            self.repository.as_ref(),
            &request,
            &current,
            &owner_mutations,
            source_completion.as_ref(),
            owner_batch.as_ref(),
            classification,
        )?;
        validate_owner_batch(
            self.repository.as_ref(),
            &request,
            &current,
            &owner_mutations,
            source_completion.as_ref(),
            owner_batch.as_ref(),
        )?;
        if resource_view.len() > 64 * 1024 {
            resource_view = "The recovery result exceeded the safe public view bound.".to_string();
        }
        let outcome = result_outcome(classification);
        let mut resource_revision = current.revision.value() as u64;
        let keep_pending = outcome != RecoveryActionResultOutcome::Terminal;
        let mut obligation_result = None;
        // Local recovery results advance the same obligation in this finish
        // batch.  An effect-specific executor (permission response) may have
        // already advanced the owner and obligation atomically; in that case
        // the immutable action result records the fresh revision without
        // rewriting that completed owner state.
        if current.revision == reserved_revision
            && action_claim_matches(&current.record, &request.action_id)
        {
            let next_revision = current
                .revision
                .next()
                .ok_or(RecoveryActionError::InvalidRequest)?;
            resource_revision = next_revision.value() as u64;
            let record = obligation_with_action_result(
                &current.record,
                &request.action_id,
                classification,
                source_completion.as_ref().map(|source| &source.record),
            )?;
            obligation_result = Some(LocalStateMutation::Obligation(ObligationMutation {
                obligation_id: request.obligation_id.clone(),
                record,
                pending: keep_pending.then(|| pending_entry(&pending)),
                expected: RevisionGuard::Expected(current.revision),
                revision: next_revision,
            }));
        }
        if (!owner_mutations.is_empty() || owner_batch.is_some()) && obligation_result.is_none() {
            // An owner projection is publishable only beside the exact claimed
            // obligation revision. Keeping the action pending lets restart retry
            // the typed readback instead of exposing a partial result.
            return Ok(RecoveryActionOutcome::InProgress {
                action_id: request.action_id,
            });
        }
        let (_, completed) = encode_recovery_completed_result(
            outcome,
            classification,
            resource_revision,
            resource_view,
        )?;
        let mut mutations = vec![LocalStateMutation::RecoveryAction(RecoveryActionMutation {
            action_id: request.action_id.clone(),
            binding_hash,
            attempt: completed_attempt(&request),
            completed: Some(completed),
            expected: RevisionGuard::Expected(Revision::new(0).expect("zero revision")),
            revision: Revision::new(1).expect("revision one"),
        })];
        let finish_payload = mutations
            .first()
            .and_then(|mutation| match mutation {
                LocalStateMutation::RecoveryAction(action) => action.completed.as_ref(),
                _ => None,
            })
            .expect("recovery result participant");
        let finish_payload_hash = finish_payload_hash(
            self.repository.as_ref(),
            &*self.authority,
            finish_payload,
            &owner_mutations,
            obligation_result
                .as_ref()
                .and_then(|mutation| match mutation {
                    LocalStateMutation::Obligation(obligation) => Some(obligation),
                    _ => None,
                }),
            owner_batch.as_ref(),
        )?;
        mutations.extend(obligation_result);
        mutations.append(&mut owner_mutations);
        let finish_operation_kind = recovery_finish_operation_kind(&mutations);
        let (expected_heads, events) = owner_batch
            .take()
            .map(|batch| (batch.expected_heads, batch.events))
            .unwrap_or_default();
        let finish = LocalAtomicBatch {
            commit_id: commit_identity(&self.authority, "finish", &request.action_id)?,
            idempotency: IdempotencyBinding {
                generation_id: self.generation_id.clone(),
                operation_kind: finish_operation_kind,
                idempotency_key: format!("{}.finish", request.action_id),
                payload_hash: finish_payload_hash,
            },
            expected_heads,
            events,
            state_mutations: mutations,
        };
        match self.repository.commit_batch(finish).await {
            Ok(CommitBatchResult::Committed(_) | CommitBatchResult::Replayed(_)) => {
                self.executor.after_commit(&effect, classification).await;
                let saved = self.get_action_status(&request.action_id).await?;
                let RecoveryActionStatus::Completed { result, .. } = saved else {
                    return Err(internal("completed-action-readback"));
                };
                Ok(RecoveryActionOutcome::Completed {
                    action_id: request.action_id,
                    result,
                })
            }
            Err(CommitBatchError::OutcomeUnknown { .. }) => {
                Ok(RecoveryActionOutcome::ActionOutcomeUnknown {
                    action_id: request.action_id,
                })
            }
            Err(CommitBatchError::StreamHeadConflict { .. }) => {
                if let Some(saved) = self.get_action(&request.action_id).await? {
                    saved_status_outcome(&saved)
                } else {
                    Ok(RecoveryActionOutcome::InProgress {
                        action_id: request.action_id,
                    })
                }
            }
            Err(error) => map_commit_error(error, &request.action_id),
        }
    }
}

/// Typed readback ports may publish only bounded owner read-model participants.
/// Recovery-owned action/obligation rows are constructed by this usecase and
/// cannot be supplied by an adapter, which keeps the finish transaction's CAS
/// fence and immutable result binding under one owner.
fn normalize_owner_mutations(
    request: &RecoveryActionRequest,
    current: &ObligationView,
    mutations: Vec<LocalStateMutation>,
) -> Result<(Vec<LocalStateMutation>, Option<ObligationMutation>), RecoveryActionError> {
    let expected_revision = current
        .revision
        .next()
        .ok_or(RecoveryActionError::InvalidRequest)?;
    let mut owner_mutations = Vec::with_capacity(mutations.len());
    let mut source_completion = None;
    for mutation in mutations {
        let LocalStateMutation::Obligation(source) = &mutation else {
            owner_mutations.push(mutation);
            continue;
        };
        if source.obligation_id != request.obligation_id {
            owner_mutations.push(mutation);
            continue;
        }
        if source.pending.is_some()
            || source.expected != RevisionGuard::Expected(current.revision)
            || source.revision != expected_revision
            || !source_completion_matches(&current.record, &source.record)
        {
            return Err(internal("readback-source-obligation-closure"));
        }
        if source_completion.replace(source.clone()).is_some() {
            return Err(internal("readback-source-obligation-duplicate"));
        }
        // The effect owner may prepare the exact source closure alongside its
        // operation participant. RecoveryActionUsecase owns the single stored
        // source row, however, because it must preserve the durable action
        // claim/result wrapper. Drop only this byte-for-byte validated closure;
        // distinct obligations (for example recovery publication) remain
        // owner participants.
    }
    Ok((owner_mutations, source_completion))
}

fn ensure_succeeded_source_completion(
    current: &ObligationView,
    classification: RecoveryResultClassification,
    source_completion: Option<ObligationMutation>,
) -> Result<Option<ObligationMutation>, RecoveryActionError> {
    if classification != RecoveryResultClassification::Succeeded {
        return if source_completion.is_none() {
            Ok(None)
        } else {
            Err(internal("readback-nonterminal-source-closure"))
        };
    }
    if source_completion.is_some() {
        return Ok(source_completion);
    }

    let mut completed = original_obligation(&current.record).clone();
    let state = match &mut completed {
        ObligationRecord::Send { state, .. }
        | ObligationRecord::PermissionResponse { state, .. }
        | ObligationRecord::StopInterrupt { state, .. }
        | ObligationRecord::SessionClose { state, .. }
        | ObligationRecord::LegacyReconciliation { state, .. }
        | ObligationRecord::RecoveryReserved { state, .. } => state,
        ObligationRecord::BackendSessionRecovery { .. } => {
            // Backend recovery completion carries the exact old/new
            // generations, provider identity and completion timestamp. Those
            // values cannot be reconstructed from a generic success label.
            return Err(internal("readback-backend-source-closure-missing"));
        }
        _ => return Err(internal("readback-source-closure-family")),
    };
    *state = ObligationStateRecord::Completed;
    let revision = current
        .revision
        .next()
        .ok_or(RecoveryActionError::InvalidRequest)?;
    Ok(Some(ObligationMutation {
        obligation_id: current.obligation_id.clone(),
        record: completed,
        pending: None,
        expected: RevisionGuard::Expected(current.revision),
        revision,
    }))
}

fn sorted_owner_identities(
    repository: &dyn LocalEventTransactionRepository,
    mutations: &[LocalStateMutation],
) -> Result<Vec<Vec<u8>>, RecoveryActionError> {
    let mut identities = mutations
        .iter()
        .map(|mutation| recovery_owner_identity_v1(repository, mutation))
        .collect::<Result<Vec<_>, _>>()?;
    identities.sort();
    Ok(identities)
}

fn sort_owner_mutations(
    repository: &dyn LocalEventTransactionRepository,
    mutations: &mut Vec<LocalStateMutation>,
) -> Result<(), RecoveryActionError> {
    let mut keyed = mutations
        .drain(..)
        .map(|mutation| {
            let identity = recovery_owner_identity_v1(repository, &mutation)?;
            Ok((identity, mutation))
        })
        .collect::<Result<Vec<_>, RecoveryActionError>>()?;
    keyed.sort_by(|left, right| left.0.cmp(&right.0));
    mutations.extend(keyed.into_iter().map(|(_, mutation)| mutation));
    Ok(())
}

fn source_completion_matches(current: &ObligationRecord, proposed: &ObligationRecord) -> bool {
    if let (
        ObligationRecord::BackendSessionRecovery {
            session_id,
            recovery_id,
            detail:
                Some(
                    crate::domain::local_event::BackendSessionRecoveryObligationRecord::EffectReserved {
                        old_provider_session_generation,
                        reserved_at_bits,
                        ..
                    },
                ),
            state: ObligationStateRecord::EffectReserved,
        },
        ObligationRecord::BackendSessionRecovery {
            session_id: proposed_session_id,
            recovery_id: proposed_recovery_id,
            detail:
                Some(
                    crate::domain::local_event::BackendSessionRecoveryObligationRecord::Completed {
                        old_provider_session_generation: proposed_old_generation,
                        provider_session_generation,
                        backend_session_id,
                        completed_at_bits,
                    },
                ),
            state: ObligationStateRecord::Completed,
        },
    ) = (original_obligation(current), proposed)
    {
        let Some(expected_generation) = old_provider_session_generation.checked_add(1) else {
            return false;
        };
        let reserved_at = f64::from_bits(*reserved_at_bits);
        let completed_at = f64::from_bits(*completed_at_bits);
        return session_id == proposed_session_id
            && recovery_id == proposed_recovery_id
            && old_provider_session_generation == proposed_old_generation
            && *provider_session_generation == expected_generation
            && !backend_session_id.is_empty()
            && reserved_at.is_finite()
            && completed_at.is_finite()
            && completed_at >= reserved_at;
    }
    let mut expected = original_obligation(current).clone();
    let state = match &mut expected {
        ObligationRecord::Send { state, .. }
        | ObligationRecord::PermissionResponse { state, .. }
        | ObligationRecord::StopInterrupt { state, .. }
        | ObligationRecord::SessionClose { state, .. }
        | ObligationRecord::BackendSessionRecovery { state, .. }
        | ObligationRecord::WorkflowShutdown { state, .. }
        | ObligationRecord::WorkflowTurnCompletion { state, .. }
        | ObligationRecord::RecoveryPublication { state, .. }
        | ObligationRecord::LegacyReconciliation { state, .. }
        | ObligationRecord::ProviderEstablish { state, .. }
        | ObligationRecord::TurnExecution { state, .. }
        | ObligationRecord::TerminalCommit { state, .. }
        | ObligationRecord::RecoveryReserved { state, .. }
        | ObligationRecord::RecoveryCompleted { state, .. } => state,
        ObligationRecord::FeedbackReservation { .. }
        | ObligationRecord::Feedback { .. }
        | ObligationRecord::WorkflowExecution { .. }
        | ObligationRecord::RecoveryTransition { .. }
        | ObligationRecord::Observed { .. } => return false,
    };
    *state = ObligationStateRecord::Completed;
    &expected == proposed
}

fn advances_one_revision(expected: RevisionGuard, revision: Revision) -> bool {
    match expected {
        RevisionGuard::Expected(current) => current.next() == Some(revision),
        RevisionGuard::Absent => false,
    }
}

fn validate_owner_mutations(
    repository: &dyn LocalEventTransactionRepository,
    request: &RecoveryActionRequest,
    current: &ObligationView,
    mutations: &[LocalStateMutation],
    source_completion: Option<&ObligationMutation>,
    owner_batch: Option<&RecoveryOwnerBatch>,
    classification: RecoveryResultClassification,
) -> Result<(), RecoveryActionError> {
    if mutations.len() > 16 {
        return Err(internal("readback-owner-participant-count"));
    }
    if classification != RecoveryResultClassification::Succeeded {
        return if mutations.is_empty() && source_completion.is_none() && owner_batch.is_none() {
            Ok(())
        } else {
            Err(internal("readback-nonterminal-owner-participants"))
        };
    }
    if source_completion.is_none() {
        return Err(internal("readback-source-obligation-closure-missing"));
    }

    let owner = current
        .pending
        .as_ref()
        .map(|pending| pending.owner.as_str())
        .ok_or_else(|| internal("readback-owner-missing"))?;
    let original = original_obligation(&current.record);
    let mut keys = std::collections::HashSet::new();
    let mut session_projection_count = 0usize;
    let mut stop_operation_count = 0usize;
    let mut stop_resolution_count = 0usize;
    let mut send_operation_count = 0usize;
    let mut lifecycle_operation_count = 0usize;
    let mut publication_count = 0usize;
    let mut stop_status_resolution = None;
    let mut stored_stop_resolution = None;
    for mutation in mutations {
        let key = match mutation {
            LocalStateMutation::SessionProjection(value)
                if value.session_id == owner
                    && supported_typed_readback_original(original)
                    && advances_one_revision(value.expected, value.revision) =>
            {
                session_projection_count += 1;
                format!("session:{}", value.session_id)
            }
            LocalStateMutation::StopResolution(value)
                if matches!(
                    original,
                    ObligationRecord::StopInterrupt { operation_id, .. }
                        if operation_id == &value.stop_operation_id
                ) =>
            {
                stop_resolution_count += 1;
                stored_stop_resolution = Some(value.resolution);
                format!("stop-resolution:{}", value.stop_operation_id)
            }
            LocalStateMutation::OperationRecord(value)
                if valid_recovery_operation(value, owner, original) =>
            {
                match &value.latest_status.value {
                    OperationStatusValue::StopCompleted { resolution } => {
                        stop_operation_count += 1;
                        stop_status_resolution = Some(*resolution);
                    }
                    OperationStatusValue::Terminal { .. } => send_operation_count += 1,
                    _ => return Err(internal("readback-owner-operation-status")),
                }
                format!("operation:{}:{}", value.kind.label(), value.operation_id)
            }
            LocalStateMutation::SessionLifecycleOperation(value)
                if valid_recovery_lifecycle_operation(value, owner, original) =>
            {
                lifecycle_operation_count += 1;
                format!("operation:{}:{}", value.kind.label(), value.operation_id)
            }
            LocalStateMutation::Obligation(value)
                if valid_recovery_publication(value, request, owner, original) =>
            {
                publication_count += 1;
                format!("obligation:{}", value.obligation_id)
            }
            _ => return Err(internal("readback-owner-participant-family")),
        };
        if !keys.insert(key) {
            return Err(internal("readback-owner-participant-duplicate"));
        }
        recovery_owner_identity_v1(repository, mutation)?;
    }

    let exact = match original {
        ObligationRecord::Send { .. } => {
            owner_batch.is_none()
                && session_projection_count == 0
                && stop_operation_count == 0
                && stop_resolution_count == 0
                && lifecycle_operation_count == 0
                && publication_count == 0
                && send_operation_count <= 1
                && mutations.len() == send_operation_count
        }
        ObligationRecord::StopInterrupt { .. } => {
            let resolution_agrees = matches!(
                (stop_status_resolution, stored_stop_resolution),
                (
                    Some(crate::domain::agent_session::events::StopResolution::Succeeded),
                    Some(crate::domain::local_event::StopResolutionKind::Succeeded),
                ) | (
                    Some(crate::domain::agent_session::events::StopResolution::Superseded),
                    Some(crate::domain::local_event::StopResolutionKind::Superseded),
                )
            );
            if owner_batch.is_some() {
                stop_operation_count == 1
                    && stop_resolution_count == 1
                    && send_operation_count == 0
                    && lifecycle_operation_count == 0
                    && publication_count == 0
                    && session_projection_count <= 1
                    && mutations.len()
                        == stop_operation_count + stop_resolution_count + session_projection_count
                    && resolution_agrees
            } else {
                mutations.is_empty()
            }
        }
        ObligationRecord::SessionClose { .. } => {
            owner_batch.is_none()
                && session_projection_count == 0
                && stop_operation_count == 0
                && stop_resolution_count == 0
                && send_operation_count == 0
                && publication_count == 0
                && lifecycle_operation_count <= 1
                && mutations.len() == lifecycle_operation_count
        }
        ObligationRecord::PermissionResponse { .. }
        | ObligationRecord::LegacyReconciliation { .. }
        | ObligationRecord::RecoveryReserved { .. } => {
            owner_batch.is_none() && mutations.is_empty()
        }
        ObligationRecord::BackendSessionRecovery { .. } => {
            owner_batch.is_some()
                && session_projection_count == 1
                && publication_count == 1
                && stop_operation_count == 0
                && stop_resolution_count == 0
                && send_operation_count == 0
                && lifecycle_operation_count == 0
                && mutations.len() == 2
        }
        _ => false,
    };
    if exact {
        Ok(())
    } else {
        Err(internal("readback-owner-participant-closure"))
    }
}

fn supported_typed_readback_original(record: &ObligationRecord) -> bool {
    matches!(
        record,
        ObligationRecord::Send { .. }
            | ObligationRecord::StopInterrupt { .. }
            | ObligationRecord::SessionClose { .. }
            | ObligationRecord::BackendSessionRecovery { .. }
    )
}

fn requires_durable_read_again_owner_evidence(record: &ObligationRecord) -> bool {
    matches!(
        original_obligation(record),
        ObligationRecord::StopInterrupt { .. }
            | ObligationRecord::Send {
                kind: SendObligationKindRecord::TurnExecution,
                ..
            }
            | ObligationRecord::SessionClose {
                action: SessionLifecycleRecordAction::Close,
                ..
            }
            | ObligationRecord::BackendSessionRecovery {
                detail: Some(
                    crate::domain::local_event::BackendSessionRecoveryObligationRecord::EffectReserved {
                        ..
                    }
                ),
                state: ObligationStateRecord::EffectReserved,
                ..
            }
    )
}

fn validate_owner_batch(
    repository: &dyn LocalEventTransactionRepository,
    _request: &RecoveryActionRequest,
    current: &ObligationView,
    owner_mutations: &[LocalStateMutation],
    source_completion: Option<&ObligationMutation>,
    batch: Option<&RecoveryOwnerBatch>,
) -> Result<(), RecoveryActionError> {
    let Some(batch) = batch else {
        return Ok(());
    };
    if batch.expected_heads.len() != 1
        || batch.events.is_empty()
        || batch.events.len() > 8
        || batch.canonical_events.is_empty()
        || batch.canonical_events.len() > 64 * 1024
    {
        return Err(internal("readback-owner-batch-bound"));
    }
    let canonical_events = repository
        .canonical_event_batch_identity_v1(&batch.events)
        .map_err(|_| internal("readback-owner-batch-canonical-events"))?;
    if canonical_events != batch.canonical_events {
        return Err(internal("readback-owner-batch-canonical-events"));
    }
    let owner = current
        .pending
        .as_ref()
        .map(|pending| pending.owner.as_str())
        .ok_or_else(|| internal("readback-owner-batch-owner"))?;
    let expected_stream = crate::domain::local_event::StreamId::agent_session(owner)
        .map_err(|_| internal("readback-owner-batch-stream"))?;
    let head = &batch.expected_heads[0];
    if head.stream_id != expected_stream
        || batch
            .events
            .iter()
            .any(|event| event.stream_id != expected_stream)
    {
        return Err(internal("readback-owner-batch-stream"));
    }
    if let ObligationRecord::StopInterrupt {
        operation_id,
        session_id,
        turn_id,
        state: ObligationStateRecord::EffectReserved | ObligationStateRecord::ReconciliationRequired,
        ..
    } = original_obligation(&current.record)
    {
        if session_id != owner || batch.events.len() != 1 {
            return Err(internal("readback-stop-batch-binding"));
        }
        let parsed_turn_id = turn_id
            .parse::<u64>()
            .map_err(|_| internal("readback-stop-batch-turn"))?;
        let (event_resolution, event_at) = match &batch.events[0].event {
            crate::domain::local_event::LocalDomainEvent::AgentSession(
                crate::domain::agent_session::events::AgentSessionDomainEvent::StopResolutionRecorded {
                    operation_id: event_operation_id,
                    turn_id: event_turn_id,
                    resolution,
                    at,
                },
            ) if event_operation_id == operation_id && *event_turn_id == parsed_turn_id => {
                (*resolution, *at)
            }
            _ => return Err(internal("readback-stop-batch-event")),
        };
        let stored_resolution = owner_mutations.iter().find_map(|mutation| match mutation {
            LocalStateMutation::StopResolution(resolution)
                if resolution.stop_operation_id == *operation_id =>
            {
                Some(resolution.resolution)
            }
            _ => None,
        });
        if !matches!(
            (event_resolution, stored_resolution),
            (
                crate::domain::agent_session::events::StopResolution::Succeeded,
                Some(crate::domain::local_event::StopResolutionKind::Succeeded),
            ) | (
                crate::domain::agent_session::events::StopResolution::Superseded,
                Some(crate::domain::local_event::StopResolutionKind::Superseded),
            )
        ) {
            return Err(internal("readback-stop-batch-resolution"));
        }
        if !event_at.is_finite()
            || batch.events[0].occurred_at_ms != (event_at * 1000.0).round() as i64
        {
            return Err(internal("readback-stop-batch-time"));
        }
        fn hash_field(hasher: &mut sha2::Sha256, value: &[u8]) {
            hasher.update((value.len() as u64).to_be_bytes());
            hasher.update(value);
        }
        let mut participant = sha2::Sha256::new();
        hash_field(&mut participant, b"stop_recovery_readback_participants_v1");
        hash_field(&mut participant, expected_stream.as_str().as_bytes());
        participant.update(head.expected.value().to_be_bytes());
        hash_field(&mut participant, &canonical_events);
        for identity in sorted_owner_identities(repository, owner_mutations)? {
            hash_field(&mut participant, &identity);
        }
        let digest: [u8; 32] = participant.finalize().into();
        if digest != batch.participant_digest {
            return Err(internal("readback-stop-batch-digest"));
        }
        return Ok(());
    }
    let (
        source_session_id,
        source_recovery_id,
        old_provider_session_generation,
    ) = match original_obligation(&current.record) {
        ObligationRecord::BackendSessionRecovery {
            session_id,
            recovery_id,
            detail:
                Some(
                    crate::domain::local_event::BackendSessionRecoveryObligationRecord::EffectReserved {
                        old_provider_session_generation,
                        ..
                    },
                ),
            state: ObligationStateRecord::EffectReserved,
        } => (
            session_id.as_str(),
            recovery_id.as_str(),
            *old_provider_session_generation,
        ),
        _ => return Err(internal("readback-owner-batch-family")),
    };
    let provider_session_generation = old_provider_session_generation
        .checked_add(1)
        .ok_or(RecoveryActionError::InvalidRequest)?;
    if source_session_id != owner || batch.events.len() != 3 {
        return Err(internal("readback-owner-batch-binding"));
    }
    let event = |index: usize| match &batch.events[index].event {
        crate::domain::local_event::LocalDomainEvent::AgentSession(event) => Ok(event),
        _ => Err(internal("readback-owner-batch-event-family")),
    };
    let first_at = match event(0)? {
        crate::domain::agent_session::events::AgentSessionDomainEvent::SessionConfigurationReactivated {
            recovery_id,
            provider_session_generation: generation,
            consumed_observation_id: None,
            at,
        } if recovery_id == source_recovery_id && *generation == provider_session_generation => *at,
        _ => return Err(internal("readback-owner-batch-configuration")),
    };
    match event(1)? {
        crate::domain::agent_session::events::AgentSessionDomainEvent::SessionGoalReactivated {
            recovery_id,
            outcome: crate::domain::agent_session::events::GoalReactivationOutcome::NoCurrentGoal,
            provider_session_generation: generation,
            restoring_turn_id: None,
            consumed_observation_id: None,
            at,
        } if recovery_id == source_recovery_id
            && *generation == provider_session_generation
            && at.to_bits() == first_at.to_bits() => {}
        _ => return Err(internal("readback-owner-batch-goal")),
    }
    match event(2)? {
        crate::domain::agent_session::events::AgentSessionDomainEvent::BackendSessionRecoveryCompleted {
            recovery_id,
            provider_session_generation: generation,
            at,
        } if recovery_id == source_recovery_id
            && *generation == provider_session_generation
            && at.to_bits() == first_at.to_bits() => {}
        _ => return Err(internal("readback-owner-batch-completed")),
    }
    let occurred_at_ms = (first_at * 1000.0).round() as i64;
    if !first_at.is_finite()
        || batch
            .events
            .iter()
            .any(|event| event.occurred_at_ms != occurred_at_ms)
    {
        return Err(internal("readback-owner-batch-time"));
    }
    let source_completion =
        source_completion.ok_or_else(|| internal("readback-backend-source-closure-missing"))?;
    let (
        completed_old_generation,
        completed_generation,
        completed_backend_session_id,
        completed_at_bits,
    ) = match &source_completion.record {
        ObligationRecord::BackendSessionRecovery {
            session_id,
            recovery_id,
            detail:
                Some(crate::domain::local_event::BackendSessionRecoveryObligationRecord::Completed {
                    old_provider_session_generation,
                    provider_session_generation,
                    backend_session_id,
                    completed_at_bits,
                }),
            state: ObligationStateRecord::Completed,
        } if session_id == source_session_id && recovery_id == source_recovery_id => (
            *old_provider_session_generation,
            *provider_session_generation,
            backend_session_id.as_str(),
            *completed_at_bits,
        ),
        _ => return Err(internal("readback-backend-source-closure")),
    };
    if completed_old_generation != old_provider_session_generation
        || completed_generation != provider_session_generation
        || completed_at_bits != first_at.to_bits()
    {
        return Err(internal("readback-backend-source-closure"));
    }
    let projection = owner_mutations.iter().find_map(|mutation| match mutation {
        LocalStateMutation::SessionProjection(projection) => match &projection.projection {
            crate::domain::local_event::SessionProjectionRecord::AgentSession(projection) => {
                Some(projection)
            }
            _ => None,
        },
        _ => None,
    });
    let Some(projection) = projection else {
        return Err(internal("readback-backend-projection"));
    };
    let publication = owner_mutations.iter().find_map(|mutation| match mutation {
        LocalStateMutation::Obligation(publication) => Some(publication),
        _ => None,
    });
    let Some(publication) = publication else {
        return Err(internal("readback-backend-publication"));
    };
    let (publication_recovery_id, publication_message_id, publication_source_obligation_id) =
        match &publication.record {
            ObligationRecord::RecoveryPublication {
                recovery_id,
                message_id,
                source_obligation_id,
                ..
            } => (
                recovery_id.as_str(),
                message_id.as_str(),
                source_obligation_id.as_str(),
            ),
            _ => return Err(internal("readback-backend-publication")),
        };
    let pending_message_id = match &projection.meta.pending_recovery_message {
        Some(crate::domain::local_event::AgentPendingRecoveryMessageRecord::Notice {
            recovery_id,
            message_id,
        }) if recovery_id == source_recovery_id => message_id.as_str(),
        _ => return Err(internal("readback-backend-projection")),
    };
    if projection.meta.id != source_session_id
        || projection.meta.agent_session_id.as_deref() != Some(completed_backend_session_id)
        || projection.meta.provider_session_generation != provider_session_generation
        || projection.meta.context_reinjection_generation != Some(provider_session_generation)
        || projection.meta.recovery_publication_snapshot.is_some()
        || projection.meta.updated_at_bits != first_at.to_bits()
        || publication_recovery_id != source_recovery_id
        || publication_message_id != pending_message_id
        || publication_source_obligation_id != current.obligation_id
    {
        return Err(internal("readback-backend-participant-binding"));
    }
    fn hash_field(hasher: &mut sha2::Sha256, value: &[u8]) {
        hasher.update((value.len() as u64).to_be_bytes());
        hasher.update(value);
    }
    let mut participant = sha2::Sha256::new();
    hash_field(
        &mut participant,
        b"backend_recovery_readback_participants_v1",
    );
    hash_field(&mut participant, expected_stream.as_str().as_bytes());
    participant.update(head.expected.value().to_be_bytes());
    hash_field(&mut participant, &canonical_events);
    for identity in sorted_owner_identities(repository, owner_mutations)? {
        hash_field(&mut participant, &identity);
    }
    let digest: [u8; 32] = participant.finalize().into();
    if digest != batch.participant_digest {
        return Err(internal("readback-owner-batch-digest"));
    }
    Ok(())
}

fn valid_recovery_operation(
    mutation: &crate::domain::local_event::OperationRecordMutation,
    owner: &str,
    original: &ObligationRecord,
) -> bool {
    match (&mutation.receipt, &mutation.latest_status.value) {
        (
            OperationReceiptRecord::Stop {
                operation_id,
                session_id,
                turn_id,
                ..
            },
            OperationStatusValue::StopCompleted { .. },
        ) => {
            matches!(
                original,
                ObligationRecord::StopInterrupt {
                    operation_id: source_operation_id,
                    session_id: source_session_id,
                    turn_id: source_turn_id,
                    ..
                } if source_operation_id == operation_id
                    && source_session_id == session_id
                    && source_turn_id == turn_id
            ) && mutation.kind == crate::domain::local_event::OperationKind::Stop
                && mutation.latest_status.kind == mutation.kind
                && !mutation.latest_status.migration_quit
                && mutation.operation_id == *operation_id
                && advances_one_revision(mutation.expected, mutation.revision)
                && session_id == owner
        }
        (
            OperationReceiptRecord::Send {
                operation_id,
                session_id,
                disposition:
                    crate::domain::agent_session::events::SendDisposition::StartedTurn { turn_id },
                ..
            },
            OperationStatusValue::Terminal { .. },
        ) => {
            matches!(
                original,
                ObligationRecord::Send {
                    operation_id: source_operation_id,
                    session_id: source_session_id,
                    kind: SendObligationKindRecord::TurnExecution,
                    turn_id: source_turn_id,
                    reserved_turn_id,
                    ..
                } if source_operation_id == operation_id
                    && source_session_id == session_id
                    && source_turn_id
                        .as_ref()
                        .or(reserved_turn_id.as_ref())
                        == Some(turn_id)
            ) && mutation.kind == crate::domain::local_event::OperationKind::Send
                && mutation.latest_status.kind == mutation.kind
                && !mutation.latest_status.migration_quit
                && mutation.operation_id == *operation_id
                && advances_one_revision(mutation.expected, mutation.revision)
                && session_id == owner
        }
        _ => false,
    }
}

fn valid_recovery_lifecycle_operation(
    mutation: &crate::domain::local_event::OperationRecordMutation,
    owner: &str,
    original: &ObligationRecord,
) -> bool {
    matches!(
        (&mutation.receipt, &mutation.latest_status.value),
        (
            OperationReceiptRecord::SessionLifecycle {
                operation_id,
                session_id,
                action: SessionLifecycleRecordAction::Close,
                ..
            },
            OperationStatusValue::Completed,
        ) if matches!(
                original,
                ObligationRecord::SessionClose {
                    operation_id: source_operation_id,
                    session_id: source_session_id,
                    action: SessionLifecycleRecordAction::Close,
                    ..
                } if source_operation_id == operation_id
                    && source_session_id == session_id
            )
            && mutation.kind == crate::domain::local_event::OperationKind::SessionLifecycle
            && mutation.latest_status.kind == mutation.kind
            && !mutation.latest_status.migration_quit
            && mutation.operation_id == *operation_id
            && advances_one_revision(mutation.expected, mutation.revision)
            && session_id == owner
    )
}

fn valid_recovery_publication(
    mutation: &ObligationMutation,
    request: &RecoveryActionRequest,
    owner: &str,
    original: &ObligationRecord,
) -> bool {
    let ObligationRecord::BackendSessionRecovery {
        session_id: source_session_id,
        recovery_id: source_recovery_id,
        detail:
            Some(crate::domain::local_event::BackendSessionRecoveryObligationRecord::EffectReserved {
                ..
            }),
        ..
    } = original
    else {
        return false;
    };
    let ObligationRecord::RecoveryPublication {
        session_id,
        recovery_id,
        message_id,
        source_obligation_id,
        detail: crate::domain::local_event::RecoveryPublicationObligationRecord::Pending { .. },
        state: ObligationStateRecord::Pending,
    } = &mutation.record
    else {
        return false;
    };
    let expected_id = {
        let digest = sha2::Sha256::digest(
            format!("recovery-publication/v1\0{session_id}\0{recovery_id}\0{message_id}")
                .as_bytes(),
        );
        format!("recovery-publication-{}", hex::encode(digest))
    };
    mutation.obligation_id == expected_id
        && mutation.obligation_id != request.obligation_id
        && source_obligation_id == &request.obligation_id
        && session_id == source_session_id
        && recovery_id == source_recovery_id
        && session_id == owner
        && mutation.expected == RevisionGuard::Absent
        && mutation.revision == Revision::new(0).expect("zero revision")
        && mutation.pending.as_ref().is_some_and(|pending| {
            pending.owner == owner
                && pending.partition == PendingPartition::Owner
                && pending.shutdown_plan.is_none()
        })
}

pub(super) fn recovery_owner_identity_v1(
    repository: &dyn LocalEventTransactionRepository,
    mutation: &LocalStateMutation,
) -> Result<Vec<u8>, RecoveryActionError> {
    if let Ok(identity) = repository.canonical_mutation_identity_v1(mutation) {
        return Ok(identity);
    }
    let operation = match mutation {
        LocalStateMutation::OperationRecord(operation)
        | LocalStateMutation::SessionLifecycleOperation(operation) => operation,
        _ => return Err(internal("readback-owner-participant-identity")),
    };
    fn field(bytes: &mut Vec<u8>, value: &[u8]) {
        bytes.extend_from_slice(&(value.len() as u64).to_be_bytes());
        bytes.extend_from_slice(value);
    }
    fn text(bytes: &mut Vec<u8>, value: &str) {
        field(bytes, value.as_bytes());
    }
    let mut bytes = b"recovery_owner_operation_identity_v1".to_vec();
    text(&mut bytes, operation.kind.label());
    text(&mut bytes, &operation.operation_id);
    match operation.expected {
        RevisionGuard::Absent => bytes.push(0),
        RevisionGuard::Expected(revision) => {
            bytes.push(1);
            bytes.extend_from_slice(&revision.value().to_be_bytes());
        }
    }
    bytes.extend_from_slice(&operation.revision.value().to_be_bytes());
    match &operation.receipt {
        OperationReceiptRecord::Stop {
            operation_id,
            session_id,
            turn_id,
            accepted_revision,
            authentication,
        } => {
            text(&mut bytes, "stop");
            text(&mut bytes, operation_id);
            text(&mut bytes, session_id);
            text(&mut bytes, turn_id);
            bytes.extend_from_slice(&accepted_revision.to_be_bytes());
            field(&mut bytes, &authentication.principal_mac);
            field(&mut bytes, &authentication.binding_hmac);
        }
        OperationReceiptRecord::Send {
            operation_id,
            session_id,
            input_ref,
            disposition,
            authentication,
        } => {
            text(&mut bytes, "send");
            text(&mut bytes, operation_id);
            text(&mut bytes, session_id);
            text(&mut bytes, input_ref);
            match disposition {
                SendDisposition::StartedTurn { turn_id } => {
                    text(&mut bytes, "started_turn");
                    text(&mut bytes, turn_id);
                }
                SendDisposition::Queued { queue_item_id } => {
                    text(&mut bytes, "queued");
                    text(&mut bytes, queue_item_id);
                }
            }
            field(&mut bytes, &authentication.principal_mac);
            field(&mut bytes, &authentication.binding_hmac);
        }
        OperationReceiptRecord::SessionLifecycle {
            operation_id,
            session_id,
            action,
            first_accepted_revision,
            commit_operation_kind,
            authentication,
        } => {
            text(&mut bytes, "session_lifecycle");
            text(&mut bytes, operation_id);
            text(&mut bytes, session_id);
            match action {
                SessionLifecycleRecordAction::Close => text(&mut bytes, "close"),
                SessionLifecycleRecordAction::ArchiveOpen => text(&mut bytes, "archive_open"),
                SessionLifecycleRecordAction::ArchiveClosed => text(&mut bytes, "archive_closed"),
                SessionLifecycleRecordAction::SwitchBackend { backend_id } => {
                    text(&mut bytes, "switch_backend");
                    text(&mut bytes, backend_id);
                }
            }
            bytes.extend_from_slice(&first_accepted_revision.to_be_bytes());
            text(&mut bytes, commit_operation_kind.label());
            field(&mut bytes, &authentication.principal_mac);
            field(&mut bytes, &authentication.binding_hmac);
        }
        _ => return Err(internal("readback-owner-operation-receipt")),
    }
    match &operation.latest_status.value {
        OperationStatusValue::StopCompleted { resolution } => {
            text(&mut bytes, "stop_completed");
            text(
                &mut bytes,
                match resolution {
                    crate::domain::agent_session::events::StopResolution::Succeeded => "succeeded",
                    crate::domain::agent_session::events::StopResolution::Superseded => {
                        "superseded"
                    }
                },
            );
        }
        OperationStatusValue::Completed => text(&mut bytes, "completed"),
        OperationStatusValue::Terminal { result } => {
            text(&mut bytes, "terminal");
            let terminal_identity = LocalStateMutation::TerminalRecord(TerminalRecordMutation {
                session_id: "recovery-owner-operation".to_string(),
                turn_id: operation.operation_id.clone(),
                terminal_identity: operation.operation_id.clone(),
                result: TerminalResultRecord::Stop {
                    operation_id: operation.operation_id.clone(),
                    reason: None,
                    exit_code: None,
                    result: result.clone(),
                },
                participant_digest: [0; 32],
            })
            .canonical_identity_v1()
            .map_err(|_| internal("readback-owner-operation-terminal"))?;
            field(&mut bytes, &terminal_identity);
        }
        _ => return Err(internal("readback-owner-operation-status")),
    }
    Ok(bytes)
}

fn recovery_source_result_identity_v1(
    repository: &dyn LocalEventTransactionRepository,
    source: &ObligationMutation,
) -> Result<Vec<u8>, RecoveryActionError> {
    let mutation = LocalStateMutation::Obligation(source.clone());
    if let Ok(identity) = repository.canonical_mutation_identity_v1(&mutation) {
        return Ok(identity);
    }

    // Legacy reconciliation payloads deliberately have no general-purpose
    // domain identity: they can contain historical event/request shapes that
    // are not valid inputs for new operations. A recovery result can still
    // update the already-stored row because the issued action identity and
    // exact expected revision bind that immutable legacy payload. Encode only
    // the closed recovery overlay and its CAS envelope here.
    let ObligationRecord::LegacyReconciliation {
        safe_actions,
        state,
        ..
    } = original_obligation(&source.record)
    else {
        return Err(internal("readback-source-completion-identity"));
    };
    let Some(action) = recovery_action(&source.record) else {
        return Err(internal("readback-source-completion-identity"));
    };
    let Some(classification) = action.classification else {
        return Err(internal("readback-source-completion-identity"));
    };
    if action.state != ObligationStateRecord::Completed {
        return Err(internal("readback-source-completion-identity"));
    }

    fn field(bytes: &mut Vec<u8>, value: &[u8]) {
        bytes.extend_from_slice(&(value.len() as u64).to_be_bytes());
        bytes.extend_from_slice(value);
    }
    fn text(bytes: &mut Vec<u8>, value: &str) {
        field(bytes, value.as_bytes());
    }
    fn state_byte(state: ObligationStateRecord) -> u8 {
        match state {
            ObligationStateRecord::Prepared => 0,
            ObligationStateRecord::Pending => 1,
            ObligationStateRecord::EffectReserved => 2,
            ObligationStateRecord::Running => 3,
            ObligationStateRecord::WaitingApproval => 4,
            ObligationStateRecord::OutcomeUnknown => 5,
            ObligationStateRecord::ReconciliationRequired => 6,
            ObligationStateRecord::Completed => 7,
            ObligationStateRecord::Cancelled => 8,
            ObligationStateRecord::Failed => 9,
        }
    }
    fn observation(record: &ObligationRecord) -> Option<&AuthoritativeEffectObservationRecord> {
        match record {
            ObligationRecord::Observed { observation, .. } => Some(observation),
            ObligationRecord::RecoveryTransition { original, .. } => observation(original),
            _ => None,
        }
    }

    let descriptor = pending_recovery_descriptor(&source.obligation_id, &source.record);
    let mut bytes = b"legacy_recovery_source_result_identity_v1".to_vec();
    text(&mut bytes, &source.obligation_id);
    text(&mut bytes, &descriptor.original_identity);
    match source.expected {
        RevisionGuard::Absent => bytes.push(0),
        RevisionGuard::Expected(revision) => {
            bytes.push(1);
            bytes.extend_from_slice(&revision.value().to_be_bytes());
        }
    }
    bytes.extend_from_slice(&source.revision.value().to_be_bytes());
    match &source.pending {
        Some(pending) => {
            bytes.push(1);
            text(&mut bytes, &pending.ordered_key);
            text(&mut bytes, &pending.owner);
            text(&mut bytes, pending.partition.label());
            match &pending.shutdown_plan {
                Some(plan) => {
                    bytes.push(1);
                    text(&mut bytes, &plan.plan_id);
                    bytes.extend_from_slice(&plan.epoch.to_be_bytes());
                }
                None => bytes.push(0),
            }
        }
        None => bytes.push(0),
    }
    bytes.push(state_byte(*state));
    bytes.extend_from_slice(&(safe_actions.len() as u64).to_be_bytes());
    for safe_action in safe_actions {
        text(&mut bytes, action_label(*safe_action));
    }
    text(&mut bytes, &action.action_id);
    bytes.extend_from_slice(&action.origin_revision.to_be_bytes());
    text(&mut bytes, action_label(action.action));
    text(&mut bytes, &action.effect_identity);
    text(&mut bytes, classification_label(classification));
    if let Some(observation) = observation(&source.record) {
        bytes.push(1);
        text(&mut bytes, &observation.effect_identity);
        bytes.extend_from_slice(&observation.origin_revision.to_be_bytes());
        text(&mut bytes, classification_label(observation.classification));
        bytes.push(u8::from(observation.cancellable));
        text(&mut bytes, &observation.safe_view);
        field(&mut bytes, &observation.result_sha256);
        field(&mut bytes, &observation.proof_mac);
    } else {
        bytes.push(0);
    }
    Ok(bytes)
}

fn finish_payload_hash(
    repository: &dyn LocalEventTransactionRepository,
    authority: &dyn OperationBindingAuthority,
    completed: &RecoveryResultRecord,
    owner_mutations: &[LocalStateMutation],
    source_completion: Option<&ObligationMutation>,
    owner_batch: Option<&RecoveryOwnerBatch>,
) -> Result<[u8; 32], RecoveryActionError> {
    let mut material = b"recovery_finish_participants_v1".to_vec();
    match completed {
        RecoveryResultRecord::Action(result) => {
            material.extend_from_slice(b"\0action\0");
            material.extend_from_slice(&result.canonical_result_sha256);
        }
        RecoveryResultRecord::FeedbackRetry {
            feedback_id,
            resource_revision,
            resolved,
        } => {
            material.extend_from_slice(b"\0feedback_retry\0");
            material.extend_from_slice(&(feedback_id.len() as u64).to_be_bytes());
            material.extend_from_slice(feedback_id.as_bytes());
            material.extend_from_slice(&resource_revision.to_be_bytes());
            material.push(u8::from(*resolved));
        }
    }
    for identity in sorted_owner_identities(repository, owner_mutations)? {
        material.extend_from_slice(&(identity.len() as u64).to_be_bytes());
        material.extend_from_slice(&identity);
    }
    if let Some(source) = source_completion {
        let identity = recovery_source_result_identity_v1(repository, source)?;
        material.extend_from_slice(&(identity.len() as u64).to_be_bytes());
        material.extend_from_slice(&identity);
    }
    if let Some(batch) = owner_batch {
        material.extend_from_slice(b"\0owner_batch\0");
        material.extend_from_slice(&batch.participant_digest);
    }
    Ok(authority.digest(&material))
}

fn recovery_finish_operation_kind(mutations: &[LocalStateMutation]) -> CommitOperationKind {
    if mutations.iter().any(|mutation| {
        matches!(
            mutation,
            LocalStateMutation::Obligation(ObligationMutation {
                record: ObligationRecord::RecoveryPublication { .. },
                expected: RevisionGuard::Absent,
                ..
            })
        )
    }) {
        // This finish creates a new owner-visible publication. It is new
        // recovery work and must remain blocked once application shutdown has
        // fixed its target set.
        CommitOperationKind::Recovery
    } else {
        // Finishing an already admitted operation/obligation may drain during
        // shutdown, subject to the store's closed OperationProgress shape.
        CommitOperationKind::OperationProgress
    }
}

fn is_internal_feedback_reservation(record: &ObligationRecord) -> bool {
    matches!(
        original_obligation(record),
        ObligationRecord::FeedbackReservation { .. }
    )
}

/// Shared backend-issued action identity derivation for pending obligations
/// and shutdown-target recovery. `resource_ref` must be the canonical durable
/// resource/effect reference, never a presentation label.
pub(crate) fn derive_recovery_action_id(
    authority: &dyn OperationBindingAuthority,
    generation_id: &str,
    resource_ref: &str,
    origin_revision: u64,
    action: RecoveryActionKind,
) -> String {
    let action_byte = match action {
        RecoveryActionKind::ReadAgain => 1,
        RecoveryActionKind::RetrySameEffect => 2,
        RecoveryActionKind::UseObservedResult => 3,
        RecoveryActionKind::CancelIfSafe => 4,
        RecoveryActionKind::KeepForManualResolution => 5,
    };
    let mut body = Vec::with_capacity(26);
    body.extend_from_slice(&[1, action_byte]);
    body.extend_from_slice(&origin_revision.to_be_bytes());
    body.extend_from_slice(&authority.digest(resource_ref.as_bytes())[..16]);
    let mac_material = [
        b"recovery-action-token/v1\0".as_slice(),
        generation_id.as_bytes(),
        b"\0".as_slice(),
        body.as_slice(),
    ]
    .concat();
    format!(
        "ra1.{}.{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&body),
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(authority.mac(&mac_material))
    )
}

fn verify_recovery_action_id(
    authority: &dyn OperationBindingAuthority,
    generation_id: &str,
    action_id: &str,
) -> bool {
    let mut segments = action_id.split('.');
    if segments.next() != Some("ra1") {
        return false;
    }
    let (Some(body), Some(mac), None) = (segments.next(), segments.next(), segments.next()) else {
        return false;
    };
    let Ok(body) = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(body) else {
        return false;
    };
    let Ok(mac) = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(mac) else {
        return false;
    };
    let mac: [u8; 32] = match mac.try_into() {
        Ok(mac) => mac,
        Err(_) => return false,
    };
    if body.len() != 26 || body[0] != 1 || !matches!(body[1], 1..=5) {
        return false;
    }
    let mac_material = [
        b"recovery-action-token/v1\0".as_slice(),
        generation_id.as_bytes(),
        b"\0".as_slice(),
        body.as_slice(),
    ]
    .concat();
    constant_time_eq_32(&mac, &authority.mac(&mac_material))
}

fn pending_entry(value: &crate::domain::local_event::PendingIndexEntryView) -> PendingIndexEntry {
    PendingIndexEntry {
        ordered_key: value.ordered_key.clone(),
        owner: value.owner.clone(),
        partition: value.partition,
        shutdown_plan: value.shutdown_plan.clone(),
    }
}

fn recovery_action(record: &ObligationRecord) -> Option<&ObligationRecoveryActionRecord> {
    match record {
        ObligationRecord::RecoveryTransition {
            recovery_action, ..
        } => Some(recovery_action),
        ObligationRecord::Observed { original, .. } => recovery_action(original),
        ObligationRecord::Send { .. }
        | ObligationRecord::PermissionResponse { .. }
        | ObligationRecord::StopInterrupt { .. }
        | ObligationRecord::SessionClose { .. }
        | ObligationRecord::BackendSessionRecovery { .. }
        | ObligationRecord::WorkflowShutdown { .. }
        | ObligationRecord::WorkflowTurnCompletion { .. }
        | ObligationRecord::RecoveryPublication { .. }
        | ObligationRecord::LegacyReconciliation { .. }
        | ObligationRecord::ProviderEstablish { .. }
        | ObligationRecord::TurnExecution { .. }
        | ObligationRecord::TerminalCommit { .. }
        | ObligationRecord::RecoveryReserved { .. }
        | ObligationRecord::RecoveryCompleted { .. }
        | ObligationRecord::FeedbackReservation { .. }
        | ObligationRecord::Feedback { .. }
        | ObligationRecord::WorkflowExecution { .. } => None,
    }
}

fn obligation_with_action_claim(
    record: &ObligationRecord,
    request: &RecoveryActionRequest,
) -> Result<ObligationRecord, RecoveryActionError> {
    if recovery_action(record)
        .is_some_and(|action| action.state != ObligationStateRecord::Completed)
    {
        return Err(internal("obligation-claim-existing"));
    }
    Ok(ObligationRecord::RecoveryTransition {
        original: Box::new(record.clone()),
        recovery_action: ObligationRecoveryActionRecord {
            action_id: request.action_id.clone(),
            origin_revision: request.origin_revision,
            action: request.action,
            effect_identity: request.obligation_id.clone(),
            state: ObligationStateRecord::EffectReserved,
            classification: None,
        },
    })
}

fn obligation_with_action_result(
    record: &ObligationRecord,
    action_id: &str,
    classification: RecoveryResultClassification,
    completed_original: Option<&ObligationRecord>,
) -> Result<ObligationRecord, RecoveryActionError> {
    fn replace_deepest_original(
        stored: &ObligationRecord,
        completed: &ObligationRecord,
    ) -> ObligationRecord {
        match stored {
            ObligationRecord::Observed {
                original,
                observation,
            } => ObligationRecord::Observed {
                original: Box::new(replace_deepest_original(original, completed)),
                observation: observation.clone(),
            },
            ObligationRecord::RecoveryTransition {
                original,
                recovery_action,
            } => ObligationRecord::RecoveryTransition {
                original: Box::new(replace_deepest_original(original, completed)),
                recovery_action: recovery_action.clone(),
            },
            _ => completed.clone(),
        }
    }
    match record {
        ObligationRecord::RecoveryTransition {
            original,
            recovery_action,
        } => {
            if recovery_action.action_id != action_id {
                return Err(internal("obligation-result-identity"));
            }
            let mut recovery_action = recovery_action.clone();
            recovery_action.state = ObligationStateRecord::Completed;
            recovery_action.classification = Some(classification);
            Ok(ObligationRecord::RecoveryTransition {
                original: completed_original
                    .map(|completed| Box::new(replace_deepest_original(original, completed)))
                    .unwrap_or_else(|| original.clone()),
                recovery_action,
            })
        }
        ObligationRecord::Observed {
            original,
            observation,
        } => Ok(ObligationRecord::Observed {
            original: Box::new(obligation_with_action_result(
                original,
                action_id,
                classification,
                completed_original,
            )?),
            observation: observation.clone(),
        }),
        ObligationRecord::Send { .. }
        | ObligationRecord::PermissionResponse { .. }
        | ObligationRecord::StopInterrupt { .. }
        | ObligationRecord::SessionClose { .. }
        | ObligationRecord::BackendSessionRecovery { .. }
        | ObligationRecord::WorkflowShutdown { .. }
        | ObligationRecord::WorkflowTurnCompletion { .. }
        | ObligationRecord::RecoveryPublication { .. }
        | ObligationRecord::LegacyReconciliation { .. }
        | ObligationRecord::ProviderEstablish { .. }
        | ObligationRecord::TurnExecution { .. }
        | ObligationRecord::TerminalCommit { .. }
        | ObligationRecord::RecoveryReserved { .. }
        | ObligationRecord::RecoveryCompleted { .. }
        | ObligationRecord::FeedbackReservation { .. }
        | ObligationRecord::Feedback { .. }
        | ObligationRecord::WorkflowExecution { .. } => Err(internal("obligation-result-claim")),
    }
}

fn action_claim_matches(record: &ObligationRecord, action_id: &str) -> bool {
    recovery_action(record).is_some_and(|claim| claim.action_id == action_id)
}

fn action_claim_matches_request(
    record: &ObligationRecord,
    request: &RecoveryActionRequest,
) -> bool {
    recovery_action(record).is_some_and(|claim| {
        claim.action_id == request.action_id
            && claim.origin_revision == request.origin_revision
            && claim.action == request.action
            && claim.effect_identity == request.obligation_id
    })
}

fn completed_attempt(request: &RecoveryActionRequest) -> RecoveryAttemptRecord {
    RecoveryAttemptRecord::Obligation {
        obligation_id: request.obligation_id.clone(),
        origin_revision: request.origin_revision,
        action: request.action,
        effect_identity: request.obligation_id.clone(),
        state: ObligationStateRecord::Completed,
        failure: None,
    }
}

fn saved_status_outcome(
    saved: &RecoveryActionView,
) -> Result<RecoveryActionOutcome, RecoveryActionError> {
    match decode_saved_status(saved).ok_or_else(|| internal("saved-action-status"))? {
        RecoveryActionStatus::Completed { action_id, result } => {
            Ok(RecoveryActionOutcome::Completed { action_id, result })
        }
        RecoveryActionStatus::InProgress { action_id }
        | RecoveryActionStatus::ReconciliationRequired { action_id, .. } => {
            Ok(RecoveryActionOutcome::InProgress { action_id })
        }
        RecoveryActionStatus::OutcomeUnknown { action_id } => {
            Ok(RecoveryActionOutcome::ActionOutcomeUnknown { action_id })
        }
    }
}

fn commit_identity(
    authority: &Arc<dyn OperationBindingAuthority>,
    step: &str,
    action_id: &str,
) -> Result<CommitIdentity, RecoveryActionError> {
    CommitIdentity::parse(&hex::encode(
        authority.digest(format!("recovery-{step}/v1\0{action_id}").as_bytes()),
    ))
    .map_err(|_| internal("commit-identity"))
}

fn action_label(action: RecoveryActionKind) -> &'static str {
    match action {
        RecoveryActionKind::ReadAgain => "read_again",
        RecoveryActionKind::RetrySameEffect => "retry_same_effect",
        RecoveryActionKind::UseObservedResult => "use_observed_result",
        RecoveryActionKind::CancelIfSafe => "cancel_if_safe",
        RecoveryActionKind::KeepForManualResolution => "keep_for_manual_resolution",
    }
}

struct RecoveryCapabilities {
    state: RecoveryResourceState,
    safe_label: String,
    actions: Vec<RecoveryActionKind>,
    active_action: Option<RecoveryActionIdentity>,
}

struct PendingRecoveryDescriptor {
    category: PendingRecoveryCategory,
    original_identity: String,
    known_status: PendingRecoveryKnownStatus,
    safe_label: &'static str,
}

fn bounded_original_identity(value: Option<&str>) -> Option<String> {
    value
        .filter(|value| !value.is_empty() && value.len() <= 512)
        .map(str::to_string)
}

fn pending_recovery_known_status(
    state: Option<ObligationStateRecord>,
) -> PendingRecoveryKnownStatus {
    match state {
        Some(ObligationStateRecord::Prepared) => PendingRecoveryKnownStatus::Prepared,
        Some(ObligationStateRecord::Pending) => PendingRecoveryKnownStatus::Pending,
        Some(ObligationStateRecord::EffectReserved) => PendingRecoveryKnownStatus::EffectReserved,
        Some(ObligationStateRecord::Running) => PendingRecoveryKnownStatus::Running,
        Some(ObligationStateRecord::WaitingApproval) => PendingRecoveryKnownStatus::WaitingApproval,
        Some(ObligationStateRecord::ReconciliationRequired) => {
            PendingRecoveryKnownStatus::ReconciliationRequired
        }
        Some(ObligationStateRecord::Failed) => PendingRecoveryKnownStatus::Failed,
        Some(ObligationStateRecord::OutcomeUnknown)
        | Some(ObligationStateRecord::Completed)
        | Some(ObligationStateRecord::Cancelled)
        | None => PendingRecoveryKnownStatus::Unknown,
    }
}

fn obligation_state(record: &ObligationRecord) -> Option<ObligationStateRecord> {
    match original_obligation(record) {
        ObligationRecord::Send { state, .. }
        | ObligationRecord::PermissionResponse { state, .. }
        | ObligationRecord::StopInterrupt { state, .. }
        | ObligationRecord::SessionClose { state, .. }
        | ObligationRecord::BackendSessionRecovery { state, .. }
        | ObligationRecord::WorkflowShutdown { state, .. }
        | ObligationRecord::WorkflowTurnCompletion { state, .. }
        | ObligationRecord::RecoveryPublication { state, .. }
        | ObligationRecord::LegacyReconciliation { state, .. }
        | ObligationRecord::ProviderEstablish { state, .. }
        | ObligationRecord::TurnExecution { state, .. }
        | ObligationRecord::TerminalCommit { state, .. }
        | ObligationRecord::RecoveryReserved { state, .. }
        | ObligationRecord::RecoveryCompleted { state, .. } => Some(*state),
        ObligationRecord::FeedbackReservation { .. }
        | ObligationRecord::Feedback { .. }
        | ObligationRecord::WorkflowExecution { .. }
        | ObligationRecord::RecoveryTransition { .. }
        | ObligationRecord::Observed { .. } => None,
    }
}

fn descriptor(
    category: PendingRecoveryCategory,
    identity: Option<String>,
    known_status: PendingRecoveryKnownStatus,
    safe_label: &'static str,
    obligation_id: &str,
) -> PendingRecoveryDescriptor {
    match identity {
        Some(original_identity) => PendingRecoveryDescriptor {
            category,
            original_identity,
            known_status,
            safe_label,
        },
        None => PendingRecoveryDescriptor {
            category: PendingRecoveryCategory::Unknown,
            original_identity: obligation_id.to_string(),
            known_status: PendingRecoveryKnownStatus::Unknown,
            safe_label: "Pending local operation",
        },
    }
}

fn pending_recovery_descriptor(
    obligation_id: &str,
    record: &ObligationRecord,
) -> PendingRecoveryDescriptor {
    let known_status = pending_recovery_known_status(obligation_state(record));
    let identity = |value: &str| bounded_original_identity(Some(value));
    let (category, original_identity, safe_label) = match original_obligation(record) {
        ObligationRecord::Send {
            operation_id,
            kind: SendObligationKindRecord::ProviderEstablish,
            ..
        } => (
            PendingRecoveryCategory::ProviderEstablish,
            identity(operation_id),
            "Provider session establishment",
        ),
        ObligationRecord::Send {
            operation_id,
            kind: SendObligationKindRecord::TurnExecution,
            disposition,
            ..
        } => match disposition {
            SendObligationDispositionRecord::Queued => (
                PendingRecoveryCategory::QueueExecution,
                identity(operation_id),
                "Queued agent execution",
            ),
            SendObligationDispositionRecord::StartedTurn => (
                PendingRecoveryCategory::TurnExecution,
                identity(operation_id),
                "Agent turn execution",
            ),
        },
        ObligationRecord::PermissionResponse { operation_id, .. } => (
            PendingRecoveryCategory::PermissionDelivery,
            identity(operation_id),
            "Permission response delivery",
        ),
        ObligationRecord::StopInterrupt { operation_id, .. }
        | ObligationRecord::TerminalCommit { operation_id, .. } => (
            PendingRecoveryCategory::TerminalCommit,
            identity(operation_id),
            "Agent turn terminalization",
        ),
        ObligationRecord::SessionClose { operation_id, .. } => (
            PendingRecoveryCategory::SessionClose,
            identity(operation_id),
            "Session lifecycle action",
        ),
        ObligationRecord::BackendSessionRecovery { recovery_id, .. } => (
            PendingRecoveryCategory::BackendRecovery,
            identity(recovery_id),
            "Backend session recovery",
        ),
        ObligationRecord::WorkflowShutdown {
            effect_identity,
            execution_id,
            ..
        } => (
            PendingRecoveryCategory::WorkflowShutdown,
            identity(effect_identity).or_else(|| identity(execution_id)),
            "Workflow shutdown",
        ),
        ObligationRecord::WorkflowTurnCompletion {
            terminal_identity, ..
        } => (
            PendingRecoveryCategory::TurnExecution,
            identity(terminal_identity),
            "Workflow turn completion handoff",
        ),
        ObligationRecord::RecoveryPublication {
            message_id,
            recovery_id,
            ..
        } => (
            PendingRecoveryCategory::RecoveryPublication,
            identity(message_id).or_else(|| identity(recovery_id)),
            "Recovery message publication",
        ),
        ObligationRecord::LegacyReconciliation { detail, .. } => match detail {
            LegacyReconciliationRecord::TurnExecution { turn_id, .. } => (
                PendingRecoveryCategory::TurnExecution,
                identity(turn_id),
                "Agent turn execution",
            ),
            LegacyReconciliationRecord::QueuedSend { queue_item_id, .. } => (
                PendingRecoveryCategory::QueueExecution,
                identity(queue_item_id),
                "Queued agent execution",
            ),
            LegacyReconciliationRecord::Permission { request, .. } => (
                PendingRecoveryCategory::PermissionDelivery,
                identity(&request.id),
                "Permission response delivery",
            ),
            LegacyReconciliationRecord::ProviderSession { session_id } => (
                PendingRecoveryCategory::ProviderEstablish,
                identity(session_id),
                "Provider session establishment",
            ),
            LegacyReconciliationRecord::BackendRecovery { recovery_id, .. } => (
                PendingRecoveryCategory::BackendRecovery,
                identity(recovery_id),
                "Backend session recovery",
            ),
            LegacyReconciliationRecord::RecoveryPublication {
                session_id,
                pending_message,
            } => (
                PendingRecoveryCategory::RecoveryPublication,
                identity(&pending_message.message_id).or_else(|| identity(session_id)),
                "Recovery message publication",
            ),
            LegacyReconciliationRecord::OperationBinding { operation_id, .. } => (
                PendingRecoveryCategory::Unknown,
                identity(operation_id),
                "Pending local operation",
            ),
        },
        ObligationRecord::ProviderEstablish {
            operation_id,
            effect_identity,
            ..
        } => (
            PendingRecoveryCategory::ProviderEstablish,
            identity(operation_id).or_else(|| identity(effect_identity)),
            "Provider session establishment",
        ),
        ObligationRecord::TurnExecution {
            operation_id,
            turn_id,
            ..
        } => (
            PendingRecoveryCategory::TurnExecution,
            identity(operation_id).or_else(|| identity(turn_id)),
            "Agent turn execution",
        ),
        ObligationRecord::RecoveryReserved {
            recovery_id,
            effect_identity,
            ..
        }
        | ObligationRecord::RecoveryCompleted {
            recovery_id,
            effect_identity,
            ..
        } => (
            PendingRecoveryCategory::BackendRecovery,
            identity(recovery_id).or_else(|| identity(effect_identity)),
            "Recovery reconciliation",
        ),
        ObligationRecord::FeedbackReservation { .. }
        | ObligationRecord::Feedback { .. }
        | ObligationRecord::WorkflowExecution { .. }
        | ObligationRecord::RecoveryTransition { .. }
        | ObligationRecord::Observed { .. } => (
            PendingRecoveryCategory::Unknown,
            Some(obligation_id.to_string()),
            "Pending local operation",
        ),
    };
    descriptor(
        category,
        original_identity,
        known_status,
        safe_label,
        obligation_id,
    )
}

/// Returns the same immutable identity exposed by pending-recovery discovery
/// when a record must fence new mutation/effect admission. Normal live work
/// remains queueable; explicit reconciliation, recovery-owned handoffs, and
/// incompatible pending records fail closed.
pub(crate) fn unresolved_recovery_original_identity(
    obligation_id: &str,
    record: &ObligationRecord,
) -> Option<String> {
    if is_internal_feedback_reservation(record) {
        return None;
    }
    let state = obligation_state(record);
    let action_unresolved = recovery_action(record).is_some_and(|action| {
        matches!(
            action.state,
            ObligationStateRecord::Prepared
                | ObligationStateRecord::EffectReserved
                | ObligationStateRecord::OutcomeUnknown
                | ObligationStateRecord::ReconciliationRequired
        )
    });
    let explicitly_unresolved = matches!(
        state,
        Some(
            ObligationStateRecord::ReconciliationRequired
                | ObligationStateRecord::Failed
                | ObligationStateRecord::OutcomeUnknown
        )
    );
    let original = original_obligation(record);
    let recovery_owned = matches!(
        original,
        ObligationRecord::LegacyReconciliation { .. }
            | ObligationRecord::BackendSessionRecovery { .. }
            | ObligationRecord::WorkflowShutdown { .. }
            | ObligationRecord::WorkflowTurnCompletion { .. }
            | ObligationRecord::RecoveryPublication { .. }
            | ObligationRecord::RecoveryReserved { .. }
            | ObligationRecord::RecoveryCompleted { .. }
    );
    let closing = matches!(original, ObligationRecord::SessionClose { .. })
        && state != Some(ObligationStateRecord::Completed);
    let known_live = matches!(
        original,
        ObligationRecord::Send { .. }
            | ObligationRecord::ProviderEstablish { .. }
            | ObligationRecord::TurnExecution { .. }
            | ObligationRecord::PermissionResponse { .. }
            | ObligationRecord::StopInterrupt { .. }
            | ObligationRecord::TerminalCommit { .. }
            | ObligationRecord::SessionClose { .. }
    );
    let blocks = action_unresolved || explicitly_unresolved || recovery_owned || closing;
    if !blocks && known_live {
        // A live send TurnExecution remains effect_reserved until its
        // terminal closure. That alone must not disable normal queueing.
        return None;
    }
    Some(pending_recovery_descriptor(obligation_id, record).original_identity)
}

fn recovery_capabilities(
    obligation_id: &str,
    _revision: u64,
    record: &ObligationRecord,
    observation: Option<&AuthoritativeEffectObservation>,
    authority: &dyn OperationBindingAuthority,
    generation_id: &str,
    supports_read_again: bool,
) -> RecoveryCapabilities {
    let original = original_obligation(record);
    let permission_payload_valid = matches!(
        original,
        ObligationRecord::PermissionResponse {
            operation_id,
            effect_identity,
            session_id,
            turn_id,
            response,
            owner_access: true,
            state: ObligationStateRecord::Pending,
            ..
        } if !operation_id.is_empty()
            && !session_id.is_empty()
            && !turn_id.is_empty()
            && !response.request_id.is_empty()
            && effect_identity == &format!("permission-response:{operation_id}")
            && super::permission::canonical_payload(session_id, response).is_ok()
    );
    if matches!(
        original,
        ObligationRecord::PermissionResponse {
            state: ObligationStateRecord::Pending,
            ..
        }
    ) && !permission_payload_valid
    {
        return RecoveryCapabilities {
            state: RecoveryResourceState::Failed,
            safe_label: "Permission response payload is unavailable".to_string(),
            actions: vec![RecoveryActionKind::KeepForManualResolution],
            active_action: None,
        };
    }

    let has_nonterminal_action_claim = recovery_action(record).is_some_and(|active| {
        matches!(
            active.state,
            ObligationStateRecord::Prepared
                | ObligationStateRecord::EffectReserved
                | ObligationStateRecord::OutcomeUnknown
                | ObligationStateRecord::ReconciliationRequired
        )
    });
    let active_action = recovery_action(record).and_then(|active| {
        if !matches!(
            active.state,
            ObligationStateRecord::Prepared
                | ObligationStateRecord::EffectReserved
                | ObligationStateRecord::OutcomeUnknown
                | ObligationStateRecord::ReconciliationRequired
        ) {
            return None;
        }
        (active.effect_identity == obligation_id
            && active.action_id
                == derive_recovery_action_id(
                    authority,
                    generation_id,
                    obligation_id,
                    active.origin_revision,
                    active.action,
                ))
        .then_some(RecoveryActionIdentity {
            action_id: active.action_id.clone(),
            action: active.action,
            origin_revision: active.origin_revision,
        })
    });
    if has_nonterminal_action_claim && active_action.is_none() {
        return RecoveryCapabilities {
            state: RecoveryResourceState::Failed,
            safe_label: "Recovery action identity is incompatible".to_string(),
            actions: Vec::new(),
            active_action: None,
        };
    }
    if let Some(active_action) = active_action {
        return RecoveryCapabilities {
            state: RecoveryResourceState::Pending,
            safe_label: safe_obligation_label(obligation_id, record),
            actions: vec![active_action.action],
            active_action: Some(active_action),
        };
    }

    let mut actions = Vec::new();
    if supports_read_again {
        actions.push(RecoveryActionKind::ReadAgain);
    }
    // Permission retry is allowed only before the provider effect is claimed
    // and only from the saved exact payload. `effect_reserved` is ambiguous
    // and deliberately never exposes blind retry.
    let saved_idempotent_retry = matches!(
        original,
        ObligationRecord::LegacyReconciliation { safe_actions, .. }
            if safe_actions.contains(&RecoveryActionKind::RetrySameEffect)
                && obligation_state(record) != Some(ObligationStateRecord::EffectReserved)
    );
    if permission_payload_valid || saved_idempotent_retry {
        actions.push(RecoveryActionKind::RetrySameEffect);
    }
    if observation.is_some() {
        actions.push(RecoveryActionKind::UseObservedResult);
    }
    if observation.is_some_and(|proof| {
        proof.classification == RecoveryResultClassification::ConfirmedNoEffect && proof.cancellable
    }) {
        actions.push(RecoveryActionKind::CancelIfSafe);
    }
    actions.push(RecoveryActionKind::KeepForManualResolution);
    RecoveryCapabilities {
        state: RecoveryResourceState::Pending,
        safe_label: safe_obligation_label(obligation_id, record),
        actions,
        active_action: None,
    }
}

fn safe_obligation_label(obligation_id: &str, record: &ObligationRecord) -> String {
    pending_recovery_descriptor(obligation_id, record)
        .safe_label
        .to_string()
}

/// Decode and verify the proof captured by the backend at effect readback.
/// A public safe view is deliberately insufficient: the digest, effect
/// identity and origin revision must all be present in the immutable record.
fn authoritative_observation(
    record: &ObligationRecord,
    expected_revision: u64,
    authority: &dyn OperationBindingAuthority,
) -> Option<AuthoritativeEffectObservation> {
    fn observation(record: &ObligationRecord) -> Option<&AuthoritativeEffectObservationRecord> {
        match record {
            ObligationRecord::Observed { observation, .. } => Some(observation),
            ObligationRecord::RecoveryTransition { original, .. } => observation(original),
            ObligationRecord::Send { .. }
            | ObligationRecord::PermissionResponse { .. }
            | ObligationRecord::StopInterrupt { .. }
            | ObligationRecord::SessionClose { .. }
            | ObligationRecord::BackendSessionRecovery { .. }
            | ObligationRecord::WorkflowShutdown { .. }
            | ObligationRecord::WorkflowTurnCompletion { .. }
            | ObligationRecord::RecoveryPublication { .. }
            | ObligationRecord::LegacyReconciliation { .. }
            | ObligationRecord::ProviderEstablish { .. }
            | ObligationRecord::TurnExecution { .. }
            | ObligationRecord::TerminalCommit { .. }
            | ObligationRecord::RecoveryReserved { .. }
            | ObligationRecord::RecoveryCompleted { .. }
            | ObligationRecord::FeedbackReservation { .. }
            | ObligationRecord::Feedback { .. }
            | ObligationRecord::WorkflowExecution { .. } => None,
        }
    }
    let observation = observation(record)?;
    let effect_identity = observation.effect_identity.clone();
    if stable_effect_identity(record)
        .is_some_and(|stored_identity| stored_identity != effect_identity)
    {
        return None;
    }
    let origin_revision = observation.origin_revision;
    if origin_revision != expected_revision {
        return None;
    }
    let classification = observation.classification;
    let classification_raw = classification_label(classification);
    let cancellable = observation.cancellable;
    let safe_view = observation.safe_view.clone();
    if safe_view.len() > 48 * 1024 {
        return None;
    }
    let canonical = serde_json::to_vec(&serde_json::json!({
        "schema": "authoritative_effect_observation_v1",
        "effect_identity": effect_identity,
        "origin_revision": origin_revision,
        "classification": classification_raw,
        "cancellable": cancellable,
        "safe_view": safe_view,
    }))
    .ok()?;
    let result_hash = observation.result_sha256;
    if !constant_time_eq_32(&result_hash, &authority.digest(&canonical)) {
        return None;
    }
    let proof_mac = observation.proof_mac;
    if !constant_time_eq_32(&proof_mac, &authority.mac(&canonical)) {
        return None;
    }
    Some(AuthoritativeEffectObservation {
        effect_identity,
        origin_revision,
        result_hash,
        safe_view,
        classification,
        cancellable,
    })
}

fn stable_effect_identity(record: &ObligationRecord) -> Option<String> {
    match original_obligation(record) {
        ObligationRecord::Send { obligation_id, .. } => Some(obligation_id.clone()),
        ObligationRecord::PermissionResponse {
            effect_identity, ..
        }
        | ObligationRecord::WorkflowShutdown {
            effect_identity, ..
        }
        | ObligationRecord::ProviderEstablish {
            effect_identity, ..
        }
        | ObligationRecord::RecoveryReserved {
            effect_identity, ..
        }
        | ObligationRecord::RecoveryCompleted {
            effect_identity, ..
        } => Some(effect_identity.clone()),
        ObligationRecord::SessionClose { obligation_id, .. } => Some(obligation_id.clone()),
        ObligationRecord::BackendSessionRecovery {
            session_id,
            recovery_id,
            ..
        } => Some(format!("backend-recovery:{session_id}:{recovery_id}")),
        ObligationRecord::WorkflowTurnCompletion {
            terminal_identity, ..
        } => Some(terminal_identity.clone()),
        ObligationRecord::RecoveryPublication { message_id, .. } => Some(message_id.clone()),
        ObligationRecord::TerminalCommit {
            terminal_identity, ..
        } => Some(terminal_identity.clone()),
        ObligationRecord::StopInterrupt { .. }
        | ObligationRecord::LegacyReconciliation { .. }
        | ObligationRecord::TurnExecution { .. }
        | ObligationRecord::FeedbackReservation { .. }
        | ObligationRecord::Feedback { .. }
        | ObligationRecord::WorkflowExecution { .. }
        | ObligationRecord::RecoveryTransition { .. }
        | ObligationRecord::Observed { .. } => None,
    }
}

fn classification_label(value: RecoveryResultClassification) -> &'static str {
    match value {
        RecoveryResultClassification::Pending => "pending",
        RecoveryResultClassification::Succeeded => "succeeded",
        RecoveryResultClassification::ConfirmedNoEffect => "confirmed_no_effect",
        RecoveryResultClassification::Ambiguous => "ambiguous",
        RecoveryResultClassification::CancelledBeforeEffect => "cancelled_before_effect",
        RecoveryResultClassification::Unchanged => "unchanged",
    }
}

fn classification_allowed_for_action(
    action: RecoveryActionKind,
    observation: Option<&AuthoritativeEffectObservation>,
    classification: RecoveryResultClassification,
) -> bool {
    match action {
        RecoveryActionKind::ReadAgain => matches!(
            classification,
            RecoveryResultClassification::Pending
                | RecoveryResultClassification::Succeeded
                | RecoveryResultClassification::ConfirmedNoEffect
                | RecoveryResultClassification::Ambiguous
        ),
        RecoveryActionKind::RetrySameEffect => matches!(
            classification,
            RecoveryResultClassification::Pending
                | RecoveryResultClassification::Succeeded
                | RecoveryResultClassification::Ambiguous
        ),
        RecoveryActionKind::UseObservedResult => {
            observation.is_some_and(|observation| observation.classification == classification)
        }
        RecoveryActionKind::CancelIfSafe => {
            classification == RecoveryResultClassification::CancelledBeforeEffect
                && observation.is_some_and(|observation| {
                    observation.classification == RecoveryResultClassification::ConfirmedNoEffect
                        && observation.cancellable
                })
        }
        RecoveryActionKind::KeepForManualResolution => {
            classification == RecoveryResultClassification::Unchanged
        }
    }
}

fn decode_saved_status(saved: &RecoveryActionView) -> Option<RecoveryActionStatus> {
    let Some(completed) = saved.completed.as_ref() else {
        let RecoveryAttemptRecord::Obligation { state, failure, .. } = &saved.attempt else {
            return None;
        };
        return Some(match state {
            ObligationStateRecord::Prepared | ObligationStateRecord::EffectReserved => {
                RecoveryActionStatus::InProgress {
                    action_id: saved.action_id.clone(),
                }
            }
            ObligationStateRecord::OutcomeUnknown => RecoveryActionStatus::OutcomeUnknown {
                action_id: saved.action_id.clone(),
            },
            ObligationStateRecord::ReconciliationRequired | ObligationStateRecord::Failed => {
                RecoveryActionStatus::ReconciliationRequired {
                    action_id: saved.action_id.clone(),
                    failure: failure.clone()?,
                }
            }
            ObligationStateRecord::Pending
            | ObligationStateRecord::Running
            | ObligationStateRecord::WaitingApproval
            | ObligationStateRecord::Completed
            | ObligationStateRecord::Cancelled => return None,
        });
    };
    Some(RecoveryActionStatus::Completed {
        action_id: saved.action_id.clone(),
        result: decode_recovery_completed_result(completed)?,
    })
}

pub(crate) fn decode_recovery_completed_result(
    completed: &RecoveryResultRecord,
) -> Option<RecoveryActionCompletedResult> {
    let RecoveryResultRecord::Action(completed) = completed else {
        return None;
    };
    let classification = completed.classification;
    let resource_view = match &completed.resource_view {
        RecoveryResourceViewRecord::Operation { kind, operation_id } => {
            format!("{} operation {operation_id}", kind.label())
        }
        RecoveryResourceViewRecord::Session { session_id } => {
            format!("Session {session_id}")
        }
        RecoveryResourceViewRecord::BackendRecovery {
            session_id,
            recovery_id,
        } => format!("Backend recovery {recovery_id} for session {session_id}"),
        RecoveryResourceViewRecord::ShutdownTarget {
            plan,
            ordinal,
            target_id,
            state,
        } => format!(
            "Shutdown target {target_id} in {}/{} at ordinal {ordinal}: {state:?}",
            plan.plan_id, plan.epoch
        ),
        RecoveryResourceViewRecord::SafeSummary(summary) => summary.clone(),
        RecoveryResourceViewRecord::ReconciliationRequired { failure } => {
            failure.label.value().to_string()
        }
    };
    if resource_view.len() > 64 * 1024 {
        return None;
    }
    let outcome = match completed.outcome {
        RecoveryResultOutcomeRecord::Pending => RecoveryActionResultOutcome::Pending,
        RecoveryResultOutcomeRecord::Terminal => RecoveryActionResultOutcome::Terminal,
        RecoveryResultOutcomeRecord::Unchanged => RecoveryActionResultOutcome::Unchanged,
    };
    if outcome != result_outcome(classification) {
        return None;
    }
    let resource_revision = completed.resource_revision;
    if resource_revision > i64::MAX as u64 {
        return None;
    }
    let canonical_result_sha256 = hex::encode(completed.canonical_result_sha256);
    let expected = canonical_result_sha256_for_decode(
        outcome,
        classification,
        resource_revision,
        &resource_view,
    )?;
    if canonical_result_sha256 != expected {
        return None;
    }
    Some(RecoveryActionCompletedResult {
        outcome,
        classification,
        resource_revision,
        canonical_result_sha256,
        resource_view,
    })
}

fn result_outcome(classification: RecoveryResultClassification) -> RecoveryActionResultOutcome {
    match classification {
        RecoveryResultClassification::Pending
        | RecoveryResultClassification::ConfirmedNoEffect
        | RecoveryResultClassification::Ambiguous => RecoveryActionResultOutcome::Pending,
        RecoveryResultClassification::Succeeded
        | RecoveryResultClassification::CancelledBeforeEffect => {
            RecoveryActionResultOutcome::Terminal
        }
        RecoveryResultClassification::Unchanged => RecoveryActionResultOutcome::Unchanged,
    }
}

fn result_outcome_label(outcome: RecoveryActionResultOutcome) -> &'static str {
    match outcome {
        RecoveryActionResultOutcome::Pending => "pending",
        RecoveryActionResultOutcome::Terminal => "terminal",
        RecoveryActionResultOutcome::Unchanged => "unchanged",
    }
}

fn canonical_result_bytes(
    outcome: RecoveryActionResultOutcome,
    classification: RecoveryResultClassification,
    resource_revision: u64,
    resource_view: &str,
) -> Option<Vec<u8>> {
    serde_json::to_vec(&serde_json::json!({
        "schema": "recovery_action_canonical_result_v1",
        "outcome": result_outcome_label(outcome),
        "classification": classification_label(classification),
        "resource_revision": resource_revision,
        "resource_view": resource_view,
    }))
    .ok()
}

fn canonical_result_sha256(
    outcome: RecoveryActionResultOutcome,
    classification: RecoveryResultClassification,
    resource_revision: u64,
    resource_view: &str,
) -> Result<String, RecoveryActionError> {
    let bytes = canonical_result_bytes(outcome, classification, resource_revision, resource_view)
        .ok_or_else(|| internal("canonical-result"))?;
    Ok(hex::encode(sha2::Sha256::digest(bytes)))
}

fn canonical_result_sha256_for_decode(
    outcome: RecoveryActionResultOutcome,
    classification: RecoveryResultClassification,
    resource_revision: u64,
    resource_view: &str,
) -> Option<String> {
    canonical_result_bytes(outcome, classification, resource_revision, resource_view)
        .map(|bytes| hex::encode(sha2::Sha256::digest(bytes)))
}

pub(crate) fn encode_recovery_completed_result(
    outcome: RecoveryActionResultOutcome,
    classification: RecoveryResultClassification,
    resource_revision: u64,
    resource_view: String,
) -> Result<(RecoveryActionCompletedResult, RecoveryResultRecord), RecoveryActionError> {
    if outcome != result_outcome(classification)
        || resource_revision > i64::MAX as u64
        || resource_view.len() > 64 * 1024
    {
        return Err(RecoveryActionError::InvalidRequest);
    }
    let canonical_result_sha256 =
        canonical_result_sha256(outcome, classification, resource_revision, &resource_view)?;
    let canonical_result_hash: [u8; 32] = hex::decode(&canonical_result_sha256)
        .map_err(|_| internal("canonical-result-hash"))?
        .try_into()
        .map_err(|_| internal("canonical-result-hash"))?;
    let payload = RecoveryResultRecord::Action(RecoveryActionResultRecord {
        outcome: match outcome {
            RecoveryActionResultOutcome::Pending => RecoveryResultOutcomeRecord::Pending,
            RecoveryActionResultOutcome::Terminal => RecoveryResultOutcomeRecord::Terminal,
            RecoveryActionResultOutcome::Unchanged => RecoveryResultOutcomeRecord::Unchanged,
        },
        classification,
        resource_revision,
        canonical_result_sha256: canonical_result_hash,
        resource_view: RecoveryResourceViewRecord::SafeSummary(resource_view.clone()),
    });
    let result = RecoveryActionCompletedResult {
        outcome,
        classification,
        resource_revision,
        canonical_result_sha256,
        resource_view,
    };
    Ok((result, payload))
}

fn saved_request_matches(
    saved: &RecoveryActionView,
    request: &RecoveryActionRequest,
) -> Result<bool, RecoveryActionError> {
    Ok(matches!(
        &saved.attempt,
        RecoveryAttemptRecord::Obligation {
            obligation_id,
            origin_revision,
            action,
            effect_identity,
            ..
        } if obligation_id == &request.obligation_id
            && *origin_revision == request.origin_revision
            && *action == request.action
            && effect_identity == &request.obligation_id
    ))
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)] // Private mapping helpers below are shared by production paths above.
mod observation_tests {
    use sha2::{Digest, Sha256};

    use super::{
        authoritative_observation, canonical_result_sha256, send_terminal_operation_is_bound,
        stop_completion_operation_is_bound, valid_recovery_operation, RecoveryActionResultOutcome,
    };
    use crate::domain::agent_session::entities::TurnResult;
    use crate::domain::agent_session::events::{
        RecoveryResultClassification, SendDisposition, StopResolution,
    };
    use crate::domain::local_event::{
        AuthoritativeEffectObservationRecord, LocalStateMutation, ObligationRecord,
        ObligationStateRecord, OperationKind, OperationReceiptRecord, OperationRecordMutation,
        OperationStatusRecord, OperationStatusValue, RecordAuthentication, Revision, RevisionGuard,
        SendObligationDispositionRecord, SendObligationKindRecord,
    };
    use crate::usecase::agent_session::operation::OperationBindingAuthority;

    struct Authority;

    impl crate::usecase::agent_session::operation::RecoveryResultCanonicalizer for Authority {}

    impl OperationBindingAuthority for Authority {
        fn mac(&self, message: &[u8]) -> [u8; 32] {
            Sha256::digest(message).into()
        }

        fn digest(&self, message: &[u8]) -> [u8; 32] {
            Sha256::digest(message).into()
        }

        fn seal_command(&self, _context: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, ()> {
            Ok(plaintext.to_vec())
        }

        fn open_command(&self, _context: &[u8], envelope: &[u8]) -> Result<Vec<u8>, ()> {
            Ok(envelope.to_vec())
        }
    }

    #[test]
    fn observed_result_requires_backend_effect_revision_and_hash_evidence() {
        let safe_view = "backend-confirmed";
        let canonical = serde_json::to_vec(&serde_json::json!({
            "schema": "authoritative_effect_observation_v1",
            "effect_identity": "effect-1",
            "origin_revision": 7,
            "classification": "succeeded",
            "cancellable": false,
            "safe_view": safe_view,
        }))
        .unwrap();
        let record = ObligationRecord::Observed {
            original: Box::new(ObligationRecord::RecoveryReserved {
                recovery_id: "recovery-1".to_string(),
                effect_identity: "effect-1".to_string(),
                state: ObligationStateRecord::ReconciliationRequired,
            }),
            observation: AuthoritativeEffectObservationRecord {
                effect_identity: "effect-1".to_string(),
                origin_revision: 7,
                classification: RecoveryResultClassification::Succeeded,
                cancellable: false,
                safe_view: safe_view.to_string(),
                result_sha256: Sha256::digest(&canonical).into(),
                proof_mac: Sha256::digest(&canonical).into(),
            },
        };
        assert!(authoritative_observation(&record, 7, &Authority).is_some());
        assert!(authoritative_observation(&record, 8, &Authority).is_none());

        let mut forged = record.clone();
        let ObligationRecord::Observed { observation, .. } = &mut forged else {
            unreachable!();
        };
        observation.safe_view = "client-forged".to_string();
        assert!(authoritative_observation(&forged, 7, &Authority).is_none());
        let mut forged_classification = record.clone();
        let ObligationRecord::Observed { observation, .. } = &mut forged_classification else {
            unreachable!();
        };
        observation.classification = RecoveryResultClassification::ConfirmedNoEffect;
        assert!(authoritative_observation(&forged_classification, 7, &Authority).is_none());
        let mut forged_cancellable = record.clone();
        let ObligationRecord::Observed { observation, .. } = &mut forged_cancellable else {
            unreachable!();
        };
        observation.cancellable = true;
        assert!(authoritative_observation(&forged_cancellable, 7, &Authority).is_none());
        assert!(authoritative_observation(
            &ObligationRecord::RecoveryReserved {
                recovery_id: "recovery-1".to_string(),
                effect_identity: "effect-1".to_string(),
                state: ObligationStateRecord::ReconciliationRequired,
            },
            7,
            &Authority,
        )
        .is_none());
    }

    #[test]
    fn canonical_result_hash_binds_classification_revision_and_view() {
        let base = canonical_result_sha256(
            RecoveryActionResultOutcome::Pending,
            RecoveryResultClassification::Pending,
            4,
            "safe",
        )
        .unwrap();
        assert_ne!(
            base,
            canonical_result_sha256(
                RecoveryActionResultOutcome::Terminal,
                RecoveryResultClassification::Succeeded,
                4,
                "safe",
            )
            .unwrap()
        );
        assert_ne!(
            base,
            canonical_result_sha256(
                RecoveryActionResultOutcome::Pending,
                RecoveryResultClassification::Pending,
                5,
                "safe",
            )
            .unwrap()
        );
        assert_ne!(
            base,
            canonical_result_sha256(
                RecoveryActionResultOutcome::Pending,
                RecoveryResultClassification::Pending,
                4,
                "other",
            )
            .unwrap()
        );
    }

    #[test]
    fn f05_stop_read_again_rejects_a_public_resolution_mixed_with_the_durable_winner() {
        let receipt = OperationReceiptRecord::Stop {
            operation_id: "stop-op".to_string(),
            session_id: "session".to_string(),
            turn_id: "7".to_string(),
            accepted_revision: 3,
            authentication: RecordAuthentication {
                principal_mac: [1; 32],
                binding_hmac: [2; 32],
            },
        };
        let succeeded = OperationStatusValue::StopCompleted {
            resolution: StopResolution::Succeeded,
        };
        let superseded = OperationStatusValue::StopCompleted {
            resolution: StopResolution::Superseded,
        };

        assert!(stop_completion_operation_is_bound(
            &receipt,
            &succeeded,
            "stop-op",
            "session",
            "7",
            StopResolution::Succeeded,
        ));
        assert!(!stop_completion_operation_is_bound(
            &receipt,
            &superseded,
            "stop-op",
            "session",
            "7",
            StopResolution::Succeeded,
        ));
    }

    #[test]
    fn f05_send_read_again_rejects_a_terminal_operation_receipt_for_another_turn() {
        let terminal_result = TurnResult::Completed {
            stop_reason: None,
            token_usage: None,
        };
        let wrong_turn_receipt = OperationReceiptRecord::Send {
            operation_id: "send-op".to_string(),
            session_id: "session".to_string(),
            input_ref: "input".to_string(),
            disposition: SendDisposition::StartedTurn {
                turn_id: "8".to_string(),
            },
            authentication: RecordAuthentication {
                principal_mac: [3; 32],
                binding_hmac: [4; 32],
            },
        };
        let status = OperationStatusValue::Terminal {
            result: terminal_result.clone(),
        };

        assert!(!send_terminal_operation_is_bound(
            &wrong_turn_receipt,
            &status,
            "send-op",
            "session",
            "7",
            &terminal_result,
        ));

        let mutation = LocalStateMutation::OperationRecord(OperationRecordMutation {
            kind: OperationKind::Send,
            operation_id: "send-op".to_string(),
            receipt: wrong_turn_receipt,
            latest_status: OperationStatusRecord {
                kind: OperationKind::Send,
                migration_quit: false,
                value: status,
            },
            expected: RevisionGuard::Expected(Revision::new(0).unwrap()),
            revision: Revision::new(1).unwrap(),
        });
        let LocalStateMutation::OperationRecord(mutation) = mutation else {
            unreachable!();
        };
        let original = ObligationRecord::Send {
            obligation_id: "send-op.exec".to_string(),
            operation_id: "send-op".to_string(),
            session_id: "session".to_string(),
            kind: SendObligationKindRecord::TurnExecution,
            disposition: SendObligationDispositionRecord::StartedTurn,
            human_message_id: Some("human".to_string()),
            assistant_message_id: Some("assistant".to_string()),
            turn_id: Some("7".to_string()),
            reserved_turn_id: Some("7".to_string()),
            dependency_obligation_ids: Vec::new(),
            canonical_payload: "{}".to_string(),
            state: ObligationStateRecord::EffectReserved,
        };
        assert!(!valid_recovery_operation(&mutation, "session", &original,));
    }
}

fn map_query_error(error: LocalEventQueryError) -> RecoveryActionError {
    match error {
        LocalEventQueryError::NotFound => RecoveryActionError::NotFound,
        LocalEventQueryError::QueryBusy => RecoveryActionError::QueryBusy,
        LocalEventQueryError::DeadlineExceeded => RecoveryActionError::DeadlineExceeded,
        LocalEventQueryError::CursorMismatch => RecoveryActionError::CursorMismatch,
        LocalEventQueryError::CursorExpired => RecoveryActionError::CursorExpired,
        LocalEventQueryError::SnapshotMismatch => RecoveryActionError::SnapshotMismatch,
        LocalEventQueryError::DetailsCompacted => RecoveryActionError::DetailsCompacted,
        LocalEventQueryError::ResponseTooLarge => RecoveryActionError::ResponseTooLarge,
        LocalEventQueryError::StorageUnavailable { failure } => {
            RecoveryActionError::StorageUnavailable { failure }
        }
        LocalEventQueryError::Corrupt { correlation_id }
        | LocalEventQueryError::Internal { correlation_id }
        | LocalEventQueryError::IncompatibleStoredEvent { correlation_id }
        | LocalEventQueryError::ReplayRequired { correlation_id } => {
            RecoveryActionError::Internal { correlation_id }
        }
        _ => RecoveryActionError::InvalidRequest,
    }
}

fn map_commit_error<T>(error: CommitBatchError, action_id: &str) -> Result<T, RecoveryActionError> {
    match error {
        CommitBatchError::PayloadConflict | CommitBatchError::StreamHeadConflict { .. } => {
            Err(RecoveryActionError::Internal {
                correlation_id: format!("recovery-action-conflict-{action_id}"),
            })
        }
        CommitBatchError::OutcomeUnknown { .. } => Err(RecoveryActionError::Internal {
            correlation_id: format!("outcome-unknown-{action_id}"),
        }),
        CommitBatchError::StorageUnavailable { failure } if failure.is_shutdown_in_progress() => {
            Err(RecoveryActionError::ShutdownInProgress)
        }
        CommitBatchError::StorageUnavailable { failure } => {
            Err(RecoveryActionError::StorageUnavailable { failure })
        }
        CommitBatchError::CapacityExceeded | CommitBatchError::SequenceExhausted => {
            Err(RecoveryActionError::InvalidRequest)
        }
        CommitBatchError::Corrupt { correlation_id } => {
            Err(RecoveryActionError::Internal { correlation_id })
        }
    }
}

fn internal(label: &str) -> RecoveryActionError {
    RecoveryActionError::Internal {
        correlation_id: format!("recovery-{label}-{}", uuid::Uuid::new_v4()),
    }
}
