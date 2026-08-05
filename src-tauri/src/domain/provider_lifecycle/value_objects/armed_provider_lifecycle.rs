use super::{ProviderKind, ProviderLifecycleScope, ProviderLifecycleSlotId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ArmedProviderLifecycle {
    slot_id: ProviderLifecycleSlotId,
    binding_id: String,
    capability: String,
    provider: ProviderKind,
    scope: ProviderLifecycleScope,
}

impl ArmedProviderLifecycle {
    pub(crate) fn new(
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

    pub(crate) fn slot_id(&self) -> &ProviderLifecycleSlotId {
        &self.slot_id
    }

    pub(crate) fn binding_id(&self) -> &str {
        &self.binding_id
    }

    pub(crate) fn capability(&self) -> &str {
        &self.capability
    }

    pub(crate) fn provider(&self) -> ProviderKind {
        self.provider
    }

    pub(crate) fn scope(&self) -> &ProviderLifecycleScope {
        &self.scope
    }
}
