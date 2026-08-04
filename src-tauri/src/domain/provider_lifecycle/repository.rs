use super::ScopedProviderLifecycleEvent;

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
}
