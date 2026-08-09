use std::fs;

use tempfile::tempdir;

use super::LocalProviderAgentLaunchGateway;
use crate::domain::agent_session::{ProviderAgentLaunchGateway, ProviderSessionLaunch};
use crate::domain::provider_lifecycle::{
    ArmedProviderLifecycle, ProviderKind, ProviderLifecycleScope, ProviderLifecycleSlotId,
    ProviderLifecycleUnavailableReason,
};

fn armed(provider: ProviderKind) -> ArmedProviderLifecycle {
    ArmedProviderLifecycle::new(
        ProviderLifecycleSlotId::new("slot-1").unwrap(),
        "binding-1".to_string(),
        "capability-1".to_string(),
        provider,
        ProviderLifecycleScope::new("agent-1").unwrap(),
    )
}

#[test]
fn test_provider_launch_gateway_claudeのpluginをlaunch単位で生成しcleanupする() {
    let data_dir = tempdir().unwrap();
    let gateway = LocalProviderAgentLaunchGateway::new(
        data_dir.path().to_path_buf(),
        "/opt/bin/claude".to_string(),
        "/opt/bin/codex".to_string(),
        "releash".to_string(),
    );

    let prepared = gateway
        .prepare(&armed(ProviderKind::Claude), ProviderSessionLaunch::New)
        .unwrap();

    assert_eq!(prepared.process().executable(), "/opt/bin/claude");
    assert_eq!(prepared.initial_hook_warning(), None);
    assert_eq!(
        prepared.process().arguments(),
        &[
            "--plugin-dir".to_string(),
            prepared
                .resource_directory()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
        ]
    );
    let resource_directory = prepared.resource_directory().unwrap();
    assert!(resource_directory.starts_with(data_dir.path()));
    assert!(resource_directory
        .join(".claude-plugin/plugin.json")
        .is_file());
    let hook = fs::read_to_string(resource_directory.join("hooks/hooks.json")).unwrap();
    assert!(hook.contains("releash hook receive --provider claude"));
    assert!(prepared
        .process()
        .environment()
        .iter()
        .any(
            |(key, value)| key == "RELEASH_PROVIDER_LIFECYCLE_CAPABILITY"
                && value == "capability-1"
        ));

    gateway.cleanup("agent-1").unwrap();
    assert!(!resource_directory.exists());
}

#[test]
fn test_provider_launch_gateway_codexのresumeを同じroot_process契約で生成する() {
    let data_dir = tempdir().unwrap();
    let gateway = LocalProviderAgentLaunchGateway::new(
        data_dir.path().to_path_buf(),
        "/opt/bin/claude".to_string(),
        "/opt/bin/codex".to_string(),
        "releash-dev".to_string(),
    );

    let prepared = gateway
        .prepare(
            &armed(ProviderKind::Codex),
            ProviderSessionLaunch::resume("codex-session-1").unwrap(),
        )
        .unwrap();

    assert_eq!(prepared.process().executable(), "/opt/bin/codex");
    assert_eq!(
        prepared.initial_hook_warning(),
        Some(ProviderLifecycleUnavailableReason::CodexHookDeliveryUnconfirmed)
    );
    assert!(prepared.resource_directory().unwrap().is_dir());
    assert!(prepared
        .process()
        .arguments()
        .windows(2)
        .any(|pair| pair == ["resume", "codex-session-1"]));
    assert!(prepared
        .process()
        .arguments()
        .iter()
        .any(|argument| { argument.contains("releash-dev hook receive --provider codex") }));
}

#[test]
fn test_provider_launch_gateway_両providerへhook実行環境を渡す() {
    let data_dir = tempdir().unwrap();
    let gateway = LocalProviderAgentLaunchGateway::new(
        data_dir.path().to_path_buf(),
        "/opt/bin/claude".to_string(),
        "/opt/bin/codex".to_string(),
        "releash".to_string(),
    );

    for provider in [ProviderKind::Claude, ProviderKind::Codex] {
        let prepared = gateway
            .prepare(&armed(provider), ProviderSessionLaunch::New)
            .unwrap();
        let launch_directory = prepared.resource_directory().unwrap();
        let marker = prepared
            .process()
            .environment()
            .iter()
            .find(|(key, _)| key == "RELEASH_PROVIDER_LIFECYCLE_HEALTH_FILE")
            .map(|(_, value)| value)
            .unwrap();
        assert_eq!(
            std::path::Path::new(marker),
            launch_directory.join("hook-health.json")
        );
        assert!(prepared
            .process()
            .environment()
            .iter()
            .any(|(key, value)| key == "RELEASH_SESSION_ID" && value == "agent-1"));
        assert!(prepared.process().environment().iter().any(|(key, value)| {
            key == "RELEASH_DATA_DIR" && std::path::Path::new(value) == data_dir.path()
        }));
        assert!(launch_directory.is_dir());
    }
}
