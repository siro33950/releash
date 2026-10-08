use super::ProviderLifecycleEvent;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProviderLifecycleOutcome {
    Applied(Vec<ProviderLifecycleEvent>),
    Duplicate,
    Rejected(ProviderLifecycleRejection),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderLifecycleRejection {
    BindingNotActive,
    InvalidCapability,
    BindingMismatch,
    ProviderMismatch,
    ScopeMismatch,
    BindingExpired,
    #[cfg_attr(
        not(any(test, feature = "test-support")),
        expect(
            dead_code,
            reason = "Retained rejection reason for the presenter contract"
        )
    )]
    SessionAlreadyAssociated,
    SessionNotAssociated,
    ProviderSessionMismatch,
    TranscriptMismatch,
}
