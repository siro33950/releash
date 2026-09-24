use super::{command::AbortExecutionCommand, WorkflowRuntimeUsecase};
use crate::domain::workflow::{ExecutionTreeArchiveRepository, ExecutionTreeId, WorkflowError};

impl WorkflowRuntimeUsecase {
    pub(crate) fn begin_worktree_mutation(
        &self,
        path: &str,
    ) -> Result<crate::usecase::worktree_operation::WorktreeMutationGuard, WorkflowError> {
        let identity = self.execution_archives.worktree_identity(path)?;
        self.worktree_operations
            .mutate(&identity)
            .map_err(map_worktree_operation_error)
    }

    pub(crate) fn begin_worktree_creation_mutation(
        &self,
        repo: &str,
        branch: &str,
    ) -> Result<Vec<crate::usecase::worktree_operation::WorktreeMutationGuard>, WorkflowError> {
        Ok(vec![
            self.begin_worktree_mutation(repo)?,
            self.begin_worktree_mutation(&crate::domain::repository::worktree_path(repo, branch))?,
        ])
    }

    pub(crate) fn begin_workspace_state_mutation(
        &self,
        name: &str,
    ) -> Result<crate::usecase::worktree_operation::WorktreeMutationGuard, WorkflowError> {
        self.worktree_operations
            .mutate(&format!("workspace-state:{name}"))
            .map_err(map_worktree_operation_error)
    }

    pub(crate) async fn begin_execution_tree_mutation(
        &self,
        tree_id: &str,
    ) -> Result<Vec<crate::usecase::worktree_operation::WorktreeMutationGuard>, WorkflowError> {
        ExecutionTreeId::new(tree_id)?;
        let location = self.execution_archives.location(tree_id).await?;
        Ok(vec![
            self.begin_worktree_mutation(&location.workspace_identity)?,
            self.begin_worktree_mutation(&location.worktree_path)?,
        ])
    }

    pub(crate) async fn lock_execution_tree(
        &self,
        tree_id: &str,
    ) -> Result<tokio::sync::OwnedMutexGuard<()>, WorkflowError> {
        let lock = {
            let mut locks = self
                .archive_locks
                .lock()
                .map_err(|_| WorkflowError::external("execution tree operation lock poisoned"))?;
            locks.retain(|_, lock| lock.strong_count() > 0);
            match locks.get(tree_id).and_then(std::sync::Weak::upgrade) {
                Some(lock) => lock,
                None => {
                    let lock = std::sync::Arc::new(tokio::sync::Mutex::new(()));
                    locks.insert(tree_id.to_string(), std::sync::Arc::downgrade(&lock));
                    lock
                }
            }
        };
        Ok(lock.lock_owned().await)
    }

    pub async fn archive_execution_tree(
        &self,
        execution_id: &str,
        reason: &str,
    ) -> Result<(), WorkflowError> {
        let _mutation = self.begin_execution_tree_mutation(execution_id).await?;
        let repository = self.execution_archives.clone();
        self.archive_execution_tree_at(
            repository.as_ref(),
            execution_id,
            reason,
            self.runtime.current_timestamp(),
        )
        .await
    }

    async fn archive_execution_tree_at(
        &self,
        repository: &dyn ExecutionTreeArchiveRepository,
        execution_id: &str,
        reason: &str,
        archived_at: f64,
    ) -> Result<(), WorkflowError> {
        let _operation = self.lock_execution_tree(execution_id).await?;
        let id = ExecutionTreeId::new(execution_id.to_string())?;
        let target = repository.target(execution_id).await?;
        if target.status.is_active() {
            let result = async {
                self.runtime
                    .register_started_execution_tree(execution_id)
                    .await?;
                self.abort_execution
                    .execute(AbortExecutionCommand {
                        execution_id: execution_id.to_string(),
                        expected_node_name: None,
                    })
                    .await
            }
            .await;
            if let Err(error) = result {
                if !matches!(
                    error,
                    WorkflowError::InvalidState(_) | WorkflowError::NotFound(_)
                ) || repository.target(execution_id).await?.status.is_active()
                {
                    return Err(error);
                }
            }
        }
        self.runtime
            .stop_execution_tree_processes(execution_id)
            .await?;
        repository.archive(&id, archived_at, reason).await?;
        if let Some(publisher) = &self.state_publisher {
            publisher.invalidate(
                crate::domain::state_subscription::StateChangeSource::Worktree(
                    target.worktree_path,
                ),
            );
        }
        Ok(())
    }

    pub async fn archive_worktree(&self, worktree_path: &str) -> Result<(), WorkflowError> {
        let repository = self.execution_archives.clone();
        let mut after = None;
        loop {
            let page = repository
                .worktree_target_page(worktree_path, after.as_deref())
                .await?;
            let Some(last) = page.last() else { break };
            after = Some(last.execution_id.clone());
            for target in page {
                self.archive_execution_tree_at(
                    repository.as_ref(),
                    &target.execution_id,
                    "worktree_removed",
                    self.runtime.current_timestamp(),
                )
                .await?;
            }
        }
        Ok(())
    }

    pub async fn restore_execution_tree(&self, execution_id: &str) -> Result<(), WorkflowError> {
        let _mutation = self.begin_execution_tree_mutation(execution_id).await?;
        let _operation = self.lock_execution_tree(execution_id).await?;
        self.restore_execution_tree_locked(execution_id).await
    }

    pub(crate) async fn restore_execution_tree_locked(
        &self,
        execution_id: &str,
    ) -> Result<(), WorkflowError> {
        let id = ExecutionTreeId::new(execution_id.to_string())?;
        let repository = self.execution_archives.clone();
        let target = repository.target(execution_id).await?;
        self.runtime
            .resolve_start_execution_worktree(target.worktree_path.clone())
            .await?;
        repository
            .restore(&id, self.runtime.current_timestamp())
            .await?;
        if let Some(publisher) = &self.state_publisher {
            publisher.invalidate(
                crate::domain::state_subscription::StateChangeSource::Worktree(
                    target.worktree_path,
                ),
            );
        }
        Ok(())
    }

    pub(crate) async fn migrate_execution_archives(
        &self,
        repository: &dyn ExecutionTreeArchiveRepository,
    ) -> Result<(), WorkflowError> {
        for record in repository.legacy_archives()? {
            self.archive_execution_tree_at(
                repository,
                &record.execution_id,
                &record.archive_reason,
                record.archived_at,
            )
            .await?;
        }
        let mut after = None;
        loop {
            let records = repository
                .legacy_session_archive_page(after.as_deref())
                .await?;
            let Some(last) = records.last() else { break };
            after = Some(last.execution_id.clone());
            for record in records {
                self.archive_execution_tree_at(
                    repository,
                    &record.execution_id,
                    &record.archive_reason,
                    record.archived_at,
                )
                .await?;
            }
        }
        repository.finish_legacy_migration()
    }
}

#[async_trait::async_trait]
impl crate::usecase::repository_usecase::WorktreeExecutionArchiver for WorkflowRuntimeUsecase {
    async fn begin_worktree_deletion(
        &self,
        path: &str,
    ) -> Result<crate::usecase::worktree_operation::WorktreeDeletionGuard, WorkflowError> {
        let identity = self.execution_archives.worktree_identity(path)?;
        let name = std::path::Path::new(&identity)
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| WorkflowError::validation("worktree path has no name"))?;
        self.worktree_operations
            .delete_many(&[identity.clone(), format!("workspace-state:{name}")])
            .await
            .map_err(map_worktree_operation_error)
    }
    async fn archive_worktree(&self, worktree_path: &str) -> Result<(), WorkflowError> {
        self.archive_worktree(worktree_path).await
    }
}

#[async_trait::async_trait]
impl crate::usecase::app_data_gc::ExecutionTreeGc for WorkflowRuntimeUsecase {
    async fn execution_trees(
        &self,
        after: Option<&str>,
    ) -> Result<Vec<crate::domain::workflow::ExecutionTreeArchiveCandidate>, WorkflowError> {
        self.execution_archives.candidate_page(after).await
    }
    async fn record_repository_root(
        &self,
        execution_id: &str,
        root: &str,
    ) -> Result<(), WorkflowError> {
        self.execution_archives
            .record_repository_root(execution_id, root, self.runtime.current_timestamp())
            .await
    }
    async fn archive_removed_tree(&self, execution_id: &str) -> Result<(), WorkflowError> {
        self.archive_execution_tree_at(
            self.execution_archives.as_ref(),
            execution_id,
            "worktree_removed",
            self.runtime.current_timestamp(),
        )
        .await
    }
}

fn map_worktree_operation_error(
    error: crate::domain::repository::RepositoryError,
) -> WorkflowError {
    match error {
        crate::domain::repository::RepositoryError::Rule(message) => {
            WorkflowError::Conflict(message)
        }
        crate::domain::repository::RepositoryError::External(message) => {
            WorkflowError::external(message)
        }
    }
}

#[cfg(test)]
#[path = "execution_archive_test.rs"]
mod execution_archive_tests;
