#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryActionKind {
    ReadAgain,
    RetrySameEffect,
    UseObservedResult,
    CancelIfSafe,
    KeepForManualResolution,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryResultClassification {
    Pending,
    Succeeded,
    ConfirmedNoEffect,
    Ambiguous,
    CancelledBeforeEffect,
    Unchanged,
}
