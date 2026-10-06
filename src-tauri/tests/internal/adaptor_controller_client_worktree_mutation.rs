use crate::adaptor_controller_api_mod::test_support::RecordingRuntimeGateway;
use releash_lib::test_support::integration::persistence::LocalEventStore;
use releash_lib::test_support::integration::persistence::LocalEventStoreConfig;
use releash_lib::test_support::integration::platform::WorktreeExecutionArchiver;
use releash_lib::test_support::integration::transport::admit;
use releash_lib::test_support::integration::wire;
use releash_lib::test_support::integration::workflow::ExecutionTreeArchiveFactRepository;
use releash_lib::test_support::integration::workflow::WorkflowRuntimeUsecase;
use std::sync::Arc;

#[tokio::test]
pub async fn test_worktree削除中_変更対象を共通境界で拒否して読み取りは通す() {
    use wire::command_request::Command as C;
    // Given
    let directory = tempfile::tempdir().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().into(),
        std::sync::Arc::new(releash_lib::test_support::integration::platform::RetryLimiter::new()),
    ))
    .unwrap();
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
        C::KillTerminalSurface(wire::KillTerminalSurfaceRequest { owner: Some(owner) }),
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
        let error = admit(Some(&runtime), &command)
            .err()
            .unwrap_or_else(|| panic!("{}", wire::command_name(&command)));
        let Some(wire::command_error::Variant::Coded(error)) = error.detail.variant else {
            panic!("{}: expected coded error", wire::command_name(&command))
        };
        assert_eq!(
            error.code.as_deref(),
            Some("WORKTREE_MUTATION_REJECTED"),
            "{}",
            wire::command_name(&command)
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
