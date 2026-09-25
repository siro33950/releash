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
    queue: Arc<crate::usecase::work_queue::WorkQueueUsecase>,
    recovery_lock: tokio::sync::Mutex<bool>,
}

impl WorkflowStartupUsecase {
    pub(crate) fn new(
        queue: std::sync::Arc<crate::usecase::work_queue::WorkQueueUsecase>,
        repository: Arc<dyn WorkflowStartupRepository>,
        runtime: Arc<dyn WorkflowStartupGateway>,
    ) -> Self {
        Self {
            repository,
            runtime,
            queue,
            recovery_lock: tokio::sync::Mutex::new(false),
        }
    }

    pub(crate) async fn execute(&self) -> Result<(), WorkflowError> {
        let mut attempted = self.recovery_lock.lock().await;
        if *attempted {
            return Ok(());
        }
        *attempted = true;
        let repository = self.repository.clone();
        let tree_ids = self
            .queue
            .execute(
                crate::usecase::work_queue::WorkKey::new("workflow_recovery_list", "daemon"),
                crate::domain::retry::RetryBackoff::RECOVERY,
                move |_| {
                    let repository = repository.clone();
                    async move {
                        repository.list_tree_ids().await.map_err(|error| {
                            crate::usecase::work_queue::WorkFailure::from_error(&error)
                        })
                    }
                },
            )
            .await
            .map_err(recovery_error)?;
        let results = futures_util::future::join_all(tree_ids.into_iter().map(|tree_id| {
            let repository = self.repository.clone();
            let runtime = self.runtime.clone();
            let checked = Arc::new(std::sync::atomic::AtomicBool::new(false));
            async move {
                self.queue
                    .execute(
                        crate::usecase::work_queue::WorkKey::new("workflow_recovery", &tree_id),
                        crate::domain::retry::RetryBackoff::RECOVERY,
                        move |action| {
                            let repository = repository.clone();
                            let runtime = runtime.clone();
                            let tree_id = tree_id.clone();
                            let checked = checked.clone();
                            async move {
                                if action == crate::domain::failure::RetryAction::Restart {
                                    checked.store(false, std::sync::atomic::Ordering::Release);
                                }
                                if !checked.load(std::sync::atomic::Ordering::Acquire) {
                                    check_startup_definition(repository.as_ref(), &tree_id)
                                        .await
                                        .map_err(|error| {
                                            crate::usecase::work_queue::WorkFailure::from_error(
                                                &error,
                                            )
                                        })?;
                                    checked.store(true, std::sync::atomic::Ordering::Release);
                                }
                                runtime
                                    .reconcile_tree(&tree_id, runtime.current_timestamp())
                                    .await
                                    .map_err(|error| {
                                        crate::usecase::work_queue::WorkFailure::from_error(&error)
                                    })
                            }
                        },
                    )
                    .await
                    .map_err(recovery_error)
            }
        }))
        .await;
        results.into_iter().collect()
    }
}

fn recovery_error(error: crate::usecase::work_queue::WorkFailure) -> WorkflowError {
    WorkflowError::StorageUnavailable {
        kind: error.kind,
        message: error.message,
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
