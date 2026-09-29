use crate::adaptor::gateway::failure_records::FailureRecordStore;
use crate::common::retry::RetryLimiter;
use crate::usecase::failure::FailureRecordingUsecase;
use crate::usecase::retry::Retrying;
use std::sync::{Arc, OnceLock};

pub(crate) fn test_retrying() -> Arc<Retrying> {
    let store = Arc::new(FailureRecordStore::default());
    Arc::new(Retrying {
        limiter: Arc::new(RetryLimiter::deterministic()),
        failures: Arc::new(FailureRecordingUsecase::new(store.clone(), None)),
        test_query: Some(store),
    })
}

pub(crate) fn shared() -> &'static Arc<Retrying> {
    static SHARED: OnceLock<Arc<Retrying>> = OnceLock::new();
    SHARED.get_or_init(test_retrying)
}
