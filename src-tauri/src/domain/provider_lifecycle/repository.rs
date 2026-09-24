use super::{
    ProviderHookHealth, ProviderKind, ProviderLifecycleScope, ScopedProviderLifecycleEvent,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProviderLifecycleRepositoryError {
    Store(crate::domain::failure::FailureKind),
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
    Store(crate::domain::failure::FailureKind),
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

impl crate::domain::failure::ClassifiedFailure for ProviderLifecycleRepositoryError {
    fn failure_kind(&self) -> crate::domain::failure::FailureKind {
        use crate::domain::failure::FailureKind;
        match self {
            Self::Store(kind) => *kind,
            Self::InvalidInput => FailureKind::InvalidInput,
            Self::StorageUnavailable => FailureKind::Temporary,
            Self::Corrupt => FailureKind::Corrupt,
        }
    }
}

impl From<crate::domain::local_event::LocalEventQueryError> for ProviderLifecycleRepositoryError {
    fn from(error: crate::domain::local_event::LocalEventQueryError) -> Self {
        use crate::domain::failure::ClassifiedFailure;
        match error.failure_kind() {
            crate::domain::failure::FailureKind::Temporary => Self::StorageUnavailable,
            crate::domain::failure::FailureKind::InvalidInput => Self::InvalidInput,
            crate::domain::failure::FailureKind::Corrupt => Self::Corrupt,
            kind => Self::Store(kind),
        }
    }
}

impl crate::domain::failure::ClassifiedFailure for ProviderHookHealthRepositoryError {
    fn failure_kind(&self) -> crate::domain::failure::FailureKind {
        use crate::domain::failure::FailureKind;
        match self {
            Self::Store(kind) => *kind,
            Self::InvalidInput => FailureKind::InvalidInput,
            Self::Conflict => FailureKind::RestartRequired,
            Self::StorageUnavailable => FailureKind::Temporary,
            Self::Corrupt => FailureKind::Corrupt,
        }
    }
}

impl From<crate::domain::local_event::LocalEventQueryError> for ProviderHookHealthRepositoryError {
    fn from(error: crate::domain::local_event::LocalEventQueryError) -> Self {
        use crate::domain::failure::ClassifiedFailure;
        match error.failure_kind() {
            crate::domain::failure::FailureKind::Temporary => Self::StorageUnavailable,
            crate::domain::failure::FailureKind::InvalidInput => Self::InvalidInput,
            crate::domain::failure::FailureKind::Corrupt => Self::Corrupt,
            kind => Self::Store(kind),
        }
    }
}

#[cfg(test)]
#[path = "repository_test.rs"]
mod repository_tests;
