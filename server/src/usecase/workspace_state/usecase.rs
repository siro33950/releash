use crate::domain::workspace_state::{
    WorkspaceState, WorkspaceStateError, WorkspaceStateRepository,
};

pub async fn save_workspace_state(
    repository: &dyn WorkspaceStateRepository,
    publisher: Option<&crate::usecase::state_subscription::StateSubscriptionUsecase>,
    worktree_name: &str,
    state: WorkspaceState,
) -> Result<(), WorkspaceStateError> {
    save_with_retry(|| async {
        let (version, previous) = repository.read(worktree_name).await?;
        let next = state.clone().preserve_omitted_panes(previous);
        repository.save(worktree_name, next, version).await
    })
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

async fn save_with_retry<Fut>(mut operation: impl FnMut() -> Fut) -> Result<(), WorkspaceStateError>
where
    Fut: std::future::Future<Output = Result<(), WorkspaceStateError>>,
{
    use crate::common::{
        operation_context,
        retry::{RetryBackoff, RetryLimiter},
    };
    operation_context::check()?;
    match operation().await {
        Err(WorkspaceStateError::VersionConflict) => {}
        result => return result,
    }
    let context = operation_context::current();
    let now = std::time::Instant::now();
    // Reserve time for returning the conflict through dispatch and ingress.
    let context = match context.remaining(now) {
        Some(remaining) => {
            context.with_deadline(operation_context::Deadline::new(now + remaining / 2))
        }
        None => context,
    };
    let retry = async {
        let limiter = RetryLimiter::new();
        let mut conflicts = 1;
        loop {
            if operation_context::current()
                .remaining(std::time::Instant::now())
                .is_some_and(|remaining| {
                    RetryBackoff::CONFLICT.maximum_delay(conflicts) >= remaining
                })
            {
                return Err(WorkspaceStateError::VersionConflict);
            }
            limiter.wait(RetryBackoff::CONFLICT, conflicts).await?;
            match operation().await {
                Err(WorkspaceStateError::VersionConflict) => conflicts += 1,
                result => return result,
            }
        }
    };
    match operation_context::scope(context.clone(), operation_context::wait(&context, retry)).await
    {
        Err(operation_context::OperationStopped::Expired) => {
            Err(WorkspaceStateError::VersionConflict)
        }
        Err(error) => Err(error.into()),
        Ok(Err(WorkspaceStateError::Technical(error)))
            if error.nature == crate::domain::failure::TechnicalFailureNature::TimedOut =>
        {
            Err(WorkspaceStateError::VersionConflict)
        }
        Ok(result) => result,
    }
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
    save_with_retry(|| async {
        let (version, _) = repository.read_repository_group(path).await?;
        repository
            .save_repository_group(path, collapsed, version)
            .await
    })
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
