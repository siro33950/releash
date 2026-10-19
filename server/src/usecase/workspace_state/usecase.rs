use crate::common::retry::{self, AttemptProgress, RetryBackoff, RetryLimiter};
use crate::domain::workspace_state::{
    WorkspaceState, WorkspaceStateError, WorkspaceStateRepository,
};

pub async fn save_workspace_state(
    repository: &dyn WorkspaceStateRepository,
    publisher: Option<&crate::usecase::state_subscription::StateSubscriptionUsecase>,
    worktree_name: &str,
    state: WorkspaceState,
) -> Result<(), WorkspaceStateError> {
    retry::attempts(
        RetryBackoff::CONFLICT,
        &RetryLimiter::new(),
        |error| match error {
            WorkspaceStateError::VersionConflict => Some(AttemptProgress::Reload),
            _ => None,
        },
        |_| async {
            let (version, previous) = repository.read(worktree_name).await?;
            let next = state.clone().preserve_omitted_panes(previous);
            repository.save(worktree_name, next, version).await
        },
    )
    .await?;
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
    query: &dyn crate::usecase::workspace_tree::WorkspaceQueryService,
    worktree_name: &str,
    worktree_root: &str,
) -> Result<
    (
        Option<WorkspaceState>,
        Vec<crate::usecase::workspace_tree::query_service::SessionNodeResolutionFailureDto>,
        Option<WorkspaceStateError>,
    ),
    WorkspaceStateError,
> {
    retry::attempts(
        RetryBackoff::CONFLICT,
        &RetryLimiter::new(),
        |error| match error {
            WorkspaceStateError::VersionConflict => Some(AttemptProgress::Reload),
            _ => None,
        },
        |_| async {
            let (version, state) = repository.read(worktree_name).await?;
            let Some(mut state) = state else {
                return Ok((None, Vec::new(), None));
            };
            let mut failures = Vec::new();
            let mut migration_failure = None;
            if let Some(layout) = state.panes.as_mut() {
                let ids = layout
                    .legacy_session_nodes()
                    .into_iter()
                    .collect::<std::collections::HashSet<_>>()
                    .into_iter()
                    .collect::<Vec<_>>();
                if !ids.is_empty() {
                    let resolution = query
                        .resolve_session_nodes(
                            &crate::domain::workspace_tree::WorkspaceIdentity::new(worktree_root),
                            &ids,
                        )
                        .await;
                    failures = resolution.failures;
                    let previous = layout.clone();
                    layout.resolve_legacy_sessions(&resolution.sessions);
                    if *layout != previous {
                        match repository.save(worktree_name, state.clone(), version).await {
                            Ok(()) => {}
                            Err(WorkspaceStateError::VersionConflict) => {
                                return Err(WorkspaceStateError::VersionConflict);
                            }
                            Err(error) => migration_failure = Some(error),
                        }
                    }
                }
            }
            let state = repository
                .filter_missing_files(state, worktree_root)
                .await?;
            Ok((Some(state), failures, migration_failure))
        },
    )
    .await
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
    retry::attempts(
        RetryBackoff::CONFLICT,
        &RetryLimiter::new(),
        |error| match error {
            WorkspaceStateError::VersionConflict => Some(AttemptProgress::Reload),
            _ => None,
        },
        |_| async {
            let (version, _) = repository.read_repository_group(path).await?;
            repository
                .save_repository_group(path, collapsed, version)
                .await
        },
    )
    .await?;
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
