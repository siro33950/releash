use super::{ProviderKind, ProviderLifecycleScope, ProviderLifecycleSlotId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArmedProviderLifecycle {
    slot_id: ProviderLifecycleSlotId,
    binding_id: String,
    capability: String,
    provider: ProviderKind,
    scope: ProviderLifecycleScope,
}

impl ArmedProviderLifecycle {
    pub fn new(
        slot_id: ProviderLifecycleSlotId,
        binding_id: String,
        capability: String,
        provider: ProviderKind,
        scope: ProviderLifecycleScope,
    ) -> Self {
        Self {
            slot_id,
            binding_id,
            capability,
            provider,
            scope,
        }
    }

    pub fn slot_id(&self) -> &ProviderLifecycleSlotId {
        &self.slot_id
    }

    pub fn binding_id(&self) -> &str {
        &self.binding_id
    }

    pub fn capability(&self) -> &str {
        &self.capability
    }

    pub fn provider(&self) -> ProviderKind {
        self.provider
    }

    pub fn scope(&self) -> &ProviderLifecycleScope {
        &self.scope
    }
}
