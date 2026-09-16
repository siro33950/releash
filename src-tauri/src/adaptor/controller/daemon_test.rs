use super::*;
use adaptor::gateway::local_event_store::store::LocalEventStoreOpenError as E;
use usecase::application_startup::StartupFailureKind as K;

#[test]
fn test_performance_fixture_replaces_both_provider_executables_without_affecting_defaults() {
    assert_eq!(
        select_provider_agent_executables(
            "claude".to_string(),
            "codex".to_string(),
            Some("/tmp/provider-fixture".to_string()),
        ),
        (
            "/tmp/provider-fixture".to_string(),
            "/tmp/provider-fixture".to_string(),
        )
    );
    assert_eq!(
        select_provider_agent_executables("claude".to_string(), "codex".to_string(), None,),
        ("claude".to_string(), "codex".to_string())
    );
}

#[test]
fn b071_store_open_failures_map_to_the_closed_safe_startup_vocabulary() {
    for (error, expected) in [
        (E::WriterLockHeld, K::StoreInUse),
        (E::StorageUnavailable, K::StorageUnavailable),
        (E::UnsupportedRuntime, K::UnsupportedRuntime),
        (E::UnsupportedStoreVersion, K::UnsupportedStoreVersion),
        (E::InitializationStateInvalid, K::InitializationStateInvalid),
        (E::StoreValidationFailed, K::StoreValidationFailed),
        (E::SchemaEvolutionFailed, K::SchemaEvolutionFailed),
    ] {
        assert_eq!(classify_startup_failure(error), expected);
    }
}
