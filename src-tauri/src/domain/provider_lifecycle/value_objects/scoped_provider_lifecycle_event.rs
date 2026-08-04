use super::{ProviderLifecycleEvent, ProviderLifecycleScope};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScopedProviderLifecycleEvent {
    scope: ProviderLifecycleScope,
    event: ProviderLifecycleEvent,
}

impl ScopedProviderLifecycleEvent {
    pub(crate) fn new(scope: ProviderLifecycleScope, event: ProviderLifecycleEvent) -> Self {
        Self { scope, event }
    }

    pub(crate) fn into_parts(self) -> (ProviderLifecycleScope, ProviderLifecycleEvent) {
        (self.scope, self.event)
    }
}
