use super::{ProviderLifecycleEvent, ProviderLifecycleScope};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopedProviderLifecycleEvent {
    scope: ProviderLifecycleScope,
    event: ProviderLifecycleEvent,
}

impl ScopedProviderLifecycleEvent {
    pub fn new(scope: ProviderLifecycleScope, event: ProviderLifecycleEvent) -> Self {
        Self { scope, event }
    }

    pub fn into_parts(self) -> (ProviderLifecycleScope, ProviderLifecycleEvent) {
        (self.scope, self.event)
    }
}
