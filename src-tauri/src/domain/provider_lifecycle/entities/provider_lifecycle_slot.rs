use super::ProviderLifecycleBinding;
use crate::domain::provider_lifecycle::{
    ProviderKind, ProviderLifecycleCapabilityHash, ProviderLifecycleEvent,
    ProviderLifecycleOutcome, ProviderLifecycleRejection, ProviderLifecycleScope,
    ProviderLifecycleSignal, ProviderLifecycleSlotId, ProviderLifecycleUnavailableObservation,
    ScopedProviderLifecycleEvent,
};

#[derive(Debug, Clone)]
pub(crate) struct ProviderLifecycleSlot {
    id: ProviderLifecycleSlotId,
    current: Option<ActiveBinding>,
}

#[derive(Debug, Clone)]
struct ActiveBinding {
    capability_hash: ProviderLifecycleCapabilityHash,
    binding: ProviderLifecycleBinding,
}

impl ProviderLifecycleSlot {
    pub(crate) fn new(id: ProviderLifecycleSlotId) -> Self {
        Self { id, current: None }
    }

    pub(crate) fn arm(
        &mut self,
        binding: ProviderLifecycleBinding,
        capability_hash: ProviderLifecycleCapabilityHash,
    ) -> Vec<ScopedProviderLifecycleEvent> {
        let mut facts = Vec::new();
        if let Some(mut current) = self.current.take() {
            let scope = current.binding.scope().clone();
            if let ProviderLifecycleOutcome::Applied(events) = current.binding.expire() {
                facts.extend(scoped(scope, events));
            }
        }
        facts.push(ScopedProviderLifecycleEvent::new(
            binding.scope().clone(),
            binding.armed_event(&self.id),
        ));
        self.current = Some(ActiveBinding {
            capability_hash,
            binding,
        });
        facts
    }

    pub(crate) fn receive(
        &mut self,
        capability_hash: &ProviderLifecycleCapabilityHash,
        signal: ProviderLifecycleSignal,
    ) -> ProviderLifecycleOutcome {
        let Some(current) = self.current.as_mut() else {
            return ProviderLifecycleOutcome::Rejected(
                ProviderLifecycleRejection::BindingNotActive,
            );
        };
        if current.binding.binding_id() != signal.binding_id() {
            return ProviderLifecycleOutcome::Rejected(ProviderLifecycleRejection::BindingExpired);
        }
        if !current.capability_hash.matches(capability_hash) {
            return ProviderLifecycleOutcome::Rejected(
                ProviderLifecycleRejection::InvalidCapability,
            );
        }
        current.binding.observe(signal)
    }

    pub(crate) fn report_unavailable(
        &mut self,
        capability_hash: &ProviderLifecycleCapabilityHash,
        observation: ProviderLifecycleUnavailableObservation,
    ) -> ProviderLifecycleOutcome {
        let Some(current) = self.current.as_mut() else {
            return ProviderLifecycleOutcome::Rejected(
                ProviderLifecycleRejection::BindingNotActive,
            );
        };
        if current.binding.binding_id() != observation.binding_id() {
            return ProviderLifecycleOutcome::Rejected(ProviderLifecycleRejection::BindingExpired);
        }
        if !current.capability_hash.matches(capability_hash) {
            return ProviderLifecycleOutcome::Rejected(
                ProviderLifecycleRejection::InvalidCapability,
            );
        }
        current.binding.mark_unavailable(observation)
    }

    pub(crate) fn release(&mut self, binding_id: &str) -> ProviderLifecycleOutcome {
        let Some(current) = self.current.as_mut() else {
            return ProviderLifecycleOutcome::Duplicate;
        };
        if current.binding.binding_id() != binding_id {
            return ProviderLifecycleOutcome::Rejected(ProviderLifecycleRejection::BindingExpired);
        }
        let outcome = current.binding.expire();
        self.current = None;
        outcome
    }

    pub(crate) fn release_scope(
        &mut self,
        scope: &ProviderLifecycleScope,
    ) -> Option<ProviderLifecycleOutcome> {
        let binding_id = self
            .current
            .as_ref()
            .filter(|current| current.binding.scope() == scope)
            .map(|current| current.binding.binding_id().to_string())?;
        Some(self.release(&binding_id))
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.current.is_none()
    }

    pub(crate) fn current_scope(&self) -> Option<&ProviderLifecycleScope> {
        self.current.as_ref().map(|current| current.binding.scope())
    }

    pub(crate) fn current_provider(&self) -> Option<ProviderKind> {
        self.current
            .as_ref()
            .map(|current| current.binding.provider())
    }
}

fn scoped(
    scope: ProviderLifecycleScope,
    events: Vec<ProviderLifecycleEvent>,
) -> Vec<ScopedProviderLifecycleEvent> {
    events
        .into_iter()
        .map(|event| ScopedProviderLifecycleEvent::new(scope.clone(), event))
        .collect()
}
