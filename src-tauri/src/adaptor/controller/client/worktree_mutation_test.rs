use super::*;
use crate::adaptor::controller::api::test_support::RecordingRuntimeGateway;
use crate::adaptor::gateway::local_event_store::{LocalEventStore, LocalEventStoreConfig};
use crate::adaptor::gateway::workflow::ExecutionTreeArchiveFactRepository;
use crate::usecase::repository_usecase::WorktreeExecutionArchiver;
use std::sync::Arc;

#[tokio::test]
async fn test_worktree削除中_変更対象を共通境界で拒否して読み取りは通す() {
    use wire::command_request::Command as C;
    // Given
    let directory = tempfile::tempdir().unwrap();
    let store =
        LocalEventStore::open(LocalEventStoreConfig::production(directory.path().into())).unwrap();
    let runtime = WorkflowRuntimeUsecase::new(
        Arc::new(RecordingRuntimeGateway::default()),
        Arc::new(ExecutionTreeArchiveFactRepository::new(
            store,
            directory.path(),
        )),
    );
    let path = "/repo-worktrees/feature";
    let _deletion = runtime.begin_worktree_deletion(path).await.unwrap();
    let owner = wire::TerminalSurfaceOwnerV1 {
        variant: Some(wire::terminal_surface_owner_v1::Variant::Workspace(
            wire::TerminalSurfaceOwnerV1Workspace {
                workspace_path: Some(path.into()),
            },
        )),
    };
    let mutations = [
        C::GitStage(wire::GitStageRequest {
            repo_path: Some(path.into()),
            ..Default::default()
        }),
        C::GitUnstage(wire::GitUnstageRequest {
            repo_path: Some(path.into()),
            ..Default::default()
        }),
        C::GitCreateBranch(wire::GitCreateBranchRequest {
            repo_path: Some(path.into()),
            ..Default::default()
        }),
        C::CreateReviewThread(wire::CreateReviewThreadRequest {
            worktree_name: Some(path.into()),
            ..Default::default()
        }),
        C::AppendReviewComment(wire::AppendReviewCommentRequest {
            worktree_name: Some(path.into()),
            ..Default::default()
        }),
        C::ResolveReviewThread(wire::ResolveReviewThreadRequest {
            worktree_name: Some(path.into()),
            ..Default::default()
        }),
        C::DeleteReviewThread(wire::DeleteReviewThreadRequest {
            worktree_name: Some(path.into()),
            ..Default::default()
        }),
        C::RenameWorkspaceSessionNode(wire::RenameWorkspaceSessionNodeRequest {
            worktree_path: Some(path.into()),
            ..Default::default()
        }),
        C::GetOrSpawnTerminalSurface(wire::GetOrSpawnTerminalSurfaceRequest {
            owner: Some(owner.clone()),
            ..Default::default()
        }),
        C::WriteTerminalSurface(wire::WriteTerminalSurfaceRequest {
            owner: Some(owner.clone()),
            ..Default::default()
        }),
        C::WritePathsToTerminalSurface(wire::WritePathsToTerminalSurfaceRequest {
            owner: Some(owner.clone()),
            ..Default::default()
        }),
        C::ResizeTerminalSurface(wire::ResizeTerminalSurfaceRequest {
            owner: Some(owner.clone()),
            ..Default::default()
        }),
        C::KillTerminalSurface(wire::KillTerminalSurfaceRequest {
            owner: Some(owner),
            ..Default::default()
        }),
        C::CreateWorktree(wire::CreateWorktreeRequest {
            repo_path: Some("/repo".into()),
            branch: Some("feature".into()),
            ..Default::default()
        }),
        C::SaveWorkspaceState(wire::SaveWorkspaceStateRequest {
            worktree_name: Some("feature".into()),
            ..Default::default()
        }),
    ];
    // When / Then
    for command in mutations {
        let error = admit(Some(&runtime), &command).err().expect(command.name());
        let Some(wire::command_error::Variant::Coded(error)) = error.detail.variant else {
            panic!("{}: expected coded error", command.name())
        };
        assert_eq!(
            error.code.as_deref(),
            Some("WORKTREE_MUTATION_REJECTED"),
            "{}",
            command.name()
        );
    }
    assert!(admit(
        Some(&runtime),
        &C::BuildDiffFileTree(wire::BuildDiffFileTreeRequest {
            entries: Some(wire::ListDiffFileEntryInput { items: vec![] })
        })
    )
    .unwrap()
    .is_empty());
    assert!(admit(
        Some(&runtime),
        &C::GitStage(wire::GitStageRequest {
            repo_path: Some("/other".into()),
            ..Default::default()
        })
    )
    .is_ok());
}

#[tokio::test]
async fn test_同期変更_scope破棄後も完了またはpanicまで削除を待機する() {
    for panic in [false, true] {
        // Given
        let runtime = WorkflowRuntimeUsecase::new(
            Arc::new(RecordingRuntimeGateway::default()),
            Arc::new(crate::usecase::workflow::NoopArchiveRepository),
        );
        let guard = runtime.begin_worktree_mutation("/repo").unwrap();
        let started = Arc::new(tokio::sync::Notify::new());
        let signal = started.clone();
        let (finish, receiver) = std::sync::mpsc::channel();
        let task = scope(vec![guard], async move {
            spawn_blocking(move || {
                signal.notify_one();
                receiver.recv().unwrap();
                assert!(!panic, "blocking mutation panic");
            })
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
