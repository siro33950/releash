use crate::adaptor::protocol::application_lifecycle_v1::{
    ApplicationStartupOutcomeDtoV1, StartupFailureActionDtoV1, StartupFailureKindDtoV1,
};

pub(crate) fn application_startup_outcome(
    value: crate::usecase::application_startup::ApplicationStartupOutcome,
) -> ApplicationStartupOutcomeDtoV1 {
    use crate::usecase::application_startup::{
        ApplicationStartupOutcome as O, StartupFailureKind as K,
    };
    match value {
        O::Ready => ApplicationStartupOutcomeDtoV1::Ready,
        O::Failed(failure) => ApplicationStartupOutcomeDtoV1::Failed {
            kind: match failure.kind {
                K::StoreInUse => StartupFailureKindDtoV1::StoreInUse,
                K::StorageUnavailable => StartupFailureKindDtoV1::StorageUnavailable,
                K::UnsupportedRuntime => StartupFailureKindDtoV1::UnsupportedRuntime,
                K::UnsupportedStoreVersion => StartupFailureKindDtoV1::UnsupportedStoreVersion,
                K::InitializationStateInvalid => {
                    StartupFailureKindDtoV1::InitializationStateInvalid
                }
                K::StoreValidationFailed => StartupFailureKindDtoV1::StoreValidationFailed,
                K::SchemaEvolutionFailed => StartupFailureKindDtoV1::SchemaEvolutionFailed,
            },
            safe_description: failure.safe_description.to_string(),
            correlation_id: failure.correlation_id,
            retry_on_next_launch: failure.retry_on_next_launch,
            actions: [StartupFailureActionDtoV1::Quit],
        },
    }
}
