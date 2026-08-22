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
fn test_agent_session_controller_domain結果をwire語彙へ変換する() {
    assert_eq!(
        AgentSessionOpenResponse::from(AgentSessionOpenOutcome::Indeterminate),
        AgentSessionOpenResponse::Indeterminate
    );
    assert_eq!(
        AgentSessionArchiveResponse::from(AgentSessionArchiveOutcome::DeleteConfirmationRequired),
        AgentSessionArchiveResponse::DeleteConfirmationRequired
    );
}

#[test]
fn test_agent_session_controller_provider未選択と未知値を起動前に拒否する() {
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
    assert_eq!(missing["code"], "AGENT_SESSION_INVALID_PROVIDER");
    assert_eq!(unknown["code"], "AGENT_SESSION_INVALID_PROVIDER");
}
