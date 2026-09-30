use crate::adaptor::gateway::failure_records::FailureRecordStore;
use crate::common::retry::RetryLimiter;
use crate::usecase::failure::FailureRecordingUsecase;
use crate::usecase::retry::Retrying;
use std::sync::{Arc, OnceLock};

pub(crate) fn test_retrying() -> Arc<Retrying> {
    test_retrying_with_store().0
}

pub(crate) fn test_retrying_with_store() -> (Arc<Retrying>, Arc<FailureRecordStore>) {
    let store = Arc::new(FailureRecordStore::default());
    let retrying = Arc::new(Retrying {
        limiter: Arc::new(RetryLimiter::deterministic()),
        failures: Arc::new(FailureRecordingUsecase::new(store.clone(), None)),
        test_query: Some(store.clone()),
    });
    (retrying, store)
}

pub(crate) fn shared() -> &'static Arc<Retrying> {
    &shared_pair().0
}

pub(crate) fn shared_store() -> &'static Arc<FailureRecordStore> {
    &shared_pair().1
}

fn shared_pair() -> &'static (Arc<Retrying>, Arc<FailureRecordStore>) {
    static SHARED: OnceLock<(Arc<Retrying>, Arc<FailureRecordStore>)> = OnceLock::new();
    SHARED.get_or_init(test_retrying_with_store)
}
