//! Shared operation identity helpers.

use crate::domain::local_event::SessionOperationFailureKind;

pub fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// Stable public protocol label for the closed failure vocabulary.
pub fn failure_kind_label(kind: SessionOperationFailureKind) -> &'static str {
    match kind {
        SessionOperationFailureKind::StorageUnavailable => "storage_unavailable",
        SessionOperationFailureKind::StorageCorrupt => "storage_corrupt",
        SessionOperationFailureKind::MigrationBlocked => "migration_blocked",
        SessionOperationFailureKind::PersistFailure => "persist_failure",
        SessionOperationFailureKind::ProtocolIncompatible => "protocol_incompatible",
        SessionOperationFailureKind::ProviderUnavailable => "provider_unavailable",
        SessionOperationFailureKind::ExternalEffectFailed => "external_effect_failed",
        SessionOperationFailureKind::OutcomeUnknown => "outcome_unknown",
        SessionOperationFailureKind::DeadlineExceeded => "deadline_exceeded",
        SessionOperationFailureKind::CapacityExceeded => "capacity_exceeded",
        SessionOperationFailureKind::StopCapacityExceeded => "stop_capacity_exceeded",
        SessionOperationFailureKind::ShutdownAuthorityMismatch => "shutdown_authority_mismatch",
        SessionOperationFailureKind::TargetRevisionChanged => "target_revision_changed",
        SessionOperationFailureKind::OwnerRevisionChanged => "owner_revision_changed",
        SessionOperationFailureKind::RuntimeGenerationChanged => "runtime_generation_changed",
        SessionOperationFailureKind::InvalidEffectIntent => "invalid_effect_intent",
        SessionOperationFailureKind::PreviousShutdownReconciliationRequired => {
            "previous_shutdown_reconciliation_required"
        }
        SessionOperationFailureKind::PreviousShutdownCompactionPending => {
            "previous_shutdown_compaction_pending"
        }
        SessionOperationFailureKind::Internal => "internal",
    }
}
