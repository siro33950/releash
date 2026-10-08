mod restored_memory_cases {
    use super::super::*;
    use crate::usecase::repository_usecase::WorktreeExecutionArchiver;
    use crate::usecase::workflow::WorkflowRuntimeUsecase;
    use std::sync::Arc;

    #[tokio::test]
    pub async fn test_同期変更_scope破棄後も完了またはpanicまで削除を待機する() {
        for panic in [false, true] {
            // Given
            let runtime = WorkflowRuntimeUsecase::new(
            Arc::new(crate::usecase::workflow::runtime_command::runtime_command_tests::tests::FakeRuntimeGateway::default()),
            Arc::new(crate::usecase::workflow::test_helpers::NoopArchiveRepository),
        );
            let guard = runtime.begin_worktree_mutation("/repo").unwrap();
            let started = Arc::new(tokio::sync::Notify::new());
            let signal = started.clone();
            let (finish, receiver) = std::sync::mpsc::channel();
            let (task,) = scope(vec![guard], async move {
                (spawn_blocking(move || {
                    signal.notify_one();
                    receiver.recv().unwrap();
                    assert!(!panic, "blocking mutation panic");
                }),)
            })
            .await;
            started.notified().await;
            let mut deletion = Box::pin(runtime.begin_worktree_deletion("/repo"));
            assert!(futures_util::poll!(&mut deletion).is_pending());
            assert!(runtime.begin_worktree_mutation("/repo").is_err());
            // When
            finish.send(()).unwrap();
            assert_eq!(task.await.is_err(), panic);
            // Then
            drop(deletion.await.unwrap());
            assert!(runtime.begin_worktree_mutation("/repo").is_ok());
        }
    }
}
