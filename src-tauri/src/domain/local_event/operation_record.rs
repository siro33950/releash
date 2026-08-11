use super::{OperationKind, OperationReceiptRecord, OperationStatusRecord, OperationStatusValue};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationRecordValidationError {
    RowIdentityMismatch,
    ReceiptFamilyMismatch,
    StatusFamilyMismatch,
    IllegalStatus,
    EmbeddedRelationMismatch,
}

pub fn validate_operation_record(
    row_kind: OperationKind,
    row_operation_id: &str,
    receipt: &OperationReceiptRecord,
    status: &OperationStatusRecord,
) -> Result<(), OperationRecordValidationError> {
    let OperationReceiptRecord::ApplicationQuit {
        operation_id,
        plan: accepted_plan,
        ..
    } = receipt;
    if operation_id != row_operation_id {
        return Err(OperationRecordValidationError::RowIdentityMismatch);
    }
    if row_kind != OperationKind::ApplicationQuit {
        return Err(OperationRecordValidationError::ReceiptFamilyMismatch);
    }
    if status.kind != OperationKind::ApplicationQuit {
        return Err(OperationRecordValidationError::StatusFamilyMismatch);
    }
    if !matches!(
        status.value,
        OperationStatusValue::Preparing
            | OperationStatusValue::Activated
            | OperationStatusValue::Completed
            | OperationStatusValue::OutcomeUnknown { .. }
            | OperationStatusValue::FailedBeforeActivation { .. }
            | OperationStatusValue::ReconciliationRequired { .. }
    ) {
        return Err(OperationRecordValidationError::IllegalStatus);
    }
    if let OperationStatusValue::OutcomeUnknown {
        operation_id: status_operation_id,
        plan,
        ..
    } = &status.value
    {
        if status_operation_id != operation_id || plan != accepted_plan {
            return Err(OperationRecordValidationError::EmbeddedRelationMismatch);
        }
    }
    Ok(())
}
