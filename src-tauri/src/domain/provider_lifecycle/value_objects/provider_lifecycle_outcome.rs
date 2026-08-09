use super::ProviderLifecycleEvent;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProviderLifecycleOutcome {
    Applied(Vec<ProviderLifecycleEvent>),
    Duplicate,
    Rejected(ProviderLifecycleRejection),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProviderLifecycleRejection {
    BindingNotActive,
    InvalidCapability,
    BindingMismatch,
    ProviderMismatch,
    ScopeMismatch,
    BindingExpired,
    SessionAlreadyAssociated,
    SessionNotAssociated,
    ProviderSessionMismatch,
    TranscriptMismatch,
}
