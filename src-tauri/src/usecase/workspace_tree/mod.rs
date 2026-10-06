//! Shared Workspace read contract and source-fact projection helpers.

pub(crate) mod query_service;
#[cfg(any(test, feature = "test-support"))]
pub(crate) mod test_support;

pub(crate) use query_service::WorkspaceQueryService;

pub(crate) mod worktree_path;
pub(crate) use worktree_path::{WorkspaceWorktreePathQuery, WorkspaceWorktreePathUsecase};

pub(crate) mod list;
pub(crate) use list::{
    WorkspaceList, WorkspaceListRepository, WorkspaceListUsecase, WorkspaceListWorktree,
};
