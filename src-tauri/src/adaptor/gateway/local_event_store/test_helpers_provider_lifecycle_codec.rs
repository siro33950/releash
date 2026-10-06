use crate::domain::provider_lifecycle::ProviderLifecycleScope;
pub fn scope() -> ProviderLifecycleScope {
    ProviderLifecycleScope::new("agent-session-1").unwrap()
}
