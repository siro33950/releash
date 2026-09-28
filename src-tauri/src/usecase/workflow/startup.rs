use crate::common::retry::AttemptProgress;
use crate::domain::workflow::repository::WorkflowStartupRepository;
use crate::domain::workflow::WorkflowError;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

#[async_trait::async_trait]
pub(crate) trait WorkflowStartupGateway: Send + Sync {
    fn current_timestamp(&self) -> f64;
    async fn reconcile_tree(&self, tree_id: &str, timestamp: f64) -> Result<(), WorkflowError>;
}

pub(crate) struct WorkflowStartupUsecase {
    repository: Arc<dyn WorkflowStartupRepository>,
    runtime: Arc<dyn WorkflowStartupGateway>,
    checked: Mutex<HashSet<String>>,
}

impl WorkflowStartupUsecase {
    pub(crate) fn new(
        repository: Arc<dyn WorkflowStartupRepository>,
        runtime: Arc<dyn WorkflowStartupGateway>,
    ) -> Self {
        Self {
            repository,
            runtime,
            checked: Mutex::new(HashSet::new()),
        }
    }

    pub(crate) async fn list_tree_ids(&self) -> Result<Vec<String>, WorkflowError> {
        self.repository.list_tree_ids().await
    }

    pub(crate) async fn recover_tree(
        &self,
        tree_id: &str,
        progress: AttemptProgress,
    ) -> Result<(), WorkflowError> {
        if progress == AttemptProgress::Reload {
            self.checked.lock().expect("checked trees").remove(tree_id);
        }
        if !self
            .checked
            .lock()
            .expect("checked trees")
            .contains(tree_id)
        {
            check_startup_definition(self.repository.as_ref(), tree_id).await?;
            self.checked
                .lock()
                .expect("checked trees")
                .insert(tree_id.to_string());
        }
        self.runtime
            .reconcile_tree(tree_id, self.runtime.current_timestamp())
            .await
    }
}

pub(crate) async fn check_startup_definition(
    repository: &dyn WorkflowStartupRepository,
    tree_id: &str,
) -> Result<(), WorkflowError> {
    let Some(record) = repository.load(tree_id).await? else {
        return Ok(());
    };
    if record.execution.is_active() {
        if let Some(reason) = record.definition_error {
            return Err(WorkflowError::IncompatibleStoredEvent(reason));
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "startup_test.rs"]
mod startup_tests;
