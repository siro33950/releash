//! Aggregate invariants for one durable operation-record row.

use super::{
    OperationKind, OperationReceiptRecord, OperationStatusRecord, OperationStatusValue,
    StopResolutionKind, StopResolutionMutation, TerminalRecordMutation, TerminalResultRecord,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationRecordValidationError {
    RowIdentityMismatch,
    ReceiptFamilyMismatch,
    StatusFamilyMismatch,
    MigrationQuitMismatch,
    IllegalStatus,
    EmbeddedRelationMismatch,
}

pub fn validate_operation_record(
    row_kind: OperationKind,
    row_operation_id: &str,
    receipt: &OperationReceiptRecord,
    status: &OperationStatusRecord,
) -> Result<(), OperationRecordValidationError> {
    let (receipt_kind, receipt_operation_id, receipt_migration_quit) = match receipt {
        OperationReceiptRecord::Send { operation_id, .. } => {
            (OperationKind::Send, operation_id.as_str(), false)
        }
        OperationReceiptRecord::PermissionResponse { operation_id, .. } => (
            OperationKind::PermissionResponse,
            operation_id.as_str(),
            false,
        ),
        OperationReceiptRecord::Stop { operation_id, .. } => {
            (OperationKind::Stop, operation_id.as_str(), false)
        }
        OperationReceiptRecord::SessionLifecycle { operation_id, .. } => (
            OperationKind::SessionLifecycle,
            operation_id.as_str(),
            false,
        ),
        OperationReceiptRecord::ApplicationQuit { operation_id, .. } => {
            (OperationKind::ApplicationQuit, operation_id.as_str(), false)
        }
        OperationReceiptRecord::MigrationApplicationQuit { operation_id, .. } => {
            (OperationKind::ApplicationQuit, operation_id.as_str(), true)
        }
    };

    if receipt_operation_id != row_operation_id {
        return Err(OperationRecordValidationError::RowIdentityMismatch);
    }
    if receipt_kind != row_kind {
        return Err(OperationRecordValidationError::ReceiptFamilyMismatch);
    }
    if status.kind != row_kind {
        return Err(OperationRecordValidationError::StatusFamilyMismatch);
    }
    if status.migration_quit != receipt_migration_quit {
        return Err(OperationRecordValidationError::MigrationQuitMismatch);
    }

    let legal = matches!(
        (row_kind, status.migration_quit, &status.value),
        (
            OperationKind::Send,
            false,
            OperationStatusValue::AwaitingProviderStart { .. }
                | OperationStatusValue::Queued { .. }
                | OperationStatusValue::ProviderStartReserved { .. }
                | OperationStatusValue::Running { .. }
                | OperationStatusValue::ReconciliationRequired { .. }
                | OperationStatusValue::Failed { .. }
                | OperationStatusValue::Terminal { .. },
        ) | (
            OperationKind::PermissionResponse,
            false,
            OperationStatusValue::AwaitingProviderResponse { .. }
                | OperationStatusValue::PermissionCompleted { .. }
                | OperationStatusValue::ReconciliationRequired { .. }
                | OperationStatusValue::Failed { .. },
        ) | (
            OperationKind::Stop,
            false,
            OperationStatusValue::Accepted
                | OperationStatusValue::StopCompleted { .. }
                | OperationStatusValue::ReconciliationRequired { .. },
        ) | (
            OperationKind::SessionLifecycle,
            false,
            OperationStatusValue::Accepted
                | OperationStatusValue::Completed
                | OperationStatusValue::ReconciliationRequired { .. },
        ) | (
            OperationKind::ApplicationQuit,
            false,
            OperationStatusValue::Preparing
                | OperationStatusValue::Activated
                | OperationStatusValue::Completed
                | OperationStatusValue::OutcomeUnknown { .. }
                | OperationStatusValue::FailedBeforeActivation { .. }
                | OperationStatusValue::ReconciliationRequired { .. },
        ) | (
            OperationKind::ApplicationQuit,
            true,
            OperationStatusValue::ExitPending
                | OperationStatusValue::Exited
                | OperationStatusValue::ReconciliationRequired { .. },
        )
    );
    if !legal {
        return Err(OperationRecordValidationError::IllegalStatus);
    }

    match (receipt, &status.value) {
        (
            OperationReceiptRecord::Send {
                disposition:
                    crate::domain::agent_session::events::SendDisposition::StartedTurn {
                        turn_id: accepted_turn_id,
                    },
                ..
            },
            OperationStatusValue::Running { turn_id },
        ) if turn_id != accepted_turn_id => {
            return Err(OperationRecordValidationError::EmbeddedRelationMismatch)
        }
        (
            OperationReceiptRecord::Send {
                disposition:
                    crate::domain::agent_session::events::SendDisposition::Queued {
                        queue_item_id: accepted_queue_item_id,
                    },
                ..
            },
            OperationStatusValue::Queued { queue_item_id, .. },
        ) if queue_item_id != accepted_queue_item_id => {
            return Err(OperationRecordValidationError::EmbeddedRelationMismatch)
        }
        (
            OperationReceiptRecord::ApplicationQuit {
                operation_id,
                plan: accepted_plan,
                ..
            },
            OperationStatusValue::OutcomeUnknown {
                operation_id: status_operation_id,
                plan,
                ..
            },
        ) if status_operation_id != operation_id || plan != accepted_plan => {
            return Err(OperationRecordValidationError::EmbeddedRelationMismatch)
        }
        _ => {}
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalAggregateValidationError {
    RowIdentityMismatch,
    ResultFamilyMismatch,
}

pub fn validate_terminal_record(
    terminal: &TerminalRecordMutation,
) -> Result<(), TerminalAggregateValidationError> {
    let valid = match &terminal.result {
        TerminalResultRecord::AgentTurn {
            session_id,
            turn_id,
            ..
        } => {
            session_id == &terminal.session_id
                && turn_id == &terminal.turn_id
                && !terminal.terminal_identity.is_empty()
        }
        TerminalResultRecord::SessionClosed { operation_id, .. }
        | TerminalResultRecord::Stop { operation_id, .. } => {
            // The row identity is the durable winner for the turn. It may be
            // different from the operation that produced the winning result
            // (for example, a send readback can observe a session-close
            // winner), so both identities must be present but must not be
            // conflated.
            !operation_id.is_empty() && !terminal.terminal_identity.is_empty()
        }
        TerminalResultRecord::LegacyStopResolution {
            session_id,
            turn_id,
            operation_id,
            ..
        } => {
            session_id == &terminal.session_id
                && turn_id == &terminal.turn_id
                && !operation_id.is_empty()
                && !terminal.terminal_identity.is_empty()
        }
        TerminalResultRecord::StopSuperseded { .. } => false,
    };
    if valid {
        Ok(())
    } else {
        Err(TerminalAggregateValidationError::RowIdentityMismatch)
    }
}

pub fn validate_stop_resolution(
    resolution: &StopResolutionMutation,
) -> Result<(), TerminalAggregateValidationError> {
    let valid = match (&resolution.resolution, &resolution.detail) {
        (StopResolutionKind::Succeeded, TerminalResultRecord::Stop { operation_id, .. }) => {
            operation_id == &resolution.stop_operation_id
        }
        (
            StopResolutionKind::Superseded,
            TerminalResultRecord::StopSuperseded {
                terminal_identity, ..
            },
        ) => !terminal_identity.is_empty(),
        (
            stored,
            TerminalResultRecord::LegacyStopResolution {
                operation_id,
                resolution: legacy,
                ..
            },
        ) => {
            operation_id == &resolution.stop_operation_id
                && matches!(
                    (stored, legacy),
                    (
                        StopResolutionKind::Succeeded,
                        crate::domain::agent_session::events::StopResolution::Succeeded,
                    ) | (
                        StopResolutionKind::Superseded,
                        crate::domain::agent_session::events::StopResolution::Superseded,
                    )
                )
        }
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(TerminalAggregateValidationError::ResultFamilyMismatch)
    }
}
