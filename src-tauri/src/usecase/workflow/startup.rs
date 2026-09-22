use crate::domain::workflow::repository::WorkflowStartupRepository;
use crate::domain::workflow::WorkflowError;
use std::sync::Arc;

#[async_trait::async_trait]
pub(crate) trait WorkflowStartupGateway: Send + Sync {
    fn current_timestamp(&self) -> f64;
    async fn is_registered_or_reserved(&self, tree_id: &str) -> bool;
    async fn reconcile_tree(&self, tree_id: &str, timestamp: f64) -> Result<(), WorkflowError>;
}

pub(crate) struct WorkflowStartupUsecase {
    repository: Arc<dyn WorkflowStartupRepository>,
    runtime: Arc<dyn WorkflowStartupGateway>,
    recovery_lock: tokio::sync::Mutex<()>,
}

impl WorkflowStartupUsecase {
    pub(crate) fn new(
        repository: Arc<dyn WorkflowStartupRepository>,
        runtime: Arc<dyn WorkflowStartupGateway>,
    ) -> Self {
        Self {
            repository,
            runtime,
            recovery_lock: tokio::sync::Mutex::new(()),
        }
    }

    pub(crate) async fn execute(&self) -> Result<(), WorkflowError> {
        let _guard = self.recovery_lock.lock().await;
        let mut first_error = None;
        for tree_id in self.repository.list_tree_ids()? {
            if self.runtime.is_registered_or_reserved(&tree_id).await {
                continue;
            }
            let timestamp = self.runtime.current_timestamp();
            let result =
                match abort_unavailable_definition(self.repository.as_ref(), &tree_id, timestamp) {
                    Ok(()) => self.runtime.reconcile_tree(&tree_id, timestamp).await,
                    Err(error) => Err(error),
                };
            if let Err(error) = result {
                let error = WorkflowError::external(format!(
                    "workflow {tree_id}: reconciliation pass failed: {error}"
                ));
                log::warn!("{error}");
                first_error.get_or_insert(error);
            }
        }
        first_error.map_or(Ok(()), Err)
    }
}

pub fn abort_unavailable_definition(
    repository: &dyn WorkflowStartupRepository,
    tree_id: &str,
    timestamp: f64,
) -> Result<(), WorkflowError> {
    let Some(mut record) = repository.load(tree_id)? else {
        return Ok(());
    };
    if let Some(fact) = record
        .execution
        .abort_unavailable_definition(record.definition_error, timestamp)
    {
        repository.append(&record.root, &fact, timestamp)?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "startup_test.rs"]
mod startup_tests;
