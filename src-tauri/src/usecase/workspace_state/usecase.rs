use crate::domain::workspace_state::{
    WorkspaceState, WorkspaceStateError, WorkspaceStateRepository,
};

pub(crate) fn save_workspace_state(
    repository: &dyn WorkspaceStateRepository,
    publisher: Option<&crate::usecase::state_subscription::StateSubscriptionPublisher>,
    worktree_name: &str,
    state: WorkspaceState,
) -> Result<(), WorkspaceStateError> {
    repository.set(worktree_name, state);
    repository.save(worktree_name)?;
    if let Some(publisher) = publisher {
        publisher.invalidate(
            crate::domain::state_subscription::StateChangeSource::WorkspaceState(
                worktree_name.into(),
            ),
        );
    }
    Ok(())
}

pub fn load_workspace_state(
    repository: &dyn WorkspaceStateRepository,
    worktree_name: &str,
    worktree_root: &str,
) -> Option<WorkspaceState> {
    super::query_service::load_workspace_state(repository, worktree_name, worktree_root)
}

#[cfg(test)]
#[path = "usecase_test.rs"]
mod usecase_tests;
