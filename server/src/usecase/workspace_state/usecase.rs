use crate::domain::workspace_state::{
    WorkspaceState, WorkspaceStateError, WorkspaceStateRepository,
};

pub async fn save_workspace_state(
    repository: &dyn WorkspaceStateRepository,
    publisher: Option<&crate::usecase::state_subscription::StateSubscriptionUsecase>,
    worktree_name: &str,
    state: WorkspaceState,
) -> Result<(), WorkspaceStateError> {
    loop {
        let (version, previous) = repository.read(worktree_name).await?;
        let next = state.clone().preserve_omitted_panes(previous);
        match repository.save(worktree_name, next, version).await {
            Err(WorkspaceStateError::VersionConflict) => continue,
            result => {
                result?;
                break;
            }
        }
    }
    if let Some(publisher) = publisher {
        publisher.notify(
            crate::usecase::state_subscription::StateChangeSource::WorkspaceState(
                worktree_name.into(),
            ),
        );
    }
    Ok(())
}

pub async fn load_workspace_state(
    repository: &dyn WorkspaceStateRepository,
    worktree_name: &str,
    worktree_root: &str,
) -> Result<Option<WorkspaceState>, crate::domain::workspace_state::WorkspaceStateError> {
    super::query_service::load_workspace_state(repository, worktree_name, worktree_root).await
}

#[cfg(test)]
#[path = "usecase_test.rs"]
mod usecase_tests;

pub async fn save_repository_group_state(
    repository: &dyn WorkspaceStateRepository,
    publisher: Option<&crate::usecase::state_subscription::StateSubscriptionUsecase>,
    path: &str,
    collapsed: bool,
) -> Result<(), WorkspaceStateError> {
    loop {
        match repository.save_repository_group(path, collapsed).await {
            Err(WorkspaceStateError::VersionConflict) => continue,
            result => {
                result?;
                break;
            }
        }
    }
    if let Some(publisher) = publisher {
        publisher.notify(
            crate::usecase::state_subscription::StateChangeSource::RepositoryGroupState(
                path.into(),
            ),
        );
    }
    Ok(())
}

pub async fn load_repository_group_state(
    repository: &dyn WorkspaceStateRepository,
    path: &str,
) -> Result<bool, WorkspaceStateError> {
    repository.load_repository_group(path).await
}

pub struct WorkspaceStateMigrationResult {
    pub unresolved_repositories: Vec<(String, crate::usecase::repository_error::UsecaseError)>,
    pub migration: Result<(), WorkspaceStateError>,
}

pub async fn migrate_workspace_states(
    repository: &dyn WorkspaceStateRepository,
    repositories: &[String],
    worktree_repository: &crate::usecase::repository_usecase::RepositoryUsecase,
) -> WorkspaceStateMigrationResult {
    let mut unresolved_repositories = Vec::new();
    let inventories = repositories
        .iter()
        .map(|path| match worktree_repository.list_worktrees(path) {
            Ok(items) => Some(items.into_iter().map(|worktree| worktree.path).collect()),
            Err(error) => {
                unresolved_repositories.push((path.clone(), error));
                None
            }
        })
        .collect();
    let worktrees = crate::domain::workspace_state::services::migration_worktree_paths(inventories);
    let migration = repository.migrate_legacy(repositories, &worktrees).await;
    WorkspaceStateMigrationResult {
        unresolved_repositories,
        migration,
    }
}
