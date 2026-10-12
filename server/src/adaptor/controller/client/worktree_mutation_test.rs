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

#[tokio::test]
async fn test_複数worktree作成_リポジトリと各作成先を一度ずつ保護する() {
    // Given
    use super::*;
    use crate::usecase::repository_usecase::WorktreeExecutionArchiver;
    use std::sync::Arc;
    let runtime = WorkflowRuntimeUsecase::new(
        Arc::new(crate::usecase::workflow::runtime_command::runtime_command_tests::tests::FakeRuntimeGateway::default()),
        Arc::new(crate::usecase::workflow::test_helpers::NoopArchiveRepository),
    );
    let command = wire::command_request::Command::CreateWorktrees(wire::CreateWorktreesRequest {
        repo_path: "/repo".into(),
        branches: vec!["feature/a".into(), "bugfix/a".into()],
        ..Default::default()
    });
    // When
    let guards = admit(Some(&runtime), &command).unwrap();
    // Then
    assert_eq!(guards.len(), 3);
    for path in [
        "/repo",
        "/repo-worktrees/feature/a",
        "/repo-worktrees/bugfix/a",
    ] {
        let mut deletion = Box::pin(runtime.begin_worktree_deletion(path));
        assert!(futures_util::poll!(&mut deletion).is_pending());
    }
    drop(guards);
    for path in [
        "/repo",
        "/repo-worktrees/feature/a",
        "/repo-worktrees/bugfix/a",
    ] {
        drop(runtime.begin_worktree_deletion(path).await.unwrap());
    }
}

#[tokio::test]
async fn test_ui状態保存_絶対pathキーの保存が終わるまでworktree削除を待つ() {
    // Given
    use super::*;
    use crate::usecase::repository_usecase::WorktreeExecutionArchiver;
    use std::sync::Arc;
    let runtime = WorkflowRuntimeUsecase::new(
        Arc::new(crate::usecase::workflow::runtime_command::runtime_command_tests::tests::FakeRuntimeGateway::default()),
        Arc::new(crate::usecase::workflow::test_helpers::NoopArchiveRepository),
    );
    let guard = runtime
        .begin_workspace_state_mutation("/repo-worktrees/feature/a")
        .unwrap();
    // When
    let mut deletion = Box::pin(runtime.begin_worktree_deletion("/repo-worktrees/feature/a"));
    // Then
    assert!(futures_util::poll!(&mut deletion).is_pending());
    drop(guard);
    drop(deletion.await.unwrap());
}

#[tokio::test]
async fn test_複数worktree作成_handler破棄後もblocking作成が終わるまで削除を待つ() {
    use super::*;
    use crate::usecase::create_worktrees::{create_worktrees, LaunchAfterCreation, WorktreeLaunch};
    use crate::usecase::repository_error::UsecaseError;
    use crate::usecase::repository_usecase::WorktreeExecutionArchiver;
    use crate::usecase::test_helpers::{usecase, FakeRepo};
    use std::sync::Arc;
    struct Launcher;
    #[async_trait::async_trait]
    impl WorktreeLaunch for Launcher {
        async fn launch(&self, _: &str, _: &LaunchAfterCreation) -> Result<(), UsecaseError> {
            Ok(())
        }
    }
    // Given
    let runtime = WorkflowRuntimeUsecase::new(
        Arc::new(crate::usecase::workflow::runtime_command::runtime_command_tests::tests::FakeRuntimeGateway::default()),
        Arc::new(crate::usecase::workflow::test_helpers::NoopArchiveRepository),
    );
    let command = wire::command_request::Command::CreateWorktrees(wire::CreateWorktreesRequest {
        repo_path: "/repo".into(),
        branches: vec!["a".into(), "b".into()],
        ..Default::default()
    });
    let guards = admit(Some(&runtime), &command).unwrap();
    let (finish, receiver) = std::sync::mpsc::channel();
    let fake = Arc::new(FakeRepo {
        create_continue: Some(parking_lot::Mutex::new(receiver)),
        ..Default::default()
    });
    let repository = Arc::new(usecase(fake.clone()));
    let handler = tokio::spawn(scope(guards, async move {
        create_worktrees(
            repository,
            &Launcher,
            "/repo".into(),
            vec!["a".into(), "b".into()],
            None,
            LaunchAfterCreation::None,
        )
        .await
    }));
    fake.create_started.notified().await;
    // When
    handler.abort();
    assert!(handler.await.unwrap_err().is_cancelled());
    let mut deletion = Box::pin(runtime.begin_worktree_deletion("/repo-worktrees/a"));
    assert!(futures_util::poll!(&mut deletion).is_pending());
    finish.send(()).unwrap();
    // Then
    drop(
        tokio::time::timeout(std::time::Duration::from_secs(2), deletion)
            .await
            .unwrap()
            .unwrap(),
    );
    assert_eq!(fake.worktree_creations.lock().len(), 1);
}
