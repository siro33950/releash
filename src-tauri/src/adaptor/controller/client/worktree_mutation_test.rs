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
        let Some(wire::command_error::Variant::Coded(error)) = error.variant else {
            panic!("{}: expected coded error", command.name())
        };
        assert_eq!(
            error.code.as_deref(),
            Some("WORKTREE_MUTATION_REJECTED"),
            "{}",
            command.name()
        );
    }
    for command in [
        C::ListWorkspaceWorktreeNodes(wire::ListWorkspaceWorktreeNodesRequest {
            worktree_path: Some(path.into()),
        }),
        C::GetWorkspaceNodeDetail(wire::GetWorkspaceNodeDetailRequest {
            worktree_path: Some(path.into()),
            ..Default::default()
        }),
        C::LoadWorkspaceState(wire::LoadWorkspaceStateRequest {
            worktree_name: Some("feature".into()),
            worktree_root: Some(path.into()),
        }),
        C::ListWorktrees(wire::ListWorktreesRequest {
            repo_path: Some(path.into()),
            ..Default::default()
        }),
    ] {
        assert!(admit(Some(&runtime), &command).unwrap().is_empty());
    }
    assert!(admit(
        Some(&runtime),
        &C::GitStage(wire::GitStageRequest {
            repo_path: Some("/other".into()),
            ..Default::default()
        })
    )
    .is_ok());
}
