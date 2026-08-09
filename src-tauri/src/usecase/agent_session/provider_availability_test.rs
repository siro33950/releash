use std::sync::Arc;

use super::{
    ProviderAgentSessionProviderDto, ProviderAvailabilityQueryService,
    ProviderAvailabilityReadUsecase,
};

struct FakeProviderAvailabilityQuery;

impl ProviderAvailabilityQueryService for FakeProviderAvailabilityQuery {
    fn available_providers(&self) -> Vec<ProviderAgentSessionProviderDto> {
        vec![ProviderAgentSessionProviderDto::Codex]
    }
}

#[test]
fn test_provider_availability_利用可能なproviderだけを候補として返しdefaultを持たない() {
    let read = ProviderAvailabilityReadUsecase::new(Arc::new(FakeProviderAvailabilityQuery));

    assert_eq!(
        read.list_available_providers(),
        vec![ProviderAgentSessionProviderDto::Codex]
    );
}
