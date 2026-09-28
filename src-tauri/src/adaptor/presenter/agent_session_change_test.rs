use super::*;

#[tokio::test]
async fn test_agent_session通知_購読対象の更新を通知する() {
    let publisher = crate::adaptor::presenter::state_subscription::test_output();
    let mut changes = crate::test_support::state_subscription::changes(&publisher);
    let notifier = ClientAgentSessionChangeNotifier::new(publisher);
    notifier.agent_session_changed("/repo");
    assert_eq!(
        changes.recv().await.unwrap(),
        crate::usecase::state_subscription::StateChangeSource::Worktree("/repo".into())
    );
}
