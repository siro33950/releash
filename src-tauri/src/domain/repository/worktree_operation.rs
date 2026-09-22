use super::RepositoryError;

#[derive(Default)]
pub(crate) struct WorktreeOperationState {
    mutations: usize,
    deletion: Option<WorktreeDeletion>,
}

enum WorktreeDeletion {
    Preparing,
    Accepted(WorktreeDeletionTarget),
}

pub(crate) struct WorktreeDeletionTarget {
    pub repository_root: String,
    pub path: String,
    pub branch: Option<String>,
}

impl WorktreeOperationState {
    pub(crate) fn is_deleting(&self) -> bool {
        self.deletion.is_some()
    }

    pub(crate) fn accept_deletion(
        &mut self,
        target: WorktreeDeletionTarget,
    ) -> Result<(), RepositoryError> {
        if !self.ready_to_delete() {
            return Err(RepositoryError::rule("worktree deletion is not ready"));
        }
        self.deletion = Some(WorktreeDeletion::Accepted(target));
        Ok(())
    }

    pub(crate) fn deletion_target(&self) -> Option<&WorktreeDeletionTarget> {
        match &self.deletion {
            Some(WorktreeDeletion::Accepted(target)) => Some(target),
            _ => None,
        }
    }

    pub(crate) fn begin_mutation(&mut self) -> Result<(), RepositoryError> {
        if self.is_deleting() {
            return Err(RepositoryError::rule("worktree deletion is in progress"));
        }
        self.mutations += 1;
        Ok(())
    }

    pub(crate) fn finish_mutation(&mut self) {
        self.mutations -= 1;
    }

    pub(crate) fn begin_deletion(&mut self) -> Result<(), RepositoryError> {
        if self.is_deleting() {
            return Err(RepositoryError::rule("worktree deletion is in progress"));
        }
        self.deletion = Some(WorktreeDeletion::Preparing);
        Ok(())
    }

    pub(crate) fn ready_to_delete(&self) -> bool {
        self.is_deleting() && self.mutations == 0
    }

    pub(crate) fn finish_deletion(&mut self) {
        self.deletion = None;
    }
}

#[cfg(test)]
#[path = "worktree_operation_test.rs"]
mod worktree_operation_tests;

pub(crate) trait WorktreeOperationLease: Send + Sync {}

#[async_trait::async_trait]
pub(crate) trait WorktreeOperationLocks: Send + Sync {
    fn mutation(&self, identity: &str) -> Result<Box<dyn WorktreeOperationLease>, RepositoryError>;
    async fn deletion(
        &self,
        identity: &str,
    ) -> Result<Box<dyn WorktreeOperationLease>, RepositoryError>;
}
