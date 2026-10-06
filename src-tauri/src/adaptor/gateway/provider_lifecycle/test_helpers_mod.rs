use super::ProviderLaunchContext;
use crate::domain::provider_lifecycle::{ProviderLifecycleScope, ProviderLifecycleSlotId};
pub fn scope() -> ProviderLifecycleScope {
    ProviderLifecycleScope::new("agent-1").unwrap()
}
pub fn context() -> ProviderLaunchContext {
    ProviderLaunchContext::new(slot_id(), "binding-1", "capability-1", scope()).unwrap()
}
pub fn slot_id() -> ProviderLifecycleSlotId {
    ProviderLifecycleSlotId::new("slot-1").unwrap()
}
