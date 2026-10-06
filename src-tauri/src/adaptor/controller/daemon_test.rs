use super::*;
use adaptor::gateway::local_event_store::store::LocalEventStoreOpenError as E;
use domain::daemon::StartupFailureKind as K;

#[test]
fn b071_store_open_failures_map_to_the_closed_safe_startup_vocabulary() {
    for (error, expected) in [
        (E::WriterLockHeld, K::StoreInUse),
        (
            E::StorageUnavailable(domain::failure::TechnicalFailure {
                nature: domain::failure::TechnicalFailureNature::Other,
                message: "disk full".into(),
            }),
            K::StorageUnavailable(domain::failure::TechnicalFailureNature::Other),
        ),
        (
            E::StorageUnavailable(domain::failure::TechnicalFailure {
                nature: domain::failure::TechnicalFailureNature::Transient,
                message: "source".into(),
            }),
            K::StorageUnavailable(domain::failure::TechnicalFailureNature::Transient),
        ),
        (
            E::StorageUnavailable(domain::failure::TechnicalFailure {
                nature: domain::failure::TechnicalFailureNature::TimedOut,
                message: "source".into(),
            }),
            K::StorageUnavailable(domain::failure::TechnicalFailureNature::TimedOut),
        ),
        (E::UnsupportedRuntime, K::UnsupportedRuntime),
        (E::UnsupportedStoreVersion, K::UnsupportedStoreVersion),
        (E::InitializationStateInvalid, K::InitializationStateInvalid),
        (E::StoreValidationFailed, K::StoreValidationFailed),
        (E::SchemaEvolutionFailed, K::SchemaEvolutionFailed),
    ] {
        assert_eq!(K::from(error), expected);
    }
}
