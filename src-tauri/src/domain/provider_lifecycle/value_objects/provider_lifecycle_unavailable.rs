#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderLifecycleUnavailableReason {
    SessionStartDeadlineExceeded,
    CodexHookDeliveryUnconfirmed,
    ProviderHookConfigurationRejected,
    LocalApiUnavailable,
}
