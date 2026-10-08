use super::{
    ProviderHookHealth, ProviderKind, ProviderLifecycleScope, ScopedProviderLifecycleEvent,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderLifecycleRepositoryError {
    Conflict,
    Store(crate::domain::failure::StorageFailure),
    InvalidInput,
    StorageUnavailable,
    Corrupt,
}

#[async_trait::async_trait]
pub trait ProviderLifecycleEventRepository: Send + Sync {
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
pub enum ProviderHookHealthRepositoryError {
    Store(crate::domain::failure::StorageFailure),
    InvalidInput,
    Conflict,
    StorageUnavailable,
    Corrupt,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionedProviderHookHealth {
    health: ProviderHookHealth,
    revision: u64,
}

impl VersionedProviderHookHealth {
    pub fn restored(health: ProviderHookHealth, revision: u64) -> Self {
        Self { health, revision }
    }

    pub fn health(&self) -> &ProviderHookHealth {
        &self.health
    }

    pub fn health_mut(&mut self) -> &mut ProviderHookHealth {
        &mut self.health
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn into_health(self) -> ProviderHookHealth {
        self.health
    }
}

#[async_trait::async_trait]
pub trait ProviderHookHealthRepository: Send + Sync {
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
