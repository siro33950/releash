//! Shared Workspace read contract and source-fact projection helpers.

mod query_service;
#[cfg(test)]
mod test_support;

pub(crate) use query_service::{
    WorkspaceNodeApprovalRoute, WorkspaceNodeCloseRoute, WorkspaceQueryService,
};
#[cfg(test)]
pub(crate) use test_support::TestWorkspaceQueryService;
