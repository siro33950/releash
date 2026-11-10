use crate::common::retry::{self, AttemptProgress, RetryBackoff, RetryLimiter};
use crate::domain::local_event::StreamVersion;
use crate::domain::workspace_state::{
    WorkspaceState, WorkspaceStateError, WorkspaceStateRepository,
};

/// client が観測した版 `expected` に対して保存する。版が進んでいれば保存せず版競合を返す。
pub async fn save_workspace_state(
    repository: &dyn WorkspaceStateRepository,
    publisher: Option<&crate::usecase::state_subscription::StateSubscriptionUsecase>,
    worktree_name: &str,
    state: WorkspaceState,
    expected: StreamVersion,
) -> Result<(StreamVersion, WorkspaceState), WorkspaceStateError> {
    let (_, previous) = repository.read(worktree_name).await?;
    let next = state.preserve_omitted_fields(previous);
    let version = repository
        .save(worktree_name, next.clone(), expected)
        .await?;
    if let Some(publisher) = publisher {
        publisher.notify(
            crate::usecase::state_subscription::StateChangeSource::WorkspaceState(
                worktree_name.into(),
            ),
        );
    }
    Ok((version, next))
}

pub async fn load_workspace_state(
    repository: &dyn WorkspaceStateRepository,
    query: &dyn crate::usecase::workspace_tree::WorkspaceQueryService,
    worktree_name: &str,
    worktree_root: &str,
) -> Result<
    (
        StreamVersion,
        Option<WorkspaceState>,
        Vec<crate::usecase::workspace_tree::query_service::SessionNodeResolutionFailureDto>,
        Option<WorkspaceStateError>,
    ),
    WorkspaceStateError,
> {
    retry::attempts(
        RetryBackoff::CONFLICT,
        &RetryLimiter::new(),
        conflict_progress,
        |_| async {
            let (mut version, state) = repository.read(worktree_name).await?;
            let mut state = state.unwrap_or_default().initialize_panes();
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
                            Ok(saved) => version = saved,
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
            Ok((version, Some(state), failures, migration_failure))
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
        conflict_progress,
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

pub(super) fn conflict_progress(error: &WorkspaceStateError) -> Option<AttemptProgress> {
    match error {
        WorkspaceStateError::VersionConflict => Some(AttemptProgress::Reload),
        _ => None,
    }
}
