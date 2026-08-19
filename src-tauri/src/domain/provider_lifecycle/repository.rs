use super::{
    ProviderHookHealth, ProviderKind, ProviderLifecycleScope, ScopedProviderLifecycleEvent,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProviderLifecycleRepositoryError {
    InvalidInput,
    StorageUnavailable,
    Corrupt,
}

#[async_trait::async_trait]
pub(crate) trait ProviderLifecycleEventRepository: Send + Sync {
    async fn append(
        &self,
        events: Vec<ScopedProviderLifecycleEvent>,
    ) -> Result<(), ProviderLifecycleRepositoryError>;

    async fn load_scope(
        &self,
        scope: &ProviderLifecycleScope,
    ) -> Result<Vec<ScopedProviderLifecycleEvent>, ProviderLifecycleRepositoryError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProviderHookHealthRepositoryError {
    InvalidInput,
    Conflict,
    StorageUnavailable,
    Corrupt,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VersionedProviderHookHealth {
    health: ProviderHookHealth,
    revision: u64,
}

impl VersionedProviderHookHealth {
    pub(crate) fn restored(health: ProviderHookHealth, revision: u64) -> Self {
        Self { health, revision }
    }

    pub(crate) fn health(&self) -> &ProviderHookHealth {
        &self.health
    }

    pub(crate) fn health_mut(&mut self) -> &mut ProviderHookHealth {
        &mut self.health
    }

    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }

    pub(crate) fn into_health(self) -> ProviderHookHealth {
        self.health
    }
}

#[async_trait::async_trait]
pub(crate) trait ProviderHookHealthRepository: Send + Sync {
    async fn load(
        &self,
        provider: ProviderKind,
    ) -> Result<VersionedProviderHookHealth, ProviderHookHealthRepositoryError>;

    async fn save(
        &self,
        health: VersionedProviderHookHealth,
        caller_request_id: &str,
    ) -> Result<VersionedProviderHookHealth, ProviderHookHealthRepositoryError>;
}
