use std::sync::Arc;

use crate::domain::agent_session::ProviderAvailabilityGateway;
use crate::domain::provider_lifecycle::ProviderKind;
use crate::usecase::agent_session::{
    ProviderAgentSessionProviderDto, ProviderAvailabilityQueryService,
};

pub(crate) struct LocalProviderAvailabilityQueryService {
    availability: Arc<dyn ProviderAvailabilityGateway>,
}

impl LocalProviderAvailabilityQueryService {
    pub(crate) fn new(availability: Arc<dyn ProviderAvailabilityGateway>) -> Self {
        Self { availability }
    }
}

impl ProviderAvailabilityQueryService for LocalProviderAvailabilityQueryService {
    fn available_providers(&self) -> Vec<ProviderAgentSessionProviderDto> {
        ProviderKind::supported()
            .iter()
            .copied()
            .filter(|provider| self.availability.is_available(*provider))
            .map(|provider| match provider {
                ProviderKind::Claude => ProviderAgentSessionProviderDto::Claude,
                ProviderKind::Codex => ProviderAgentSessionProviderDto::Codex,
            })
            .collect()
    }
}
