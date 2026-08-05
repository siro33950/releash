mod provider_lifecycle_binding;
mod provider_lifecycle_slot;

pub(crate) use provider_lifecycle_binding::ProviderLifecycleBinding;
pub(crate) use provider_lifecycle_slot::ProviderLifecycleSlot;

#[cfg(test)]
#[path = "provider_lifecycle_binding_test.rs"]
mod provider_lifecycle_binding_tests;

#[cfg(test)]
#[path = "provider_lifecycle_slot_test.rs"]
mod provider_lifecycle_slot_tests;
