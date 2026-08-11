use super::LocalProviderHookHealthFailureQuery;
use crate::domain::provider_lifecycle::{ProviderKind, ProviderLifecycleUnavailableReason};
use crate::usecase::provider_lifecycle::ProviderHookHealthFailureQuery;

#[tokio::test]
async fn test_provider_hook_health_failure_query_launch_markerだけをboundedに変換する() {
    let directory = tempfile::tempdir().unwrap();
    for (agent, launch, provider) in [
        ("agent-a", "launch-a", "claude"),
        ("agent-b", "launch-b", "codex"),
    ] {
        let marker = directory
            .path()
            .join("provider-launches")
            .join(agent)
            .join(launch)
            .join("hook-health.json");
        crate::infrastructure::provider_lifecycle::write_provider_hook_local_api_failure(
            directory.path(),
            &marker,
            provider,
            launch,
        )
        .unwrap();
    }
    let invalid = directory
        .path()
        .join("provider-launches/agent-c/launch-c/hook-health.json");
    std::fs::create_dir_all(invalid.parent().unwrap()).unwrap();
    std::fs::write(&invalid, br#"{"provider":"claude","secret":"ignored"}"#).unwrap();
    let query = LocalProviderHookHealthFailureQuery::new(directory.path().to_path_buf());

    let observations = query.list(2).await.unwrap();

    assert_eq!(observations.len(), 2);
    assert_eq!(observations[0].provider, ProviderKind::Claude);
    assert_eq!(observations[0].launch_id, "launch-a");
    assert_eq!(
        observations[0].reason,
        ProviderLifecycleUnavailableReason::LocalApiUnavailable
    );
    assert_eq!(observations[1].provider, ProviderKind::Codex);
    assert_eq!(observations[1].launch_id, "launch-b");
}
