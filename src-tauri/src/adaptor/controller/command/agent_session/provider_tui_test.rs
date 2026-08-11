use super::*;

#[tokio::test(flavor = "current_thread")]
async fn test_provider_availability_controller_blocking操作中もasync_runtimeを占有しない() {
    let heartbeat = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let heartbeat_task = {
        let heartbeat = heartbeat.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            heartbeat.store(true, std::sync::atomic::Ordering::SeqCst);
        })
    };

    run_provider_availability_blocking(|| {
        std::thread::sleep(std::time::Duration::from_millis(50));
        Ok(())
    })
    .await
    .unwrap();
    heartbeat_task.await.unwrap();

    assert!(heartbeat.load(std::sync::atomic::Ordering::SeqCst));
}

#[test]
fn test_provider_agent_session_controller_lifecycle入力を閉じた型へ変換する() {
    assert_eq!(
        parse_lifecycle("open").unwrap(),
        ProviderAgentSessionLifecycleDto::Open
    );
    assert_eq!(
        parse_lifecycle("paused").unwrap(),
        ProviderAgentSessionLifecycleDto::Paused
    );
    assert_eq!(
        parse_lifecycle("archived").unwrap(),
        ProviderAgentSessionLifecycleDto::Archived
    );
    assert!(parse_lifecycle("deleted").is_err());
}

#[test]
fn test_provider_agent_session_controller_domain結果をwire語彙へ変換する() {
    assert_eq!(
        ProviderAgentSessionOpenResponse::from(ProviderAgentSessionOpenOutcome::Indeterminate),
        ProviderAgentSessionOpenResponse::Indeterminate
    );
    assert_eq!(
        ProviderAgentSessionArchiveResponse::from(
            AgentSessionArchiveOutcome::DeleteConfirmationRequired
        ),
        ProviderAgentSessionArchiveResponse::DeleteConfirmationRequired
    );
}

#[test]
fn test_provider_agent_session_controller_provider未選択と未知値を起動前に拒否する() {
    assert_eq!(
        parse_provider("claude").unwrap(),
        crate::domain::provider_lifecycle::ProviderKind::Claude
    );
    assert_eq!(
        parse_provider("codex").unwrap(),
        crate::domain::provider_lifecycle::ProviderKind::Codex
    );
    let missing = serde_json::to_value(parse_provider("").unwrap_err()).unwrap();
    let unknown = serde_json::to_value(parse_provider("unknown").unwrap_err()).unwrap();
    assert_eq!(missing["code"], "PROVIDER_AGENT_SESSION_INVALID_PROVIDER");
    assert_eq!(unknown["code"], "PROVIDER_AGENT_SESSION_INVALID_PROVIDER");
}

#[test]
fn test_provider_agent_session_controller_wire型とapperrorを規約境界へ置く() {
    let source = include_str!("provider_tui.rs");

    assert!(!source.contains("pub(crate) enum ProviderAgentSessionOpenOutcomeDto"));
    assert!(!source.contains("pub(crate) enum ProviderAgentSessionArchiveOutcomeDto"));
    assert!(!source.contains("pub(crate) struct ProviderHookHealthWarningDto"));
    assert!(!source.contains("Result<String, String>"));
    assert!(!source.contains("Result<(), String>"));
}
