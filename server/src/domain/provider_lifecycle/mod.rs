pub(crate) mod entities;
pub(crate) mod error;
pub(crate) mod gateway;
pub(crate) mod repository;
pub(crate) mod value_objects;

pub(crate) use entities::{
    ProviderHookHealth, ProviderHookHealthEvent, ProviderHookHealthOutcome,
    ProviderLifecycleBinding, ProviderLifecycleSlot,
};
pub(crate) use error::ProviderLifecycleInputError;
#[cfg(any(test, feature = "test-support"))]
pub(crate) use error::ProviderLifecycleReplayError;
pub(crate) use gateway::ProviderLifecycleCredentialGateway;
pub(crate) use repository::{
    ProviderHookHealthRepository, ProviderHookHealthRepositoryError,
    ProviderLifecycleEventRepository, ProviderLifecycleRepositoryError,
    VersionedProviderHookHealth,
};
pub(crate) use value_objects::{
    ArmedProviderLifecycle, IssuedProviderLifecycleCredential, ProviderKind,
    ProviderLifecycleCapabilityHash, ProviderLifecycleEvent, ProviderLifecycleIngressResult,
    ProviderLifecycleOutcome, ProviderLifecycleRejection, ProviderLifecycleScope,
    ProviderLifecycleSignal, ProviderLifecycleSignalKind, ProviderLifecycleSlotId,
    ProviderLifecycleUnavailableReason, ScopedProviderLifecycleEvent,
};

mod payload;
pub(crate) use payload::{
    ProviderPayloadError, ProviderPayloadInterpretation, ProviderPayloadInterpreter,
};
