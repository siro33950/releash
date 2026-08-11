use base64::Engine;
use sha2::Digest;

use crate::domain::local_event::{
    RecoveryActionKind, RecoveryResourceViewRecord, RecoveryResultClassification,
    RecoveryResultOutcomeRecord, RecoveryResultRecord, SafeOperationFailure,
};

use super::ports::OperationBindingAuthority;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryActionIdentity {
    pub action_id: String,
    pub action: RecoveryActionKind,
    pub origin_revision: u64,
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

pub(crate) fn derive_recovery_action_id(
    authority: &dyn OperationBindingAuthority,
    installation_id: &str,
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
        installation_id.as_bytes(),
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

pub(crate) fn decode_recovery_completed_result(
    completed: &RecoveryResultRecord,
) -> Option<RecoveryActionCompletedResult> {
    let RecoveryResultRecord::Action(completed) = completed;
    let classification = completed.classification;
    let resource_view = match &completed.resource_view {
        RecoveryResourceViewRecord::ShutdownTarget {
            plan,
            ordinal,
            target_id,
            state,
        } => format!(
            "Shutdown target {target_id} in {} at ordinal {ordinal}: {state:?}",
            plan.shutdown_id
        ),
        RecoveryResourceViewRecord::SafeSummary(summary) => summary.clone(),
    };
    if resource_view.len() > 64 * 1024 {
        return None;
    }
    let outcome = match completed.outcome {
        RecoveryResultOutcomeRecord::Pending => RecoveryActionResultOutcome::Pending,
        RecoveryResultOutcomeRecord::Terminal => RecoveryActionResultOutcome::Terminal,
        RecoveryResultOutcomeRecord::Unchanged => RecoveryActionResultOutcome::Unchanged,
    };
    if outcome != result_outcome(classification) || completed.resource_revision > i64::MAX as u64 {
        return None;
    }
    let canonical_result_sha256 = hex::encode(completed.canonical_result_sha256);
    let expected = canonical_result_sha256_for_decode(
        outcome,
        classification,
        completed.resource_revision,
        &resource_view,
    )?;
    if canonical_result_sha256 != expected {
        return None;
    }
    Some(RecoveryActionCompletedResult {
        outcome,
        classification,
        resource_revision: completed.resource_revision,
        canonical_result_sha256,
        resource_view,
    })
}

fn result_outcome(classification: RecoveryResultClassification) -> RecoveryActionResultOutcome {
    match classification {
        RecoveryResultClassification::Pending | RecoveryResultClassification::Ambiguous => {
            RecoveryActionResultOutcome::Pending
        }
        RecoveryResultClassification::Succeeded
        | RecoveryResultClassification::ConfirmedNoEffect
        | RecoveryResultClassification::CancelledBeforeEffect => {
            RecoveryActionResultOutcome::Terminal
        }
        RecoveryResultClassification::Unchanged => RecoveryActionResultOutcome::Unchanged,
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

fn result_outcome_label(outcome: RecoveryActionResultOutcome) -> &'static str {
    match outcome {
        RecoveryActionResultOutcome::Pending => "pending",
        RecoveryActionResultOutcome::Terminal => "terminal",
        RecoveryActionResultOutcome::Unchanged => "unchanged",
    }
}

fn canonical_result_sha256_for_decode(
    outcome: RecoveryActionResultOutcome,
    classification: RecoveryResultClassification,
    resource_revision: u64,
    resource_view: &str,
) -> Option<String> {
    serde_json::to_vec(&serde_json::json!({
        "schema": "recovery_action_canonical_result_v1",
        "outcome": result_outcome_label(outcome),
        "classification": classification_label(classification),
        "resource_revision": resource_revision,
        "resource_view": resource_view,
    }))
    .ok()
    .map(|bytes| hex::encode(sha2::Sha256::digest(bytes)))
}
