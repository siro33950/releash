use crate::adaptor::gateway::failure_records::FailureRecordStore;
use crate::common::retry::RetryLimiter;
use crate::usecase::failure::FailureRecordingUsecase;
use crate::usecase::retry::Retrying;
use std::sync::{Arc, OnceLock};

pub fn test_retrying() -> Arc<Retrying> {
    test_retrying_with_store().0
}

pub fn test_retrying_with_store() -> (Arc<Retrying>, Arc<FailureRecordStore>) {
    let store = Arc::new(FailureRecordStore::default());
    let retrying = Arc::new(Retrying {
        limiter: Arc::new(RetryLimiter::deterministic()),
        failures: Arc::new(FailureRecordingUsecase::new(store.clone(), None)),
    });
    (retrying, store)
}

pub fn shared() -> &'static Arc<Retrying> {
    &shared_pair().0
}

pub fn shared_store() -> &'static Arc<FailureRecordStore> {
    &shared_pair().1
}

fn shared_pair() -> &'static (Arc<Retrying>, Arc<FailureRecordStore>) {
    static SHARED: OnceLock<(Arc<Retrying>, Arc<FailureRecordStore>)> = OnceLock::new();
    SHARED.get_or_init(test_retrying_with_store)
}

impl crate::usecase::workflow::WorkflowRuntimeUsecase {
    pub fn new(
        runtime: Arc<dyn crate::usecase::workflow::ports::WorkflowRuntimeCommandGateway>,
        execution_archives: Arc<dyn crate::domain::workflow::ExecutionTreeArchiveRepository>,
    ) -> Self {
        #[cfg(test)]
        let retrying = crate::usecase::test_helpers::retry::test_retrying();
        #[cfg(not(test))]
        let retrying = shared().clone();
        Self::new_with_worktree_operations(
            retrying,
            runtime,
            execution_archives,
            Default::default(),
        )
    }
}

pub fn shared_limiter() -> Arc<RetryLimiter> {
    shared().limiter.clone()
}

pub fn record_retry_failure(
    retrying: &Retrying,
    key: &crate::usecase::failure::FailureKey,
    failure: crate::usecase::failure::WorkFailure,
) {
    retrying.failures.observed(key, failure);
}
