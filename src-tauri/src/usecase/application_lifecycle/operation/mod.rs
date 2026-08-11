//! Application lifecycle operation identity contracts backed by the
//! permanent local event authority.
//!
//! This module owns application quit caller journaling, durable operation
//! binding, and shutdown-target reconciliation semantics.

pub(crate) mod binding;
pub(crate) mod caller_journal;
pub(crate) mod identity;
pub(crate) mod ports;
pub(crate) mod record;
pub(crate) mod recovery;

/// Shared principal for operations issued by authenticated local transports.
pub(crate) const LOCAL_INSTALLATION_OPERATION_PRINCIPAL: &str = "local-app";

pub(crate) use caller_journal::{
    BoundCallerOperation, CallerAttemptJournal, CallerJournalError, PendingCallerAttempt,
    PendingCallerAttemptPage,
};
pub(crate) use identity::{constant_time_eq_32, validate_operation_identity};
pub(crate) use ports::{OperationBindingAuthority, RecoveryResultCanonicalizer};
pub(crate) use recovery::{
    decode_recovery_completed_result, derive_recovery_action_id, RecoveryActionError,
    RecoveryActionIdentity, RecoveryActionOutcome, RecoveryActionRejection,
    RecoveryActionResultOutcome, RecoveryActionStatus,
};
