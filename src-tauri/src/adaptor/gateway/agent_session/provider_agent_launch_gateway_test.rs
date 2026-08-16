use std::fs;

use tempfile::tempdir;

use super::LocalProviderAgentLaunchGateway;
use crate::domain::agent_session::{
    aggregates::ResolvedProviderExecutable, ProviderAgentLaunchGateway, ProviderLaunchOptions,
    ProviderSessionLaunch,
};
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
    let gateway =
        LocalProviderAgentLaunchGateway::new(data_dir.path().to_path_buf(), "releash".to_string());

    let prepared = gateway
        .prepare(
            &armed(ProviderKind::Claude),
            ResolvedProviderExecutable::new("/opt/bin/claude".into()).unwrap(),
            ProviderSessionLaunch::New,
            data_dir.path().to_str().unwrap(),
        )
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
        "releash-dev".to_string(),
    );

    let prepared = gateway
        .prepare(
            &armed(ProviderKind::Codex),
            ResolvedProviderExecutable::new("/opt/bin/codex".into()).unwrap(),
            ProviderSessionLaunch::resume("codex-session-1").unwrap(),
            data_dir.path().to_str().unwrap(),
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
fn test_provider_launch_gateway_modelとpermissionをresume引数より前にcli引数へ注入する() {
    let data_dir = tempdir().unwrap();
    let gateway =
        LocalProviderAgentLaunchGateway::new(data_dir.path().to_path_buf(), "releash".to_string());

    for (provider, executable, permission_flag, resume_flag) in [
        (
            ProviderKind::Claude,
            "/opt/bin/claude",
            "--permission-mode",
            "--resume",
        ),
        (ProviderKind::Codex, "/opt/bin/codex", "--sandbox", "resume"),
    ] {
        let launch = ProviderSessionLaunch::resume("session-1")
            .unwrap()
            .with_options(ProviderLaunchOptions::new(
                Some("model-x".to_string()),
                Some("permission-y".to_string()),
            ));
        let prepared = gateway
            .prepare(
                &armed(provider),
                ResolvedProviderExecutable::new(executable.into()).unwrap(),
                launch,
                data_dir.path().to_str().unwrap(),
            )
            .unwrap();

        let arguments = prepared.process().arguments();
        assert!(
            arguments
                .windows(2)
                .any(|pair| pair == ["--model", "model-x"]),
            "{provider:?}: model must be injected verbatim: {arguments:?}"
        );
        assert!(
            arguments
                .windows(2)
                .any(|pair| pair == [permission_flag, "permission-y"]),
            "{provider:?}: permission must be injected verbatim: {arguments:?}"
        );
        let model_index = arguments
            .iter()
            .position(|argument| argument == "--model")
            .unwrap();
        let permission_index = arguments
            .iter()
            .position(|argument| argument == permission_flag)
            .unwrap();
        let resume_index = arguments
            .iter()
            .position(|argument| argument == resume_flag)
            .unwrap();
        assert!(model_index < resume_index);
        assert!(permission_index < resume_index);
    }
}

#[test]
fn test_provider_launch_gateway_両providerへhook実行環境を渡す() {
    let data_dir = tempdir().unwrap();
    let gateway =
        LocalProviderAgentLaunchGateway::new(data_dir.path().to_path_buf(), "releash".to_string());

    for provider in [ProviderKind::Claude, ProviderKind::Codex] {
        let prepared = gateway
            .prepare(
                &armed(provider),
                ResolvedProviderExecutable::new(
                    match provider {
                        ProviderKind::Claude => "/opt/bin/claude",
                        ProviderKind::Codex => "/opt/bin/codex",
                    }
                    .into(),
                )
                .unwrap(),
                ProviderSessionLaunch::New,
                data_dir.path().to_str().unwrap(),
            )
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

#[test]
fn test_provider_launch_gateway_解決済みbase_branchを両providerへ渡す() {
    let data_dir = tempdir().unwrap();
    let (repo_dir, repo) = crate::test_support::git::create_test_repo();
    crate::test_support::git::create_initial_commit(&repo);
    let repo_path = repo_dir.path().to_str().unwrap();
    let base_branch = repo.head().unwrap().shorthand().unwrap().to_string();
    let base_commit = repo.head().unwrap().peel_to_commit().unwrap();
    repo.branch("feature", &base_commit, false).unwrap();
    repo.set_head("refs/heads/feature").unwrap();
    repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
        .unwrap();
    crate::adaptor::gateway::repository::git_config::set_branch_base_override(
        repo_path,
        "feature",
        Some(&base_branch),
    )
    .unwrap();
    let gateway =
        LocalProviderAgentLaunchGateway::new(data_dir.path().to_path_buf(), "releash".to_string());

    for provider in [ProviderKind::Claude, ProviderKind::Codex] {
        let prepared = gateway
            .prepare(
                &armed(provider),
                ResolvedProviderExecutable::new(
                    match provider {
                        ProviderKind::Claude => "/opt/bin/claude",
                        ProviderKind::Codex => "/opt/bin/codex",
                    }
                    .into(),
                )
                .unwrap(),
                ProviderSessionLaunch::New,
                repo_path,
            )
            .unwrap();

        assert!(prepared
            .process()
            .environment()
            .iter()
            .any(|(key, value)| { key == "RELEASH_BASE_BRANCH" && value == &base_branch }));
    }
}

#[test]
fn test_provider_launch_gateway_base_branch未解決なら環境変数を渡さない() {
    let data_dir = tempdir().unwrap();
    let (repo_dir, repo) = crate::test_support::git::create_test_repo();
    let head = crate::test_support::git::create_initial_commit(&repo);
    repo.set_head_detached(head).unwrap();
    let gateway =
        LocalProviderAgentLaunchGateway::new(data_dir.path().to_path_buf(), "releash".to_string());

    let prepared = gateway
        .prepare(
            &armed(ProviderKind::Claude),
            ResolvedProviderExecutable::new("/opt/bin/claude".into()).unwrap(),
            ProviderSessionLaunch::New,
            repo_dir.path().to_str().unwrap(),
        )
        .unwrap();

    assert!(prepared
        .process()
        .environment()
        .iter()
        .all(|(key, _)| key != "RELEASH_BASE_BRANCH"));
}

#[cfg(unix)]
#[test]
fn test_provider_launch_gateway_non_utf8実行pathをterminal_processまで保持する() {
    use std::os::unix::ffi::OsStringExt;

    let data_dir = tempdir().unwrap();
    let gateway =
        LocalProviderAgentLaunchGateway::new(data_dir.path().to_path_buf(), "releash".to_string());
    let executable = std::path::PathBuf::from(std::ffi::OsString::from_vec(
        b"/opt/bin/claude-\xff".to_vec(),
    ));

    let prepared = gateway
        .prepare(
            &armed(ProviderKind::Claude),
            ResolvedProviderExecutable::new(executable.clone()).unwrap(),
            ProviderSessionLaunch::New,
            data_dir.path().to_str().unwrap(),
        )
        .unwrap();

    assert_eq!(prepared.process().executable(), executable.as_os_str());
}
