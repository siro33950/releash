use std::sync::Arc;

use super::ProviderAgentSessionProviderDto;

pub(crate) trait ProviderAvailabilityQueryService: Send + Sync {
    fn available_providers(&self) -> Vec<ProviderAgentSessionProviderDto>;
}

pub(crate) struct ProviderAvailabilityReadUsecase {
    query: Arc<dyn ProviderAvailabilityQueryService>,
}

impl ProviderAvailabilityReadUsecase {
    pub(crate) fn new(query: Arc<dyn ProviderAvailabilityQueryService>) -> Self {
        Self { query }
    }

    pub(crate) fn list_available_providers(&self) -> Vec<ProviderAgentSessionProviderDto> {
        self.query.available_providers()
    }
}
