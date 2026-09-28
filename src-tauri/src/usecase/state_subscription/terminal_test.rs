use super::*;

struct PendingTimer;

impl SubscriptionTimer for PendingTimer {
    fn interval(
        &self,
        _: std::time::Duration,
    ) -> std::pin::Pin<Box<dyn futures_util::Stream<Item = ()> + Send>> {
        Box::pin(futures_util::stream::pending())
    }
}

#[tokio::test]
async fn test_terminal入力識別子_上限を受け付け超過を拒否する() {
    // Given
    let usecase = StateSubscriptionUsecase::new(vec![], std::sync::Arc::new(PendingTimer));
    let target = SubscriptionTarget::Terminal(
        crate::domain::terminal_surface::TerminalSurfaceOwner::workspace(
            crate::domain::workspace_tree::WorkspaceIdentity::new("/repo"),
        )
        .unwrap(),
    );
    let at_limit = "a".repeat(TERMINAL_INPUT_ID_MAX_BYTES);
    let over_limit = "a".repeat(TERMINAL_INPUT_ID_MAX_BYTES + 1);
    // When
    let accepted = usecase
        .start_terminal("client", &target, &at_limit, None)
        .await;
    let rejected = usecase
        .start_terminal("client", &target, &over_limit, None)
        .await;
    // Then
    assert!(matches!(
        accepted,
        Err(StateReadError {
            source: StateReadFailure::Technical(_),
            ..
        })
    ));
    assert!(matches!(
        rejected,
        Err(StateReadError {
            source: StateReadFailure::InvalidTerminalInput,
            ..
        })
    ));
}
