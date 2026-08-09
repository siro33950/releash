use crate::domain::provider_lifecycle::ProviderKind;

pub(crate) trait ProviderAvailabilityGateway: Send + Sync {
    fn is_available(&self, provider: ProviderKind) -> bool;
}
