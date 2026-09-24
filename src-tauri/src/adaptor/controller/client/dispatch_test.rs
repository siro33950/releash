use super::*;
use crate::domain::state_subscription::StateChangeSource;
use crate::usecase::state_subscription::StateReadsFixture;

#[tokio::test]
async fn test_更新通知_成功時だけ購読対象を更新する() {
    // Given
    let fixture = StateReadsFixture::new();
    let publisher = fixture.subscriptions.publisher();
    let git_host = fixture.reads.git_host.clone();
    let mut changes = publisher.subscribe_changes();
    let mut dispatch = ClientCommandDispatch::new(Arc::new(ApplicationStartupAuthority::ready()))
        .with_state_publisher(publisher);
    dispatch.register_domain(
        &["fetch_issues"],
        Box::new(move |command| {
            let git_host = git_host.clone();
            Box::pin(async move {
                let wire::command_request::Command::FetchIssues(args) = command else {
                    return Err(invalid_request("Mismatched command"));
                };
                if args.repo_path.as_deref() == Some("/fail") {
                    return Err(invalid_request("Fetch failed"));
                }
                git_host.fetch_issues(args.repo_path.as_deref().unwrap());
                Ok(wire::command_result::Command::FetchIssues(wire::Unit {}))
            })
        }),
    );
    // When
    for path in ["/repo", "/fail"] {
        let result = dispatch
            .dispatch(wire::command_request::Command::FetchIssues(
                wire::FetchIssuesRequest {
                    repo_path: Some(path.into()),
                },
            ))
            .await;
        // Then
        if path == "/repo" {
            assert!(result.is_ok());
            assert_eq!(
                changes.try_recv().unwrap(),
                StateChangeSource::Issues(path.into())
            );
        } else {
            assert!(result.is_err());
            assert!(matches!(
                changes.try_recv(),
                Err(tokio::sync::broadcast::error::TryRecvError::Empty)
            ));
        }
    }
}
