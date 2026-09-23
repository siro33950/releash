//! Shared Workspace read contract and source-fact projection helpers.

mod query_service;
#[cfg(test)]
mod test_support;

pub(crate) use query_service::WorkspaceQueryService;
#[cfg(test)]
pub(crate) use test_support::TestWorkspaceQueryService;

mod worktree_path;
pub(crate) use worktree_path::{WorkspaceWorktreePathQuery, WorkspaceWorktreePathUsecase};

mod list;
mod list_query_service;
pub(crate) use list::{
    WorkspaceBranchDto, WorkspaceListSnapshotDto, WorkspaceListStatusDto, WorkspaceListUsecase,
    WorkspaceRepositoryListDto, WorkspaceWorktreeListDto,
};
pub(crate) use list_query_service::WorkspaceListServices;
#[cfg(all(test, feature = "desktop"))]
pub(crate) use list_query_service::{WorkspaceListQueryService, WorkspaceListUsecaseError};
