use super::RepositoryError;

#[derive(Default)]
pub(crate) struct WorktreeOperationState {
    mutations: usize,
    deleting: bool,
}

impl WorktreeOperationState {
    pub(crate) fn begin_mutation(&mut self) -> Result<(), RepositoryError> {
        if self.deleting {
            return Err(RepositoryError::rule("worktree deletion is in progress"));
        }
        self.mutations += 1;
        Ok(())
    }

    pub(crate) fn finish_mutation(&mut self) {
        self.mutations -= 1;
    }

    pub(crate) fn begin_deletion(&mut self) -> Result<(), RepositoryError> {
        if self.deleting {
            return Err(RepositoryError::rule("worktree deletion is in progress"));
        }
        self.deleting = true;
        Ok(())
    }

    pub(crate) fn ready_to_delete(&self) -> bool {
        self.deleting && self.mutations == 0
    }

    pub(crate) fn finish_deletion(&mut self) {
        self.deleting = false;
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
