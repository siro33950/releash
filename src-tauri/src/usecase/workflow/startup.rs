use crate::domain::workflow::repository::WorkflowStartupRepository;
use crate::domain::workflow::WorkflowError;
use std::sync::Arc;

#[async_trait::async_trait]
pub(crate) trait WorkflowStartupGateway: Send + Sync {
    fn current_timestamp(&self) -> f64;
    async fn reconcile_tree(&self, tree_id: &str, timestamp: f64) -> Result<(), WorkflowError>;
}

pub(crate) struct WorkflowStartupUsecase {
    repository: Arc<dyn WorkflowStartupRepository>,
    runtime: Arc<dyn WorkflowStartupGateway>,
    recovery_lock: tokio::sync::Mutex<bool>,
}

impl WorkflowStartupUsecase {
    pub(crate) fn new(
        repository: Arc<dyn WorkflowStartupRepository>,
        runtime: Arc<dyn WorkflowStartupGateway>,
    ) -> Self {
        Self {
            repository,
            runtime,
            recovery_lock: tokio::sync::Mutex::new(false),
        }
    }

    pub(crate) async fn execute(&self) -> Result<(), WorkflowError> {
        let mut attempted = self.recovery_lock.lock().await;
        if *attempted {
            return Ok(());
        }
        *attempted = true;
        let mut first_error = None;
        for tree_id in self.repository.list_tree_ids()? {
            let timestamp = self.runtime.current_timestamp();
            let result = match super::command::retry_control_plane_conflicts(|| async {
                abort_unavailable_definition(self.repository.as_ref(), &tree_id, timestamp)
            })
            .await
            {
                Ok(()) => self.runtime.reconcile_tree(&tree_id, timestamp).await,
                Err(error @ WorkflowError::Conflict(_)) => {
                    log::warn!(
                        "workflow {tree_id}: startup definition abort was not applied: {error}"
                    );
                    first_error.get_or_insert(error);
                    continue;
                }
                Err(error) => Err(error),
            };
            if let Err(error) = result {
                let reason = format!("workflow {tree_id}: startup advancement failed: {error}");
                log::warn!("{reason}");
                if let Err(abort_error) = super::command::retry_control_plane_conflicts(|| async {
                    abort_startup_failure(
                        self.repository.as_ref(),
                        &tree_id,
                        reason.clone(),
                        timestamp,
                    )
                })
                .await
                {
                    log::error!("{reason}; abort failed: {abort_error}");
                }
                first_error.get_or_insert_with(|| WorkflowError::external(reason));
            }
        }
        first_error.map_or(Ok(()), Err)
    }
}

fn abort_startup_failure(
    repository: &dyn WorkflowStartupRepository,
    tree_id: &str,
    reason: String,
    timestamp: f64,
) -> Result<(), WorkflowError> {
    let Some(mut record) = repository.load(tree_id)? else {
        return Ok(());
    };
    if let Some(fact) = record.execution.abort_with_reason(reason, timestamp) {
        repository.append(&record.root, &fact, timestamp, Some(&record.revision))?;
    }
    Ok(())
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
        repository.append(&record.root, &fact, timestamp, Some(&record.revision))?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "startup_test.rs"]
mod startup_tests;
